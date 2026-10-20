import { appendFileSync, existsSync, mkdirSync, readFileSync, truncateSync } from "node:fs";
import { join } from "node:path";
import type { TxRecord } from "./record.js";

/**
 * Append-only record log. The cursor is the last committed record, so committing a record and
 * advancing the cursor is a single append: a crash can lose at most the record being written,
 * never skip one.
 */
export interface Store {
  cursor(): string | undefined;
  has(signature: string): boolean;
  get(signature: string): TxRecord | undefined;
  commit(record: TxRecord): void;
  records(): Iterable<TxRecord>;
}

export class MemoryStore implements Store {
  protected list: TxRecord[] = [];
  protected bySig = new Map<string, TxRecord>();

  cursor() {
    return this.list.at(-1)?.signature;
  }
  has(sig: string) {
    return this.bySig.has(sig);
  }
  get(sig: string) {
    return this.bySig.get(sig);
  }
  commit(rec: TxRecord) {
    if (this.bySig.has(rec.signature)) return;
    this.list.push(rec);
    this.bySig.set(rec.signature, rec);
  }
  records() {
    return this.list;
  }
}

export class FileStore extends MemoryStore {
  readonly path: string;

  constructor(dir: string) {
    super();
    mkdirSync(dir, { recursive: true });
    this.path = join(dir, "records.jsonl");
    if (!existsSync(this.path)) return;
    const text = readFileSync(this.path, "utf8");
    const lines = text.split("\n");
    let offset = 0;
    // Only newline-terminated lines are complete; anything after the last one is a torn write.
    for (const line of lines.slice(0, -1)) {
      let rec: TxRecord;
      try {
        rec = JSON.parse(line);
      } catch {
        break;
      }
      super.commit(rec);
      offset += Buffer.byteLength(line) + 1;
    }
    if (offset < Buffer.byteLength(text)) truncateSync(this.path, offset);
  }

  commit(rec: TxRecord) {
    if (this.has(rec.signature)) return;
    appendFileSync(this.path, JSON.stringify(rec) + "\n");
    super.commit(rec);
  }
}
