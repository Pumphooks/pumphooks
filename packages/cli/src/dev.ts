// `pumphooks` developer commands for ABI v1 program hooks: scaffold, build, test, simulate a
// policy call, benchmark, security-check, deploy (devnet / localnet only) and register.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { Connection, Keypair, PublicKey, Transaction } from "@solana/web3.js";
import { v1 } from "@pump-hooks/sdk";
import { analyze, measuredFromBench } from "@pump-hooks/hook-analyzer";
import { compare, hasFailures, parseResults, renderDiff } from "@pump-hooks/hook-bench";

export const DEV_USAGE = `pumphooks — build, test and ship PUMP HOOKS program hooks (ABI v1)

usage: pumphooks <command> [args]

  init <name> [--env router|account] [--dir <parent>] [--sdk-path <crates/hook-sdk>]
                                   scaffold a program hook (Rust, hook-sdk) with hook.json
  build [dir]                      cargo build-sbf → <dir>/target/deploy
  test [dir]                       cargo test
  simulate --program <file.so> [--env router] [--callback before_swap] [--amount n] [--amount-b n]
           [--slot n] [--actor pk] [--asset pk] [--accounts <dir>]
                                   run one policy call against the program in LiteSVM
  simulate --stack <hex|file> [--members n] [--bench bench/results.json]
                                   validate a stack with the kernel rules and estimate compute
  bench [--baseline bench/baseline.json] [--no-run] [--tolerance 5]
                                   measured compute units (Mollusk + LiteSVM) vs the baseline
  security-check [--stack <hex|file>] [--runs n] [--deploy-dir target/deploy] [--out file]
                                   stack diagnostics + stateful fuzzing of the token wrapper
                                   (not an audit)
  deploy --cluster devnet|localnet --program <file.so> --keypair <file> [--program-id <keypair>] [--yes]
  register --cluster devnet|localnet --manifest hook.json --keypair <file> [--program <id>] [--so <file.so>] [--yes]

deploy and register refuse mainnet. Without --yes they print / simulate what they would do.
Keypairs are only read from the --keypair path given; the Solana CLI config is never read.`;

export const CLUSTERS: Record<string, string> = {
  devnet: "https://api.devnet.solana.com",
  localnet: "http://127.0.0.1:8899",
};

export interface DevDeps {
  out: (s: string) => void;
  /** Runs a process with inherited stdio; returns its exit code. */
  spawn?: (cmd: string, args: string[], cwd?: string) => number;
  cwd?: string;
  connection?: (url: string) => Pick<Connection, "getAccountInfo" | "simulateTransaction" | "getLatestBlockhash" | "sendRawTransaction" | "confirmTransaction">;
}

const OPTIONS = {
  env: { type: "string" },
  dir: { type: "string" },
  "sdk-path": { type: "string" },
  program: { type: "string" },
  callback: { type: "string" },
  amount: { type: "string" },
  "amount-b": { type: "string" },
  slot: { type: "string" },
  actor: { type: "string" },
  asset: { type: "string" },
  accounts: { type: "string" },
  stack: { type: "string" },
  members: { type: "string" },
  bench: { type: "string" },
  baseline: { type: "string" },
  "no-run": { type: "boolean" },
  tolerance: { type: "string" },
  runs: { type: "string" },
  "deploy-dir": { type: "string" },
  out: { type: "string" },
  cluster: { type: "string" },
  keypair: { type: "string" },
  "program-id": { type: "string" },
  manifest: { type: "string" },
  so: { type: "string" },
  yes: { type: "boolean" },
  json: { type: "boolean" },
  help: { type: "boolean", short: "h" },
} as const;

export const DEV_COMMANDS = ["init", "build", "test", "bench", "security-check", "deploy", "register"];

/** `simulate` is a dev command only with --program or --stack; otherwise it is the market simulate. */
export function isDevCommand(argv: string[]): boolean {
  const cmd = argv.find((a) => !a.startsWith("-"));
  if (cmd === "simulate") return argv.includes("--program") || argv.includes("--stack");
  return cmd !== undefined && DEV_COMMANDS.includes(cmd);
}

