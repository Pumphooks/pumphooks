"use client";

import { useMemo, useState } from "react";
import { PublicKey } from "@solana/web3.js";
import { v1 } from "@pump-hooks/sdk";
import { analyze, measuredFromBench, type Analysis } from "@pump-hooks/hook-analyzer";
import { bench } from "@/lib/artifacts";

type Env = v1.EnvironmentId;
type Field = { key: string; label: string; kind: "key" | "int" | "time"; hint?: string; def: string };

// Builtins with a config form here. Others (conditional, dynamic-fee, price-limit) need richer editors.
const FORMS: Record<number, { fields: Field[]; encode: (v: Record<string, string>) => Uint8Array }> = {
  1: {
    fields: [{ key: "a", label: "allowed wallet", kind: "key", def: "" }],
    encode: (v) => v1.configs.allowlistInline(v1.Target.Actor, [key(v.a)]),
  },
  9: {
    fields: [{ key: "a", label: "denied wallet", kind: "key", def: "" }],
    encode: (v) => v1.configs.denylist(v1.Target.Actor, [key(v.a)]),
  },
  6: {
    fields: [
      { key: "asset", label: "asset mint", kind: "key", hint: "empty = native SOL", def: "" },
      { key: "max", label: "max amount (base units)", kind: "int", def: "1000000000" },
    ],
    encode: (v) => v1.configs.maxSize(key(v.asset), BigInt(v.max || "0")),
  },
  2: {
    fields: [
      { key: "asset", label: "asset mint", kind: "key", hint: "empty = native SOL", def: "" },
      { key: "limit", label: "limit per period", kind: "int", def: "5000000000" },
      { key: "period", label: "period (seconds)", kind: "int", def: "86400" },
    ],
    encode: (v) => v1.configs.spendingLimit(key(v.asset), BigInt(v.limit || "0"), Number(v.period || "0")),
  },
  7: {
    fields: [
      { key: "nb", label: "not before (unix)", kind: "time", def: "0" },
      { key: "na", label: "not after (unix)", kind: "time", def: "0" },
    ],
    encode: (v) => v1.configs.timeWindow({ notBefore: BigInt(v.nb || "0"), notAfter: BigInt(v.na || "0") }),
  },
  3: {
    fields: [
      { key: "delay", label: "delay (seconds)", kind: "int", def: "3600" },
      { key: "nb", label: "not before (unix)", kind: "time", def: "0" },
    ],
    encode: (v) => v1.configs.timelock(Number(v.delay || "0"), BigInt(v.nb || "0")),
  },
  14: { fields: [{ key: "max", label: "max per slot", kind: "int", def: "1" }], encode: (v) => v1.configs.slotLimit(Number(v.max || "0")) },
  13: {
    fields: [{ key: "net", label: "gatekeeper network", kind: "key", def: "" }],
    encode: (v) => v1.configs.credential(v1.Target.Actor, key(v.net)),
  },
  8: { fields: [], encode: () => v1.configs.accounting() },
  4: {
    fields: [
      { key: "t", label: "threshold", kind: "int", def: "2" },
      { key: "mask", label: "member mask", kind: "int", hint: "bit per member", def: "3" },
    ],
    encode: (v) => v1.configs.multisig(Number(v.t || "0"), Number(v.mask || "0")),
  },
  11: { fields: [{ key: "bps", label: "max price impact (bps)", kind: "int", def: "100" }], encode: (v) => v1.configs.priceImpact(Number(v.bps || "0")) },
};

function key(s: string): Uint8Array {
  return s.trim() ? new PublicKey(s.trim()).toBytes() : new Uint8Array(32);
}

interface Entry {
  uid: number;
  builtin: number;
  callbacks: number[];
  mandatory: boolean;
  values: Record<string, string>;
}

const bits = (mask: number) => Array.from({ length: 16 }, (_, i) => i).filter((i) => mask & (1 << i));
const measured = measuredFromBench(bench as never);
const hex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");

