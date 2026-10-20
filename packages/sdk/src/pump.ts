/**
 * The SDK's only knowledge of Pump internals. Mirrors `crates/pump-adapter`: layouts, account
 * orders and discriminators of the six trade instructions the router forwards.
 */
import { AccountMeta, PublicKey } from "@solana/web3.js";
import { Reader, ZERO_KEY } from "./codec.js";
import {
  NATIVE_MINT,
  PUMP_AMM_PROGRAM_ID,
  PUMP_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
  TradeMode,
} from "./constants.js";
import { ammPda, associatedToken, pumpPda } from "./pda.js";

export const POOL_DISCRIMINATOR = Uint8Array.from([241, 154, 109, 4, 17, 177, 109, 188]);
export const BONDING_CURVE_DISCRIMINATOR = Uint8Array.from([23, 183, 248, 55, 96, 216, 172, 96]);
export const AMM_GLOBAL_CONFIG_DISCRIMINATOR = Uint8Array.from([149, 8, 156, 202, 160, 252, 176, 217]);

const eq = (a: Uint8Array, b: Uint8Array) => a.length >= b.length && b.every((x, i) => a[i] === x);

export interface PoolView {
  index: number;
  creator: PublicKey;
  baseMint: PublicKey;
  quoteMint: PublicKey;
  lpMint: PublicKey;
  poolBaseTokenAccount: PublicKey;
  poolQuoteTokenAccount: PublicKey;
  coinCreator: PublicKey;
  isMayhem: boolean;
  isCashback: boolean;
  virtualQuoteReserves: bigint;
  creatorFeeBps: bigint;
  isHolderReward: boolean;
}

export function decodePool(d: Uint8Array): PoolView {
  if (d.length < 211 || !eq(d, POOL_DISCRIMINATOR)) throw new Error("not a PumpSwap pool");
  const r = (o: number) => new Reader(d, o);
  return {
    index: r(9).u16(),
    creator: r(11).pubkey(),
    baseMint: r(43).pubkey(),
    quoteMint: r(75).pubkey(),
    lpMint: r(107).pubkey(),
    poolBaseTokenAccount: r(139).pubkey(),
    poolQuoteTokenAccount: r(171).pubkey(),
    coinCreator: r(211).pubkey(),
    isMayhem: r(243).bool(),
    isCashback: r(244).bool(),
    virtualQuoteReserves: r(245).i128(),
    creatorFeeBps: r(261).u64(),
    isHolderReward: r(270).bool(),
  };
}

export interface CurveView {
  virtualTokenReserves: bigint;
  virtualQuoteReserves: bigint;
  realTokenReserves: bigint;
  realQuoteReserves: bigint;
  tokenTotalSupply: bigint;
  complete: boolean;
  creator: PublicKey;
  isMayhem: boolean;
  isCashback: boolean;
  /** Resolved: wSOL for SOL curves (raw field default or wSOL). */
  quoteMint: PublicKey;
  isHolderReward: boolean;
}

export function decodeCurve(d: Uint8Array): CurveView {
  if (d.length < 49 || !eq(d, BONDING_CURVE_DISCRIMINATOR)) throw new Error("not a Pump bonding curve");
  const r = (o: number) => new Reader(d, o);
  const q = r(83).pubkey();
  return {
    virtualTokenReserves: r(8).u64(),
    virtualQuoteReserves: r(16).u64(),
    realTokenReserves: r(24).u64(),
    realQuoteReserves: r(32).u64(),
    tokenTotalSupply: r(40).u64(),
    complete: r(48).bool(),
    creator: r(49).pubkey(),
    isMayhem: r(81).bool(),
    isCashback: r(82).bool(),
    quoteMint: q.equals(ZERO_KEY) ? NATIVE_MINT : q,
    isHolderReward: r(124).bool(),
  };
}

/** PumpSwap GlobalConfig.buyback_fee_recipients. */
export function decodeAmmBuybackRecipients(d: Uint8Array): PublicKey[] {
  if (d.length < 899 || !eq(d, AMM_GLOBAL_CONFIG_DISCRIMINATOR)) throw new Error("not a PumpSwap GlobalConfig");
  const r = new Reader(d, 643);
  return Array.from({ length: 8 }, () => r.pubkey()).filter((k) => !k.equals(ZERO_KEY));
}

/** Pump Global.buyback_fee_recipients (bonding-curve trades). */
export function decodeCurveBuybackRecipients(d: Uint8Array): PublicKey[] {
  if (d.length < 741 + 256) throw new Error("not a Pump Global");
  const r = new Reader(d, 741);
  return Array.from({ length: 8 }, () => r.pubkey()).filter((k) => !k.equals(ZERO_KEY));
}

export interface TokenAccountView {
  mint: PublicKey;
  owner: PublicKey;
  amount: bigint;
}

export function decodeTokenAccount(d: Uint8Array): TokenAccountView {
  if (d.length < 165) throw new Error("not a token account");
  const r = new Reader(d);
  return { mint: r.pubkey(), owner: r.pubkey(), amount: r.u64() };
}

