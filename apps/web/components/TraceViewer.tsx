"use client";

import { useState } from "react";
import { v1 } from "@pump-hooks/sdk";
import { short, type TraceJson } from "@/lib/artifacts";

const b64 = (s: string) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));

function delta(before: string, after: string) {
  const d = BigInt(after) - BigInt(before);
  return d === 0n ? "" : `${d > 0n ? "+" : ""}${d}`;
}

/** With `index`, the caller selects the trace and the chips are not shown. */
export function TraceViewer({ traces, index }: { traces: TraceJson[]; index?: number }) {
  const [own, setI] = useState(0);
  const i = index ?? own;
  const t = traces[i];
  const labels = new Map(t.accountDiffs.filter((d) => d.label).map((d) => [d.address, d.label!]));
  const name = (a: string) => labels.get(a) ?? short(a);
  const ret = t.returnData ? v1.decodePolicyResult(b64(t.returnData.data)) : null;
  return (
    <div>
      {index === undefined ? (
        <div className="chips" role="tablist">
          {traces.map((x, k) => (
            <button key={k} className="chip" aria-pressed={k === i} onClick={() => setI(k)}>
              <span>{x.label}</span>
              <span className={x.status === "success" ? "ok" : "deny"}>{x.status === "success" ? "ok" : "failed"}</span>
            </button>
          ))}
        </div>
      ) : null}

      <div className="section">
        {t.note ? <p style={{ marginTop: 0 }}>{t.note}</p> : null}
        <div className="kv">
          <div>
            <div className="k">Status</div>
            <div className={`v ${t.status === "success" ? "ok" : "deny"}`}>{t.status}</div>
          </div>
          <div>
            <div className="k">Compute units</div>
            <div className="v">{Number(t.computeUnits.total).toLocaleString("en-US")}</div>
          </div>
          <div>
            <div className="k">Invocations</div>
            <div className="v">{t.invocations.length}</div>
          </div>
          <div>
            <div className="k">Accounts changed</div>
            <div className="v">{t.accountDiffs.filter((d) => d.changedRanges.length || d.lamportsBefore !== d.lamportsAfter).length}</div>
          </div>
          {t.error ? (
            <div style={{ gridColumn: "1 / -1" }}>
              <div className="k">Error</div>
              <div className="v deny" title={t.error}>{t.error}</div>
            </div>
          ) : null}
          {ret ? (
            <div style={{ gridColumn: "1 / -1" }}>
              <div className="k">Hook decision (return data)</div>
              <div className={`v ${ret.decision === "allow" ? "ok" : "deny"}`}>
                {ret.decision}
                {ret.decision === "reject" ? ` · reason ${ret.reason}${ret.reason >= v1.CUSTOM_REASON_BASE ? " (hook-defined)" : ""}` : ""}
              </div>
            </div>
          ) : null}
        </div>
      </div>

      <div className="grid-2">
        <div className="col">
          <div className="section-h">
            <h3>Invocations</h3>
            <span className="aside">{t.provenance.invocations?.provenance}</span>
          </div>
          <table className="table">
            <thead>
              <tr>
                <th>Depth</th>
                <th>Program</th>
                <th>CU</th>
                <th>Result</th>
              </tr>
            </thead>
            <tbody>
              {t.invocations.map((v, k) => (
                <tr key={k}>
                  <td>{"·".repeat(v.depth - 1)}{v.depth}</td>
                  <td title={v.programId}>{v.program ?? short(v.programId)}</td>
                  <td>{v.consumed?.toLocaleString("en-US") ?? "—"}</td>
                  <td className={v.result === "success" ? "ok" : "deny"}>{v.result}</td>
                </tr>
              ))}
            </tbody>
          </table>

          <div className="section-h" style={{ marginTop: 22 }}>
            <h3>Token balances</h3>
            <span className="aside">{t.provenance.tokenBalances?.provenance}</span>
          </div>
          {t.tokenBalances.length ? (
            <table className="table">
              <tbody>
                {t.tokenBalances.map((b) => (
                  <tr key={b.account}>
                    <td title={b.account}>{name(b.account)}</td>
                    <td>{b.before} → {b.after}</td>
                    <td className={b.delta.startsWith("-") ? "deny" : "ok"}>{b.delta === "0" ? "" : b.delta}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <p className="mute">No token balance changed.</p>
          )}

          <div className="section-h" style={{ marginTop: 22 }}>
            <h3>Hook steps</h3>
            <span className="aside">{t.provenance.hookSteps?.provenance}</span>
          </div>
          {t.hookSteps.length ? (
            <table className="table">
              <tbody>
                {t.hookSteps.map((h, k) => (
                  <tr key={k}>
                    <td>{h.engine}.{h.callback}</td>
                    <td>#{h.entry} {h.name}{h.mandatory ? " (mandatory)" : ""}</td>
                    <td className={h.result === "ran" ? "ok" : h.result === "rejected" ? "deny" : "mute"}>{h.result}{h.reason ? ` ${h.reason}` : ""}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <p className="mute">No builtin entries ran (empty stack, or not a token-hooks transfer).</p>
          )}
        </div>
        <div className="col">
          <div className="section-h">
            <h3>Account diffs</h3>
            <span className="aside">{t.provenance.accountDiffs?.provenance}</span>
          </div>
          <table className="table">
            <thead>
              <tr>
                <th>Account</th>
                <th>Lamports</th>
                <th>Data</th>
              </tr>
            </thead>
            <tbody>
              {t.accountDiffs
                .filter((d) => d.changedRanges.length || d.lamportsBefore !== d.lamportsAfter || d.created || d.closed)
                .map((d) => (
                  <tr key={d.address}>
                    <td title={d.address}>
                      {name(d.address)}
                      {d.signer ? <span className="faint"> signer</span> : null}
                      {d.created ? <span className="ok"> created</span> : null}
                      {d.closed ? <span className="deny"> closed</span> : null}
                    </td>
                    <td>{delta(d.lamportsBefore, d.lamportsAfter)}</td>
                    <td>{d.changedRanges.map(([a, b]) => `[${a},${b})`).join(" ")}</td>
                  </tr>
                ))}
            </tbody>
          </table>
          <details open>
            <summary>Logs ({t.logs.length})</summary>
            <div className="logbox">{t.logs.join("\n")}</div>
          </details>
        </div>
      </div>
    </div>
  );
}
