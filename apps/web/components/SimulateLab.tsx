"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { PublicKey } from "@solana/web3.js";
import { useWallet } from "@solana/wallet-adapter-react";
import { ModuleId, type ExecutionResult } from "@pump-hooks/sdk";
import { DitherStage } from "./DitherStage";
import { TracePipeline } from "./TracePipeline";
import { isPubkey, useNetwork } from "./Providers";
import { CATALOG } from "@/lib/catalog";
import { parseValues, presetValues } from "@/lib/config-form";
import { CLUSTERS } from "@/lib/cluster";
import { parseSol } from "@/lib/format";

const RUNTIME = CATALOG.filter((c) => c.runtime && c.available);

export function SimulateLab() {
  const router = useRouter();
  const { client, program, cluster } = useNetwork();
  const wallet = useWallet();
  const [mint, setMint] = useState("");
  const [picked, setPicked] = useState<string[]>(["launch-shield", "volume-guard"]);
  const [amt, setAmt] = useState("0.25");
  const [sig, setSig] = useState("");
  const [st, setSt] = useState<{ s: "idle" | "loading" } | { s: "ok"; result: ExecutionResult; decimals: number } | { s: "error"; msg: string }>({ s: "idle" });

  const live = program.state === "live";
  const quoteIn = parseSol(amt);
  const run = async () => {
    if (!wallet.publicKey || !isPubkey(mint) || !quoteIn) return;
    setSt({ s: "loading" });
    try {
      const token = await client.detect(new PublicKey(mint.trim()));
      if (!token.routable) throw new Error(token.notRoutableReason ?? "this token cannot be routed right now");
      const hooks = RUNTIME.filter((c) => picked.includes(c.slug))
        .slice(0, 4)
        .map((c) => {
          const id = c.moduleId as ModuleId;
          const p = parseValues(id, presetValues(id, token.decimals, wallet.publicKey!), token.decimals);
          return { module: id, config: p.config };
        });
      const result = await client.simulate(await client.previewStack({ mint: token.mint, authority: wallet.publicKey, hooks, quoteIn }), wallet.publicKey);
      setSt({ s: "ok", result, decimals: token.decimals });
    } catch (e) {
      setSt({ s: "error", msg: (e as Error).message });
    }
  };

  return (
    <main className="page">
      <div className="page-head">
        <DitherStage mode="banner" />
        <div className="crumb">
          <b>04</b> / Simulate
        </div>
        <h1 className="h2">Run it before it runs.</h1>
        <p className="lede">
          Simulate a routed buy against live {CLUSTERS[cluster].label} state with a hook stack that doesn&apos;t exist yet: market creation, installs and the
          trade execute together in one simulated transaction, and the trace below is decoded from its logs. Nothing is signed or sent.
        </p>
      </div>

      <div className="grid-main">
        <div className="col">
          <section className="section">
            <div className="section-h">
              <span className="n">01</span>
              <h3>Token</h3>
            </div>
            <input className="input" placeholder="Pump token mint (CA)" value={mint} onChange={(e) => setMint(e.target.value)} spellCheck={false} />
          </section>
          <section className="section">
            <div className="section-h">
              <span className="n">02</span>
              <h3>Stack</h3>
              <span className="aside">presets · up to 4 runtime hooks · configure fully in Hook a token</span>
            </div>
            <div className="chips">
              {RUNTIME.map((c) => {
                const on = picked.includes(c.slug);
                return (
                  <button
                    key={c.slug}
                    className="chip"
                    aria-pressed={on}
                    disabled={!on && picked.length >= 4}
                    onClick={() => setPicked(on ? picked.filter((s) => s !== c.slug) : [...picked, c.slug])}
                  >
                    {c.name}
                  </button>
                );
              })}
            </div>
            {picked.includes("sentient") && <p className="faint" style={{ fontSize: 12 }}>SENTIENT uses your wallet as the preview signer; with no signal submitted the regime is Normal.</p>}
          </section>
          <section className="section">
            <div className="section-h">
              <span className="n">03</span>
              <h3>Trade</h3>
            </div>
            <div className="grid-2" style={{ gap: 8 }}>
              <div className="input-unit">
                <input className="input" value={amt} onChange={(e) => setAmt(e.target.value)} inputMode="decimal" />
                <span>SOL buy</span>
              </div>
              <button className="btn" disabled={!live || !wallet.publicKey || !isPubkey(mint) || !quoteIn || st.s === "loading"} onClick={run}>
                {st.s === "loading" ? <span className="spin" /> : "Simulate routed buy"}
              </button>
            </div>
            {!live && (
              <div className="notice" style={{ marginTop: 12 }}>
                Pump Hooks is {program.state === "loading" ? "loading" : "not deployed"} on {CLUSTERS[cluster].label}; simulation needs the program on-chain.
              </div>
            )}
            {live && !wallet.publicKey && <p className="faint" style={{ fontSize: 12 }}>Connect a wallet: it is the simulated payer and must hold enough SOL.</p>}
          </section>
          <section className="section">
            <div className="section-h">
              <span className="n">04</span>
              <h3>Decode a transaction</h3>
            </div>
            <form
              className="grid-2"
              style={{ gap: 8 }}
              onSubmit={(e) => {
                e.preventDefault();
                if (sig.trim()) router.push(`/tx/${sig.trim()}`);
              }}
            >
              <input className="input" placeholder="Transaction signature" value={sig} onChange={(e) => setSig(e.target.value)} spellCheck={false} />
              <button className="btn outline" disabled={!/^[1-9A-HJ-NP-Za-km-z]{64,90}$/.test(sig.trim())}>
                Open trace
              </button>
            </form>
          </section>
        </div>
        <div className="col" style={{ position: "sticky", top: "var(--header)" }}>
          <section className="section">
            <div className="section-h">
              <span className="n">05</span>
              <h3>Trace</h3>
              {st.s === "ok" && <span className={`aside ${st.result.ok ? "ok" : "deny"}`}>{st.result.ok ? "would succeed" : "would revert"}</span>}
            </div>
            {st.s === "idle" && <p className="mute">Wallet → Pump Hooks → BeforeBuy → Pump CPI → AfterBuy → settlement, once simulated.</p>}
            {st.s === "loading" && <div className="skeleton" />}
            {st.s === "error" && <div className="notice deny">{st.msg}</div>}
            {st.s === "ok" && <TracePipeline result={st.result} user={wallet.publicKey ?? undefined} decimals={st.decimals} />}
            {st.s === "ok" && (
              <Link className="btn" style={{ width: "100%", marginTop: 14 }} href={`/hook?mint=${mint.trim()}`}>
                Configure and activate this stack <span className="chev">›</span>
              </Link>
            )}
          </section>
        </div>
      </div>
    </main>
  );
}
