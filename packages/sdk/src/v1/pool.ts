// hooks-core (POOL engine) encoders: constant-product pools whose swaps and liquidity changes run
// the pool's hook stack. Pool hooks govern only pools in this program.

import { anchorDiscriminator, bytes32, compareBytes, concat, findPda, meta, seed, u32, u64, u8, vecU8, type InstructionV1, type KeyLike } from "./ix.js";
import { Environment } from "./stack.js";
import { programData } from "./registry.js";
import { SPL_TOKEN_PROGRAM_ID, SYSTEM_PROGRAM_ID } from "./tokens.js";

export const HOOKS_CORE_PROGRAM_ID = "8L8VFMpzP7S3ZfSyr9vu5vDVy93jgfWVfEzY4RTadFzw";

export const PoolCallback = {
  BeforeInitialize: 0, AfterInitialize: 1, BeforeAddLiquidity: 2, AfterAddLiquidity: 3, BeforeRemoveLiquidity: 4, AfterRemoveLiquidity: 5,
  BeforeSwap: 6, AfterSwap: 7, BeforeDonate: 8, AfterDonate: 9, Quote: 10,
} as const;

const ZERO = new Uint8Array(32);
const NONE = meta(HOOKS_CORE_PROGRAM_ID);

export const poolPda = {
  protocol: () => findPda([seed("protocol")], HOOKS_CORE_PROGRAM_ID)[0],
  pool: (mint0: KeyLike, mint1: KeyLike, hookProgram: KeyLike | null, salt: bigint) =>
    findPda([seed("pool"), bytes32(mint0), bytes32(mint1), hookProgram ? bytes32(hookProgram) : ZERO, u64(salt)], HOOKS_CORE_PROGRAM_ID)[0],
  vault: (pool: KeyLike, mint: KeyLike) => findPda([seed("vault"), bytes32(pool), bytes32(mint)], HOOKS_CORE_PROGRAM_ID)[0],
  lpMint: (pool: KeyLike) => findPda([seed("lp"), bytes32(pool)], HOOKS_CORE_PROGRAM_ID)[0],
  policy: (pool: KeyLike) => findPda([seed("pool-policy"), bytes32(pool)], HOOKS_CORE_PROGRAM_ID)[0],
};

/** A hook-less pool's addresses. Mints must be sorted (`mint0 < mint1` bytewise). */
export interface PoolKeys {
  pool: string;
  mint: [string, string];
  tokenProgram: [string, string];
  vault: [string, string];
  lpMint: string;
}

export function poolKeys(mint0: KeyLike, mint1: KeyLike, tokenProgram: [KeyLike, KeyLike] = [SPL_TOKEN_PROGRAM_ID, SPL_TOKEN_PROGRAM_ID], salt = 0n): PoolKeys {
  if (compareBytes(bytes32(mint0), bytes32(mint1)) >= 0) throw new Error("mint0 must sort before mint1");
  const pool = poolPda.pool(mint0, mint1, null, salt);
  const m: [string, string] = [meta(mint0).address, meta(mint1).address];
  return {
    pool,
    mint: m,
    tokenProgram: [meta(tokenProgram[0]).address, meta(tokenProgram[1]).address],
    vault: [poolPda.vault(pool, m[0]), poolPda.vault(pool, m[1])],
    lpMint: poolPda.lpMint(pool),
  };
}

/** Only the core's upgrade authority may initialize. */
export function initializeProtocol(admin: KeyLike, feeAuthority: KeyLike): InstructionV1 {
  return {
    programId: HOOKS_CORE_PROGRAM_ID,
    accounts: [meta(admin, true, true), meta(poolPda.protocol(), true), meta(HOOKS_CORE_PROGRAM_ID), meta(programData(HOOKS_CORE_PROGRAM_ID)), meta(SYSTEM_PROGRAM_ID)],
    data: concat(anchorDiscriminator("initialize_protocol"), bytes32(feeAuthority)),
  };
}

