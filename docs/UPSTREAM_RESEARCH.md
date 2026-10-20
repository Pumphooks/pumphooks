# Upstream research

Source-level study, 2026-10-20. Every claim below was checked in source, tests or deployed binaries,
not READMEs. Licenses are in [UPSTREAM_LICENSES.md](UPSTREAM_LICENSES.md); the resulting plan is in
[INTEGRATION_PLAN.md](INTEGRATION_PLAN.md).

## 1. What Pump Hooks already has

| Component | Path | What it is |
| --- | --- | --- |
| ABI v1 | `crates/hook-standard` | Four environments (token, router, pool, account) with fixed callback ids; 216-byte `StackEntry`, 8-entry `HookStack` with a compact wire form; capability bits; `PolicyContext`/`PolicyResult` for program hooks; namespaced discriminators; enforcement classes (native / program / router / wrapper / unavailable); typed cross-environment state records (`state.rs`: owner, schema, namespace, subject, freshness, sequence). |
| Kernel | `crates/hook-kernel` | One evaluator for every environment: `validate_stack`, monotone fail-closed `evaluate`, `carry_state`, `requires_guard` (mandatory entries), `compute_estimate`, `plan_hash`; 8 builtins (allowlist, spending-limit, timelock, multisig, conditional, max-size, time-window, accounting); the CPI invoker for program hooks. |
| Registry | `programs/hook-registry` | Hooks (slugs, authorities), versions (builtin or program, environment bits, callbacks per environment, capabilities, deploy-slot observation), reviews (community / tested / audited), `verify_install`. |
| Router (transaction engine) | `programs/hook-router` | Route policies (private by default, mandatory entries with delay) around an allowlisted venue CPI; measures token and SOL deltas; receipts with nonce; venues PumpSwap, Raydium CPMM (mainnet, devnet), Meteora DAMM v2, all verified against real binaries. |
| Pool engine | `programs/hooks-core` | Constant-product pools with one program hook (Uniswap-v4-style callbacks, deltas, dynamic fee, custom curve, hook credit) plus a builtin policy stack on the shared kernel. Withdrawals can never be gated. |
| Account engine | `programs/smart-accounts` | Members, proposals with timelocks, approvals, sessions, locks, recovery; deposits, withdrawals, executes and authority changes run through the kernel. |
| Pump router (older, v0) | `programs/pump-hooks` + `crates/hook-runtime`, `policy-vm` | Hooks around Pump curve and PumpSwap trades. The TypeScript SDK, CLI, indexer and web app target this program. |
| Test hook | `programs/test-hook` | Scriptable hostile pool hook for tests. |
| TS | `packages/sdk`, `packages/cli`, `services/indexer`, `apps/web` | Target the v0 router only. |
| Token environment | — | Declared in the ABI (`before_transfer` native, `before_wrap`/`before_unwrap` wrapper) and in builtin tables, but **no program existed** before this work. |

## 2. Patcha (`patcha-fi/patcha`, MIT, single commit 2026-06-13)

What the source actually contains:

- An Anchor program `patcha-hook-executor` that stores installations per (pool, slug) and exposes
  `trigger_hook(callback, amount_in, tick)`, called by anyone. **No AMM calls it**; Orca and
  Raydium are tagged by a `dex` byte only. Outcomes are events; a fee "override" is emitted, not
  applied.
- Six builtins in `crates/hook-runtime/src/builtin/`:
  - `dynamic_fee`: `fee = base + (max − base)·min(amount_in, pivot)/pivot` (defaults 30, 100 bps,
    pivot 1e9). No volatility input.
  - `time_lock`: deny add and remove liquidity before an absolute timestamp.
  - `whitelist_gate`: linear scan of a list; the stored Merkle root is never verified.
  - `range_order`: after-swap reason code when a tick crosses a target. Never vetoes, never
    moves liquidity.
  - `anti_mev`: `bps = amount_in·10_000/reference_depth`, deny above a cap. A size check against a
    configured constant, not pool state, and not MEV detection.
  - `kyc_gate`: membership in a list; the attestation authority is never checked.
- README-only: price-impact cap, JIT defense, Meteora adapter, Merkle proofs, burn-to-install.
- Tests: ~41 Rust unit tests of the formulas. No program or integration tests.

Usable: the idea list and the size-ramped fee formula. Not usable: the trigger model (it does not
enforce anything), the list-based "KYC", the "MEV" check.

## 3. LiteSVM (Apache-2.0)

- Rust 0.16 (our pin, Agave 4.2): `TransactionMetadata { logs, inner_instructions,
  compute_units_consumed, return_data, fee }`; `simulate_transaction` adds `post_accounts`. No
  pre-accounts: diffs come from reading accounts before and after. Feature flags add an
  invocation-inspect callback and register tracing.
