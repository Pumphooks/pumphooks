// Demo B on devnet: max-size route policy on routed PumpSwap buys, against the deployed hook-registry
// and hook-router and the live devnet PumpSwap pool. Writes apps/web/data/devnet-router.json.
//   tsx scripts/devnet-router-demo.mts --keypair <registry + router admin.json> [--url <devnet rpc>]
import { execSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { v1 } from "@pump-hooks/sdk";
import { publishBuiltinIxs } from "../src/common.ts";
import { ATA_PROGRAM, PUMPSWAP_POOL, ata } from "../src/router.ts";
import { accountData, admin, exists, explorer, rpc, send, type Sent } from "./devnet.mts";
import type { Address } from "@solana/kit";

const SOL = 1_000_000_000n;
const CAP = SOL / 20n;
const MINT = "44yV5xk8C1QVRGz82Q8KLcpft5gqbNd3SG8Lt1NrhLo9";
const POOL_BASE = "Drcqe56BudT2DLNRWRpkQuKLP5m6QApBEa4LTwzMAAfg";
const POOL_QUOTE = "Dep73q7XvHGEPgpPU5PPFGLxtyhUyD1xEDSG2Qhq3Be8";
const BUYBACK = "5YxQFdt3Tr9zJLvkFccqXVUwhdTWJQc1fFg2YPbxvxeD";
const ROOT = resolve(import.meta.dirname, "../../..");

const user = admin.address;
const setup: { label: string; signature?: string; note?: string }[] = [];
const steps: { title: string; expect: "ok" | "rejected"; ok: boolean; signature: string; explorer: string; computeUnits: number | null; outcome: string }[] = [];
const sol = (l: bigint) => `${(Number(l) / 1e9).toFixed(3)} SOL`;
const tokens = async (a: string) => {
  const d = await accountData(a);
  return d ? v1.tokenAmount(d) : 0n;
};
const rejection = (s: Sent) => {
  const l = s.logs.find((x) => /hook stack rejected/.test(x)) ?? s.logs.find((x) => /Error Message:|failed:/.test(x)) ?? "failed";
  return l.replace(/^Program log: /, "").replace(/^AnchorError.*Error Message: /, "");
};

const { ixs: publish, version } = publishBuiltinIxs(admin.address, v1.Builtin.MaxSize, v1.Environment.Router);
if (await exists(version)) setup.push({ label: "builtin max-size (router) already published", note: version });
else setup.push({ label: "publish builtin max-size (router) in hook-registry", signature: (await send("publish max-size", publish)).signature });

const seed = 1n;
const policy = v1.routerPda.policy(admin.address, seed);
if (await exists(policy)) setup.push({ label: "route policy already created", note: policy });
else {
  const stack = v1.encodeStack(v1.Environment.Router, [
    { builtin: v1.Builtin.MaxSize, callbacks: [v1.RouterCallback.BeforeSwap], mandatory: true, hookVersion: v1.bytes32(version), config: v1.configs.maxSize(v1.bytes32(v1.NATIVE_MINT), CAP) },
  ]);
  const s = await send("create route policy", [v1.createRoutePolicy({ authority: admin.address, seed, shared: true, stack, versions: [version] })]);
  setup.push({ label: `create a shared route policy: max-size ${sol(CAP)} on before_swap (mandatory)`, signature: s.signature });
}

const mintInfo = (await rpc.getAccountInfo(MINT as Address, { encoding: "base64", dataSlice: { offset: 0, length: 0 } }).send()).value;
if (!mintInfo) throw new Error("devnet mint not found");
const baseProgram = mintInfo.owner as string;
const [base, quote] = [ata(user, baseProgram, MINT), ata(user, v1.SPL_TOKEN_PROGRAM_ID, v1.NATIVE_MINT)];
const createAta = (m: string, program: string): v1.InstructionV1 => ({
  programId: ATA_PROGRAM,
  accounts: [v1.meta(user, true, true), v1.meta(ata(user, program, m), true), v1.meta(user), v1.meta(m), v1.meta(v1.SYSTEM_PROGRAM_ID), v1.meta(program)],
  data: v1.u8(1),
});
const wrap = SOL / 4n;
const s0 = await send("token accounts + wrap SOL", [
  createAta(v1.NATIVE_MINT, v1.SPL_TOKEN_PROGRAM_ID),
  createAta(MINT, baseProgram),
  { programId: v1.SYSTEM_PROGRAM_ID, accounts: [v1.meta(user, true, true), v1.meta(quote, true)], data: v1.concat(v1.u32(2), v1.u64(wrap)) },
  { programId: v1.SPL_TOKEN_PROGRAM_ID, accounts: [v1.meta(quote, true)], data: v1.u8(17) },
]);
setup.push({ label: `trader token accounts (idempotent); wrap ${sol(wrap)}`, signature: s0.signature });

const venueAccounts = v1.pumpSwap.accounts({ pool: PUMPSWAP_POOL, user, baseMint: MINT, poolBaseTokenAccount: POOL_BASE, poolQuoteTokenAccount: POOL_QUOTE, baseTokenProgram: baseProgram, buybackRecipient: BUYBACK });
let nonce = BigInt(Date.now());
const buy = (quoteIn: bigint) => {
  nonce += 1n;
  const ix = v1.route({ user, policy, userInput: quote, userOutput: base, venueAccounts, venueProgram: v1.PUMP_AMM_PROGRAM_ID, venueData: v1.pumpSwap.data("buy-exact-quote-in", quoteIn, 1n), maxIn: quoteIn, minOut: 1n, maxLamports: SOL / 100n, nonce });
  return { ix, receipt: v1.routerPda.receipt(user, nonce) };
};
const step = (title: string, expect: "ok" | "rejected", s: Sent, outcome: string) => {
  steps.push({ title, expect, ok: s.ok === (expect === "ok"), signature: s.signature, explorer: explorer(s.signature), computeUnits: s.computeUnits, outcome });
};

const small = buy(SOL / 50n);
const s1 = await send("routed buy under the cap", [small.ix], { land: true });
const rc = s1.ok ? v1.decodeReceipt((await accountData(small.receipt))!) : null;
step(`Routed buy for ${sol(SOL / 50n)} (cap is ${sol(CAP)})`, "ok", s1, rc ? `receipt: spent ${sol(rc.spent)}, received ${rc.received} base (measured by the router, venue verified: ${rc.venueVerified})` : rejection(s1));

const before = await tokens(base);
const big = buy(SOL / 10n);
const s2 = await send("routed buy over the cap", [big.ix], { land: true });
step(`Routed buy for ${sol(SOL / 10n)}`, "rejected", s2, s2.ok ? "executed (unexpected)" : `${rejection(s2)}; trader base balance unchanged (${(await tokens(base)) === before})`);

const direct: v1.InstructionV1 = { programId: v1.PUMP_AMM_PROGRAM_ID, accounts: venueAccounts, data: v1.pumpSwap.data("buy-exact-quote-in", SOL / 10n, 1n) };
const s3 = await send("same buy sent to PumpSwap directly", [direct], { land: true });
step(`Same ${sol(SOL / 10n)} buy sent to PumpSwap directly`, "ok", s3, s3.ok ? `executed: router hooks do not apply outside the router (received ${(await tokens(base)) - before} base)` : rejection(s3));

const out = {
  cluster: "devnet",
  generatedAt: new Date().toISOString(),
  commit: execSync("git rev-parse HEAD", { cwd: ROOT }).toString().trim(),
  command: "pnpm --filter @pump-hooks/demos exec tsx scripts/devnet-router-demo.mts --keypair <admin>",
  programs: { "hook-registry": v1.REGISTRY_PROGRAM_ID, "hook-router": v1.ROUTER_PROGRAM_ID, pumpswap: v1.PUMP_AMM_PROGRAM_ID },
  pool: PUMPSWAP_POOL,
  policy,
  hookVersion: version,
  trader: user,
  setup,
  steps,
};
writeFileSync(resolve(ROOT, "apps/web/data/devnet-router.json"), JSON.stringify(out, null, 2) + "\n");
for (const s of steps) console.log(`${s.ok ? "✓" : "✗"} ${s.title}: ${s.outcome}`);
process.exit(steps.every((s) => s.ok) ? 0 : 1);
