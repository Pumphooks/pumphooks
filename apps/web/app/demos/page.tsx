import type { Metadata } from "next";
import { demos, devnetRouter as dn, short } from "@/lib/artifacts";
import { PageHead, Provenance, Section } from "@/components/Provenance";
import { Demos } from "@/components/Demos";

export const metadata: Metadata = { title: "Demos" };

const tx = (sig: string) => `https://explorer.solana.com/tx/${sig}?cluster=devnet`;
const addr = (a: string) => `https://explorer.solana.com/address/${a}?cluster=devnet`;

export default function DemosPage() {
  const steps = demos.demos.reduce((n, d) => n + d.steps.length, 0);
  return (
    <main className="page">
      <PageHead n="12" crumb="Demos" title="One demo per engine, run against the real programs.">
        Each demo builds its transactions with the <code>@pump-hooks/sdk</code> v1 builders and executes them in LiteSVM against the compiled
        programs: the registry publishes the hook, the engine installs it, and every step shows what the program actually did, including the
        rejections. Demo B also ran on devnet; its transactions are linked below. Nothing here is on mainnet.
      </PageHead>
      <Section n="01" title="Recorded runs (LiteSVM)" aside={`${demos.demos.length} demos, ${steps} steps`}>
        <Demos />
        <Provenance p={demos} />
      </Section>
      <Section n="02" title="Demo B on devnet" aside={`ran ${dn.generatedAt.slice(0, 10)}`}>
        <p style={{ marginTop: 0 }}>
          The same max-size route policy, created on the devnet deployment of <a href={addr(dn.programs["hook-router"])}>hook-router</a> with the hook
          published in <a href={addr(dn.programs["hook-registry"])}>hook-registry</a>, trading the devnet PumpSwap pool{" "}
          <a href={addr(dn.pool)}>{short(dn.pool)}</a>. The rejected buy was sent without preflight so its failure is recorded on chain.
        </p>
        <table className="table">
          <thead>
            <tr>
              <th>Step</th>
              <th>Status</th>
              <th>CU</th>
              <th>Outcome</th>
              <th>Transaction</th>
            </tr>
          </thead>
          <tbody>
            {dn.steps.map((s) => (
              <tr key={s.signature}>
                <td>{s.title}</td>
                <td className={s.expect === "ok" ? "ok" : "deny"}>
                  {s.expect === "ok" ? "ok" : "rejected"}
                  {s.ok ? "" : " (unexpected)"}
                </td>
                <td>{s.computeUnits?.toLocaleString("en-US") ?? "—"}</td>
                <td style={{ wordBreak: "break-word" }}>{s.outcome}</td>
                <td>
                  <a href={tx(s.signature)} target="_blank" rel="noopener noreferrer">
                    {short(s.signature, 6)} ↗
                  </a>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <details>
          <summary className="mute" style={{ cursor: "pointer", fontSize: 13 }}>Setup transactions</summary>
          <ul style={{ fontFamily: "var(--mono)", fontSize: 12 }}>
            {dn.setup.map((s) => (
              <li key={s.label}>
                {s.label}
                {s.signature ? (
                  <>
                    {" "}
                    <a href={tx(s.signature)} target="_blank" rel="noopener noreferrer">
                      {short(s.signature, 6)} ↗
                    </a>
                  </>
                ) : null}
              </li>
            ))}
            <li>
              route policy <a href={addr(dn.policy)}>{short(dn.policy)}</a>, hook version <a href={addr(dn.hookVersion)}>{short(dn.hookVersion)}</a>
            </li>
          </ul>
        </details>
        <p className="mute" style={{ fontSize: 13 }}>
          Recorded from commit {dn.commit.slice(0, 7)} with <code>{dn.command}</code>.
        </p>
      </Section>
      <Section n="03" title="Run it yourself">
        <pre className="code">{`scripts/build_programs.sh && scripts/dump_programs.sh   # router demo needs the PumpSwap fixtures
pnpm -r --filter "./packages/**" build
pumphooks demo                 # all four; or: pumphooks demo token|router|pool|account [--json]
pnpm --filter @pump-hooks/demos test`}</pre>
        <p className="mute" style={{ fontSize: 13 }}>
          The router demo uses PumpSwap binaries dumped from devnet and a cloned devnet pool. Its last step calls PumpSwap directly to show that
          router hooks are bypassable: PumpSwap has no native hooks.
        </p>
      </Section>
    </main>
  );
}
