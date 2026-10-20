import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import {
  AccountRole,
  address,
  appendTransactionMessageInstructions,
  compileTransaction,
  compileTransactionMessage,
  createTransactionMessage,
  getBase58Decoder,
  getBase64Decoder,
  getBase64Encoder,
  lamports,
  partiallySignTransaction,
  pipe,
  setTransactionMessageFeePayer,
  setTransactionMessageLifetimeUsingBlockhash,
  type Address,
  type Instruction,
  type KeyPairSigner,
  type Transaction,
} from "@solana/kit";
import { Clock, FailedTransactionMetadata, LiteSVM, type TransactionMetadata } from "litesvm";
import { CALLBACKS, decodeStack, entriesOn, POLICY_STACK_OFFSET, type EnvironmentName } from "./abi.js";

export type Provenance = "observed" | "reconstructed";

/** How each trace field was obtained. "observed" = read from the runtime result or account state. */
export const FIELD_PROVENANCE: Record<string, { provenance: Provenance; source: string }> = {
  status: { provenance: "observed", source: "LiteSVM transaction result" },
  error: { provenance: "observed", source: "LiteSVM transaction error" },
  instructions: { provenance: "observed", source: "compiled transaction message" },
  cpiTree: { provenance: "observed", source: "LiteSVM inner instructions with stack heights" },
  invocations: { provenance: "observed", source: "runtime program logs (invoke / consumed / success / failed)" },
  computeUnits: { provenance: "observed", source: "LiteSVM computeUnitsConsumed" },
  accountDiffs: { provenance: "observed", source: "account state read before and after execution" },
  tokenBalances: { provenance: "observed", source: "SPL Token / Token-2022 account data before and after" },
  logs: { provenance: "observed", source: "runtime logs" },
  returnData: { provenance: "observed", source: "LiteSVM return data" },
  hookSteps: {
    provenance: "reconstructed",
    source: "installed hook stack (pre-state) + kernel in-order evaluation + rejection log; entries do not log individually",
  },
};

export interface ProgramSpec {
  id: string;
  path: string;
  name?: string;
}

export interface AccountState {
  address: string;
  lamports: bigint;
  owner: string;
  executable: boolean;
  data: Uint8Array;
}

export interface AccountRef {
  address: string;
  signer: boolean;
  writable: boolean;
}

export interface TopInstruction {
  index: number;
  programId: string;
  program?: string;
  accounts: AccountRef[];
  data: string;
}

export interface CpiNode {
  programId: string;
  program?: string;
  stackHeight: number;
  accounts: string[];
  data: string;
  children: CpiNode[];
}

export interface Invocation {
  programId: string;
  program?: string;
  depth: number;
  consumed?: number;
  budget?: number;
  result: "success" | "failed" | "incomplete";
  error?: string;
  logs: string[];
}

export interface AccountDiff {
  address: string;
  label?: string;
  signer: boolean;
  writable: boolean;
  created: boolean;
  closed: boolean;
  lamportsBefore: bigint;
  lamportsAfter: bigint;
  ownerBefore: string | null;
  ownerAfter: string | null;
  dataLenBefore: number;
  dataLenAfter: number;
  /** Changed byte ranges `[start, end)` within the common length. */
  changedRanges: [number, number][];
}

export interface TokenBalance {
  account: string;
  mint: string;
  owner: string;
  program: string;
  before: bigint;
  after: bigint;
  delta: bigint;
}

export interface HookStep {
  engine: EnvironmentName;
  program: string;
  policy: string;
  callback: string;
  entry: number;
  name: string;
  kind: "builtin" | "program";
  mandatory: boolean;
  result: "ran" | "rejected" | "not-reached";
  reason?: string;
}

export interface ExecutionTrace {
  label: string;
  status: "success" | "failed";
  error?: string;
  feePayer: string;
  instructions: TopInstruction[];
  /** CPIs made by each top-level instruction, as trees. */
  cpiTree: CpiNode[][];
  invocations: Invocation[];
  computeUnits: { total: bigint; perInstruction: (number | undefined)[] };
  accountDiffs: AccountDiff[];
  tokenBalances: TokenBalance[];
  hookSteps: HookStep[];
  logs: string[];
  returnData?: { programId: string; data: string };
  provenance: typeof FIELD_PROVENANCE;
  /** Exact pre-state and message, for `exportRepro`. */
  replay: { pre: AccountState[]; message: ReproMessage; clock: ReproClock };
}

