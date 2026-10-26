// Shows a hooked market's rules running on chain with three routed trades from --keypair:
//   1. buy --sol (every hook runs; expected ALLOW)
//   2. sell half the tokens (expected ALLOW; starts Sell Limit's cooldown)
//   3. sell the rest right away (expected DENY by Sell Limit's cooldown; landed as a failed tx)
// Each step is simulated first and printed; nothing is sent without --send.
//   pnpm --filter @pump-hooks/cli exec tsx scripts/proof-trades.ts --url <rpc> --keypair <wallet.json> --market <address> --sol 0.01 [--send]
import { readFileSync } from "node:fs";
import { parseArgs } from "node:util";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import { PumpHooksClient, PUMP_ALT, type BuiltTransaction } from "@pump-hooks/sdk";

const { values } = parseArgs({ options: { url: { type: "string" }, keypair: { type: "string" }, market: { type: "string" }, sol: { type: "string", default: "0.01" }, send: { type: "boolean", default: false } } });
if (!values.url || !values.keypair || !values.market) throw new Error("usage: proof-trades.ts --url <rpc> --keypair <wallet.json> --market <address> [--sol 0.01] [--send]");

const conn = new Connection(values.url, "confirmed");
const user = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(values.keypair, "utf8"))));
const client = new PumpHooksClient(conn, { lookupTables: [PUMP_ALT.mainnet], microLamportsPerCu: 200_000 });
const market = await client.getMarket(new PublicKey(values.market));
const token = await client.detect(market.config.mint);
const ata = PublicKey.findProgramAddressSync(
  [user.publicKey.toBuffer(), token.baseTokenProgram.toBuffer(), market.config.mint.toBuffer()],
  new PublicKey("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"),
)[0];
const balance = async () => BigInt((await conn.getTokenAccountBalance(ata).catch(() => ({ value: { amount: "0" } }))).value.amount);

async function step(label: string, built: BuiltTransaction, land: boolean) {
  const sim = await client.simulate(built, user.publicKey);
  console.log(`${label}: ${sim.ok ? "ALLOW" : `DENY ${sim.error?.code ?? ""} ${sim.error?.name ?? ""}${sim.denied ? ` (module ${sim.denied.moduleId})` : ""}`} · ${sim.unitsConsumed} CU`);
  if (!sim.ok && !sim.denied) console.log(sim.logs.slice(-6).join("\n"));
  if (!values.send || (!sim.ok && !land)) return sim.ok;
  const { blockhash, lastValidBlockHeight } = await conn.getLatestBlockhash("confirmed");
  const tx = built.toTransaction(blockhash, user.publicKey);
  tx.sign([user]);
  const sig = await conn.sendRawTransaction(tx.serialize(), { skipPreflight: true, maxRetries: 5 });
  for (;;) {
    const [st] = (await conn.getSignatureStatuses([sig])).value;
    if (st?.confirmationStatus === "confirmed" || st?.confirmationStatus === "finalized") {
      console.log(`  landed ${st.err ? "REJECTED" : "OK"}: ${sig}`);
      return !st.err;
    }
    if ((await conn.getBlockHeight("confirmed")) > lastValidBlockHeight) throw new Error(`expired: ${sig}`);
    await new Promise((r) => setTimeout(r, 1500));
  }
}

const quoteIn = BigInt(Math.round(Number(values.sol) * 1e9));
if (!(await step(`1. buy ${values.sol} SOL`, await client.buy({ market, user: user.publicKey, quoteIn }), false))) process.exit(1);
if (!values.send) process.exit(0);
const held = await balance();
console.log(`  holding ${held} base units`);
if (!(await step("2. sell half", await client.sell({ market, user: user.publicKey, baseIn: held / 2n }), false))) process.exit(1);
await step("3. sell the rest immediately", await client.sell({ market, user: user.publicKey, baseIn: await balance() }), true);
