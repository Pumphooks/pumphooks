import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { AccountRole, address, getAddressEncoder, getProgramDerivedAddress, type Address, type Instruction } from "@solana/kit";

export const SYSTEM = address("11111111111111111111111111111111");
export const SPL = address("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
export const T22 = address("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
export const TOKEN_HOOKS = address("FZBBkQQamgfMCAoNk6PAdZi9nYZGMsGw9gDyTMNC3v54");

export const DEPLOY = fileURLToPath(new URL("../../../target/deploy", import.meta.url));
/** Set in CI after `scripts/build_programs.sh`; locally, true when the binaries exist. */
export const SBF = process.env.PUMPHOOKS_SBF === "1" || existsSync(resolve(DEPLOY, "token_hooks.so"));

const enc = getAddressEncoder();
export const bytes = (a: Address) => new Uint8Array(enc.encode(a));
const u64 = (n: bigint) => {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, n, true);
  return b;
};
const cat = (...parts: Uint8Array[]) => {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
};
const W = AccountRole.WRITABLE;
const R = AccountRole.READONLY;
const WS = AccountRole.WRITABLE_SIGNER;
const RS = AccountRole.READONLY_SIGNER;

export function createAccount(from: Address, to: Address, lamports: bigint, space: number, owner: Address): Instruction {
  const data = cat(new Uint8Array([0, 0, 0, 0]), u64(lamports), u64(BigInt(space)), bytes(owner));
  return { programAddress: SYSTEM, accounts: [{ address: from, role: WS }, { address: to, role: WS }], data };
}

// Token-2022 sizes: base mint padded to 165, account-type byte, TLV (type u16, len u16, value).
export const MINT_WITH_HOOK = 165 + 1 + 4 + 64;
export const ACCOUNT_WITH_HOOK = 165 + 1 + 4 + 1;
export const MINT_PLAIN = 82;
export const ACCOUNT_PLAIN = 165;

export function initTransferHook(program: Address, mint: Address, authority: Address, hookProgram: Address): Instruction {
  return { programAddress: program, accounts: [{ address: mint, role: W }], data: cat(new Uint8Array([36, 0]), bytes(authority), bytes(hookProgram)) };
}
export function initMint2(program: Address, mint: Address, authority: Address, decimals: number): Instruction {
  return { programAddress: program, accounts: [{ address: mint, role: W }], data: cat(new Uint8Array([20, decimals]), bytes(authority), new Uint8Array([0])) };
}
export function initAccount3(program: Address, account: Address, mint: Address, owner: Address): Instruction {
  return { programAddress: program, accounts: [{ address: account, role: W }, { address: mint, role: R }], data: cat(new Uint8Array([18]), bytes(owner)) };
}
export function mintTo(program: Address, mint: Address, to: Address, authority: Address, amount: bigint): Instruction {
  return {
    programAddress: program,
    accounts: [{ address: mint, role: W }, { address: to, role: W }, { address: authority, role: RS }],
    data: cat(new Uint8Array([7]), u64(amount)),
  };
}
export function transferChecked(program: Address, from: Address, mint: Address, to: Address, owner: Address, amount: bigint, decimals: number, extra: Instruction["accounts"] = []): Instruction {
  return {
    programAddress: program,
    accounts: [{ address: from, role: W }, { address: mint, role: R }, { address: to, role: W }, { address: owner, role: RS }, ...(extra ?? [])],
    data: cat(new Uint8Array([12]), u64(amount), new Uint8Array([decimals])),
  };
}

export const anchorDisc = (ns: string, name: string) => new Uint8Array(createHash("sha256").update(`${ns}:${name}`).digest()).slice(0, 8);

export async function tokenHooksPdas(mint: Address) {
  const [policy, policyBump] = await getProgramDerivedAddress({ programAddress: TOKEN_HOOKS, seeds: [new TextEncoder().encode("token-policy"), bytes(mint)] });
  const [metas] = await getProgramDerivedAddress({ programAddress: TOKEN_HOOKS, seeds: [new TextEncoder().encode("extra-account-metas"), bytes(mint)] });
  return { policy, policyBump, metas };
}

/** `token_hooks::create_policy(delay, stack)`; `stack` is the compact HookStack. */
export async function createPolicy(payer: Address, authority: Address, mint: Address, stack: Uint8Array): Promise<Instruction> {
  const { policy, metas } = await tokenHooksPdas(mint);
  const len = new Uint8Array(4);
  new DataView(len.buffer).setUint32(0, stack.length, true);
  return {
    programAddress: TOKEN_HOOKS,
    accounts: [
      { address: payer, role: WS },
      { address: authority, role: RS },
      { address: mint, role: R },
      { address: policy, role: W },
      { address: metas, role: W },
      { address: SYSTEM, role: R },
    ],
    data: cat(anchorDisc("global", "create_policy"), u64(0n), len, stack),
  };
}

export const EMPTY_TOKEN_STACK = new Uint8Array([1, 0, 0, 0, 0, 0, 0, 0]);

/** A one-entry token stack: builtin MAX_SIZE (6) on before_transfer, capped at `max` of `mint`. */
export function maxSizeStack(mint: Address, max: bigint): Uint8Array {
  const e = new Uint8Array(216);
  e.set(new Uint8Array(32).fill(7), 0); // hook_version
  e[64] = 0; // builtin
  e[65] = 6; // MAX_SIZE
  e[66] = 1; // mandatory
  new DataView(e.buffer).setUint16(68, 1, true); // before_transfer
  new DataView(e.buffer).setUint32(72, (1 << 2) | (1 << 3), true);
  e.set(bytes(mint), 88);
  e.set(u64(max), 88 + 32);
  return cat(new Uint8Array([1, 0, 1, 0, 0, 0, 0, 0]), e);
}
