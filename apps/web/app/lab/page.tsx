import type { Metadata } from "next";
import { lab } from "@/lib/artifacts";
import { PageHead, Provenance, Section } from "@/components/Provenance";
import { TraceViewer } from "@/components/TraceViewer";

export const metadata: Metadata = { title: "Execution lab" };

export default function LabPage() {
  return (
    <main className="page">
      <PageHead n="06" crumb="Execution lab" title="Every transaction, taken apart.">
        Traces recorded by <code>@pump-hooks/execution-lab</code> running the compiled token-wrapper, token-hooks and example hook programs in
        LiteSVM. Fields marked <i>observed</i> come from the runtime; <i>reconstructed</i> fields (hook order) are derived from the stack and logs.
      </PageHead>
      <Section n="01" title="Recorded traces" aside={`${lab.traces.length} transactions`}>
        <TraceViewer traces={lab.traces} />
        <Provenance p={lab} />
      </Section>
      <Section n="02" title="Run it yourself">
        <pre className="code">{`scripts/build_programs.sh
pnpm --filter @pump-hooks/execution-lab test          # traces, diffs, replayable repro bundles
pumphooks simulate --program target/deploy/anti_jit_hook.so --env router --callback before_swap --amount 5`}</pre>
        <p className="mute" style={{ fontSize: 13 }}>
          Reproduction bundles (<code>exportRepro</code>) pin each program by sha256 and replay to the same status, compute units and log digest.
        </p>
      </Section>
    </main>
  );
}
