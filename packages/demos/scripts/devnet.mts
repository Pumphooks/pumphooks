// Shared by the devnet scripts: devnet-only RPC, an explicit keypair, send + confirm with logs.
import { readFileSync } from "node:fs";
import { parseArgs } from "node:util";
import {
  appendTransactionMessageInstructions,
  createKeyPairSignerFromBytes,
  createSolanaRpc,
  createTransactionMessage,
  getBase64EncodedWireTransaction,
  pipe,
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  signTransactionMessageWithSigners,
  type Address,
  type Signature,
} from "@solana/kit";
import { toKit } from "@pump-hooks/hook-security";
import { v1 } from "@pump-hooks/sdk";

const { values } = parseArgs({
  options: { keypair: { type: "string" }, url: { type: "string", default: process.env.DEVNET_RPC ?? "https://api.devnet.solana.com" } },
});
if (!values.keypair) throw new Error("--keypair <file> is required");
if (!/devnet/.test(values.url!)) throw new Error("devnet only: pass a devnet RPC URL (--url or DEVNET_RPC)");

export const rpc = createSolanaRpc(values.url!);
export const admin = await createKeyPairSignerFromBytes(Uint8Array.from(JSON.parse(readFileSync(values.keypair, "utf8"))));
export const explorer = (sig: string) => `https://explorer.solana.com/tx/${sig}?cluster=devnet`;

export async function exists(a: string): Promise<boolean> {
  return (await rpc.getAccountInfo(a as Address, { encoding: "base64", dataSlice: { offset: 0, length: 0 } }).send()).value !== null;
}

export async function accountData(a: string): Promise<Uint8Array | null> {
  const v = (await rpc.getAccountInfo(a as Address, { encoding: "base64" }).send()).value;
  return v ? Uint8Array.from(Buffer.from(v.data[0], "base64")) : null;
}

export interface Sent {
  signature: string;
  ok: boolean;
  logs: string[];
  computeUnits: number | null;
}

/**
 * Sends one transaction. With `land` a transaction that fails simulation is still sent (skipping
 * preflight), so the rejection is recorded on chain; otherwise a failing simulation throws.
 */
export async function send(label: string, ixs: v1.InstructionV1[], o: { land?: boolean } = {}): Promise<Sent> {
  const budget: v1.InstructionV1 = { programId: "ComputeBudget111111111111111111111111111111", accounts: [], data: v1.concat(v1.u8(2), v1.u32(400_000)) };
  const fee: v1.InstructionV1 = { programId: "ComputeBudget111111111111111111111111111111", accounts: [], data: v1.concat(v1.u8(3), v1.u64(10_000n)) };
  const { value: blockhash } = await rpc.getLatestBlockhash().send();
  const msg = pipe(
    createTransactionMessage({ version: 0 }),
    (m) => setTransactionMessageFeePayerSigner(admin, m),
    (m) => setTransactionMessageLifetimeUsingBlockhash(blockhash, m),
    (m) => appendTransactionMessageInstructions([budget, fee, ...ixs].map(toKit), m),
  );
  const wire = getBase64EncodedWireTransaction(await signTransactionMessageWithSigners(msg));
  const sim = await rpc.simulateTransaction(wire, { encoding: "base64" }).send();
  if (sim.value.err && !o.land) throw new Error(`${label}: simulation failed ${JSON.stringify(sim.value.err, (_k, v) => (typeof v === "bigint" ? v.toString() : v))}\n${(sim.value.logs ?? []).join("\n")}`);
  const signature = await rpc.sendTransaction(wire, { encoding: "base64", skipPreflight: true }).send();
  for (let i = 0; i < 90; i++) {
    const [st] = (await rpc.getSignatureStatuses([signature]).send()).value;
    if (st?.confirmationStatus === "confirmed" || st?.confirmationStatus === "finalized") {
      const tx = await rpc.getTransaction(signature as Signature, { encoding: "json", maxSupportedTransactionVersion: 0, commitment: "confirmed" }).send();
      const r: Sent = { signature, ok: !st.err, logs: tx?.meta?.logMessages ?? [], computeUnits: tx?.meta?.computeUnitsConsumed != null ? Number(tx.meta.computeUnitsConsumed) : null };
      console.log(`${r.ok ? "ok      " : "rejected"} ${label}\n         ${explorer(signature)}`);
      return r;
    }
    await new Promise((res) => setTimeout(res, 1000));
  }
  throw new Error(`${label}: not confirmed (${signature})`);
}
