import { describe, expect, it } from "vitest";
import { fuzz, Rng, shrink, type Target } from "../src/index.js";

// A counter with a planted bug: "add 7" after a "reset" pushes it past the bound.
type Op = { add: number } | { reset: true };
const buggy: Target<{ n: number; armed: boolean }, Op, number> = {
  name: "planted",
  setup: async () => ({ n: 0, armed: false }),
  generate: (rng) => (rng.int(6) === 0 ? { reset: true } : { add: rng.int(8) }),
  apply: async (s, op) => {
    if ("reset" in op) {
      s.n = 0;
      s.armed = true;
    } else {
      s.n += op.add === 7 && s.armed ? 100 : op.add % 3;
    }
    return { ok: true };
  },
  snapshot: (s) => s.n,
  invariants: [{ name: "bounded", check: (s) => (s.n > 50 ? `n=${s.n}` : null) }],
  describe: (op) => ("reset" in op ? "reset" : `add ${op.add}`),
};

describe("fuzzer", () => {
  it("is deterministic per seed", () => {
    const a = new Rng(42);
    const b = new Rng(42);
    expect(Array.from({ length: 20 }, () => a.int(1000))).toEqual(Array.from({ length: 20 }, () => b.int(1000)));
    expect(new Rng(43).int(1_000_000)).not.toBe(new Rng(42).int(1_000_000));
  });

  it("finds the planted bug and shrinks it to the two operations that cause it", async () => {
    const r = await fuzz(buggy, { seed: 7, runs: 20, steps: 60 });
    expect(r.failure?.invariant).toBe("bounded");
    expect(r.failure!.shrunk.map(buggy.describe)).toEqual(["reset", "add 7"]);
    const again = await fuzz(buggy, { seed: 7, runs: 20, steps: 60 });
    expect(again.failure!.ops).toEqual(r.failure!.ops);
  });

  it("shrink keeps the same invariant", async () => {
    const s = await shrink(buggy, [{ add: 1 }, { reset: true }, { add: 2 }, { add: 7 }, { add: 3 }], "bounded");
    expect(s.ops).toEqual([{ reset: true }, { add: 7 }]);
  });
});
