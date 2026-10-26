"use client";

import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { PublicKey } from "@solana/web3.js";
import { useWallet } from "@solana/wallet-adapter-react";
import {
  Callback,
  CONFIG_SCHEMAS,
  describePolicy,
  executionResult,
  MARKET_FLAGS,
  Side,
  Venue,
  type ExecutionResult,
  type HookedMarket,
  type ModuleId,
  type PolicyAccount,
  type UserMarketState,
} from "@pump-hooks/sdk";
import { DitherStage } from "./DitherStage";
import { TracePipeline } from "./TracePipeline";
import { useNetwork } from "./Providers";
import { formatValue } from "@/lib/config-form";
import { CLUSTERS, explorerUrl } from "@/lib/cluster";
import { ago, parseSol, short, sol, units } from "@/lib/format";
import { fetchTokenMeta, type TokenMeta } from "@/lib/metadata";
import { sendBatches } from "@/lib/tx";

interface Recent {
  signature: string;
  blockTime: number | null | undefined;
  result?: ExecutionResult;
}

interface Data {
  market: HookedMarket;
  meta?: TokenMeta;
  decimals: number;
  policies: PolicyAccount[];
}

const callbacksOf = (mask: number) =>
  Array.from({ length: 12 }, (_, i) => i)
    .filter((i) => mask & (1 << i))
    .map((i) => Callback[i]);

export function MarketView({ address }: { address: string }) {
  const { client, connection, cluster, program } = useNetwork();
  const [st, setSt] = useState<{ s: "loading" } | { s: "error"; msg: string } | { s: "ok"; d: Data }>({ s: "loading" });
  const [recent, setRecent] = useState<Recent[] | "loading" | "error">("loading");

  useEffect(() => {
    let live = true;
    setSt({ s: "loading" });
    (async () => {
      const key = new PublicKey(address);
      const market = await client.getMarket(key);
      const [meta, token, policies] = await Promise.all([
        fetchTokenMeta(connection, market.config.mint),
        client.detect(market.config.mint).catch(() => undefined),
        client.getPolicies(market).catch(() => []),
      ]);
      if (live) setSt({ s: "ok", d: { market, meta, decimals: token?.decimals ?? 6, policies } });
    })().catch((e) => live && setSt({ s: "error", msg: (e as Error).message }));

    (async () => {
      const sigs = await connection.getSignaturesForAddress(new PublicKey(address), { limit: 15 }, "confirmed");
      const txs = await connection.getTransactions(
        sigs.map((s) => s.signature),
        { maxSupportedTransactionVersion: 0, commitment: "confirmed" },
      );
      if (!live) return;
      setRecent(
        sigs.map((s, i) => {
          const tx = txs[i];
          return {
            signature: s.signature,
            blockTime: s.blockTime,
            result: tx ? executionResult(tx.meta?.logMessages ?? [], tx.meta?.err, tx.meta?.computeUnitsConsumed ?? undefined) : undefined,
          };
        }),
      );
    })().catch(() => live && setRecent("error"));
    return () => {
      live = false;
    };
  }, [address, client, connection]);

  return (
    <main className="page">
      <div className="page-head">
        <DitherStage mode="banner" />
        <div className="crumb">
          Market / <b>{short(address, 6)}</b>
        </div>
        {st.s === "ok" ? (
          <>
            <h1 className="h2">{st.d.meta ? `${st.d.meta.name} (${st.d.meta.symbol})` : short(st.d.market.config.mint, 6)}</h1>
            <p className="lede mono" style={{ fontSize: 12.5, wordBreak: "break-all" }}>
              mint {st.d.market.config.mint.toBase58()}
            </p>
            <div className="tags" style={{ marginTop: 12 }}>
              <span className="tag ink">{st.d.market.config.venue === Venue.PumpSwap ? "PumpSwap" : "Pump curve"}</span>
              {st.d.market.config.flags & MARKET_FLAGS.PAUSED ? <span className="tag warn">paused</span> : <span className="tag">routing</span>}
              {st.d.market.config.flags & MARKET_FLAGS.CREATOR_VERIFIED ? <span className="tag">creator-owned</span> : <span className="tag">community market</span>}
              {st.d.market.config.flags & MARKET_FLAGS.BASE_TOKEN_2022 ? <span className="tag">Token-2022</span> : null}
            </div>
          </>
        ) : (
          <h1 className="h2">Hooked market</h1>
        )}
      </div>

      {st.s === "loading" && <div className="skeleton" style={{ margin: "28px 0" }} />}
      {st.s === "error" && (
        <div className="notice deny" style={{ margin: "28px 0" }}>
          {program.state !== "live" ? `Pump Hooks isn't deployed on ${CLUSTERS[cluster].label}, so there are no markets here. ` : ""}
          {st.msg}
        </div>
      )}
      {st.s === "ok" && <Loaded d={st.d} recent={recent} />}
    </main>
  );
}