class UsageError extends Error {}

const ENVS: Record<string, v1.EnvironmentId> = { token: 0, router: 1, pool: 2, account: 3 };

function envId(name: string | undefined, fallback: v1.EnvironmentId = 1): v1.EnvironmentId {
  if (!name) return fallback;
  const e = ENVS[name];
  if (e === undefined) throw new UsageError(`unknown environment ${name}`);
  return e;
}

function callbackId(env: v1.EnvironmentId, name: string): number {
  const i = v1.CALLBACK_NAMES[env].indexOf(name);
  if (i < 0) throw new UsageError(`unknown callback ${name} for ${v1.ENVIRONMENT_NAMES[env]}: ${v1.CALLBACK_NAMES[env].join(", ")}`);
  return i;
}

/** Hex bytes or a file holding hex / raw bytes. */
export function readStack(arg: string, cwd: string): Uint8Array {
  const hex = /^[0-9a-fA-F]+$/;
  if (hex.test(arg) && arg.length % 2 === 0) return Uint8Array.from(Buffer.from(arg, "hex"));
  const raw = readFileSync(resolve(cwd, arg));
  const text = raw.toString("utf8").trim();
  return hex.test(text) ? Uint8Array.from(Buffer.from(text, "hex")) : new Uint8Array(raw);
}

export function findRepoRoot(from: string): string | null {
  let d = resolve(from);
  for (;;) {
    if (existsSync(join(d, "bench", "Cargo.toml")) && existsSync(join(d, "crates", "hook-sdk"))) return d;
    const up = dirname(d);
    if (up === d) return null;
    d = up;
  }
}

// ---------------------------------------------------------------------------------------------
// init

export interface HookManifest {
  slug: string;
  version: string;
  environment: string;
  callbacks: string[];
  capabilities: string[];
  manifestUri: string;
  configSchemaHash?: string;
}

const TEMPLATE_CALLBACK: Record<number, string> = { 1: "before_swap", 3: "before_withdraw" };
const TEMPLATE_PROGRAM: Record<number, string> = { 1: "ROUTER_PROGRAM_ID", 3: "SMART_ACCOUNTS_PROGRAM_ID" };
const TEMPLATE_ENV: Record<number, string> = { 1: "Router", 3: "Account" };

