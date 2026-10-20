"use client";

import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { PublicKey } from "@solana/web3.js";
import { useWallet } from "@solana/wallet-adapter-react";
import {
  Callback,
  callbacksOf,
  MAX_HOOK_SLOTS,
  MAX_RUNTIME_HOOKS,
  ModuleId,
  NATIVE_MINT,
  Venue,
  type ExecutionResult,
  type HookedMarket,
  type PumpTokenInfo,
} from "@pump-hooks/sdk";
import { useNetwork, isPubkey } from "./Providers";
import { DitherStage } from "./DitherStage";
import { ConfigEditor } from "./ConfigEditor";
import { TracePipeline } from "./TracePipeline";
import { CATALOG, CATEGORIES, CATEGORY_NOTES, type Category, type CatalogEntry } from "@/lib/catalog";
import { parseValues, presetValues, type FormValues } from "@/lib/config-form";
import { explorerUrl, CLUSTERS } from "@/lib/cluster";
import { fetchTokenMeta, type TokenMeta } from "@/lib/metadata";
import { parseSol, short, sol, units } from "@/lib/format";
import { pack, sendBatches, type SentTx } from "@/lib/tx";

interface Detected {
  token: PumpTokenInfo;
  meta?: TokenMeta;
  markets?: HookedMarket[];
  marketsError?: string;
}

type Async<T> = { status: "idle" } | { status: "loading" } | { status: "ok"; data: T } | { status: "error"; error: string };

const BEFORE = new Set([Callback.BeforeBuy, Callback.BeforeSell, Callback.BeforeSwap]);

