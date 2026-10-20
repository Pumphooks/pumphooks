import { PUMP_HOOKS_PROGRAM_ID } from "@pump-hooks/sdk";
import type { Connection, Finality, PublicKey } from "@solana/web3.js";
import { toRecord, type TxRecord } from "./record.js";
import { Stats } from "./stats.js";
import type { Store } from "./store.js";

export type IndexerRpc = Pick<Connection, "getSignaturesForAddress" | "getTransaction">;

export interface IndexerOptions {
  programId?: PublicKey;
  /** Default "finalized", so indexed history never includes transactions from dropped forks. */
  commitment?: Finality;
  pageSize?: number;
  /** On an empty store, index at most this many of the newest transactions. Default: all history. */
  backfill?: number;
  onRecord?: (r: TxRecord) => void;
}

/**
 * Walks the program's signatures newest→oldest down to the cursor, then processes them
 * oldest→newest, committing each as it goes. Re-running is always safe: committed signatures
 * are skipped, and a transaction the RPC cannot return yet stops the pass without advancing.
 */
export class Indexer {
  readonly stats: Stats;
  private readonly programId: PublicKey;
  private readonly commitment: Finality;
  private readonly pageSize: number;

  constructor(
    private readonly rpc: IndexerRpc,
    readonly store: Store,
    private readonly opts: IndexerOptions = {},
  ) {
    this.programId = opts.programId ?? PUMP_HOOKS_PROGRAM_ID;
    this.commitment = opts.commitment ?? "finalized";
    this.pageSize = opts.pageSize ?? 1000;
    this.stats = Stats.from(store.records());
  }

  async poll(): Promise<{ fetched: number; committed: number; blocked?: string }> {
    const until = this.store.cursor();
    const pending: string[] = [];
    let before: string | undefined;
    for (;;) {
      const page = await this.rpc.getSignaturesForAddress(this.programId, { before, until, limit: this.pageSize }, this.commitment);
      pending.push(...page.map((p) => p.signature));
      if (page.length < this.pageSize) break;
      if (!until && this.opts.backfill !== undefined && pending.length >= this.opts.backfill) break;
      before = page[page.length - 1].signature;
    }
    const todo = (!until && this.opts.backfill !== undefined ? pending.slice(0, this.opts.backfill) : pending).reverse();

    let committed = 0;
    for (const sig of todo) {
      if (this.store.has(sig)) continue;
      const tx = await this.rpc.getTransaction(sig, { maxSupportedTransactionVersion: 0, commitment: this.commitment });
      if (!tx) return { fetched: todo.length, committed, blocked: sig };
      const rec = toRecord(sig, tx, this.programId);
      this.store.commit(rec);
      this.stats.apply(rec);
      this.opts.onRecord?.(rec);
      committed += 1;
    }
    return { fetched: todo.length, committed };
  }
}
