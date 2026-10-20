import { Buffer } from "buffer";
import { PublicKey } from "@solana/web3.js";
import { u16le } from "./codec.js";
import {
  ATA_PROGRAM_ID,
  PUMP_AMM_PROGRAM_ID,
  PUMP_FEES_PROGRAM_ID,
  PUMP_HOOKS_PROGRAM_ID,
  PUMP_PROGRAM_ID,
} from "./constants.js";

const enc = (s: string) => Buffer.from(s);
const find = (seeds: (Buffer | Uint8Array)[], program: PublicKey) => PublicKey.findProgramAddressSync(seeds, program)[0];

export function associatedToken(owner: PublicKey, tokenProgram: PublicKey, mint: PublicKey): PublicKey {
  return find([owner.toBuffer(), tokenProgram.toBuffer(), mint.toBuffer()], ATA_PROGRAM_ID);
}

/** Pump Hooks accounts. */
export const hooksPda = {
  global: (program = PUMP_HOOKS_PROGRAM_ID) => find([enc("global")], program),
  hookDefinition: (moduleId: number, program = PUMP_HOOKS_PROGRAM_ID) => find([enc("hook"), u16le(moduleId)], program),
  /** One market per (mint, namespace). The namespace is the creating authority. */
  market: (mint: PublicKey, namespace: PublicKey, program = PUMP_HOOKS_PROGRAM_ID) =>
    find([enc("market"), mint.toBuffer(), namespace.toBuffer()], program),
  userState: (market: PublicKey, user: PublicKey, program = PUMP_HOOKS_PROGRAM_ID) =>
    find([enc("user"), market.toBuffer(), user.toBuffer()], program),
  policy: (market: PublicKey, index: number, program = PUMP_HOOKS_PROGRAM_ID) =>
    find([enc("policy"), market.toBuffer(), Uint8Array.of(index)], program),
};

/** Pump bonding-curve program accounts. */
export const pumpPda = {
  global: () => find([enc("global")], PUMP_PROGRAM_ID),
  bondingCurve: (mint: PublicKey) => find([enc("bonding-curve"), mint.toBuffer()], PUMP_PROGRAM_ID),
  poolAuthority: (mint: PublicKey) => find([enc("pool-authority"), mint.toBuffer()], PUMP_PROGRAM_ID),
  eventAuthority: () => find([enc("__event_authority")], PUMP_PROGRAM_ID),
  userVolumeAccumulator: (user: PublicKey) => find([enc("user_volume_accumulator"), user.toBuffer()], PUMP_PROGRAM_ID),
  feeConfig: () => find([enc("fee_config"), PUMP_PROGRAM_ID.toBuffer()], PUMP_FEES_PROGRAM_ID),
};

/** PumpSwap (pump_amm) accounts. */
export const ammPda = {
  globalConfig: () => find([enc("global_config")], PUMP_AMM_PROGRAM_ID),
  pool: (index: number, creator: PublicKey, baseMint: PublicKey, quoteMint: PublicKey) =>
    find([enc("pool"), u16le(index), creator.toBuffer(), baseMint.toBuffer(), quoteMint.toBuffer()], PUMP_AMM_PROGRAM_ID),
  /** The pool Pump creates on migration: index 0, created by the curve's pool-authority. */
  canonicalPool: (baseMint: PublicKey, quoteMint: PublicKey) =>
    ammPda.pool(0, pumpPda.poolAuthority(baseMint), baseMint, quoteMint),
  eventAuthority: () => find([enc("__event_authority")], PUMP_AMM_PROGRAM_ID),
  userVolumeAccumulator: (user: PublicKey) =>
    find([enc("user_volume_accumulator"), user.toBuffer()], PUMP_AMM_PROGRAM_ID),
  feeConfig: () => find([enc("fee_config"), PUMP_AMM_PROGRAM_ID.toBuffer()], PUMP_FEES_PROGRAM_ID),
};
