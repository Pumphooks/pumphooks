import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { v1 } from "@pump-hooks/sdk";
import {
  applyWrapperOp,
  ATTACKS,
  CATEGORIES,
  describeWrapperOp,
  fuzz,
  renderReport,
  setupWrapper,
  snapshotWrapper,
  WRAPPER_INVARIANTS,
  wrapperTarget,
  type AttackId,
} from "../src/index.js";

const DEPLOY = fileURLToPath(new URL("../../../target/deploy", import.meta.url));
const SBF = process.env.PUMPHOOKS_SBF === "1" || (existsSync(resolve(DEPLOY, "token_wrapper.so")) && existsSync(resolve(DEPLOY, "token_hooks.so")));

describe.skipIf(!SBF)("token-wrapper + token-hooks (compiled programs)", () => {
  it("every attack fails and changes nothing", async () => {
    const s = await setupWrapper(DEPLOY);
    expect((await applyWrapperOp(s, { kind: "wrap", user: 1, amount: 5_000n, minOut: 5_000n, expectOk: true })).ok).toBe(true);
    for (const id of Object.keys(ATTACKS) as AttackId[]) {
      const before = snapshotWrapper(s);
      const r = await applyWrapperOp(s, { kind: "attack", attack: id, user: 0, victim: 1, amount: 1_000n });
      expect(r.ok, `${id} must fail`).toBe(false);
      expect(snapshotWrapper(s).accounts, id).toEqual(before.accounts);
    }
  });

  it("stateful fuzzing holds every invariant", async () => {
    const r = await fuzz(wrapperTarget(DEPLOY), { seed: 20261020, runs: 3, steps: 40 });
    if (r.failure) throw new Error(renderReport({ catalog: CATEGORIES, fuzz: [r as never], describe: describeWrapperOp as never }));
    expect(r.executed).toBe(120);
    expect(r.outcomes.ok).toBeGreaterThan(20);
    expect(r.outcomes.failed).toBeGreaterThan(5);
    for (const k of ["wrap", "unwrap", "transfer"]) expect(r.coverage[k]).toBeGreaterThan(5);
  });

  it("negative control: forged unbacked supply is caught by the invariants and by verify_reserves", async () => {
    const s = await setupWrapper(DEPLOY);
    await applyWrapperOp(s, { kind: "wrap", user: 0, amount: 1_000n, minOut: 0n, expectOk: true });
    const before = snapshotWrapper(s);
    const mint = s.lab.getAccount(s.wrappedMint)!;
    new DataView(mint.data.buffer, mint.data.byteOffset).setBigUint64(36, 1_001n, true);
    s.lab.setAccount(mint);
    const result = await applyWrapperOp(s, { kind: "verify", expectOk: true });
    expect(result.ok).toBe(false);
    expect(s.lastTrace!.logs.join("\n")).toMatch(/ReserveShortfall/);
    const after = snapshotWrapper(s);
    const violated: string[] = [];
    for (const inv of WRAPPER_INVARIANTS) {
      if (await inv.check(s, { op: { kind: "verify", expectOk: true }, result, before, after })) violated.push(inv.name);
    }
    expect(violated).toEqual(expect.arrayContaining(["reserves-cover-supply", "wrapped-balances-sum-to-supply", "honest-operations-behave-as-specified"]));
  });

  it("policy decodes from the live account", async () => {
    const s = await setupWrapper(DEPLOY, 2);
    const p = v1.decodeTokenPolicy(s.lab.getAccount(s.policy)!.data);
    expect(p.mint).toBe(s.wrappedMint);
    expect(p.stack.entries).toHaveLength(0);
    expect(p.metasBump).toBe(v1.tokenHooksPda.extraAccountMetas(s.wrappedMint)[1]);
  });
});
