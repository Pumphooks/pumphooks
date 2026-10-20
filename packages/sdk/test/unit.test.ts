import { describe, expect, it } from "vitest";
import { Keypair, PublicKey } from "@solana/web3.js";
import {
  ammPda,
  Callback,
  callbackMask,
  compilePolicy,
  cp,
  decodeConfig,
  decodeEvents,
  describePolicy,
  encodeConfig,
  encodeSignal,
  eventDiscriminator,
  hooksPda,
  ModuleId,
  MODULES,
  NATIVE_MINT,
  PolicyCompileError,
  PROGRAM_ERRORS,
  pumpPda,
  PUMP_HOOKS_PROGRAM_ID,
  Regime,
  regimeSignal,
  signalMessage,
  Writer,
} from "../src/index.js";

describe("pda", () => {
  it("canonical pool and bonding curve match the cloned devnet token", () => {
    const mint = new PublicKey("44yV5xk8C1QVRGz82Q8KLcpft5gqbNd3SG8Lt1NrhLo9");
    expect(ammPda.canonicalPool(mint, NATIVE_MINT).toBase58()).toBe("9Nhddo1PcCBms54wcLYbxoRcAJbpmSvHWMcbdMf9tdmZ");
    const curveMint = new PublicKey("3LP8SpdbbvY7Rk7DR8VfhntB2tugXFARBkvM5E73Ljwv");
    expect(pumpPda.bondingCurve(curveMint).toBase58()).toBe("AwHTvGo8UcR4CEyqmTWs1xEbeajBNwC5ELFdMdPyB33w");
  });

  it("market is namespaced by authority", () => {
    const mint = Keypair.generate().publicKey;
    const a = Keypair.generate().publicKey;
    const b = Keypair.generate().publicKey;
    expect(hooksPda.market(mint, a).equals(hooksPda.market(mint, b))).toBe(false);
    expect(hooksPda.market(mint, a).equals(hooksPda.market(mint, a))).toBe(true);
  });
});

describe("module table", () => {
  it("is indexed by id and internally consistent", () => {
    MODULES.forEach((m, i) => {
      expect(m.id).toBe(i + 1);
      expect(m.defaultCallbacks & ~m.callbacks).toBe(0);
      expect(m.runtime).toBe(m.callbacks !== 0);
      expect(m.capabilities & (1 << 8)).toBe(0); // no EXTERNAL_CPI in v1
    });
  });
});

describe("configs", () => {
  it("round-trips Volume Guard and SENTIENT", () => {
    const vg = { maxBuyQuote: 5n * 10n ** 9n, maxSellQuote: 1n, windowSecs: 3600, windowMaxQuote: 7n, walletWindowMaxQuote: 9n };
    const enc = encodeConfig(ModuleId.VolumeGuard, vg);
    expect(enc.length).toBe(48);
    expect(decodeConfig(ModuleId.VolumeGuard, enc)).toEqual(vg);
    const signer = Keypair.generate().publicKey;
    const s = decodeConfig(ModuleId.Sentient, encodeConfig(ModuleId.Sentient, { signer, defensiveMaxQuote: 3n, maxSignalTtl: 600 }));
    expect((s.signer as PublicKey).equals(signer)).toBe(true);
    expect(s.maxSignalTtl).toBe(600);
    expect(s.flags).toBe(3); // guards cover buys and sells unless told otherwise
    expect(decodeConfig(ModuleId.Sentient, encodeConfig(ModuleId.Sentient, { signer, flags: 1 })).flags).toBe(1);
  });

  it("rejects unknown fields", () => {
    expect(() => encodeConfig(ModuleId.VolumeGuard, { maxBuy: 1n })).toThrow(/unknown config field/);
  });
});

