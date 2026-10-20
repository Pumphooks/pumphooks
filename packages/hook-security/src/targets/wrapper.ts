// Stateful target: token-wrapper + token-hooks + bundled SPL Token / Token-2022, all real binaries.

import { resolve } from "node:path";
import { AccountRole, address, generateKeyPairSigner, type Instruction, type KeyPairSigner } from "@solana/kit";
import { ExecutionLab, type ExecutionTrace } from "@pump-hooks/execution-lab";
import { v1 } from "@pump-hooks/sdk";
import type { Invariant, Rng, StepResult, Target } from "../fuzz.js";

const SOL = 1_000_000_000n;
const INITIAL = 1_000_000n;
const MINT_PLAIN = 82;
const ACCOUNT_PLAIN = 165;
const ACCOUNT_WITH_HOOK = 165 + 1 + 4 + 1;

export function toKit(ix: v1.InstructionV1): Instruction {
  return {
    programAddress: address(ix.programId),
    accounts: ix.accounts.map((a) => ({
      address: address(a.address),
      role: a.signer ? (a.writable ? AccountRole.WRITABLE_SIGNER : AccountRole.READONLY_SIGNER) : a.writable ? AccountRole.WRITABLE : AccountRole.READONLY,
    })),
    data: ix.data,
  };
}

const sys = {
  createAccount: (from: string, to: string, lamports: bigint, space: number, owner: string): v1.InstructionV1 => ({
    programId: v1.SYSTEM_PROGRAM_ID,
    accounts: [v1.meta(from, true, true), v1.meta(to, true, true)],
    data: v1.concat(v1.u32(0), v1.u64(lamports), v1.u64(BigInt(space)), v1.bytes32(owner)),
  }),
};
const tok = {
  initMint2: (program: string, mint: string, authority: string, decimals: number): v1.InstructionV1 => ({
    programId: program,
    accounts: [v1.meta(mint, true)],
    data: v1.concat(v1.u8(20), v1.u8(decimals), v1.bytes32(authority), v1.u8(0)),
  }),
  initAccount3: (program: string, account: string, mint: string, owner: string): v1.InstructionV1 => ({
    programId: program,
    accounts: [v1.meta(account, true), v1.meta(mint)],
    data: v1.concat(v1.u8(18), v1.bytes32(owner)),
  }),
  mintTo: (program: string, mint: string, to: string, authority: string, amount: bigint): v1.InstructionV1 => ({
    programId: program,
    accounts: [v1.meta(mint, true), v1.meta(to, true), v1.meta(authority, false, true)],
    data: v1.concat(v1.u8(7), v1.u64(amount)),
  }),
};

export interface User {
  signer: KeyPairSigner;
  underlying: string;
  wrapped: string;
}

export interface WrapperState {
  lab: ExecutionLab;
  payer: KeyPairSigner;
  underlyingMint: string;
  wrappedMint: string;
  vault: string;
  wrapper: string;
  policy: string;
  users: User[];
  decimals: number;
  lastTrace?: ExecutionTrace;
}

export type WrapperOp =
  | { kind: "wrap"; user: number; amount: bigint; minOut: bigint; expectOk: boolean }
  | { kind: "unwrap"; user: number; amount: bigint; minOut: bigint; expectOk: boolean }
  | { kind: "transfer"; from: number; to: number; amount: bigint; expectOk: boolean }
  | { kind: "verify"; expectOk: true }
  | { kind: "attack"; attack: AttackId; user: number; victim: number; amount: bigint };

/** Adversarial operations; every one must fail and leave state unchanged. */
export const ATTACKS = {
  "wrap-from-victim": "wrap signed by the attacker, debiting the victim's underlying account",
  "unwrap-victim": "unwrap signed by the attacker, burning the victim's wrapped tokens",
  "fake-vault": "wrap that names the attacker's own token account as the vault",
  "direct-mint": "Token-2022 mint_to on the wrapped mint signed by the attacker",
  "direct-execute": "token-hooks execute called directly, outside a Token-2022 transfer",
  "foreign-policy": "unwrap that passes another mint's policy account to the hook check",
  "transfer-no-hook": "wrapped transfer_checked without the hook's extra accounts",
} as const;
export type AttackId = keyof typeof ATTACKS;

export interface Snapshot {
  accounts: Record<string, string>;
  vault: bigint;
  supply: bigint;
  underlying: bigint[];
  wrapped: bigint[];
  totals: { wrapped: bigint; unwrapped: bigint };
}

const hex = (b: Uint8Array) => Buffer.from(b).toString("hex");

