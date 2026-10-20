import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  appendTransactionMessageInstructions,
  compileTransaction,
  createTransactionMessage,
  getTransactionEncoder,
  pipe,
  setTransactionMessageFeePayer,
  setTransactionMessageLifetimeUsingBlockhash,
  address,
  blockhash,
  type Address,
} from "@solana/kit";
import { describe, expect, it } from "vitest";
import { v1 } from "@pump-hooks/sdk";
import { applyWrapperOp, setupWrapper, snapshotWrapper, toKit } from "@pump-hooks/hook-security";
import { CompileError, compile, MAX_TX_SIZE, transactionSize, withMeasuredCompute, type Operation } from "../src/index.js";

const DEPLOY = fileURLToPath(new URL("../../../target/deploy", import.meta.url));
const SBF = process.env.PUMPHOOKS_SBF === "1" || existsSync(resolve(DEPLOY, "token_wrapper.so"));
const key = () => v1.b58(crypto.getRandomValues(new Uint8Array(32)));
const none = () => null;

function kitSize(feePayer: string, ixs: v1.InstructionV1[]): number {
  const msg = pipe(
    createTransactionMessage({ version: 0 }),
    (m) => setTransactionMessageFeePayer(address(feePayer), m),
    (m) => setTransactionMessageLifetimeUsingBlockhash({ blockhash: blockhash("11111111111111111111111111111111"), lastValidBlockHeight: 0n }, m),
    (m) => appendTransactionMessageInstructions(ixs.map(toKit), m),
  );
  return getTransactionEncoder().encode(compileTransaction(msg)).length;
}

const raw = (id: string, accounts: v1.InstructionV1["accounts"], data = new Uint8Array(8), after?: string[]): Operation => ({
  kind: "instruction",
  id,
  ix: { programId: v1.TOKEN_HOOKS_PROGRAM_ID, accounts, data },
  cu: 10_000,
  after,
});

describe("transaction compiler", () => {
  it("orders by dependency, merges metas and computes the exact size", async () => {
    const [payer, a, b, c] = [key(), key(), key(), key()];
    const tx = await compile(
      {
        feePayer: payer,
        signers: [a],
        operations: [raw("second", [v1.meta(a, false, true), v1.meta(b, true)], undefined, ["first"]), raw("first", [v1.meta(b), v1.meta(c), v1.meta(a, true)])],
      },
      none,
    );
    expect(tx.order).toEqual(["first", "second"]);
    const A = tx.accounts.find((x) => x.address === a)!;
    expect([A.signer, A.writable]).toEqual([true, true]);
    expect(tx.requiredSigners.sort()).toEqual([payer, a].sort());
    expect(tx.size).toBe(kitSize(payer, tx.instructions));
    expect(tx.computeUnitLimit).toBe(Math.ceil(20_000 * 1.1) + 300);
    expect(tx.warnings.join()).toContain(b);
  });

  it("refuses cycles, unknown dependencies and missing signers", async () => {
    const p = key();
    const err = async (ops: Operation[], signers: string[] = []) => {
      try {
        await compile({ feePayer: p, signers, operations: ops }, none);
      } catch (e) {
        return (e as CompileError).code;
      }
      return "ok";
    };
    expect(await err([raw("a", [], undefined, ["b"]), raw("b", [], undefined, ["a"])])).toBe("cycle");
    expect(await err([raw("a", [], undefined, ["zzz"])])).toBe("unknown-dependency");
    expect(await err([raw("a", []), raw("a", [])])).toBe("duplicate-id");
    const s = key();
    expect(await err([raw("a", [v1.meta(s, false, true)])])).toBe("missing-signer");
    expect(await err([raw("a", [v1.meta(s, false, true)])], [s])).toBe("ok");
  });

  it("never splits: oversize plans fail with a lookup-table suggestion, and lookup tables make them fit", async () => {
    const p = key();
    const many = Array.from({ length: 40 }, () => key());
    const ops = [0, 1, 2, 3].map((i) => raw(`op${i}`, many.slice(i * 10, i * 10 + 10).map((k) => v1.meta(k, true))));
    const e = await compile({ feePayer: p, signers: [], operations: ops }, none).catch((x) => x as CompileError);
    expect(e).toBeInstanceOf(CompileError);
    expect((e as CompileError).code).toBe("too-large");
    expect((e as CompileError).message).toMatch(/not split automatically/);
    const tx = await compile({ feePayer: p, signers: [], operations: ops, lookupTables: [{ address: key(), addresses: many }] }, none);
    expect(tx.size).toBeLessThanOrEqual(MAX_TX_SIZE);
    expect(tx.lookupTablesUsed).toHaveLength(1);
    expect(tx.accounts.filter((a) => a.lookupTable)).toHaveLength(40);
    expect(transactionSize(tx.accounts, tx.instructions)).toBe(tx.size);
  });
});

