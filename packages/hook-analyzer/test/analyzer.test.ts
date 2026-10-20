import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { v1 } from "@pump-hooks/sdk";
import { analyze, measuredFromBench, validateStack } from "../src/index.js";

const vectors = JSON.parse(readFileSync(new URL("../../sdk/test/vectors/kernel.json", import.meta.url), "utf8")) as {
  stacks: { name: string; members: number; result: string; hex: string }[];
};
const bench = JSON.parse(readFileSync(new URL("../../../bench/baseline.json", import.meta.url), "utf8"));
const T = v1.Environment.Token;
const k = (b: number) => new Uint8Array(32).fill(b);
const codes = (a: ReturnType<typeof analyze>) => a.diagnostics.map((d) => d.code);
const NOW = 1_800_000_000n;

describe("validateStack agrees with the Rust kernel", () => {
  it.each(vectors.stacks.map((s) => [s.name, s] as const))("%s", (_n, s) => {
    expect(validateStack(new Uint8Array(Buffer.from(s.hex, "hex")), s.members)).toBe(s.result);
  });
  it("covers every StackError variant", () => {
    const kinds = new Set(vectors.stacks.map((s) => s.result.replace(/\(.*/, "")));
    for (const k of ["ok", "Version", "Environment", "TooLong", "UnknownImpl", "Callbacks", "Capabilities", "Dependency", "Config", "Program", "TooManyProgramHooks", "Duplicate", "DirtySlot"]) expect(kinds).toContain(k);
  });
});

describe("composition diagnostics", () => {
  it("a sound token stack has no errors and uses measured compute when available", () => {
    const s = v1.encodeStack(T, [
      { builtin: 6, mandatory: true, config: v1.configs.maxSize(k(4), 1_000n) },
      { builtin: 1, callbacks: [0], config: v1.configs.allowlistInline(v1.Target.Counterparty, [k(1)]) },
    ]);
    const a = analyze(s, { now: NOW, policyDelaySecs: 86_400n, measuredCu: measuredFromBench(bench) });
    expect(a.valid).toBe(true);
    expect(a.diagnostics.filter((d) => d.severity === "error")).toEqual([]);
    expect(a.compute.estimate).toBe(2_800);
    expect(a.compute.measured).toBeGreaterThan(0);
    expect(a.compute.measured).toBeLessThan(a.compute.estimate);
    expect(a.enforcement.find((e) => e.callback === "before_transfer")!.class).toBe("native");
    expect(codes(a)).toContain("wrapper-only");
  });

  it("flags conflicts and dead policies", () => {
    const s = v1.encodeStack(T, [
      { builtin: 1, callbacks: [0], config: v1.configs.allowlistInline(v1.Target.Counterparty, [k(1)]) },
      { builtin: 9, callbacks: [0], config: v1.configs.denylist(v1.Target.Counterparty, [k(1)]) },
      { builtin: 7, callbacks: [0], config: v1.configs.timeWindow({ notBefore: 1n, notAfter: NOW - 10n }) },
      { builtin: 2, config: v1.configs.spendingLimit(k(4), 100n, 60) },
      { builtin: 6, callbacks: [0], config: v1.configs.maxSize(k(4), 1_000n) },
    ]);
    const a = analyze(s, { now: NOW, policyDelaySecs: 0n });
    expect(a.kernel).toBe("ok");
    expect(a.valid).toBe(false);
    expect(codes(a)).toEqual(expect.arrayContaining(["allow-deny-conflict", "window-closed", "limit-below-cap", "nothing-mandatory"]));
  });

  it("applies the token engine's install rules", () => {
    const record = v1.encodeEntry(T, { builtin: 1, callbacks: [0], config: v1.configs.allowlistInline(0, [k(1)]) });
    record[88 + 1] = 1;
    record.set(k(2), 88 + 48);
    const creds = [5, 6, 7].map((n) => v1.encodeEntry(T, { builtin: 13, callbacks: [0], hookVersion: k(60 + n), config: v1.configs.credential(1, k(n)) }));
    const a = analyze(v1.encodeStack(T, [record, ...creds]), { now: NOW });
    expect(a.kernel).toBe("ok");
    expect(codes(a)).toEqual(expect.arrayContaining(["token-record-allowlist", "token-credentials"]));
  });

  it("router stacks are labelled bypassable; unpinned program hooks and zero delay are flagged", () => {
    const s = v1.encodeStack(v1.Environment.Router, [{ builtin: 14, config: v1.configs.slotLimit(1), mandatory: true }]);
    const prog = new Uint8Array(216);
    prog.set(k(70), 0);
    prog.set(k(9), 32);
    prog[64] = 1;
    new DataView(prog.buffer).setUint16(68, 1, true);
    new DataView(prog.buffer).setUint32(72, v1.Cap.VALIDATE_OPERATION | v1.Cap.VETO_OPERATION, true);
    const full = v1.encodeStack(v1.Environment.Router, [s.slice(8), prog]);
    const a = analyze(full, { now: NOW, policyDelaySecs: 0n });
    expect(a.kernel).toBe("ok");
    expect(codes(a)).toEqual(expect.arrayContaining(["bypassable", "unpinned-program", "no-delay"]));
    expect(analyze(full, { now: NOW, computeBudget: 1_000 }).diagnostics.some((d) => d.code === "compute")).toBe(true);
  });

  it("reports kernel refusals", () => {
    const bad = v1.encodeStack(T, [{ builtin: 6, config: v1.configs.maxSize(k(4), 1n) }]);
    bad[8 + 72] ^= 0xff;
    const a = analyze(bad, { now: NOW });
    expect(a.valid).toBe(false);
    expect(a.kernel).toBe("Capabilities(0)");
  });
});
