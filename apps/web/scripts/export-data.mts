// Regenerates apps/web/data/*.json from real runs of the compiled programs in LiteSVM.
//   scripts/build_programs.sh && pnpm -r --filter "./packages/**" build && pnpm --filter @pump-hooks/web export-data
// Every artifact records the binaries' sha256 and the commit it was produced from.

import { execSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { generateKeyPairSigner } from "@solana/kit";
import { v1 } from "@pump-hooks/sdk";
import { ExecutionLab, toJson, type ExecutionTrace } from "@pump-hooks/execution-lab";
import { ATTACKS, CATEGORIES, DISCLAIMER, WRAPPER_INVARIANTS, applyWrapperOp, fuzz, setupWrapper, toKit, wrapperTarget } from "@pump-hooks/hook-security";
import { CompileError, compile, withMeasuredCompute, type CompiledTransaction, type Operation } from "@pump-hooks/transaction-compiler";

const ROOT = resolve(import.meta.dirname, "../../..");
const DEPLOY = join(ROOT, "target", "deploy");
const OUT = resolve(import.meta.dirname, "../data");
const SO = ["token_hooks.so", "token_wrapper.so", "anti_jit_hook.so"];

const sha = (b: Uint8Array) => createHash("sha256").update(b).digest("hex");
const programs = Object.fromEntries(SO.map((f) => [f, sha(readFileSync(join(DEPLOY, f)))]));
const meta = (command: string) => ({
  commit: execSync("git rev-parse HEAD", { cwd: ROOT }).toString().trim(),
  generatedAt: new Date().toISOString(),
  runtime: "LiteSVM (node, litesvm 1.6.1) with the compiled .so files",
  programs,
  command,
});

function slim(t: ExecutionTrace, note?: string) {
  const { replay: _replay, ...rest } = t;
  return { ...rest, note };
}

async function lab() {
  const traces = [];
  const s = await setupWrapper(DEPLOY, 2);
  s.lab.label(s.payer.address, "fee payer");
  s.lab.label(s.users[0].signer.address, "alice").label(s.users[1].signer.address, "bob");
  s.lab.label(s.users[0].wrapped, "alice wrapped").label(s.users[1].wrapped, "bob wrapped");
  s.lab.label(s.users[0].underlying, "alice underlying").label(s.vault, "vault").label(s.policy, "token-policy");

  await applyWrapperOp(s, { kind: "wrap", user: 0, amount: 1_000n, minOut: 1_000n, expectOk: true });
  traces.push(slim(s.lastTrace!, "Wrap 1000 underlying SPL tokens into the hooked Token-2022 mint: measured deposit, then mint."));
  await applyWrapperOp(s, { kind: "transfer", from: 0, to: 1, amount: 250n, expectOk: true });
  traces.push(slim(s.lastTrace!, "Wrapped transfer: Token-2022 calls token-hooks Execute mid-transfer (empty policy stack)."));
  await applyWrapperOp(s, { kind: "attack", attack: "transfer-no-hook", user: 0, victim: 1, amount: 10n });
  traces.push(slim(s.lastTrace!, `Attack "${ATTACKS["transfer-no-hook"]}": refused, nothing moves.`));
  await applyWrapperOp(s, { kind: "unwrap", user: 1, amount: 250n, minOut: 250n, expectOk: true });
  traces.push(slim(s.lastTrace!, "Unwrap: burn first, then release from the vault."));

  // Program hook: one policy call to the example anti-JIT hook, as the router would send it.
  const hl = new ExecutionLab({ sigverify: false });
  const hook = (await generateKeyPairSigner()).address;
  const payer = (await generateKeyPairSigner()).address;
  const subject = (await generateKeyPairSigner()).address;
  hl.addProgram({ id: hook, path: join(DEPLOY, "anti_jit_hook.so"), name: "anti-jit-hook" });
  hl.airdrop(payer, 1_000_000_000n);
  hl.setAccount({ address: subject, lamports: 1_000_000_000n, owner: v1.ENVIRONMENT_PROGRAMS[1], executable: false, data: new Uint8Array(64) });
  hl.label(subject, "route policy (owned by hook-router)");
  const ix = v1.policyCall({ hook, context: { environment: 1, callback: 2, subject, actor: payer, amount: 5n, slot: 1000n } });
  const t = await hl.execute({ payer: payer as never, label: "router before_swap → anti-jit-hook", instructions: [toKit(ix)] });
  traces.push(
    slim(
      t,
      "Policy call to the example program hook without its config account: it answers reject 1002 (not configured) through return data. Signature verification is off to emulate the router's invoke_signed for the callback-authority PDA.",
    ),
  );
  return { ...meta("pnpm --filter @pump-hooks/web export-data"), traces };
}

function summarize(tx: CompiledTransaction) {
  return {
    order: tx.order,
    size: tx.size,
    computeUnitLimit: tx.computeUnitLimit,
    computeProvenance: tx.computeProvenance,
    requiredSigners: tx.requiredSigners,
    warnings: tx.warnings,
    lookupTablesUsed: tx.lookupTablesUsed,
    accounts: tx.accounts,
    instructions: tx.instructions.map((i) => ({ programId: i.programId, accounts: i.accounts.length, dataLen: i.data.length })),
  };
}

async function compiler() {
  const s = await setupWrapper(DEPLOY, 2);
  const [a, b] = s.users;
  s.lab.label(s.payer.address, "fee payer");
  await applyWrapperOp(s, { kind: "wrap", user: 1, amount: 500n, minOut: 500n, expectOk: true });
  const read = (addr: string) => s.lab.getAccount(addr)?.data ?? null;
  const common = { underlyingMint: s.underlyingMint, underlyingProgram: v1.SPL_TOKEN_PROGRAM_ID };
  const plan: Operation[] = [
    { kind: "unwrap", id: "bob-unwraps", user: b.signer.address, ...common, userUnderlying: b.underlying, userWrapped: b.wrapped, amount: 300n, minOut: 300n, after: ["send"] },
    { kind: "wrap", id: "alice-wraps", user: a.signer.address, ...common, userUnderlying: a.underlying, userWrapped: a.wrapped, amount: 1_000n, minOut: 1_000n },
    { kind: "hooked-transfer", id: "send", source: a.wrapped, destination: b.wrapped, sourceOwner: a.signer.address, destinationOwner: b.signer.address, mint: s.wrappedMint, amount: 400n, decimals: 6, after: ["alice-wraps"] },
  ];
  const tx = await compile({ feePayer: s.payer.address, signers: [a.signer.address, b.signer.address], operations: plan }, read);
  const run = await s.lab.execute({ payer: s.payer, signers: [a.signer, b.signer], label: "compiled plan", instructions: tx.instructions.map(toKit) });
  if (run.status !== "success") throw new Error(run.logs.join("\n"));
  const measured = withMeasuredCompute(tx, run.computeUnits.total);

  const failing: Operation[] = [
    { kind: "wrap", id: "wrap", user: a.signer.address, ...common, userUnderlying: a.underlying, userWrapped: a.wrapped, amount: 10n, minOut: 10n },
    { kind: "hooked-transfer", id: "overspend", source: a.wrapped, destination: b.wrapped, sourceOwner: a.signer.address, destinationOwner: b.signer.address, mint: s.wrappedMint, amount: 1_000_000n, decimals: 6, after: ["wrap"] },
  ];
  const ftx = await compile({ feePayer: s.payer.address, signers: [a.signer.address], operations: failing }, read);
  const frun = await s.lab.execute({ payer: s.payer, signers: [a.signer], label: "atomic failure", instructions: ftx.instructions.map(toKit) });

  const key = async () => (await generateKeyPairSigner()).address;
  const p = await key();
  const raw = (id: string, accounts: v1.InstructionV1["accounts"], after?: string[]): Operation => ({ kind: "instruction", id, ix: { programId: v1.TOKEN_HOOKS_PROGRAM_ID, accounts, data: new Uint8Array(8) }, cu: 10_000, after });
  const errors: { case: string; code: string; message: string }[] = [];
  const tryCompile = async (name: string, ops: Operation[], signers: string[] = []) => {
    try {
      await compile({ feePayer: p, signers, operations: ops }, () => null);
    } catch (e) {
      errors.push({ case: name, code: (e as CompileError).code, message: (e as Error).message });
    }
  };
  await tryCompile("dependency cycle", [raw("a", [], ["b"]), raw("b", [], ["a"])]);
  await tryCompile("signer not provided", [raw("a", [v1.meta(await key(), false, true)])]);
  const many = await Promise.all(Array.from({ length: 40 }, key));
  await tryCompile("40 writable accounts, no lookup table", [0, 1, 2, 3].map((i) => raw(`op${i}`, many.slice(i * 10, i * 10 + 10).map((k) => v1.meta(k, true)))));

  return {
    ...meta("pnpm --filter @pump-hooks/web export-data"),
    plan: plan.map((o) => ({ id: o.id, kind: o.kind, after: o.after ?? [] })),
    compiled: summarize(tx),
    executed: { status: run.status, computeUnits: run.computeUnits.total, measuredLimit: measured.computeUnitLimit },
    atomicFailure: {
      plan: failing.map((o) => ({ id: o.id, kind: o.kind, after: o.after ?? [] })),
      status: frun.status,
      error: frun.error,
      accountsChanged: frun.accountDiffs.filter((d) => d.changedRanges.length || d.lamportsBefore !== d.lamportsAfter).map((d) => d.label ?? d.address),
    },
    errors,
  };
}

async function security() {
  const runs = 8;
  const report = await fuzz(wrapperTarget(DEPLOY), { seed: 1, runs, steps: 40 });
  return {
    ...meta("pnpm --filter @pump-hooks/web export-data (fuzz seed 1, 8 runs × 40 steps)"),
    disclaimer: DISCLAIMER,
    fuzz: report,
    invariants: WRAPPER_INVARIANTS.map((i) => i.name),
    attacks: ATTACKS,
    categories: CATEGORIES,
  };
}

mkdirSync(OUT, { recursive: true });
for (const [name, f] of [["lab", lab], ["compiler", compiler], ["security", security]] as const) {
  const data = await f();
  writeFileSync(join(OUT, `${name}.json`), toJson(data) + "\n");
  console.log(`wrote data/${name}.json`);
}
