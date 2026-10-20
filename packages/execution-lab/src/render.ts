import type { CpiNode, ExecutionTrace } from "./lab.js";

const short = (a: string) => (a.length > 12 ? `${a.slice(0, 4)}…${a.slice(-4)}` : a);

/** Plain-text rendering of a trace; reconstructed sections are labelled as such. */
export function renderTrace(t: ExecutionTrace): string {
  const out: string[] = [];
  out.push(`${t.label}: ${t.status.toUpperCase()}${t.error ? ` ${t.error}` : ""}`);
  out.push(`compute: ${t.computeUnits.total} CU (observed)`);
  out.push("");
  out.push("instructions (observed):");
  t.instructions.forEach((ix, i) => {
    const cu = t.computeUnits.perInstruction[i];
    out.push(`  #${i} ${ix.program ?? short(ix.programId)}${cu !== undefined ? `  ${cu} CU` : ""}`);
    const walk = (n: CpiNode, depth: number) => {
      out.push(`  ${"  ".repeat(depth)}└ ${n.program ?? short(n.programId)} [height ${n.stackHeight}]`);
      n.children.forEach((c) => walk(c, depth + 1));
    };
    (t.cpiTree[i] ?? []).forEach((n) => walk(n, 1));
  });
  if (t.hookSteps.length) {
    out.push("");
    out.push("hook order (reconstructed from the installed stack and the kernel's in-order rule):");
    for (const s of t.hookSteps) {
      out.push(`  ${s.engine}.${s.callback} #${s.entry} ${s.name}${s.mandatory ? " (mandatory)" : ""}: ${s.result}${s.reason ? ` reason ${s.reason}` : ""}`);
    }
  }
  const changed = t.accountDiffs.filter((d) => d.lamportsBefore !== d.lamportsAfter || d.changedRanges.length || d.created || d.closed || d.dataLenBefore !== d.dataLenAfter);
  if (changed.length) {
    out.push("");
    out.push("account changes (observed):");
    for (const d of changed) {
      const lam = d.lamportsAfter - d.lamportsBefore;
      const bytes = d.changedRanges.reduce((n, [a, b]) => n + (b - a), 0);
      const flags = [d.created && "created", d.closed && "closed"].filter(Boolean).join(", ");
      out.push(`  ${d.label ?? short(d.address)}: ${lam === 0n ? "" : `${lam > 0n ? "+" : ""}${lam} lamports `}${bytes ? `${bytes} bytes changed ` : ""}${flags}`.trimEnd());
    }
  }
  if (t.tokenBalances.some((b) => b.delta !== 0n)) {
    out.push("");
    out.push("token balances (observed):");
    for (const b of t.tokenBalances.filter((x) => x.delta !== 0n)) {
      out.push(`  ${short(b.account)} (${short(b.mint)}): ${b.before} -> ${b.after} (${b.delta > 0n ? "+" : ""}${b.delta})`);
    }
  }
  return out.join("\n");
}
