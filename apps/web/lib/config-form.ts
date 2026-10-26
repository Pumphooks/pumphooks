import { PublicKey } from "@solana/web3.js";
import {
  CONFIG_PRESETS,
  CONFIG_SCHEMAS,
  MODULE_FLAGS,
  ModuleId,
  validateConfig,
  type ConfigField,
  type ModuleConfig,
} from "@pump-hooks/sdk";
import { duration, parseSol, parseUnits, sol, units } from "./format";

export type FormValues = Record<string, string>;

export const FLAG_SETS: Partial<Record<ModuleId, Record<string, number>>> = {
  [ModuleId.LaunchShield]: MODULE_FLAGS.launchShield,
  [ModuleId.VolatilityGuard]: MODULE_FLAGS.volatilityGuard,
  [ModuleId.Sentient]: MODULE_FLAGS.sentient,
};

export const flagLabel = (name: string) => name.toLowerCase().replace(/_/g, " ");

/** Flag fields whose bits are not the module-wide flag set. */
const FIELD_FLAGS: Record<string, Record<string, number>> = {
  [`${ModuleId.TradingHours}.weekdays`]: { MON: 1, TUE: 2, WED: 4, THU: 8, FRI: 16, SAT: 32, SUN: 64 },
  [`${ModuleId.TradingHours}.sides`]: { BUYS: MODULE_FLAGS.tradingHours.BUYS, SELLS: MODULE_FLAGS.tradingHours.SELLS },
};

/** Flag fields that hold one of several values rather than a bit set. */
export const FIELD_CHOICES: Record<string, Record<string, number>> = {
  [`${ModuleId.TradingHours}.dst`]: { "no daylight saving": 0, "US daylight saving": 1, "EU daylight saving": 2 },
};

/** What an all-clear flag field means, where 0 is not "none". */
const EMPTY_FLAGS: Record<string, string> = {
  [`${ModuleId.TradingHours}.weekdays`]: "every day",
  [`${ModuleId.TradingHours}.sides`]: "buys and sells",
};

export const flagsFor = (module: ModuleId, field: string) => FIELD_FLAGS[`${module}.${field}`] ?? FLAG_SETS[module] ?? {};
export const choicesFor = (module: ModuleId, field: string) => FIELD_CHOICES[`${module}.${field}`];

/** Minutes after midnight ↔ "HH:MM". */
export const hhmm = (m: number) => `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
const parseHhmm = (s: string): bigint | undefined => {
  const t = /^(\d{1,2}):(\d{2})$/.exec(s);
  if (!t) return undefined;
  const [h, m] = [Number(t[1]), Number(t[2])];
  return h < 24 && m < 60 ? BigInt(h * 60 + m) : undefined;
};
/** UTC offset in minutes → "UTC+05:30". */
export const utcOffset = (m: number) => `UTC${m < 0 ? "−" : "+"}${hhmm(Math.abs(m))}`;
const isClock = (f: ConfigField) => f.unit === "minutes" && f.type === "u16";

export function presetValues(module: ModuleId, decimals: number, signer?: PublicKey): FormValues {
  const preset: ModuleConfig = { ...(CONFIG_PRESETS[module] ?? {}) };
  if (module === ModuleId.Sentient && signer) preset.signer = signer;
  const out: FormValues = {};
  for (const f of CONFIG_SCHEMAS[module] ?? []) out[f.name] = toText(f, preset[f.name], decimals);
  return out;
}

function toText(f: ConfigField, v: ModuleConfig[string] | undefined, decimals: number): string {
  if (v === undefined) return f.unit === "key" ? "" : "0";
  if (v instanceof PublicKey) return v.toBase58();
  if (f.unit === "lamports") return sol(BigInt(v), 9);
  if (f.unit === "base") return units(BigInt(v), decimals, decimals);
  if (isClock(f)) return hhmm(Number(v));
  return v.toString();
}

/** Read-only rendering of a decoded slot config value. */
export function formatValue(module: ModuleId, f: ConfigField, v: ModuleConfig[string] | undefined, decimals: number): string {
  if (v === undefined) return "—";
  if (v instanceof PublicKey) return v.equals(PublicKey.default) ? "unset" : v.toBase58();
  const n = BigInt(v);
  switch (f.unit) {
    case "lamports":
      return n === 0n ? "off" : `${sol(n)} SOL`;
    case "base":
      return n === 0n ? "off" : units(n, decimals);
    case "seconds":
      return n === 0n ? "0" : duration(n);
    case "bps":
      return `${(Number(n) / 100).toFixed(2)}%`;
    case "timestamp":
      return n === 0n ? "unset" : new Date(Number(n) * 1000).toISOString().replace("T", " ").slice(0, 16) + " UTC";
    case "minutes":
      return isClock(f) ? hhmm(Number(n)) : utcOffset(Number(n));
    case "flags": {
      const choices = choicesFor(module, f.name);
      if (choices) return Object.entries(choices).find(([, x]) => x === Number(n))?.[0] ?? n.toString();
      const set = Object.entries(flagsFor(module, f.name)).filter(([, bit]) => (Number(n) & bit) !== 0);
      return set.length ? set.map(([k]) => flagLabel(k)).join(", ") : EMPTY_FLAGS[`${module}.${f.name}`] ?? "none";
    }
    default:
      return n.toLocaleString();
  }
}

export interface ParsedConfig {
  config: ModuleConfig;
  /** Field-level parse errors. */
  fieldErrors: Record<string, string>;
  /** Mirror of on-chain validation, run only when every field parses. */
  problems: string[];
}

export function parseValues(module: ModuleId, values: FormValues, decimals: number): ParsedConfig {
  const config: ModuleConfig = {};
  const fieldErrors: Record<string, string> = {};
  for (const f of CONFIG_SCHEMAS[module] ?? []) {
    const raw = (values[f.name] ?? "").trim();
    if (f.unit === "key") {
      try {
        if (raw) config[f.name] = new PublicKey(raw);
        else fieldErrors[f.name] = "required";
      } catch {
        fieldErrors[f.name] = "not a public key";
      }
      continue;
    }
    const num = raw.replace(/[,_\s]/g, "") || "0";
    let v: bigint | undefined;
    if (f.unit === "lamports") v = parseSol(num);
    else if (f.unit === "base") v = parseUnits(num, decimals);
    else if (isClock(f)) v = parseHhmm(raw);
    else if (f.type === "i16") v = /^-?\d+$/.test(num) ? BigInt(num) : undefined;
    else v = /^\d+$/.test(num) ? BigInt(num) : undefined;
    if (v === undefined) {
      fieldErrors[f.name] = f.unit === "lamports" ? "SOL amount" : isClock(f) ? "time as HH:MM" : "whole number";
      continue;
    }
    const range = {
      u8: [0n, 0xffn],
      u16: [0n, 0xffffn],
      i16: [-0x8000n, 0x7fffn],
      u32: [0n, 0xffff_ffffn],
      u64: [0n, (1n << 64n) - 1n],
      i64: [0n, (1n << 63n) - 1n],
      pubkey: [0n, 0n],
    }[f.type];
    if (v < range[0] || v > range[1]) {
      fieldErrors[f.name] = "out of range";
      continue;
    }
    config[f.name] = f.type === "u64" || f.type === "i64" ? v : Number(v);
  }
  const problems = Object.keys(fieldErrors).length ? [] : validateConfig(module, config);
  return { config, fieldErrors, problems };
}