function balance(s: WrapperState, addr: string): bigint {
  const a = s.lab.getAccount(addr);
  return a ? v1.tokenAmount(a.data) : 0n;
}

export async function setupWrapper(deployDir: string, userCount = 3): Promise<WrapperState> {
  const lab = new ExecutionLab();
  lab.addProgram({ id: v1.TOKEN_HOOKS_PROGRAM_ID, path: resolve(deployDir, "token_hooks.so"), name: "token-hooks" });
  lab.addProgram({ id: v1.TOKEN_WRAPPER_PROGRAM_ID, path: resolve(deployDir, "token_wrapper.so"), name: "token-wrapper" });
  const payer = await generateKeyPairSigner();
  lab.airdrop(payer.address, 100n * SOL);
  const mint = await generateKeyPairSigner();
  const decimals = 6;
  const rent = (n: number) => lab.svm.minimumBalanceForRentExemption(BigInt(n));
  const u = mint.address;
  const wrappedMint = v1.wrapperPda.wrappedMint(u)[0];

  const run = async (ixs: v1.InstructionV1[], signers: KeyPairSigner[], label: string) => {
    const t = await lab.execute({ payer, signers, label, instructions: ixs.map(toKit) });
    if (t.status !== "success") throw new Error(`${label} failed: ${t.error}\n${t.logs.join("\n")}`);
  };
  await run(
    [
      sys.createAccount(payer.address, u, rent(MINT_PLAIN), MINT_PLAIN, v1.SPL_TOKEN_PROGRAM_ID),
      tok.initMint2(v1.SPL_TOKEN_PROGRAM_ID, u, payer.address, decimals),
      v1.createWrapper({ payer: payer.address, underlyingMint: u, underlyingProgram: v1.SPL_TOKEN_PROGRAM_ID, hookAuthority: payer.address }),
      v1.createTokenPolicy({ payer: payer.address, authority: payer.address, mint: wrappedMint, stack: v1.encodeStack(v1.Environment.Token, []) }),
    ],
    [mint],
    "create wrapper",
  );
  const users: User[] = [];
  for (let i = 0; i < userCount; i++) {
    const [signer, ua, wa] = await Promise.all([generateKeyPairSigner(), generateKeyPairSigner(), generateKeyPairSigner()]);
    lab.airdrop(signer.address, SOL);
    await run(
      [
        sys.createAccount(payer.address, ua.address, rent(ACCOUNT_PLAIN), ACCOUNT_PLAIN, v1.SPL_TOKEN_PROGRAM_ID),
        tok.initAccount3(v1.SPL_TOKEN_PROGRAM_ID, ua.address, u, signer.address),
        sys.createAccount(payer.address, wa.address, rent(ACCOUNT_WITH_HOOK), ACCOUNT_WITH_HOOK, v1.TOKEN_2022_PROGRAM_ID),
        tok.initAccount3(v1.TOKEN_2022_PROGRAM_ID, wa.address, wrappedMint, signer.address),
        tok.mintTo(v1.SPL_TOKEN_PROGRAM_ID, u, ua.address, payer.address, INITIAL),
      ],
      [ua, wa],
      `user ${i}`,
    );
    users.push({ signer, underlying: ua.address, wrapped: wa.address });
  }
  return {
    lab,
    payer,
    underlyingMint: u,
    wrappedMint,
    vault: v1.wrapperPda.vault(u)[0],
    wrapper: v1.wrapperPda.wrapper(u)[0],
    policy: v1.tokenHooksPda.policy(wrappedMint)[0],
    users,
    decimals,
  };
}

function operateArgs(s: WrapperState, signer: User, underlyingAcct: string, wrappedAcct: string, amount: bigint, minOut: bigint) {
  return {
    user: signer.signer.address,
    underlyingMint: s.underlyingMint,
    underlyingProgram: v1.SPL_TOKEN_PROGRAM_ID,
    userUnderlying: underlyingAcct,
    userWrapped: wrappedAcct,
    amount,
    minOut,
  };
}

function hookAccounts(s: WrapperState, from: User, to: User) {
  const policy = s.lab.getAccount(s.policy)!;
  return v1.transferHookAccounts(s.wrappedMint, v1.decodeTokenPolicy(policy.data).stack, { owner: from.signer.address }, { owner: to.signer.address });
}

