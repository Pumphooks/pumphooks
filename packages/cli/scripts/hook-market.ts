// Hooks a Pump token with Max Wallet + Sell Limit + Holder XP, owned by --keypair. Simulates the
// whole stack with a test buy first; sends nothing on --dry-run or if the simulation fails.
//   pnpm --filter @pump-hooks/cli exec tsx scripts/hook-market.ts --url <rpc> --keypair <authority.json> --mint <CA> [--dry-run]
import { readFileSync } from "node:fs";
import { parseArgs } from "node:util";
import {
  ComputeBudgetProgram,
  Connection,
  Keypair,
  PublicKey,
  TransactionMessage,
  VersionedTransaction,
  type AddressLookupTableAccount,
  type TransactionInstruction,
} from "@solana/web3.js";
import { ModuleId, PumpHooksClient, PUMP_ALT, validateConfig, type ModuleConfig } from "@pump-hooks/sdk";

const { values } = parseArgs({ options: { url: { type: "string" }, keypair: { type: "string" }, mint: { type: "string" }, "dry-run": { type: "boolean", default: false } } });
if (!values.url || !values.keypair || !values.mint) throw new Error("usage: hook-market.ts --url <rpc> --keypair <authority.json> --mint <CA> [--dry-run]");

const conn = new Connection(values.url, "confirmed");
const authority = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(values.keypair, "utf8"))));
const mint = new PublicKey(values.mint);
const client = new PumpHooksClient(conn, { lookupTables: [PUMP_ALT.mainnet], microLamportsPerCu: 200_000 });

const token = await client.detect(mint);
const unit = 10n ** BigInt(token.decimals ?? 6);
const tokens = (n: bigint) => n * unit;
const SOL = 1_000_000_000n;

const hooks: { module: ModuleId; config: ModuleConfig }[] = [
  { module: ModuleId.MaxWallet, config: { maxBase: tokens(20_000_000n), startTs: 0, durationSecs: 0 } },
  { module: ModuleId.SellLimit, config: { maxSellBps: 2_500, cooldownSecs: 60, exemptBelow: tokens(1_000_000n), startTs: 0, durationSecs: 7 * 86_400 } },
  {
    module: ModuleId.HolderXp,
    config: { perTradeCapQuote: 5n * SOL, xpPerSol: 1_000, vestSecs: 3_600, dailyCap: 10_000, holdXpPerDay: 100, holdCapBase: tokens(1_000_000n), minTradeQuote: SOL / 100n },
  },
];
for (const h of hooks) {
  const problems = validateConfig(h.module, h.config);
  if (problems.length) throw new Error(`${ModuleId[h.module]} config: ${problems.join("; ")}`);
}

console.log(`mint ${mint.toBase58()} · venue ${token.venue} · routable ${token.routable} · authority ${authority.publicKey.toBase58()}`);
if (!token.routable) throw new Error(token.notRoutableReason);

const existing = (await client.findMarkets(mint)).filter((m) => m.config.authority.equals(authority.publicKey));
if (existing.length) throw new Error(`already hooked by this authority: ${existing[0].address.toBase58()}`);

const sim = await client.simulate(await client.previewStack({ mint, authority: authority.publicKey, hooks, quoteIn: SOL / 100n }), authority.publicKey);
if (!sim.ok) throw new Error(`simulation failed: ${sim.error ? `${sim.error.name} ${sim.error.msg}` : JSON.stringify(sim.rawError)}\n${sim.logs.slice(-15).join("\n")}`);
console.log(`simulation ok · test buy 0.01 SOL → ${sim.swap?.baseAmount} base units · ${sim.unitsConsumed} CU`);
if (values["dry-run"]) process.exit(0);

const { market, ix } = await client.createMarket(mint, authority.publicKey);
const ixs = [ix, ...hooks.map((h) => client.installHook({ ...h, market, authority: authority.publicKey }))];
const alts = await client.loadLookupTables();

const priority = ComputeBudgetProgram.setComputeUnitPrice({ microLamports: 200_000 });
function build(batch: TransactionInstruction[], blockhash: string, alts: AddressLookupTableAccount[]) {
  const msg = new TransactionMessage({ payerKey: authority.publicKey, recentBlockhash: blockhash, instructions: [priority, ...batch] }).compileToV0Message(alts);
  const tx = new VersionedTransaction(msg);
  tx.sign([authority]);
  return tx;
}
const fits = (batch: TransactionInstruction[]) => {
  try {
    return build(batch, PublicKey.default.toBase58(), alts).serialize().length <= 1232;
  } catch {
    return false;
  }
};
const batches: TransactionInstruction[][] = [];
for (const i of ixs) {
  const cur = batches.at(-1);
  if (cur && fits([...cur, i])) cur.push(i);
  else batches.push([i]);
}

for (const [n, batch] of batches.entries()) {
  const { blockhash, lastValidBlockHeight } = await conn.getLatestBlockhash("confirmed");
  const tx = build(batch, blockhash, alts);
  const signature = await conn.sendRawTransaction(tx.serialize(), { maxRetries: 5 });
  for (;;) {
    const [st] = (await conn.getSignatureStatuses([signature])).value;
    if (st?.err) throw new Error(`tx ${n + 1}/${batches.length} failed: ${signature}`);
    if (st?.confirmationStatus === "confirmed" || st?.confirmationStatus === "finalized") break;
    if ((await conn.getBlockHeight("confirmed")) > lastValidBlockHeight) throw new Error(`tx ${n + 1}/${batches.length} expired: ${signature}`);
    await new Promise((r) => setTimeout(r, 1500));
  }
  console.log(`tx ${n + 1}/${batches.length}: ${signature}`);
}
console.log(`market ${market.toBase58()}`);
