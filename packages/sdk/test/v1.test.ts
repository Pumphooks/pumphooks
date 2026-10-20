import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { Keypair } from "@solana/web3.js";
import { v1 } from "../src/index.js";

const vectors = JSON.parse(readFileSync(new URL("./vectors/kernel.json", import.meta.url), "utf8")) as {
  builtins: { id: number; caps: number; estimate: number; callbacks: number[] }[];
  entries: { name: string; hex: string }[];
};
const hex = (b: Uint8Array) => Buffer.from(b).toString("hex");
const k = (b: number) => new Uint8Array(32).fill(b);
const T = v1.Environment.Token;

describe("sdk v1 mirrors the kernel", () => {
  it("capabilities, estimates and callback masks", () => {
    for (const b of vectors.builtins) {
      expect(v1.requiredCaps(b.id), `caps ${b.id}`).toBe(b.caps);
      expect(v1.computeEstimate(b.id), `estimate ${b.id}`).toBe(b.estimate);
      b.callbacks.forEach((mask, env) => expect(v1.supportedCallbacks(b.id, env as v1.EnvironmentId), `callbacks ${b.id}/${env}`).toBe(mask));
    }
  });

  it("entry encodings", () => {
    const built: Record<string, Uint8Array> = {
      allowlist: v1.encodeEntry(T, { builtin: 1, callbacks: [0], mandatory: true, config: v1.configs.allowlistInline(v1.Target.Counterparty, [k(1), k(2)]) }),
      denylist: v1.encodeEntry(T, { builtin: 9, callbacks: [0], mandatory: true, config: v1.configs.denylist(v1.Target.Actor, [k(3)]) }),
      "max-size": v1.encodeEntry(T, { builtin: 6, callbacks: [0], config: v1.configs.maxSize(k(4), 1_000_000n) }),
      "spending-limit": v1.encodeEntry(T, { builtin: 2, callbacks: [0], mandatory: true, config: v1.configs.spendingLimit(k(4), 5_000n, 86_400) }),
      "time-window": v1.encodeEntry(T, { builtin: 7, callbacks: [0], config: v1.configs.timeWindow({ notBefore: 1_700_000_000n, notAfter: 1_900_000_000n, dailyStart: 3600, dailyEnd: 7200 }) }),
      timelock: v1.encodeEntry(T, { builtin: 3, callbacks: [0, 3], mandatory: true, config: v1.configs.timelock(600, 1_800_000_000n) }),
      accounting: v1.encodeEntry(T, { builtin: 8, callbacks: [0, 2, 3] }),
      credential: v1.encodeEntry(T, { builtin: 13, callbacks: [0], mandatory: true, config: v1.configs.credential(v1.Target.Counterparty, k(5)) }),
      "slot-limit": v1.encodeEntry(v1.Environment.Router, { builtin: 14, callbacks: [2], config: v1.configs.slotLimit(3) }),
      multisig: v1.encodeEntry(v1.Environment.Account, { builtin: 4, callbacks: [2], mandatory: true, config: v1.configs.multisig(2, 0b111) }),
      "price-impact": v1.encodeEntry(v1.Environment.Pool, { builtin: 11, callbacks: [6], config: v1.configs.priceImpact(150) }),
    };
    expect(Object.keys(built).sort()).toEqual(vectors.entries.map((e: { name: string }) => e.name).sort());
    for (const e of vectors.entries) expect(hex(built[e.name]), e.name).toBe(e.hex);
  });

  it("rejects callbacks a builtin cannot run on", () => {
    expect(() => v1.encodeEntry(T, { builtin: 2, callbacks: [2] })).toThrow(/cannot run/);
    expect(() => v1.encodeEntry(T, { builtin: 14 })).toThrow(/cannot run/);
  });

  it("stack and policy round-trip", () => {
    const mint = Keypair.generate().publicKey;
    const stack = v1.encodeStack(T, [{ builtin: 6, config: v1.configs.maxSize(mint.toBytes(), 10n) }]);
    const d = v1.decodeStack(stack);
    expect(d.entries[0].slug).toBe("max-size");
    expect(d.entries[0].callbacks).toBe(0b1101);

    const policy = new Uint8Array(v1.TOKEN_POLICY_STACK_OFFSET + 1736);
    policy.set(v1.anchorDiscriminator("TokenPolicy", "account"));
    policy.set(mint.toBytes(), 8);
    policy[128] = 254;
    policy[129] = 253;
    policy.set(stack, v1.TOKEN_POLICY_STACK_OFFSET);
    const p = v1.decodeTokenPolicy(policy);
    expect(p.mint).toBe(mint.toBase58());
    expect([p.bump, p.metasBump]).toEqual([254, 253]);
    expect(p.stack.entries).toHaveLength(1);
  });

  it("instruction layouts", () => {
    const [payer, mint, u] = [0, 1, 2].map(() => Keypair.generate().publicKey);
    const ix = v1.createTokenPolicy({ payer, authority: payer, mint, stack: v1.encodeStack(T, []) });
    expect(ix.accounts.map((a) => [a.signer, a.writable])).toEqual([[true, true], [true, false], [false, false], [false, true], [false, true], [false, false]]);
    expect(hex(ix.data.slice(0, 8))).toBe(hex(v1.anchorDiscriminator("create_policy")));
    expect(() => v1.createTokenPolicy({ payer, authority: payer, mint, stack: v1.encodeStack(v1.Environment.Pool, []) })).toThrow();

    const w = v1.wrap({ user: payer, underlyingMint: u, underlyingProgram: v1.SPL_TOKEN_PROGRAM_ID, userUnderlying: mint, userWrapped: mint, amount: 5n, minOut: 4n });
    expect(w.accounts).toHaveLength(11);
    expect(w.accounts[7].address).toBe(v1.tokenHooksPda.policy(v1.wrapperPda.wrappedMint(u)[0])[0]);
    expect(v1.toWeb3(w).keys[0].isSigner).toBe(true);
  });
});
