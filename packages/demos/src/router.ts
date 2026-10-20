import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { v1 } from "@pump-hooks/sdk";
import { fmtSol, publishBuiltin, rejection, Runner, setUpgradeAuthority, setupRegistry, signer, SOL, type DemoOptions, type DemoResult } from "./common.js";
import { ExecutionLab } from "@pump-hooks/execution-lab";

export const PUMPSWAP_POOL = "9Nhddo1PcCBms54wcLYbxoRcAJbpmSvHWMcbdMf9tdmZ";
const PUMP_PROGRAMS = [
  ["pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA", "pump_amm", "pumpswap"],
  ["6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P", "pump", "pump"],
  ["pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ", "pump_fees", "pump-fees"],
] as const;
const ATA_PROGRAM = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";

export function routerFixturesAvailable(fixturesDir: string): boolean {
  return PUMP_PROGRAMS.every(([, f]) => existsSync(join(fixturesDir, "programs", `${f}.so`))) && existsSync(join(fixturesDir, PUMPSWAP_POOL, "manifest.json"));
}

const ata = (owner: string, program: string, mint: string) => v1.findPda([v1.bytes32(owner), v1.bytes32(program), v1.bytes32(mint)], ATA_PROGRAM)[0];

/**
 * DEMO B, TRANSACTION (router): a routed PumpSwap buy under a route policy that caps the trade
 * size. Runs the real PumpSwap binary on a pool cloned from devnet. Router hooks are bypassable:
 * the last step calls PumpSwap directly and the cap does not apply.
 */
