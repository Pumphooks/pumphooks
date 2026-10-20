import { join } from "node:path";
import { generateKeyPairSigner, type KeyPairSigner } from "@solana/kit";
import { ExecutionLab, type ExecutionTrace } from "@pump-hooks/execution-lab";
import { toKit } from "@pump-hooks/hook-security";
import { v1 } from "@pump-hooks/sdk";

export const SOL = 1_000_000_000n;

export type Engine = "token" | "router" | "pool" | "account";

export interface DemoStep {
  title: string;
  /** What the demo expects: the step is part of the demonstration only if it does exactly this. */
  expect: "ok" | "rejected";
  ok: boolean;
  /** One line explaining the outcome: the hook's rejection, the program error, or the measured effect. */
  outcome: string;
  trace: ExecutionTrace;
}

export interface DemoResult {
  id: Engine;
  title: string;
  /** How the engine enforces its hooks, stated plainly. */
  enforcement: string;
  setup: string[];
  steps: DemoStep[];
}

export interface DemoOptions {
  /** `target/deploy` with the compiled programs. */
  deployDir: string;
  /** `tests/fixtures` (PumpSwap binaries and the cloned devnet pool), needed by the router demo. */
  fixturesDir?: string;
}

/** Instruction runner shared by the demos: every transaction gets the full compute budget. */
export class Runner {
  readonly steps: DemoStep[] = [];
  readonly setup: string[] = [];
  constructor(
    readonly lab: ExecutionLab,
    readonly payer: KeyPairSigner,
  ) {}

  async exec(ixs: v1.InstructionV1[], signers: KeyPairSigner[], label: string): Promise<ExecutionTrace> {
    const budget: v1.InstructionV1 = { programId: "ComputeBudget111111111111111111111111111111", accounts: [], data: v1.concat(v1.u8(2), v1.u32(1_400_000)) };
    return this.lab.execute({ payer: this.payer, signers, label, instructions: [budget, ...ixs].map(toKit) });
  }

  /** Setup transaction: must succeed. */
  async must(ixs: v1.InstructionV1[], signers: KeyPairSigner[], label: string): Promise<ExecutionTrace> {
    const t = await this.exec(ixs, signers, label);
    if (t.status !== "success") throw new Error(`${label} failed: ${t.error}\n${t.logs.join("\n")}`);
    this.setup.push(label);
    return t;
  }

  /** A demonstrated step; `outcome` describes the result from the trace. */
  async step(
    title: string,
    expect: DemoStep["expect"],
    ixs: v1.InstructionV1[],
    signers: KeyPairSigner[],
    outcome: (t: ExecutionTrace) => string = (t) => (t.status === "success" ? "executed" : rejection(t)),
  ): Promise<DemoStep> {
    const trace = await this.exec(ixs, signers, title);
    const ok = (trace.status === "success") === (expect === "ok");
    const { replay: _replay, ...slim } = trace;
    const s: DemoStep = { title, expect, ok, outcome: outcome(trace), trace: slim as ExecutionTrace };
    this.steps.push(s);
    return s;
  }
}

/** The most specific reason a failed transaction gives: a hook rejection, then a program error message. */
export function rejection(t: ExecutionTrace): string {
  const find = (re: RegExp) => t.logs.find((l) => re.test(l));
  const line = find(/hook stack rejected|PolicyRejected|rejected:/) ?? find(/Error Message:/) ?? find(/failed:/);
  if (!line) return t.error ?? "failed";
  return line.replace(/^Program log: /, "").replace(/^AnchorError.*Error Message: /, "").replace(/\.$/, "");
}

export async function signer(lab: ExecutionLab, sol = 10n, name?: string): Promise<KeyPairSigner> {
  const k = await generateKeyPairSigner();
  lab.airdrop(k.address, sol * SOL);
  if (name) lab.label(k.address, name);
  return k;
}

/** Marks `authority` as the upgrade authority of an upgradeable program loaded into the lab. */
export function setUpgradeAuthority(lab: ExecutionLab, program: string, authority: string) {
  const pd = v1.programData(program);
  const a = lab.getAccount(pd);
  if (!a) throw new Error(`${program} has no ProgramData`);
  const data = new Uint8Array(a.data);
  data[12] = 1;
  data.set(v1.bytes32(authority), 13);
  lab.setAccount({ ...a, data });
}