describe("composer", () => {
  it("compiles WHEN/IF/THEN", () => {
    const p = compilePolicy({
      when: "BeforeSell",
      if: [{ field: "TRADE_SIZE", op: ">", value: 2n * 10n ** 9n }, { field: "HOLDER_DURATION", op: "<", value: 3600 }],
      then: [{ kind: "DENY" }],
      denyReason: 7,
      label: "no early dumps",
    });
    expect(p.trigger).toBe(Callback.BeforeSell);
    expect(p.conditions).toHaveLength(2);
    expect(p.actions[0].kind).toBe(2);
    expect(describePolicy(p)).toBe("WHEN BeforeSell IF TRADE_SIZE > 2000000000 AND HOLDER_DURATION < 3600 THEN DENY(9007)");
  });

  it("enforces the VM's rules", () => {
    expect(() => compilePolicy({ when: "AfterBuy", then: [{ kind: "DENY" }] })).toThrow(PolicyCompileError);
    expect(() => compilePolicy({ when: "BeforeBuy", then: [{ kind: "AWARD_POINTS", points: 1 }] })).toThrow(/After/);
    expect(() => compilePolicy({ when: "AfterBuy", then: [{ kind: "AWARD_POINTS", points: 10_001 }] })).toThrow(/10000/);
    expect(() => compilePolicy({ when: "BeforeBuy", then: [{ kind: "DENY" }, { kind: "DENY" }] })).toThrow(/one DENY/);
    expect(() => compilePolicy({ when: "BeforeBuy", if: [{ field: "VOLATILITY", op: ">", value: 1 }], then: [{ kind: "DENY" }] })).toThrow(/Volatility Guard/);
    expect(() => compilePolicy({ when: "BeforeBuy", if: [{ field: "TREASURY_BALANCE", op: ">", value: 1 }], then: [{ kind: "DENY" }] })).toThrow(/not available/);
    expect(() => compilePolicy({ when: "AfterBuy", then: [] })).toThrow(/at least one/);
  });
});

describe("events", () => {
  it("only decodes data logged while Pump Hooks is executing", () => {
    const disc = eventDiscriminator("HookRemoved");
    const body = new Writer().bytes(disc).pubkey(PUMP_HOOKS_PROGRAM_ID).u16(4).done().toString("base64");
    const logs = [
      `Program ${PUMP_HOOKS_PROGRAM_ID.toBase58()} invoke [1]`,
      `Program pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA invoke [2]`,
      `Program data: ${body}`,
      `Program pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA success`,
      `Program data: ${body}`,
      `Program ${PUMP_HOOKS_PROGRAM_ID.toBase58()} success`,
    ];
    const ev = decodeEvents(logs);
    expect(ev).toHaveLength(1);
    expect(ev[0]).toMatchObject({ name: "HookRemoved", data: { moduleId: 4 } });
  });

  it("error table starts at 6000 and includes the router's denial", () => {
    expect(PROGRAM_ERRORS[0].code).toBe(6000);
    expect(PROGRAM_ERRORS.find((e) => e.name === "HookDenied")?.code).toBe(6018);
  });
});

describe("pricing", () => {
  it("matches the adapter's constant-product math", () => {
    expect(cp.baseOut(1000n, 1000n, 1000n)).toBe(500n);
    expect(cp.quoteOut(1000n, 1000n, 1000n)).toBe(500n);
    expect(cp.quoteIn(500n, 1000n, 1000n)).toBe(1000n);
    expect(cp.quoteIn(1000n, 1000n, 1000n)).toBe(2n ** 64n - 1n);
  });
});

describe("signals", () => {
  it("are 96 bytes and domain separated", () => {
    const enc = encodeSignal(regimeSignal(PUMP_HOOKS_PROGRAM_ID, Regime.Defensive, 1n, 1000n, 300n));
    expect(enc.length).toBe(96);
    const msg = signalMessage(enc);
    expect(Buffer.from(msg.subarray(0, 20)).toString()).toBe("PUMP_HOOKS_SIGNAL_V1");
    expect(msg.length).toBe(116);
  });

  it("callback masks", () => {
    expect(callbackMask(Callback.BeforeSwap, Callback.AfterSwap)).toBe(0b110000);
  });
});
