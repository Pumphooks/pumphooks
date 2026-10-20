import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { Callback, CONFIG_PRESETS, CONFIG_SCHEMAS, PUMP_HOOKS_PROGRAM_ID, type ModuleId } from "@pump-hooks/sdk";
import { DitherStage } from "@/components/DitherStage";
import { RegistryPanel } from "@/components/RegistryPanel";
import { CALLBACK_NOTES, CAPABILITY_INFO, CATALOG, catalogEntry, sourceUrl } from "@/lib/catalog";
import { sol } from "@/lib/format";

export function generateStaticParams() {
  return CATALOG.map((c) => ({ slug: c.slug }));
}

export async function generateMetadata({ params }: { params: Promise<{ slug: string }> }): Promise<Metadata> {
  const c = catalogEntry((await params).slug);
  return { title: c ? c.name : "Hook" };
}

const ALL_CAPS = Object.keys(CAPABILITY_INFO);

export default async function HookDetail({ params }: { params: Promise<{ slug: string }> }) {
  const c = catalogEntry((await params).slug);
  if (!c) notFound();
  const schema = CONFIG_SCHEMAS[c.moduleId as ModuleId] ?? [];
  const preset = CONFIG_PRESETS[c.moduleId as ModuleId] ?? {};
  const src = sourceUrl(c.source);
  return (
    <main className="page">
      <div className="page-head">
        <DitherStage mode="banner" />
        <div className="crumb">
          <Link href="/hooks">02 / Hooks</Link> / <b>{c.slug}</b>
        </div>
        <h1 className="h2">{c.name}</h1>
        <p className="lede">{c.summary}</p>
        <div className="tags" style={{ marginTop: 16 }}>
          {c.categories.map((k) => (
            <span className="tag" key={k}>{k}</span>
          ))}
          <span className={`tag ${c.available ? "ink" : "warn"}`}>{c.available ? (c.runtime ? "runtime module" : "config-only module") : "reserved"}</span>
          <span className="tag warn">unaudited</span>
        </div>
      </div>

      <div className="grid-main">
        <div className="col">
          <section className="section">
            <div className="section-h">
              <span className="n">01</span>
              <h3>Behavior</h3>
            </div>
            <ul style={{ margin: 0, paddingLeft: 18, display: "grid", gap: 8, fontSize: 14.5 }}>
              {c.behavior.map((b) => (
                <li key={b}>{b}</li>
              ))}
            </ul>
            {c.limitations.map((l) => (
              <div className="notice warn" key={l} style={{ marginTop: 14 }}>
                <b>Limitation.</b> {l}
              </div>
            ))}
          </section>

          <section className="section">
            <div className="section-h">
              <span className="n">02</span>
              <h3>Callbacks</h3>
              <span className="aside">ABI v1 · wire values frozen</span>
            </div>
            <table className="table">
              <thead>
                <tr>
                  <th>#</th>
                  <th>Callback</th>
                  <th>Subscribable</th>
                  <th>Default</th>
                  <th>Semantics</th>
                </tr>
              </thead>
              <tbody>
                {Array.from({ length: 12 }, (_, i) => {
                  const n = Callback[i];
                  const sub = c.callbacks.includes(n);
                  return (
                    <tr key={n} className={i >= 6 ? "faint" : undefined}>
                      <td>{i}</td>
                      <td>{n}</td>
                      <td className={sub ? "ok" : "faint"}>{i >= 6 ? "reserved" : sub ? "yes" : "—"}</td>
                      <td>{c.defaultCallbacks.includes(n) ? "●" : ""}</td>
                      <td style={{ fontFamily: "var(--display)", fontSize: 13 }}>{CALLBACK_NOTES[n]}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </section>

          <section className="section">
            <div className="section-h">
              <span className="n">03</span>
              <h3>Capabilities</h3>
              <span className="aside">granted at install; enforced by the executor</span>
            </div>
            <table className="table">
              <tbody>
                {ALL_CAPS.map((k) => {
                  const has = c.capabilities.includes(k);
                  return (
                    <tr key={k} className={has ? undefined : "faint"}>
                      <td style={{ width: 24 }}>{has ? "■" : "□"}</td>
                      <td style={{ width: 210 }}>{k}</td>
                      <td style={{ fontFamily: "var(--display)", fontSize: 13 }}>{CAPABILITY_INFO[k]}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </section>

          <section className="section">
            <div className="section-h">
              <span className="n">04</span>
              <h3>Config schema</h3>
              <span className="aside">48 bytes · little-endian</span>
            </div>
            {schema.length === 0 ? (
              <p className="mute">No slot config. {c.slug === "policy" ? "Rules are separate Policy accounts created in the Composer." : ""}</p>
            ) : (
              <table className="table">
                <thead>
                  <tr>
                    <th>Field</th>
                    <th>Type</th>
                    <th>Offset</th>
                    <th>Unit</th>
                    <th>Preset</th>
                    <th>Description</th>
                  </tr>
                </thead>
                <tbody>
                  {schema.map((f) => {
                    const p = preset[f.name];
                    return (
                      <tr key={f.name}>
                        <td>{f.name}</td>
                        <td>{f.type}</td>
                        <td>{f.offset}</td>
                        <td>{f.unit}</td>
                        <td>{p === undefined ? "—" : f.unit === "lamports" ? `${sol(BigInt(p as bigint))} SOL` : String(p)}</td>
                        <td style={{ fontFamily: "var(--display)", fontSize: 13 }}>{f.description}</td>
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
              <span className="n">05</span>
              <h3>Identity</h3>
            </div>
            <div className="kv">
              <div>
                <div className="k">Module ID</div>
                <div className="v">{c.moduleId} (0x{c.moduleId.toString(16).padStart(4, "0")})</div>
              </div>
              <div>
                <div className="k">Version</div>
                <div className="v">v{c.version}</div>
              </div>
              <div>
                <div className="k">Developer</div>
                <div className="v">{c.developer}</div>
              </div>
              <div>
                <div className="k">Program</div>
                <div className="v" title={PUMP_HOOKS_PROGRAM_ID.toBase58()}>{PUMP_HOOKS_PROGRAM_ID.toBase58().slice(0, 10)}… (built-in)</div>
              </div>
              <div>
                <div className="k">Priority range</div>
                <div className="v">
                  {c.priority.min}–{c.priority.max} · default {c.priority.default}
                </div>
              </div>
              <div>
                <div className="k">Markets</div>
                <div className="v">{c.compatibleMarkets.join(" · ")}</div>
              </div>
              <div>
                <div className="k">Source</div>
                <div className="v" title={c.source}>{src ? <a href={src}>{c.source}</a> : c.source}</div>
              </div>
              <div>
                <div className="k">Verification</div>
                <div className="v" title={c.verification}>{c.verification}</div>
              </div>
              <div>
                <div className="k">Audit</div>
                <div className="v warn">{c.audit}</div>
              </div>
            </div>
          </section>
          <section className="section">
            <div className="section-h">
              <span className="n">06</span>
              <h3>Registry</h3>
            </div>
            <RegistryPanel moduleId={c.moduleId} />
          </section>
          {c.available && (
            <Link className="btn" href="/hook" style={{ width: "100%", marginTop: 10 }}>
              Install on a Pump market <span className="chev">›</span>
            </Link>
          )}
        </div>
      </div>
    </main>
  );
}