export interface ReproMessage {
  feePayer: string;
  instructions: { programId: string; accounts: { address: string; role: number }[]; data: string }[];
}

export interface ReproClock {
  slot: string;
  epochStartTimestamp: string;
  epoch: string;
  leaderScheduleEpoch: string;
  unixTimestamp: string;
}

/** Decodes the hook stack that ran inside a program invocation. */
export interface HookResolver {
  programId: string;
  engine: EnvironmentName;
  /** Instruction name logged by Anchor ("Program log: Instruction: <name>"). */
  instruction: string;
  /** Index of the policy account in the program's instruction accounts. */
  policyAccount: number;
  stackOffset: number;
  /** Callbacks the instruction runs, in order. */
  callbacks: number[];
}

export const TOKEN_HOOKS_ID = "FZBBkQQamgfMCAoNk6PAdZi9nYZGMsGw9gDyTMNC3v54";
export const HOOK_ROUTER_ID = "2a3wWdS8z9g7vBcYWSjdETyEzo7DEjV3iYRZFsM52q4U";

export const RESOLVERS: HookResolver[] = [
  { programId: TOKEN_HOOKS_ID, engine: "token", instruction: "Execute", policyAccount: 5, stackOffset: POLICY_STACK_OFFSET.tokenPolicy, callbacks: [0] },
  // route: before_execute, before_swap, venue CPI, after_swap, after_execute.
  { programId: HOOK_ROUTER_ID, engine: "router", instruction: "Route", policyAccount: 2, stackOffset: POLICY_STACK_OFFSET.routePolicy, callbacks: [0, 2, 3, 1] },
];

const KNOWN_PROGRAMS: Record<string, string> = {
  "11111111111111111111111111111111": "system",
  ComputeBudget111111111111111111111111111111: "compute-budget",
  TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA: "spl-token",
  TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb: "token-2022",
  ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL: "associated-token",
  MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr: "memo",
  [TOKEN_HOOKS_ID]: "token-hooks",
  [HOOK_ROUTER_ID]: "hook-router",
};
const TOKEN_PROGRAMS = new Set(["TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"]);

const b64 = { enc: getBase64Decoder(), dec: getBase64Encoder() };
const b58 = getBase58Decoder();
const toB64 = (d: Uint8Array) => b64.enc.decode(d);
const fromB64 = (s: string) => new Uint8Array(b64.dec.encode(s));

export function sha256(bytes: Uint8Array): string {
  return createHash("sha256").update(bytes).digest("hex");
}

export interface LabOptions {
  sigverify?: boolean;
  resolvers?: HookResolver[];
}

export class ExecutionLab {
  readonly svm: LiteSVM;
  readonly programs = new Map<string, { path: string; name?: string; sha256: string }>();
  readonly labels = new Map<string, string>();
  readonly resolvers: HookResolver[];

  constructor(opts: LabOptions = {}) {
    this.svm = new LiteSVM().withSigverify(opts.sigverify ?? true);
    this.resolvers = opts.resolvers ?? RESOLVERS;
  }

  label(addr: string, name: string): this {
    this.labels.set(addr, name);
    return this;
  }

  addProgram(spec: ProgramSpec): this {
    const bytes = readFileSync(spec.path);
    this.svm.addProgram(address(spec.id), bytes);
    this.programs.set(spec.id, { path: spec.path, name: spec.name, sha256: sha256(bytes) });
    if (spec.name) this.labels.set(spec.id, spec.name);
    return this;
  }

  setAccount(a: AccountState): this {
    this.svm.setAccount({
      address: address(a.address),
      lamports: lamports(a.lamports),
      programAddress: address(a.owner),
      executable: a.executable,
      data: a.data,
      space: BigInt(a.data.length),
    });
    return this;
  }

  /** Loads a `solana account --output json` file (the format in tests/fixtures). */
  loadAccountJson(path: string): AccountState {
    const j = JSON.parse(readFileSync(path, "utf8"));
    const a: AccountState = {
      address: j.pubkey,
      lamports: BigInt(j.account.lamports),
      owner: j.account.owner,
      executable: j.account.executable,
      data: fromB64(j.account.data[0]),
    };
    this.setAccount(a);
    return a;
  }

  loadAccountDir(dir: string): AccountState[] {
    return readdirSync(dir)
      .filter((f) => f.endsWith(".json") && f !== "manifest.json")
      .map((f) => this.loadAccountJson(join(dir, f)));
  }

