import { join } from "node:path";
import { generateKeyPairSigner } from "@solana/kit";
import { ExecutionLab } from "@pump-hooks/execution-lab";
import { v1 } from "@pump-hooks/sdk";
import { fmtSol, publishBuiltin, rejection, Runner, setupRegistry, signer, SOL, type DemoOptions, type DemoResult } from "./common.js";

const HOUR = 3_600n;
const DAY = 86_400n;

/**
 * DEMO D, ACCOUNT: a smart account whose withdrawals need a proposal at least one hour old and
 * stay under 2 SOL per day. Funds sit in the account's vault PDA; only smart-accounts signs for it.
 */
export async function accountDemo(o: DemoOptions): Promise<DemoResult> {
  const lab = new ExecutionLab();
  const T0 = BigInt(Math.floor(Date.now() / 1000));
  lab.setClock({ unixTimestamp: T0 });
  const owner = await signer(lab, 100n, "owner");
  const r = new Runner(lab, owner);
  await setupRegistry(r, o.deployDir, owner);
  lab.addProgram({ id: v1.SMART_ACCOUNTS_PROGRAM_ID, path: join(o.deployDir, "smart_accounts.so"), name: "smart-accounts" });
  const limitV = await publishBuiltin(r, owner, v1.Builtin.SpendingLimit, v1.Environment.Account);
  const lockV = await publishBuiltin(r, owner, v1.Builtin.Timelock, v1.Environment.Account);

  const stack = v1.encodeStack(v1.Environment.Account, [
    { builtin: v1.Builtin.SpendingLimit, callbacks: [v1.AccountCallback.BeforeWithdraw], mandatory: true, hookVersion: v1.bytes32(limitV), config: v1.configs.spendingLimit(new Uint8Array(32), 2n * SOL, Number(DAY)) },
    { builtin: v1.Builtin.Timelock, callbacks: [v1.AccountCallback.BeforeWithdraw], mandatory: true, hookVersion: v1.bytes32(lockV), config: v1.configs.timelock(Number(HOUR), 0n) },
  ]);
  const account = v1.smartAccountPda.account(owner.address, 1n);
  const vault = v1.smartAccountPda.vault(account);
  lab.label(account, "smart account").label(vault, "vault");
  await r.must([v1.createSmartAccount({ payer: owner.address, seed: 1n, owner: owner.address, policyDelaySecs: DAY, stack, versions: [limitV, lockV] })], [owner], "create smart account: spending limit 2 SOL/day + 1 h timelock on withdrawals (both mandatory, 1 day to change)");
  await r.must([v1.depositSol({ depositor: owner.address, account, amount: 5n * SOL })], [owner], "deposit 5 SOL");

  const dest = (await generateKeyPairSigner()).address;
  lab.label(dest, "recipient");
  const lamports = (a: string) => lab.getAccount(a)?.lamports ?? 0n;
  let proposals = 0n;
  const proposeWithdraw = async (amount: bigint) => {
    const index = proposals++;
    const actionHash = v1.withdrawActionHash({ amount, destination: dest });
    await r.step(`Propose a withdrawal of ${fmtSol(amount)}`, "ok", [v1.propose({ proposer: owner.address, account, index, actionHash })], [owner], () => "proposal recorded; the timelock counts from now");
    return v1.smartAccountPda.proposal(account, index);
  };
  const withdraw = (amount: bigint, proposal?: string) => v1.withdrawSol({ authority: owner.address, account, destination: dest, amount, proposal });

  await r.step("Owner withdraws 1 SOL immediately", "rejected", [withdraw(SOL)], [owner]);
  const p1 = await proposeWithdraw(SOL);
  lab.setClock({ unixTimestamp: T0 + HOUR - 60n });
  await r.step("Withdraw with the proposal 59 minutes later", "rejected", [withdraw(SOL, p1)], [owner]);
  lab.setClock({ unixTimestamp: T0 + HOUR });
  await r.step("Withdraw 2 SOL with the 1 SOL proposal", "rejected", [withdraw(2n * SOL, p1)], [owner], (t) => `${rejection(t)}: the proposal's hash binds amount and destination`);
  await r.step("Withdraw 1 SOL with the proposal after 1 hour", "ok", [withdraw(SOL, p1)], [owner], () => `recipient holds ${fmtSol(lamports(dest))}; vault ${fmtSol(lamports(vault))}`);
  await r.step("Replay the same proposal", "rejected", [withdraw(SOL, p1)], [owner]);

  const p2 = await proposeWithdraw(3n * SOL / 2n);
  lab.setClock({ unixTimestamp: T0 + 2n * HOUR + 1n });
  await r.step("Withdraw 1.5 SOL (aged proposal; 1 SOL already spent today)", "rejected", [withdraw(3n * SOL / 2n, p2)], [owner], (t) => `${rejection(t)}: 1 + 1.5 > 2 SOL per day`);

  const mallory = await signer(lab, 1n, "stranger");
  await r.step("A stranger tries to withdraw", "rejected", [v1.withdrawSol({ authority: mallory.address, account, destination: mallory.address, amount: 1n })], [mallory]);

  return {
    id: "account",
    title: "Account hook: timelock + daily spending limit",
    enforcement: "Program: every withdrawal from the smart account's vault runs its stack. Mandatory entries can only be changed through an aged proposal.",
    setup: r.setup,
    steps: r.steps,
  };
}
