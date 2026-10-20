#!/usr/bin/env node
import { Connection } from "@solana/web3.js";
import { PumpHooksClient, PUMP_ALT } from "@pump-hooks/sdk";
import { run } from "./cli.js";

const argv = process.argv.slice(2);
const urlIdx = argv.indexOf("--url");
const url = urlIdx >= 0 ? argv[urlIdx + 1] : "https://api.devnet.solana.com";
if (/mainnet/i.test(url) && !argv.includes("--dry-run") && argv.some((a) => ["create", "install", "uninstall", "trade"].includes(a))) {
  // Mainnet writes need an explicit opt-in; the program is not deployed there.
  if (process.env.PUMP_HOOKS_ALLOW_MAINNET !== "1") {
    console.error("refusing to send to a mainnet RPC without PUMP_HOOKS_ALLOW_MAINNET=1");
    process.exit(2);
  }
}
const connection = new Connection(url, "confirmed");
const clientOptions = { lookupTables: [/mainnet/i.test(url) ? PUMP_ALT.mainnet : PUMP_ALT.devnet] };
const client = new PumpHooksClient(connection, clientOptions);

run(argv, {
  rpc: connection,
  out: (s) => console.log(s),
  clientOptions,
  send: async (b, signers) => {
    const { signature, result } = await client.send(b, signers);
    return { ...result, signature };
  },
}).then((code) => process.exit(code));
