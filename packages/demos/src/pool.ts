import { join } from "node:path";
import { ExecutionLab } from "@pump-hooks/execution-lab";
import { v1 } from "@pump-hooks/sdk";
import { publishBuiltin, rejection, Runner, setUpgradeAuthority, setupRegistry, signer, spl, type DemoOptions, type DemoResult } from "./common.js";

const SEED = 1_000_000_000n;

/**
 * Fee actually charged on an exact-in swap, recovered from measured balances: the constant-product
 * input that yields `out` from the pre-swap reserves, subtracted from what was paid.
 */
export function impliedFeeBps(amountIn: bigint, out: bigint, reserveIn: bigint, reserveOut: bigint): number {
  const net = (out * reserveIn + (reserveOut - out) - 1n) / (reserveOut - out);
  return Number(((amountIn - net) * 100_000n) / amountIn) / 10;
}

/**
 * DEMO C, POOL: a hooks-core pool with a bounded dynamic-fee policy. The fee rises with swap size
 * from 30 to 100 bps and never exceeds the bound (or the pool's own max fee).
 */
export async function poolDemo(o: DemoOptions): Promise<DemoResult> {
  const lab = new ExecutionLab();
  const admin = await signer(lab, 1000n, "pool creator");
  const r = new Runner(lab, admin);
  await setupRegistry(r, o.deployDir, admin);
  lab.addProgram({ id: v1.HOOKS_CORE_PROGRAM_ID, path: join(o.deployDir, "hooks_core.so"), name: "hooks-core" });
  setUpgradeAuthority(lab, v1.HOOKS_CORE_PROGRAM_ID, admin.address);
  await r.must([v1.initializeProtocol(admin.address, admin.address)], [admin], "initialize hooks-core");
  const version = await publishBuiltin(r, admin, v1.Builtin.DynamicFee, v1.Environment.Pool);

  const [a, b] = [await spl.mint(r, admin), await spl.mint(r, admin)];
  const [m0, m1] = v1.compareBytes(v1.bytes32(a), v1.bytes32(b)) < 0 ? [a, b] : [b, a];
  const keys = v1.poolKeys(m0, m1);
  lab.label(keys.pool, "pool").label(keys.vault[0], "vault 0").label(keys.vault[1], "vault 1").label(m0, "token 0").label(m1, "token 1");
  await r.must([v1.initializePool({ creator: admin.address, keys, feePpm: 3_000, maxFeePpm: 100_000 })], [admin], "create pool (stored fee 0.30%, max 10%)");
  const stack = v1.encodeStack(v1.Environment.Pool, [
    { builtin: v1.Builtin.DynamicFee, callbacks: [v1.PoolCallback.BeforeSwap], hookVersion: v1.bytes32(version), config: v1.configs.dynamicFee({ baseBps: 30, maxBps: 100, pivot: 100_000_000n }) },
  ]);
  await r.must([v1.attachPoolPolicy({ creator: admin.address, pool: keys.pool, stack, versions: [version] })], [admin], "attach policy: dynamic fee 30 → 100 bps, pivot 100 tokens");

  const lp = [await spl.account(r, m0, admin.address, "lp token 0"), await spl.account(r, m1, admin.address, "lp token 1"), await spl.account(r, keys.lpMint, admin.address, "lp shares")];
  await r.must([spl.mintTo(m0, lp[0], admin.address, SEED), spl.mintTo(m1, lp[1], admin.address, SEED)], [], "mint liquidity");
  await r.must([v1.addLiquidity({ owner: admin.address, keys, owner0: lp[0], owner1: lp[1], ownerLp: lp[2], max0: SEED, max1: SEED, minShares: 0n, hasPolicy: true })], [], "seed the pool with 1,000 of each token");

  const trader = await signer(lab, 10n, "trader");
  const t = [await spl.account(r, m0, trader.address, "trader token 0"), await spl.account(r, m1, trader.address, "trader token 1")];
  await r.must([spl.mintTo(m0, t[0], admin.address, SEED), spl.mintTo(m1, t[1], admin.address, SEED)], [], "fund the trader");

  const bal = (x: string) => v1.tokenAmount(lab.getAccount(x)!.data);
  const sell0 = async (title: string, amount: bigint) => {
    const [r0, r1, out0] = [bal(keys.vault[0]), bal(keys.vault[1]), bal(t[1])];
    await r.step(title, "ok", [v1.swap({ trader: trader.address, keys, trader0: t[0], trader1: t[1], zeroForOne: true, exactIn: true, amount, limit: 1n, hasPolicy: true })], [trader], () => {
      const out = bal(t[1]) - out0;
      return `received ${out} of token 1; fee charged ${impliedFeeBps(amount, out, r0, r1)} bps (recovered from measured reserves)`;
    });
  };
  await sell0("Swap 1 token 0 → token 1 (small)", 1_000_000n);
  await sell0("Swap 100 token 0 (at the pivot)", 100_000_000n);
  await sell0("Swap 300 token 0 (past the pivot)", 300_000_000n);

  const [r0, r1] = [bal(keys.vault[0]), bal(keys.vault[1])];
  const feeless = (1_000_000n * r1) / (r0 + 1_000_000n);
  await r.step(
    "Swap 1 token 0 demanding the fee-free output",
    "rejected",
    [v1.swap({ trader: trader.address, keys, trader0: t[0], trader1: t[1], zeroForOne: true, exactIn: true, amount: 1_000_000n, limit: feeless, hasPolicy: true })],
    [trader],
    (tr) => `${rejection(tr)}: the policy's fee is charged, so ${feeless} out is unreachable`,
  );
  const withoutPolicy = v1.swap({ trader: trader.address, keys, trader0: t[0], trader1: t[1], zeroForOne: true, exactIn: true, amount: 1_000_000n, limit: 1n, hasPolicy: false });
  await r.step("Swap without passing the pool's policy account", "rejected", [withoutPolicy], [trader], (tr) => `${rejection(tr)}: a pool with a policy cannot be traded around it`);

  return {
    id: "pool",
    title: "Pool hook: bounded dynamic fee",
    enforcement: "Program: every swap of this hooks-core pool runs its stack. It governs only pools in this program, not PumpSwap or other AMMs.",
    setup: r.setup,
    steps: r.steps,
  };
}
