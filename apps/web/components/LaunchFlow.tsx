"use client";

import Link from "next/link";
import { useEffect, useMemo, useState } from "react";
import { ComputeBudgetProgram, Keypair, PublicKey } from "@solana/web3.js";
import { useWallet } from "@solana/wallet-adapter-react";
import { bundle, COIN_LIMITS, ModuleId, type ExecutionResult } from "@pump-hooks/sdk";
import { useNetwork } from "./Providers";
import { DitherStage } from "./DitherStage";
import { ConfigEditor } from "./ConfigEditor";
import { CATALOG, type CatalogEntry } from "@/lib/catalog";
import { parseValues, presetValues, type FormValues } from "@/lib/config-form";
import { parseSol, short } from "@/lib/format";
import { pack, sendBatches, sendWithSigners, type SentTx } from "@/lib/tx";

const DECIMALS = 6;
const MAX_RULES = 4;
const MAX_IMAGE = 2 * 1024 * 1024;
const CREATE_CU = 300_000;
/** Rules that run on every routed trade and need nothing beyond their config. */
const LAUNCH_RULES = CATALOG.filter((c) => c.available && c.runtime && c.moduleId !== ModuleId.Sentient && c.moduleId !== ModuleId.Policy);
const DEFAULT_RULES = [ModuleId.MaxWallet, ModuleId.SellLimit].map((m) => LAUNCH_RULES.find((c) => c.moduleId === m)?.slug).filter(Boolean) as string[];

type Step = { status: "idle" } | { status: "busy"; step: string } | { status: "error"; error: string; logs?: string[] };
interface Launched {
  mint: PublicKey;
  market: PublicKey;
  create: SentTx;
  firstBuy?: SentTx;
  firstBuyError?: string;
}

const bytes = (s: string) => new TextEncoder().encode(s).length;