export async function applyWrapperOp(s: WrapperState, op: WrapperOp): Promise<StepResult> {
  const exec = async (ixs: v1.InstructionV1[], signers: KeyPairSigner[], label: string): Promise<StepResult> => {
    const t = await s.lab.execute({ payer: s.payer, signers, label, instructions: ixs.map(toKit) });
    s.lastTrace = t;
    return { ok: t.status === "success", error: t.error, detail: t };
  };
  const U = s.users;
  switch (op.kind) {
    case "wrap":
    case "unwrap": {
      const u = U[op.user];
      const f = op.kind === "wrap" ? v1.wrap : v1.unwrap;
      return exec([f(operateArgs(s, u, u.underlying, u.wrapped, op.amount, op.minOut))], [u.signer], op.kind);
    }
    case "transfer": {
      const [a, b] = [U[op.from], U[op.to]];
      return exec(
        [v1.transferChecked({ source: a.wrapped, mint: s.wrappedMint, destination: b.wrapped, owner: a.signer.address, amount: op.amount, decimals: s.decimals, extra: hookAccounts(s, a, b) })],
        [a.signer],
        "transfer",
      );
    }
    case "verify":
      return exec([v1.verifyReserves(s.underlyingMint)], [], "verify_reserves");
    case "attack": {
      const [a, v] = [U[op.user], U[op.victim]];
      const amt = op.amount === 0n ? 1n : op.amount;
      switch (op.attack) {
        case "wrap-from-victim":
          return exec([v1.wrap(operateArgs(s, a, v.underlying, a.wrapped, amt, 0n))], [a.signer], op.attack);
        case "unwrap-victim":
          return exec([v1.unwrap(operateArgs(s, a, a.underlying, v.wrapped, amt, 0n))], [a.signer], op.attack);
        case "fake-vault": {
          const ix = v1.wrap(operateArgs(s, a, a.underlying, a.wrapped, amt, 0n));
          ix.accounts[4] = v1.meta(v.underlying, true);
          return exec([ix], [a.signer], op.attack);
        }
        case "direct-mint":
          return exec([tok.mintTo(v1.TOKEN_2022_PROGRAM_ID, s.wrappedMint, a.wrapped, a.signer.address, amt)], [a.signer], op.attack);
        case "direct-execute":
          return exec(
            [
              {
                programId: v1.TOKEN_HOOKS_PROGRAM_ID,
                accounts: [
                  v1.meta(a.wrapped),
                  v1.meta(s.wrappedMint),
                  v1.meta(v.wrapped),
                  v1.meta(a.signer.address),
                  v1.meta(v1.tokenHooksPda.extraAccountMetas(s.wrappedMint)[0]),
                  v1.meta(s.policy, true),
                ],
                data: v1.concat(Uint8Array.of(105, 37, 101, 197, 75, 251, 102, 26), v1.u64(amt)),
              },
            ],
            [],
            op.attack,
          );
        case "foreign-policy": {
          const ix = v1.unwrap(operateArgs(s, a, a.underlying, a.wrapped, amt, 0n));
          ix.accounts[7] = v1.meta(v1.tokenHooksPda.policy(s.underlyingMint)[0], true);
          return exec([ix], [a.signer], op.attack);
        }
        case "transfer-no-hook":
          return exec([v1.transferChecked({ source: a.wrapped, mint: s.wrappedMint, destination: v.wrapped, owner: a.signer.address, amount: amt, decimals: s.decimals })], [a.signer], op.attack);
      }
    }
  }
}

export function generateWrapperOp(rng: Rng, s: WrapperState): WrapperOp {
  const n = s.users.length;
  const user = rng.int(n);
  const r = rng.int(20);
  if (r < 6) {
    const bal = balance(s, s.users[user].underlying);
    const amount = rng.amount(bal + (rng.int(4) === 0 ? 10n : 0n));
    const minOut = rng.int(5) === 0 ? amount + 1n : rng.amount(amount);
    return { kind: "wrap", user, amount, minOut, expectOk: amount > 0n && amount <= bal && minOut <= amount };
  }
  if (r < 11) {
    const bal = balance(s, s.users[user].wrapped);
    const amount = rng.amount(bal + (rng.int(4) === 0 ? 10n : 0n));
    const minOut = rng.int(5) === 0 ? amount + 1n : rng.amount(amount);
    return { kind: "unwrap", user, amount, minOut, expectOk: amount > 0n && amount <= bal && minOut <= amount };
  }
  if (r < 15) {
    const bal = balance(s, s.users[user].wrapped);
    const amount = rng.amount(bal + (rng.int(4) === 0 ? 10n : 0n));
    return { kind: "transfer", from: user, to: rng.int(n), amount, expectOk: amount <= bal };
  }
  if (r < 16) return { kind: "verify", expectOk: true };
  const victim = (user + 1 + rng.int(n - 1)) % n;
  const attack = rng.pick(Object.keys(ATTACKS) as AttackId[]);
  return { kind: "attack", attack, user, victim, amount: rng.amount(balance(s, s.users[victim].wrapped) + balance(s, s.users[victim].underlying)) };
}