function Loaded({ d, recent }: { d: Data; recent: Recent[] | "loading" | "error" }) {
  const { cluster } = useNetwork();
  const c = d.market.config;
  return (
    <div className="grid-main">
      <div className="col">
        <section className="section">
          <div className="section-h">
            <span className="n">01</span>
            <h3>Hook stack</h3>
            <span className="aside">{c.slots.length} installed · execution order</span>
          </div>
          {c.slots.length === 0 && <p className="mute">No hooks installed. Routed trades pass straight through to Pump.</p>}
          {c.slots.map((s) => {
            const schema = CONFIG_SCHEMAS[s.moduleId as ModuleId] ?? [];
            return (
              <details key={s.index} className="slot">
                <summary>
                  <span className="mono">{String(s.priority).padStart(4, "0")}</span> · {s.module?.name ?? `module ${s.moduleId}`} · v{s.version}
                  <span className={s.enabled ? "ok" : "deny"} style={{ float: "right" }}>
                    {s.enabled ? "enabled" : "disabled"}
                  </span>
                </summary>
                <p className="faint mono" style={{ fontSize: 11 }}>
                  {callbacksOf(s.callbacks).join(" · ")}
                </p>
                {schema.length > 0 && (
                  <table className="table">
                    <tbody>
                      {schema.map((f) => (
                        <tr key={f.name}>
                          <td style={{ width: "40%" }}>{f.name}</td>
                          <td>{formatValue(s.moduleId as ModuleId, f, s.decodedConfig[f.name], d.decimals)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                )}
              </details>
            );
          })}
        </section>

        <section className="section">
          <div className="section-h">
            <span className="n">02</span>
            <h3>Policies</h3>
            <span className="aside">{d.policies.length} / 4</span>
          </div>
          {d.policies.length === 0 && <p className="mute">No policies. Compose one in the <Link className="link-u" href={`/build?market=${d.market.address.toBase58()}`}>Composer</Link>.</p>}
          {d.policies.map((p) => (
            <div key={p.index} className="code" style={{ marginBottom: 8 }}>
              <span className="faint">#{p.index} {p.enabled ? "" : "(disabled) "}· {p.execCount.toString()} actions · </span>
              <span className="hl">{describePolicy(p)}</span>
            </div>
          ))}
        </section>

        <section className="section">
          <div className="section-h">
            <span className="n">03</span>
            <h3>Recent transactions</h3>
            <span className="aside">from chain, last 15</span>
          </div>
          {recent === "loading" && <div className="skeleton" />}
          {recent === "error" && <p className="mute">Could not load signatures from this RPC.</p>}
          {Array.isArray(recent) && recent.length === 0 && <p className="mute">No transactions yet.</p>}
          {Array.isArray(recent) && recent.length > 0 && (
            <table className="table">
              <thead>
                <tr>
                  <th>When</th>
                  <th>Kind</th>
                  <th>Amount</th>
                  <th>Hooks</th>
                  <th>Result</th>
                </tr>
              </thead>
              <tbody>
                {recent.map((r) => {
                  const res = r.result;
                  const swap = res?.swap;
                  const kind = swap ? (swap.side === Side.Buy ? "buy" : "sell") : res?.denied ? "denied trade" : (res?.events[0]?.name ?? "—");
                  return (
                    <tr key={r.signature}>
                      <td>
                        <Link href={`/tx/${r.signature}`}>{ago(r.blockTime)}</Link>
                      </td>
                      <td>{kind}</td>
                      <td>{swap ? `${sol(swap.quoteAmount)} SOL` : "—"}</td>
                      <td>{res?.trace?.entries.length ?? "—"}</td>
                      <td className={res?.ok ? "ok" : "deny"}>{res ? (res.ok ? "ok" : (res.error?.name ?? "failed")) : "?"}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </section>
      </div>

      <div className="col" style={{ position: "sticky", top: "var(--header)" }}>
        <section className="section">
          <div className="section-h">
            <span className="n">04</span>
            <h3>Routed flow</h3>
          </div>
          <p className="faint" style={{ fontSize: 12, marginTop: 0 }}>Counts only trades routed through Pump Hooks, not all trading in this token.</p>
          <div className="kv">
            <div>
              <div className="k">Routed buys</div>
              <div className="v">{sol(c.routedBuyQuote)} SOL</div>
            </div>
            <div>
              <div className="k">Routed sells</div>
              <div className="v">{sol(c.routedSellQuote)} SOL</div>
            </div>
            <div>
              <div className="k">Trades</div>
              <div className="v">{c.tradeCount.toString()}</div>
            </div>
            <div>
              <div className="k">Activated</div>
              <div className="v">{c.activatedAt > 0n ? ago(Number(c.activatedAt)) : "—"}</div>
            </div>
            <div>
              <div className="k">Authority</div>
              <div className="v">
                <a href={explorerUrl(cluster, "address", c.authority.toBase58())} target="_blank" rel="noreferrer">
                  {short(c.authority, 5)}
                </a>
              </div>
            </div>
            <div>
              <div className="k">Venue</div>
              <div className="v">
                <a href={explorerUrl(cluster, "address", (c.venue === Venue.PumpSwap ? c.pool : c.bondingCurve).toBase58())} target="_blank" rel="noreferrer">
                  {short(c.venue === Venue.PumpSwap ? c.pool : c.bondingCurve, 5)}
                </a>
              </div>
            </div>
          </div>
        </section>
        <TradePanel d={d} />
      </div>
    </div>
  );
}

/** A buy reverts if it would return less than the simulated amount minus this. */
const SLIPPAGE_BPS = 500n;

function TradePanel({ d }: { d: Data }) {
  const { client, connection, writesEnabled } = useNetwork();
  const wallet = useWallet();
  const [amt, setAmt] = useState("0.1");
  const [me, setMe] = useState<UserMarketState | undefined>();
  const [sim, setSim] = useState<{ s: "idle" | "loading" } | { s: "done"; result: ExecutionResult } | { s: "error"; msg: string }>({ s: "idle" });
  const [sent, setSent] = useState<{ s: "idle" | "loading" } | { s: "done"; sig: string; ok: boolean } | { s: "error"; msg: string }>({ s: "idle" });
  const quoteIn = parseSol(amt);
  const paused = (d.market.config.flags & MARKET_FLAGS.PAUSED) !== 0;

  const loadMe = useCallback(() => {
    if (!wallet.publicKey) return setMe(undefined);
    client.getUserState(d.market.address, wallet.publicKey).then(setMe, () => setMe(undefined));
  }, [client, d.market.address, wallet.publicKey]);
  useEffect(loadMe, [loadMe]);

  const simulate = async () => {
    if (!wallet.publicKey || !quoteIn) return;
    setSim({ s: "loading" });
    setSent({ s: "idle" });
    try {
      const result = await client.simulate(await client.buy({ market: d.market, user: wallet.publicKey, quoteIn }), wallet.publicKey);
      setSim({ s: "done", result });
    } catch (e) {
      setSim({ s: "error", msg: (e as Error).message });
    }
  };

  const minBaseOut = sim.s === "done" && sim.result.swap ? (sim.result.swap.baseAmount * (10_000n - SLIPPAGE_BPS)) / 10_000n : undefined;

  const send = async () => {
    if (!wallet.publicKey || !quoteIn || !minBaseOut) return;
    setSent({ s: "loading" });
    try {
      const built = await client.buy({ market: d.market, user: wallet.publicKey, quoteIn, minBaseOut });
      const [r] = await sendBatches(connection, wallet, [built.instructions], await client.loadLookupTables());
      setSent({ s: "done", sig: r.signature, ok: r.result.ok });
      loadMe();
    } catch (e) {
      setSent({ s: "error", msg: (e as Error).message });
    }
  };

  return (
    <section className="section">
      <div className="section-h">
        <span className="n">05</span>
        <h3>Routed buy</h3>
      </div>
      {me && (
        <div className="kv" style={{ marginBottom: 12 }}>
          <div>
            <div className="k">Your XP</div>
            <div className="v">
              {me.state.xp.toString()}
              {me.state.pendingXp > 0n ? ` (+${me.state.pendingXp} vesting)` : ""}
            </div>
          </div>
          <div>
            <div className="k">Policy points</div>
            <div className="v">{me.state.policyPoints.toString()}</div>
          </div>
          <div>
            <div className="k">Your routed buys</div>
            <div className="v">{sol(me.state.routedBuyQuote)} SOL</div>
          </div>
          <div>
            <div className="k">Your trades</div>
            <div className="v">{me.state.tradeCount}</div>
          </div>
        </div>
      )}
      <div className="input-unit">
        <input
          className="input"
          value={amt}
          onChange={(e) => {
            setAmt(e.target.value);
            setSim({ s: "idle" });
          }}
          inputMode="decimal"
        />
        <span>SOL</span>
      </div>
      <div className="grid-2" style={{ gap: 8, marginTop: 10 }}>
        <button className="btn outline" disabled={!wallet.publicKey || !quoteIn || paused || sim.s === "loading"} onClick={simulate}>
          {sim.s === "loading" ? <span className="spin" /> : "Simulate"}
        </button>
        <button
          className="btn"
          disabled={!wallet.publicKey || !quoteIn || paused || !writesEnabled || !minBaseOut || sim.s !== "done" || !sim.result.ok || sent.s === "loading"}
          onClick={send}
        >
          {sent.s === "loading" ? <span className="spin" /> : "Buy through hooks"}
        </button>
      </div>
      {!wallet.publicKey && <p className="faint" style={{ fontSize: 12 }}>Connect a wallet to simulate against live state.</p>}
      {paused && <p className="faint" style={{ fontSize: 12 }}>This market is paused by its authority.</p>}
      {sim.s === "error" && <div className="notice deny" style={{ marginTop: 10 }}>{sim.msg}</div>}
      {sim.s === "done" && (
        <div style={{ marginTop: 14 }}>
          <TracePipeline result={sim.result} user={wallet.publicKey ?? undefined} venue={d.market.config.venue} decimals={d.decimals} symbol={d.meta?.symbol} />
          {sim.result.swap && (
            <p className="faint mono" style={{ fontSize: 11 }}>
              simulated: {units(sim.result.swap.baseAmount, d.decimals)} {d.meta?.symbol ?? "tokens"} for {sol(sim.result.swap.quoteAmount)} SOL (incl. Pump fees)
              {minBaseOut ? ` · buy reverts below ${units(minBaseOut, d.decimals)} (${Number(SLIPPAGE_BPS) / 100}% slippage)` : ""}
            </p>
          )}
        </div>
      )}
      {sent.s === "error" && <div className="notice deny" style={{ marginTop: 10 }}>{sent.msg}</div>}
      {sent.s === "done" && (
        <div className={`notice ${sent.ok ? "ok" : "deny"}`} style={{ marginTop: 10 }}>
          {sent.ok ? "Confirmed." : "Failed."}{" "}
          <Link className="link-u" href={`/tx/${sent.sig}`}>
            View execution trace
          </Link>
        </div>
      )}
    </section>
  );
}
