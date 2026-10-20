# Implementation status

Running checklist for the expansion described in [INTEGRATION_PLAN.md](INTEGRATION_PLAN.md).
Test levels: **unit** (host Rust/TS), **localnet** (compiled SBF programs in LiteSVM, with real
third-party binaries where named), **devnet** (deployed and exercised on devnet), **mainnet
verified** (against mainnet binaries/state, not deployed). Nothing is deployed to mainnet.

| Area | Item | Status | Level | Evidence |
| --- | --- | --- | --- | --- |
| Research | Upstream research, licenses, plan | done | — | `docs/UPSTREAM_*.md`, `docs/INTEGRATION_PLAN.md` |
| Hook library | `denylist` (9) | done | unit, localnet (pool) | `crates/hook-kernel/src/tests_library.rs`, `tests/core/tests/library.rs` |
| Hook library | `dynamic-fee` (10), size and volatility modes; pool applies it | done | unit, localnet (pool) | same |
| Hook library | `price-impact` (11) | done | unit, localnet (pool) | same |
| Hook library | `price-limit` (12): realized (router, pool) and pool-price range | done | unit, localnet (pool; router via real PumpSwap) | same, `tests/hooks/tests/router.rs` |
| Hook library | `credential` (13): Civic gateway tokens | done | unit, localnet (router) | same |
| Hook library | `slot-limit` (14) | done | unit, localnet (pool) | same |
| DEX adapters | PumpSwap, Raydium CPMM (mainnet, devnet), Meteora DAMM v2 (mainnet, devnet) | done | localnet + mainnet/devnet binaries | `tests/hooks/tests/venues.rs`, `router.rs` |
| DEX adapters | Raydium CLMM `swap_v2` (mainnet) | done | localnet, mainnet binary + cloned pool | `raydium_clmm_mainnet_binary_routes_both_ways` |
| DEX adapters | Orca Whirlpools `swap_v2`, direction-aware accounts | done | localnet, mainnet binary + cloned pool | `orca_whirlpool_mainnet_binary_routes_both_ways` |
| DEX adapters | Meteora DLMM `swap2` via CPI | done | localnet, mainnet binary + cloned pair | `meteora_dlmm_mainnet_binary_routes_both_ways` |
| DEX adapters | Raydium CLMM (devnet) | listed, unverified | — | no devnet pool exercised |
| DEX adapters | Pool account must be owned by the venue program | done | localnet | all venue tests |
| Token engine | `token-hooks`: Token-2022 `Execute`, policies, metas rebuilt from the stack, mid-transfer check | done | localnet (bundled Token-2022 11.0) | `tests/hooks/tests/tokens.rs` |
| Token engine | Credential via gateway-token PDA resolved by Token-2022 | done | localnet | `credential_policy_resolves_gateway_tokens_from_extra_metas`. A mutation of the PDA seed fails it. |
| Token engine | `token-wrapper`: vault, measured mint, burn-then-release, reserve check, extension screen, transfer-fee assets | done | localnet | `wrapper_*` tests |
| Token engine | Devnet deployment of token-hooks / token-wrapper | not done | — | needs funded devnet deployer; no mainnet without audit |
| Execution lab | `packages/execution-lab`: LiteSVM traces (CPI tree, invocations, CU, diffs, token balances, logs, return data), reconstructed hook order, repro export / replay | done | unit (bundled SPL Token), localnet (token-hooks binary) | `packages/execution-lab/test/lab.test.ts`; replay reproduces CU and log digest |
| Compute benchmarks | `bench/` (Rust): Mollusk per-builtin CU of token-hooks `Execute`, LiteSVM Token-2022 transfers with and without the hook, `bench/baseline.json` regression gate (5%), each builtin proven to deny under a denying config; `packages/hook-bench` compare / render CLI | done | localnet (real `.so`, Mollusk and LiteSVM) | `docs/benchmarks/report.md`; `cargo test -p hook-bench`; `packages/hook-bench/test` |
| SDK v1 | `packages/sdk/src/v1`: stack / entry / config encoders for every environment, token-hooks and token-wrapper instructions, PDAs, policy and wrapper decoders, `toWeb3` | done | unit | `packages/sdk/test/v1.test.ts` against `test/vectors/kernel.json`, which `crates/hook-kernel/tests/vectors.rs` regenerates and checks |
| Security | `packages/hook-security`: seeded stateful fuzzer with shrinking; wrapper target (8 invariants incl. exact backing, conservation, atomic failure, predicted outcome of every honest op); 7 adversarial operations; 15-category catalog whose cited tests are checked to exist; markdown report (states it is not an audit) | done | localnet (token-wrapper, token-hooks, bundled SPL Token / Token-2022) | `packages/hook-security/test/*.test.ts`; negative control forges unbacked supply and is caught |
| Security | Trident fuzzing (`trident-tests/`) | not done | — | Trident pins an older Agave / Anchor stack than this workspace (Agave 4, Anchor 0.31.2); needs a separate workspace. The TS stateful fuzzer covers the wrapper meanwhile |
| Security | External audit | not done | — | nothing in this repository is audited |
| Hook library | Manifest `hooks/library/builtins.json` checked against the kernel | done | unit | `library_manifest_matches_the_kernel` |