export function Playground() {
  const [env, setEnv] = useState<Env>(v1.Environment.Router);
  const [entries, setEntries] = useState<Entry[]>([]);
  const [members, setMembers] = useState("0");
  const [delay, setDelay] = useState("0");
  const [uid, setUid] = useState(1);

  const available = Object.keys(FORMS)
    .map(Number)
    .filter((id) => v1.supportedCallbacks(id, env) !== 0);

  const add = (id: number) => {
    setEntries((es) => [
      ...es,
      { uid, builtin: id, callbacks: bits(v1.supportedCallbacks(id, env)), mandatory: false, values: Object.fromEntries(FORMS[id].fields.map((f) => [f.key, f.def])) },
    ]);
    setUid(uid + 1);
  };
  const update = (u: number, f: (e: Entry) => Entry) => setEntries((es) => es.map((e) => (e.uid === u ? f(e) : e)));

  const result = useMemo((): { bytes: Uint8Array; a: Analysis } | { error: string } => {
    try {
      const bytes = v1.encodeStack(
        env,
        entries.map((e) => ({ builtin: e.builtin, callbacks: e.callbacks, mandatory: e.mandatory, config: FORMS[e.builtin].encode(e.values) })),
      );
      return { bytes, a: analyze(bytes, { members: Number(members) || undefined, policyDelaySecs: BigInt(delay || "0"), measuredCu: measured }) };
    } catch (e) {
      return { error: (e as Error).message };
    }
  }, [env, entries, members, delay]);

  return (
    <div className="grid-main">
      <div className="col">
        <div className="section">
          <div className="section-h">
            <span className="n">01</span>
            <h3>Environment</h3>
          </div>
          <div className="chips">
            {v1.ENVIRONMENT_NAMES.map((n, i) => (
              <button
                key={n}
                className="chip"
                aria-pressed={env === i}
                onClick={() => {
                  setEnv(i as Env);
                  setEntries([]);
                }}
              >
                <span>{n}</span>
                <span className="c">{v1.CALLBACK_NAMES[i as Env].length} callbacks</span>
              </button>
            ))}
          </div>
        </div>

        <div className="section">
          <div className="section-h">
            <span className="n">02</span>
            <h3>Add builtins</h3>
            <span className="aside">{entries.length} / {v1.MAX_STACK}</span>
          </div>
          <div className="tags">
            {available.map((id) => (
              <button key={id} className="btn small outline" onClick={() => add(id)} disabled={entries.length >= v1.MAX_STACK}>
                + {v1.BUILTIN_SLUGS[id]}
              </button>
            ))}
          </div>
        </div>

        <div className="section">
          <div className="section-h">
            <span className="n">03</span>
            <h3>Stack</h3>
          </div>
          {entries.length === 0 ? <p className="mute">Empty. Add a builtin above.</p> : null}
          {entries.map((e, i) => (
            <div key={e.uid} className="config-card">
              <header>
                #{i} {v1.BUILTIN_SLUGS[e.builtin]}
                <span className="p">
                  <button className="icon-btn" style={{ color: "inherit", background: "none", border: 0, cursor: "pointer" }} onClick={() => setEntries((es) => es.filter((x) => x.uid !== e.uid))}>
                    remove
                  </button>
                </span>
              </header>
              <div className="body">
                <div className="field">
                  <label>callbacks</label>
                  <div className="tags">
                    {bits(v1.supportedCallbacks(e.builtin, env)).map((cb) => (
                      <label key={cb} className="tag" style={{ cursor: "pointer" }}>
                        <input
                          type="checkbox"
                          checked={e.callbacks.includes(cb)}
                          onChange={(ev) => update(e.uid, (x) => ({ ...x, callbacks: ev.target.checked ? [...x.callbacks, cb].sort() : x.callbacks.filter((c) => c !== cb) }))}
                        />
                        {v1.CALLBACK_NAMES[env][cb]}
                      </label>
                    ))}
                  </div>
                </div>
                <div className="field">
                  <label>mandatory</label>
                  <input type="checkbox" checked={e.mandatory} onChange={(ev) => update(e.uid, (x) => ({ ...x, mandatory: ev.target.checked }))} />
                </div>
                {FORMS[e.builtin].fields.map((f) => (
                  <div className="field" key={f.key}>
                    <label>
                      {f.label}
                      {f.hint ? <small>{f.hint}</small> : null}
                    </label>
                    <input
                      className="input"
                      value={e.values[f.key]}
                      placeholder={f.kind === "key" ? "base58 address" : "0"}
                      onChange={(ev) => update(e.uid, (x) => ({ ...x, values: { ...x.values, [f.key]: ev.target.value } }))}
                    />
                  </div>
                ))}
              </div>
            </div>
          ))}
          <div className="field">
            <label>
              policy change delay (s)<small>how long a stack update waits before it applies</small>
            </label>
            <input className="input" value={delay} onChange={(ev) => setDelay(ev.target.value)} />
          </div>
          {env === v1.Environment.Account ? (
            <div className="field">
              <label>account members</label>
              <input className="input" value={members} onChange={(ev) => setMembers(ev.target.value)} />
            </div>
          ) : null}
        </div>
      </div>

      <div className="col">
        <div className="section-h">
          <span className="n">04</span>
          <h3>Analysis</h3>
          <span className="aside">kernel rules + composition checks</span>
        </div>
        {"error" in result ? (
          <div className="notice deny">
            <b>Cannot encode:</b> {result.error}
          </div>
        ) : (
          <Report bytes={result.bytes} a={result.a} env={env} />
        )}
      </div>
    </div>
  );
}

