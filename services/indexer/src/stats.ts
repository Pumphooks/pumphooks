import type { TxRecord } from "./record.js";

export interface MarketStats {
  market: string;
  mint?: string;
  authority?: string;
  routedBuys: number;
  routedSells: number;
  routedBuyQuote: bigint;
  routedSellQuote: bigint;
  denied: number;
  failed: number;
  lastSlot: number;
  modules: Record<number, number>;
}

export interface ModuleStats {
  moduleId: number;
  /** Callback runs in successful transactions. */
  executions: number;
  /** Routed trades this module denied. */
  denials: number;
  /** Net installs (installed minus removed). */
  installs: number;
}

/** Aggregates derived purely from records; replaying the same records always yields the same stats. */
export class Stats {
  markets = new Map<string, MarketStats>();
  modules = new Map<number, ModuleStats>();
  transactions = 0;
  lastSlot = 0;

  static from(records: Iterable<TxRecord>) {
    const s = new Stats();
    for (const r of records) s.apply(r);
    return s;
  }

  private market(k: string): MarketStats {
    let m = this.markets.get(k);
    if (!m) {
      m = { market: k, routedBuys: 0, routedSells: 0, routedBuyQuote: 0n, routedSellQuote: 0n, denied: 0, failed: 0, lastSlot: 0, modules: {} };
      this.markets.set(k, m);
    }
    return m;
  }

  private module(id: number): ModuleStats {
    let m = this.modules.get(id);
    if (!m) {
      m = { moduleId: id, executions: 0, denials: 0, installs: 0 };
      this.modules.set(id, m);
    }
    return m;
  }

  apply(r: TxRecord) {
    this.transactions += 1;
    this.lastSlot = Math.max(this.lastSlot, r.slot);
    for (const c of r.marketsCreated) Object.assign(this.market(c.market), { mint: c.mint, authority: c.authority });
    for (const i of r.installs) {
      this.module(i.moduleId).installs += i.delta;
      const m = this.market(i.market);
      m.modules[i.moduleId] = (m.modules[i.moduleId] ?? 0) + i.delta;
      if (m.modules[i.moduleId] <= 0) delete m.modules[i.moduleId];
    }
    if (r.ok) for (const h of r.hooks) this.module(h.moduleId).executions += 1;
    if (r.swap) {
      const m = this.market(r.swap.market);
      m.mint ??= r.swap.mint;
      if (r.swap.side === "buy") {
        m.routedBuys += 1;
        m.routedBuyQuote += BigInt(r.swap.quote);
      } else {
        m.routedSells += 1;
        m.routedSellQuote += BigInt(r.swap.quote);
      }
      m.lastSlot = Math.max(m.lastSlot, r.slot);
    }
    if (r.denied) {
      this.module(r.denied.moduleId).denials += 1;
      const m = this.market(r.denied.market);
      m.denied += 1;
      m.lastSlot = Math.max(m.lastSlot, r.slot);
    } else if (!r.ok) {
      const k = r.hooks[0]?.market;
      if (k) this.market(k).failed += 1;
    }
  }
}
