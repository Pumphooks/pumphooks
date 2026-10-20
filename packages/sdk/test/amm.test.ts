/**
 * End to end through the SDK on an existing (pre-existing, migrated) devnet Pump token cloned into
 * LiteSVM, executing against Pump's real pump_amm binary.
 */
import { beforeEach, describe, expect, it } from "vitest";
import { Keypair } from "@solana/web3.js";
import nacl from "tweetnacl";
import {
  associatedToken,
  Callback,
  decodeSentientState,
  decodeTokenAccount,
  encodeSignal,
  ModuleId,
  NATIVE_MINT,
  Regime,
  regimeSignal,
  signalMessage,
  TOKEN_PROGRAM_ID,
  Venue,
} from "../src/index.js";
import { Env, fixturesAvailable, POOL_FIXTURE, SOL } from "./harness.js";

describe.skipIf(!fixturesAvailable())("existing PumpSwap token via SDK", () => {
  let env: Env;
  let owner: Keypair;
  let engine: Keypair;

  beforeEach(async () => {
    env = new Env(POOL_FIXTURE);
    env.bootstrap();
    owner = env.wallet(10n * SOL);
    engine = Keypair.generate();
  });

  async function hook() {
    const c = env.client;
    const { market, ix, token } = await c.createMarket(env.mint, owner.publicKey);
    const p = owner.publicKey;
    const r = env.ixs(
      [
        ix,
        c.installHook({ market, authority: p, module: "volume-guard", config: { maxBuyQuote: SOL, maxSellQuote: SOL } }),
        c.installHook({ market, authority: p, module: ModuleId.HolderXp, config: { perTradeCapQuote: 10n * SOL, xpPerSol: 100, vestSecs: 3600, dailyCap: 10_000, minTradeQuote: SOL / 100n } }),
        c.installHook({ market, authority: p, module: "sentient", config: { signer: engine.publicKey, defensiveMaxQuote: SOL / 10n, maxSignalTtl: 3600 } }),
        c.installHook({ market, authority: p, module: "twap", config: { maxTotalQuote: 5n * SOL, maxSliceQuote: SOL / 2n, minIntervalSecs: 60, maxDurationSecs: 86_400, maxSlippageBps: 100 } }),
      ],
      [owner],
    );
    expect(r.ok, r.logs.join("\n")).toBe(true);
    expect(r.events.map((e) => e.name)).toEqual(["MarketCreated", "HookInstalled", "HookInstalled", "HookInstalled", "HookInstalled"]);
    return { market, token };
  }

  it("detects the existing token and its PumpSwap pool", async () => {
    const t = await env.client.detect(env.mint);
    expect(t.venue).toBe(Venue.PumpSwap);
    expect(t.pool.toBase58()).toBe(POOL_FIXTURE);
    expect(t.quoteMint.equals(NATIVE_MINT)).toBe(true);
    expect(t.curve.complete).toBe(true);
    expect(t.routable).toBe(true);
    expect(t.reserves.base > 0n && t.reserves.quote > 0n).toBe(true);
  });

  it("registry definitions match the SDK module table", async () => {
    for (const id of [1, 2, 3, 4, 5, 6, 8, 9]) {
      const d = (await env.client.getHookDefinition(id))!;
      const m = (await import("../src/index.js")).moduleInfo(id);
      expect([d.callbacks, d.capabilities, d.priorityMin, d.priorityMax, d.runtime, d.auditStatus]).toEqual([m.callbacks, m.capabilities, m.priorityMin, m.priorityMax, m.runtime, 0]);
    }
  });

  it("hooks the token, routes a buy through the stack and decodes the trace", async () => {
    const { market } = await hook();
    const m = await env.client.getMarket(market);
    expect(m.config.slots.map((s) => s.module?.slug)).toEqual(["volume-guard", "holder-xp", "twap", "sentient"]);
    expect(m.config.slots[0].decodedConfig.maxBuyQuote).toBe(SOL);

    const user = env.wallet(5n * SOL);
    const est = await env.client.quoteBuy(env.mint, SOL / 4n);
    const built = await env.client.buy({ market: m, user: user.publicKey, quoteIn: SOL / 4n });
    const sim = await env.client.simulate(built, user.publicKey);
    expect(sim.ok, `${String(sim.rawError)}\n${sim.logs.join("\n")}`).toBe(true);

    const r = env.exec(built, [user]);
    expect(r.ok, r.logs.join("\n")).toBe(true);
    expect(r.trace!.entries.map((e) => [Callback[e.callback], e.moduleId, e.allowed])).toEqual([
      ["BeforeSwap", 2, true],
      ["BeforeSwap", 8, true],
      ["AfterBuy", 4, true],
      ["AfterSwap", 2, true],
      ["AfterSwap", 8, true],
    ]);
    const swap = r.swap!;
    expect(swap.quoteAmount).toBe(SOL / 4n);
    const bal = decodeTokenAccount(env.svm.getAccount(associatedToken(user.publicKey, m.config.baseTokenProgram, env.mint))!.data).amount;
    expect(swap.baseAmount).toBe(bal);
    expect(bal).toBeLessThanOrEqual(est.baseOut!); // fees make the executed fill smaller than the fee-free estimate
    expect(sim.swap!.baseAmount).toBe(bal);
    // wSOL account created by this trade is closed again.
    expect(env.svm.getAccount(associatedToken(user.publicKey, TOKEN_PROGRAM_ID, NATIVE_MINT))).toBeNull();

    const us = (await env.client.getUserState(market, user.publicKey))!;
    expect(us.state.pendingXp).toBe(25n);
    expect(us.state.routedBuyQuote).toBe(SOL / 4n);

    // Sell everything back through the stack.
    env.advance(10n, 25n);
    const sell = env.exec(await env.client.sell({ market, user: user.publicKey, baseIn: bal }), [user]);
    expect(sell.ok, sell.logs.join("\n")).toBe(true);
    expect(sell.swap!.side).toBe(1);
    expect(sell.swap!.quoteAmount > 0n).toBe(true);
    expect((await env.client.getUserState(market, user.publicKey))!.state.forfeitedXp).toBe(25n);
  });

  it("a denial reverts the trade and is decoded", async () => {
    const { market } = await hook();
    const user = env.wallet(5n * SOL);
    const before = env.lamports(user.publicKey);
    const r = env.exec(await env.client.buy({ market, user: user.publicKey, quoteIn: 2n * SOL }), [user]);
    expect(r.ok).toBe(false);
    expect(r.error?.name).toBe("HookDenied");
    expect(r.denied).toMatchObject({ moduleId: ModuleId.VolumeGuard, reasonCode: 2001, callback: Callback.BeforeSwap });
    expect(before - env.lamports(user.publicKey)).toBe(5000n); // only the fee
  });

  it("SENTIENT: a relayed signed signal switches the regime, which then caps routed size", async () => {
    const { market } = await hook();
    const relayer = env.wallet(SOL);
    const enc = encodeSignal(regimeSignal(market, Regime.Defensive, 1n, env.now(), 600n, 8000));
    const signature = nacl.sign.detached(signalMessage(enc), engine.secretKey);
    const r = env.exec(env.client.submitSignal({ market, signer: engine.publicKey, signal: enc, signature, payer: relayer.publicKey }), [relayer]);
    expect(r.ok, r.logs.join("\n")).toBe(true);
    expect(r.events[0]).toMatchObject({ name: "StrategySignalApplied", data: { nonce: 1n, param: Regime.Defensive } });
    const slot = (await env.client.getMarket(market)).config.slots.find((s) => s.moduleId === ModuleId.Sentient)!;
    expect(decodeSentientState(slot.state).regime).toBe(Regime.Defensive);

    // Replay is rejected.
    env.svm.expireBlockhash();
    const replay = env.exec(env.client.submitSignal({ market, signer: engine.publicKey, signal: enc, signature, payer: relayer.publicKey }), [relayer]);
    expect(replay.ok).toBe(false);

    // A signature from any other key is rejected.
    const forged = encodeSignal(regimeSignal(market, Regime.Normal, 2n, env.now(), 600n));
    const bad = env.exec(
      env.client.submitSignal({ market, signer: engine.publicKey, signal: forged, signature: nacl.sign.detached(signalMessage(forged), Keypair.generate().secretKey), payer: relayer.publicKey }),
      [relayer],
    );
    expect(bad.ok).toBe(false);

    const user = env.wallet(5n * SOL);
    const denied = env.exec(await env.client.buy({ market, user: user.publicKey, quoteIn: SOL / 2n }), [user]);
    expect(denied.denied, `${JSON.stringify(denied.rawError)} ${denied.logs.join("\n")}`).toMatchObject({ moduleId: ModuleId.Sentient, reasonCode: 8001 });
    const small = env.exec(await env.client.buy({ market, user: user.publicKey, quoteIn: SOL / 20n }), [user]);
    expect(small.ok, small.logs.join("\n")).toBe(true);
  });

  it("Composer policy compiled by the SDK is enforced on-chain", async () => {
    const { market } = await hook();
    const p = owner.publicKey;
    expect(env.ixs([env.client.installHook({ market, authority: p, module: "policy" })], [owner]).ok).toBe(true);
    const m = await env.client.getMarket(market);
    const { index, ix } = env.client.createPolicy({
      market: m,
      authority: p,
      spec: { when: "AfterBuy", if: [{ field: "TRADE_SIZE", op: ">=", value: SOL / 10n }], then: [{ kind: "AWARD_POINTS", points: 50 }], label: "reward routed buys" },
    });
    expect(index).toBe(0);
    const created = env.ixs([ix], [owner]);
    expect(created.ok, created.logs.join("\n")).toBe(true);
    const policies = await env.client.getPolicies(await env.client.getMarket(market));
    expect(policies).toHaveLength(1);
    expect(policies[0].actions[0]).toMatchObject({ kind: 4, value: 50n });

    const user = env.wallet(5n * SOL);
    const r = env.exec(await env.client.buy({ market, user: user.publicKey, quoteIn: SOL / 5n }), [user]);
    expect(r.ok, r.logs.join("\n")).toBe(true);
    expect(r.policies[0]).toMatchObject({ policyIndex: 0, matched: true });
    expect(r.swap!.policyPoints).toBe(50n);
  });
});
