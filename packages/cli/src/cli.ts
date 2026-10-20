import { parseArgs } from "node:util";
import { readFileSync } from "node:fs";
import { Keypair, PublicKey, Signer, TransactionInstruction } from "@solana/web3.js";
import {
  bundle,
  BuiltTransaction,
  Callback,
  callbackMask,
  callbacksOf,
  compilePolicy,
  CONFIG_SCHEMAS,
  decodeGlobalConfig,
  describePolicy,
  ExecutionResult,
  hooksPda,
  ModuleConfig,
  moduleInfo,
  MODULES,
  PolicySpec,
  PUMP_HOOKS_PROGRAM_ID,
  PumpHooksClient,
  RpcLike,
  TradeMode,
  Venue,
} from "@pump-hooks/sdk";
import { json, marketText, parseSol, parseUnits, resultText, sol, table, units } from "./format.js";

export interface Deps {
  rpc: RpcLike;
  out: (s: string) => void;
  /** Sign and send; returns the decoded execution and signature. */
  send?: (b: BuiltTransaction, signers: Signer[]) => Promise<ExecutionResult & { signature?: string }>;
  readKeypair?: (path: string) => Keypair;
  clientOptions?: ConstructorParameters<typeof PumpHooksClient>[1];
}

export const USAGE = `pump-hooks — programmable execution layer for existing Pump markets

Hooks run at the Pump Hooks router boundary: they govern trades routed through Pump Hooks.
Trades sent to Pump or PumpSwap directly are not affected.

usage: pump-hooks <command> [args] [--url <rpc>] [--keypair <file>] [--json]

  status                              program, registry and kill-switch state
  inspect <mint>                      detect an existing Pump token and its hooked markets
  hooks [module]                      hook registry (modules, callbacks, capabilities, installs)
  market show <market>                a market's hook stack and config
  market create <mint>                hook an existing Pump token (namespace = --keypair)
  install <market> <module>           --config k=v,... [--priority n] [--callbacks BeforeSwap,AfterSwap]
                                      (quote-size fields are in SOL; suffix "lamports" for raw units)
  uninstall <market> <module>
  policy list <market>
  policy create <market> --spec <file.json|json>
  quote <mint> (--buy <SOL> | --sell <tokens>)   fee-free constant-product estimate
  simulate <market> (--buy <SOL> | --sell <tokens>) [--user <pubkey>]
  trade <market> (--buy <SOL> | --sell <tokens>) [--min-out <n>]
  tx <signature>                      decode a routed transaction's hook trace

write commands accept --dry-run (simulate only). Default RPC: https://api.devnet.solana.com.
This CLI never reads the Solana CLI config; pass --url and --keypair explicitly.`;

const OPTIONS = {
  url: { type: "string" },
  keypair: { type: "string" },
  json: { type: "boolean" },
  "dry-run": { type: "boolean" },
  config: { type: "string" },
  priority: { type: "string" },
  callbacks: { type: "string" },
  buy: { type: "string" },
  sell: { type: "string" },
  "min-out": { type: "string" },
  user: { type: "string" },
  spec: { type: "string" },
  help: { type: "boolean", short: "h" },
} as const;

type Values = ReturnType<typeof parse>["values"];
const parse = (argv: string[]) => parseArgs({ args: argv, options: OPTIONS, allowPositionals: true, strict: true });

class UsageError extends Error {}
const need = <T>(v: T | undefined, what: string): T => {
  if (v === undefined || v === "") throw new UsageError(`missing ${what}`);
  return v;
};
const key = (s: string | undefined, what: string) => {
  try {
    return new PublicKey(need(s, what));
  } catch (e) {
    if (e instanceof UsageError) throw e;
    throw new UsageError(`invalid ${what}: ${s}`);
  }
};

export function parseConfig(module: number, spec: string | undefined): ModuleConfig {
  if (!spec) return {};
  const schema = CONFIG_SCHEMAS[module as keyof typeof CONFIG_SCHEMAS] ?? [];
  const out: ModuleConfig = {};
  for (const part of spec.split(",").map((s) => s.trim()).filter(Boolean)) {
    const [k, v] = part.split("=");
    const f = schema.find((x) => x.name === k);
    if (!f || v === undefined) throw new UsageError(`unknown config field "${k}"; fields: ${schema.map((x) => x.name).join(", ")}`);
    if (f.type === "pubkey") out[k] = new PublicKey(v);
    else if (f.unit === "lamports") out[k] = parseSol(v);
    else if (f.type === "u64" || f.type === "i64") out[k] = BigInt(v);
    else out[k] = Number(v);
  }
  return out;
}