/** Deploys and initializes the registry with `admin` as its upgrade authority. */
export async function setupRegistry(r: Runner, deployDir: string, admin: KeyPairSigner) {
  r.lab.addProgram({ id: v1.REGISTRY_PROGRAM_ID, path: join(deployDir, "hook_registry.so"), name: "hook-registry" });
  setUpgradeAuthority(r.lab, v1.REGISTRY_PROGRAM_ID, admin.address);
  const init: v1.InstructionV1 = {
    programId: v1.REGISTRY_PROGRAM_ID,
    accounts: [v1.meta(admin.address, true, true), v1.meta(v1.registryPda.config(), true), v1.meta(v1.REGISTRY_PROGRAM_ID), v1.meta(v1.programData(v1.REGISTRY_PROGRAM_ID)), v1.meta(v1.SYSTEM_PROGRAM_ID)],
    data: v1.anchorDiscriminator("initialize"),
  };
  await r.must([init], [admin], "initialize hook-registry");
}

/** `create_hook` + `publish_version` (index 0) for builtin `id` in one environment. */
export function publishBuiltinIxs(admin: string, id: number, env: v1.EnvironmentId): { ixs: v1.InstructionV1[]; hook: string; version: string } {
  const slug = v1.BUILTIN_SLUGS[id];
  const hook = v1.registryPda.hook(slug);
  const callbacks: [number, number, number, number] = [0, 0, 0, 0];
  callbacks[env] = v1.supportedCallbacks(id, env);
  const params: v1.PublishParams = {
    semver: [1, 0, 0],
    implKind: v1.IMPL_BUILTIN,
    builtinId: id,
    environments: 1 << env,
    callbacks,
    capabilities: v1.requiredCaps(id),
    configSchemaHash: new Uint8Array(32),
    manifestHash: new Uint8Array(32),
    manifestUri: `builtin:${slug}`,
    artifactHash: new Uint8Array(32),
  };
  return { ixs: [v1.createHook(admin, slug), v1.publishVersion({ publisher: admin, hook, index: 0, params })], hook, version: v1.registryPda.version(hook, 0) };
}

/** Publishes builtin `id` for one environment and returns its `HookVersion` address. */
export async function publishBuiltin(r: Runner, admin: KeyPairSigner, id: number, env: v1.EnvironmentId): Promise<string> {
  const slug = v1.BUILTIN_SLUGS[id];
  const { ixs, version } = publishBuiltinIxs(admin.address, id, env);
  await r.must(ixs, [admin], `publish builtin ${slug} (${v1.ENVIRONMENT_NAMES[env]})`);
  r.lab.label(version, `${slug} v1.0.0`);
  return version;
}

export const fmtSol = (l: bigint) => `${(Number(l) / 1e9).toFixed(4)} SOL`;

/** Classic SPL Token mint / account creation, as explicit instructions. */
export const spl = {
  async mint(r: Runner, authority: KeyPairSigner, decimals = 6): Promise<string> {
    const m = await generateKeyPairSigner();
    const rent = r.lab.svm.minimumBalanceForRentExemption(82n);
    await r.must(
      [
        { programId: v1.SYSTEM_PROGRAM_ID, accounts: [v1.meta(r.payer.address, true, true), v1.meta(m.address, true, true)], data: v1.concat(v1.u32(0), v1.u64(rent), v1.u64(82n), v1.bytes32(v1.SPL_TOKEN_PROGRAM_ID)) },
        { programId: v1.SPL_TOKEN_PROGRAM_ID, accounts: [v1.meta(m.address, true)], data: v1.concat(v1.u8(20), v1.u8(decimals), v1.bytes32(authority.address), v1.u8(0)) },
      ],
      [m],
      "create SPL mint",
    );
    return m.address;
  },
  async account(r: Runner, mint: string, owner: string, label?: string): Promise<string> {
    const a = await generateKeyPairSigner();
    const rent = r.lab.svm.minimumBalanceForRentExemption(165n);
    await r.must(
      [
        { programId: v1.SYSTEM_PROGRAM_ID, accounts: [v1.meta(r.payer.address, true, true), v1.meta(a.address, true, true)], data: v1.concat(v1.u32(0), v1.u64(rent), v1.u64(165n), v1.bytes32(v1.SPL_TOKEN_PROGRAM_ID)) },
        { programId: v1.SPL_TOKEN_PROGRAM_ID, accounts: [v1.meta(a.address, true), v1.meta(mint)], data: v1.concat(v1.u8(18), v1.bytes32(owner)) },
      ],
      [a],
      label ?? "create token account",
    );
    if (label) r.lab.label(a.address, label);
    return a.address;
  },
  mintTo: (mint: string, to: string, authority: string, amount: bigint): v1.InstructionV1 => ({
    programId: v1.SPL_TOKEN_PROGRAM_ID,
    accounts: [v1.meta(mint, true), v1.meta(to, true), v1.meta(authority, false, true)],
    data: v1.concat(v1.u8(7), v1.u64(amount)),
  }),
};