export function HookFlow() {
  const params = useSearchParams();
  const router = useRouter();
  const { client, connection, cluster, program, writesEnabled } = useNetwork();
  const wallet = useWallet();

  const [input, setInput] = useState(params.get("mint") ?? "");
  const [mint, setMint] = useState<PublicKey | undefined>(() => (isPubkey(params.get("mint") ?? "") ? new PublicKey(params.get("mint")!) : undefined));
  const [det, setDet] = useState<Async<Detected>>({ status: "idle" });
  const [cats, setCats] = useState<Set<Category>>(new Set());
  const [selected, setSelected] = useState<string[]>([]);
  const [forms, setForms] = useState<Record<string, FormValues>>({});
  const [testBuy, setTestBuy] = useState("0.05");
  const [sim, setSim] = useState<Async<{ result: ExecutionResult; mode: "preview" | "existing" }>>({ status: "idle" });
  const [act, setAct] = useState<Async<{ sent: SentTx[]; market: PublicKey }> & { step?: string }>({ status: "idle" });

  const detect = useCallback(
    async (m: PublicKey) => {
      setDet({ status: "loading" });
      setSim({ status: "idle" });
      setAct({ status: "idle" });
      try {
        const [token, meta] = await Promise.all([client.detect(m), fetchTokenMeta(connection, m)]);
        let markets: HookedMarket[] | undefined;
        let marketsError: string | undefined;
        try {
          markets = program.state === "live" ? await client.findMarkets(m) : [];
        } catch {
          marketsError = "This RPC does not allow program-account scans; existing markets for other namespaces are not listed.";
        }
        setDet({ status: "ok", data: { token, meta, markets, marketsError } });
      } catch (e) {
        setDet({ status: "error", error: (e as Error).message });
      }
    },
    [client, connection, program.state],
  );

  useEffect(() => {
    if (mint) detect(mint);
  }, [mint, detect]);

  const token = det.status === "ok" ? det.data.token : undefined;
  const meta = det.status === "ok" ? det.data.meta : undefined;
  const symbol = meta?.symbol || "tokens";
  const decimals = token?.decimals ?? 6;
  const mine = useMemo(() => {
    if (det.status !== "ok" || !wallet.publicKey) return undefined;
    return det.data.markets?.find((m) => m.config.authority.equals(wallet.publicKey!) && m.config.namespace.equals(wallet.publicKey!));
  }, [det, wallet.publicKey]);

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!isPubkey(input)) return;
    const k = new PublicKey(input.trim());
    router.replace(`/hook?mint=${k.toBase58()}`, { scroll: false });
    setMint(k);
  };

  const entries = selected.map((s) => CATALOG.find((c) => c.slug === s)!).sort((a, b) => a.info.defaultPriority - b.info.defaultPriority);
  const runtimeCount = entries.filter((e) => e.runtime).length;

  const toggle = (c: CatalogEntry) => {
    setSim({ status: "idle" });
    if (selected.includes(c.slug)) return setSelected(selected.filter((s) => s !== c.slug));
    setSelected([...selected, c.slug]);
    if (!forms[c.slug]) setForms({ ...forms, [c.slug]: presetValues(c.moduleId as ModuleId, decimals) });
  };

  const parsed = Object.fromEntries(entries.map((e) => [e.slug, parseValues(e.moduleId as ModuleId, forms[e.slug] ?? {}, decimals)]));
  const configsValid = entries.every((e) => Object.keys(parsed[e.slug].fieldErrors).length === 0 && parsed[e.slug].problems.length === 0);
  const stackHooks = entries.map((e) => ({ module: e.moduleId as ModuleId, config: parsed[e.slug].config }));

  const live = program.state === "live";
  const canSim = !!token?.routable && live && !!wallet.publicKey && configsValid && entries.length > 0;

  const simulate = async () => {
    if (!token || !wallet.publicKey) return;
    const quoteIn = parseSol(testBuy);
    if (!quoteIn || quoteIn <= 0n) return setSim({ status: "error", error: "Enter a test buy amount in SOL" });
    setSim({ status: "loading" });
    try {
      if (mine) {
        const result = await client.simulate(await client.buy({ market: mine, user: wallet.publicKey, quoteIn }), wallet.publicKey);
        setSim({ status: "ok", data: { result, mode: "existing" } });
      } else {
        const built = await client.previewStack({ mint: token.mint, authority: wallet.publicKey, hooks: stackHooks, quoteIn });
        const result = await client.simulate(built, wallet.publicKey);
        setSim({ status: "ok", data: { result, mode: "preview" } });
      }
    } catch (e) {
      setSim({ status: "error", error: (e as Error).message });
    }
  };

  const activate = async () => {
    if (!token || !wallet.publicKey) return;
    setAct({ status: "loading", step: "Building transactions" });
    try {
      const authority = wallet.publicKey;
      const { market, ix } = await client.createMarket(token.mint, authority);
      const installs = stackHooks.map((h) => client.installHook({ ...h, market, authority }));
      const alts = await client.loadLookupTables();
      const batches = pack([ix, ...installs], authority, alts);
      const sent = await sendBatches(connection, wallet, batches, alts, (i, sig) =>
        setAct({ status: "loading", step: sig ? `Confirming ${i + 1}/${batches.length}` : `Waiting for wallet ${i + 1}/${batches.length}` }),
      );
      const failed = sent.find((s) => !s.result.ok);
      if (failed) throw new Error(failed.result.error ? `${failed.result.error.name}: ${failed.result.error.msg}` : "transaction failed");
      setAct({ status: "ok", data: { sent, market } });
    } catch (e) {
      setAct({ status: "error", error: (e as Error).message });
    }
  };

  const visible = CATALOG.filter((c) => cats.size === 0 || c.categories.some((k) => cats.has(k)));

  return (
    <main className="page">
      <div className="page-head">
        <DitherStage mode="banner" />
        <div className="crumb">
          <b>01</b> / Hook a token
        </div>
        <h1 className="h2">Program an existing Pump market.</h1>
        <p className="lede" style={{ marginBottom: 20 }}>
          Paste a contract address. Detect what&apos;s already there, choose behavior, simulate it against the live market, then
          activate it — the token keeps trading on Pump.
        </p>
        <form className="ca-box" style={{ marginTop: 0 }} onSubmit={submit}>
          <input value={input} onChange={(e) => setInput(e.target.value)} placeholder="Token mint address (CA)" spellCheck={false} autoComplete="off" aria-label="Token mint address" />
          <button type="submit" disabled={!isPubkey(input)} aria-label="Detect">
            →
          </button>
        </form>
        <p className="mute" style={{ fontSize: 13, marginTop: 12 }}>
          Don&apos;t have one yet?{" "}
          <a className="link-u" href="https://pump.fun/create" target="_blank" rel="noreferrer">
            Create one through Pump
          </a>{" "}
          and come back with its address.
        </p>
      </div>

      <div className="grid-main">
        <div className="col">
          <Section n="01" title="Detect" aside={det.status === "ok" ? CLUSTERS[cluster].label : undefined}>
            {det.status === "idle" && <p className="mute">Enter a Pump token&apos;s mint address to begin.</p>}
            {det.status === "loading" && <Skeleton />}
            {det.status === "error" && <div className="notice deny">{det.error}</div>}
            {det.status === "ok" && <Detection d={det.data} cluster={cluster} symbol={symbol} />}
          </Section>

          <Section n="02" title="Categories" locked={!token}>
            <div className="chips">
              {CATEGORIES.map((c) => {
                const n = CATALOG.filter((x) => x.categories.includes(c) && x.available).length;
                const on = cats.has(c);
                return (
                  <button
                    key={c}
                    className="chip"
                    aria-pressed={on}
                    onClick={() => {
                      const s = new Set(cats);
                      if (on) s.delete(c);
                      else s.add(c);
                      setCats(s);
                    }}
                  >
                    {c}
                    <span className="c">{n}</span>
                  </button>
                );
              })}
            </div>
            {[...cats].map((c) => CATEGORY_NOTES[c] && <div key={c} className="notice" style={{ marginTop: 12 }}><b>{c}.</b> {CATEGORY_NOTES[c]}</div>)}
          </Section>

          <Section n="03" title="Stack" locked={!token} aside={`${entries.length}/${MAX_HOOK_SLOTS} slots · ${runtimeCount}/${MAX_RUNTIME_HOOKS} runtime`}>
            <div className="module-list">
              {visible.map((c, i) => {
                const on = selected.includes(c.slug);
                const full = !on && (entries.length >= MAX_HOOK_SLOTS || (c.runtime && runtimeCount >= MAX_RUNTIME_HOOKS));
                const disabled = !c.available || full || !!mine;
                return (
                  <div key={c.slug} className={`module${on ? " on" : ""}${!c.available ? " disabled" : ""}`}>
                    <span className="idx">{String(i + 1).padStart(2, "0")}</span>
                    <div>
                      <div className="name">
                        <Link href={`/hooks/${c.slug}`}>{c.name}</Link>
                        <span className="tags">
                          {c.categories.map((k) => (
                            <span className="tag" key={k}>{k}</span>
                          ))}
                          {!c.runtime && <span className="tag">config-only</span>}
                          {!c.available && <span className="tag warn">reserved</span>}
                        </span>
                      </div>
                      <div className="desc">{c.summary}</div>
                    </div>
                    <button
                      className="toggle"
                      aria-pressed={on}
                      aria-label={`${on ? "Remove" : "Add"} ${c.name}`}
                      disabled={disabled}
                      title={!c.available ? "Not available in this release" : full ? "Stack limit reached" : mine ? "Manage your existing market instead" : undefined}
                      onClick={() => toggle(c)}
                    />
                  </div>
                );
              })}
            </div>
            {entries.length > 0 && <ExecutionOrder entries={entries} />}
          </Section>

          <Section n="04" title="Configure" locked={entries.length === 0}>
            {entries.map((e) => (
              <div className="config-card" key={e.slug}>
                <header>
                  <span>{e.name.toUpperCase()}</span>
                  <span className="p">priority {e.info.defaultPriority}</span>
                </header>
                <div className="body">
                  {e.moduleId === ModuleId.Sentient && (
                    <p className="mute" style={{ fontSize: 12.5, margin: "8px 0 4px" }}>
                      <b>signer</b> is your SENTIENT engine&apos;s public key. The engine keeps its private key off-chain and can only submit bounded,
                      expiring regime signals for this market.
                    </p>
                  )}
                  <ConfigEditor module={e.moduleId as ModuleId} values={forms[e.slug] ?? {}} parsed={parsed[e.slug]} onChange={(v) => { setForms({ ...forms, [e.slug]: v }); setSim({ status: "idle" }); }} symbol={symbol} />
                </div>
              </div>
            ))}
          </Section>
        </div>

        <div className="col" style={{ position: "sticky", top: "var(--header)" }}>
          <Section n="05" title="Simulate">
            {!live && (
              <div className="notice warn">
                <b>Pump Hooks isn&apos;t deployed on {CLUSTERS[cluster].label} yet.</b> Detection and quotes are live; simulation and activation need the
                program on-chain. The same flow runs locally against Pump&apos;s real programs with <span className="mono">pnpm test</span>.
              </div>
            )}
            {live && !wallet.publicKey && <div className="notice">Connect a wallet — the simulation uses it as market authority and test buyer. Nothing is signed.</div>}
            {mine && (
              <div className="notice">
                You already have a market for this token. <Link className="link-u" href={`/market/${mine.address.toBase58()}`}>Manage it</Link> — simulating a buy against it.
              </div>
            )}
            <div className="field" style={{ borderTop: 0 }}>
              <label htmlFor="testbuy">
                test buy
                <small>Routed through the whole stack</small>
              </label>
              <div className="input-unit">
                <input id="testbuy" className="input" value={testBuy} onChange={(e) => setTestBuy(e.target.value)} />
                <span>SOL</span>
              </div>
            </div>
            <button className="btn" style={{ width: "100%", marginTop: 8 }} disabled={!canSim || sim.status === "loading"} onClick={simulate}>
              {sim.status === "loading" ? <span className="spin" /> : null} Simulate {mine ? "buy" : "stack + buy"}
            </button>
            <div style={{ marginTop: 14 }}>
              {sim.status === "error" && <div className="notice deny">{sim.error}</div>}
              {sim.status === "ok" && (
                <>
                  <TracePipeline result={sim.data.result} user={wallet.publicKey ?? undefined} venue={token?.venue} decimals={decimals} symbol={symbol} />
                  <p className="mute" style={{ fontSize: 12, marginTop: 8 }}>
                    {sim.data.mode === "preview"
                      ? "One simulated transaction: create market → install hooks → routed buy. Nothing was written on-chain."
                      : "Simulated routed buy against your existing market."}
                  </p>
                  <details>
                    <summary>Program logs</summary>
                    <div className="logbox">{sim.data.result.logs.join("\n")}</div>
                  </details>
                </>
              )}
            </div>
          </Section>

          <Section n="06" title="Activate">
            {act.status === "ok" ? (
              <div className="notice ok reveal">
                <b>Market active.</b> Routed trades for this token now run your stack.
                <div className="btn-row" style={{ marginTop: 12 }}>
                  <Link className="btn small" href={`/market/${act.data.market.toBase58()}`}>
                    Open market <span className="chev">›</span>
                  </Link>
                  {act.data.sent.map((s, i) => (
                    <Link key={s.signature} className="link-u mono" href={`/tx/${s.signature}`}>
                      tx {i + 1}: {short(s.signature)}
                    </Link>
                  ))}
                </div>
              </div>
            ) : (
              <>
                <p className="mute" style={{ fontSize: 13, marginTop: 0 }}>
                  Creates your market (namespace = your wallet) and installs the stack. You stay the market authority; Pump Hooks never takes custody
                  of the token, its fees or its pool.
                </p>
                <button
                  className="btn signal"
                  style={{ width: "100%" }}
                  disabled={!writesEnabled || !wallet.publicKey || !token?.routable || !configsValid || entries.length === 0 || !!mine || act.status === "loading" || sim.status !== "ok" || !sim.data.result.ok}
                  onClick={activate}
                >
                  {act.status === "loading" ? (
                    <>
                      <span className="spin" /> {act.step}
                    </>
                  ) : (
                    "Activate stack"
                  )}
                </button>
                <p className="faint" style={{ fontSize: 12 }}>A successful simulation is required first.</p>
                {act.status === "error" && <div className="notice deny">{act.error}</div>}
              </>
            )}
          </Section>
          <div className="notice" style={{ marginTop: 6 }}>
            <b>Router boundary.</b> Hooks run on trades routed through Pump Hooks. PumpSwap doesn&apos;t call external hooks, so trades sent to Pump
            directly bypass the stack.
          </div>
        </div>
      </div>
    </main>
  );
}

