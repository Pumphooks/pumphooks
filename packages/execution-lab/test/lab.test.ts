import { resolve } from "node:path";
import { AccountRole, generateKeyPairSigner, type KeyPairSigner } from "@solana/kit";
import { describe, expect, it } from "vitest";
import { ExecutionLab, exportRepro, renderTrace, replay, toJson, decodeStack, type ReproBundle } from "../src/index.js";
import {
  ACCOUNT_PLAIN,
  ACCOUNT_WITH_HOOK,
  createAccount,
  createPolicy,
  DEPLOY,
  EMPTY_TOKEN_STACK,
  initAccount3,
  initMint2,
  initTransferHook,
  maxSizeStack,
  MINT_PLAIN,
  MINT_WITH_HOOK,
  mintTo,
  SBF,
  SPL,
  T22,
  TOKEN_HOOKS,
  tokenHooksPdas,
  transferChecked,
} from "./helpers.js";

const SOL = 1_000_000_000n;

async function funded(lab: ExecutionLab): Promise<KeyPairSigner> {
  const k = await generateKeyPairSigner();
  lab.airdrop(k.address, 10n * SOL);
  return k;
}

describe("execution lab: SPL Token (bundled program)", () => {
  it("traces a transfer with observed balances, diffs and compute", async () => {
    const lab = new ExecutionLab();
    const payer = await funded(lab);
    const [mint, a, b] = await Promise.all([generateKeyPairSigner(), generateKeyPairSigner(), generateKeyPairSigner()]);
    const rentMint = lab.svm.minimumBalanceForRentExemption(BigInt(MINT_PLAIN));
    const rentAcct = lab.svm.minimumBalanceForRentExemption(BigInt(ACCOUNT_PLAIN));
    const setup = await lab.execute({
      payer,
      signers: [mint, a, b],
      label: "setup",
      instructions: [
        createAccount(payer.address, mint.address, rentMint, MINT_PLAIN, SPL),
        initMint2(SPL, mint.address, payer.address, 6),
        createAccount(payer.address, a.address, rentAcct, ACCOUNT_PLAIN, SPL),
        initAccount3(SPL, a.address, mint.address, payer.address),
        createAccount(payer.address, b.address, rentAcct, ACCOUNT_PLAIN, SPL),
        initAccount3(SPL, b.address, mint.address, payer.address),
        mintTo(SPL, mint.address, a.address, payer.address, 1_000n),
      ],
    });
    expect(setup.status).toBe("success");
    expect(setup.accountDiffs.filter((d) => d.created).map((d) => d.address).sort()).toEqual([mint.address, a.address, b.address].sort());

    const t = await lab.execute({ payer, label: "transfer", instructions: [transferChecked(SPL, a.address, mint.address, b.address, payer.address, 250n, 6)] });
    expect(t.status).toBe("success");
    expect(t.instructions).toHaveLength(1);
    expect(t.instructions[0].program).toBe("spl-token");
    expect(t.instructions[0].accounts.map((x) => [x.writable, x.signer])).toEqual([
      [true, false],
      [false, false],
      [true, false],
      [true, true], // fee payer is also the owner
    ]);
    expect(t.computeUnits.total).toBeGreaterThan(0n);
    expect(t.computeUnits.perInstruction[0]).toBe(Number(t.computeUnits.total));
    const byAccount = Object.fromEntries(t.tokenBalances.map((x) => [x.account, x]));
    expect(byAccount[a.address].delta).toBe(-250n);
    expect(byAccount[b.address].delta).toBe(250n);
    // Only the amount field (bytes 64..72) of each token account changed.
    const diffA = t.accountDiffs.find((d) => d.address === a.address)!;
    expect(diffA.changedRanges.every(([s, e]) => s >= 64 && e <= 72)).toBe(true);
    expect(t.cpiTree[0]).toEqual([]);
    expect(t.provenance.hookSteps.provenance).toBe("reconstructed");
    expect(t.provenance.tokenBalances.provenance).toBe("observed");

    const failed = await lab.execute({ payer, label: "overdraft", instructions: [transferChecked(SPL, a.address, mint.address, b.address, payer.address, 10_000n, 6)] });
    expect(failed.status).toBe("failed");
    expect(failed.error).toMatch(/InstructionError\(0, Custom\(1\)\)/); // InsufficientFunds
    expect(failed.invocations[0].result).toBe("failed");
    expect(failed.tokenBalances.every((x) => x.delta === 0n)).toBe(true);
    expect(renderTrace(failed)).toContain("FAILED");
  });
});

