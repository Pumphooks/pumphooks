/** One measured case from `bench/results.json` (written by `cargo run -p hook-bench`). */
export interface Measurement {
  name: string;
  compute_units: number;
  ok: boolean;
  /** The kernel's static estimate for the case's builtins, when there is one. */
  kernel_estimate: number | null;
}

export interface Results {
  runtime: string;
  /** `[program, sha256 of the .so]` pairs the numbers were measured against. */
  programs: [string, string][];
  unit: Measurement[];
  transaction: Measurement[];
}

export const EMPTY_CASE = "execute: empty stack";

export function parseResults(json: string): Results {
  const r = JSON.parse(json) as Results;
  if (typeof r.runtime !== "string" || !Array.isArray(r.programs) || !Array.isArray(r.unit) || !Array.isArray(r.transaction)) {
    throw new Error("not a hook-bench results file");
  }
  for (const m of [...r.unit, ...r.transaction]) {
    if (typeof m.name !== "string" || !Number.isInteger(m.compute_units) || m.compute_units < 0 || typeof m.ok !== "boolean") {
      throw new Error(`malformed measurement: ${JSON.stringify(m)}`);
    }
  }
  return r;
}

export function allCases(r: Results): Measurement[] {
  return [...r.unit, ...r.transaction];
}

export interface Marginal {
  name: string;
  /** Measured CU above the empty stack. */
  marginal: number;
  kernelEstimate: number | null;
}

/** Per-case cost above `execute: empty stack`. */
export function marginals(r: Results): Marginal[] {
  const base = r.unit.find((m) => m.name === EMPTY_CASE);
  if (!base) throw new Error(`results have no "${EMPTY_CASE}" case`);
  return r.unit
    .filter((m) => m !== base)
    .map((m) => ({ name: m.name.replace(/^execute: /, ""), marginal: m.compute_units - base.compute_units, kernelEstimate: m.kernel_estimate }));
}

export type DiffStatus = "regressed" | "improved" | "unchanged" | "added" | "missing" | "failed";

export interface Diff {
  name: string;
  baseline: number | null;
  current: number | null;
  delta: number | null;
  /** Percent change against the baseline, rounded to one decimal. */
  pct: number | null;
  status: DiffStatus;
}

/** Compares `current` with `baseline`. A case regresses when it exceeds the baseline by more than `tolerancePct`. */
export function compare(baseline: Results, current: Results, tolerancePct = 5): Diff[] {
  const now = new Map(allCases(current).map((m) => [m.name, m]));
  const out: Diff[] = [];
  for (const b of allCases(baseline)) {
    const c = now.get(b.name);
    now.delete(b.name);
    if (!c) {
      out.push({ name: b.name, baseline: b.compute_units, current: null, delta: null, pct: null, status: "missing" });
      continue;
    }
    const delta = c.compute_units - b.compute_units;
    const pct = b.compute_units === 0 ? 0 : Math.round((delta / b.compute_units) * 1000) / 10;
    const status: DiffStatus = !c.ok
      ? "failed"
      : delta * 100 > b.compute_units * tolerancePct
        ? "regressed"
        : delta < 0
          ? "improved"
          : "unchanged";
    out.push({ name: b.name, baseline: b.compute_units, current: c.compute_units, delta, pct, status });
  }
  for (const c of now.values()) {
    out.push({ name: c.name, baseline: null, current: c.compute_units, delta: null, pct: null, status: c.ok ? "added" : "failed" });
  }
  return out;
}

export function hasFailures(diffs: Diff[]): boolean {
  return diffs.some((d) => d.status === "regressed" || d.status === "missing" || d.status === "failed");
}

export function renderDiff(diffs: Diff[]): string {
  const rows = diffs.map((d) => {
    const n = (v: number | null) => (v === null ? "—" : String(v));
    const pct = d.pct === null ? "—" : `${d.pct > 0 ? "+" : ""}${d.pct}%`;
    return `| ${d.name} | ${n(d.baseline)} | ${n(d.current)} | ${pct} | ${d.status} |`;
  });
  return ["| Case | Baseline CU | Current CU | Change | Status |", "| --- | ---: | ---: | ---: | --- |", ...rows].join("\n") + "\n";
}
