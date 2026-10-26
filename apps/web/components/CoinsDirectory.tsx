"use client";

import Link from "next/link";
import { useEffect, useMemo, useState } from "react";
import { PublicKey } from "@solana/web3.js";
import { decodeCurve, MARKET_FLAGS, pumpPda, type HookedMarket } from "@pump-hooks/sdk";
import { useNetwork } from "./Providers";
import { DitherStage } from "./DitherStage";
import { CLUSTERS } from "@/lib/cluster";
import { fetchTokenMeta, type TokenMeta } from "@/lib/metadata";
import { short, sol } from "@/lib/format";

/** Only /api/launch-metadata writes here, so a URI on this host means the coin was launched on this site. */
const LAUNCH_METADATA_PREFIX = "https://1ma9miipl5s6qrtp.public.blob.vercel-storage.com/coins/";

interface Coin {
  mint: PublicKey;
  market: HookedMarket;
  otherMarkets: number;
  meta?: TokenMeta;
  image?: string;
  graduated?: boolean;
  launchedHere: boolean;
}

type Load = { status: "loading" } | { status: "error"; error: string } | { status: "ok"; coins: Coin[] };
type Filter = "all" | "launched" | "hooked";

async function imageOf(uri: string): Promise<string | undefined> {
  if (!uri) return undefined;
  try {
    const j = (await (await fetch(`/api/coin-image?uri=${encodeURIComponent(uri)}`)).json()) as { image?: string | null };
    return j.image ?? undefined;
  } catch {
    return undefined;
  }
}

const volume = (m: HookedMarket) => m.config.routedBuyQuote + m.config.routedSellQuote;

export function CoinsDirectory() {
  const { client, connection, cluster, program } = useNetwork();
  const [load, setLoad] = useState<Load>({ status: "loading" });
  const [filter, setFilter] = useState<Filter>("all");

  useEffect(() => {
    if (program.state !== "live") return;
    let stop = false;
    (async () => {
      setLoad({ status: "loading" });
      try {
        const markets = await client.allMarkets();
        const byMint = new Map<string, HookedMarket[]>();
        for (const m of markets) byMint.set(m.config.mint.toBase58(), [...(byMint.get(m.config.mint.toBase58()) ?? []), m]);
        const groups = [...byMint.values()].map((ms) =>
          ms.sort((a, b) => Number(b.config.flags & MARKET_FLAGS.CREATOR_VERIFIED) - Number(a.config.flags & MARKET_FLAGS.CREATOR_VERIFIED) || Number(volume(b) - volume(a))),
        );
        const mints = groups.map((g) => g[0].config.mint);
        const curves: (Awaited<ReturnType<typeof connection.getMultipleAccountsInfo>>[number])[] = [];
        for (let i = 0; i < mints.length; i += 100) curves.push(...(await connection.getMultipleAccountsInfo(mints.slice(i, i + 100).map((m) => pumpPda.bondingCurve(m)))));
        const coins = await Promise.all(
          groups.map(async (g, i): Promise<Coin> => {
            const meta = await fetchTokenMeta(connection, mints[i]);
            let graduated: boolean | undefined;
            try {
              graduated = curves[i] ? decodeCurve(curves[i]!.data).complete : undefined;
            } catch {
              graduated = undefined;
            }
            return {
              mint: mints[i],
              market: g[0],
              otherMarkets: g.length - 1,
              meta,
              image: meta ? await imageOf(meta.uri) : undefined,
              graduated,
              launchedHere: !!meta?.uri.startsWith(LAUNCH_METADATA_PREFIX),
            };
          }),
        );
        coins.sort((a, b) => Number(volume(b.market) - volume(a.market)) || Number(b.market.config.createdAt - a.market.config.createdAt));
        if (!stop) setLoad({ status: "ok", coins });
      } catch (e) {
        if (!stop) setLoad({ status: "error", error: (e as Error).message });
      }
    })();
    return () => {
      stop = true;
    };
  }, [client, connection, program.state]);

  const shown = useMemo(() => {
    if (load.status !== "ok") return [];
    return load.coins.filter((c) => filter === "all" || (filter === "launched") === c.launchedHere);
  }, [load, filter]);
  const counts = load.status === "ok" ? { all: load.coins.length, launched: load.coins.filter((c) => c.launchedHere).length, hooked: load.coins.filter((c) => !c.launchedHere).length } : undefined;

  return (
    <main className="page">
      <div className="page-head">
        <DitherStage mode="banner" />
        <div className="crumb">
          <b>02</b> / Coins
        </div>
        <h1 className="h2">Coins running on Pump Hooks.</h1>
        <p className="lede" style={{ marginBottom: 20 }}>
          Every pump.fun coin with a Pump Hooks market on {CLUSTERS[cluster].label}, read live from the chain: coins launched here with rules from
          block one, and existing coins their creators or communities hooked later.
        </p>
        <div className="btn-row">
          <Link className="btn" href="/launch">
            Launch a coin <span className="chev">›</span>
          </Link>
          <Link className="btn outline" href="/hook">
            Hook an existing coin
          </Link>
        </div>
      </div>

      <section className="section">
        <div className="chips" style={{ marginBottom: 16 }}>
          {(["all", "launched", "hooked"] as const).map((f) => (
            <button key={f} className="chip" aria-pressed={filter === f} onClick={() => setFilter(f)}>
              {f === "all" ? "All" : f === "launched" ? "Launched here" : "Hooked later"}
              {counts && <span className="c">{counts[f]}</span>}
            </button>
          ))}
        </div>

        {program.state !== "live" && <div className="notice">Pump Hooks isn&apos;t deployed on {CLUSTERS[cluster].label}. Switch to mainnet.</div>}
        {program.state === "live" && load.status === "loading" && <p className="mute"><span className="spin" /> Reading markets from chain…</p>}
        {load.status === "error" && <div className="notice deny">{load.error}</div>}
        {load.status === "ok" && shown.length === 0 && <p className="mute">No coins here yet.</p>}

        <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(290px, 1fr))", gap: 14 }}>
          {shown.map((c) => (
            <CoinCard key={c.mint.toBase58()} c={c} />
          ))}
        </div>

        <div className="notice" style={{ marginTop: 18 }}>
          <b>Router boundary.</b> A coin&apos;s rules run on trades routed through Pump Hooks. Trades sent to pump.fun directly bypass them. Listing
          here is not an endorsement: anyone can launch or hook a coin.
        </div>
      </section>
    </main>
  );
}

