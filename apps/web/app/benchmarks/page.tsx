import type { Metadata } from "next";
import { compare, marginals, EMPTY_CASE, type Results } from "@pump-hooks/hook-bench";
import { baseline, bench } from "@/lib/artifacts";
import { PageHead, Section } from "@/components/Provenance";

export const metadata: Metadata = { title: "Benchmarks" };

const fmt = (n: number) => n.toLocaleString("en-US");

export default function BenchmarksPage() {
  const results = bench as unknown as Results;
  const empty = results.unit.find((m) => m.name === EMPTY_CASE)!;
  const m = marginals(results);
  const diffs = compare(baseline as unknown as Results, results, 5);
  const tx = results.transaction;
  const plain = tx[0]?.compute_units ?? 0;
  const max = Math.max(...tx.map((t) => t.compute_units));
  return (
    <main className="page">
      <PageHead n="07" crumb="Benchmarks" title="Compute, measured.">
        Compute units of the compiled <code>token-hooks</code> program, measured with Mollusk (one instruction) and LiteSVM (full Token-2022
        transfers). Every case below ran and succeeded; each builtin is also checked to reject under a denying config, so a number is never the cost
        of skipping the check.
      </PageHead>

      <Section n="01" title="Token-2022 transfer" aside={results.runtime}>
        <table className="table">
          <thead>
            <tr>
              <th>Case</th>
              <th>CU</th>
              <th>Over plain transfer</th>
              <th style={{ width: "40%" }} />
            </tr>
          </thead>
          <tbody>
            {tx.map((t) => (
              <tr key={t.name}>
                <td>{t.name}</td>
                <td>{fmt(t.compute_units)}</td>
                <td>{t.compute_units === plain ? "—" : `+${fmt(t.compute_units - plain)}`}</td>
                <td>
                  <div style={{ height: 8, background: "var(--ink)", width: `${(100 * t.compute_units) / max}%` }} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <p className="mute" style={{ fontSize: 13 }}>
          The hook adds Token-2022&apos;s transfer-hook CPI, extra-account resolution and the policy load. Builtin evaluation itself is small next to
          that, as the per-builtin table shows.
        </p>
      </Section>

      <Section n="02" title="Per builtin (token-hooks Execute)" aside={`empty stack ${fmt(empty.compute_units)} CU`}>
        <table className="table">
          <thead>
            <tr>
              <th>Stack</th>
              <th>Measured CU</th>
              <th>Above empty</th>
              <th>Kernel static estimate</th>
            </tr>
          </thead>
          <tbody>
            {m.map((x) => (
              <tr key={x.name}>
                <td>{x.name}</td>
                <td>{fmt(x.marginal + empty.compute_units)}</td>
                <td>+{fmt(x.marginal)}</td>
                <td className="mute">{x.kernelEstimate === null ? "—" : fmt(x.kernelEstimate)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <p className="mute" style={{ fontSize: 13 }}>
          Static estimates are deliberately conservative upper bounds used for compute budgeting before simulation.
        </p>
      </Section>

      <Section n="03" title="Regression gate" aside="tolerance 5%">
        <table className="table">
          <thead>
            <tr>
              <th>Case</th>
              <th>Baseline</th>
              <th>Current</th>
              <th>Status</th>
            </tr>
          </thead>
          <tbody>
            {diffs.map((d) => (
              <tr key={d.name}>
                <td>{d.name}</td>
                <td>{d.baseline === null ? "—" : fmt(d.baseline)}</td>
                <td>{d.current === null ? "—" : fmt(d.current)}</td>
                <td className={d.status === "regressed" || d.status === "failed" || d.status === "missing" ? "deny" : d.status === "improved" ? "ok" : "mute"}>
                  {d.status}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Section>

      <Section n="04" title="Binaries and reproduction">
        <div className="kv">
          {results.programs.map(([name, hash]) => (
            <div key={name} style={{ gridColumn: "1 / -1" }}>
              <div className="k">{name}.so sha256</div>
              <div className="v" title={hash}>{hash}</div>
            </div>
          ))}
        </div>
        <pre className="code">{`scripts/build_programs.sh programs/token-hooks
cargo run --release -p hook-bench        # writes bench/results.json and docs/benchmarks/report.md
cargo test --release -p hook-bench       # fails on a >5% regression against bench/baseline.json
pumphooks bench --no-run                 # the table above, in a terminal`}</pre>
      </Section>
    </main>
  );
}