export function LaunchFlow() {
  const { client, connection, cluster, program, writesEnabled } = useNetwork();
  const wallet = useWallet();

  const [name, setName] = useState("");
  const [symbol, setSymbol] = useState("");
  const [description, setDescription] = useState("");
  const [twitter, setTwitter] = useState("");
  const [telegram, setTelegram] = useState("");
  const [website, setWebsite] = useState("");
  const [image, setImage] = useState<File>();
  const [preview, setPreview] = useState<string>();
  const [selected, setSelected] = useState<string[]>(DEFAULT_RULES);
  const [forms, setForms] = useState<Record<string, FormValues>>(() =>
    Object.fromEntries(DEFAULT_RULES.map((s) => [s, presetValues(CATALOG.find((c) => c.slug === s)!.moduleId as ModuleId, DECIMALS)])),
  );
  const [firstBuy, setFirstBuy] = useState("");
  const [mint, setMint] = useState(() => Keypair.generate());
  const [state, setState] = useState<Step>({ status: "idle" });
  const [done, setDone] = useState<Launched>();

  useEffect(() => {
    if (!image) return setPreview(undefined);
    const url = URL.createObjectURL(image);
    setPreview(url);
    return () => URL.revokeObjectURL(url);
  }, [image]);

  const entries = selected.map((s) => LAUNCH_RULES.find((c) => c.slug === s)!).sort((a, b) => a.info.defaultPriority - b.info.defaultPriority);
  const parsed = Object.fromEntries(entries.map((e) => [e.slug, parseValues(e.moduleId as ModuleId, forms[e.slug] ?? {}, DECIMALS)]));
  const configsValid = entries.every((e) => Object.keys(parsed[e.slug].fieldErrors).length === 0 && parsed[e.slug].problems.length === 0);
  const hooks = entries.map((e) => ({ module: e.moduleId as ModuleId, config: parsed[e.slug].config }));

  const buyLamports = firstBuy.trim() ? parseSol(firstBuy) : 0n;
  const problems = useMemo(() => {
    const p: string[] = [];
    if (!name.trim()) p.push("Name is required");
    else if (bytes(name.trim()) > COIN_LIMITS.name) p.push(`Name is too long (max ${COIN_LIMITS.name} bytes)`);
    if (!symbol.trim()) p.push("Ticker is required");
    else if (bytes(symbol.trim()) > COIN_LIMITS.symbol) p.push(`Ticker is too long (max ${COIN_LIMITS.symbol} bytes)`);
    if (!image) p.push("Image is required");
    else if (image.size > MAX_IMAGE) p.push("Image must be 2 MB or smaller");
    if (buyLamports === undefined || buyLamports < 0n) p.push("First buy must be a SOL amount");
    if (!configsValid) p.push("Fix the rule settings below");
    return p;
  }, [name, symbol, image, buyLamports, configsValid]);

  const toggle = (c: CatalogEntry) => {
    if (selected.includes(c.slug)) return setSelected(selected.filter((s) => s !== c.slug));
    setSelected([...selected, c.slug]);
    if (!forms[c.slug]) setForms({ ...forms, [c.slug]: presetValues(c.moduleId as ModuleId, DECIMALS) });
  };

  const live = program.state === "live" && cluster === "mainnet-beta";
  const busy = state.status === "busy";
  const ready = live && writesEnabled && !!wallet.publicKey && problems.length === 0 && !busy;

  const launch = async () => {
    if (!wallet.publicKey || !image) return;
    const creator = wallet.publicKey;
    try {
      setState({ status: "busy", step: "Uploading image + metadata" });
      const form = new FormData();
      form.set("image", image);
      form.set("name", name.trim());
      form.set("symbol", symbol.trim());
      form.set("description", description.trim());
      form.set("twitter", twitter.trim());
      form.set("telegram", telegram.trim());
      form.set("website", website.trim());
      const res = await fetch("/api/launch-metadata", { method: "POST", body: form });
      const meta = (await res.json()) as { uri?: string; error?: string };
      if (!res.ok || !meta.uri) throw new Error(meta.error ?? `metadata upload failed (${res.status})`);

      setState({ status: "busy", step: "Building transactions" });
      const { market, create, firstBuy: buy } = await client.launch({
        mint: mint.publicKey,
        creator,
        name: name.trim(),
        symbol: symbol.trim(),
        uri: meta.uri,
        hooks,
        firstBuy: buyLamports ? { quoteIn: buyLamports } : undefined,
      });
      const createIxs = [ComputeBudgetProgram.setComputeUnitLimit({ units: CREATE_CU }), ...create];
      const alts = await client.loadLookupTables();
      if (pack(createIxs, creator, alts).length > 1) throw new Error("too many rules to fit the create transaction; remove one");

      setState({ status: "busy", step: "Simulating" });
      const sim: ExecutionResult = await client.simulate(bundle(createIxs, creator, alts), creator);
      if (!sim.ok) {
        setState({ status: "error", error: sim.error ? `Simulation failed: ${sim.error.name}: ${sim.error.msg}` : "Simulation failed", logs: sim.logs });
        return;
      }

      const sent = await sendWithSigners(connection, wallet, createIxs, [mint], alts, (sig) =>
        setState({ status: "busy", step: sig ? "Confirming coin + rules" : "Approve in your wallet (1 of " + (buy ? 2 : 1) + ")" }),
      );
      if (!sent.result.ok) {
        const e = sent.result.error;
        setState({ status: "error", error: e ? `${e.name}: ${e.msg}` : "create transaction failed", logs: sent.result.logs });
        return;
      }
      const launched: Launched = { mint: mint.publicKey, market, create: sent };
      if (buy) {
        try {
          const [b] = await sendBatches(connection, wallet, [buy.instructions], alts, (_, sig) =>
            setState({ status: "busy", step: sig ? "Confirming first buy" : "Approve the first buy (2 of 2)" }),
          );
          launched.firstBuy = b;
          if (!b.result.ok) launched.firstBuyError = b.result.error ? `${b.result.error.name}: ${b.result.error.msg}` : "first buy failed";
        } catch (e) {
          launched.firstBuyError = (e as Error).message;
        }
      }
      setDone(launched);
      setMint(Keypair.generate());
      setState({ status: "idle" });
    } catch (e) {
      setState({ status: "error", error: (e as Error).message });
    }
  };

  return (
    <main className="page">
      <div className="page-head">
        <DitherStage mode="banner" />
        <div className="crumb">
          <b>00</b> / Launch a coin
        </div>
        <h1 className="h2">Launch on pump.fun with rules from block one.</h1>
        <p className="lede" style={{ marginBottom: 20 }}>
          One transaction creates the coin on pump.fun&apos;s own program, opens its Pump Hooks market and installs your rules. The coin is a normal
          pump.fun coin: same bonding curve, same graduation to PumpSwap. You are the creator and the market authority.
        </p>
        <p className="mute" style={{ fontSize: 13, marginTop: 0 }}>
          Already have a coin? <Link className="link-u" href="/hook">Hook an existing token</Link> instead.
        </p>
      </div>

      {done ? (
        <Done d={done} />
      ) : (
        <div className="grid-main">
          <div className="col">
            <Section n="01" title="Coin">
              <Field id="name" label="name" hint={`${bytes(name.trim())}/${COIN_LIMITS.name}`}>
                <input id="name" className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Pump Hooks" autoComplete="off" />
              </Field>
              <Field id="symbol" label="ticker" hint={`${bytes(symbol.trim())}/${COIN_LIMITS.symbol}`}>
                <input id="symbol" className="input" value={symbol} onChange={(e) => setSymbol(e.target.value)} placeholder="PHOOK" autoComplete="off" />
              </Field>
              <Field id="description" label="description" hint="optional">
                <textarea id="description" className="input" rows={3} value={description} onChange={(e) => setDescription(e.target.value)} />
              </Field>
              <Field id="image" label="image" hint="PNG, JPEG, GIF or WebP · max 2 MB">
                <div style={{ display: "flex", gap: 14, alignItems: "center" }}>
                  {preview && <img src={preview} alt="" width={56} height={56} style={{ objectFit: "cover", borderRadius: 6 }} />}
                  <input id="image" type="file" accept="image/png,image/jpeg,image/gif,image/webp" onChange={(e) => setImage(e.target.files?.[0])} />
                </div>
              </Field>
              <Field id="twitter" label="x / twitter" hint="optional">
                <input id="twitter" className="input" value={twitter} onChange={(e) => setTwitter(e.target.value)} placeholder="https://x.com/…" />
              </Field>
              <Field id="telegram" label="telegram" hint="optional">
                <input id="telegram" className="input" value={telegram} onChange={(e) => setTelegram(e.target.value)} placeholder="https://t.me/…" />
              </Field>
              <Field id="website" label="website" hint="optional">
                <input id="website" className="input" value={website} onChange={(e) => setWebsite(e.target.value)} placeholder="https://…" />
              </Field>
            </Section>

            <Section n="02" title="Rules" aside={`${entries.length}/${MAX_RULES}`}>
              <div className="module-list">
                {LAUNCH_RULES.map((c, i) => {
                  const on = selected.includes(c.slug);
                  return (
                    <div key={c.slug} className={`module${on ? " on" : ""}`}>
                      <span className="idx">{String(i + 1).padStart(2, "0")}</span>
                      <div>
                        <div className="name">
                          <Link href={`/hooks/${c.slug}`}>{c.name}</Link>
                        </div>
                        <div className="desc">{c.summary}</div>
                      </div>
                      <button
                        className="toggle"
                        aria-pressed={on}
                        aria-label={`${on ? "Remove" : "Add"} ${c.name}`}
                        disabled={!on && entries.length >= MAX_RULES}
                        onClick={() => toggle(c)}
                      />
                    </div>
                  );
                })}
              </div>
              {entries.length === 0 && <div className="notice" style={{ marginTop: 12 }}>No rules selected: the coin launches with an empty Pump Hooks market you can add rules to later.</div>}
            </Section>

            <Section n="03" title="Configure" locked={entries.length === 0}>
              {entries.map((e) => (
                <div className="config-card" key={e.slug}>
                  <header>
                    <span>{e.name.toUpperCase()}</span>
                    <span className="p">priority {e.info.defaultPriority}</span>
                  </header>
                  <div className="body">
                    <ConfigEditor module={e.moduleId as ModuleId} values={forms[e.slug] ?? {}} parsed={parsed[e.slug]} onChange={(v) => setForms({ ...forms, [e.slug]: v })} symbol={symbol.trim() || "tokens"} />
                  </div>
                </div>
              ))}
            </Section>
          </div>

          <div className="col" style={{ position: "sticky", top: "var(--header)" }}>
            <Section n="04" title="Launch">
              {!live && <div className="notice warn">Launching is mainnet only. Switch the network to mainnet.</div>}
              {live && !wallet.publicKey && <div className="notice">Connect a wallet. It becomes the coin&apos;s creator and the market authority.</div>}
              <Field id="firstbuy" label="first buy" hint="optional · routed through your rules">
                <div className="input-unit">
                  <input id="firstbuy" className="input" value={firstBuy} onChange={(e) => setFirstBuy(e.target.value)} placeholder="0" />
                  <span>SOL</span>
                </div>
              </Field>
              <div className="kv" style={{ marginTop: 10 }}>
                <div>
                  <div className="k">Mint (generated here)</div>
                  <div className="v mono">{short(mint.publicKey, 6)}</div>
                </div>
                <div>
                  <div className="k">Signatures</div>
                  <div className="v">{buyLamports ? "2 wallet approvals" : "1 wallet approval"}</div>
                </div>
              </div>
              {problems.length > 0 && wallet.publicKey && (
                <ul className="mute" style={{ fontSize: 12.5, paddingLeft: 18 }}>
                  {problems.map((p) => (
                    <li key={p}>{p}</li>
                  ))}
                </ul>
              )}
              <button className="btn signal" style={{ width: "100%", marginTop: 10 }} disabled={!ready} onClick={launch}>
                {busy ? (
                  <>
                    <span className="spin" /> {state.step}
                  </>
                ) : (
                  "Launch coin"
                )}
              </button>
              <p className="faint" style={{ fontSize: 12 }}>
                The create step is simulated before your wallet is asked to sign. Cost: about 0.017 SOL (pump.fun&apos;s creation rent, the market
                account and fees) plus your first buy.
              </p>
              {state.status === "error" && (
                <div className="notice deny">
                  {state.error}
                  {state.logs && state.logs.length > 0 && (
                    <details>
                      <summary>Program logs</summary>
                      <div className="logbox">{state.logs.join("\n")}</div>
                    </details>
                  )}
                </div>
              )}
            </Section>
            <div className="notice" style={{ marginTop: 6 }}>
              <b>Router boundary.</b> Rules run on trades routed through Pump Hooks (this site, the SDK, integrations). pump.fun doesn&apos;t call
              external hooks, so trades sent to pump.fun directly bypass them. Coin metadata is stored on this site&apos;s storage, not IPFS.
            </div>
          </div>
        </div>
      )}
    </main>
  );
}

