import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { Keypair } from "@solana/web3.js";
import { v1 } from "@pump-hooks/sdk";
import { ExecutionLab } from "@pump-hooks/execution-lab";
import { HookManifest, isDevCommand, registerInstructions, runDev, scaffold } from "../src/dev.js";

const ROOT = resolve(__dirname, "../../..");
const DEPLOY = join(ROOT, "target", "deploy");
const SBF = process.env.PUMPHOOKS_SBF === "1" || (existsSync(join(DEPLOY, "hook_registry.so")) && existsSync(join(DEPLOY, "anti_jit_hook.so")));

function cli(argv: string[], extra: Partial<Parameters<typeof runDev>[1]> = {}) {
  let out = "";
  const calls: { cmd: string; args: string[]; cwd?: string }[] = [];
  const p = runDev(argv, {
    out: (s) => (out += s + "\n"),
    spawn: (cmd, args, cwd) => (calls.push({ cmd, args, cwd }), 0),
    cwd: ROOT,
    ...extra,
  });
  return p.then((code) => ({ code, out, calls }));
}

describe("routing", () => {
  it("dev commands vs market commands", () => {
    expect(isDevCommand(["init", "x"])).toBe(true);
    expect(isDevCommand(["security-check"])).toBe(true);
    expect(isDevCommand(["demo", "pool"])).toBe(true);
    expect(isDevCommand(["simulate", "--program", "a.so"])).toBe(true);
    expect(isDevCommand(["simulate", "MARKET", "--buy", "1"])).toBe(false);
    expect(isDevCommand(["inspect", "MINT"])).toBe(false);
  });
});

describe("init", () => {
  it("scaffolds a standalone crate and refuses to overwrite", async () => {
    const parent = mkdtempSync(join(tmpdir(), "pumphooks-init-"));
    const r = await cli(["init", "max-in", "--dir", parent, "--sdk-path", "crates/hook-sdk"]);
    expect(r.code).toBe(0);
    const cargo = readFileSync(join(parent, "max-in", "Cargo.toml"), "utf8");
    expect(cargo).toContain(`hook-sdk = { path = ${JSON.stringify(join(ROOT, "crates/hook-sdk"))} }`);
    expect(cargo).toContain("[workspace]");
    const manifest = JSON.parse(readFileSync(join(parent, "max-in", "hook.json"), "utf8")) as HookManifest;
    expect(manifest).toMatchObject({ slug: "max-in", environment: "router", callbacks: ["before_swap"] });
    expect((await cli(["init", "max-in", "--dir", parent])).code).toBe(2);
    expect((await cli(["init", "Bad_Name", "--dir", parent])).code).toBe(2);
    expect((await cli(["init", "p", "--env", "pool", "--dir", parent])).code).toBe(2);
  });

  it("account template targets smart-accounts", () => {
    const files = scaffold("acct", 3, '{ path = "x" }');
    expect(files["src/lib.rs"]).toContain("Environment::Account");
    expect(files["src/lib.rs"]).toContain("SMART_ACCOUNTS_PROGRAM_ID");
  });

  it.skipIf(process.env.PUMPHOOKS_SBF !== "1")("the scaffold's own tests pass with cargo", () => {
    const parent = mkdtempSync(join(tmpdir(), "pumphooks-cargo-"));
    return cli(["init", "scaffold-check", "--dir", parent, "--sdk-path", "crates/hook-sdk"]).then(() => {
      const r = spawnSync("cargo", ["test", "--manifest-path", join(parent, "scaffold-check", "Cargo.toml")], { encoding: "utf8" });
      expect(r.status, r.stderr).toBe(0);
      expect(r.stdout).toContain("allows_up_to_the_limit_and_rejects_above ... ok");
    });
  }, 600_000);
});

describe("build / test / bench wrap cargo", () => {
  it("build and test", async () => {
    const b = await cli(["build", "examples/anti-jit-hook"]);
    expect(b.calls[0]).toEqual({ cmd: "cargo", args: ["build-sbf", "--manifest-path", join(ROOT, "examples/anti-jit-hook/Cargo.toml"), "--sbf-out-dir", join(ROOT, "examples/anti-jit-hook/target/deploy")], cwd: undefined });
    const t = await cli(["test", "examples/anti-jit-hook"]);
    expect(t.calls[0].args).toEqual(["test", "--manifest-path", join(ROOT, "examples/anti-jit-hook/Cargo.toml")]);
  });

  it("bench compares the recorded results with the baseline", async () => {
    const r = await cli(["bench", "--no-run"]);
    expect(r.calls).toEqual([]);
    expect(r.out).toContain("execute: empty stack");
    expect(r.code).toBe(0);
    const run = await cli(["bench"]);
    expect(run.calls[0]).toEqual({ cmd: "cargo", args: ["run", "--release", "-p", "hook-bench"], cwd: ROOT });
  });
});

