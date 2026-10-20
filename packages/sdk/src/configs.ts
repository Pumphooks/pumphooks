/**
 * Module config schemas. Each first-party module stores a fixed 48-byte little-endian config in its
 * hook slot; offsets mirror `crates/hook-runtime/src/modules/*::Config::encode`.
 */
import { PublicKey } from "@solana/web3.js";
import { Reader, Writer } from "./codec.js";
import { ModuleId, SLOT_CONFIG_LEN } from "./constants.js";

export type FieldType = "u16" | "u32" | "u64" | "i64" | "pubkey";
export type Unit = "lamports" | "seconds" | "bps" | "count" | "xp" | "base" | "timestamp" | "flags" | "key";

export interface ConfigField {
  name: string;
  type: FieldType;
  offset: number;
  unit: Unit;
  description: string;
}

export const CONFIG_SCHEMAS: Partial<Record<ModuleId, readonly ConfigField[]>> = {
  [ModuleId.LaunchShield]: [
    { name: "startTs", type: "i64", offset: 0, unit: "timestamp", description: "Protection start (0 = market activation)" },
    { name: "durationSecs", type: "u32", offset: 8, unit: "seconds", description: "Length of the protected phase" },
    { name: "cooldownSecs", type: "u32", offset: 12, unit: "seconds", description: "Per-wallet cooldown between routed buys" },
    { name: "maxBuyQuote", type: "u64", offset: 16, unit: "lamports", description: "Max routed buy size (0 = none)" },
    { name: "maxSellQuote", type: "u64", offset: 24, unit: "lamports", description: "Max routed sell size (0 = none)" },
    { name: "maxBuysPerSlot", type: "u16", offset: 32, unit: "count", description: "Routed buys per slot (0 = none)" },
    { name: "flags", type: "u16", offset: 34, unit: "flags", description: "bit0 deny same-slot round trips, bit1 cooldown applies to sells" },
  ],
  [ModuleId.VolumeGuard]: [
    { name: "maxBuyQuote", type: "u64", offset: 0, unit: "lamports", description: "Max routed buy size (0 = none)" },
    { name: "maxSellQuote", type: "u64", offset: 8, unit: "lamports", description: "Max routed sell size (0 = none)" },
    { name: "windowSecs", type: "u32", offset: 16, unit: "seconds", description: "Rolling window length (0 = none)" },
    { name: "windowMaxQuote", type: "u64", offset: 24, unit: "lamports", description: "Market routed volume per window" },
    { name: "walletWindowMaxQuote", type: "u64", offset: 32, unit: "lamports", description: "Wallet routed volume per window" },
  ],
  [ModuleId.VolatilityGuard]: [
    { name: "maxDeviationBps", type: "u32", offset: 0, unit: "bps", description: "Deny when |spot − EMA| / EMA exceeds this" },
    { name: "emaAlphaBps", type: "u16", offset: 4, unit: "bps", description: "Weight of a new observation in the EMA" },
    { name: "minObservations", type: "u16", offset: 6, unit: "count", description: "Observations before enforcement" },
    { name: "checkMinQuote", type: "u64", offset: 8, unit: "lamports", description: "Trades below this are not checked" },
    { name: "flags", type: "u32", offset: 16, unit: "flags", description: "bit0 check buys, bit1 check sells (default both)" },
  ],
  [ModuleId.HolderXp]: [
    { name: "perTradeCapQuote", type: "u64", offset: 0, unit: "lamports", description: "Quote above this per trade earns nothing extra" },
    { name: "xpPerSol", type: "u32", offset: 8, unit: "xp", description: "XP per 1 SOL of routed buy" },
    { name: "vestSecs", type: "u32", offset: 12, unit: "seconds", description: "Hold period before buy XP vests" },
    { name: "dailyCap", type: "u32", offset: 16, unit: "xp", description: "XP per wallet per day" },
    { name: "holdXpPerDay", type: "u32", offset: 20, unit: "xp", description: "XP per day for holding" },
    { name: "holdCapBase", type: "u64", offset: 24, unit: "base", description: "Balance that earns the full holding XP" },
    { name: "minTradeQuote", type: "u64", offset: 32, unit: "lamports", description: "Trades below this earn no XP" },
  ],
  [ModuleId.TreasuryRouter]: [
    { name: "reserveBps", type: "u16", offset: 0, unit: "bps", description: "Share to reserve" },
    { name: "strategyBps", type: "u16", offset: 2, unit: "bps", description: "Share to strategy" },
    { name: "rewardsBps", type: "u16", offset: 4, unit: "bps", description: "Share to rewards" },
    { name: "operationsBps", type: "u16", offset: 6, unit: "bps", description: "Share to operations" },
  ],
  [ModuleId.Twap]: [
    { name: "maxTotalQuote", type: "u64", offset: 0, unit: "lamports", description: "Max quote per TWAP program" },
    { name: "maxSliceQuote", type: "u64", offset: 8, unit: "lamports", description: "Max quote per slice" },
    { name: "minIntervalSecs", type: "u32", offset: 16, unit: "seconds", description: "Min time between slices" },
    { name: "maxDurationSecs", type: "u32", offset: 20, unit: "seconds", description: "Max TWAP duration" },
    { name: "maxSlippageBps", type: "u16", offset: 24, unit: "bps", description: "Max slippage per slice" },
  ],
  [ModuleId.Sentient]: [
    { name: "signer", type: "pubkey", offset: 0, unit: "key", description: "Ed25519 key of the off-chain SENTIENT engine" },
    { name: "defensiveMaxQuote", type: "u64", offset: 32, unit: "lamports", description: "Routed trade cap while DEFENSIVE" },
    { name: "maxSignalTtl", type: "u32", offset: 40, unit: "seconds", description: "Longest regime one signal may set" },
    { name: "flags", type: "u32", offset: 44, unit: "flags", description: "bit0 apply to buys, bit1 apply to sells (default both)" },
  ],
  [ModuleId.Policy]: [],
};

