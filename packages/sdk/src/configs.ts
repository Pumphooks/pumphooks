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

const SOL = 1_000_000_000n;
const DAY = 86_400;

/**
 * Conservative starting configurations for the market-configuration UI and CLI. Every preset passes
 * `validateConfig`; SENTIENT has no preset signer, the market must supply its engine key.
 */
export const CONFIG_PRESETS: Partial<Record<ModuleId, ModuleConfig>> = {
  [ModuleId.LaunchShield]: { durationSecs: 3_600, cooldownSecs: 30, maxBuyQuote: SOL, maxBuysPerSlot: 4, flags: MODULE_FLAGS.launchShield.DENY_SAME_SLOT_ROUND_TRIP },
  [ModuleId.VolumeGuard]: { maxBuyQuote: 5n * SOL, maxSellQuote: 5n * SOL, windowSecs: 3_600, walletWindowMaxQuote: 20n * SOL },
  [ModuleId.VolatilityGuard]: { maxDeviationBps: 1_500, emaAlphaBps: 2_000, minObservations: 5, checkMinQuote: SOL / 10n, flags: 3 },
  [ModuleId.HolderXp]: { perTradeCapQuote: 10n * SOL, xpPerSol: 100, vestSecs: 3_600, dailyCap: 10_000, minTradeQuote: SOL / 100n },
  [ModuleId.TreasuryRouter]: { reserveBps: 4_000, strategyBps: 3_000, rewardsBps: 2_000, operationsBps: 1_000 },
  [ModuleId.Twap]: { maxTotalQuote: 10n * SOL, maxSliceQuote: SOL, minIntervalSecs: 60, maxDurationSecs: DAY, maxSlippageBps: 100 },
  [ModuleId.Sentient]: { defensiveMaxQuote: SOL / 2n, maxSignalTtl: 3_600, flags: 3 },
};

const n = (v: ConfigValue | undefined): bigint => (v === undefined || v instanceof PublicKey ? 0n : BigInt(v));

/**
 * Client-side mirror of each module's on-chain `validate_config`. Returns human-readable problems;
 * an empty list means the program will accept the config (the program remains the authority).
 */
export function validateConfig(module: ModuleId, values: ModuleConfig = {}): string[] {
  const v = { ...(CONFIG_DEFAULTS[module] ?? {}), ...values } as ModuleConfig;
  const g = (k: string) => n(v[k]);
  const out: string[] = [];
  const need = (ok: boolean, msg: string) => { if (!ok) out.push(msg); };
  switch (module) {
    case ModuleId.LaunchShield:
      need(g("durationSecs") > 0n, "durationSecs must be > 0");
      need(g("durationSecs") <= 30n * 86_400n, "durationSecs must be ≤ 30 days");
      need(g("cooldownSecs") <= 86_400n, "cooldownSecs must be ≤ 1 day");
      need(g("startTs") >= 0n, "startTs must be ≥ 0");
      need((g("flags") & ~3n) === 0n, "unknown flag bits");
      break;
    case ModuleId.VolumeGuard: {
      const windowed = g("windowMaxQuote") > 0n || g("walletWindowMaxQuote") > 0n;
      need(g("maxBuyQuote") > 0n || g("maxSellQuote") > 0n || windowed, "set at least one limit");
      need(!windowed || g("windowSecs") > 0n, "window limits need windowSecs > 0");
      need(g("windowSecs") <= 7n * 86_400n, "windowSecs must be ≤ 7 days");
      break;
    }
    case ModuleId.VolatilityGuard:
      need(g("maxDeviationBps") > 0n && g("maxDeviationBps") <= 1_000_000n, "maxDeviationBps must be 1..1,000,000");
      need(g("emaAlphaBps") > 0n && g("emaAlphaBps") <= 10_000n, "emaAlphaBps must be 1..10,000");
      need(g("flags") > 0n && (g("flags") & ~3n) === 0n, "flags must check buys, sells or both");
      break;
    case ModuleId.HolderXp:
      need(g("xpPerSol") > 0n, "xpPerSol must be > 0");
      need(g("perTradeCapQuote") > 0n, "perTradeCapQuote must be > 0");
      need(g("dailyCap") > 0n, "dailyCap must be > 0");
      need(g("vestSecs") <= 30n * 86_400n, "vestSecs must be ≤ 30 days");
      need(g("holdXpPerDay") === 0n || g("holdCapBase") > 0n, "holding XP needs holdCapBase > 0");
      break;
    case ModuleId.TreasuryRouter:
      need(g("reserveBps") + g("strategyBps") + g("rewardsBps") + g("operationsBps") === 10_000n, "shares must sum to 10,000 bps");
      break;
    case ModuleId.Twap:
      need(g("maxTotalQuote") > 0n, "maxTotalQuote must be > 0");
      need(g("maxSliceQuote") > 0n && g("maxSliceQuote") <= g("maxTotalQuote"), "maxSliceQuote must be 1..maxTotalQuote");
      need(g("minIntervalSecs") >= 10n, "minIntervalSecs must be ≥ 10");
      need(g("maxDurationSecs") > 0n, "maxDurationSecs must be > 0");
      need(g("maxSlippageBps") > 0n && g("maxSlippageBps") <= 2_000n, "maxSlippageBps must be 1..2,000");
      break;
    case ModuleId.Sentient: {
      const s = v.signer;
      need(s instanceof PublicKey && !s.equals(PublicKey.default), "signer (engine key) is required");
      need(g("maxSignalTtl") > 0n && g("maxSignalTtl") <= 86_400n, "maxSignalTtl must be 1..86,400");
      need((g("flags") & ~3n) === 0n, "unknown flag bits");
      break;
    }
  }
  return out;
}

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