- npm `litesvm` 1.6.1 (Node bindings): `logs()`, `innerInstructions()` with stack heights,
  `computeUnitsConsumed()`, `returnData()`; `simulateTransaction().postAccounts()`. No fee field,
  no tracing hooks. Our packages currently pin npm 0.3.3.
- Consequence for the execution lab: program invocations, CPI nesting (stack height), compute,
  logs, return data and errors are **observed**; hook invocation order is **reconstructed** from
  logs and inner instructions; account diffs are **observed** by snapshotting the accounts the
  transaction names.

## 4. Mollusk (Apache-2.0)

- Single-instruction harness. `MolluskComputeUnitBencher` writes Markdown tables
  (`| Name | CUs | Delta |`) with the delta against the previous run. No JSON output.
- Version compatibility: 0.16.x needs Agave 4.3 and conflicts with litesvm 0.16;
  **0.15.1 (Agave 4.2) resolves with our lockfile.**

## 5. Trident (MIT)

- 0.12 (stable) runs its own in-process SVM on **Agave 2.x**; honggfuzz/AFL were removed.
  Flows are picked by a seeded RNG; invariants are assertions; supports Anchor ≥ 0.29.
- It must live in its own Cargo workspace because our workspace is on Agave 4.x.

## 6. Token-2022 transfer hooks

- `Execute` discriminator `sha256("spl-transfer-hook-interface:execute")[..8]` =
  `[105,37,101,197,75,251,102,26]`, data `amount: u64`. Accounts: source, mint, destination,
  owner, validation PDA `["extra-account-metas", mint]`, then the resolved extras.
- Token-2022 sets `transferring` on both token accounts during the CPI, so a hook can reject
  direct calls by checking that flag.
- Extra metas are resolved by Token-2022 and clients from a TLV list; seeds may reference earlier
  account keys or slices of their data.
- Our lockfile carries `spl-transfer-hook-interface` 0.9 / `spl-tlv-account-resolution` 0.9 (via
  `anchor-spl` 0.31).
- A hook only applies to mints created with the TransferHook extension. **Existing SPL tokens
  cannot gain one**; the only route is a wrapper mint.
- Civic's hook (no license) checks a gateway token for the **destination token account**. The
  gateway-token layout is public in `solana-gateway` 0.5.0 (Apache-2.0).

## 7. Smart accounts (Squads, AGPL-3.0)

Studied for feature coverage only: spending limits, time locks, batch execution, sessions
(Squads v4 "spending limits", smart-account "settings transactions"). Our `smart-accounts` already
covers spending limits, timelocks, multisig, sessions, recovery and locks. Nothing is copied.

## 8. DEX venues

Verified facts (program source, IDL, and for CPMM/DAMM v2 the deployed binary):

| Venue | Program | Swap | User signer / input / output | CPI | Transfer-hook mints |
| --- | --- | --- | --- | --- | --- |
| PumpSwap | `pAMMBay6…` | `buy_v2`, `buy_exact_quote_in_v2`, `sell_v2` | 1 / 6 or 5 / 5 or 6 | yes (verified) | no |
| Raydium CPMM | `CPMMoo8L…`, devnet `DRaycpLY…` | `swap_base_input`, `swap_base_output` | 0 / 4 / 5 | yes (verified) | no |
| Meteora DAMM v2 | `cpamdpZC…` | `swap2`, `swap` | 8 / 2 / 3 | yes (verified), except B→A at stack height > 2 during the rate-limiter window | no |
| Raydium CLMM | `CAMMCzo5…`, devnet `DRayAUgE…` | `swap_v2` | 0 / 3 / 4, + bitmap extension and tick arrays | no guard in source | no |
| Orca Whirlpools | `whirLbMi…` | `swap_v2` | 3 / 7 or 9 / 9 or 7 (by `a_to_b`), + tick arrays, oracle | no guard in source | yes (TokenBadge) |
| Meteora DLMM | `LBUZKhRx…` | `swap2` | 10 / 4 / 5, + bin arrays | closed source; IDL has `CpiDisabled` (6100), trigger unknown | yes |

Details are in [research/amm-venues.md](research/amm-venues.md).

## 9. What none of the upstreams provide

- Native AMM callbacks into third-party hooks on Solana. Raydium, Orca, Meteora and PumpSwap do not
  call external hooks. Pump Hooks' router hooks remain routed (bypassable) hooks.
- A transaction compiler that understands hook capabilities.
- A composition analyzer across independently developed hooks.
