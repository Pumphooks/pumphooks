import { appendFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { ModuleId } from "@pump-hooks/sdk";
import { Env, fixturesAvailable, POOL_FIXTURE, SOL } from "../../../packages/sdk/test/harness.js";
import { Indexer, type IndexerRpc } from "../src/indexer.js";
import { serve } from "../src/server.js";
import { FileStore, MemoryStore } from "../src/store.js";
import type { AddressInfo } from "node:net";

interface ChainTx {
  signature: string;
  slot: number;
  logs: string[];
  err: unknown;
  cu?: number;
}

/** A minimal RPC over a list of real program executions, newest-first like the real endpoint. */
class FakeChain {
  txs: ChainTx[] = [];
  unavailable = new Set<string>();
  calls = 0;

  push(sig: string, r: { logs: string[]; rawError?: unknown; unitsConsumed?: number }) {
    this.txs.push({ signature: sig, slot: 1000 + this.txs.length, logs: r.logs, err: r.rawError ?? null, cu: r.unitsConsumed });
  }

  rpc(): IndexerRpc {
    return {
      getSignaturesForAddress: (async (_addr: unknown, o: { before?: string; until?: string; limit?: number } = {}) => {
        this.calls += 1;
        const desc = [...this.txs].reverse();
        let i = o.before ? desc.findIndex((t) => t.signature === o.before) + 1 : 0;
        const out = [];
        for (; i < desc.length && out.length < (o.limit ?? 1000); i++) {
          if (desc[i].signature === o.until) break;
          out.push({ signature: desc[i].signature, slot: desc[i].slot, err: desc[i].err, memo: null, blockTime: desc[i].slot });
        }
        return out;
      }) as unknown as IndexerRpc["getSignaturesForAddress"],
      getTransaction: (async (sig: string) => {
        const t = this.txs.find((x) => x.signature === sig);
        if (!t || this.unavailable.has(sig)) return null;
        return { slot: t.slot, blockTime: t.slot, meta: { logMessages: t.logs, err: t.err, computeUnitsConsumed: t.cu } };
      }) as unknown as IndexerRpc["getTransaction"],
    };
  }
}

const snapshot = (ix: Indexer) =>
  JSON.parse(
    JSON.stringify({ markets: [...ix.stats.markets.values()], modules: [...ix.stats.modules.values()], n: ix.stats.transactions }, (_, v) =>
      typeof v === "bigint" ? v.toString() : v,
    ),
  );

describe.skipIf(!fixturesAvailable())("indexer over real Pump Hooks executions", () => {
  const chain = new FakeChain();
  let market = "";
  let dir = "";
  let more: () => Promise<void>;

  beforeAll(async () => {
    const env = new Env(POOL_FIXTURE);
    env.bootstrap();
    const owner = env.wallet(10n * SOL);
    const user = env.wallet(5n * SOL);
    const c = env.client;
    const created = await c.createMarket(env.mint, owner.publicKey);
    market = created.market.toBase58();
    const setup = env.ixs(
      [
        created.ix,
        c.installHook({ market: created.market, authority: owner.publicKey, module: ModuleId.VolumeGuard, config: { maxBuyQuote: SOL / 2n, maxSellQuote: SOL } }),
      ],
      [owner],
    );
    expect(setup.ok).toBe(true);
    chain.push("setup", setup);
    const m = await c.getMarket(created.market);
    const buy = env.exec(await c.buy({ market: m, user: user.publicKey, quoteIn: SOL / 10n }), [user]);
    expect(buy.ok).toBe(true);
    chain.push("buy-1", buy);
    env.advance(2n);
    const denied = env.exec(await c.buy({ market: m, user: user.publicKey, quoteIn: SOL }), [user]);
    expect(denied.ok).toBe(false);
    chain.push("denied-1", denied);
    more = async () => {
      env.advance(2n);
      const r = env.exec(await c.buy({ market: await c.getMarket(created.market), user: user.publicKey, quoteIn: SOL / 5n }), [user]);
      expect(r.ok).toBe(true);
      chain.push("buy-2", r);
    };
    dir = mkdtempSync(join(tmpdir(), "ph-indexer-"));
  });

  afterAll(() => dir && rmSync(dir, { recursive: true, force: true }));

  it("decodes routed trades, denials, installs and hook executions", async () => {
    const ix = new Indexer(chain.rpc(), new MemoryStore());
    expect(await ix.poll()).toEqual({ fetched: 3, committed: 3 });
    const m = ix.stats.markets.get(market)!;
    expect(m.routedBuys).toBe(1);
    expect(m.routedBuyQuote).toBeGreaterThan(0n);
    expect(m.routedBuyQuote).toBeLessThanOrEqual(SOL / 10n);
    expect(m.denied).toBe(1);
    expect(m.modules).toEqual({ [ModuleId.VolumeGuard]: 1 });
    const vg = ix.stats.modules.get(ModuleId.VolumeGuard)!;
    expect(vg.installs).toBe(1);
    expect(vg.denials).toBe(1);
    expect(vg.executions).toBeGreaterThan(0);
    const d = ix.store.get("denied-1")!;
    expect(d.ok).toBe(false);
    expect(d.error?.name).toBe("HookDenied");
    expect(d.swap).toBeUndefined();
    expect(d.denied).toMatchObject({ market, moduleId: ModuleId.VolumeGuard, callback: "BeforeSwap" });
    expect(ix.store.cursor()).toBe("denied-1");
  });

  it("resumes across pages and RPC gaps without skipping or double counting", async () => {
    const full = new Indexer(chain.rpc(), new MemoryStore());
    await full.poll();

    chain.unavailable.add("buy-1");
    const ix = new Indexer(chain.rpc(), new MemoryStore(), { pageSize: 1 });
    expect(await ix.poll()).toEqual({ fetched: 3, committed: 1, blocked: "buy-1" });
    expect(ix.store.cursor()).toBe("setup");
    chain.unavailable.clear();
    expect(await ix.poll()).toEqual({ fetched: 2, committed: 2 });
    expect(await ix.poll()).toEqual({ fetched: 0, committed: 0 });
    expect(snapshot(ix)).toEqual(snapshot(full));
  });

  it("persists to disk, survives a torn write and replays to identical stats", async () => {
    const a = new Indexer(chain.rpc(), new FileStore(dir));
    await a.poll();
    appendFileSync(join(dir, "records.jsonl"), '{"signature":"half-writ');

    const b = new Indexer(chain.rpc(), new FileStore(dir));
    expect(b.store.cursor()).toBe("denied-1");
    expect(snapshot(b)).toEqual(snapshot(a));
    expect(await b.poll()).toEqual({ fetched: 0, committed: 0 });

    await more();
    expect(await b.poll()).toEqual({ fetched: 1, committed: 1 });
    expect(b.stats.markets.get(market)!.routedBuys).toBe(2);
    const c = new Indexer(chain.rpc(), new FileStore(dir));
    expect(snapshot(c)).toEqual(snapshot(b));
  });

  it("serves stats over a read-only HTTP API", async () => {
    const ix = new Indexer(chain.rpc(), new MemoryStore());
    await ix.poll();
    const server = serve(ix, 0);
    await new Promise((r) => server.once("listening", r));
    const base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
    try {
      const health = await (await fetch(`${base}/health`)).json();
      expect(health).toMatchObject({ ok: true, cursor: chain.txs.at(-1)!.signature, transactions: chain.txs.length });
      const mods = (await (await fetch(`${base}/modules`)).json()) as { moduleId: number }[];
      expect(mods.find((m) => m.moduleId === ModuleId.VolumeGuard)).toMatchObject({ installs: 1, denials: 1 });
      const m = (await (await fetch(`${base}/markets/${market}`)).json()) as { denied: number; recent: { signature: string }[] };
      expect(m.denied).toBe(1);
      expect(m.recent[0].signature).toBe(chain.txs.at(-1)!.signature);
      expect((await fetch(`${base}/tx/nope`)).status).toBe(404);
      expect((await fetch(`${base}/health`, { method: "POST" })).status).toBe(405);
    } finally {
      server.close();
    }
  });

  it("limits first-run backfill to the newest transactions", async () => {
    const ix = new Indexer(chain.rpc(), new MemoryStore(), { backfill: 2, pageSize: 1 });
    const r = await ix.poll();
    expect(r.committed).toBe(2);
    expect([...ix.store.records()].map((x) => x.signature)).toEqual(chain.txs.slice(-2).map((t) => t.signature));
  });
});
