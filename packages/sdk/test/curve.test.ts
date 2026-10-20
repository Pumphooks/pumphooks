/** SDK routed trades on an existing, un-migrated Pump coin (bonding curve, real `pump` binary). */
import { describe, expect, it } from "vitest";
import { associatedToken, Callback, decodeTokenAccount, ModuleId, Venue } from "../src/index.js";
import { DEEP_CURVE_FIXTURE, Env, fixturesAvailable, SOL } from "./harness.js";

describe.skipIf(!fixturesAvailable())("existing bonding-curve token via SDK", () => {
  it("detects, hooks, buys and sells", async () => {
    const env = new Env(DEEP_CURVE_FIXTURE);
    env.bootstrap();
    const owner = env.wallet(10n * SOL);
    const t = await env.client.detect(env.mint);
    expect(t.venue).toBe(Venue.BondingCurve);
    expect(t.curve.complete).toBe(false);
    expect(t.isToken2022).toBe(false);

    const { market, ix } = await env.client.createMarket(env.mint, owner.publicKey);
    const setup = env.ixs(
      [ix, env.client.installHook({ market, authority: owner.publicKey, module: ModuleId.LaunchShield, config: { durationSecs: 86_400, cooldownSecs: 0, maxBuyQuote: SOL } })],
      [owner],
    );
    expect(setup.ok, setup.logs.join("\n")).toBe(true);

    const user = env.wallet(3n * SOL);
    const buy = env.exec(await env.client.buy({ market, user: user.publicKey, quoteIn: SOL / 5n }), [user]);
    expect(buy.ok, buy.logs.join("\n")).toBe(true);
    expect(buy.trace!.entries.map((e) => [Callback[e.callback], e.moduleId])).toEqual([["BeforeBuy", 1]]);
    expect(buy.swap!.quoteAmount).toBe(SOL / 5n);
    const ata = associatedToken(user.publicKey, t.baseTokenProgram, env.mint);
    const bal = decodeTokenAccount(env.svm.getAccount(ata)!.data).amount;
    expect(buy.swap!.baseAmount).toBe(bal);

    const big = env.exec(await env.client.buy({ market, user: user.publicKey, quoteIn: 2n * SOL }), [user]);
    expect(big.denied).toMatchObject({ moduleId: ModuleId.LaunchShield, reasonCode: 1001 });

    env.advance(5n, 12n);
    const sell = env.exec(await env.client.sell({ market, user: user.publicKey, baseIn: bal }), [user]);
    expect(sell.ok, sell.logs.join("\n")).toBe(true);
    expect(decodeTokenAccount(env.svm.getAccount(ata)!.data).amount).toBe(0n);
  });
});