function CoinCard({ c }: { c: Coin }) {
  const cfg = c.market.config;
  const mint = c.mint.toBase58();
  const rules = cfg.slots.map((s) => s.module?.name ?? `module ${s.moduleId}`);
  return (
    <div className="section reveal" style={{ border: "1px solid var(--rule-2)", padding: 16, margin: 0, display: "flex", flexDirection: "column", gap: 12 }}>
      <div style={{ display: "flex", gap: 12, alignItems: "center" }}>
        {c.image ? (
          <img src={c.image} alt="" width={52} height={52} style={{ objectFit: "cover", borderRadius: 6, flex: "none" }} />
        ) : (
          <div className="mono" style={{ width: 52, height: 52, borderRadius: 6, background: "var(--ink)", color: "var(--paper)", display: "grid", placeItems: "center", flex: "none" }}>
            {(c.meta?.symbol || "?").slice(0, 2).toUpperCase()}
          </div>
        )}
        <div style={{ minWidth: 0 }}>
          <div className="h3" style={{ fontSize: 17, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{c.meta?.name || "Unnamed token"}</div>
          <div className="mono mute" style={{ fontSize: 12 }}>
            {c.meta?.symbol ? `$${c.meta.symbol}` : ""} · {short(mint, 4)}
          </div>
        </div>
      </div>
      <div className="tags">
        {c.launchedHere && <span className="tag ink">launched here</span>}
        {cfg.flags & MARKET_FLAGS.CREATOR_VERIFIED ? <span className="tag ok">creator verified</span> : null}
        {c.graduated !== undefined && <span className="tag">{c.graduated ? "graduated" : "bonding curve"}</span>}
        {cfg.flags & MARKET_FLAGS.PAUSED ? <span className="tag deny">paused</span> : null}
      </div>
      <div className="mute" style={{ fontSize: 12.5 }}>{rules.length ? rules.join(" · ") : "No rules installed"}</div>
      <div className="kv" style={{ gridTemplateColumns: "1fr 1fr" }}>
        <div>
          <div className="k">Routed trades</div>
          <div className="v">{cfg.tradeCount.toString()}</div>
        </div>
        <div>
          <div className="k">Routed volume</div>
          <div className="v">{sol(volume(c.market), 2)} SOL</div>
        </div>
      </div>
      {c.otherMarkets > 0 && <div className="faint" style={{ fontSize: 12 }}>+{c.otherMarkets} other market{c.otherMarkets > 1 ? "s" : ""} for this coin</div>}
      <div className="btn-row" style={{ marginTop: "auto" }}>
        <Link className="btn small" href={`/market/${c.market.address.toBase58()}`}>
          Trade with rules <span className="chev">›</span>
        </Link>
        <a className="link-u" href={`https://pump.fun/coin/${mint}`} target="_blank" rel="noreferrer">
          pump.fun
        </a>
      </div>
    </div>
  );
}
