# Integration plan

How the upstream research ([UPSTREAM_RESEARCH.md](UPSTREAM_RESEARCH.md)) lands in Pump Hooks
without a second kernel, registry or ABI. Status of each item is tracked in
[IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md).

## Principles

- Every policy runs on `crates/hook-kernel` and installs from `programs/hook-registry`. New hook
  kinds are new builtin ids or program hooks on ABI v1; nothing forks the stack format.
- ABI v1 is append-only. New builtin ids, output kinds and reasons are appended; existing ids,
  layouts and program ids do not change.
- Enforcement is stated per callback: **native** (Token-2022 calls our hook program), **program**
  (our program owns the state), **routed** (only when the user routes through us), **automation**
  (off-chain; never enforcement).

## Hook library (kernel builtins)

| Id | Builtin | Environments (callbacks) | Notes |
| --- | --- | --- | --- |
| 9 | `denylist` | token transfer/wrap, router, pool entry, account | Inline keys only. A record-based denylist can be bypassed by omitting the record, so it is not offered. |
| 10 | `dynamic-fee` | pool `before_swap` | Size-ramp mode (formula studied in Patcha) and volatility mode (deviation of the pre-swap price from an EMA kept in entry state). Output `FEE_BPS`; the pool applies it, clamped to its fee cap. |
| 11 | `price-impact` | pool `before_swap` | Constant-product impact `amount_in / (reserve_in + amount_in)` from live reserves the pool supplies. |
| 12 | `price-limit` | router `after_swap`, pool `after_swap` | Realized price bounds (`received/spent` within `[min, max]`) and, in pools, the post-swap pool price within a range: range-order and limit execution guards. Rejection reverts the swap. |
| 13 | `credential` | token transfer, router, pool entry, account | Identity.com gateway token (Civic Pass) for the actor or counterparty: program owner, wallet, gatekeeper network, active state, expiry. |
| 14 | `slot-limit` | router, pool entry | At most N operations per slot on the subject, optionally one per actor per slot. Limits same-slot sandwiches through this subject; it cannot see other venues. |

Not builtins, by design:

- **Liquidity timelock.** Pool policies never gate withdrawals (`pool_policies_cannot_gate_withdrawals`).
  A timelocked LP position is LP tokens held in a smart account whose `before_withdraw` stack has a
  `conditional` (asset = LP mint ⇒ time ≥ unlock). The preset ships in the SDK; it uses existing
  builtins.
- **Anti-JIT.** Implemented as an example **pool program hook** (`examples/anti-jit-hook`): it
  records each LP's add slot and charges removals in the same slot through the pool's bounded
  `after_remove_liquidity` delta, which the core caps.

## DEX adapters (router)

Existing PumpSwap, Raydium CPMM and DAMM v2 adapters are unchanged. Added: Raydium CLMM `swap_v2`,
Orca Whirlpools `swap_v2` (direction-dependent input/output, read from the `a_to_b` argument) and
Meteora DLMM `swap2`. Each is tested against the binary dumped from the public cluster, with live
pool, tick/bin array and oracle accounts cloned by `scripts/fetch_venue_fixtures.py`. An adapter is
marked verified only after that test passes. Orca ships disabled pending license review.

## Token engine

- `programs/token-hooks`: Token-2022 transfer-hook program. Per mint: a `TokenPolicy` PDA with a
  builtin stack (token environment, `before_transfer`) and the `extra-account-metas` list. Rejects
  calls where the source account is not mid-transfer. Credential checks resolve the gateway token
  from the destination owner through `AccountData` seeds.
- `programs/token-wrapper`: wraps eligible SPL / Token-2022 tokens into a Token-2022 mint with the
  transfer hook. Wrapped supply never exceeds the vault balance (measured deltas, checked after
  every operation); redemption burns first and pays from the vault; no admin withdrawal path
  exists. Underlying mints with risky extensions are refused.

## Developer tooling

| Package | Role | Engine |
| --- | --- | --- |
| `packages/sdk` (extended, `v1/`) | Encoders for registry, router, pools, smart accounts, token hooks, wrapper; stack builders | — |
| `packages/execution-lab` | Load compiled programs, set accounts and balances, run transactions, structured traces, export reproductions | npm `litesvm` |
| `packages/hook-bench` + `bench/` | CU, accounts and size per scenario; baseline vs hooked; regression thresholds | execution-lab; Mollusk 0.15.1 for single-instruction benches |
| `packages/hook-security` | Invariants, seeded stateful fuzzing with shrinking, attack suites in 15 categories | execution-lab; Trident in `trident-tests/` |
| `packages/transaction-compiler` | Operations + policies → one atomic transaction (or an explicit error), with accounts, signers, ordering, CU, size, lookup tables, simulation | sdk v1 + execution-lab |
| `packages/hook-analyzer` | Deterministic composition diagnostics (capabilities, ordering, dependencies, conflicts, compute, authorities, upgrades) | kernel rules mirrored in TS, checked against the Rust kernel by shared vectors |
| `crates/hook-sdk` | Rust SDK for custom program hooks (policy-call decoding, authority verification, results, state records) | hook-standard |
| `packages/cli` (extended) | `pumphooks init/build/test/simulate/bench/security-check/deploy/register` | the packages above |

## Cross-engine composition

Shared state uses the existing typed state records (`hook-standard::state`): producer-owned
accounts with schema, namespace, subject, freshness and sequence. Readers verify them in the
kernel; only the producer program can write them. Atomic cross-engine flows are single
transactions that call several engines in sequence; each engine still enforces its own stack.

## Explicitly out of scope

- Claiming native hooks on third-party AMMs.
- Retrofitting transfer hooks onto existing SPL mints.
- Mainnet deployment of custody (wrapper, pools, smart accounts) before an audit.
