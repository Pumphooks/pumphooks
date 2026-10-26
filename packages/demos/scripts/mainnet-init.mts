// Initializes hook-registry and hooks-core on mainnet. Idempotent; each instruction is simulated first.
//   tsx scripts/mainnet-init.mts --url <mainnet rpc> --keypair <upgrade-authority.json> [--dry-run]
// Both programs check on chain that the signer is their upgrade authority.
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
} from "@solana/kit";
import { toKit } from "@pump-hooks/hook-security";
import { v1 } from "@pump-hooks/sdk";

const { values } = parseArgs({ options: { keypair: { type: "string" }, url: { type: "string" }, "dry-run": { type: "boolean", default: false } } });
if (!values.keypair || !values.url) throw new Error("usage: mainnet-init.mts --url <rpc> --keypair <upgrade-authority.json> [--dry-run]");

const rpc = createSolanaRpc(values.url);
const admin = await createKeyPairSignerFromBytes(Uint8Array.from(JSON.parse(readFileSync(values.keypair, "utf8"))));
const exists = async (a: string) => (await rpc.getAccountInfo(a as Address, { encoding: "base64", dataSlice: { offset: 0, length: 0 } }).send()).value !== null;

async function send(label: string, ix: v1.InstructionV1) {
  const fee: v1.InstructionV1 = { programId: "ComputeBudget111111111111111111111111111111", accounts: [], data: v1.concat(v1.u8(3), v1.u64(200_000n)) };
  const { value: blockhash } = await rpc.getLatestBlockhash().send();
  const msg = pipe(
    createTransactionMessage({ version: 0 }),
    (m) => setTransactionMessageFeePayerSigner(admin, m),
    (m) => setTransactionMessageLifetimeUsingBlockhash(blockhash, m),
    (m) => appendTransactionMessageInstructions([fee, ix].map(toKit), m),
  );
  const wire = getBase64EncodedWireTransaction(await signTransactionMessageWithSigners(msg));
  const sim = await rpc.simulateTransaction(wire, { encoding: "base64" }).send();
  if (sim.value.err) throw new Error(`${label}: simulation failed\n${(sim.value.logs ?? []).join("\n")}`);
  if (values["dry-run"]) return console.log(`[dry-run] ${label}: simulation ok`);
  const signature = await rpc.sendTransaction(wire, { encoding: "base64", skipPreflight: true }).send();
  for (let i = 0; i < 90; i++) {
    const [st] = (await rpc.getSignatureStatuses([signature]).send()).value;
    if (st?.confirmationStatus === "confirmed" || st?.confirmationStatus === "finalized") {
      if (st.err) throw new Error(`${label}: failed ${signature}`);
      return console.log(`${label}: ${signature}`);
    }
    await new Promise((r) => setTimeout(r, 1500));
  }
  throw new Error(`${label}: not confirmed (${signature})`);
}

console.log(`authority ${admin.address}`);

if (await exists(v1.registryPda.config())) console.log("hook-registry: already initialized");
else
  await send("hook-registry initialize", {
    programId: v1.REGISTRY_PROGRAM_ID,
    accounts: [v1.meta(admin.address, true, true), v1.meta(v1.registryPda.config(), true), v1.meta(v1.REGISTRY_PROGRAM_ID), v1.meta(v1.programData(v1.REGISTRY_PROGRAM_ID)), v1.meta(v1.SYSTEM_PROGRAM_ID)],
    data: v1.anchorDiscriminator("initialize"),
  });

if (await exists(v1.poolPda.protocol())) console.log("hooks-core: already initialized");
else await send("hooks-core initialize_protocol", v1.initializeProtocol(admin.address, admin.address));
