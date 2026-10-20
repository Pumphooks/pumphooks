import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { compare, hasFailures, marginals, parseResults, renderDiff, type Results } from "../src/index.js";

const committed = () => parseResults(readFileSync(new URL("../../../bench/baseline.json", import.meta.url), "utf8"));
const clone = (r: Results): Results => JSON.parse(JSON.stringify(r));

describe("hook-bench", () => {
  it("parses the committed baseline and every case succeeded", () => {
    const r = committed();
    expect(r.programs[0][0]).toBe("token_hooks");
    expect(r.programs[0][1]).toMatch(/^[0-9a-f]{64}$/);
    expect(r.unit.length).toBeGreaterThan(1);
    expect([...r.unit, ...r.transaction].every((m) => m.ok)).toBe(true);
  });

  it("computes marginal cost against the empty stack", () => {
    const m = marginals(committed());
    expect(m.find((x) => x.name === "allowlist")!.marginal).toBeGreaterThan(0);
    expect(m.every((x) => x.marginal > 0)).toBe(true);
  });

  it("flags regressions beyond tolerance, missing and failed cases", () => {
    const base = committed();
    expect(hasFailures(compare(base, base))).toBe(false);

    const slower = clone(base);
    slower.unit[0].compute_units = Math.ceil(base.unit[0].compute_units * 1.04);
    expect(compare(base, slower)[0].status).toBe("unchanged");
    slower.unit[0].compute_units = Math.ceil(base.unit[0].compute_units * 1.06);
    const d = compare(base, slower);
    expect(d[0].status).toBe("regressed");
    expect(hasFailures(d)).toBe(true);
    expect(renderDiff(d)).toContain("regressed");

    const fewer = clone(base);
    fewer.transaction.pop();
    expect(compare(base, fewer).some((x) => x.status === "missing")).toBe(true);

    const broken = clone(base);
    broken.unit[1].ok = false;
    expect(compare(base, broken)[1].status).toBe("failed");
  });

  it("rejects malformed input", () => {
    expect(() => parseResults("{}")).toThrow();
    expect(() => parseResults(JSON.stringify({ runtime: "", programs: [], unit: [{ name: "x", compute_units: -1, ok: true }], transaction: [] }))).toThrow();
  });
});
