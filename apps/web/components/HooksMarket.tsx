"use client";

import Link from "next/link";
import { useState } from "react";
import { Callback, PUMP_HOOKS_PROGRAM_ID } from "@pump-hooks/sdk";
import { DitherStage } from "./DitherStage";
import { useRegistry, type RegistryState } from "./Registry";
import { useNetwork } from "./Providers";
import { CATALOG, CATEGORIES, type Category, type CatalogEntry } from "@/lib/catalog";
import { CLUSTERS } from "@/lib/cluster";
import { short } from "@/lib/format";

export function HooksMarket() {
  const [cats, setCats] = useState<Set<Category>>(new Set());
  const reg = useRegistry(CATALOG.map((c) => c.moduleId));
  const { cluster } = useNetwork();
  const list = CATALOG.filter((c) => cats.size === 0 || c.categories.some((k) => cats.has(k)));
  return (
    <main className="page">
      <div className="page-head">
        <DitherStage mode="banner" />
        <div className="crumb">
          <b>02</b> / Hooks
        </div>
        <h1 className="h2">The app store for market behavior.</h1>
        <p className="lede">
          Every hook a Pump market can install: its interface, capabilities, config schema, source and registry state. V1 ships first-party
          modules that execute inside the router; external CPI hooks are not enabled.
        </p>
      </div>

      <div className="filters">
        <button className="chip" style={{ flex: "0 0 auto", minWidth: 0, border: "1px solid var(--rule)" }} aria-pressed={cats.size === 0} onClick={() => setCats(new Set())}>
          ALL <span className="c">{CATALOG.length}</span>
        </button>
        {CATEGORIES.map((c) => {
          const on = cats.has(c);
          const n = CATALOG.filter((x) => x.categories.includes(c)).length;
          return (
            <button
              key={c}
              className="chip"
              style={{ flex: "0 0 auto", minWidth: 0, border: "1px solid var(--rule)" }}
              aria-pressed={on}
              onClick={() => {
                const s = new Set(cats);
                if (on) s.delete(c);
                else s.add(c);
                setCats(s);
              }}
            >
              {c} <span className="c">{n}</span>
            </button>
          );
        })}
        <span className="mono mute" style={{ marginLeft: "auto", fontSize: 11 }}>
          registry · {CLUSTERS[cluster].label.toLowerCase()} · {reg.status === "ok" ? `${reg.defs.size} registered` : reg.status === "loading" ? "loading" : reg.reason}
        </span>
      </div>

      {list.length === 0 ? (
        <div className="notice">
          <b>No hooks in this category yet.</b> Liquidity callbacks are reserved in the ABI; no V1 module ships for them.
        </div>
      ) : (
        <div className="market-grid">
          {list.map((c) => (
            <HookCard key={c.slug} c={c} reg={reg} />
          ))}
        </div>
      )}
    </main>
  );
}

function HookCard({ c, reg }: { c: CatalogEntry; reg: RegistryState }) {
  const def = reg.status === "ok" ? reg.defs.get(c.moduleId) : undefined;
  const subs = new Set(c.callbacks);
  return (
    <Link href={`/hooks/${c.slug}`} className="hook-card">
      <div className="top">
        <span className="id">
          0x{c.moduleId.toString(16).padStart(2, "0")} · v{c.version} · {c.developer}
        </span>
        <span className="tags">
          {!c.available ? <span className="tag warn">reserved</span> : !c.runtime ? <span className="tag">config-only</span> : <span className="tag ink">runtime</span>}
        </span>
      </div>
      <h3>{c.name}</h3>
      <p>{c.summary}</p>
      <div>
        <div className="cb-strip" title="Callbacks: 6 V1 trade callbacks, then 6 reserved">
          {Array.from({ length: 12 }, (_, i) => (
            <i key={i} className={i >= 6 ? "res" : subs.has(Callback[i]) ? "on" : ""} title={Callback[i]} />
          ))}
        </div>
        <div className="mono faint" style={{ fontSize: 10, marginTop: 4 }}>
          {c.callbacks.length ? c.callbacks.join(" · ") : "no callbacks — enforced by the strategy vault"}
        </div>
      </div>
      <div className="meta">
        <span>Program</span>
        <b>{short(PUMP_HOOKS_PROGRAM_ID)} (built-in)</b>
        <span>Capabilities</span>
        <b>{c.capabilities.length}</b>
        <span>Installs</span>
        <b>{def ? def.installs.toString() : "—"}</b>
        <span>Executions</span>
        <b title="Needs the indexer">—</b>
        <span>Markets</span>
        <b>curve · pumpswap</b>
        <span>Audit</span>
        <b className="warn">unaudited</b>
      </div>
    </Link>
  );
}
