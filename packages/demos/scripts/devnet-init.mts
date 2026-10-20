// Initializes the deployed v1 programs on devnet: hook-registry and hook-router (PumpSwap venue).
//   tsx scripts/devnet-init.mts --keypair <upgrade-authority.json> [--url https://api.devnet.solana.com]
// Devnet only. Already-initialized programs are skipped. The keypair must be the programs' upgrade authority.
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

const { values } = parseArgs({ options: { keypair: { type: "string" }, url: { type: "string", default: "https://api.devnet.solana.com" } } });
if (!values.keypair) throw new Error("--keypair <upgrade-authority.json> is required");
if (!/devnet/.test(values.url!)) throw new Error("devnet only: pass a devnet RPC URL");

const rpc = createSolanaRpc(values.url!);
const admin = await createKeyPairSignerFromBytes(Uint8Array.from(JSON.parse(readFileSync(values.keypair, "utf8"))));
const exists = async (a: string) => (await rpc.getAccountInfo(a as Address, { encoding: "base64", dataSlice: { offset: 0, length: 0 } }).send()).value !== null;

async function send(label: string, ixs: v1.InstructionV1[]) {
  const { value: blockhash } = await rpc.getLatestBlockhash().send();
  const msg = pipe(
    createTransactionMessage({ version: 0 }),
    (m) => setTransactionMessageFeePayerSigner(admin, m),
    (m) => setTransactionMessageLifetimeUsingBlockhash(blockhash, m),
    (m) => appendTransactionMessageInstructions(ixs.map(toKit), m),
  );
  const tx = await signTransactionMessageWithSigners(msg);
  const wire = getBase64EncodedWireTransaction(tx);
  const sim = await rpc.simulateTransaction(wire, { encoding: "base64" }).send();
  if (sim.value.err) throw new Error(`${label}: simulation failed ${JSON.stringify(sim.value.err)}\n${(sim.value.logs ?? []).join("\n")}`);
  const sig = await rpc.sendTransaction(wire, { encoding: "base64" }).send();
  for (let i = 0; i < 60; i++) {
    const [st] = (await rpc.getSignatureStatuses([sig]).send()).value;
    if (st?.err) throw new Error(`${label}: ${JSON.stringify(st.err)} (${sig})`);
    if (st?.confirmationStatus === "confirmed" || st?.confirmationStatus === "finalized") {
      console.log(`${label}: ${sig}`);
      return;
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  throw new Error(`${label}: not confirmed (${sig})`);
}

console.log(`authority ${admin.address} on ${values.url}`);

if (await exists(v1.registryPda.config())) console.log("hook-registry: already initialized");
else
  await send("hook-registry initialize", [
    {
      programId: v1.REGISTRY_PROGRAM_ID,
      accounts: [v1.meta(admin.address, true, true), v1.meta(v1.registryPda.config(), true), v1.meta(v1.REGISTRY_PROGRAM_ID), v1.meta(v1.programData(v1.REGISTRY_PROGRAM_ID)), v1.meta(v1.SYSTEM_PROGRAM_ID)],
      data: v1.anchorDiscriminator("initialize"),
    },
  ]);

if (await exists(v1.routerPda.config())) console.log("hook-router: already initialized");
else await send("hook-router initialize + enable PumpSwap", [v1.initializeRouter(admin.address), v1.setVenue(admin.address, v1.Venue.PumpSwap, true)]);