export function scaffold(name: string, env: v1.EnvironmentId, sdkDep: string): Record<string, string> {
  const crate = name.replace(/-/g, "_");
  const cb = TEMPLATE_CALLBACK[env];
  const manifest: HookManifest = {
    slug: name,
    version: "0.1.0",
    environment: v1.ENVIRONMENT_NAMES[env],
    callbacks: [cb],
    capabilities: ["VALIDATE_OPERATION", "VETO_OPERATION"],
    manifestUri: "",
  };
  return {
    "Cargo.toml": `[package]
name = "${name}"
version = "0.1.0"
edition = "2021"
publish = false

[lib]
crate-type = ["cdylib", "lib"]

[features]
no-entrypoint = []

[dependencies]
hook-sdk = ${sdkDep}
solana-program = "2"

[lints.rust]
unexpected_cfgs = { level = "allow" }

# Standalone: not part of an enclosing workspace.
[workspace]
`,
    "src/lib.rs": `//! ${name}: a PUMP HOOKS program hook (${v1.ENVIRONMENT_NAMES[env]} environment, \`${cb}\`).
//!
//! The environment calls this program with a \`PolicyContext\`; \`hook_sdk::process\` verifies
//! the caller (callback-authority PDA signature, subject ownership, ABI version) before
//! \`evaluate\` runs. A hook never receives the user's signature and cannot move user funds.

use hook_sdk::{Call, Decision, Environment, PolicyHook, CUSTOM_REASON_BASE, ${TEMPLATE_PROGRAM[env]}};
use solana_program::{account_info::AccountInfo, entrypoint::ProgramResult, program_error::ProgramError, pubkey::Pubkey};

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(process_instruction);

/// Largest amount this hook allows per operation, in the asset's base units.
pub const MAX_AMOUNT: u64 = 1_000_000_000;
pub const TOO_LARGE: u16 = CUSTOM_REASON_BASE;

pub struct Hook;

impl PolicyHook for Hook {
    const ENVIRONMENT: Environment = Environment::${TEMPLATE_ENV[env]};
    const ENVIRONMENT_PROGRAM: Pubkey = ${TEMPLATE_PROGRAM[env]};

    fn evaluate(_program_id: &Pubkey, call: &Call, _accounts: &[AccountInfo]) -> Result<Decision, ProgramError> {
        Ok(if call.amount() > MAX_AMOUNT { Decision::reject(TOO_LARGE) } else { Decision::allow() })
    }
}

pub fn process_instruction(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    hook_sdk::process_only::<Hook>(program_id, accounts, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decide(amount: u64) -> Decision {
        let mut ctx = hook_sdk::testing::context(Environment::${TEMPLATE_ENV[env]}, ${v1.CALLBACK_NAMES[env].indexOf(cb)}, Pubkey::new_unique(), Pubkey::new_unique());
        ctx.amount = amount;
        Hook::evaluate(&Pubkey::new_unique(), &Call { ctx, op_data: &[] }, &[]).unwrap()
    }

    #[test]
    fn allows_up_to_the_limit_and_rejects_above() {
        assert!(decide(MAX_AMOUNT).is_allow());
        assert_eq!(decide(MAX_AMOUNT + 1), Decision::reject(TOO_LARGE));
    }
}
`,
    "hook.json": JSON.stringify(manifest, null, 2) + "\n",
    ".gitignore": "target/\n",
    "README.md": `# ${name}

A PUMP HOOKS program hook for the ${v1.ENVIRONMENT_NAMES[env]} environment (\`${cb}\`).

    pumphooks test                      # unit tests
    pumphooks build                     # SBF build → target/deploy/${crate}.so
    pumphooks simulate --program target/deploy/${crate}.so --env ${v1.ENVIRONMENT_NAMES[env]} --callback ${cb} --amount 5
    pumphooks deploy --cluster devnet --program target/deploy/${crate}.so --keypair <devnet keypair>
    pumphooks register --cluster devnet --manifest hook.json --keypair <same keypair> --program <program id>

${env === 1 ? "Router hooks govern trades routed through hook-router only; the pool stays directly callable.\n" : ""}`,
  };
}

// ---------------------------------------------------------------------------------------------
// register

const sha256 = (b: Uint8Array | string) => new Uint8Array(createHash("sha256").update(b).digest());

function canonical(v: unknown): string {
  if (Array.isArray(v)) return `[${v.map(canonical).join(",")}]`;
  if (v && typeof v === "object") {
    const o = v as Record<string, unknown>;
    return `{${Object.keys(o).sort().map((k) => `${JSON.stringify(k)}:${canonical(o[k])}`).join(",")}}`;
  }
  return JSON.stringify(v);
}

/** Program hooks outside the pool may only claim policy capabilities (hook_standard::POLICY_PROGRAM_CAPS). */
const POLICY_PROGRAM_CAPS =
  v1.Cap.READ_ACCOUNT_STATE | v1.Cap.READ_TOKEN_BALANCE | v1.Cap.VALIDATE_OPERATION | v1.Cap.VETO_OPERATION | v1.Cap.UPDATE_OWN_STATE | v1.Cap.PRODUCE_STATE | v1.Cap.CONSUME_STATE;