describe.skipIf(!SBF)("compiled against token-wrapper + token-hooks", () => {
  it("one atomic transaction: wrap, hooked transfer, unwrap; simulated, then sized to measured compute", async () => {
    const s = await setupWrapper(DEPLOY, 2);
    const [a, b] = s.users;
    const read = (addr: string) => s.lab.getAccount(addr)?.data ?? null;
    const common = { underlyingMint: s.underlyingMint, underlyingProgram: v1.SPL_TOKEN_PROGRAM_ID };
    const tx = await compile(
      {
        feePayer: s.payer.address,
        signers: [a.signer.address, b.signer.address],
        operations: [
          { kind: "unwrap", id: "b-unwraps", user: b.signer.address, ...common, userUnderlying: b.underlying, userWrapped: b.wrapped, amount: 300n, minOut: 300n, after: ["send"] },
          { kind: "wrap", id: "a-wraps", user: a.signer.address, ...common, userUnderlying: a.underlying, userWrapped: a.wrapped, amount: 1_000n, minOut: 1_000n },
          { kind: "hooked-transfer", id: "send", source: a.wrapped, destination: b.wrapped, sourceOwner: a.signer.address, destinationOwner: b.signer.address, mint: s.wrappedMint, amount: 400n, decimals: 6, after: ["a-wraps"] },
        ],
      },
      read,
    );
    expect(tx.order).toEqual(["a-wraps", "send", "b-unwraps"]);
    expect(tx.size).toBe(kitSize(s.payer.address, tx.instructions));

    const run = async (ixs: v1.InstructionV1[]) => s.lab.execute({ payer: s.payer, signers: [a.signer, b.signer], instructions: ixs.map(toKit) });
    const before = snapshotWrapper(s);
    const t = await run(tx.instructions);
    expect(t.status, t.logs.join("\n")).toBe("success");
    const after = snapshotWrapper(s);
    expect(after.wrapped).toEqual([600n, 100n]);
    expect(after.vault).toBe(before.vault + 700n);
    expect(Number(t.computeUnits.total)).toBeLessThan(tx.computeUnitLimit);

    const measured = withMeasuredCompute(tx, t.computeUnits.total);
    expect(measured.computeProvenance).toBe("simulated");
    expect(measured.computeUnitLimit).toBeLessThan(tx.computeUnitLimit);
    const again = await run(measured.instructions);
    expect(again.status).toBe("success");
  });

  it("atomic failure: a failing step reverts the steps before it", async () => {
    const s = await setupWrapper(DEPLOY, 2);
    const [a, b] = s.users;
    await applyWrapperOp(s, { kind: "wrap", user: 0, amount: 50n, minOut: 50n, expectOk: true });
    const read = (addr: string) => s.lab.getAccount(addr)?.data ?? null;
    const tx = await compile(
      {
        feePayer: s.payer.address,
        signers: [a.signer.address],
        operations: [
          { kind: "wrap", id: "wrap", user: a.signer.address, underlyingMint: s.underlyingMint, underlyingProgram: v1.SPL_TOKEN_PROGRAM_ID, userUnderlying: a.underlying, userWrapped: a.wrapped, amount: 10n, minOut: 10n },
          { kind: "hooked-transfer", id: "overspend", source: a.wrapped, destination: b.wrapped, sourceOwner: a.signer.address, destinationOwner: b.signer.address, mint: s.wrappedMint, amount: 10_000n, decimals: 6, after: ["wrap"] },
        ],
      },
      read,
    );
    const before = snapshotWrapper(s);
    const t = await s.lab.execute({ payer: s.payer, signers: [a.signer], instructions: tx.instructions.map(toKit) });
    expect(t.status).toBe("failed");
    expect(snapshotWrapper(s).accounts).toEqual(before.accounts);
  });

  it("explains an unhooked mint instead of building a transfer that cannot work", async () => {
    const s = await setupWrapper(DEPLOY, 2);
    const e = await compile(
      { feePayer: s.payer.address, signers: [], operations: [{ kind: "hooked-transfer", id: "t", source: key(), destination: key(), sourceOwner: s.payer.address, destinationOwner: key(), mint: s.underlyingMint as Address, amount: 1n, decimals: 6 }] },
      (addr) => s.lab.getAccount(addr)?.data ?? null,
    ).catch((x) => x as CompileError);
    expect((e as CompileError).code).toBe("unresolved");
  });
});
