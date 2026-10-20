// Initializes the deployed v1 programs on devnet: hook-registry and hook-router (PumpSwap venue).
//   tsx scripts/devnet-init.mts --keypair <upgrade-authority.json> [--url <devnet rpc>]
// Devnet only. Already-initialized programs are skipped. The keypair must be the programs' upgrade authority.
import { v1 } from "@pump-hooks/sdk";
import { admin, exists, send } from "./devnet.mts";

console.log(`authority ${admin.address}`);

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
