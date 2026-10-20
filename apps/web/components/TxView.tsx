"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { PublicKey } from "@solana/web3.js";
import { executionResult, PUMP_HOOKS_PROGRAM_ID, type ExecutionResult } from "@pump-hooks/sdk";
import { DitherStage } from "./DitherStage";
import { TracePipeline } from "./TracePipeline";
import { useNetwork } from "./Providers";
import { CLUSTERS, explorerUrl } from "@/lib/cluster";
import { ago, short, sol } from "@/lib/format";

interface Loaded {
  result: ExecutionResult;
  slot: number;
  blockTime: number | null | undefined;
  fee: number;
  signer: PublicKey;
  routed: boolean;
  decimals?: number;
}

type State = { s: "loading" } | { s: "notfound" } | { s: "error"; msg: string } | { s: "ok"; tx: Loaded };

/** JSON for decoded events: keys as base58, u64 as decimal strings, bytes as hex. */
export const eventJson = (data: unknown) =>
  JSON.stringify(
    data,
    (_, v) => {
      if (typeof v === "bigint") return v.toString();
      if (v instanceof PublicKey) return v.toBase58();
      if (v instanceof Uint8Array) return Array.from(v, (b) => b.toString(16).padStart(2, "0")).join("");
      return v;
    },
    2,
  );

export function TxView({ signature }: { signature: string }) {
  const { connection, cluster } = useNetwork();
  const [st, setSt] = useState<State>({ s: "loading" });

  useEffect(() => {
    let live = true;
    setSt({ s: "loading" });
    (async () => {
      const tx = await connection.getTransaction(signature, { maxSupportedTransactionVersion: 0, commitment: "confirmed" });
      if (!live) return;
      if (!tx) return setSt({ s: "notfound" });
      const keys = tx.transaction.message.getAccountKeys({ accountKeysFromLookups: tx.meta?.loadedAddresses });
      const routed = keys.keySegments().flat().some((k) => k.equals(PUMP_HOOKS_PROGRAM_ID));
      const result = executionResult(tx.meta?.logMessages ?? [], tx.meta?.err, tx.meta?.computeUnitsConsumed ?? undefined);
      let decimals: number | undefined;
      if (result.swap) {
        const info = await connection.getParsedAccountInfo(result.swap.mint).catch(() => null);
        const parsed = info?.value?.data;
        if (parsed && "parsed" in parsed) decimals = parsed.parsed?.info?.decimals;
      }
      if (live) setSt({ s: "ok", tx: { result, slot: tx.slot, blockTime: tx.blockTime, fee: tx.meta?.fee ?? 0, signer: keys.get(0)!, routed, decimals } });
    })().catch((e) => live && setSt({ s: "error", msg: (e as Error).message }));
    return () => {
      live = false;
    };
  }, [connection, signature]);

  return (
    <main className="page">
      <div className="page-head">
        <DitherStage mode="banner" />
        <div className="crumb">
          Explorer / <b>transaction</b>
        </div>
        <h1 className="h2">Execution trace</h1>
        <p className="lede mono" style={{ fontSize: 13, wordBreak: "break-all" }}>
          {signature}
        </p>
        <div className="btn-row" style={{ marginTop: 12 }}>
          <a className="link-u" href={explorerUrl(cluster, "tx", signature)} target="_blank" rel="noreferrer">
            Solana Explorer ↗
          </a>
        </div>
      </div>

      {st.s === "loading" && <div className="skeleton" style={{ margin: "28px 0" }} />}
      {st.s === "notfound" && (
        <div className="notice warn" style={{ margin: "28px 0" }}>
          Not found on {CLUSTERS[cluster].label} at confirmed commitment. Check the cluster switch in the header.
        </div>
      )}
      {st.s === "error" && <div className="notice deny" style={{ margin: "28px 0" }}>{st.msg}</div>}
      {st.s === "ok" && <Loaded tx={st.tx} cluster={cluster} />}
    </main>
  );
}

