// Runs the demos against target/deploy and prints each step: pnpm --filter @pump-hooks/demos demo [token|router|pool|account]
import { join, resolve } from "node:path";
import { DEMOS, type Engine } from "../src/index.js";

const ROOT = resolve(import.meta.dirname, "../../..");
const which = (process.argv[2] ? [process.argv[2]] : Object.keys(DEMOS)) as Engine[];
let failed = false;
for (const id of which) {
  const d = await DEMOS[id]({ deployDir: join(ROOT, "target", "deploy"), fixturesDir: join(ROOT, "tests", "fixtures") });
  console.log(`\n${d.title}\n  ${d.enforcement}`);
  for (const s of d.steps) {
    failed ||= !s.ok;
    console.log(`  ${s.ok ? "✓" : "✗"} [${s.trace.status === "success" ? "ok" : "rejected"}, ${s.trace.computeUnits.total} CU] ${s.title}\n      ${s.outcome}`);
  }
}
process.exit(failed ? 1 : 0);
