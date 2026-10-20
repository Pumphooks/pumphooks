/**
 * SENTIENT strategy signals. The engine signs a bounded proposal; anyone may relay it. The program
 * verifies the Ed25519 signature (precompile in the same transaction) against the signer configured
 * in the market's SENTIENT slot, then checks bounds. A signal can only change the SENTIENT regime or
 * queue intents the strategy vault validates; it never signs for, or moves, user funds.
 */
import { Buffer } from "buffer";
import { Ed25519Program, PublicKey, TransactionInstruction } from "@solana/web3.js";
import { Writer } from "./codec.js";
import { Regime, SignalAction } from "./constants.js";

export const SIGNAL_DOMAIN = Buffer.from("PUMP_HOOKS_SIGNAL_V1");

export interface StrategySignal {
  action: SignalAction;
  /** Target `Regime` for ChangeStrategyState. */
  param: number;
  confidenceBps: number;
  amountBps: number;
  market: PublicKey;
  nonce: bigint;
  issuedAt: bigint;
  validUntil: bigint;
}

export function encodeSignal(s: StrategySignal): Uint8Array {
  return new Writer()
    .u8(1)
    .u8(s.action)
    .u8(s.param)
    .u8(0)
    .u16(s.confidenceBps)
    .u16(s.amountBps)
    .pubkey(s.market)
    .u64(s.nonce)
    .i64(s.issuedAt)
    .i64(s.validUntil)
    .bytes(new Uint8Array(32))
    .done();
}

/** The exact bytes the engine signs: `SIGNAL_DOMAIN ‖ signal`. */
export function signalMessage(encoded: Uint8Array): Uint8Array {
  return Buffer.concat([SIGNAL_DOMAIN, encoded]);
}

/** Ed25519 verification instruction for a signature produced off-chain. */
export function signalVerifyIx(signer: PublicKey, encoded: Uint8Array, signature: Uint8Array): TransactionInstruction {
  return Ed25519Program.createInstructionWithPublicKey({ publicKey: signer.toBytes(), message: signalMessage(encoded), signature });
}

export function regimeSignal(market: PublicKey, regime: Regime, nonce: bigint, now: bigint, ttlSecs: bigint, confidenceBps = 0): StrategySignal {
  return {
    action: SignalAction.ChangeStrategyState,
    param: regime,
    confidenceBps,
    amountBps: 0,
    market,
    nonce,
    issuedAt: now,
    validUntil: now + ttlSecs,
  };
}
