/**
 * LiteSVM harness for SDK tests: the compiled Pump Hooks program, Pump's real devnet binaries and
 * cloned devnet state of existing Pump tokens (see scripts/fetch_fixtures.sh,
 * scripts/fetch_curve_fixtures.sh, scripts/dump_programs.sh).
 */
import { readdirSync, readFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { AccountInfo, Keypair, PublicKey, SimulatedTransactionResponse, TransactionInstruction, VersionedTransaction } from "@solana/web3.js";
import { Clock, FailedTransactionMetadata, LiteSVM, TransactionMetadata } from "litesvm";
import {
  bundle,
  BuiltTransaction,
  executionResult,
  ExecutionResult,
  initializeIx,
  MODULES,
  PUMP_AMM_PROGRAM_ID,
  PUMP_FEES_PROGRAM_ID,
  PUMP_HOOKS_PROGRAM_ID,
  PUMP_PROGRAM_ID,
  PumpHooksClient,
  registerHookIx,
  RpcLike,
} from "../src/index.js";

export const ROOT = join(__dirname, "../../..");
export const POOL_FIXTURE = "9Nhddo1PcCBms54wcLYbxoRcAJbpmSvHWMcbdMf9tdmZ";
export const DEEP_CURVE_FIXTURE = "curve-AwHTvGo8UcR4CEyqmTWs1xEbeajBNwC5ELFdMdPyB33w";
export const SOL = 1_000_000_000n;

export const fixturesAvailable = () =>
  ["pump", "pump_amm", "pump_fees"].every((p) => existsSync(join(ROOT, `tests/fixtures/programs/${p}.so`))) &&
  existsSync(join(ROOT, "target/deploy/pump_hooks.so"));

function errValue(e: ReturnType<FailedTransactionMetadata["err"]>): unknown {
  const anyE = e as unknown as { index?: number; err?: () => unknown };
  if (typeof anyE.err === "function" && typeof anyE.index === "number") {
    const inner = anyE.err() as { code?: number };
    return { InstructionError: [anyE.index, typeof inner?.code === "number" ? { Custom: inner.code } : String(inner)] };
  }
  return String(e);
}

export class Env {
  svm: LiteSVM;
  admin = Keypair.generate();
  manifest: { mint: string; pool?: string; buyback_fee_recipient: string; accounts: Record<string, string> };
  client: PumpHooksClient;
  rpc: RpcLike;

  constructor(fixture: string) {
    this.svm = new LiteSVM().withPrecompiles().withLogBytesLimit(undefined);
    const now = BigInt(Math.floor(Date.now() / 1000));
    const c = this.svm.getClock();
    this.svm.setClock(new Clock(400_000_000n, c.epochStartTimestamp, c.epoch, c.leaderScheduleEpoch, now));
    this.svm.addProgramFromFile(PUMP_AMM_PROGRAM_ID, join(ROOT, "tests/fixtures/programs/pump_amm.so"));
    this.svm.addProgramFromFile(PUMP_PROGRAM_ID, join(ROOT, "tests/fixtures/programs/pump.so"));
    this.svm.addProgramFromFile(PUMP_FEES_PROGRAM_ID, join(ROOT, "tests/fixtures/programs/pump_fees.so"));
    this.svm.addProgramFromFile(PUMP_HOOKS_PROGRAM_ID, join(ROOT, "target/deploy/pump_hooks.so"));

    const dir = join(ROOT, "tests/fixtures", fixture);
    this.manifest = JSON.parse(readFileSync(join(dir, "manifest.json"), "utf8"));
    for (const f of readdirSync(dir)) {
      if (f === "manifest.json" || !f.endsWith(".json")) continue;
      const j = JSON.parse(readFileSync(join(dir, f), "utf8"));
      const key = new PublicKey(j.pubkey);
      if (key.toBase58() === "So11111111111111111111111111111111111111112" && this.svm.getAccount(key)) continue;
      this.svm.setAccount(key, {
        lamports: Number(j.account.lamports),
        data: Buffer.from(j.account.data[0], "base64"),
        owner: new PublicKey(j.account.owner),
        executable: j.account.executable,
        rentEpoch: 0,
      });
    }
    this.svm.airdrop(this.admin.publicKey, 100n * SOL);

    const svm = this.svm;
    const toInfo = (k: PublicKey): AccountInfo<Buffer> | null => {
      const a = svm.getAccount(k);
      return a ? { ...a, data: Buffer.from(a.data) } : null;
    };
    const recipient = new PublicKey(this.manifest.buyback_fee_recipient);
    this.rpc = {
      getAccountInfo: async (k: PublicKey) => toInfo(k),
      getMultipleAccountsInfo: async (ks: PublicKey[]) => ks.map(toInfo),
      getLatestBlockhash: async () => ({ blockhash: svm.latestBlockhash(), lastValidBlockHeight: 0 }),
      simulateTransaction: (async (tx: VersionedTransaction) => {
        // Simulations are unsigned (like RPC `sigVerify: false`).
        svm.withSigverify(false);
        const r = svm.simulateTransaction(tx);
        svm.withSigverify(true);
        const meta = r instanceof FailedTransactionMetadata ? r.meta() : r.meta();
        const value: SimulatedTransactionResponse = {
          err: r instanceof FailedTransactionMetadata ? (errValue(r.err()) as SimulatedTransactionResponse["err"]) : null,
          logs: meta.logs(),
          unitsConsumed: Number(meta.computeUnitsConsumed()),
          accounts: null,
        };
        return { context: { slot: 0 }, value };
      }) as unknown as RpcLike["simulateTransaction"],
    } as RpcLike;
    this.client = new PumpHooksClient(this.rpc, { pickBuybackRecipient: () => recipient });
  }

  get mint() {
    return new PublicKey(this.manifest.mint);
  }

  now(): bigint {
    return this.svm.getClock().unixTimestamp;
  }

  advance(secs: bigint, slots = 1n) {
    const c = this.svm.getClock();
    this.svm.setClock(new Clock(c.slot + slots, c.epochStartTimestamp, c.epoch, c.leaderScheduleEpoch, c.unixTimestamp + secs));
    this.svm.expireBlockhash();
  }

  /** Sign and execute; returns the same decoded result the SDK produces from RPC logs. */
  exec(b: BuiltTransaction, signers: Keypair[]): ExecutionResult {
    const tx = b.toTransaction(this.svm.latestBlockhash(), signers[0].publicKey);
    tx.sign(signers);
    const r = this.svm.sendTransaction(tx);
    this.svm.expireBlockhash();
    if (r instanceof TransactionMetadata) return executionResult(r.logs(), null, Number(r.computeUnitsConsumed()));
    return executionResult(r.meta().logs(), errValue(r.err()), Number(r.meta().computeUnitsConsumed()));
  }

  ixs(instructions: TransactionInstruction[], signers: Keypair[]): ExecutionResult {
    return this.exec(bundle(instructions, signers[0].publicKey), signers);
  }

  bootstrap() {
    const a = this.admin.publicKey;
    const ixs = [initializeIx({ authority: a })];
    for (const m of MODULES.filter((m) => m.available)) ixs.push(registerHookIx({ authority: a, moduleId: m.id, sourceHash: new Uint8Array(32).fill(m.id) }));
    const r = this.ixs(ixs, [this.admin]);
    if (!r.ok) throw new Error(`bootstrap failed: ${r.logs.join("\n")}`);
  }

  wallet(sol: bigint): Keypair {
    const k = Keypair.generate();
    this.svm.airdrop(k.publicKey, sol);
    return k;
  }

  lamports(k: PublicKey): bigint {
    return this.svm.getBalance(k) ?? 0n;
  }
}