export function publishParams(m: HookManifest, artifact?: Uint8Array): v1.PublishParams {
  const env = envId(m.environment);
  const semver = m.version.split(".").map(Number);
  if (semver.length !== 3 || semver.some((n) => !Number.isInteger(n) || n < 0 || n > 0xffff)) throw new UsageError(`invalid version ${m.version}`);
  const callbacks: [number, number, number, number] = [0, 0, 0, 0];
  callbacks[env] = m.callbacks.reduce((mask, c) => mask | (1 << callbackId(env, c)), 0);
  if (!callbacks[env]) throw new UsageError("manifest lists no callbacks");
  let capabilities = 0;
  for (const c of m.capabilities) {
    const bit = (v1.Cap as Record<string, number>)[c];
    if (bit === undefined) throw new UsageError(`unknown capability ${c}`);
    capabilities |= bit;
  }
  if (env !== 2 && capabilities & ~POLICY_PROGRAM_CAPS) throw new UsageError("program hooks may only claim policy capabilities (no asset-moving capabilities)");
  if (!capabilities) throw new UsageError("manifest lists no capabilities");
  if (!v1.validSlug(m.slug)) throw new UsageError(`invalid slug ${m.slug}`);
  return {
    semver: semver as [number, number, number],
    implKind: v1.IMPL_PROGRAM,
    builtinId: 0,
    environments: 1 << env,
    callbacks,
    capabilities,
    configSchemaHash: m.configSchemaHash ? Uint8Array.from(Buffer.from(m.configSchemaHash, "hex")) : new Uint8Array(32),
    manifestHash: sha256(canonical(m)),
    manifestUri: m.manifestUri,
    artifactHash: artifact ? sha256(artifact) : new Uint8Array(32),
  };
}

/** `create_hook` (when the slug is new) + `publish_version` at the hook's next index. */
export function registerInstructions(o: {
  manifest: HookManifest;
  publisher: string;
  program: string;
  existing: v1.HookAccount | null;
  artifact?: Uint8Array;
}): { instructions: v1.InstructionV1[]; hook: string; version: string; index: number } {
  const params = publishParams(o.manifest, o.artifact);
  const hook = v1.registryPda.hook(o.manifest.slug);
  if (o.existing && o.existing.authority !== o.publisher) throw new UsageError(`hook ${o.manifest.slug} belongs to ${o.existing.authority}`);
  const index = o.existing?.versionCount ?? 0;
  const instructions = [
    ...(o.existing ? [] : [v1.createHook(o.publisher, o.manifest.slug)]),
    v1.publishVersion({ publisher: o.publisher, hook, index, program: o.program, params }),
  ];
  return { instructions, hook, version: v1.registryPda.version(hook, index), index };
}

// ---------------------------------------------------------------------------------------------

function clusterUrl(cluster: string | undefined): string {
  if (!cluster) throw new UsageError("--cluster devnet|localnet is required");
  const url = CLUSTERS[cluster];
  if (!url) {
    throw new UsageError(
      /mainnet/i.test(cluster)
        ? "mainnet is refused: these programs are unaudited and mainnet deployment needs explicit approval outside this tool"
        : `unknown cluster ${cluster} (devnet | localnet)`,
    );
  }
  return url;
}

function readKeypair(path: string): Keypair {
  return Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(path, "utf8"))));
}

