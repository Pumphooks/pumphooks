import { describe, expect, it } from "vitest";
import { Keypair, Signer } from "@solana/web3.js";
import { ModuleId } from "@pump-hooks/sdk";
import { Env, fixturesAvailable, POOL_FIXTURE, SOL } from "../../sdk/test/harness.js";
import { parseConfig, run } from "../src/cli.js";
import { parseSol, parseUnits, sol } from "../src/format.js";

describe("parsing", () => {
  it("amounts", () => {
    expect(parseSol("0.25")).toBe(250_000_000n);
    expect(parseSol("1sol")).toBe(SOL);
    expect(parseSol("42lamports")).toBe(42n);
    expect(parseUnits("1.5", 6)).toBe(1_500_000n);
    expect(() => parseUnits("1.1234567", 6)).toThrow();
    expect(sol(1_500_000_000n)).toBe("1.500000000 SOL");
  });

  it("module config", () => {
    expect(parseConfig(ModuleId.VolumeGuard, "maxBuyQuote=1.5,windowSecs=600")).toEqual({ maxBuyQuote: 1_500_000_000n, windowSecs: 600 });
    expect(parseConfig(ModuleId.VolumeGuard, "maxBuyQuote=1,maxSellQuote=7lamports")).toEqual({ maxBuyQuote: SOL, maxSellQuote: 7n });
    expect(() => parseConfig(ModuleId.VolumeGuard, "nope=1")).toThrow(/fields: maxBuyQuote/);
  });
});

describe.skipIf(!fixturesAvailable())("commands against an existing PumpSwap token", () => {
  it("inspect → market create → install → simulate → trade", async () => {
    const env = new Env(POOL_FIXTURE);
    env.bootstrap();
    const owner = env.wallet(10n * SOL);
    const user = env.wallet(5n * SOL);
    const keys: Record<string, Keypair> = { owner, user };
    let out = "";
    const deps = {
      rpc: env.rpc,
      out: (s: string) => (out += s + "\n"),
      readKeypair: (p: string) => keys[p],
      clientOptions: env.client.opts,
      send: async (b: Parameters<Env["exec"]>[0], signers: Signer[]) => env.exec(b, signers as Keypair[]),
    };
    const cli = async (...a: string[]) => {
      out = "";
      return run(a, deps);
    };
    const mint = env.mint.toBase58();

    expect(await cli("status")).toBe(0);
    expect(out).toMatch(/registry +11\/11 first-party modules registered/);

    expect(await cli("inspect", mint)).toBe(0);
    expect(out).toMatch(/PumpSwap \(migrated\) pool 9Nhddo1P/);
    expect(out).toMatch(/routable +yes/);

    expect(await cli("hooks")).toBe(0);
    expect(out).toMatch(/volume-guard/);
    expect(out).not.toMatch(/AUDITED/);
    expect(out).toMatch(/unaudited/);

    expect(await cli("market", "create", mint, "--keypair", "owner")).toBe(0);
    const market = env.client.marketAddress(env.mint, owner.publicKey).toBase58();
    expect(out).toMatch(new RegExp(`market ${market}`));
    expect(out).toMatch(/MarketCreated/);

    expect(await cli("install", market, "volume-guard", "--config", "maxBuyQuote=1,maxSellQuote=1", "--keypair", "owner")).toBe(0);
    expect(await cli("install", market, "holder-xp", "--config", "perTradeCapQuote=10,xpPerSol=100,vestSecs=3600,dailyCap=10000", "--keypair", "owner")).toBe(0);
    expect(await cli("market", "show", market)).toBe(0);
    expect(out).toMatch(/Volume Guard +1100 +BeforeSwap,AfterSwap/);
    expect(out).toMatch(/maxBuyQuote=1\.000000000 /);

    expect(await cli("quote", mint, "--buy", "0.1")).toBe(0);
    expect(out).toMatch(/constant-product, pre-fee/);

    expect(await cli("simulate", market, "--buy", "0.1", "--user", user.publicKey.toBase58())).toBe(0);
    expect(out).toMatch(/simulated, not sent/);
    expect(out).toMatch(/BeforeSwap +Volume Guard +ALLOW/);

    expect(await cli("trade", market, "--buy", "2", "--keypair", "user")).toBe(1);
    expect(out).toMatch(/denied by +Volume Guard at BeforeSwap: 2001/);

    expect(await cli("trade", market, "--buy", "0.2", "--keypair", "user")).toBe(0);
    expect(out).toMatch(/swap +BUY .* for 0\.200000000 SOL \(observed\)/);
    expect(out).toMatch(/AfterBuy +Holder XP +ALLOW/);

    expect(await cli("uninstall", market, "holder-xp", "--keypair", "owner", "--dry-run")).toBe(0);
    expect(out).toMatch(/HookRemoved/);

    expect(await cli("install", market, "volume-guard", "--keypair", "owner")).toBe(1);
    expect(out).toMatch(/BadHookConfig/);
    expect(await cli("install", market, "volume-guard", "--config", "maxBuyQuote=2", "--keypair", "owner")).toBe(1);
    expect(out).toMatch(/Duplicate/i);

    expect(await cli("frobnicate")).toBe(2);
    expect(await cli("market", "create", mint)).toBe(2);
    expect(out).toMatch(/--keypair/);
  });
});