  getAccount(addr: string): AccountState | null {
    const a = this.svm.getAccount(address(addr));
    if (!a.exists) return null;
    return { address: addr, lamports: BigInt(a.lamports), owner: a.programAddress, executable: a.executable, data: new Uint8Array(a.data) };
  }

  airdrop(addr: string, amount: bigint): void {
    this.svm.airdrop(address(addr), lamports(amount));
  }

  setClock(c: { slot?: bigint; unixTimestamp?: bigint }): void {
    const cur = this.svm.getClock();
    this.svm.setClock(new Clock(c.slot ?? cur.slot, cur.epochStartTimestamp, cur.epoch, cur.leaderScheduleEpoch, c.unixTimestamp ?? cur.unixTimestamp));
  }

  /** Runs the instructions as one transaction and returns its trace. Signers are used only to sign. */
  async execute(req: { instructions: Instruction[]; payer: KeyPairSigner | Address; signers?: KeyPairSigner[]; label?: string }): Promise<ExecutionTrace> {
    const payer = typeof req.payer === "string" ? req.payer : req.payer.address;
    const msg = pipe(
      createTransactionMessage({ version: 0 }),
      (m) => setTransactionMessageFeePayer(payer, m),
      (m) => setTransactionMessageLifetimeUsingBlockhash({ blockhash: this.svm.latestBlockhash(), lastValidBlockHeight: 0n }, m),
      (m) => appendTransactionMessageInstructions(req.instructions, m),
    );
    const compiled = compileTransactionMessage(msg);
    let tx: Transaction = compileTransaction(msg) as Transaction;
    const signers = [...(req.signers ?? []), ...(typeof req.payer === "string" ? [] : [req.payer])];
    if (signers.length) {
      tx = (await partiallySignTransaction(signers.map((s) => s.keyPair), tx as never)) as Transaction;
    }
    // Unsigned slots (replays run with sigverify off) get zero signatures.
    const sigs = { ...tx.signatures } as Record<string, Uint8Array | null>;
    for (const k of Object.keys(sigs)) if (!sigs[k]) sigs[k] = new Uint8Array(64);
    tx = { ...tx, signatures: sigs } as unknown as Transaction;

    const keys = compiled.staticAccounts.map(String);
    const h = compiled.header;
    const nSig = h.numSignerAccounts;
    const refs: AccountRef[] = keys.map((k, i) => ({
      address: k,
      signer: i < nSig,
      writable: i < nSig ? i < nSig - h.numReadonlySignerAccounts : i < keys.length - h.numReadonlyNonSignerAccounts,
    }));

    const pre = new Map(keys.map((k) => [k, this.getAccount(k)]));
    const clock = this.svm.getClock();
    const result = this.svm.sendTransaction(tx);
    this.svm.expireBlockhash();
    const failed = result instanceof FailedTransactionMetadata;
    const meta: TransactionMetadata = failed ? result.meta() : result;
    const post = new Map(keys.map((k) => [k, this.getAccount(k)]));

    const instructions: TopInstruction[] = compiled.instructions.map((ix, index) => {
      const programId = keys[ix.programAddressIndex];
      return {
        index,
        programId,
        program: this.name(programId),
        accounts: (ix.accountIndices ?? []).map((i) => refs[i]),
        data: toB64(new Uint8Array(ix.data ?? new Uint8Array())),
      };
    });

    const cpiTree = buildCpiTree(meta, keys, (p) => this.name(p));
    const logs = meta.logs();
    const invocations = parseInvocations(logs, (p) => this.name(p));
    const perInstruction = topLevelConsumption(invocations, instructions.length);
    const ret = meta.returnData();
    const retData = ret.data();

    const accountDiffs = refs.map((r) => diffAccount(r, pre.get(r.address) ?? null, post.get(r.address) ?? null, this.labels.get(r.address)));
    const tokenBalances = refs
      .map((r) => tokenBalance(r.address, pre.get(r.address) ?? null, post.get(r.address) ?? null))
      .filter((t): t is TokenBalance => t !== null);

    const replayMessage: ReproMessage = {
      feePayer: payer,
      instructions: req.instructions.map((ix) => ({
        programId: ix.programAddress,
        accounts: (ix.accounts ?? []).map((a) => ({ address: a.address, role: a.role })),
        data: toB64(new Uint8Array(ix.data ?? new Uint8Array())),
      })),
    };

    const trace: ExecutionTrace = {
      label: req.label ?? "transaction",
      status: failed ? "failed" : "success",
      error: failed ? describeError(result) : undefined,
      feePayer: payer,
      instructions,
      cpiTree,
      invocations,
      computeUnits: { total: meta.computeUnitsConsumed(), perInstruction },
      accountDiffs,
      tokenBalances,
      hookSteps: [],
      logs,
      returnData: retData.length ? { programId: b58.decode(ret.programId()), data: toB64(retData) } : undefined,
      provenance: FIELD_PROVENANCE,
      replay: {
        pre: [...pre.values()].filter((a): a is AccountState => a !== null && !this.programs.has(a.address)),
        message: replayMessage,
        clock: {
          slot: clock.slot.toString(),
          epochStartTimestamp: clock.epochStartTimestamp.toString(),
          epoch: clock.epoch.toString(),
          leaderScheduleEpoch: clock.leaderScheduleEpoch.toString(),
          unixTimestamp: clock.unixTimestamp.toString(),
        },
      },
    };
    trace.hookSteps = resolveHooks(trace, this.resolvers, pre);
    return trace;
  }

