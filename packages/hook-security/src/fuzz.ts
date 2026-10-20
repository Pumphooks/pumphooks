/** Deterministic PRNG (mulberry32); every run is reproducible from its seed. */
export class Rng {
  private s: number;
  constructor(seed: number) {
    this.s = seed >>> 0 || 0x9e3779b9;
  }
  next(): number {
    let t = (this.s = (this.s + 0x6d2b79f5) >>> 0);
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  }
  int(n: number): number {
    return Math.floor(this.next() * n);
  }
  pick<T>(xs: readonly T[]): T {
    return xs[this.int(xs.length)];
  }
  /** Uniform in [0, max], biased toward the edges (0, 1, max) a quarter of the time. */
  amount(max: bigint): bigint {
    if (max <= 0n) return 0n;
    const r = this.int(8);
    if (r === 0) return 0n;
    if (r === 1) return 1n;
    if (r === 2) return max;
    return BigInt(Math.floor(this.next() * Number(max)));
  }
}

export interface StepResult {
  ok: boolean;
  error?: string;
  /** Whatever the target wants invariants to see (e.g. the execution trace). */
  detail?: unknown;
}

export interface CheckContext<Op, Snap> {
  op: Op;
  result: StepResult;
  before: Snap;
  after: Snap;
}

export interface Invariant<S, Op, Snap> {
  name: string;
  /** Returns a violation message, or null. */
  check(state: S, ctx: CheckContext<Op, Snap>): Promise<string | null> | string | null;
}

export interface Target<S, Op, Snap> {
  name: string;
  setup(): Promise<S>;
  generate(rng: Rng, state: S): Op;
  apply(state: S, op: Op): Promise<StepResult>;
  snapshot(state: S): Promise<Snap> | Snap;
  invariants: Invariant<S, Op, Snap>[];
  describe(op: Op): string;
}

export interface Violation<Op> {
  invariant: string;
  message: string;
  step: number;
  ops: Op[];
}

export interface FuzzReport<Op> {
  target: string;
  seed: number;
  runs: number;
  steps: number;
  executed: number;
  /** Count of steps per operation kind (`describe` up to the first space). */
  coverage: Record<string, number>;
  outcomes: { ok: number; failed: number };
  failure?: Violation<Op> & { shrunk: Op[]; shrunkMessage: string; run: number; runSeed: number };
}

async function checkAll<S, Op, Snap>(t: Target<S, Op, Snap>, s: S, ctx: CheckContext<Op, Snap>): Promise<{ invariant: string; message: string } | null> {
  for (const inv of t.invariants) {
    const m = await inv.check(s, ctx);
    if (m) return { invariant: inv.name, message: m };
  }
  return null;
}

/** Runs `ops` on a fresh setup; returns the first violation. */
export async function replayOps<S, Op, Snap>(t: Target<S, Op, Snap>, ops: Op[]): Promise<Violation<Op> | null> {
  const s = await t.setup();
  for (let i = 0; i < ops.length; i++) {
    const before = await t.snapshot(s);
    const result = await t.apply(s, ops[i]);
    const after = await t.snapshot(s);
    const v = await checkAll(t, s, { op: ops[i], result, before, after });
    if (v) return { ...v, step: i, ops: ops.slice(0, i + 1) };
  }
  return null;
}

/** Removes chunks, then single ops, while the same invariant still fails. */
export async function shrink<S, Op, Snap>(t: Target<S, Op, Snap>, ops: Op[], invariant: string): Promise<{ ops: Op[]; message: string }> {
  let cur = ops;
  let message = "";
  for (let chunk = Math.max(1, Math.floor(cur.length / 2)); chunk >= 1; chunk = Math.floor(chunk / 2)) {
    let i = 0;
    while (i < cur.length) {
      const cand = [...cur.slice(0, i), ...cur.slice(i + chunk)];
      const v = cand.length ? await replayOps(t, cand) : null;
      if (v && v.invariant === invariant) {
        cur = v.ops;
        message = v.message;
      } else {
        i += chunk;
      }
    }
    if (chunk === 1) break;
  }
  if (!message) message = (await replayOps(t, cur))?.message ?? "";
  return { ops: cur, message };
}

export async function fuzz<S, Op, Snap>(t: Target<S, Op, Snap>, o: { seed: number; runs: number; steps: number; shrink?: boolean }): Promise<FuzzReport<Op>> {
  const report: FuzzReport<Op> = { target: t.name, seed: o.seed, runs: o.runs, steps: o.steps, executed: 0, coverage: {}, outcomes: { ok: 0, failed: 0 } };
  for (let run = 0; run < o.runs; run++) {
    const runSeed = (o.seed + run * 0x9e3779b1) >>> 0;
    const rng = new Rng(runSeed);
    const s = await t.setup();
    const ops: Op[] = [];
    for (let step = 0; step < o.steps; step++) {
      const op = t.generate(rng, s);
      ops.push(op);
      const kind = t.describe(op).split(" ")[0];
      report.coverage[kind] = (report.coverage[kind] ?? 0) + 1;
      const before = await t.snapshot(s);
      const result = await t.apply(s, op);
      report.executed++;
      report.outcomes[result.ok ? "ok" : "failed"]++;
      const after = await t.snapshot(s);
      const v = await checkAll(t, s, { op, result, before, after });
      if (v) {
        const shrunk = o.shrink === false ? { ops, message: v.message } : await shrink(t, ops, v.invariant);
        report.failure = { ...v, step, ops, shrunk: shrunk.ops, shrunkMessage: shrunk.message, run, runSeed };
        return report;
      }
    }
  }
  return report;
}
