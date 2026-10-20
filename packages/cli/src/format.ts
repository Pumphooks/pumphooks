import { PublicKey } from "@solana/web3.js";
import {
  Callback,
  callbacksOf,
  capabilitiesOf,
  CONFIG_SCHEMAS,
  describeReason,
  ExecutionResult,
  MARKET_FLAGS,
  MarketConfig,
  moduleInfo,
  MODULES,
  Venue,
} from "@pump-hooks/sdk";

export const LAMPORTS = 1_000_000_000n;

export function sol(lamports: bigint): string {
  const neg = lamports < 0n;
  const v = neg ? -lamports : lamports;
  return `${neg ? "-" : ""}${v / LAMPORTS}.${(v % LAMPORTS).toString().padStart(9, "0")} SOL`;
}

/** "0.25", "0.25sol", "250000000lamports" or a plain integer of lamports. */
export function parseSol(s: string): bigint {
  const t = s.trim().toLowerCase();
  if (t.endsWith("lamports")) return BigInt(t.slice(0, -8));
  const n = t.endsWith("sol") ? t.slice(0, -3) : t;
  return parseUnits(n, 9);
}

export function parseUnits(s: string, decimals: number): bigint {
  if (!/^\d+(\.\d+)?$/.test(s)) throw new Error(`not a number: ${s}`);
  const [w, f = ""] = s.split(".");
  if (f.length > decimals) throw new Error(`${s} has more than ${decimals} decimals`);
  return BigInt(w) * 10n ** BigInt(decimals) + BigInt(f.padEnd(decimals, "0") || "0");
}

export function units(v: bigint, decimals: number): string {
  const d = 10n ** BigInt(decimals);
  return `${v / d}.${(v % d).toString().padStart(decimals, "0")}`;
}

export const short = (k: PublicKey | string) => {
  const s = typeof k === "string" ? k : k.toBase58();
  return `${s.slice(0, 4)}…${s.slice(-4)}`;
};

export const moduleName = (id: number) => MODULES.find((m) => m.id === id)?.name ?? `module ${id}`;

export function table(rows: (string | number)[][]): string {
  const w = rows[0].map((_, i) => Math.max(...rows.map((r) => String(r[i]).length)));
  return rows.map((r) => r.map((c, i) => String(c).padEnd(w[i])).join("  ").trimEnd()).join("\n");
}

export function json(v: unknown): string {
  return JSON.stringify(
    v,
    (_, x) => (typeof x === "bigint" ? x.toString() : x instanceof PublicKey ? x.toBase58() : x instanceof Uint8Array ? Buffer.from(x).toString("hex") : x),
    2,
  );
}

export function marketText(address: PublicKey, m: MarketConfig): string {
  const flags = Object.entries(MARKET_FLAGS).filter(([, b]) => m.flags & b).map(([k]) => k);
  const head = [
    `market        ${address.toBase58()}`,
    `mint          ${m.mint.toBase58()}`,
    `venue         ${m.venue === Venue.PumpSwap ? `PumpSwap pool ${m.pool.toBase58()}` : `bonding curve ${m.bondingCurve.toBase58()}`}`,
    `authority     ${m.authority.toBase58()}`,
    `flags         ${flags.join(" ") || "-"}`,
    `routed        buys ${sol(m.routedBuyQuote)}  sells ${sol(m.routedSellQuote)}  trades ${m.tradeCount}`,
    "",
    "hook stack (execution order)",
  ];
  const rows: (string | number)[][] = [["#", "module", "priority", "callbacks", "capabilities", "enabled"]];
  for (const s of m.slots) {
    rows.push([
      s.index,
      moduleName(s.moduleId),
      s.priority,
      callbacksOf(s.callbacks).map((c) => Callback[c]).join(",") || "config-only",
      capabilitiesOf(s.capabilities).join(","),
      s.enabled ? "yes" : "no",
    ]);
  }
  const cfg = m.slots.map((s) => {
    const schema = CONFIG_SCHEMAS[s.moduleId as keyof typeof CONFIG_SCHEMAS] ?? [];
    const entries = Object.entries(s.decodedConfig).map(([k, v]) => {
      const unit = schema.find((f) => f.name === k)?.unit;
      return `${k}=${v instanceof PublicKey ? v.toBase58() : unit === "lamports" ? sol(BigInt(v)).replace(" SOL", "") : String(v)}`;
    });
    return `  ${moduleName(s.moduleId)}: ${entries.join(" ") || "-"}`;
  });
  return [...head, m.slots.length ? table(rows) : "  (no hooks installed)", "", "config", ...cfg].join("\n");
}

export function resultText(r: ExecutionResult, decimals = 6): string {
  const out: string[] = [];
  out.push(r.ok ? "status        OK" : `status        FAILED${r.error ? ` ${r.error.name} (${r.error.code}): ${r.error.msg}` : ` ${JSON.stringify(r.rawError)}`}`);
  if (r.unitsConsumed !== undefined) out.push(`compute       ${r.unitsConsumed} CU`);
  if (r.denied) {
    out.push(`denied by     ${moduleName(r.denied.moduleId)} at ${Callback[r.denied.callback]}: ${r.denied.reasonCode} ${describeReason(r.denied.reasonCode)}`);
  }
  if (r.trace) {
    out.push("", "hook trace");
    for (const e of r.trace.entries) {
      out.push(`  ${Callback[e.callback].padEnd(11)} ${moduleName(e.moduleId).padEnd(18)} ${e.allowed ? "ALLOW" : `DENY ${e.reasonCode} ${describeReason(e.reasonCode)}`}`);
    }
  }
  if (r.swap) {
    const s = r.swap;
    out.push(
      "",
      `swap          ${s.side === 0 ? "BUY" : "SELL"} ${units(s.baseAmount, decimals)} base for ${sol(s.quoteAmount)} (observed)`,
      `reserves      base ${s.preBaseReserves} → ${s.postBaseReserves}  quote ${s.preQuoteReserves} → ${s.postQuoteReserves}`,
      `wallet        xp ${s.xp}  pending xp ${s.pendingXp}  policy points ${s.policyPoints}  sequence ${s.sequence}`,
    );
  }
  for (const p of r.policies) out.push(`policy #${p.policyIndex}   matched=${p.matched} result=${["allow", "deny", "record", "signal"][p.result]} reason=${p.reasonCode}`);
  const other = r.events.filter((e) => !["HookExecutionEvent", "RoutedSwapEvent", "HookDeniedEvent", "PolicyExecutionEvent"].includes(e.name));
  for (const e of other) out.push(`event         ${e.name}`);
  return out.join("\n");
}

export { moduleInfo };
