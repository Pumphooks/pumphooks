"use client";

import { Callback, describeReason, moduleInfo, Side, Venue, type ExecutionResult, type TraceItem } from "@pump-hooks/sdk";
import type { PublicKey } from "@solana/web3.js";
import { short, sol, units } from "@/lib/format";

const BEFORE = new Set([Callback.BeforeBuy, Callback.BeforeSell, Callback.BeforeSwap]);
const name = (id: number) => {
  try {
    return moduleInfo(id).name;
  } catch {
    return `module ${id}`;
  }
};

/**
 * Wallet → Pump Hooks → before hooks → Pump CPI → after hooks → settlement, decoded from the
 * transaction's own logs. Nothing here is inferred: rows exist only for events the program emitted.
 */
export function TracePipeline({
  result,
  user,
  venue,
  decimals = 6,
  symbol = "tokens",
}: {
  result: ExecutionResult;
  user?: PublicKey;
  venue?: Venue;
  decimals?: number;
  symbol?: string;
}) {
  const entries = result.trace?.entries ?? [];
  const before = entries.filter((e) => BEFORE.has(e.callback));
  const after = entries.filter((e) => !BEFORE.has(e.callback));
  const denied = result.denied;
  const swap = result.swap;
  const v = swap?.venue ?? venue;
  const cpi = v === Venue.PumpSwap ? "PumpSwap · buy/sell v2" : v === Venue.BondingCurve ? "Pump curve · buy/sell v3" : "Pump";
  const reached = !!swap;
  let i = 0;
  const idx = () => String(++i).padStart(2, "0");

  return (
    <div className="pipeline reveal">
      <Row i={idx()} stage="Wallet" body={user ? short(user, 6) : "signer"} r="signs" />
      <Row i={idx()} stage="Pump Hooks" body={swap ? (swap.side === Side.Buy ? "route · buy" : "route · sell") : "route"} r={result.unitsConsumed ? `${result.unitsConsumed.toLocaleString()} CU` : ""} />
      {before.map((e, k) => (
        <HookRow key={`b${k}`} i={idx()} e={e} />
      ))}
      {denied && !before.some((e) => !e.allowed) && (
        <div className="pipe-row hook denied">
          <span className="i">{idx()}</span>
          <span className="stage-n">{Callback[denied.callback]}</span>
          <span>{name(denied.moduleId)} — {describeReason(denied.reasonCode)}</span>
          <span className="r deny">DENY {denied.reasonCode}</span>
        </div>
      )}
      <div className={`pipe-row cpi${reached ? "" : " skipped"}`}>
        <span className="i">{idx()}</span>
        <span className="stage-n">CPI</span>
        <span>{cpi}</span>
        <span className="r">{reached ? "executed" : "not reached"}</span>
      </div>
      {after.map((e, k) => (
        <HookRow key={`a${k}`} i={idx()} e={e} />
      ))}
      {result.policies.map((p, k) => (
        <div className={`pipe-row hook${p.result === 1 ? " denied" : ""}`} key={`p${k}`}>
          <span className="i">{idx()}</span>
          <span className="stage-n">Policy #{p.policyIndex}</span>
          <span>{p.matched ? "matched" : "no match"}</span>
          <span className={`r ${p.result === 1 ? "deny" : "mute"}`}>{["allow", "deny", "record", "signal"][p.result] ?? p.result}</span>
        </div>
      ))}
      <div className="pipe-row">
        <span className="i">{idx()}</span>
        <span className="stage-n">Settlement</span>
        <span>
          {swap ? (
            <>
              {swap.side === Side.Buy ? "paid" : "received"} <b>{sol(swap.quoteAmount)} SOL</b> · {swap.side === Side.Buy ? "received" : "sold"}{" "}
              <b>
                {units(swap.baseAmount, decimals)} {symbol}
              </b>
              {swap.pendingXp > 0n ? ` · +${swap.pendingXp} XP vesting` : ""}
              {swap.policyPoints > 0n ? ` · +${swap.policyPoints} pts` : ""}
            </>
          ) : result.ok ? (
            "no routed swap in this transaction"
          ) : (
            <span className="deny">reverted — nothing moved{result.error ? ` (${result.error.name})` : ""}</span>
          )}
        </span>
        <span className={`r ${result.ok ? "ok" : "deny"}`}>{result.ok ? "OK" : "REVERTED"}</span>
      </div>
    </div>
  );
}

function Row({ i, stage, body, r }: { i: string; stage: string; body: React.ReactNode; r?: React.ReactNode }) {
  return (
    <div className="pipe-row">
      <span className="i">{i}</span>
      <span className="stage-n">{stage}</span>
      <span>{body}</span>
      <span className="r mute">{r}</span>
    </div>
  );
}

function HookRow({ i, e }: { i: string; e: TraceItem }) {
  return (
    <div className={`pipe-row hook${e.allowed ? "" : " denied"}`}>
      <span className="i">{i}</span>
      <span className="stage-n">{Callback[e.callback]}</span>
      <span>
        {name(e.moduleId)}
        {!e.allowed ? <span className="deny"> — {describeReason(e.reasonCode)}</span> : null}
      </span>
      <span className={`r ${e.allowed ? "ok" : "deny"}`}>{e.allowed ? "ALLOW" : `DENY ${e.reasonCode}`}</span>
    </div>
  );
}