function Report({ bytes, a, env }: { bytes: Uint8Array; a: Analysis; env: Env }) {
  const h = hex(bytes);
  return (
    <div>
      <div className="kv">
        <div>
          <div className="k">Kernel</div>
          <div className={`v ${a.valid ? "ok" : "deny"}`} title={a.kernel}>{a.kernel}</div>
        </div>
        <div>
          <div className="k">Static CU</div>
          <div className="v">{a.compute.estimate.toLocaleString("en-US")}</div>
        </div>
        <div>
          <div className="k">Measured CU</div>
          <div
            className="v"
            title={
              a.compute.measured === undefined
                ? `not every builtin here is benched (measured: ${Object.keys(measured).join(", ")})`
                : "sum of measured per-builtin marginals from bench/results.json (token-hooks execute)"
            }
          >
            {a.compute.measured === undefined ? "not benched" : a.compute.measured.toLocaleString("en-US")}
          </div>
        </div>
        <div>
          <div className="k">Bytes</div>
          <div className="v">{bytes.length}</div>
        </div>
      </div>

      <div className="section-h" style={{ marginTop: 20 }}>
        <h3>Diagnostics</h3>
      </div>
      {a.diagnostics.length === 0 ? <p className="mute">No findings.</p> : null}
      {a.diagnostics.map((d, i) => (
        <div key={i} className={`notice ${d.severity === "error" ? "deny" : d.severity === "warning" ? "warn" : ""}`}>
          <b>{d.code}</b> {d.message}
        </div>
      ))}

      <div className="section-h" style={{ marginTop: 20 }}>
        <h3>Enforcement</h3>
      </div>
      <table className="table">
        <tbody>
          {a.enforcement.map((e) => (
            <tr key={e.callback}>
              <td>{e.callback}</td>
              <td className={e.class === "router" ? "warn" : e.class === "unavailable" ? "deny" : undefined}>{e.class}</td>
              <td className="mute">{e.entries.length ? `entries ${e.entries.join(", ")}` : "—"}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <details open>
        <summary>Stack bytes · {v1.ENVIRONMENT_NAMES[env]}</summary>
        <pre className="code" style={{ whiteSpace: "pre-wrap", wordBreak: "break-all" }}>{`pumphooks simulate --stack ${h}`}</pre>
      </details>
    </div>
  );
}