export async function routerDemo(o: DemoOptions): Promise<DemoResult> {
  const fx = o.fixturesDir;
  if (!fx || !routerFixturesAvailable(fx)) throw new Error("router demo needs tests/fixtures (scripts/dump_programs.sh and the cloned PumpSwap pool)");
  const lab = new ExecutionLab();
  const admin = await signer(lab, 1000n, "admin");
  const r = new Runner(lab, admin);

  await setupRegistry(r, o.deployDir, admin);
  lab.addProgram({ id: v1.ROUTER_PROGRAM_ID, path: join(o.deployDir, "hook_router.so"), name: "hook-router" });
  setUpgradeAuthority(lab, v1.ROUTER_PROGRAM_ID, admin.address);
  for (const [id, f, name] of PUMP_PROGRAMS) lab.addProgram({ id, path: join(fx, "programs", `${f}.so`), name });
  const manifest = JSON.parse(readFileSync(join(fx, PUMPSWAP_POOL, "manifest.json"), "utf8"));
  for (const [label, key] of Object.entries(manifest.accounts as Record<string, string>)) {
    if (!(label === "quote_mint" && lab.getAccount(key))) lab.loadAccountJson(join(fx, PUMPSWAP_POOL, `${key}.json`));
    lab.label(key, label.replace(/_/g, " "));
  }
  r.setup.push(`load PumpSwap (devnet binaries) and the cloned devnet pool ${PUMPSWAP_POOL}`);
  const mint: string = manifest.mint;
  const baseProgram = lab.getAccount(mint)!.owner;

  await r.must([v1.initializeRouter(admin.address), v1.setVenue(admin.address, v1.Venue.PumpSwap, true)], [admin], "initialize hook-router; enable the PumpSwap venue");
  const version = await publishBuiltin(r, admin, v1.Builtin.MaxSize, v1.Environment.Router);
  const project = await signer(lab, 10n, "policy authority");
  const stack = v1.encodeStack(v1.Environment.Router, [
    { builtin: v1.Builtin.MaxSize, callbacks: [v1.RouterCallback.BeforeSwap], mandatory: true, hookVersion: v1.bytes32(version), config: v1.configs.maxSize(v1.bytes32(v1.NATIVE_MINT), SOL) },
  ]);
  const policy = v1.routerPda.policy(project.address, 1n);
  lab.label(policy, "route policy");
  await r.must([v1.createRoutePolicy({ authority: project.address, seed: 1n, shared: true, stack, versions: [version] })], [project], "create a shared route policy: max-size 1 SOL on before_swap (mandatory)");

  const user = await signer(lab, 10n, "trader");
  const [base, quote] = [ata(user.address, baseProgram, mint), ata(user.address, v1.SPL_TOKEN_PROGRAM_ID, v1.NATIVE_MINT)];
  lab.label(base, "trader base").label(quote, "trader wSOL");
  const createAta = (m: string, program: string): v1.InstructionV1 => ({
    programId: ATA_PROGRAM,
    accounts: [v1.meta(user.address, true, true), v1.meta(ata(user.address, program, m), true), v1.meta(user.address), v1.meta(m), v1.meta(v1.SYSTEM_PROGRAM_ID), v1.meta(program)],
    data: v1.u8(1),
  });
  const wrapSol: v1.InstructionV1[] = [
    { programId: v1.SYSTEM_PROGRAM_ID, accounts: [v1.meta(user.address, true, true), v1.meta(quote, true)], data: v1.concat(v1.u32(2), v1.u64(5n * SOL)) },
    { programId: v1.SPL_TOKEN_PROGRAM_ID, accounts: [v1.meta(quote, true)], data: v1.u8(17) },
  ];
  await r.must([createAta(v1.NATIVE_MINT, v1.SPL_TOKEN_PROGRAM_ID), createAta(mint, baseProgram), ...wrapSol], [user], "trader: token accounts, 5 SOL wrapped");

  const venueAccounts = v1.pumpSwap.accounts({
    pool: manifest.pool,
    user: user.address,
    baseMint: mint,
    poolBaseTokenAccount: manifest.accounts.pool_base_token_account,
    poolQuoteTokenAccount: manifest.accounts.pool_quote_token_account,
    baseTokenProgram: baseProgram,
    buybackRecipient: manifest.buyback_fee_recipient,
  });
  let nonce = 0n;
  const buy = (quoteIn: bigint) => {
    nonce += 1n;
    return {
      ix: v1.route({
        user: user.address,
        policy,
        userInput: quote,
        userOutput: base,
        venueAccounts,
        venueProgram: v1.PUMP_AMM_PROGRAM_ID,
        venueData: v1.pumpSwap.data("buy-exact-quote-in", quoteIn, 1n),
        maxIn: quoteIn,
        minOut: 1n,
        maxLamports: SOL / 100n,
        nonce,
      }),
      receipt: v1.routerPda.receipt(user.address, nonce),
    };
  };
  const tokens = (a: string) => v1.tokenAmount(lab.getAccount(a)!.data);

  const small = buy(SOL / 2n);
  await r.step("Routed buy for 0.5 SOL (cap is 1 SOL)", "ok", [small.ix], [user], () => {
    const rc = v1.decodeReceipt(lab.getAccount(small.receipt)!.data);
    return `receipt: spent ${fmtSol(rc.spent)}, received ${rc.received} base (measured by the router, venue verified: ${rc.venueVerified})`;
  });
  const big = buy(2n * SOL);
  const before = tokens(base);
  await r.step("Routed buy for 2 SOL", "rejected", [big.ix], [user], (t) => `${rejection(t)}; trader base balance unchanged (${tokens(base) === before})`);
  const direct: v1.InstructionV1 = { programId: v1.PUMP_AMM_PROGRAM_ID, accounts: venueAccounts, data: v1.pumpSwap.data("buy-exact-quote-in", 2n * SOL, 1n) };
  await r.step("Same 2 SOL buy sent to PumpSwap directly", "ok", [direct], [user], () => `executed: router hooks do not apply outside the router (received ${tokens(base) - before} base)`);

  return {
    id: "router",
    title: "Transaction hook: maximum trade size on routed PumpSwap buys",
    enforcement: "Router: applies only to trades sent through hook-router. PumpSwap has no native hooks and stays directly callable, as the last step shows.",
    setup: r.setup,
    steps: r.steps,
  };
}