describe("deploy", () => {
  it("refuses mainnet and unknown clusters", async () => {
    for (const c of ["mainnet", "mainnet-beta", "testnet"]) {
      const r = await cli(["deploy", "--cluster", c, "--program", "x.so", "--keypair", "k.json", "--yes"]);
      expect(r.code).toBe(2);
      expect(r.calls).toEqual([]);
    }
    expect((await cli(["deploy", "--cluster", "mainnet", "--program", "x.so", "--keypair", "k"])).out).toContain("mainnet is refused");
  });

  it("prints the exact command and only runs it with --yes", async () => {
    const so = "target/deploy/anti_jit_hook.so";
    if (!existsSync(join(ROOT, so))) return;
    const args = ["deploy", "--cluster", "devnet", "--program", so, "--keypair", ".keys/devnet-deployer.json"];
    const dry = await cli(args);
    expect(dry.code).toBe(0);
    expect(dry.calls).toEqual([]);
    expect(dry.out).toContain("--url https://api.devnet.solana.com");
    const go = await cli([...args, "--yes"]);
    expect(go.calls[0].cmd).toBe("solana");
    expect(go.calls[0].args).toEqual(["program", "deploy", join(ROOT, so), "--url", "https://api.devnet.solana.com", "--keypair", join(ROOT, ".keys/devnet-deployer.json"), "--upgrade-authority", join(ROOT, ".keys/devnet-deployer.json")]);
  });
});

const MANIFEST: HookManifest = {
  slug: "anti-jit",
  version: "1.2.3",
  environment: "router",
  callbacks: ["before_swap"],
  capabilities: ["VALIDATE_OPERATION", "VETO_OPERATION", "UPDATE_OWN_STATE"],
  manifestUri: "https://example.invalid/anti-jit.json",
};

describe("register", () => {
  it("rejects manifests the registry would refuse", () => {
    const base = { publisher: Keypair.generate().publicKey.toBase58(), program: Keypair.generate().publicKey.toBase58(), existing: null };
    expect(() => registerInstructions({ ...base, manifest: { ...MANIFEST, capabilities: ["REQUEST_AUTHORIZED_TRANSFER"] } })).toThrow(/policy capabilities/);
    expect(() => registerInstructions({ ...base, manifest: { ...MANIFEST, callbacks: ["before_transfer"] } })).toThrow(/unknown callback/);
    expect(() => registerInstructions({ ...base, manifest: { ...MANIFEST, version: "1.2" } })).toThrow(/version/);
    const other = { authority: Keypair.generate().publicKey.toBase58(), slug: "anti-jit", versionCount: 1, status: 0 };
    expect(() => registerInstructions({ ...base, manifest: MANIFEST, existing: other })).toThrow(/belongs to/);
  });

  it("refuses mainnet and reports a cluster without the registry", async () => {
    const kpPath = join(mkdtempSync(join(tmpdir(), "pumphooks-kp-")), "k.json");
    writeFileSync(kpPath, JSON.stringify(Array.from(Keypair.generate().secretKey)));
    const m = join(ROOT, "examples/anti-jit-hook/hook.json");
    const program = Keypair.generate().publicKey.toBase58();
    expect((await cli(["register", "--cluster", "mainnet", "--manifest", m, "--keypair", kpPath, "--program", program])).code).toBe(2);
    const absent = await cli(["register", "--cluster", "devnet", "--manifest", m, "--keypair", kpPath, "--program", program], {
      connection: () => ({ getAccountInfo: async () => null }) as never,
    });
    expect(absent.code).toBe(1);
    expect(absent.out).toContain("is not deployed on devnet");
  });

  it.skipIf(!SBF)("create_hook + publish_version run against the real registry program", async () => {
    const lab = new ExecutionLab({ sigverify: false });
    const admin = Keypair.generate().publicKey.toBase58();
    const dev = Keypair.generate().publicKey.toBase58();
    const hookProgram = Keypair.generate().publicKey.toBase58();
    lab.addProgram({ id: v1.REGISTRY_PROGRAM_ID, path: join(DEPLOY, "hook_registry.so") });
    lab.addProgram({ id: hookProgram, path: join(DEPLOY, "anti_jit_hook.so") });
    const setAuthority = (program: string, authority: string) => {
      const pd = v1.programData(program);
      const a = lab.getAccount(pd)!;
      const data = new Uint8Array(a.data);
      data[12] = 1;
      data.set(v1.bytes32(authority), 13);
      lab.setAccount({ ...a, data });
    };
    setAuthority(v1.REGISTRY_PROGRAM_ID, admin);
    setAuthority(hookProgram, dev);
    lab.airdrop(admin, 10_000_000_000n);
    lab.airdrop(dev, 10_000_000_000n);
    const kit = (ix: v1.InstructionV1) =>
      ({ programAddress: ix.programId, accounts: ix.accounts.map((a) => ({ address: a.address, role: (a.signer ? 2 : 0) | (a.writable ? 1 : 0) })), data: ix.data }) as never;
    const init: v1.InstructionV1 = {
      programId: v1.REGISTRY_PROGRAM_ID,
      accounts: [v1.meta(admin, true, true), v1.meta(v1.registryPda.config(), true), v1.meta(v1.REGISTRY_PROGRAM_ID), v1.meta(v1.programData(v1.REGISTRY_PROGRAM_ID)), v1.meta("11111111111111111111111111111111")],
      data: v1.anchorDiscriminator("initialize"),
    };
    expect((await lab.execute({ payer: admin as never, instructions: [kit(init)] })).status).toBe("success");

    const so = new Uint8Array(readFileSync(join(DEPLOY, "anti_jit_hook.so")));
    const first = registerInstructions({ manifest: MANIFEST, publisher: dev, program: hookProgram, existing: null, artifact: so });
    expect(first.instructions).toHaveLength(2);
    const t1 = await lab.execute({ payer: dev as never, instructions: first.instructions.map(kit) });
    expect(t1.status, t1.logs.join("\n")).toBe("success");

    const hook = v1.decodeHook(lab.getAccount(first.hook)!.data);
    expect(hook).toMatchObject({ authority: dev, slug: "anti-jit", versionCount: 1 });
    const ver = v1.decodeHookVersion(lab.getAccount(first.version)!.data);
    expect(ver).toMatchObject({ hook: first.hook, index: 0, semver: [1, 2, 3], implKind: v1.IMPL_PROGRAM, program: hookProgram, environments: 0b10, publisher: dev });
    expect(ver.callbacks).toEqual([0, 1 << 2, 0, 0]);
    expect(ver.capabilities).toBe(v1.Cap.VALIDATE_OPERATION | v1.Cap.VETO_OPERATION | v1.Cap.UPDATE_OWN_STATE);
    expect(ver.manifestUri).toBe(MANIFEST.manifestUri);
    expect(Buffer.from(ver.artifactHash).toString("hex")).toBe(spawnSync("shasum", ["-a", "256", join(DEPLOY, "anti_jit_hook.so")], { encoding: "utf8" }).stdout.split(" ")[0]);

    // Next version: no create_hook, next index.
    const second = registerInstructions({ manifest: { ...MANIFEST, version: "1.2.4" }, publisher: dev, program: hookProgram, existing: hook });
    expect(second.instructions).toHaveLength(1);
    expect(second.index).toBe(1);
    expect((await lab.execute({ payer: dev as never, instructions: second.instructions.map(kit) })).status).toBe("success");

    // Someone who is not the program's upgrade authority cannot publish it under their own hook.
    const mallory = Keypair.generate().publicKey.toBase58();
    lab.airdrop(mallory, 10_000_000_000n);
    const forged = registerInstructions({ manifest: { ...MANIFEST, slug: "not-mine" }, publisher: mallory, program: hookProgram, existing: null });
    expect((await lab.execute({ payer: mallory as never, instructions: forged.instructions.map(kit) })).status).toBe("failed");
  });
});