function Loaded({ tx, cluster }: { tx: Loaded; cluster: keyof typeof CLUSTERS }) {
  const { result } = tx;
  const market = result.swap?.market ?? result.trace?.market ?? result.denied?.market;
  return (
    <div className="grid-main">
      <div className="col">
        <section className="section">
          <div className="section-h">
            <span className="n">01</span>
            <h3>Pipeline</h3>
            <span className={`aside ${result.ok ? "ok" : "deny"}`}>{result.ok ? "confirmed" : "failed"}</span>
          </div>
          {!tx.routed ? (
            <div className="notice">
              This transaction did not invoke Pump Hooks. Trades sent straight to Pump or PumpSwap bypass the router, so no hooks ran.
            </div>
          ) : result.trace || result.swap || result.denied ? (
            <TracePipeline result={result} user={tx.signer} decimals={tx.decimals} />
          ) : (
            <div className="notice">
              Pump Hooks was invoked, but not for a routed trade{result.events.length ? ` (${result.events.map((e) => e.name).join(", ")})` : ""}.
              {!result.ok && result.error ? ` Failed: ${result.error.name} — ${result.error.msg}` : ""}
            </div>
          )}
          {!result.ok && result.error && (result.trace || result.denied) && (
            <div className="notice deny" style={{ marginTop: 12 }}>
              {result.error.name} ({result.error.code}): {result.error.msg}. The whole transaction reverted; events are reconstructed from its logs.
            </div>
          )}
        </section>

        <section className="section">
          <div className="section-h">
            <span className="n">02</span>
            <h3>Events</h3>
            <span className="aside">{result.events.length} decoded</span>
          </div>
          {result.events.length === 0 && <p className="mute">No Pump Hooks events in the logs.</p>}
          {result.events.map((e, i) => (
            <details key={i} open={e.name === "RoutedSwapEvent"}>
              <summary>
                {String(i + 1).padStart(2, "0")} · {e.name}
              </summary>
              <div className="code" style={{ fontSize: 11 }}>
                {eventJson(e.data)}
              </div>
            </details>
          ))}
        </section>

        <section className="section">
          <div className="section-h">
            <span className="n">03</span>
            <h3>Logs</h3>
            <span className="aside">{result.logs.length} lines</span>
          </div>
          <details>
            <summary>Program logs</summary>
            <div className="code" style={{ fontSize: 11, maxHeight: 520, overflow: "auto" }}>
              {result.logs.join("\n")}
            </div>
          </details>
        </section>
      </div>

      <div className="col" style={{ position: "sticky", top: "var(--header)" }}>
        <section className="section">
          <div className="section-h">
            <span className="n">04</span>
            <h3>Summary</h3>
          </div>
          <div className="kv">
            <div>
              <div className="k">Status</div>
              <div className={`v ${result.ok ? "ok" : "deny"}`}>{result.ok ? "success" : (result.error?.name ?? "failed")}</div>
            </div>
            <div>
              <div className="k">Slot</div>
              <div className="v">{tx.slot.toLocaleString()}</div>
            </div>
            <div>
              <div className="k">Time</div>
              <div className="v">{tx.blockTime ? ago(tx.blockTime) : "—"}</div>
            </div>
            <div>
              <div className="k">Compute</div>
              <div className="v">{result.unitsConsumed?.toLocaleString() ?? "—"} CU</div>
            </div>
            <div>
              <div className="k">Fee</div>
              <div className="v">{sol(BigInt(tx.fee), 6)} SOL</div>
            </div>
            <div>
              <div className="k">Signer</div>
              <div className="v">
                <a href={explorerUrl(cluster, "address", tx.signer.toBase58())} target="_blank" rel="noreferrer">
                  {short(tx.signer, 5)}
                </a>
              </div>
            </div>
            <div>
              <div className="k">Hooks run</div>
              <div className="v">{result.trace?.entries.length ?? 0}</div>
            </div>
            <div>
              <div className="k">Policies run</div>
              <div className="v">{result.policies.length}</div>
            </div>
            {market && (
              <div style={{ gridColumn: "1 / -1" }}>
                <div className="k">Market</div>
                <div className="v">
                  <Link href={`/market/${market.toBase58()}`}>{market.toBase58()}</Link>
                </div>
              </div>
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
