// After `solana program deploy`: initialize pump-hooks and register every available first-party module.
// Idempotent: finished steps are skipped, so it can be re-run after a partial failure.
//
//   pnpm --filter @pump-hooks/cli exec tsx scripts/bootstrap-pump-hooks.ts --url <rpc> --keypair <upgrade-authority.json> [--dry-run]
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  ComputeBudgetProgram,
  Connection,
  Keypair,
  PublicKey,
  sendAndConfirmTransaction,
  Transaction,
  type TransactionInstruction,
} from "@solana/web3.js";
import { decodeGlobalConfig, hooksPda, initializeIx, MODULES, PUMP_HOOKS_PROGRAM_ID, registerHookIx } from "@pump-hooks/sdk";

const ROOT = join(import.meta.dirname, "../../..");
const BPF_UPGRADEABLE = new PublicKey("BPFLoaderUpgradeab1e11111111111111111111111");
const REGISTERS_PER_TX = 4;

const arg = (name: string) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : undefined;
};
const url = arg("url");
const keypairPath = arg("keypair");
const dryRun = process.argv.includes("--dry-run");
if (!url || !keypairPath) {
  console.error("usage: bootstrap-pump-hooks.ts --url <rpc> --keypair <upgrade-authority.json> [--dry-run]");
  process.exit(2);
}

/** The registered source hash is sha256 of the module's source at this commit, so anyone can check it. */
function sourceFile(slug: string) {
  return slug === "policy" ? "crates/policy-vm/src/lib.rs" : `crates/hook-runtime/src/modules/${slug.replaceAll("-", "_")}.rs`;
}
const sourceHash = (slug: string) => new Uint8Array(createHash("sha256").update(readFileSync(join(ROOT, sourceFile(slug)))).digest());

const fail = (msg: string): never => {
  console.error(`ABORT: ${msg}`);
  process.exit(1);
};

const conn = new Connection(url, "confirmed");
const admin = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(keypairPath, "utf8"))));
const me = admin.publicKey;

async function send(label: string, ixs: TransactionInstruction[]) {
  if (dryRun) return console.log(`[dry-run] ${label}`);
  const tx = new Transaction().add(ComputeBudgetProgram.setComputeUnitPrice({ microLamports: 200_000 }), ...ixs);
  const sig = await sendAndConfirmTransaction(conn, tx, [admin], { commitment: "confirmed" });
  console.log(`${label}: ${sig}`);
}

const programData = PublicKey.findProgramAddressSync([PUMP_HOOKS_PROGRAM_ID.toBuffer()], BPF_UPGRADEABLE)[0];
const [prog, pd, global] = await conn.getMultipleAccountsInfo([PUMP_HOOKS_PROGRAM_ID, programData, hooksPda.global()]);
if (!prog?.executable || !prog.owner.equals(BPF_UPGRADEABLE)) fail(`${PUMP_HOOKS_PROGRAM_ID.toBase58()} is not deployed (upgradeable) on ${url}`);
const authority = pd && pd.data[12] === 1 ? new PublicKey(pd.data.subarray(13, 45)) : undefined;
if (!authority?.equals(me)) fail(`upgrade authority is ${authority?.toBase58() ?? "none"}, not ${me.toBase58()}`);
console.log(`program ${PUMP_HOOKS_PROGRAM_ID.toBase58()} · upgrade authority ${me.toBase58()} · balance ${(await conn.getBalance(me)) / 1e9} SOL`);

if (global) {
  const g = decodeGlobalConfig(global.data);
  if (!g.authority.equals(me)) fail(`already initialized by ${g.authority.toBase58()}`);
  console.log("initialize: already done");
} else {
  await send("initialize", [initializeIx({ authority: me })]);
}

const modules = MODULES.filter((m) => m.available);
const defs = await conn.getMultipleAccountsInfo(modules.map((m) => hooksPda.hookDefinition(m.id)));
const todo = modules.filter((_, i) => !defs[i]);
for (const m of modules.filter((_, i) => defs[i])) console.log(`register ${m.slug}: already done`);
for (let i = 0; i < todo.length; i += REGISTERS_PER_TX) {
  const batch = todo.slice(i, i + REGISTERS_PER_TX);
  await send(
    `register ${batch.map((m) => m.slug).join(", ")}`,
    batch.map((m) => registerHookIx({ authority: me, moduleId: m.id, sourceHash: sourceHash(m.slug) })),
  );
}

if (!dryRun) {
  const g = decodeGlobalConfig((await conn.getAccountInfo(hooksPda.global()))!.data);
  const now = await conn.getMultipleAccountsInfo(modules.map((m) => hooksPda.hookDefinition(m.id)));
  const missing = modules.filter((_, i) => !now[i]).map((m) => m.slug);
  console.log(`done · authority ${g.authority.toBase58()} · paused ${g.paused} · registered ${modules.length - missing.length}/${modules.length}`);
  if (missing.length) fail(`not registered: ${missing.join(", ")}`);
}
