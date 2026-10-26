/**
 * Launching a new coin with rules attached, against pump.fun's mainnet binaries and live mainnet
 * Global / mayhem state (fixture `launch-mainnet`): create_v2 → create_market → installs in one
 * transaction, then a routed first buy built before the coin existed.
 */
import { existsSync } from "node:fs";
import { join } from "node:path";
import { Keypair, TransactionMessage, VersionedTransaction } from "@solana/web3.js";
import { describe, expect, it } from "vitest";
import {
  associatedToken,
  CONFIG_PRESETS,
  createCoinIx,
  decodeCurve,
  decodeTokenAccount,
  MARKET_FLAGS,
  MAYHEM_PROGRAM_ID,
  ModuleId,
  pumpPda,
  TOKEN_2022_PROGRAM_ID,
  Venue,
} from "../src/index.js";
import { Env, mainnetFixturesAvailable, ROOT, SOL } from "./harness.js";

const MAYHEM_SO = join(ROOT, "tests/fixtures/programs/mayhem_mainnet.so");
const available = mainnetFixturesAvailable() && existsSync(MAYHEM_SO);

function launchEnv() {
  const env = new Env("launch-mainnet");
  env.svm.addProgramFromFile(MAYHEM_PROGRAM_ID, MAYHEM_SO);
  env.bootstrap();
  return env;
}

describe("createCoinIx", () => {
  it("rejects metadata over Pump's limits", () => {
    const k = Keypair.generate().publicKey;
    expect(() => createCoinIx({ mint: k, user: k, name: "x".repeat(33), symbol: "X", uri: "https://x" })).toThrow(/name/);
    expect(() => createCoinIx({ mint: k, user: k, name: "X", symbol: "X".repeat(11), uri: "https://x" })).toThrow(/symbol/);
  });
});

describe.skipIf(!available)("launch on mainnet pump.fun", () => {
  it("creates a coin with rules in one transaction; the first buy and later trades go through them", async () => {
    const env = launchEnv();
    const creator = env.wallet(10n * SOL);
    const mint = Keypair.generate();
    const hooks = [
      { module: ModuleId.MaxWallet, config: CONFIG_PRESETS[ModuleId.MaxWallet] },
      { module: ModuleId.SellLimit, config: CONFIG_PRESETS[ModuleId.SellLimit] },
      { module: ModuleId.HolderXp, config: CONFIG_PRESETS[ModuleId.HolderXp] },
    ];
    const { market, create, firstBuy } = await env.client.launch({
      mint: mint.publicKey,
      creator: creator.publicKey,
      name: "Launch Test",
      symbol: "LTEST",
      uri: "https://example.com/metadata/0123456789abcdef0123456789abcdef.json",
      hooks,
      firstBuy: { quoteIn: SOL / 2n },
    });

    // The whole create step fits one v0 transaction with the Pump lookup table.
    const alts = await env.client.loadLookupTables();
    const msg = new TransactionMessage({ payerKey: creator.publicKey, recentBlockhash: env.svm.latestBlockhash(), instructions: create }).compileToV0Message(alts);
    const size = new VersionedTransaction(msg).serialize().length;
    expect(size).toBeLessThanOrEqual(1232);

    const before = env.svm.getBalance(creator.publicKey)!;
    const created = env.ixs(create, [creator, mint]);
    const cost = before - env.svm.getBalance(creator.publicKey)!;
    expect(created.ok, created.logs.join("\n")).toBe(true);

    const curve = decodeCurve(env.svm.getAccount(pumpPda.bondingCurve(mint.publicKey))!.data);
    expect(curve.creator.equals(creator.publicKey)).toBe(true);
    expect(curve.complete).toBe(false);
    expect(env.svm.getAccount(mint.publicKey)!.owner.equals(TOKEN_2022_PROGRAM_ID)).toBe(true);

    const m = await env.client.getMarket(market);
    expect(m.config.venue).toBe(Venue.BondingCurve);
    expect(m.config.flags & MARKET_FLAGS.CREATOR_VERIFIED).toBeTruthy();
    expect(m.config.slots.map((s) => s.moduleId).sort()).toEqual(hooks.map((h) => h.module).sort());

    const token = await env.client.detect(mint.publicKey);
    expect(token.routable).toBe(true);

    env.advance(1n);
    const first = env.exec(firstBuy!, [creator]);
    expect(first.ok, first.logs.join("\n")).toBe(true);
    const ata = associatedToken(creator.publicKey, TOKEN_2022_PROGRAM_ID, mint.publicKey);
    expect(decodeTokenAccount(env.svm.getAccount(ata)!.data).amount).toBe(first.swap!.baseAmount);

    const user = env.wallet(40n * SOL);
    env.advance(2n);
    const buy = env.exec(await env.client.buy({ market, user: user.publicKey, quoteIn: SOL / 10n }), [user]);
    expect(buy.ok, buy.logs.join("\n")).toBe(true);
    env.advance(5n, 12n);
    const whale = env.exec(await env.client.buy({ market, user: user.publicKey, quoteIn: 30n * SOL }), [user]);
    expect(whale.denied).toMatchObject({ moduleId: ModuleId.MaxWallet });
    console.log(`launch: create costs ${Number(cost) / 1e9} SOL; tx ${size} bytes, ${created.unitsConsumed} CU; first buy ${first.unitsConsumed} CU`);
  });
});
