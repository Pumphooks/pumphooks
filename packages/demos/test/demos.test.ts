import { existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { accountDemo, impliedFeeBps, poolDemo, routerDemo, routerFixturesAvailable, tokenDemo, type DemoResult } from "../src/index.js";

const ROOT = resolve(__dirname, "../../..");
const deployDir = join(ROOT, "target", "deploy");
const fixturesDir = join(ROOT, "tests", "fixtures");
const built = (...so: string[]) => so.every((f) => existsSync(join(deployDir, f)));
const SBF = process.env.PUMPHOOKS_SBF === "1";

function outcomes(d: DemoResult) {
  for (const s of d.steps) expect(s.ok, `${s.title}: expected ${s.expect}, got ${s.trace.status}\n${s.outcome}\n${s.trace.logs.join("\n")}`).toBe(true);
  return d.steps.map((s) => s.outcome);
}

describe("fee recovery", () => {
  it("inverts the constant-product output", () => {
    // 1_000_000 in, 3_000 ppm fee, 1e9 / 1e9 reserves.
    const net = 1_000_000n - 3_000n;
    const out = (net * 1_000_000_000n) / (1_000_000_000n + net);
    expect(impliedFeeBps(1_000_000n, out, 1_000_000_000n, 1_000_000_000n)).toBeCloseTo(30, 0);
  });
});

describe.skipIf(!SBF && !built("token_hooks.so", "token_wrapper.so", "hook_registry.so"))("demo A: token allowlist", () => {
  it("allows the listed recipient, refuses others and hook-less transfers", async () => {
    const o = outcomes(await tokenDemo({ deployDir }));
    expect(o[1]).toContain("bob holds 400");
    expect(o[2]).toMatch(/entry 0/);
    expect(o[2]).toContain("carol still holds 0");
  });
});

describe.skipIf(!(SBF || built("hook_router.so", "hook_registry.so")) || !routerFixturesAvailable(fixturesDir))("demo B: routed max size", () => {
  it("caps routed buys and shows the venue stays directly callable", async () => {
    const d = await routerDemo({ deployDir, fixturesDir });
    const o = outcomes(d);
    expect(o[0]).toMatch(/spent 0\.5000 SOL, received \d+ base .*venue verified: true/);
    expect(o[1]).toMatch(/hook stack rejected: callback 2 entry 0/);
    expect(o[1]).toContain("unchanged (true)");
    expect(o[2]).toContain("router hooks do not apply");
    expect(d.steps[1].trace.hookSteps.some((h) => h.result === "rejected")).toBe(true);
  });
});

describe.skipIf(!SBF && !built("hooks_core.so", "hook_registry.so"))("demo C: pool dynamic fee", () => {
  it("charges a size-dependent fee that never exceeds the bound", async () => {
    const d = await poolDemo({ deployDir });
    const o = outcomes(d);
    const fee = (s: string) => Number(/fee charged ([\d.]+) bps/.exec(s)![1]);
    const [small, pivot, past] = [fee(o[0]), fee(o[1]), fee(o[2])];
    expect(small).toBeGreaterThanOrEqual(30);
    expect(small).toBeLessThan(31);
    expect(pivot).toBeCloseTo(100, 0);
    expect(past).toBeCloseTo(100, 0);
  });
});

describe.skipIf(!SBF && !built("smart_accounts.so", "hook_registry.so"))("demo D: smart account", () => {
  it("needs aged, exact proposals and keeps the daily limit", async () => {
    const d = await accountDemo({ deployDir });
    const o = outcomes(d);
    const by = (t: string) => o[d.steps.findIndex((s) => s.title.startsWith(t))];
    expect(by("Owner withdraws 1 SOL immediately")).toMatch(/timelock/i);
    expect(by("Withdraw with the proposal 59")).toMatch(/not elapsed/);
    expect(by("Withdraw 2 SOL with")).toMatch(/ActionMismatch|does not match/i);
    expect(by("Withdraw 1 SOL with the proposal after")).toContain("recipient holds 1.0000 SOL");
    expect(by("Replay")).toMatch(/Stale/i);
    expect(by("Withdraw 1.5 SOL")).toMatch(/spending limit/i);
    expect(by("A stranger")).toMatch(/Unauthorized|not authorized/i);
  });
});
