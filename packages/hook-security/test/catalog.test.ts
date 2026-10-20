import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { ATTACKS, CATEGORIES, WRAPPER_INVARIANTS } from "../src/index.js";

const ROOT = fileURLToPath(new URL("../../..", import.meta.url));

function rustSources(dir: string): string[] {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    if (f === "target" || f === "node_modules") return [];
    return statSync(p).isDirectory() ? rustSources(p) : f.endsWith(".rs") ? [readFileSync(p, "utf8")] : [];
  });
}

describe("attack catalog", () => {
  const sources = [...rustSources(join(ROOT, "tests")), ...rustSources(join(ROOT, "bench"))].join("\n");

  it("has 15 distinct categories, each with executed evidence", () => {
    expect(CATEGORIES).toHaveLength(15);
    expect(new Set(CATEGORIES.map((c) => c.id)).size).toBe(15);
    for (const c of CATEGORIES) expect(c.evidence.length, c.id).toBeGreaterThan(0);
  });

  it("every cited test exists", () => {
    for (const c of CATEGORIES) {
      for (const e of c.evidence) {
        if (e.kind === "rust") expect(new RegExp(`fn ${e.test}\\(`).test(sources), `${c.id}: rust test ${e.test}`).toBe(true);
        if (e.kind === "fuzz-attack") expect(Object.keys(ATTACKS), `${c.id}: attack ${e.test}`).toContain(e.test);
        if (e.kind === "fuzz-invariant") expect(WRAPPER_INVARIANTS.map((i) => i.name), `${c.id}: invariant ${e.test}`).toContain(e.test);
      }
    }
  });

  it("every attack and invariant is cited", () => {
    const cited = new Set(CATEGORIES.flatMap((c) => c.evidence.map((e) => e.test)));
    for (const a of Object.keys(ATTACKS)) expect(cited.has(a), a).toBe(true);
  });
});