/** A pool without a program hook (policies attach separately). Fees in parts per million. */
export function initializePool(a: { creator: KeyLike; keys: PoolKeys; feePpm: number; maxFeePpm: number; salt?: bigint }): InstructionV1 {
  const k = a.keys;
  return {
    programId: HOOKS_CORE_PROGRAM_ID,
    accounts: [
      meta(a.creator, true, true),
      meta(poolPda.protocol(), true),
      meta(k.mint[0]),
      meta(k.mint[1]),
      meta(k.pool, true),
      meta(k.vault[0], true),
      meta(k.vault[1], true),
      meta(k.lpMint, true),
      NONE,
      NONE,
      NONE,
      NONE,
      meta(k.tokenProgram[0]),
      meta(k.tokenProgram[1]),
      meta(SPL_TOKEN_PROGRAM_ID),
      meta(SYSTEM_PROGRAM_ID),
    ],
    data: concat(anchorDiscriminator("initialize_pool"), ZERO, u32(0), u32(a.feePpm), u32(a.maxFeePpm), u64(a.salt ?? 0n), u8(0), vecU8(new Uint8Array())),
  };
}

/** Attaches the pool's hook stack; only the pool creator, and only once. `versions`: registry versions of the entries. */
export function attachPoolPolicy(a: { creator: KeyLike; pool: KeyLike; stack: Uint8Array; versions: KeyLike[] }): InstructionV1 {
  if (a.stack[1] !== Environment.Pool) throw new Error("pool policies take pool-environment stacks");
  return {
    programId: HOOKS_CORE_PROGRAM_ID,
    accounts: [meta(a.creator, true, true), meta(a.pool, true), meta(poolPda.policy(a.pool), true), meta(SYSTEM_PROGRAM_ID), ...a.versions.map((v) => meta(v))],
    data: concat(anchorDiscriminator("attach_policy"), vecU8(a.stack)),
  };
}

const poolIo = (k: PoolKeys) => [
  meta(k.pool, true),
  meta(k.vault[0], true),
  meta(k.vault[1], true),
  meta(k.mint[0]),
  meta(k.mint[1]),
  meta(k.tokenProgram[0]),
  meta(k.tokenProgram[1]),
];
const hookIo = [NONE, NONE, NONE];
const policyMeta = (k: PoolKeys, hasPolicy: boolean) => (hasPolicy ? meta(poolPda.policy(k.pool), true) : NONE);

export function addLiquidity(a: { owner: KeyLike; keys: PoolKeys; owner0: KeyLike; owner1: KeyLike; ownerLp: KeyLike; max0: bigint; max1: bigint; minShares: bigint; hasPolicy: boolean }): InstructionV1 {
  const k = a.keys;
  return {
    programId: HOOKS_CORE_PROGRAM_ID,
    accounts: [
      meta(a.owner, false, true),
      meta(poolPda.protocol(), true),
      ...poolIo(k),
      meta(k.lpMint, true),
      meta(a.owner0, true),
      meta(a.owner1, true),
      meta(a.ownerLp, true),
      meta(SPL_TOKEN_PROGRAM_ID),
      ...hookIo,
      policyMeta(k, a.hasPolicy),
    ],
    data: concat(anchorDiscriminator("add_liquidity"), u64(a.max0), u64(a.max1), u64(a.minShares), vecU8(new Uint8Array())),
  };
}

/** `zeroForOne`: sell token 0 for token 1. `exactIn`: `amount` is the input and `limit` the minimum output. */
export function swap(a: { trader: KeyLike; keys: PoolKeys; trader0: KeyLike; trader1: KeyLike; zeroForOne: boolean; exactIn: boolean; amount: bigint; limit: bigint; hasPolicy: boolean }): InstructionV1 {
  const k = a.keys;
  return {
    programId: HOOKS_CORE_PROGRAM_ID,
    accounts: [meta(a.trader, false, true), meta(poolPda.protocol()), ...poolIo(k), meta(a.trader0, true), meta(a.trader1, true), ...hookIo, policyMeta(k, a.hasPolicy)],
    data: concat(anchorDiscriminator("swap"), u8(a.zeroForOne ? 1 : 0), u8(a.exactIn ? 1 : 0), u64(a.amount), u64(a.limit), vecU8(new Uint8Array())),
  };
}
