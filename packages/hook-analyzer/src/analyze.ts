import { v1 } from "@pump-hooks/sdk";
import { validateStack } from "./validate.js";

export type Severity = "error" | "warning" | "info";

export interface Diagnostic {
  severity: Severity;
  code: string;
  message: string;
  entries?: number[];
}

/** How a callback is enforced (`hook_standard::Environment::enforcement`). */
export type Enforcement = "native" | "unavailable" | "wrapper" | "router" | "program";

export function enforcement(env: number, callback: number): Enforcement {
  if (env === 0) return callback === 0 ? "native" : callback === 1 ? "unavailable" : "wrapper";
  if (env === 1) return "router";
  return "program";
}

export interface AnalyzeContext {
  /** Unix time used for time-based checks (defaults to now). */
  now?: bigint;
  /** Policy change delay in seconds (token / router policies). */
  policyDelaySecs?: bigint;
  /** Multisig member count of the environment. */
  members?: number;
  /** Measured marginal CU per builtin slug (e.g. from bench/results.json). */
  measuredCu?: Record<string, number>;
  /** Compute available to the stack. */
  computeBudget?: number;
}

export interface Analysis {
  valid: boolean;
  kernel: string;
  environment: string;
  diagnostics: Diagnostic[];
  compute: { estimate: number; measured?: number };
  enforcement: { callback: string; class: Enforcement; entries: number[] }[];
}

const hex = (b: Uint8Array) => Buffer.from(b).toString("hex");
const zero = (b: Uint8Array) => b.every((x) => x === 0);

function keys(cfg: Uint8Array, from: number): string[] {
  return [cfg.slice(from, from + 32), cfg.slice(from + 32, from + 64)].filter((k) => !zero(k)).map(hex);
}

