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
import { parseSol, parseUnits, sol, units } from "./format";

export type FormValues = Record<string, string>;

export const FLAG_SETS: Partial<Record<ModuleId, Record<string, number>>> = {
  [ModuleId.LaunchShield]: MODULE_FLAGS.launchShield,
  [ModuleId.VolatilityGuard]: MODULE_FLAGS.volatilityGuard,
  [ModuleId.Sentient]: MODULE_FLAGS.sentient,
};

export const flagLabel = (name: string) => name.toLowerCase().replace(/_/g, " ");

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
  return v.toString();
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
    let v: bigint | undefined;
    if (f.unit === "lamports") v = parseSol(raw || "0");
    else if (f.unit === "base") v = parseUnits(raw || "0", decimals);
    else v = /^\d+$/.test(raw || "0") ? BigInt(raw || "0") : undefined;
    if (v === undefined) {
      fieldErrors[f.name] = f.unit === "lamports" ? "SOL amount" : "whole number";
      continue;
    }
    const max = { u16: 0xffffn, u32: 0xffff_ffffn, u64: (1n << 64n) - 1n, i64: (1n << 63n) - 1n, pubkey: 0n }[f.type];
    if (v > max) {
      fieldErrors[f.name] = "too large";
      continue;
    }
    config[f.name] = f.type === "u64" || f.type === "i64" ? v : Number(v);
  }
  const problems = Object.keys(fieldErrors).length ? [] : validateConfig(module, config);
  return { config, fieldErrors, problems };
}