function Done({ d }: { d: Launched }) {
  const mint = d.mint.toBase58();
  return (
    <div className="section reveal">
      <div className="notice ok">
        <b>Coin launched.</b> Your rules are live for trades routed through Pump Hooks.
      </div>
      <div className="kv" style={{ marginTop: 14 }}>
        <div>
          <div className="k">Contract address</div>
          <div className="v mono" style={{ wordBreak: "break-all" }}>{mint}</div>
        </div>
        <div>
          <div className="k">Create transaction</div>
          <div className="v">
            <Link className="link-u mono" href={`/tx/${d.create.signature}`}>{short(d.create.signature, 6)}</Link>
          </div>
        </div>
        {d.firstBuy && (
          <div>
            <div className="k">First buy</div>
            <div className="v">
              <Link className="link-u mono" href={`/tx/${d.firstBuy.signature}`}>{short(d.firstBuy.signature, 6)}</Link>
            </div>
          </div>
        )}
      </div>
      {d.firstBuyError && <div className="notice deny" style={{ marginTop: 12 }}>The coin and rules are live, but the first buy failed: {d.firstBuyError}</div>}
      <div className="btn-row" style={{ marginTop: 16 }}>
        <Link className="btn" href={`/market/${d.market.toBase58()}`}>
          Open market <span className="chev">›</span>
        </Link>
        <Link className="btn" href="/coins">
          All coins
        </Link>
        <a className="btn" href={`https://pump.fun/coin/${mint}`} target="_blank" rel="noreferrer">
          View on pump.fun
        </a>
      </div>
    </div>
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

function Field({ id, label, hint, children }: { id: string; label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="field">
      <label htmlFor={id}>
        {label}
        {hint && <small>{hint}</small>}
      </label>
      {children}
    </div>
  );
}
