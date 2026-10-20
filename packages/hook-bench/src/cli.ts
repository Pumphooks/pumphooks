#!/usr/bin/env node
// hook-bench compare <baseline.json> <results.json> [--tolerance 5]
import { readFileSync } from "node:fs";
import { compare, hasFailures, parseResults, renderDiff } from "./index.js";

const args = process.argv.slice(2);
if (args[0] !== "compare" || args.length < 3) {
  console.error("usage: hook-bench compare <baseline.json> <results.json> [--tolerance <pct>]");
  process.exit(2);
}
const t = args.indexOf("--tolerance");
const tolerance = t >= 0 ? Number(args[t + 1]) : 5;
const diffs = compare(parseResults(readFileSync(args[1], "utf8")), parseResults(readFileSync(args[2], "utf8")), tolerance);
process.stdout.write(renderDiff(diffs));
process.exit(hasFailures(diffs) ? 1 : 0);