export async function runDev(argv: string[], deps: DevDeps): Promise<number> {
  const cwd = deps.cwd ?? process.cwd();
  const spawn = deps.spawn ?? ((cmd, args, dir) => spawnSync(cmd, args, { stdio: "inherit", cwd: dir ?? cwd }).status ?? 1);
  let values: ReturnType<typeof parse>["values"];
  let positionals: string[];
  const parse = (a: string[]) => parseArgs({ args: a, options: OPTIONS, allowPositionals: true, strict: true });
  try {
    ({ values, positionals } = parse(argv));
  } catch (e) {
    deps.out(`${(e as Error).message}\n\n${DEV_USAGE}`);
    return 2;
  }
  if (values.help) {
    deps.out(DEV_USAGE);
    return 0;
  }
  const [cmd, a1] = positionals;
  const need = (v: string | undefined, what: string) => {
    if (!v) throw new UsageError(`missing ${what}`);
    return v;
  };
  const dir = (p?: string) => resolve(cwd, p ?? ".");
  try {
    switch (cmd) {
      case "init": {
        const name = need(a1, "<name>");
        if (!v1.validSlug(name)) throw new UsageError("name must be a slug: a-z, 0-9, '-'");
        const env = envId(values.env);
        if (env !== 1 && env !== 3) throw new UsageError("init supports --env router or account (pool hooks use the pool callback ABI; token hooks are builtins)");
        const target = join(dir(values.dir), name);
        if (existsSync(target)) throw new UsageError(`${target} already exists`);
        const sdkDep = values["sdk-path"]
          ? `{ path = ${JSON.stringify(resolve(cwd, values["sdk-path"]))} }`
          : `{ git = "https://github.com/Pumphooks/pumphooks", package = "hook-sdk" }`;
        for (const [rel, text] of Object.entries(scaffold(name, env, sdkDep))) {
          mkdirSync(dirname(join(target, rel)), { recursive: true });
          writeFileSync(join(target, rel), text);
        }
        deps.out(`created ${target}\n  cd ${name} && pumphooks test && pumphooks build`);
        return 0;
      }

      case "build": {
        const d = dir(a1);
        return spawn("cargo", ["build-sbf", "--manifest-path", join(d, "Cargo.toml"), "--sbf-out-dir", join(d, "target", "deploy")]);
      }

      case "test":
        return spawn("cargo", ["test", "--manifest-path", join(dir(a1), "Cargo.toml")]);

      case "simulate": {
        if (values.stack) {
          const bytes = readStack(values.stack, cwd);
          const benchPath = values.bench ?? (findRepoRoot(cwd) ? join(findRepoRoot(cwd)!, "bench", "results.json") : undefined);
          const measured = benchPath && existsSync(benchPath) ? measuredFromBench(JSON.parse(readFileSync(benchPath, "utf8"))) : undefined;
          const a = analyze(bytes, { members: values.members ? Number(values.members) : undefined, measuredCu: measured });
          if (values.json) deps.out(JSON.stringify(a, (_k, v) => (typeof v === "bigint" ? v.toString() : v), 2));
          else {
            deps.out(`environment   ${a.environment}\nkernel        ${a.kernel}\ncompute       estimate ${a.compute.estimate}${a.compute.measured !== undefined ? `, measured ${a.compute.measured}` : ""}`);
            for (const d of a.diagnostics) deps.out(`${d.severity.padEnd(8)} ${d.code}: ${d.message}`);
          }
          return a.valid ? 0 : 1;
        }
        return await simulateProgram(values, cwd, deps);
      }

      case "bench": {
        const root = findRepoRoot(cwd);
        if (!root) throw new UsageError("bench runs inside the pumphooks repository (bench/)");
        if (!values["no-run"]) {
          const code = spawn("cargo", ["run", "--release", "-p", "hook-bench"], root);
          if (code !== 0) return code;
        }
        const baseline = resolve(cwd, values.baseline ?? join(root, "bench", "baseline.json"));
        const diffs = compare(
          parseResults(readFileSync(baseline, "utf8")),
          parseResults(readFileSync(join(root, "bench", "results.json"), "utf8")),
          values.tolerance ? Number(values.tolerance) : 5,
        );
        deps.out(renderDiff(diffs));
        return hasFailures(diffs) ? 1 : 0;
      }

      case "security-check":
        return await securityCheck(values, cwd, deps);

      case "deploy": {
        const url = clusterUrl(values.cluster);
        const so = resolve(cwd, need(values.program, "--program <file.so>"));
        const keypair = resolve(cwd, need(values.keypair, "--keypair <file>"));
        if (!existsSync(so)) throw new UsageError(`${so} not found (pumphooks build first)`);
        const args = ["program", "deploy", so, "--url", url, "--keypair", keypair, "--upgrade-authority", keypair];
        if (values["program-id"]) args.push("--program-id", resolve(cwd, values["program-id"]));
        deps.out(`solana ${args.join(" ")}`);
        if (!values.yes) {
          deps.out("(not run; pass --yes to deploy)");
          return 0;
        }
        return spawn("solana", args);
      }

      case "register": {
        const url = clusterUrl(values.cluster);
        const manifest = JSON.parse(readFileSync(resolve(cwd, need(values.manifest, "--manifest hook.json")), "utf8")) as HookManifest;
        const kp = readKeypair(resolve(cwd, need(values.keypair, "--keypair <file>")));
        const program = new PublicKey(need(values.program, "--program <program id>")).toBase58();
        const artifact = values.so ? new Uint8Array(readFileSync(resolve(cwd, values.so))) : undefined;
        const conn = deps.connection ? deps.connection(url) : new Connection(url, "confirmed");
        if (!(await conn.getAccountInfo(new PublicKey(v1.REGISTRY_PROGRAM_ID)))) {
          deps.out(`hook-registry (${v1.REGISTRY_PROGRAM_ID}) is not deployed on ${values.cluster}`);
          return 1;
        }
        const existingInfo = await conn.getAccountInfo(new PublicKey(v1.registryPda.hook(manifest.slug)));
        const existing = existingInfo ? v1.decodeHook(new Uint8Array(existingInfo.data)) : null;
        const r = registerInstructions({ manifest, publisher: kp.publicKey.toBase58(), program, existing, artifact });
        deps.out(`hook ${manifest.slug} (${r.hook}) version #${r.index} → ${r.version}`);
        const { blockhash, lastValidBlockHeight } = await conn.getLatestBlockhash();
        const tx = new Transaction({ feePayer: kp.publicKey, blockhash, lastValidBlockHeight }).add(...r.instructions.map(v1.toWeb3));
        tx.sign(kp);
        if (!values.yes) {
          const sim = await conn.simulateTransaction(tx);
          deps.out(`(simulated, not sent) ${sim.value.err ? `failed: ${JSON.stringify(sim.value.err)}` : "ok"}`);
          for (const l of sim.value.logs ?? []) deps.out(`  ${l}`);
          return sim.value.err ? 1 : 0;
        }
        const sig = await conn.sendRawTransaction(tx.serialize());
        await conn.confirmTransaction({ signature: sig, blockhash, lastValidBlockHeight }, "confirmed");
        deps.out(`signature ${sig}`);
        return 0;
      }

      default:
        throw new UsageError(`unknown command ${cmd ?? ""}`);
    }
  } catch (e) {
    deps.out(e instanceof UsageError ? `${e.message}\n\n${DEV_USAGE}` : `error: ${(e as Error).message}`);
    return e instanceof UsageError ? 2 : 1;
  }
}

