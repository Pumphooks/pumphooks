import type { Metadata } from "next";
import { security } from "@/lib/artifacts";
import { PageHead, Provenance, Section } from "@/components/Provenance";

export const metadata: Metadata = { title: "Security lab" };

const KIND: Record<string, string> = { rust: "Rust test", "fuzz-attack": "attack", "fuzz-invariant": "invariant" };

export default function SecurityLabPage() {
  const f = security.fuzz;
  return (
    <main className="page">
      <PageHead n="08" crumb="Security lab" title="Attacked on purpose.">
        Stateful fuzzing, invariants and adversarial operations against the compiled programs, plus the tests behind each attack category.
      </PageHead>

      <div className="notice deny" style={{ marginTop: 22 }}>
        <b>Not audited.</b> {security.disclaimer} Nothing in Pump Hooks has had an external audit. On mainnet: pump-hooks, the hook registry, the pool engine, smart accounts and the token wrapper.
      </div>

      <Section n="01" title="Stateful fuzzing" aside={f.target}>
        <div className="kv">
          <div>
            <div className="k">Seed</div>
            <div className="v">{f.seed}</div>
          </div>
          <div>
            <div className="k">Runs × steps</div>
            <div className="v">
              {f.runs} × {f.steps}
            </div>
          </div>
          <div>
            <div className="k">Operations executed</div>
            <div className="v">{f.executed}</div>
          </div>
          <div>
            <div className="k">Succeeded / refused</div>
            <div className="v">
              {f.outcomes.ok} / {f.outcomes.failed}
            </div>
          </div>
          <div style={{ gridColumn: "span 2" }}>
            <div className="k">Invariant violations</div>
            <div className={`v ${f.failure ? "deny" : "ok"}`}>{f.failure ? `${f.failure.invariant}: ${f.failure.message}` : "none in this run"}</div>
          </div>
        </div>
        <div className="grid-2">
          <div className="col">
            <div className="section-h">
              <h3>Coverage</h3>
            </div>
            <table className="table">
              <tbody>
                {Object.entries(f.coverage)
                  .sort((a, b) => b[1] - a[1])
                  .map(([k, v]) => (
                    <tr key={k}>
                      <td>{k}</td>
                      <td>{v}</td>
                    </tr>
                  ))}
              </tbody>
            </table>
          </div>
          <div className="col">
            <div className="section-h">
              <h3>Invariants checked after every step</h3>
            </div>
            <ol style={{ margin: 0, paddingLeft: 18, fontFamily: "var(--mono)", fontSize: 12 }}>
              {security.invariants.map((i) => (
                <li key={i} style={{ padding: "4px 0" }}>
                  {i}
                </li>
              ))}
            </ol>
            <p className="mute" style={{ fontSize: 13 }}>
              A failing sequence is shrunk to a minimal reproduction. The test suite also forges unbacked supply to prove the reserve invariant
              catches it.
            </p>
          </div>
        </div>
        <Provenance p={security} />
      </Section>

      <Section n="02" title="Adversarial operations" aside="each must fail and change nothing">
        <table className="table">
          <tbody>
            {Object.entries(security.attacks).map(([k, v]) => (
              <tr key={k}>
                <td style={{ width: 200 }}>{k}</td>
                <td style={{ fontFamily: "var(--display)", fontSize: 13 }}>{v}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </Section>

      <Section n="03" title="Attack categories" aside={`${security.categories.length} categories`}>
        <div className="module-list">
          {security.categories.map((c, i) => (
            <div className="module" key={c.id}>
              <span className="idx">{String(i + 1).padStart(2, "0")}</span>
              <div>
                <div className="name">{c.title}</div>
                <div className="desc">{c.threat}</div>
                <div className="tags" style={{ marginTop: 8 }}>
                  {c.evidence.map((e) => (
                    <span key={e.test} className="tag" title={`${KIND[e.kind] ?? e.kind}, ${e.level}`}>
                      {e.test}
                    </span>
                  ))}
                </div>
                {c.gaps ? (
                  <div className="desc warn" style={{ marginTop: 8 }}>
                    Gap: {c.gaps}
                  </div>
                ) : null}
              </div>
              <span className="tag">{c.evidence.length} tests</span>
            </div>
          ))}
        </div>
      </Section>

      <Section n="04" title="Run it yourself">
        <pre className="code">{`scripts/build_programs.sh
pumphooks security-check --runs 8 --out security-report.md
pumphooks security-check --stack <hex>      # plus diagnostics for your own stack`}</pre>
      </Section>
    </main>
  );
}