describe.skipIf(!SBF)("execution lab: token-hooks (requires scripts/build_programs.sh)", () => {
  async function hookedMint() {
    const lab = new ExecutionLab();
    lab.addProgram({ id: TOKEN_HOOKS, path: resolve(DEPLOY, "token_hooks.so"), name: "token-hooks" });
    const payer = await funded(lab);
    const [mint, a, b] = await Promise.all([generateKeyPairSigner(), generateKeyPairSigner(), generateKeyPairSigner()]);
    const rent = (n: number) => lab.svm.minimumBalanceForRentExemption(BigInt(n));
    const setup = await lab.execute({
      payer,
      signers: [mint, a, b],
      instructions: [
        createAccount(payer.address, mint.address, rent(MINT_WITH_HOOK), MINT_WITH_HOOK, T22),
        initTransferHook(T22, mint.address, payer.address, TOKEN_HOOKS),
        initMint2(T22, mint.address, payer.address, 6),
        createAccount(payer.address, a.address, rent(ACCOUNT_WITH_HOOK), ACCOUNT_WITH_HOOK, T22),
        initAccount3(T22, a.address, mint.address, payer.address),
        createAccount(payer.address, b.address, rent(ACCOUNT_WITH_HOOK), ACCOUNT_WITH_HOOK, T22),
        initAccount3(T22, b.address, mint.address, payer.address),
        mintTo(T22, mint.address, a.address, payer.address, 1_000n),
        await createPolicy(payer.address, payer.address, mint.address, EMPTY_TOKEN_STACK),
      ],
    });
    expect(setup.status, setup.logs.join("\n")).toBe("success");
    const pdas = await tokenHooksPdas(mint.address);
    // Install a MAX_SIZE(100) entry by writing the policy's stack bytes (the lab loads arbitrary state).
    const policy = lab.getAccount(pdas.policy)!;
    const data = new Uint8Array(policy.data);
    data.set(maxSizeStack(mint.address, 100n), 136);
    lab.setAccount({ ...policy, data });
    expect(decodeStack(data.slice(136)).entries.map((e) => e.name)).toEqual(["max-size"]);
    lab.label(a.address, "alice").label(b.address, "bob").label(pdas.policy, "token-policy");
    const send = (amount: bigint, label: string) =>
      lab.execute({
        payer,
        label,
        instructions: [
          transferChecked(T22, a.address, mint.address, b.address, payer.address, amount, 6, [
            { address: pdas.policy, role: AccountRole.WRITABLE },
            { address: TOKEN_HOOKS, role: AccountRole.READONLY },
            { address: pdas.metas, role: AccountRole.READONLY },
          ]),
        ],
      });
    return { lab, send, pdas };
  }

  it("shows Token-2022 calling the hook, and the hook order the kernel ran", async () => {
    const { send, pdas } = await hookedMint();
    const ok = await send(60n, "transfer 60");
    expect(ok.status, ok.logs.join("\n")).toBe("success");
    // CPI tree (observed): Token-2022 -> token-hooks Execute at stack height 2.
    expect(ok.cpiTree[0]).toHaveLength(1);
    expect(ok.cpiTree[0][0]).toMatchObject({ program: "token-hooks", stackHeight: 2 });
    expect(ok.invocations.map((i) => [i.program, i.depth, i.result])).toEqual([
      ["token-2022", 1, "success"],
      ["token-hooks", 2, "success"],
    ]);
    expect(ok.invocations[1].consumed).toBeGreaterThan(0);
    expect(ok.hookSteps).toEqual([
      expect.objectContaining({ engine: "token", callback: "before_transfer", entry: 0, name: "max-size", mandatory: true, result: "ran", policy: pdas.policy }),
    ]);
    // The policy's sequence and the balances changed; nothing else.
    const policyDiff = ok.accountDiffs.find((d) => d.address === pdas.policy)!;
    expect(policyDiff.changedRanges.length).toBeGreaterThan(0);
    expect(ok.tokenBalances.map((b) => b.delta).sort()).toEqual([-60n, 60n].sort());

    const denied = await send(150n, "transfer 150");
    expect(denied.status).toBe("failed");
    expect(denied.hookSteps).toEqual([expect.objectContaining({ name: "max-size", result: "rejected", reason: expect.stringMatching(/^\d+/) })]);
    expect(denied.invocations.find((i) => i.program === "token-hooks")?.result).toBe("failed");
    expect(renderTrace(denied)).toContain("hook order (reconstructed");
  });

  it("exports a reproduction that replays identically and detects tampering", async () => {
    const { lab, send } = await hookedMint();
    const denied = await send(150n, "transfer 150");
    const bundle = exportRepro(lab, denied, { pathRoot: resolve(DEPLOY, "../..") });
    expect(bundle.programs.map((p) => p.path)).toEqual(["target/deploy/token_hooks.so"]);
    const json = toJson(bundle);
    expect(json).not.toMatch(/secretKey|privateKey/);
    const parsed = JSON.parse(json) as ReproBundle;
    const r = await replay(parsed, { pathRoot: resolve(DEPLOY, "../..") });
    expect(r.matches).toEqual({ status: true, error: true, computeUnits: true, logs: true });
    expect(r.trace.hookSteps.map((s) => s.result)).toEqual(["rejected"]);

    await expect(replay({ ...parsed, programs: [{ ...parsed.programs[0], sha256: "00" }] }, { pathRoot: resolve(DEPLOY, "../..") })).rejects.toThrow(/does not match/);
    // An edited message (amount 150 -> 80, under the cap) succeeds, and the comparison reports it.
    const edited = structuredClone(parsed);
    edited.message.instructions[0].data = parsed.message.instructions[0].data.replace(/^.{4}/, "DFAA");
    const r2 = await replay(edited, { pathRoot: resolve(DEPLOY, "../..") });
    expect(r2.trace.status).toBe("success");
    expect(r2.identical).toBe(false);
  });
});