type Values = Record<string, string | boolean | undefined>;

async function simulateProgram(values: Values, cwd: string, deps: DevDeps): Promise<number> {
  const { ExecutionLab } = await import("@pump-hooks/execution-lab");
  const so = resolve(cwd, String(values.program));
  const env = envId(values.env as string | undefined);
  const callback = callbackId(env, (values.callback as string | undefined) ?? v1.CALLBACK_NAMES[env][env === 1 ? 2 : 0]);
  const envProgram = v1.ENVIRONMENT_PROGRAMS[env];
  if (!envProgram) throw new UsageError(`${v1.ENVIRONMENT_NAMES[env]} does not call program hooks with policy calls`);

  const lab = new ExecutionLab({ sigverify: false });
  const hook = Keypair.generate().publicKey.toBase58();
  const payer = Keypair.generate().publicKey.toBase58();
  const subject = Keypair.generate().publicKey.toBase58();
  lab.addProgram({ id: hook, path: so, name: "hook" });
  lab.airdrop(payer, 1_000_000_000n);
  lab.setAccount({ address: subject, lamports: 1_000_000_000n, owner: envProgram, executable: false, data: new Uint8Array(64) });
  const forwarded = values.accounts ? lab.loadAccountDir(resolve(cwd, String(values.accounts))).map((a) => ({ address: a.address, writable: a.owner === hook })) : [];
  const slot = values.slot ? BigInt(String(values.slot)) : 1000n;
  lab.setClock({ slot });
  const ix = v1.policyCall({
    hook,
    context: {
      environment: env,
      callback,
      subject,
      actor: (values.actor as string | undefined) ?? payer,
      asset: values.asset as string | undefined,
      amount: values.amount ? BigInt(String(values.amount)) : 0n,
      amountB: values["amount-b"] ? BigInt(String(values["amount-b"])) : 0n,
      slot,
    },
    forwarded,
  });
  const role = (a: { signer: boolean; writable: boolean }) => (a.signer ? 2 : 0) | (a.writable ? 1 : 0);
  const trace = await lab.execute({
    payer: payer as never,
    label: "policy call",
    instructions: [{ programAddress: ix.programId as never, accounts: ix.accounts.map((a) => ({ address: a.address as never, role: role(a) })), data: ix.data } as never],
  });
  const ret = trace.returnData ? v1.decodePolicyResult(Uint8Array.from(Buffer.from(trace.returnData.data, "base64"))) : null;
  const summary = {
    status: trace.status,
    error: trace.error,
    environment: v1.ENVIRONMENT_NAMES[env],
    callback: v1.CALLBACK_NAMES[env][callback],
    computeUnits: trace.computeUnits.total.toString(),
    result: ret ? { ...ret, outputValue: ret.outputValue.toString() } : null,
    note: "simulated in LiteSVM with signature verification off: the environment's callback-authority signature (invoke_signed on chain) is emulated",
    logs: trace.logs,
  };
  if (values.json) deps.out(JSON.stringify(summary, null, 2));
  else {
    deps.out(`${summary.environment}.${summary.callback}  ${trace.status}${trace.error ? ` (${trace.error})` : ""}  ${summary.computeUnits} CU`);
    deps.out(ret ? `decision      ${ret.decision}${ret.decision === "reject" ? ` reason ${ret.reason}` : ""}${ret.outputKind ? ` output ${ret.outputKind}=${ret.outputValue}` : ""}` : "decision      none (no valid PolicyResult; environments treat this as a rejection)");
    deps.out(`note          ${summary.note}`);
    for (const l of trace.logs) deps.out(`  ${l}`);
  }
  return trace.status === "success" && ret !== null ? 0 : 1;
}