export function analyze(bytes: Uint8Array, ctx: AnalyzeContext = {}): Analysis {
  const now = ctx.now ?? BigInt(Math.floor(Date.now() / 1000));
  const kernel = validateStack(bytes, ctx.members ?? 0);
  const d: Diagnostic[] = [];
  const push = (severity: Severity, code: string, message: string, entries?: number[]) => d.push({ severity, code, message, entries });
  if (kernel !== "ok") push("error", "kernel", `the kernel refuses this stack: ${kernel}`);

  let stack: v1.DecodedStack;
  try {
    stack = v1.decodeStack(bytes);
  } catch (e) {
    return { valid: false, kernel, environment: "unknown", diagnostics: d, compute: { estimate: 0 }, enforcement: [] };
  }
  const env = stack.environment;
  const E = stack.entries;
  const names = v1.CALLBACK_NAMES[env];

  // Token engine install rules (token_hooks::check_token_stack).
  if (env === v1.Environment.Token) {
    for (const e of E) {
      if (e.kind !== "builtin") push("error", "token-program-hook", "token stacks hold builtins only: a program hook would add a CPI level to every transfer", [e.index]);
      if (e.builtinId === 1 && e.config[1] !== 0) push("error", "token-record-allowlist", "token allowlists must be inline: Token-2022 cannot resolve state records for the hook", [e.index]);
    }
    const creds = E.filter((e) => e.builtinId === 13);
    if (creds.length > 2) push("error", "token-credentials", `at most 2 credential entries per token stack (found ${creds.length})`, creds.map((e) => e.index));
  }

  // Enforcement class of every callback in use.
  const enf: Analysis["enforcement"] = [];
  for (let c = 0; c < names.length; c++) {
    const on = E.filter((e) => (e.callbacks >> c) & 1).map((e) => e.index);
    if (!on.length) continue;
    const cls = enforcement(env, c);
    enf.push({ callback: names[c], class: cls, entries: on });
    if (cls === "router") push("warning", "bypassable", `${names[c]}: router hooks apply only to trades routed through Pump Hooks; the venue stays directly callable`, on);
    if (cls === "wrapper") push("info", "wrapper-only", `${names[c]}: enforced by token-wrapper on wrap / unwrap of this mint only`, on);
    if (cls === "unavailable") push("error", "unenforceable", `${names[c]}: Token-2022 has no after-transfer hook; nothing would run this entry`, on);
  }

  // Conflicts and dead policies.
  for (const a of E.filter((e) => e.builtinId === 1 && e.config[1] === 0)) {
    for (const b of E.filter((e) => e.builtinId === 9 && e.config[0] === a.config[0] && (e.callbacks & a.callbacks) !== 0)) {
      const shared = keys(a.config, 16).filter((k) => keys(b.config, 8).includes(k));
      if (shared.length) push("error", "allow-deny-conflict", `the same key is both allowed and denied; every operation by it is refused`, [a.index, b.index]);
    }
  }
  for (const e of E.filter((x) => x.builtinId === 7)) {
    const v = new DataView(e.config.buffer, e.config.byteOffset);
    const notAfter = v.getBigInt64(8, true);
    if (notAfter !== 0n && notAfter <= now) push("error", "window-closed", "time-window ended; every operation on its callbacks is refused", [e.index]);
  }
  for (const e of E.filter((x) => x.builtinId === 3)) {
    const nb = new DataView(e.config.buffer, e.config.byteOffset).getBigInt64(8, true);
    if (nb > now + 365n * 86_400n) push("warning", "timelock-far", `timelock opens more than a year from now (${nb})`, [e.index]);
  }
  for (const e of E.filter((x) => x.builtinId === 2)) {
    const v = new DataView(e.config.buffer, e.config.byteOffset);
    if (v.getBigUint64(32, true) === 0n) push("warning", "zero-limit", "spending limit is 0: every non-zero amount is refused", [e.index]);
  }
  const caps = E.filter((x) => x.builtinId === 6);
  const limits = E.filter((x) => x.builtinId === 2);
  for (const m of caps) {
    for (const l of limits) {
      const mv = new DataView(m.config.buffer, m.config.byteOffset).getBigUint64(32, true);
      const lv = new DataView(l.config.buffer, l.config.byteOffset).getBigUint64(32, true);
      if (hex(m.config.slice(0, 32)) === hex(l.config.slice(0, 32)) && lv < mv) push("info", "limit-below-cap", `max-size ${mv} is never reached: the spending limit ${lv} is lower`, [m.index, l.index]);
    }
  }
  const seen = new Map<string, number>();
  for (const e of E) {
    const k = `${e.kind}:${e.builtinId}:${e.callbacks}:${hex(e.config)}:${hex(e.program)}`;
    if (seen.has(k)) push("warning", "redundant", "entry repeats an earlier entry's policy", [seen.get(k)!, e.index]);
    else seen.set(k, e.index);
  }

  // Dependencies: an entry consuming an output must share a callback with its producer.
  for (const e of E) {
    for (let j = 0; j < e.index; j++) {
      if ((e.dependsOn >> j) & 1 && (E[j].callbacks & e.callbacks) === 0) push("warning", "dependency-never-co-runs", `depends on entry ${j}, which never runs on the same callback`, [j, e.index]);
    }
  }

  // Authorities and upgrades.
  for (const e of E.filter((x) => x.kind === "program")) {
    if (e.pinnedSlot === 0n) push("warning", "unpinned-program", "program hook is not pinned: an upgrade of that program changes this policy immediately", [e.index]);
  }
  const mandatory = E.filter((e) => e.mandatory);
  if (mandatory.length && ctx.policyDelaySecs !== undefined && ctx.policyDelaySecs === 0n) {
    push("warning", "no-delay", "mandatory entries with a zero policy delay can be removed in one transaction", mandatory.map((e) => e.index));
  }
  if (!mandatory.length && E.length) push("info", "nothing-mandatory", "no entry is mandatory: the authority can remove any of them without a delay");

  // Compute.
  const estimate = E.reduce((s, e) => s + (e.kind === "builtin" ? v1.computeEstimate(e.builtinId) : 25_000), 0);
  let measured: number | undefined;
  if (ctx.measuredCu && E.every((e) => e.kind === "builtin" && ctx.measuredCu![e.slug] !== undefined)) {
    measured = E.reduce((s, e) => s + ctx.measuredCu![e.slug], 0);
  }
  const cost = measured ?? estimate;
  if (ctx.computeBudget !== undefined && cost > ctx.computeBudget) push("error", "compute", `stack needs ~${cost} CU, budget ${ctx.computeBudget}`);

  return {
    valid: kernel === "ok" && !d.some((x) => x.severity === "error"),
    kernel,
    environment: v1.ENVIRONMENT_NAMES[env],
    diagnostics: d,
    compute: { estimate, measured },
    enforcement: enf,
  };
}

/** Marginal CU per builtin slug from a hook-bench results file (`execute: <slug>` minus the empty stack). */
export function measuredFromBench(results: { unit: { name: string; compute_units: number }[] }): Record<string, number> {
  const base = results.unit.find((m) => m.name === "execute: empty stack")?.compute_units;
  if (base === undefined) return {};
  const out: Record<string, number> = {};
  for (const m of results.unit) {
    const slug = m.name.replace(/^execute: /, "");
    if (v1.BUILTIN_SLUGS && Object.values(v1.BUILTIN_SLUGS).includes(slug)) out[slug] = m.compute_units - base;
  }
  return out;
}