describe("simulate", () => {
  it("analyzes a stack", async () => {
    const stack = v1.encodeStack(v1.Environment.Router, [{ builtin: v1.Builtin.MaxSize, config: v1.configs.maxSize(new Uint8Array(32), 1000n), mandatory: true }]);
    const r = await cli(["simulate", "--stack", Buffer.from(stack).toString("hex")]);
    expect(r.code, r.out).toBe(0);
    expect(r.out).toContain("kernel        ok");
    const bad = new Uint8Array(stack);
    bad[8 + 64] = 9;
    expect((await cli(["simulate", "--stack", Buffer.from(bad).toString("hex")])).code).toBe(1);
  });

  it.skipIf(!SBF || !existsSync(join(DEPLOY, "hooks_core.so")))("demo runs an engine demo against the compiled programs", async () => {
    const r = await cli(["demo", "pool", "--json"]);
    expect(r.code, r.out).toBe(0);
    const d = JSON.parse(r.out);
    expect(d.id).toBe("pool");
    expect(d.steps.every((s: { ok: boolean }) => s.ok)).toBe(true);
    expect((await cli(["demo", "nope"])).code).toBe(2);
  });

  it.skipIf(!SBF)("runs a policy call against the example hook program", async () => {
    const so = join(DEPLOY, "anti_jit_hook.so");
    const r = await cli(["simulate", "--program", so, "--env", "router", "--callback", "before_swap", "--amount", "5", "--json"]);
    const j = JSON.parse(r.out);
    expect(j.status).toBe("success");
    // Not configured for this policy: the hook's own reason.
    expect(j.result).toMatchObject({ decision: "reject", reason: 1002 });
    expect(Number(j.computeUnits)).toBeGreaterThan(0);
    // An instruction that is not a policy call for the router is not accepted as one.
    const wrongCb = await cli(["simulate", "--program", so, "--env", "account", "--callback", "before_withdraw", "--json"]);
    expect(JSON.parse(wrongCb.out).status).toBe("failed");
  });
});
