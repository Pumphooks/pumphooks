import { Connection, PublicKey, type Finality } from "@solana/web3.js";
import { PUMP_HOOKS_PROGRAM_ID } from "@pump-hooks/sdk";
import { Indexer } from "./indexer.js";
import { serve } from "./server.js";
import { FileStore } from "./store.js";

// Configuration is environment-only; this service never reads Solana CLI config or keypairs.
const env = process.env;
const rpcUrl = env.RPC_URL ?? "https://api.devnet.solana.com";
const dataDir = env.DATA_DIR ?? "./data";
const port = Number(env.PORT ?? 8787);
const pollMs = Number(env.POLL_MS ?? 5000);
const commitment = (env.COMMITMENT ?? "finalized") as Finality;
const programId = env.PROGRAM_ID ? new PublicKey(env.PROGRAM_ID) : PUMP_HOOKS_PROGRAM_ID;

const store = new FileStore(dataDir);
const indexer = new Indexer(new Connection(rpcUrl, commitment), store, {
  programId,
  commitment,
  backfill: env.BACKFILL ? Number(env.BACKFILL) : undefined,
});
const server = serve(indexer, port);
const host = new URL(rpcUrl).host;
console.log(`indexer: ${programId.toBase58()} on ${host} · ${store.path} (${indexer.stats.transactions} records) · http://localhost:${port}`);

let stop = false;
let delay = pollMs;
const loop = async () => {
  while (!stop) {
    try {
      const r = await indexer.poll();
      if (r.committed) console.log(`+${r.committed} tx · cursor ${store.cursor()} · slot ${indexer.stats.lastSlot}`);
      if (r.blocked) console.log(`waiting for ${r.blocked} at ${commitment}`);
      delay = pollMs;
    } catch (e) {
      delay = Math.min(delay * 2, 60_000);
      console.error(`poll failed (${(e as Error).message}); retrying in ${delay} ms`);
    }
    await new Promise((r) => setTimeout(r, delay));
  }
};
for (const sig of ["SIGINT", "SIGTERM"] as const)
  process.on(sig, () => {
    stop = true;
    server.close();
    process.exit(0);
  });
void loop();
