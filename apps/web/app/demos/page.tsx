import type { Metadata } from "next";
import { demos } from "@/lib/artifacts";
import { PageHead, Provenance, Section } from "@/components/Provenance";
import { Demos } from "@/components/Demos";

export const metadata: Metadata = { title: "Demos" };

export default function DemosPage() {
  const steps = demos.demos.reduce((n, d) => n + d.steps.length, 0);
  return (
    <main className="page">
      <PageHead n="12" crumb="Demos" title="One demo per engine, run against the real programs.">
        Each demo builds its transactions with the <code>@pump-hooks/sdk</code> v1 builders and executes them in LiteSVM against the compiled
        programs: the registry publishes the hook, the engine installs it, and every step shows what the program actually did, including the
        rejections. These are local runs, not devnet or mainnet transactions.
      </PageHead>
      <Section n="01" title="Recorded runs" aside={`${demos.demos.length} demos, ${steps} steps`}>
        <Demos />
        <Provenance p={demos} />
      </Section>
      <Section n="02" title="Run it yourself">
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