export const MODULE_FLAGS = {
  launchShield: { DENY_SAME_SLOT_ROUND_TRIP: 1 << 0, COOLDOWN_SELLS: 1 << 1 },
  volatilityGuard: { CHECK_BUYS: 1 << 0, CHECK_SELLS: 1 << 1 },
  sentient: { APPLY_BUYS: 1 << 0, APPLY_SELLS: 1 << 1 },
} as const;

/** Values used when a field is omitted. Guards default to covering both sides; 0 would disable them. */
export const CONFIG_DEFAULTS: Partial<Record<ModuleId, Record<string, number>>> = {
  [ModuleId.VolatilityGuard]: { flags: MODULE_FLAGS.volatilityGuard.CHECK_BUYS | MODULE_FLAGS.volatilityGuard.CHECK_SELLS },
  [ModuleId.Sentient]: { flags: MODULE_FLAGS.sentient.APPLY_BUYS | MODULE_FLAGS.sentient.APPLY_SELLS },
};

export type ConfigValue = bigint | number | PublicKey;
export type ModuleConfig = Record<string, ConfigValue>;

export function encodeConfig(module: ModuleId, values: ModuleConfig = {}): Uint8Array {
  const schema = CONFIG_SCHEMAS[module];
  if (!schema) throw new Error(`module ${module} has no config schema`);
  const known = new Set(schema.map((f) => f.name));
  for (const k of Object.keys(values)) if (!known.has(k)) throw new Error(`unknown config field ${k} for module ${module}`);
  const out = new Uint8Array(SLOT_CONFIG_LEN);
  const defaults = CONFIG_DEFAULTS[module] ?? {};
  for (const f of schema) {
    const v = values[f.name] ?? defaults[f.name];
    if (v === undefined) continue;
    const w = new Writer();
    switch (f.type) {
      case "u16": w.u16(Number(v)); break;
      case "u32": w.u32(Number(v)); break;
      case "u64": w.u64(BigInt(v as bigint)); break;
      case "i64": w.i64(BigInt(v as bigint)); break;
      case "pubkey": w.pubkey(v as PublicKey); break;
    }
    out.set(w.done(), f.offset);
  }
  return out;
}

export function decodeConfig(module: ModuleId, data: Uint8Array): ModuleConfig {
  const schema = CONFIG_SCHEMAS[module] ?? [];
  const out: ModuleConfig = {};
  for (const f of schema) {
    const r = new Reader(data, f.offset);
    switch (f.type) {
      case "u16": out[f.name] = r.u16(); break;
      case "u32": out[f.name] = r.u32(); break;
      case "u64": out[f.name] = r.u64(); break;
      case "i64": out[f.name] = r.i64(); break;
      case "pubkey": out[f.name] = r.pubkey(); break;
    }
  }
  return out;
}

/** SENTIENT slot state: regime, regime_until, last_nonce, flow window. */
export function decodeSentientState(state: Uint8Array) {
  const r = (o: number) => new Reader(state, o);
  return {
    regime: r(0).u8(),
    regimeUntil: r(8).i64(),
    lastNonce: r(16).u64(),
    flowWindowStart: r(24).i64(),
    buyFlow: r(32).u64(),
    sellFlow: r(40).u64(),
  };
}
