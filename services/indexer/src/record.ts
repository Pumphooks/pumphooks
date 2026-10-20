import { Callback, eventsOf, executionResult, PUMP_HOOKS_PROGRAM_ID, Side, type ExecutionResult } from "@pump-hooks/sdk";
import type { PublicKey } from "@solana/web3.js";

/**
 * Everything the indexer keeps about one transaction, as plain JSON (keys base58, u64 as decimal
 * strings). Derived only from the transaction's own logs and error, so a record can be rebuilt by
 * re-fetching the transaction.
 */
export interface TxRecord {
  signature: string;
  slot: number;
  blockTime: number | null;
  ok: boolean;
  error?: { code: number; name: string };
  unitsConsumed?: number;
  swap?: {
    market: string;
    user: string;
    mint: string;
    side: "buy" | "sell";
    quote: string;
    base: string;
    hooksRun: number;
    pendingXp: string;
    policyPoints: string;
  };
  hooks: { market: string; moduleId: number; callback: string; allowed: boolean; reasonCode: number }[];
  denied?: { market: string; user: string; moduleId: number; callback: string; reasonCode: number };
  policies: { market: string; index: number; matched: boolean; result: number }[];
  /** Only from successful transactions. */
  installs: { market: string; moduleId: number; delta: 1 | -1 }[];
  /** Only from successful transactions. */
  marketsCreated: { market: string; mint: string; authority: string }[];
  events: string[];
}

const b58 = (k: PublicKey) => k.toBase58();

export function toRecord(
  signature: string,
  tx: { slot: number; blockTime?: number | null; meta: { logMessages?: string[] | null; err?: unknown; computeUnitsConsumed?: number } | null },
  programId: PublicKey = PUMP_HOOKS_PROGRAM_ID,
): TxRecord {
  const r: ExecutionResult = executionResult(tx.meta?.logMessages ?? [], tx.meta?.err ?? null, tx.meta?.computeUnitsConsumed, programId);
  const ok = r.ok;
  const s = r.swap;
  return {
    signature,
    slot: tx.slot,
    blockTime: tx.blockTime ?? null,
    ok,
    error: r.error ? { code: r.error.code, name: r.error.name } : undefined,
    unitsConsumed: r.unitsConsumed,
    swap:
      ok && s
        ? {
            market: b58(s.market),
            user: b58(s.user),
            mint: b58(s.mint),
            side: s.side === Side.Buy ? "buy" : "sell",
            quote: s.quoteAmount.toString(),
            base: s.baseAmount.toString(),
            hooksRun: s.hooksRun,
            pendingXp: s.pendingXp.toString(),
            policyPoints: s.policyPoints.toString(),
          }
        : undefined,
    hooks: r.trace
      ? r.trace.entries.map((e) => ({ market: b58(r.trace!.market), moduleId: e.moduleId, callback: Callback[e.callback], allowed: e.allowed, reasonCode: e.reasonCode }))
      : [],
    denied: r.denied
      ? { market: b58(r.denied.market), user: b58(r.denied.user), moduleId: r.denied.moduleId, callback: Callback[r.denied.callback], reasonCode: r.denied.reasonCode }
      : undefined,
    policies: r.policies.map((p) => ({ market: b58(p.market), index: p.policyIndex, matched: p.matched, result: p.result })),
    installs: ok
      ? [
          ...eventsOf(r.events, "HookInstalled").map((e) => ({ market: b58(e.market), moduleId: e.moduleId, delta: 1 as const })),
          ...eventsOf(r.events, "HookRemoved").map((e) => ({ market: b58(e.market), moduleId: e.moduleId, delta: -1 as const })),
        ]
      : [],
    marketsCreated: ok ? eventsOf(r.events, "MarketCreated").map((e) => ({ market: b58(e.market), mint: b58(e.mint), authority: b58(e.authority) })) : [],
    events: r.events.map((e) => e.name),
  };
}
