/**
 * Coin creation through Pump's own `create_v2`. Pump Hooks never owns the coin: the creator signs
 * `create_v2` themselves, then `create_market` reads the fresh curve in the same transaction.
 */
import { Buffer } from "buffer";
import { PublicKey, TransactionInstruction } from "@solana/web3.js";
import { Writer } from "./codec.js";
import {
  ATA_PROGRAM_ID,
  MAYHEM_PROGRAM_ID,
  NATIVE_MINT,
  PUMP_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
  TOKEN_2022_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  Venue,
} from "./constants.js";
import { associatedToken, pumpPda } from "./pda.js";
import type { MarketConfig } from "./accounts.js";

export const CREATE_V2_DISCRIMINATOR = Uint8Array.from([214, 144, 76, 236, 95, 139, 49, 180]);

/** Pump's on-chain limits for `create_v2` metadata. */
export const COIN_LIMITS = { name: 32, symbol: 10, uri: 200 } as const;

const enc = (s: string) => Buffer.from(s);
const find = (seeds: Uint8Array[], program: PublicKey) => PublicKey.findProgramAddressSync(seeds, program)[0];

export const mayhemPda = {
  globalParams: () => find([enc("global-params")], MAYHEM_PROGRAM_ID),
  solVault: () => find([enc("sol-vault")], MAYHEM_PROGRAM_ID),
  state: (mint: PublicKey) => find([enc("mayhem-state"), mint.toBuffer()], MAYHEM_PROGRAM_ID),
  tokenVault: (mint: PublicKey) => associatedToken(mayhemPda.solVault(), TOKEN_2022_PROGRAM_ID, mint),
};

export const pumpMintAuthority = () => find([enc("mint-authority")], PUMP_PROGRAM_ID);

export interface CreateCoinArgs {
  mint: PublicKey;
  user: PublicKey;
  /** Defaults to `user`. The creator receives Pump's creator fees. */
  creator?: PublicKey;
  name: string;
  symbol: string;
  uri: string;
}

function str(w: Writer, s: string, max: number, field: string) {
  const b = enc(s);
  if (b.length === 0 || b.length > max) throw new Error(`${field} must be 1-${max} bytes`);
  w.u32(b.length).bytes(b);
}

/** Pump `create_v2`: a Token-2022 coin on a SOL bonding curve, no mayhem, no cashback. */
export function createCoinIx(p: CreateCoinArgs): TransactionInstruction {
  const bondingCurve = pumpPda.bondingCurve(p.mint);
  const w = new Writer().bytes(CREATE_V2_DISCRIMINATOR);
  str(w, p.name, COIN_LIMITS.name, "name");
  str(w, p.symbol, COIN_LIMITS.symbol, "symbol");
  str(w, p.uri, COIN_LIMITS.uri, "uri");
  w.pubkey(p.creator ?? p.user)
    .bool(false) // is_mayhem_mode
    .bool(false) // is_cashback_enabled (cashback coins cannot be routed)
    .u64(0) // creator_fee_bps
    .bool(false); // is_holder_reward
  const m = (pubkey: PublicKey, isWritable = false, isSigner = false) => ({ pubkey, isWritable, isSigner });
  return new TransactionInstruction({
    programId: PUMP_PROGRAM_ID,
    keys: [
      m(p.mint, true, true),
      m(pumpMintAuthority()),
      m(bondingCurve, true),
      m(associatedToken(bondingCurve, TOKEN_2022_PROGRAM_ID, p.mint), true),
      m(pumpPda.global()),
      m(p.user, true, true),
      m(SYSTEM_PROGRAM_ID),
      m(TOKEN_2022_PROGRAM_ID),
      m(ATA_PROGRAM_ID),
      m(MAYHEM_PROGRAM_ID, true),
      m(mayhemPda.globalParams()),
      m(mayhemPda.solVault(), true),
      m(mayhemPda.state(p.mint), true),
      m(mayhemPda.tokenVault(p.mint), true),
      m(pumpPda.eventAuthority()),
      m(PUMP_PROGRAM_ID),
    ],
    data: w.done(),
  });
}

/**
 * The market `create_market` will write for a coin created by `createCoinIx` in the same
 * transaction, so trades against it can be built before the coin exists.
 */
export function freshCurveMarketConfig(mint: PublicKey, authority: PublicKey): MarketConfig {
  const z = PublicKey.default;
  return {
    namespace: authority, authority, pendingAuthority: z, mint, bondingCurve: pumpPda.bondingCurve(mint),
    pool: z, quoteMint: NATIVE_MINT, baseTokenProgram: TOKEN_2022_PROGRAM_ID, quoteTokenProgram: TOKEN_PROGRAM_ID,
    strategyVault: z, createdAt: 0n, activatedAt: 0n, routedBuyQuote: 0n, routedSellQuote: 0n, tradeCount: 0n,
    sequence: 0n, flags: 0, venue: Venue.BondingCurve, policyMask: 0, slots: [],
  };
}
