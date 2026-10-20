"use client";

import { useState } from "react";
import { demos, type DemoJson } from "@/lib/artifacts";
import { TraceViewer } from "@/components/TraceViewer";

const ENGINE: Record<DemoJson["id"], string> = { token: "A · Token", router: "B · Transaction", pool: "C · Pool", account: "D · Account" };

export function Demos() {
  const [d, setD] = useState(0);
  const [s, setS] = useState(0);
  const demo = demos.demos[d];
  const pick = (k: number) => {
    setD(k);
    setS(0);
  };
  return (
    <div>
      <div className="chips" role="tablist">
        {demos.demos.map((x, k) => (
          <button key={x.id} className="chip" aria-pressed={k === d} onClick={() => pick(k)}>
            <span>{ENGINE[x.id]}</span>
            <span className={x.steps.every((st) => st.ok) ? "ok" : "deny"}>
              {x.steps.filter((st) => st.ok).length}/{x.steps.length}
            </span>
          </button>
        ))}
      </div>

      <div className="section">
        <h3 style={{ margin: "0 0 6px" }}>{demo.title}</h3>
        <p className="mute" style={{ marginTop: 0 }}>{demo.enforcement}</p>
        <details>
          <summary className="mute" style={{ cursor: "pointer", fontSize: 13 }}>Setup: {demo.setup.length} transactions, all succeeded</summary>
          <ol style={{ fontFamily: "var(--mono)", fontSize: 12 }}>
            {demo.setup.map((x, k) => (
              <li key={k}>{x}</li>
            ))}
          </ol>
        </details>
      </div>

      <table className="table">
        <thead>
          <tr>
            <th>#</th>
            <th>Step</th>
            <th>Expected</th>
            <th>Status</th>
            <th>CU</th>
            <th>Outcome</th>
          </tr>
        </thead>
        <tbody>
          {demo.steps.map((st, k) => (
            <tr key={k} className="link" onClick={() => setS(k)} style={k === s ? { background: "var(--paper-2)" } : undefined}>
              <td>{k + 1}</td>
              <td>
                <button type="button" onClick={() => setS(k)} aria-pressed={k === s} style={{ all: "unset", cursor: "pointer" }}>
                  {st.title}
                </button>
              </td>
              <td>{st.expect}</td>
              <td className={st.trace.status === "success" ? "ok" : "deny"}>
                {st.trace.status === "success" ? "ok" : "rejected"}
                {st.ok ? "" : " (unexpected)"}
              </td>
              <td>{Number(st.trace.computeUnits.total).toLocaleString("en-US")}</td>
              <td style={{ wordBreak: "break-word" }}>{st.outcome}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <div className="section">
        <p className="mute" style={{ fontSize: 13, margin: 0 }}>
          Step {s + 1}: {demo.steps[s].title}
        </p>
      </div>
      <TraceViewer key={demo.id} traces={demo.steps.map((st) => st.trace)} index={s} />
    </div>
  );
}