async function securityCheck(values: Values, cwd: string, deps: DevDeps): Promise<number> {
  const { CATEGORIES, fuzz, renderReport, wrapperTarget, DISCLAIMER } = await import("@pump-hooks/hook-security");
  let failed = false;
  const parts: string[] = [];
  if (values.stack) {
    const a = analyze(readStack(String(values.stack), cwd));
    parts.push(`## Stack\n\nkernel: ${a.kernel}\n\n${a.diagnostics.map((d) => `- ${d.severity} ${d.code}: ${d.message}`).join("\n") || "- no findings"}\n`);
    failed ||= !a.valid || a.diagnostics.some((d) => d.severity === "error");
  }
  const root = findRepoRoot(cwd);
  const deployDir = resolve(cwd, String(values["deploy-dir"] ?? (root ? join(root, "target", "deploy") : "target/deploy")));
  const reports = [];
  if (existsSync(join(deployDir, "token_wrapper.so")) && existsSync(join(deployDir, "token_hooks.so"))) {
    const r = await fuzz(wrapperTarget(deployDir), { seed: 1, runs: values.runs ? Number(values.runs) : 4, steps: 40 });
    reports.push(r);
    failed ||= !!r.failure;
  } else {
    parts.push(`(fuzzing skipped: build token_wrapper.so and token_hooks.so into ${deployDir})\n`);
  }
  const report = `${renderReport({ catalog: CATEGORIES, fuzz: reports })}\n${parts.join("\n")}`;
  if (values.out) writeFileSync(resolve(cwd, String(values.out)), report);
  deps.out(values.out ? `wrote ${values.out}\n${DISCLAIMER}` : report);
  return failed ? 1 : 0;
}