export async function run(argv: string[], deps: Deps): Promise<number> {
  let values: Values;
  let positionals: string[];
  try {
    ({ values, positionals } = parse(argv));
  } catch (e) {
    deps.out(`${(e as Error).message}\n\n${USAGE}`);
    return 2;
  }
  if (values.help || positionals.length === 0) {
    deps.out(USAGE);
    return positionals.length === 0 && !values.help ? 2 : 0;
  }
  const client = new PumpHooksClient(deps.rpc, deps.clientOptions);
  const asJson = !!values.json;
  const print = (text: string, data: unknown) => deps.out(asJson ? json(data) : text);
  const signer = () => {
    const path = need(values.keypair, "--keypair <file> (required for write commands)");
    return (deps.readKeypair ?? ((p) => Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(p, "utf8"))))))(path);
  };
  const execute = async (ixs: TransactionInstruction[] | BuiltTransaction, kp: Keypair): Promise<number> => {
    const b = Array.isArray(ixs) ? bundle(ixs, kp.publicKey) : ixs;
    if (values["dry-run"] || !deps.send) {
      const r = await client.simulate(b, kp.publicKey);
      print(`(simulated, not sent)\n${resultText(r)}`, { simulated: true, ...r, logs: r.logs });
      return r.ok ? 0 : 1;
    }
    const r = await deps.send(b, [kp]);
    print(`${r.signature ? `signature     ${r.signature}\n` : ""}${resultText(r)}`, r);
    return r.ok ? 0 : 1;
  };

  const [cmd, a1, a2] = positionals;
  try {
    switch (cmd) {
      case "status": {
        const g = await deps.rpc.getAccountInfo(hooksPda.global(client.programId));
        const prog = await deps.rpc.getAccountInfo(client.programId);
        if (!prog) {
          print(`program ${client.programId.toBase58()} is not deployed on this cluster`, { deployed: false });
          return 1;
        }
        if (!g) {
          print(`program ${client.programId.toBase58()} deployed; protocol not initialized`, { deployed: true, initialized: false });
          return 0;
        }
        const gc = decodeGlobalConfig(g.data);
        const defs = await Promise.all(MODULES.map((m) => client.getHookDefinition(m.id)));
        const killed = MODULES.filter((m) => gc.disabledModules & (1n << BigInt(m.id))).map((m) => m.name);
        print(
          [
            `program       ${client.programId.toBase58()}`,
            `authority     ${gc.authority.toBase58()}`,
            `paused        ${gc.paused}`,
            `kill switch   ${killed.join(", ") || "none"}`,
            `markets       ${gc.marketCount}`,
            `registry      ${defs.filter(Boolean).length}/${MODULES.filter((m) => m.available).length} first-party modules registered`,
          ].join("\n"),
          { deployed: true, initialized: true, ...gc, registered: defs.filter(Boolean).length, killed },
        );
        return 0;
      }

      case "inspect": {
        const mint = key(a1, "<mint>");
        const t = await client.detect(mint);
        let markets: Awaited<ReturnType<typeof client.findMarkets>> = [];
        let note = "";
        try {
          markets = await client.findMarkets(mint);
        } catch {
          note = " (RPC does not allow getProgramAccounts; pass a market address to `market show`)";
        }
        const venue = t.venue === Venue.PumpSwap ? `PumpSwap (migrated) pool ${t.pool.toBase58()}` : t.venue === Venue.BondingCurve ? "bonding curve (not migrated)" : "curve complete, migration pending";
        print(
          [
            `mint          ${mint.toBase58()}`,
            `token         ${t.isToken2022 ? "Token-2022" : "SPL Token"}, ${t.decimals} decimals`,
            `creator       ${t.creator.toBase58()}`,
            `bonding curve ${t.bondingCurve.toBase58()}`,
            `venue         ${venue}`,
            `quote mint    ${t.quoteMint.toBase58()}`,
            `reserves      base ${units(t.reserves.base, t.decimals)}  quote ${sol(t.reserves.quote)}`,
            `flags         ${[t.isHolderReward && "holder-rewards", t.isMayhem && "mayhem", t.isCashback && "cashback"].filter(Boolean).join(" ") || "-"}`,
            `routable      ${t.routable ? "yes" : `no — ${t.notRoutableReason}`}`,
            `hooked        ${markets.length ? markets.map((m) => `${m.address.toBase58()} (${m.config.slots.length} hooks, namespace ${m.config.namespace.toBase58()})`).join("\n              ") : "none"}${note}`,
          ].join("\n"),
          { ...t, markets },
        );
        return 0;
      }

      case "hooks": {
        const list = a1 ? [moduleInfo(isNaN(Number(a1)) ? a1 : Number(a1))] : MODULES;
        const defs = await Promise.all(list.map((m) => client.getHookDefinition(m.id)));
        const rows: (string | number)[][] = [["id", "slug", "name", "v", "callbacks", "priority", "registered", "installs", "audit"]];
        list.forEach((m, i) => {
          const d = defs[i];
          rows.push([
            m.id, m.slug, m.name, m.version,
            m.callbacks ? callbacksOf(m.callbacks).map((c) => Callback[c]).join(",") : "config-only",
            `${m.priorityMin}-${m.priorityMax}`,
            !m.available ? "not available" : d ? (d.enabled ? "yes" : "disabled") : "no",
            d ? String(d.installs) : "-",
            d && d.auditStatus !== 0 ? `status ${d.auditStatus}` : "unaudited",
          ]);
        });
        print(table(rows), list.map((m, i) => ({ ...m, definition: defs[i] })));
        return 0;
      }

      case "market": {
        if (a1 === "show") {
          const m = await client.getMarket(key(a2, "<market>"));
          print(marketText(m.address, m.config), m);
          return 0;
        }
        if (a1 === "create") {
          const kp = signer();
          const { market, ix, token } = await client.createMarket(key(a2, "<mint>"), kp.publicKey);
          deps.out(`hooking existing Pump token ${token.mint.toBase58()} → market ${market.toBase58()}`);
          return execute([ix], kp);
        }
        throw new UsageError("market show <market> | market create <mint>");
      }

      case "install": {
        const kp = signer();
        const market = key(a1, "<market>");
        const info = moduleInfo(isNaN(Number(a2)) ? need(a2, "<module>") : Number(a2));
        const cbs = values.callbacks?.split(",").map((c) => {
          const v = (Callback as unknown as Record<string, number>)[c.trim()];
          if (v === undefined) throw new UsageError(`unknown callback ${c}`);
          return v as Callback;
        });
        const ix = client.installHook({
          market,
          authority: kp.publicKey,
          module: info.id,
          config: parseConfig(info.id, values.config),
          priority: values.priority ? Number(values.priority) : undefined,
          callbacks: cbs ? callbackMask(...cbs) : undefined,
        });
        return execute([ix], kp);
      }

      case "uninstall": {
        const kp = signer();
        return execute([client.removeHook({ market: key(a1, "<market>"), authority: kp.publicKey, module: isNaN(Number(a2)) ? need(a2, "<module>") : Number(a2) })], kp);
      }

      case "policy": {
        const market = await client.getMarket(key(a2, "<market>"));
        if (a1 === "list") {
          const ps = await client.getPolicies(market);
          print(ps.length ? ps.map((p) => `#${p.index} ${p.enabled ? "" : "(disabled) "}${describePolicy(p)}  executions=${p.execCount}`).join("\n") : "no policies", ps);
          return 0;
        }
        if (a1 === "create") {
          const kp = signer();
          const raw = need(values.spec, "--spec <file.json|json>");
          const spec = JSON.parse(raw.trim().startsWith("{") ? raw : readFileSync(raw, "utf8")) as PolicySpec;
          const { index, ix } = client.createPolicy({ market, authority: kp.publicKey, spec });
          deps.out(`policy #${index}: ${describePolicy(compilePolicy(spec))}`);
          return execute([ix], kp);
        }
        throw new UsageError("policy list <market> | policy create <market> --spec ...");
      }

      case "quote": {
        const mint = key(a1, "<mint>");
        const t = await client.detect(mint);
        if (values.buy) {
          const q = await client.quote(mint, TradeMode.ExactQuoteIn, parseSol(values.buy));
          print(`buy ${sol(q.quoteIn!)} → ~${units(q.baseOut!, t.decimals)} tokens (${q.method}; use simulate for the executed amount)`, q);
        } else {
          const q = await client.quote(mint, TradeMode.ExactBaseIn, parseUnits(need(values.sell, "--buy or --sell"), t.decimals));
          print(`sell ${units(q.baseIn!, t.decimals)} tokens → ~${sol(q.quoteOut!)} (${q.method}; use simulate for the executed amount)`, q);
        }
        return 0;
      }

      case "simulate":
      case "trade": {
        const market = await client.getMarket(key(a1, "<market>"));
        const decimals = (await client.detect(market.config.mint)).decimals;
        const user = cmd === "simulate" && values.user ? new PublicKey(values.user) : signer().publicKey;
        const minOut = values["min-out"];
        const built = values.buy
          ? await client.buy({ market, user, quoteIn: parseSol(values.buy), minBaseOut: minOut ? parseUnits(minOut, decimals) : undefined })
          : await client.sell({ market, user, baseIn: parseUnits(need(values.sell, "--buy or --sell"), decimals), minQuoteOut: minOut ? parseSol(minOut) : undefined });
        if (cmd === "simulate") {
          const r = await client.simulate(built, user);
          print(`(simulated, not sent)\n${resultText(r, decimals)}`, r);
          return r.ok ? 0 : 1;
        }
        return execute(built, signer());
      }

      case "tx": {
        const r = await client.getExecutionTrace(need(a1, "<signature>"));
        print(`slot          ${r.slot}\n${resultText(r)}`, r);
        return r.ok ? 0 : 1;
      }

      default:
        throw new UsageError(`unknown command ${cmd}`);
    }
  } catch (e) {
    deps.out(e instanceof UsageError ? `${e.message}\n\n${USAGE}` : `error: ${(e as Error).message}`);
    return e instanceof UsageError ? 2 : 1;
  }
}

export { PUMP_HOOKS_PROGRAM_ID };