  private name(programId: string): string | undefined {
    return this.labels.get(programId) ?? this.programs.get(programId)?.name ?? KNOWN_PROGRAMS[programId];
  }
}

// ---------------------------------------------------------------- trace building

function describeError(r: FailedTransactionMetadata): string {
  const e = r.err() as unknown;
  if (typeof e === "number") return `TransactionError(${e})`;
  const o = e as { index?: number; err?: () => unknown; toString(): string };
  if (typeof o.index === "number" && typeof o.err === "function") {
    const inner = o.err() as unknown;
    const code = (inner as { code?: number })?.code;
    return code !== undefined ? `InstructionError(${o.index}, Custom(${code}))` : `InstructionError(${o.index}, ${String(inner)})`;
  }
  return o.toString();
}

function buildCpiTree(meta: TransactionMetadata, keys: string[], name: (p: string) => string | undefined): CpiNode[][] {
  return meta.innerInstructions().map((list) => {
    const roots: CpiNode[] = [];
    const stack: CpiNode[] = [];
    for (const inner of list) {
      const ci = inner.instruction();
      const programId = keys[ci.programIdIndex()] ?? `#${ci.programIdIndex()}`;
      const node: CpiNode = {
        programId,
        program: name(programId),
        stackHeight: inner.stackHeight(),
        accounts: [...ci.accounts()].map((i) => keys[i] ?? `#${i}`),
        data: toB64(ci.data()),
        children: [],
      };
      while (stack.length && stack[stack.length - 1].stackHeight >= node.stackHeight) stack.pop();
      (stack.length ? stack[stack.length - 1].children : roots).push(node);
      stack.push(node);
    }
    return roots;
  });
}

const INVOKE = /^Program (\w+) invoke \[(\d+)\]$/;
const CONSUMED = /^Program (\w+) consumed (\d+) of (\d+) compute units$/;
const SUCCESS = /^Program (\w+) success$/;
const FAILED = /^Program (\w+) failed: (.*)$/;

export function parseInvocations(logs: string[], name: (p: string) => string | undefined = () => undefined): Invocation[] {
  const out: Invocation[] = [];
  const open: Invocation[] = [];
  for (const line of logs) {
    let m: RegExpMatchArray | null;
    if ((m = line.match(INVOKE))) {
      const inv: Invocation = { programId: m[1], program: name(m[1]), depth: Number(m[2]), result: "incomplete", logs: [] };
      out.push(inv);
      open.push(inv);
    } else if ((m = line.match(CONSUMED))) {
      const top = open[open.length - 1];
      if (top && top.programId === m[1]) {
        top.consumed = Number(m[2]);
        top.budget = Number(m[3]);
      }
    } else if ((m = line.match(SUCCESS))) {
      const top = open.pop();
      if (top) top.result = "success";
    } else if ((m = line.match(FAILED))) {
      const top = open.pop();
      if (top) {
        top.result = "failed";
        top.error = m[2];
      }
    } else if (open.length) {
      open[open.length - 1].logs.push(line);
    }
  }
  return out;
}

function topLevelConsumption(invocations: Invocation[], n: number): (number | undefined)[] {
  const top = invocations.filter((i) => i.depth === 1);
  return Array.from({ length: n }, (_, i) => top[i]?.consumed);
}

