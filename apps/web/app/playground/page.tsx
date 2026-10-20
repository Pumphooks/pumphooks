import type { Metadata } from "next";
import { PageHead } from "@/components/Provenance";
import { Playground } from "@/components/Playground";

export const metadata: Metadata = { title: "Playground" };

export default function PlaygroundPage() {
  return (
    <main className="page">
      <PageHead n="11" crumb="Playground" title="Compose a stack, see what it does.">
        Build an ABI v1 hook stack for any engine. It is encoded with <code>@pump-hooks/sdk</code> and checked in your browser by the same rules
        the on-chain kernel applies at install (mirrored and tested against vectors generated from the Rust kernel), plus composition diagnostics:
        conflicts, dead entries, bypassable callbacks and compute.
      </PageHead>
      <Playground />
    </main>
  );
}
