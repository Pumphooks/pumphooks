/**
 * The SDK flow the website uses (detect → create market → install rules → buy / sell), against
 * pump.fun's current mainnet binaries, the mainnet lookup table and live mainnet state.
 */
import { describe, expect, it } from "vitest";
import {
  associatedToken,
  CONFIG_PRESETS,
  decodeConfig,
  decodeTokenAccount,
  encodeConfig,
  ModuleId,
  validateConfig,
  Venue,
} from "../src/index.js";
import { Env, MAINNET_CURVE_FIXTURE, MAINNET_POOL_FIXTURE, mainnetFixturesAvailable, SOL } from "./harness.js";

const NEW_RULES = [ModuleId.MaxWallet, ModuleId.SellLimit, ModuleId.TradingHours] as const;

describe("new rule presets", () => {
  it("pass validation and round-trip through the 48-byte encoding", () => {
    for (const m of NEW_RULES) {
      const preset = CONFIG_PRESETS[m]!;
      expect(validateConfig(m, preset)).toEqual([]);
      const decoded = decodeConfig(m, encodeConfig(m, preset));
      for (const [k, v] of Object.entries(preset)) expect(BigInt(decoded[k] as number | bigint)).toBe(BigInt(v as number | bigint));
    }
    expect(decodeConfig(ModuleId.TradingHours, encodeConfig(ModuleId.TradingHours, { ...CONFIG_PRESETS[ModuleId.TradingHours], tzOffsetMin: -720 })).tzOffsetMin).toBe(-720);
  });
});

describe.skipIf(!mainnetFixturesAvailable())("mainnet pump.fun via SDK", () => {
  it("curve coin: every new rule's preset installs on chain; max wallet denies a buy over the cap", async () => {
    const env = new Env(MAINNET_CURVE_FIXTURE);
    env.bootstrap();
    const owner = env.wallet(10n * SOL);
    const t = await env.client.detect(env.mint);
    expect(t.venue).toBe(Venue.BondingCurve);

    const { market, ix } = await env.client.createMarket(env.mint, owner.publicKey);
    const presets = NEW_RULES.filter((m) => m !== ModuleId.TradingHours).map((module) =>
      env.client.installHook({ market, authority: owner.publicKey, module, config: CONFIG_PRESETS[module] }),
    );
    const setup = env.ixs([ix, ...presets], [owner]);
    expect(setup.ok, setup.logs.join("\n")).toBe(true);

    const user = env.wallet(30n * SOL);
    const buy = env.exec(await env.client.buy({ market, user: user.publicKey, quoteIn: SOL / 10n }), [user]);
    expect(buy.ok, buy.logs.join("\n")).toBe(true);
    const ata = associatedToken(user.publicKey, t.baseTokenProgram, env.mint);
    expect(decodeTokenAccount(env.svm.getAccount(ata)!.data).amount).toBe(buy.swap!.baseAmount);

    // The 2%-of-supply preset is far above 0.1 SOL of tokens but well below 25 SOL of them.
    env.advance(5n, 12n);
    const whale = env.exec(await env.client.buy({ market, user: user.publicKey, quoteIn: 25n * SOL }), [user]);
    expect(whale.denied).toMatchObject({ moduleId: ModuleId.MaxWallet, reasonCode: 10001 });
  });

  it("pool coin: trading hours preset installs; sell limit denies dumping the whole bag", async () => {
    const env = new Env(MAINNET_POOL_FIXTURE);
    env.bootstrap();
    const owner = env.wallet(10n * SOL);
    const t = await env.client.detect(env.mint);
    expect(t.venue).toBe(Venue.PumpSwap);
    const { market, ix } = await env.client.createMarket(env.mint, owner.publicKey);
    const sellLimit = { maxSellBps: 5_000, cooldownSecs: 60, exemptBelow: 0n };
    const setup = env.ixs(
      [
        ix,
        env.client.installHook({ market, authority: owner.publicKey, module: ModuleId.SellLimit, config: sellLimit }),
        env.client.installHook({ market, authority: owner.publicKey, module: ModuleId.TradingHours, config: { ...CONFIG_PRESETS[ModuleId.TradingHours], weekdays: 0, openMin: 0, closeMin: 1439, dst: 0, tzOffsetMin: 0 } }),
      ],
      [owner],
    );
    expect(setup.ok, setup.logs.join("\n")).toBe(true);

    const user = env.wallet(5n * SOL);
    const buy = env.exec(await env.client.buy({ market, user: user.publicKey, quoteIn: SOL / 10n }), [user]);
    expect(buy.ok, buy.logs.join("\n")).toBe(true);
    const bag = buy.swap!.baseAmount;

    env.advance(5n, 12n);
    const dump = env.exec(await env.client.sell({ market, user: user.publicKey, baseIn: bag }), [user]);
    expect(dump.denied).toMatchObject({ moduleId: ModuleId.SellLimit, reasonCode: 11001 });
    const half = env.exec(await env.client.sell({ market, user: user.publicKey, baseIn: bag / 2n }), [user]);
    expect(half.ok, half.logs.join("\n")).toBe(true);
  });
});
