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
| Hook library | Manifest `hooks/library/builtins.json` checked against the kernel | done | unit | `library_manifest_matches_the_kernel` |