function diffAccount(r: AccountRef, before: AccountState | null, after: AccountState | null, label?: string): AccountDiff {
  const a = before?.data ?? new Uint8Array();
  const b = after?.data ?? new Uint8Array();
  const ranges: [number, number][] = [];
  const n = Math.min(a.length, b.length);
  let start = -1;
  for (let i = 0; i < n; i++) {
    if (a[i] !== b[i]) {
      if (start < 0) start = i;
    } else if (start >= 0) {
      ranges.push([start, i]);
      start = -1;
    }
  }
  if (start >= 0) ranges.push([start, n]);
  return {
    address: r.address,
    label,
    signer: r.signer,
    writable: r.writable,
    created: !before && !!after,
    closed: !!before && !after,
    lamportsBefore: before?.lamports ?? 0n,
    lamportsAfter: after?.lamports ?? 0n,
    ownerBefore: before?.owner ?? null,
    ownerAfter: after?.owner ?? null,
    dataLenBefore: a.length,
    dataLenAfter: b.length,
    changedRanges: ranges,
  };
}

function parseTokenAccount(a: AccountState | null): { mint: string; owner: string; amount: bigint; program: string } | null {
  if (!a || !TOKEN_PROGRAMS.has(a.owner) || a.data.length < 165) return null;
  if (a.data.length > 165 && a.data[165] !== 2) return null;
  const v = new DataView(a.data.buffer, a.data.byteOffset, a.data.byteLength);
  return { mint: b58.decode(a.data.slice(0, 32)), owner: b58.decode(a.data.slice(32, 64)), amount: v.getBigUint64(64, true), program: a.owner };
}

function tokenBalance(addr: string, before: AccountState | null, after: AccountState | null): TokenBalance | null {
  const b = parseTokenAccount(before);
  const a = parseTokenAccount(after);
  const t = a ?? b;
  if (!t) return null;
  const pre = b?.amount ?? 0n;
  const post = a?.amount ?? 0n;
  return { account: addr, mint: t.mint, owner: t.owner, program: t.program, before: pre, after: post, delta: post - pre };
}

const REJECTED = /hook stack rejected: callback (\d+) entry (\d+) reason (\d+)(?: \(([^)]*)\))?/;

function resolveHooks(trace: ExecutionTrace, resolvers: HookResolver[], pre: Map<string, AccountState | null>): HookStep[] {
  const steps: HookStep[] = [];
  const calls = flattenCalls(trace);
  for (const inv of trace.invocations) {
    const r = resolvers.find((x) => x.programId === inv.programId && inv.logs.includes(`Program log: Instruction: ${x.instruction}`));
    if (!r) continue;
    const call = calls.find((c) => c.programId === inv.programId && !c.used);
    if (!call) continue;
    call.used = true;
    const policy = call.accounts[r.policyAccount];
    const acct = policy ? pre.get(policy) : undefined;
    if (!policy || policy === r.programId || !acct || acct.data.length < r.stackOffset + 8) continue;
    let stack;
    try {
      stack = decodeStack(acct.data.slice(r.stackOffset));
    } catch {
      continue;
    }
    const rejected = inv.logs.map((l) => l.match(REJECTED)).find(Boolean);
    let stop = false;
    for (const cb of r.callbacks) {
      for (const e of entriesOn(stack, cb)) {
        let result: HookStep["result"] = stop ? "not-reached" : "ran";
        let reason: string | undefined;
        if (!stop && rejected && Number(rejected[1]) === cb && Number(rejected[2]) === e.index) {
          result = "rejected";
          reason = `${rejected[3]}${rejected[4] ? ` (${rejected[4]})` : ""}`;
          stop = true;
        }
        steps.push({
          engine: r.engine,
          program: r.programId,
          policy,
          callback: CALLBACKS[r.engine][cb] ?? String(cb),
          entry: e.index,
          name: e.name,
          kind: e.kind,
          mandatory: e.mandatory,
          result,
          reason,
        });
      }
    }
  }
  return steps;
}

/** Every instruction (top-level and CPI) in execution order with its account list. */
function flattenCalls(trace: ExecutionTrace): { programId: string; accounts: string[]; used?: boolean }[] {
  const out: { programId: string; accounts: string[]; used?: boolean }[] = [];
  const walk = (n: CpiNode) => {
    out.push({ programId: n.programId, accounts: n.accounts });
    n.children.forEach(walk);
  };
  trace.instructions.forEach((ix, i) => {
    out.push({ programId: ix.programId, accounts: ix.accounts.map((a) => a.address) });
    (trace.cpiTree[i] ?? []).forEach(walk);
  });
  return out;
}

export { AccountRole, fromB64, toB64 };