// ---- trade account sets (17 accounts each)

const W = true;
const AMM_V2_META: [boolean, boolean][] = [
  [false, W], [true, W], [false, false], [false, false], [false, false], [false, W], [false, W],
  [false, W], [false, W], [false, false], [false, false], [false, false], [false, W], [false, false],
  [false, W], [false, false], [false, false],
];
const CURVE_V3_META: [boolean, boolean][] = [
  [false, false], [false, false], [false, false], [false, false], [false, false], [false, W], [false, W],
  [false, W], [true, W], [false, W], [false, W], [false, W], [false, false], [false, W], [false, false],
  [false, false], [false, false],
];

export interface AmmKeys {
  pool: PublicKey;
  baseMint: PublicKey;
  quoteMint: PublicKey;
  poolBaseTokenAccount: PublicKey;
  poolQuoteTokenAccount: PublicKey;
  baseTokenProgram: PublicKey;
  quoteTokenProgram: PublicKey;
}

export function ammV2Keys(k: AmmKeys, user: PublicKey, buybackRecipient: PublicKey): PublicKey[] {
  return [
    k.pool,
    user,
    ammPda.globalConfig(),
    k.baseMint,
    k.quoteMint,
    associatedToken(user, k.baseTokenProgram, k.baseMint),
    associatedToken(user, k.quoteTokenProgram, k.quoteMint),
    k.poolBaseTokenAccount,
    k.poolQuoteTokenAccount,
    k.baseTokenProgram,
    k.quoteTokenProgram,
    SYSTEM_PROGRAM_ID,
    ammPda.userVolumeAccumulator(user),
    ammPda.feeConfig(),
    associatedToken(buybackRecipient, k.quoteTokenProgram, k.quoteMint),
    ammPda.eventAuthority(),
    PUMP_AMM_PROGRAM_ID,
  ];
}

export interface CurveKeys {
  baseMint: PublicKey;
  /** Resolved quote mint (wSOL for SOL curves). */
  quoteMint: PublicKey;
  baseTokenProgram: PublicKey;
  quoteTokenProgram: PublicKey;
}

export function curveV3Keys(k: CurveKeys, user: PublicKey, buybackRecipient: PublicKey): PublicKey[] {
  const bc = pumpPda.bondingCurve(k.baseMint);
  const sol = k.quoteMint.equals(NATIVE_MINT);
  return [
    pumpPda.global(),
    k.baseMint,
    k.quoteMint,
    k.baseTokenProgram,
    k.quoteTokenProgram,
    bc,
    associatedToken(bc, k.baseTokenProgram, k.baseMint),
    associatedToken(bc, k.quoteTokenProgram, k.quoteMint),
    user,
    associatedToken(user, k.baseTokenProgram, k.baseMint),
    associatedToken(user, k.quoteTokenProgram, k.quoteMint),
    pumpPda.userVolumeAccumulator(user),
    pumpPda.feeConfig(),
    sol ? buybackRecipient : associatedToken(buybackRecipient, k.quoteTokenProgram, k.quoteMint),
    SYSTEM_PROGRAM_ID,
    pumpPda.eventAuthority(),
    PUMP_PROGRAM_ID,
  ];
}

const metas = (keys: PublicKey[], meta: [boolean, boolean][]): AccountMeta[] =>
  keys.map((pubkey, i) => ({ pubkey, isSigner: meta[i][0], isWritable: meta[i][1] }));

export const ammV2Metas = (k: AmmKeys, user: PublicKey, buyback: PublicKey) =>
  metas(ammV2Keys(k, user, buyback), AMM_V2_META);
export const curveV3Metas = (k: CurveKeys, user: PublicKey, buyback: PublicKey) =>
  metas(curveV3Keys(k, user, buyback), CURVE_V3_META);

// ---- pricing (pre-trade estimates; the router reports observed amounts)

/** Constant-product estimates on the given reserves, before Pump fees. */
export const cp = {
  baseOut(quoteIn: bigint, baseReserve: bigint, quoteReserve: bigint): bigint {
    if (quoteReserve + quoteIn === 0n) return 0n;
    return (baseReserve * quoteIn) / (quoteReserve + quoteIn);
  },
  quoteOut(baseIn: bigint, baseReserve: bigint, quoteReserve: bigint): bigint {
    if (baseReserve + baseIn === 0n) return 0n;
    return (quoteReserve * baseIn) / (baseReserve + baseIn);
  },
  quoteIn(baseOut: bigint, baseReserve: bigint, quoteReserve: bigint): bigint {
    if (baseOut >= baseReserve) return 2n ** 64n - 1n;
    return (quoteReserve * baseOut + (baseReserve - baseOut) - 1n) / (baseReserve - baseOut);
  },
};

export const isBuy = (m: TradeMode) => m !== TradeMode.ExactBaseIn;
