import type { Category } from "./catalog.js";
import type { FuzzReport } from "./fuzz.js";

export const DISCLAIMER =
  "This report lists automated tests and their results. It is not an audit, and passing it does not mean the programs are free of vulnerabilities.";

export function renderReport(o: { catalog: Category[]; fuzz: FuzzReport<unknown>[]; describe?: (op: unknown) => string; programs?: Record<string, string> }): string {
  const d = o.describe ?? ((op: unknown) => JSON.stringify(op, (_k, v) => (typeof v === "bigint" ? v.toString() : v)));
  const out: string[] = ["# Pump Hooks security check", "", `> ${DISCLAIMER}`, ""];
  if (o.programs) {
    out.push("Binaries:", "");
    for (const [n, h] of Object.entries(o.programs)) out.push(`- \`${n}\` sha256 \`${h}\``);
    out.push("");
  }
  out.push("## Stateful fuzzing", "");
  for (const f of o.fuzz) {
    out.push(`### ${f.target}`, "");
    out.push(`seed ${f.seed}, ${f.runs} runs x ${f.steps} steps, ${f.executed} transactions (${f.outcomes.ok} succeeded, ${f.outcomes.failed} failed as expected or not)`);
    out.push("", `operations: ${Object.entries(f.coverage).map(([k, n]) => `${k} ${n}`).join(", ")}`, "");
    if (f.failure) {
      out.push(`**FAILED** invariant \`${f.failure.invariant}\` at run ${f.failure.run} (run seed ${f.failure.runSeed}), step ${f.failure.step}: ${f.failure.message}`, "");
      out.push(`Shrunk to ${f.failure.shrunk.length} operation(s): ${f.failure.shrunkMessage}`, "", "```");
      f.failure.shrunk.forEach((op, i) => out.push(`${i}. ${d(op)}`));
      out.push("```", "");
    } else {
      out.push("No invariant violated.", "");
    }
  }
  out.push("## Attack categories", "", "| Category | Executed tests | Known gaps |", "| --- | --- | --- |");
  for (const c of o.catalog) {
    out.push(`| ${c.title} | ${c.evidence.map((e) => `\`${e.test}\``).join(", ")} | ${c.gaps ?? "—"} |`);
  }
  out.push("");
  return out.join("\n");
}