export function snapshotWrapper(s: WrapperState): Snapshot {
  const tracked = [s.vault, s.wrappedMint, s.wrapper, s.policy, ...s.users.flatMap((u) => [u.underlying, u.wrapped])];
  const accounts: Record<string, string> = {};
  for (const a of tracked) {
    const acc = s.lab.getAccount(a);
    accounts[a] = acc ? `${acc.lamports}:${acc.owner}:${hex(acc.data)}` : "absent";
  }
  const w = v1.decodeWrapper(s.lab.getAccount(s.wrapper)!.data);
  return {
    accounts,
    vault: balance(s, s.vault),
    supply: v1.mintSupply(s.lab.getAccount(s.wrappedMint)!.data),
    underlying: s.users.map((u) => balance(s, u.underlying)),
    wrapped: s.users.map((u) => balance(s, u.wrapped)),
    totals: { wrapped: w.totalWrapped, unwrapped: w.totalUnwrapped },
  };
}

const sum = (xs: bigint[]) => xs.reduce((a, b) => a + b, 0n);

export const WRAPPER_INVARIANTS: Invariant<WrapperState, WrapperOp, Snapshot>[] = [
  {
    name: "reserves-cover-supply",
    check: (_s, { after }) => (after.vault >= after.supply ? null : `vault ${after.vault} < wrapped supply ${after.supply}`),
  },
  {
    name: "fee-free-underlying-is-exactly-backed",
    check: (_s, { after }) => (after.vault === after.supply ? null : `vault ${after.vault} != supply ${after.supply} for a fee-free underlying`),
  },
  {
    name: "wrapped-balances-sum-to-supply",
    check: (_s, { after }) => (sum(after.wrapped) === after.supply ? null : `holders ${sum(after.wrapped)} != supply ${after.supply}`),
  },
  {
    name: "underlying-conserved",
    check: (s, { after }) => {
      const total = sum(after.underlying) + after.vault;
      const expected = INITIAL * BigInt(s.users.length);
      return total === expected ? null : `underlying total ${total} != ${expected}`;
    },
  },
  {
    name: "wrapper-totals-match-supply",
    check: (_s, { after }) => (after.totals.wrapped - after.totals.unwrapped === after.supply ? null : `totals ${after.totals.wrapped}-${after.totals.unwrapped} != supply ${after.supply}`),
  },
  {
    name: "failed-operations-change-nothing",
    check: (_s, { result, before, after }) => {
      if (result.ok) return null;
      const changed = Object.keys(before.accounts).filter((k) => before.accounts[k] !== after.accounts[k]);
      return changed.length ? `failed transaction changed ${changed.join(", ")}` : null;
    },
  },
  {
    name: "attacks-fail",
    check: (_s, { op, result }) => (op.kind === "attack" && result.ok ? `attack "${op.attack}" succeeded: ${ATTACKS[op.attack]}` : null),
  },
  {
    name: "honest-operations-behave-as-specified",
    check: (_s, { op, result }) =>
      op.kind !== "attack" && result.ok !== op.expectOk ? `${op.kind} expected ${op.expectOk ? "success" : "failure"}, got ${result.ok ? "success" : `failure ${result.error}`}` : null,
  },
];

export function describeWrapperOp(op: WrapperOp): string {
  switch (op.kind) {
    case "wrap":
    case "unwrap":
      return `${op.kind} user=${op.user} amount=${op.amount} min_out=${op.minOut}`;
    case "transfer":
      return `transfer ${op.from}->${op.to} amount=${op.amount}`;
    case "verify":
      return "verify_reserves";
    case "attack":
      return `attack:${op.attack} attacker=${op.user} victim=${op.victim} amount=${op.amount}`;
  }
}

export function wrapperTarget(deployDir: string, extraInvariants: Invariant<WrapperState, WrapperOp, Snapshot>[] = []): Target<WrapperState, WrapperOp, Snapshot> {
  return {
    name: "token-wrapper + token-hooks",
    setup: () => setupWrapper(deployDir),
    generate: generateWrapperOp,
    apply: applyWrapperOp,
    snapshot: snapshotWrapper,
    invariants: [...WRAPPER_INVARIANTS, ...extraInvariants],
    describe: describeWrapperOp,
  };
}