function Section({ n, title, aside, locked, children }: { n: string; title: string; aside?: string; locked?: boolean; children: React.ReactNode }) {
  return (
    <section className={`section${locked ? " locked" : ""}`}>
      <div className="section-h">
        <span className="n">{n}</span>
        <h3>{title}</h3>
        {aside && <span className="aside">{aside}</span>}
      </div>
      {children}
    </section>
  );
}

function Skeleton() {
  return (
    <div className="kv">
      {Array.from({ length: 8 }, (_, i) => (
        <div key={i}>
          <div className="skeleton" style={{ width: "40%", height: 9 }} />
          <div className="skeleton" style={{ width: "80%", marginTop: 8 }} />
        </div>
      ))}
    </div>
  );
}

function Kv({ k, children, title }: { k: string; children: React.ReactNode; title?: string }) {
  return (
    <div>
      <div className="k">{k}</div>
      <div className="v" title={title}>{children}</div>
    </div>
  );
}

function Detection({ d, cluster, symbol }: { d: Detected; cluster: "devnet" | "mainnet-beta"; symbol: string }) {
  const t = d.token;
  const addr = (k: PublicKey) => (
    <a href={explorerUrl(cluster, "address", k.toBase58())} target="_blank" rel="noreferrer">
      {short(k, 5)}
    </a>
  );
  const sq = t.quoteMint.equals(NATIVE_MINT);
  const state = t.venue === Venue.PumpSwap ? "Migrated · PumpSwap" : t.venue === Venue.BondingCurve ? "Bonding curve" : "Curve complete · awaiting migration";
  return (
    <div className="reveal">
      <div style={{ display: "flex", alignItems: "baseline", gap: 12, marginBottom: 14, flexWrap: "wrap" }}>
        <span className="h3">{d.meta?.name || "Unnamed token"}</span>
        <span className="mono mute">{d.meta?.symbol ? `$${d.meta.symbol}` : ""}</span>
        <span className="tags" style={{ marginLeft: "auto" }}>
          <span className={`tag ${t.routable ? "ok" : "deny"}`}>{t.routable ? "routable" : "not routable"}</span>
          <span className="tag ink">{state}</span>
        </span>
      </div>
      <div className="kv">
        <Kv k="Mint">{addr(t.mint)}</Kv>
        <Kv k="Creator">{addr(t.creator)}</Kv>
        <Kv k="Token program">{t.isToken2022 ? "Token-2022" : "SPL Token"} · {t.decimals} dp</Kv>
        <Kv k="Quote mint">{sq ? "SOL (wSOL)" : addr(t.quoteMint)}</Kv>
        <Kv k={t.venue === Venue.PumpSwap ? "Pool" : "Bonding curve"}>{addr(t.venue === Venue.PumpSwap ? t.pool : t.bondingCurve)}</Kv>
        <Kv k={t.venue === Venue.PumpSwap ? "Pool quote reserve" : "Curve SOL (real)"}>
          {sol(t.venue === Venue.PumpSwap ? t.reserves.quote : t.curve.realQuoteReserves, 3)} {sq ? "SOL" : "quote"}
        </Kv>
        <Kv k={t.venue === Venue.PumpSwap ? "Pool token reserve" : "Curve tokens left"}>
          {units(t.venue === Venue.PumpSwap ? t.reserves.base : t.curve.realTokenReserves, t.decimals, 0)} {symbol}
        </Kv>
        <Kv k="Holder rewards">{t.isHolderReward ? "enabled" : "off"}{t.isCashback ? " · cashback" : ""}{t.isMayhem ? " · mayhem" : ""}</Kv>
        <Kv k="Hooked markets">
          {d.markets === undefined ? "—" : d.markets.length === 0 ? "none yet" : `${d.markets.length}`}
        </Kv>
      </div>
      {!t.routable && <div className="notice deny" style={{ marginTop: 12 }}>{t.notRoutableReason}</div>}
      {d.marketsError && <div className="notice" style={{ marginTop: 12 }}>{d.marketsError}</div>}
      {d.markets && d.markets.length > 0 && (
        <table className="table" style={{ marginTop: 14 }}>
          <thead>
            <tr>
              <th>Existing HookConfig</th>
              <th>Authority</th>
              <th>Hooks</th>
              <th>Routed trades</th>
            </tr>
          </thead>
          <tbody>
            {d.markets.map((m) => (
              <tr key={m.address.toBase58()}>
                <td>
                  <Link className="link-u" href={`/market/${m.address.toBase58()}`}>{short(m.address, 5)}</Link>
                </td>
                <td>{short(m.config.authority)}</td>
                <td>{m.config.slots.map((s) => s.module?.name ?? s.moduleId).join(", ") || "—"}</td>
                <td>{m.config.tradeCount.toString()}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function ExecutionOrder({ entries }: { entries: CatalogEntry[] }) {
  const runtime = entries.filter((e) => e.runtime);
  const cbs = (e: CatalogEntry) => callbacksOf(e.info.defaultCallbacks);
  const before = runtime.filter((e) => cbs(e).some((c) => BEFORE.has(c)) || e.moduleId === ModuleId.Policy);
  const after = runtime.filter((e) => cbs(e).some((c) => !BEFORE.has(c)) || e.moduleId === ModuleId.Policy);
  const configOnly = entries.filter((e) => !e.runtime);
  return (
    <div style={{ marginTop: 18 }}>
      <div className="section-h" style={{ marginBottom: 8 }}>
        <h3 style={{ fontSize: 11 }}>Execution order</h3>
      </div>
      <div className="pipeline">
        <div className="pipe-row">
          <span className="i">→</span>
          <span className="stage-n">Before</span>
          <span>{before.map((e) => e.name).join("  ›  ") || "—"}</span>
          <span className="r mute">can deny</span>
        </div>
        <div className="pipe-row cpi">
          <span className="i">⇄</span>
          <span className="stage-n">Pump CPI</span>
          <span>wallet-signed buy / sell</span>
          <span className="r" />
        </div>
        <div className="pipe-row">
          <span className="i">←</span>
          <span className="stage-n">After</span>
          <span>{after.map((e) => e.name).join("  ›  ") || "—"}</span>
          <span className="r mute">observe · re-check</span>
        </div>
        {configOnly.length > 0 && (
          <div className="pipe-row skipped">
            <span className="i">·</span>
            <span className="stage-n">Config-only</span>
            <span>{configOnly.map((e) => e.name).join(", ")}</span>
            <span className="r">enforced by vault</span>
          </div>
        )}
      </div>
    </div>
  );
}
