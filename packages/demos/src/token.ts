import { setupWrapper } from "@pump-hooks/hook-security";
import { v1 } from "@pump-hooks/sdk";
import { publishBuiltin, rejection, Runner, setupRegistry, type DemoOptions, type DemoResult } from "./common.js";

/**
 * DEMO A, TOKEN: wrap an SPL token into its hooked Token-2022 twin, then enforce a recipient
 * allowlist on every wrapped transfer. Token-2022 calls token-hooks inside `transfer_checked`.
 */
export async function tokenDemo(o: DemoOptions): Promise<DemoResult> {
  const s = await setupWrapper(o.deployDir, 3);
  const [alice, bob, carol] = s.users;
  const r = new Runner(s.lab, s.payer);
  r.setup.push("deploy token-hooks + token-wrapper; create the wrapper for a fresh SPL mint with an empty token policy");
  s.lab.label(s.payer.address, "issuer").label(s.policy, "token-policy").label(s.vault, "vault");
  for (const [u, n] of [[alice, "alice"], [bob, "bob"], [carol, "carol"]] as const) {
    s.lab.label(u.signer.address, n).label(u.wrapped, `${n} wrapped`).label(u.underlying, `${n} underlying`);
  }

  await setupRegistry(r, o.deployDir, s.payer);
  const version = await publishBuiltin(r, s.payer, v1.Builtin.Allowlist, v1.Environment.Token);
  const stack = v1.encodeStack(v1.Environment.Token, [
    { builtin: v1.Builtin.Allowlist, callbacks: [v1.TokenCallback.BeforeTransfer], hookVersion: v1.bytes32(version), config: v1.configs.allowlistInline(v1.Target.Counterparty, [v1.bytes32(bob.signer.address)]) },
  ]);
  await r.must([v1.updateTokenPolicy({ authority: s.payer.address, mint: s.wrappedMint, stack, versions: [version] })], [s.payer], "install allowlist(recipient = bob) on the wrapped mint");

  const balance = (a: string) => v1.tokenAmount(s.lab.getAccount(a)!.data);
  const transfer = (to: typeof bob, amount: bigint) =>
    v1.transferChecked({
      source: alice.wrapped,
      mint: s.wrappedMint,
      destination: to.wrapped,
      owner: alice.signer.address,
      amount,
      decimals: s.decimals,
      extra: v1.transferHookAccounts(s.wrappedMint, v1.decodeTokenPolicy(s.lab.getAccount(s.policy)!.data).stack, { owner: alice.signer.address }, { owner: to.signer.address }),
    });

  await r.step("Alice wraps 1,000 tokens", "ok", [v1.wrap({ user: alice.signer.address, underlyingMint: s.underlyingMint, underlyingProgram: v1.SPL_TOKEN_PROGRAM_ID, userUnderlying: alice.underlying, userWrapped: alice.wrapped, amount: 1_000n, minOut: 1_000n })], [alice.signer], () => `alice holds ${balance(alice.wrapped)} wrapped; vault holds ${balance(s.vault)}`);
  await r.step("Alice sends 400 to Bob (allowlisted)", "ok", [transfer(bob, 400n)], [alice.signer], () => `bob holds ${balance(bob.wrapped)} wrapped`);
  await r.step("Alice sends 100 to Carol (not allowlisted)", "rejected", [transfer(carol, 100n)], [alice.signer], (t) => `${rejection(t)}; carol still holds ${balance(carol.wrapped)}`);
  const noHook = v1.transferChecked({ source: alice.wrapped, mint: s.wrappedMint, destination: carol.wrapped, owner: alice.signer.address, amount: 100n, decimals: s.decimals });
  await r.step("Alice sends to Carol without the hook accounts", "rejected", [noHook], [alice.signer], (t) => `Token-2022 refuses the transfer: ${rejection(t)}`);
  await r.step("Bob unwraps 400 back to the SPL token", "ok", [v1.unwrap({ user: bob.signer.address, underlyingMint: s.underlyingMint, underlyingProgram: v1.SPL_TOKEN_PROGRAM_ID, userUnderlying: bob.underlying, userWrapped: bob.wrapped, amount: 400n, minOut: 400n })], [bob.signer], () => `bob: ${balance(bob.wrapped)} wrapped, ${balance(bob.underlying) - 1_000_000n} more underlying`);

  return {
    id: "token",
    title: "Token hook: recipient allowlist on a wrapped token",
    enforcement: "Native for the wrapped Token-2022 mint: every transfer_checked runs the stack. The original SPL token is unaffected; holders opt in by wrapping.",
    setup: r.setup,
    steps: r.steps,
  };
}
