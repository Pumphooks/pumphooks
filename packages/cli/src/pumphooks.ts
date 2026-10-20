#!/usr/bin/env node
// `pumphooks`: the developer commands (see dev.ts); anything else is the `pump-hooks` market CLI.
import { DEV_USAGE, isDevCommand, runDev } from "./dev.js";

const argv = process.argv.slice(2);
if (argv.length === 0 || argv[0] === "--help" || argv[0] === "-h") {
  console.log(DEV_USAGE);
  console.log("\nmarket commands (status, inspect, hooks, market, install, policy, quote, simulate <market>, trade, tx): pumphooks <command> --help");
  process.exit(argv.length === 0 ? 2 : 0);
}
if (isDevCommand(argv)) {
  runDev(argv, { out: (s) => console.log(s) }).then((code) => process.exit(code));
} else {
  await import("./bin.js");
}
