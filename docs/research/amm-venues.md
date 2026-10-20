# AMM venue CPI surfaces for the Pump Hooks router

Status: research, 2026-10-20. Scope: which Solana venues the transaction router can CPI into,
what the swap instruction looks like, what blocks CPI, and how hard each adapter is.

Rules used in this document:

- Every claim cites source (`<repo>@<commit>:<file>:<line>`), an on-chain IDL fetched from the
  program's Anchor IDL account, official docs (URL), or an RPC query made on 2026-10-20.
- "Closed source" means the program source is not public. For those venues the account list comes
  from the IDL and the official SDK, and anything the IDL cannot show is marked **unverified**.
- Pump venues are already covered in [../research-pump.md](../research-pump.md). This document
  only adds what that file does not say.

Paths below are relative to `.ref/research/` unless they start with `crates/`, `programs/` or
`docs/` (repo paths) or `.ref/bins/`.

---

## 0. Summary

| Venue | Program ID (mainnet) | Swap instruction we would use | Rating | Key blocker / extra work |
| --- | --- | --- | --- | --- |
| Pump bonding curve | `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P` | `buy_v3` / `buy_exact_quote_in_v3` / `sell_v3` (17 accounts) | **(a)** | None. Already implemented in `crates/pump-adapter`. |
| PumpSwap | `pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA` | `buy_v2` / `buy_exact_quote_in_v2` / `sell_v2` (17 accounts) | **(a)** | None. Already implemented. |
| Raydium CPMM | `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C` | `swap_base_input` / `swap_base_output` (13 accounts) | **(a)** | Official CPI crate is git-only and pins Anchor `=1.0.2`; we are on 0.31.1. Building the instruction by hand avoids it. |
| Raydium CLMM | `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK` | `swap_v2` (13 accounts + bitmap extension + tick arrays) | **(b)** | Variable tick-array remaining accounts computed off-chain. The input token account's authority must be the `payer` signer. |
| Raydium LaunchLab | `LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj` | `buy_exact_in` / `sell_exact_in` (15 + 3 or 4 remaining) | **(b)** | Closed source. Quote token must be SPL Token. A graduated pool moves to CPMM or AMM v4. |
| Meteora DAMM v2 | `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG` | `swap2` (14 accounts + optional instructions sysvar) | **(b)** | **Anti-sniper introspection.** During a rate-limiter window, B→A swaps fail at stack height >2 and fail if any earlier instruction in the transaction references the pool. |
| Meteora DLMM | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` | `swap2` (16 accounts + transfer-hook slices + bin arrays) | **(b)** | Closed source. Bin arrays and transfer-hook accounts must be resolved off-chain. The IDL has a `CpiDisabled` error whose trigger is not visible. |
| Meteora DBC | `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN` | `swap2` (15 accounts), or `swap2_with_transfer_hook` | **(b)** | Same anti-sniper introspection as DAMM v2, for buys, with **no** whitelist. A completed curve rejects swaps (`PoolIsCompleted`) and the market moves to DAMM. |
| Jupiter v6 (on-chain) | `JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4` | `route_v2` / `shared_accounts_route_v2` from `/swap/v2/build` | **(b)** | The route (accounts and `route_plan`) is built off-chain and is opaque to the router. Mainnet only. Rate-limited Meteora pools inside a route fail (stack height 3). |
| Jupiter `/order` (Meta-Aggregator, Ultra) | n/a | returns a whole transaction | **(c)** | Not an instruction. It cannot be wrapped by a router CPI. |

Surprises, in order of impact:

1. **Meteora DAMM v2 and DBC reject CPI deeper than stack height 2** during a pool's rate-limiter
   window (`get_stack_height() > 2`). A top-level router → Meteora call is height 2 and passes. An
   aggregator → our router → Meteora call is height 3 and fails. The only whitelisted caller is the
   OKX smart wallet `va1t8sdGkReA6XFgAeZGXmdQoiEtMirwy4ifLv7yGdH` (DAMM v2 only). See §5 and §7.
2. **Transaction v1 (SIMD-0385, 4096 bytes) is already active on mainnet**, at slot 447,120,000.
   `docs/research.md` §2 says it is "listed for Agave 4.2". v1 messages cap at 64 addresses and have
   no lookup tables. See §10.
3. **SIMD-0268 (CPI depth 8) is not activated on mainnet or devnet.** No feature account exists on
   either cluster. CPI depth is still 4.
4. **None of the official Raydium or Meteora CPI crates are on crates.io.** The `raydium-cp-swap`
   and `raydium-clmm` crates there are third-party. `jupiter-cpi` on crates.io (4.0.3, 2023) and on
   git (6.0.0, Jan 2024) both predate the `*_v2` route instructions that the current program and
   `/build` use.
5. **`pump-rust-client 0.5.0` was published 2026-10-19.** `research-pump.md` was written against
   0.4.1. It needs a diff check (not done here).
6. Jupiter's docs say "CPI cannot use Address Lookup Tables". The runtime does not support that
   claim for v0 transactions; see §9.4.
7. **Router adapters verified (2026-10-20).** `tests/hooks/tests/venues.rs` routes through
   `hook-router` into the real binaries (`scripts/dump_programs.sh`), devnet and mainnet builds:
   - Raydium CPMM (`CPMMoo8L…`, `DRaycpLY…`): pools created by the venue's own `initialize` on the
     network's real AmmConfig index 0 and create-pool fee receiver; `swap_base_input` both ways and
     `swap_base_output`.
   - Meteora DAMM v2 (`cpamdpZC…`): funded SPL pools cloned from each cluster
     (`scripts/fetch_venue_fixtures.py`, `tests/fixtures/venues/`); `swap2` exact-in both ways,
     exact-out, and the legacy `swap`.
   - Each route's receipt equals the measured balance deltas. The router's own min-out and max-in
     bind regardless of the venue's limits, and no SOL leaves the wallet beyond rent and fees.
   - Moving the DAMM signer index from 8 to 7 makes the router refuse with `AccountMismatch`,
     which shows the suite detects a wrong adapter entry.
   - Not covered: swaps inside a DAMM v2 rate-limiter window. Both cloned pools are past it. Item 1
     still applies to those swaps.
   - Concentrated liquidity (mainnet binaries, SOL/USDC pools cloned with the neighbouring tick or
     bin arrays by `fetch_venue_fixtures.py concentrated`):
     - Raydium CLMM `swap_v2` both ways (bitmap extension first in the remaining accounts, then tick
       arrays from the current one in the price direction).
     - Orca Whirlpools `swap_v2` both ways. The adapter reads `a_to_b` (data byte 41) to decide which
       owner account is the input; claiming the other one is refused with `AccountMismatch`. Orca's
       code is under the Orca License; nothing was copied, only the public instruction interface.
     - Meteora DLMM `swap2` both ways **via CPI from the router**: `CpiDisabled` (6100) was not raised
       for this pair. The bitmap extension must be passed writable.
   - The router also checks that the venue's pool account is owned by the venue program.

---

## 1. Sources inspected

Downloaded with `codeload.github.com/<owner>/<repo>/tar.gz/HEAD`. Commit SHAs are from
`api.github.com/repos/<owner>/<repo>/commits/HEAD` at download time.

| Repo | Commit | Date | Used for |
| --- | --- | --- | --- |
| `pump-fun/pump-public-docs` | `639b87fb4` | 2026-10-13 | Pump IDLs, `docs/CPI_README.md` (reused from research-pump.md) |
| `raydium-io/raydium-cp-swap` | `f4b9b2634` | 2026-10-20 | CPMM program source |
| `raydium-io/raydium-clmm` | `edfc5c0a0` | 2026-10-20 | CLMM program source |
| `raydium-io/raydium-cpi` | `115df2779` | 2026-07-20 | Official CPI crates (cpmm, clmm, launch, amm, locking) |
| `raydium-io/raydium-cpi-example` | `336e096e9` | 2025-02-07 | CPI examples |
| `raydium-io/raydium-sdk-V2` | `e2e8bcd86` | 2026-10-08 | LaunchLab instruction builder (`src/raydium/launchpad/instrument.ts`) |
| `MeteoraAg/damm-v2` | `5398b5c7c` | 2026-10-15 | DAMM v2 (`cp-amm`) program source |
| `MeteoraAg/dynamic-bonding-curve` | `91e696eef` | 2026-10-16 | DBC program source |
| `MeteoraAg/dlmm-sdk` | `576919e3e` | 2026-09-03 | DLMM IDL (`idls/dlmm.json`), Rust CLI, CHANGELOG (program source is not public) |
| `MeteoraAg/cpi-examples` | `3326f57fb` | 2025-04-05 | Official DLMM swap CPI example |
| `MeteoraAg/damm-v2-sdk` | `79ebbfe59` | 2026-09-30 | (downloaded, not needed) |
| `jup-ag/jupiter-cpi` | `12bc5f67b` | 2024-01-10 | CPI crate and old IDL |
| `jup-ag/sol-swap-cpi` | `a9c313289` | 2024-11-25 | Official CPI example |
| `jup-ag/jupiter-swap-api-client` | `fb95cfad9` | 2025-12-29 | Swap API client (`/quote`, `/swap`, `/swap-instructions`) |
| `anza-xyz/agave` (partial) | `1c12b3ab8` + `runtime/src/bank.rs` from `322ec0a3c` | 2026-10-20 | Runtime limits |

On-chain Anchor IDLs fetched on 2026-10-20 (account = `createWithSeed(findPDA([], program), "anchor:idl", program)`),
saved to `.ref/research/idls/`:

| Program | IDL account | File | IDL version |
| --- | --- | --- | --- |
| Jupiter v6 | `C88XWfp26heEmDkmfSzeXP7Fd7GQJ2j9dDTUsyiZbUTa` | `idls/jupiter_v6.onchain.json` | 0.1.0, includes `route_v2` |
| Raydium LaunchLab | `E6wT2uNeoWUvDrwdch1R8ETsyewR1ZM4WAwsa5hLJK5Z` | `idls/raydium_launchlab.onchain.json` | 0.2.0 |
| Meteora DLMM | `7UZRobkzaKVm1RbCH5WdFaYCGzCRjnu3prziHAsYiSyr` | `idls/meteora_dlmm.onchain.json` | 0.12.0 (same as `dlmm-sdk/idls/dlmm.json`) |

Official docs used:

- Raydium program addresses: <https://docs.raydium.io/reference/program-addresses>
- Raydium Rust CPI guide: <https://docs.raydium.io/sdk-api/rust-cpi>
- Jupiter Build (Router path): <https://developers.jup.ag/docs/swap/build>
- Jupiter common instructions / CPI: <https://developers.jup.ag/docs/swap/build/common-instructions>
- Jupiter Swap API overview: <https://developers.jup.ag/docs/swap>
- Jupiter old CPI page (archived, "no longer actively maintained"): <https://dev.jup.ag/docs/old/apis/cpi>

Discriminators below are `sha256("global:<name>")[..8]`, computed locally. Each one was
cross-checked against the IDL `discriminator` field or SDK constants where those exist (DLMM IDL,
Jupiter on-chain IDL, LaunchLab on-chain IDL, `raydium-sdk-V2/src/raydium/launchpad/instrument.ts:13-15`).

---

## 2. Pump bonding curve and PumpSwap (reuse)

Everything in [research-pump.md](../research-pump.md) §1 stands. The only additions:

| Item | Bonding curve | PumpSwap |
| --- | --- | --- |
| Program ID | `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P`, same on devnet (`pump-public-docs/idl/pump.json` `address`; RPC: executable on both clusters) | `pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA`, same on devnet (`idl/pump_amm.json`; RPC) |
| Swap ix | `buy_v3` `[7,5,29,196,245,23,101,80]` args `(amount: u64, max_sol_cost: u64, partial_fill: OptionBool)`; `sell_v3`; `buy_exact_quote_in_v3` | `buy_v2` `[184,23,238,97,103,197,211,61]` args `(base_amount_out: u64, max_quote_amount_in: u64)`; `sell_v2`; `buy_exact_quote_in_v2` |
| Accounts | 17. Writable: `bonding_curve`, both curve ATAs, `user` (signer), both user ATAs, `user_volume_accumulator`, `buyback_fee_recipient`. Order in `idl/pump.json` | 17. Writable: `pool`, `user` (signer), both user ATAs, both pool ATAs, `user_volume_accumulator`, `buyback_fee_recipient`. Order in `idl/pump_amm.json` |
| CPI | Supported. Official guide `docs/CPI_README.md:1-46` (CPI into `buy_v2` with `user: Signer`). Binary scan: no `sol_get_stack_height` or `sol_get_processed_sibling_instruction` import (`.ref/bins/devnet/pump.so`, §11) | Same; `.ref/bins/devnet/pump_amm.so` has no CPI-guard syscalls |
| Repo implementation | `crates/pump-adapter/src/ix.rs:91-181` builds both account lists | same |
| Rust crate | `pump-rust-client` **0.5.0** on crates.io (updated 2026-10-19). research-pump.md used 0.4.1 | same crate |
| Devnet binary | Dumped: `.ref/bins/devnet/pump.so` | `.ref/bins/devnet/pump_amm.so` |
| Rating | **(a)** | **(a)** |

---

## 3. Raydium CPMM (`raydium-cp-swap`)

**Program ID.** Mainnet `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C`, devnet
`DRaycpLY18LhpbydsBWbVJtxpNv9oXPgjRSfpF2bWpYb` (`raydium-cp-swap@f4b9b2634:programs/cp-swap/src/lib.rs:22-25`;
matches docs.raydium.io/reference/program-addresses). An older devnet deployment
`CPMDWBwJDtYax9qW7AyRuVC19Cc4L4Vcy4n2BHAbHkCW` (listed in the 2025 `raydium-cpi-example` README) is also still
executable on devnet. Use the `DRay…` ID; it matches the current source's `devnet` feature.

**Swap instructions** (`lib.rs:355-373`). Both use the same `Swap` accounts struct.

| Instruction | Discriminator | Args |
| --- | --- | --- |
| `swap_base_input` | `[143,190,90,218,196,30,51,222]` | `amount_in: u64, minimum_amount_out: u64` |
| `swap_base_output` | `[55,217,98,86,163,74,180,173]` | `max_amount_in: u64, amount_out: u64` |

**Accounts** (`instructions/swap_base_input.rs:9-73`). No remaining accounts.

| # | Account | Signer | Writable | Constraint |
| --- | --- | --- | --- | --- |
| 0 | `payer` | ✓ | | Transfer authority for the input token account |
| 1 | `authority` | | | PDA `["vault_and_lp_mint_auth_seed"]` (`AUTH_SEED`, `lib.rs:82`) |
| 2 | `amm_config` | | | `== pool_state.amm_config` |
| 3 | `pool_state` | | ✓ | |
| 4 | `input_token_account` | | ✓ | Any token account; no owner constraint |
| 5 | `output_token_account` | | ✓ | Any token account; no owner constraint |
| 6 | `input_vault` | | ✓ | One of the pool's vaults |
| 7 | `output_vault` | | ✓ | One of the pool's vaults |
| 8 | `input_token_program` | | | SPL Token or Token-2022 |
| 9 | `output_token_program` | | | SPL Token or Token-2022 |
| 10 | `input_token_mint` | | | `== input_vault.mint` |
| 11 | `output_token_mint` | | | `== output_vault.mint` |
| 12 | `observation_state` | | ✓ | `== pool_state.observation_key` |

**CPI.** Nothing blocks CPI. There is no stack-height or instructions-sysvar check anywhere in
`programs/cp-swap/src` (grep for `get_stack_height`, `load_instruction_at`, `sysvar::instructions`:
no hits), and the devnet binary imports neither syscall (§11). The input transfer is
`transfer_checked` with `authority = payer` (`utils/token.rs:17-41`), so `payer` must be the owner
or delegate of `input_token_account`. The output account is unconstrained, so the router can pay out
straight to the user's ATA. Events use `emit!` (logs), not `emit_cpi!` (`swap_base_input.rs:165`).
Slippage is checked on the amount received after transfer fees (`:142-155`).

**Token-2022.**

- Swaps compute transfer fees on both legs (`swap_base_input.rs:85-89`, `:143-149`).
- Pool creation allows only `TransferFeeConfig`, `MetadataPointer`, `TokenMetadata`,
  `InterestBearingConfig` and `ScaledUiAmount`, unless the admin has created a `SupportMintAssociated`
  record for the mint (`utils/token.rs:335-360`).
- Transfers do not forward transfer-hook extra accounts (`utils/token.rs:17-71`; no `TransferHook`
  reference in the program). A transfer-hook mint therefore cannot be swapped even if it were
  whitelisted.

**CPI crate.** `raydium-cpmm-cpi` 0.1.0, git only
(`raydium-cpi@115df2779:programs/cpmm-cpi/Cargo.toml`, README "Usage"). It is not on crates.io
(404 on 2026-10-20). It pins `anchor-lang = "=1.0.2"`. Our `programs/pump-hooks` uses Anchor 0.31.1;
the README points older Anchor users to other branches (`anchor-0.31.0`). The instruction is 13
fixed metas plus 24 bytes of data, so a hand-built instruction (as `crates/pump-adapter` does for
Pump) is simpler than adding the dependency.

**Devnet binary.** Dumped: `.ref/bins/devnet/raydium_cpmm.so`. It was built with the `devnet`
feature, so it carries the devnet admin and fee keys (`lib.rs:27-80`).

**Rating: (a)** verified implementable now.

---

## 4. Raydium CLMM (`raydium-clmm`)

**Program ID.** Mainnet `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK`, devnet
`DRayAUgENGQBKVaX8owNhgzkEDyoHTGVEGHVJT1E9pfH` (`raydium-clmm@edfc5c0a0:programs/amm/src/lib.rs:24-27`;
Raydium docs). An older devnet ID `devi51mZmdwUJGU9hjN27vEz64Gps7uUefqxg27EAtH` is still executable on devnet.

**Swap instructions** (`lib.rs:636-692`).

| Instruction | Discriminator | Args | Notes |
| --- | --- | --- | --- |
| `swap_v2` | `[43,4,237,11,26,201,30,98]` | `amount: u64, other_amount_threshold: u64, sqrt_price_limit_x64: u128, is_base_input: bool` | Use this one: supports Token-2022 |
| `swap` | `[248,198,158,145,225,117,135,200]` | same | Marked deprecated (`lib.rs:625`); SPL Token only |
| `swap_router_base_in` | `[69,125,115,218,245,186,242,196]` | `amount_in: u64, amount_out_minimum: u64` | Multi-hop |

**`swap_v2` accounts** (`instructions/swap_v2.rs:14-79`).

| # | Account | Signer | Writable | Constraint |
| --- | --- | --- | --- | --- |
| 0 | `payer` | ✓ | | |
| 1 | `amm_config` | | | `== pool_state.amm_config` |
| 2 | `pool_state` | | ✓ | |
| 3 | `input_token_account` | | ✓ | `token::mint = input_vault.mint`, **`token::authority = payer`** (`:28-33`) |
| 4 | `output_token_account` | | ✓ | `token::mint = output_vault.mint` |
| 5 | `input_vault` | | ✓ | Checked against the pool vaults (`:118-127`) |
| 6 | `output_vault` | | ✓ | |
| 7 | `observation_state` | | ✓ | `== pool_state.observation_key` |
| 8 | `token_program` | | | SPL Token |
| 9 | `token_program_2022` | | | Token-2022 (always required) |
| 10 | `memo_program` | | | |
| 11 | `input_vault_mint` | | | `== input_vault.mint` |
| 12 | `output_vault_mint` | | | `== output_vault.mint` |
| 13.. | remaining: optional `tickarray_bitmap_extension`, then `tick_array_*` | | ✓ | Classified by data length; the loop stops at the first account that is neither (`:129-141`) |

**CPI.** No stack-height or introspection checks (grep: none; binary: no CPI-guard syscalls). Extra
work for the router:

- The client must compute the tick arrays the swap will cross and pass them in order, plus the
  bitmap extension (PDA `["pool_tick_array_bitmap_extension", pool]`, `states/pool.rs:21`) when the
  price is outside the default bitmap range. Tick arrays are 60 ticks each (`states/tick_array.rs:13`).
- The router must forward a variable-length remaining-account tail. The tail size depends on price
  movement, and slippage must leave room for crossing extra arrays.
- Because of `token::authority = payer`, if the router swaps from a router-PDA-owned account, the
  router PDA must be `payer` (signed via `invoke_signed`). If it swaps from the user's own account,
  the user must be the signer and the router forwards that signature.
- With `sqrt_price_limit_x64 = 0` a partial fill is rejected (`:319-334`), so semantics are exact-in
  or exact-out.
- Dynamic fees and limit orders are new in this commit. Limit orders are matched inside tick state
  (`instructions/swap.rs:672-696`, `states/tick_array.rs:293-299`) and need no extra accounts.

**Token-2022.** Same pool-creation allowlist as CPMM (`util/token.rs:436-461`), with admin
whitelisting via `SupportMintAssociated` (`instructions/create_pool.rs:128-133`). Transfer fees are
handled in `swap_v2` (`swap_v2.rs:101-109`, `:203-243`). Transfer hooks are not supported (no
references in the program).

**CPI crate.** `raydium-clmm-cpi` 0.1.0, git only (`raydium-cpi@115df2779:programs/clmm-cpi`),
Anchor `=1.0.2`. The crates.io crate `raydium-clmm` 0.1.13 is third-party (its repository field
points at `ohaddahan/solana-idls`). Do not use it.

**Devnet binary.** Dumped: `.ref/bins/devnet/raydium_clmm.so`.

**Rating: (b)** implementable, but needs the off-chain tick-array resolution, a variable
remaining-account tail, and care with the `payer` = input authority rule.

---

## 5. Raydium LaunchLab (closed source)

**Program ID.** Mainnet `LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj`, devnet
`DRay6fNdQ5J82H7xV6uq2aV3mNrUZ1J4PgSKsWgptcm6` (`raydium-cpi@115df2779:programs/launch-cpi/src/lib.rs:10-13`;
Raydium docs, which also state "LaunchLab … source not publicly available").
`github.com/raydium-io/raydium-launch` returns 404.

**Swap instructions.** From the on-chain IDL (`idls/raydium_launchlab.onchain.json`), the CPI stub crate
(`launch-cpi/src/lib.rs:114-171`) and the SDK (`raydium-sdk-V2@e2e8bcd86:src/raydium/launchpad/instrument.ts:13-15`).

| Instruction | Discriminator | Args |
| --- | --- | --- |
| `buy_exact_in` | `[250,234,13,123,213,156,19,236]` | `amount_in: u64, minimum_amount_out: u64, share_fee_rate: u64` |
| `buy_exact_out` | `[24,211,116,40,105,3,153,56]` | `amount_out: u64, maximum_amount_in: u64, share_fee_rate: u64` |
| `sell_exact_in` | `[149,39,222,155,211,124,152,26]` | `amount_in: u64, minimum_amount_out: u64, share_fee_rate: u64` |
| `sell_exact_out` | `[95,200,71,34,8,9,11,166]` | `amount_out: u64, maximum_amount_in: u64, share_fee_rate: u64` |

**Accounts.** Fixed 15 from the IDL. The remaining tail comes from `launch-cpi/src/context.rs:490-509`
and `instrument.ts:386-413`.

| # | Account | Signer | Writable | Notes |
| --- | --- | --- | --- | --- |
| 0 | `payer` | ✓ | ✓ | |
| 1 | `authority` | | | PDA `["vault_auth_seed"]` (`launch-cpi/src/lib.rs:55`) |
| 2 | `global_config` | | | `== pool_state.global_config` |
| 3 | `platform_config` | | | `== pool_state.platform_config` |
| 4 | `pool_state` | | ✓ | |
| 5 | `user_base_token` | | ✓ | `token::mint = base_token_mint` |
| 6 | `user_quote_token` | | ✓ | `token::mint = quote_token_mint` |
| 7 | `base_vault` | | ✓ | `== pool_state.base_vault` |
| 8 | `quote_vault` | | ✓ | `== pool_state.quote_vault` |
| 9 | `base_token_mint` | | | |
| 10 | `quote_token_mint` | | | |
| 11 | `base_token_program` | | | `Interface<TokenInterface>` |
| 12 | `quote_token_program` | | | **`Program<Token>`: SPL Token only** (`context.rs:484`) |
| 13 | `event_authority` | | | `#[event_cpi]` (`context.rs:401`) |
| 14 | `program` | | | |
| 15 | `share_fee_receiver` | | ✓ | Only if `share_fee_rate > 0` |
| +1 | `system_program` | | | Always pushed by the SDK. The crate comment says it is required after an upgrade timestamp |
| +2 | `platform_fee_vault` | | ✓ | PDA `[platform_config, quote_mint]` |
| +3 | `creator_fee_vault` | | ✓ | PDA `[creator, quote_mint]` |

**CPI.** Raydium's Rust CPI guide documents CPI into `buy_exact_in` via `declare_program!` from the
IDL (docs.raydium.io/sdk-api/rust-cpi, "Applying the pattern: LaunchLab"). The devnet binary imports
no CPI-guard syscalls (§11). Because the source is closed, the absence of other checks is
**unverified**. A LiteSVM test against `.ref/bins/devnet/raydium_launchlab.so` should confirm it
before we claim support.

**Lifecycle.** After fundraising the pool migrates to CPMM or AMM v4 according to
`pool_state.migrate_type`. The Raydium docs say the CPI caller must branch on it. Token-2022 pools
(`initialize_with_token_2022`) must migrate to CPMM (`launch-cpi/src/lib.rs:90-105`).

**Token-2022.** Base token may be Token-2022 with an optional `TransferFeeExtensionParams`
(`lib.rs:96-105`). Quote token is SPL Token only (account 12 above).

**CPI crate.** `raydium-launch-cpi` 0.1.0, git only, interface stubs (every handler body is `Ok(())`),
Anchor `=1.0.2`. Alternatively use `declare_program!` from the on-chain IDL.

**Devnet binary.** Dumped: `.ref/bins/devnet/raydium_launchlab.so`.

**Rating: (b)** implementable from IDL and SDK. Extra work: the variable 3–4 account remaining tail,
the SPL-only quote, post-graduation routing to CPMM or AMM v4, and a LiteSVM verification because
the source is closed.

---

## 6. Meteora DAMM v2 (`cp-amm`)

**Program ID.** `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG` on mainnet and devnet
(`damm-v2@5398b5c7c:programs/cp-amm/src/lib.rs:42`; RPC: executable on both).

**Swap path.** The Anchor handlers are stubs (`lib.rs:264-270`). A custom pinocchio entrypoint
matches the `swap`/`swap2` discriminators and runs `p_handle_swap` (`entrypoint.rs:28-108`).
Everything else falls back to Anchor (`entrypoint.rs:110-125`).

| Instruction | Discriminator | Args |
| --- | --- | --- |
| `swap2` | `[65,75,63,76,235,91,91,136]` | `SwapParameters2 { amount_0: u64, amount_1: u64, swap_mode: u8 }`. `swap_mode`: 0 ExactIn, 1 PartialFill, 2 ExactOut (`instructions/swap/ix_swap.rs:19-49`) |
| `swap` | `[248,198,158,145,225,117,135,200]` | `SwapParameters { amount_in: u64, minimum_amount_out: u64 }`, mapped to ExactIn |

**Accounts** (`ix_swap.rs:51-98`; `SWAP_IX_ACCOUNTS = 14`, `ix_p_swap.rs:28-29`).

| # | Account | Signer | Writable | Notes |
| --- | --- | --- | --- | --- |
| 0 | `pool_authority` | | | Const PDA `const_pda::pool_authority::ID` |
| 1 | `pool` | | ✓ | `has_one = token_a_vault, token_b_vault` |
| 2 | `input_token_account` | | ✓ | Direction is inferred from its mint (`ix_p_swap.rs:32-42`) |
| 3 | `output_token_account` | | ✓ | |
| 4 | `token_a_vault` | | ✓ | |
| 5 | `token_b_vault` | | ✓ | |
| 6 | `token_a_mint` | | | |
| 7 | `token_b_mint` | | | |
| 8 | `payer` | ✓ | | Transfer authority (`p_helper.rs:16-37`) |
| 9 | `token_a_program` | | | |
| 10 | `token_b_program` | | | |
| 11 | `referral_token_account` | | ✓ | Optional. Pass the program ID for None (`ix_swap.rs:194-197`) |
| 12 | `event_authority` | | | Self-CPI event (`ix_p_swap.rs:248-273`) |
| 13 | `program` | | | |
| 14 | `Sysvar1nstructions1111111111111111111111111` | | | Remaining[0], **required only when the rate limiter applies** (`ix_p_swap.rs:275-283`) |

The fast path deserializes at most `SWAP_IX_ACCOUNTS + 1 = 15` accounts
(`entrypoint.rs:31-33`), so no other remaining accounts (for example transfer-hook extras) can reach
the swap.

**CPI guard (anti-sniper rate limiter).** `ix_p_swap.rs:131-145` and `:275-349`:

- Applies only when the pool's base fee is a rate limiter, the trade is B→A, and the current point
  is within `[activation_point, activation_point + max_limiter_duration]`
  (`base_fee/fee_rate_limiter.rs:116-142`; maximum duration 12 h, `constants.rs:55`).
- When it applies, the swap requires the instructions sysvar. If the top-level instruction is not
  DAMM v2 itself, then:
  - `get_stack_height() > 2` fails unless the top-level program is in
    `RATE_LIMITER_STACK_WHITELIST_PROGRAMS`. That list contains only the OKX smart wallet
    (`constants.rs:62-65`).
  - Any already-processed sibling CPI that is a DAMM v2 swap on the same pool fails (`:305-315`).
- Any earlier top-level instruction in the transaction that mentions the pool address fails
  (`:318-346`).

Consequences for the router:

- Router at top level → DAMM v2 is stack height 2. **Allowed.**
- One swap per pool per transaction.
- No earlier instruction in the transaction may reference the pool. Pre-hooks must run inside the
  router instruction, not as a separate earlier instruction.
- If our router is itself called by an aggregator or smart wallet (height ≥3), B→A swaps on
  rate-limited pools **fail** during the window.
- The router should always append the instructions sysvar. It is harmless when the limiter is off,
  and the alternative is reading the pool's fee config to decide.

**Pool access.** `can_swap(payer)` requires `current_point >= activation_point`. The pool's
`whitelisted_vault` (alpha vault) may trade from `pre_activation_point`
(`pool_action_access/permissionless.rs:50-60`).

**Token-2022.** Permissionless pools accept SPL Token, plus Token-2022 with only `TransferFeeConfig`,
`MetadataPointer` and `TokenMetadata`, or `TransferHook` with **both program ID and authority unset**.
Native-mint Token-2022 is rejected (`utils/token.rs:222-262`). Other extensions need an admin
`TokenBadge` (`:264-290`). Transfers are plain `TransferChecked` with no hook accounts
(`p_helper.rs:16-60`).

**CPI crate.** No published crate (`cp-amm` and `damm-v2` both 404 on crates.io). The program crate
has a `cpi` feature (`programs/cp-amm/Cargo.toml:16`) but is not published, and it pulls pinocchio
from git. Hand-build the 14 or 15 metas.

**Devnet binary.** Dumped: `.ref/bins/devnet/meteora_damm_v2.so`.

**Rating: (b)** implementable. The router must be the top-level instruction, must not let another
instruction touch the pool, and should append the instructions sysvar. Nested routing through the
router is blocked for rate-limited pools.

---

## 7. Meteora DBC (`dynamic-bonding-curve`)

**Program ID.** `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN` on mainnet and devnet
(`dynamic-bonding-curve@91e696eef:programs/dynamic-bonding-curve/src/lib.rs:29`; RPC).

**Swap instructions** (`lib.rs:235-268`).

| Instruction | Discriminator | Args | Pools |
| --- | --- | --- | --- |
| `swap2` | `[65,75,63,76,235,91,91,136]` | `SwapParameters2 { amount_0, amount_1, swap_mode: u8 }` | `VirtualPool` only |
| `swap` | `[248,198,158,145,225,117,135,200]` | `SwapParameters { amount_in, minimum_amount_out }` | `VirtualPool` only |
| `swap2_with_transfer_hook` | `[183,93,153,40,24,230,194,151]` | `SwapParameters2, TransferHookAccountsInfo` | `TransferHookPool` only |

**Accounts** (`instructions/swap/ix_swap.rs:11-61`).

| # | Account | Signer | Writable | Notes |
| --- | --- | --- | --- | --- |
| 0 | `pool_authority` | | | Const PDA |
| 1 | `config` | | | |
| 2 | `pool` | | ✓ | `VirtualPool` (or `TransferHookPool` for the hook variant) |
| 3 | `input_token_account` | | ✓ | |
| 4 | `output_token_account` | | ✓ | |
| 5 | `base_vault` | | ✓ | |
| 6 | `quote_vault` | | ✓ | |
| 7 | `base_mint` | | (✓ in hook variant, `ix_swap2_with_transfer_hook.rs:36-39`) | |
| 8 | `quote_mint` | | | |
| 9 | `payer` | ✓ | | |
| 10 | `token_base_program` | | | |
| 11 | `token_quote_program` | | | |
| 12 | `referral_token_account` | | ✓ | Optional, `dup` allowed |
| 13 | `event_authority` | | | `#[event_cpi]`; the handler emits up to 4 self-CPI events (`ix_swap.rs:96-143`) |
| 14 | `program` | | | |
| 15.. | remaining: optional instructions sysvar **first**, then transfer-hook accounts | | | At most 1 non-hook remaining account (`process_swap.rs:174-195`) |

**CPI guard.** Same design as DAMM v2 (`process_swap.rs:200-212`, `:438-493`). Differences:

- Applies to **QuoteToBase (buys)** during `[activation_point, activation_point + max_limiter_duration]`
  (`base_fee/fee_rate_limiter.rs:42-63`).
- **No whitelist**: `get_stack_height() > 2` fails outright (`process_swap.rs:453-455`).
- Missing sysvar while the limiter applies fails with `FailToValidateSingleSwapInstruction`.
- "First swap with min fee" only applies to a top-level DBC instruction placed right after the pool's
  `initialize_*` instruction (`process_swap.rs:214-221`, `:506-556`). Routed swaps pay the normal fee.

For a launchpad venue that is the important window: early buys through any CPI caller deeper than
height 2 fail.

**Lifecycle.** Once quote reserves reach `migration_quote_threshold`, swaps fail with
`PoolIsCompleted` (`process_swap.rs:224-227`) until migration to DAMM v2 or DAMM v1. The router
should map this to a clear "market migrating" denial, as it does for Pump.

**Token-2022.**

- Base mint is created by DBC with `MetadataPointer`, optionally `TransferFeeConfig` and
  `TransferHook` (`utils/token.rs:410-428`).
- Quote mint: SPL Token, or Token-2022 with only metadata extensions and a zero transfer fee with no
  fee authority (`utils/token.rs:343-376`), or badged.
- Transfer-hook pools need `swap2_with_transfer_hook` with the hook's extra accounts.

**CPI crate.** None published (`dynamic-bonding-curve` 404). There is an off-chain quote crate
`dynamic-bonding-curve-sdk` in the repo (not published).

**Devnet binary.** Dumped: `.ref/bins/devnet/meteora_dbc.so`.

**Rating: (b)**, with the same top-level-only caveat as DAMM v2 but stricter (no whitelist, and it
hits buys, which is the main launchpad flow). Supporting transfer-hook pools adds extra-account
resolution.

---

## 8. Meteora DLMM (`lb_clmm`, closed source)

**Program ID.** `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` on mainnet and devnet
(`dlmm-sdk@576919e3e:idls/dlmm.json` `address`, IDL version 0.12.0; on-chain IDL identical version;
RPC executable on both). The program source is not in any public repo. `dlmm-sdk` ships the IDL and a
compiled `artifacts/lb_clmm.so`.

**Swap instructions** (IDL).

| Instruction | Discriminator | Args |
| --- | --- | --- |
| `swap2` | `[65,75,63,76,235,91,91,136]` | `amount_in: u64, min_amount_out: u64, remaining_accounts_info: RemainingAccountsInfo` |
| `swap_exact_out2` | `[43,215,247,132,137,60,243,81]` | `max_in_amount: u64, out_amount: u64, remaining_accounts_info` |
| `swap_with_price_impact2` | `[74,98,192,214,177,51,75,51]` | `amount_in: u64, active_id: Option<i32>, max_price_impact_bps: u16, remaining_accounts_info` |
| `swap` / `swap_exact_out` / `swap_with_price_impact` | `[248,198,…]` / `[250,73,…]` / `[56,173,…]` | v1 variants. Their IDL pins both token programs to SPL Token |

`RemainingAccountsInfo { slices: Vec<{ accounts_type: AccountsType, length: u8 }> }` with
`AccountsType ∈ { TransferHookX, TransferHookY, TransferHookReward, TransferHookMultiReward(u8), TransferHookReferral }`.

**`swap2` accounts** (IDL).

| # | Account | Signer | Writable | Notes |
| --- | --- | --- | --- | --- |
| 0 | `lb_pair` | | ✓ | |
| 1 | `bin_array_bitmap_extension` | | ✓ | Optional. Pass the program ID for None (`cli/src/instructions/swap_exact_in.rs:106-109`) |
| 2 | `reserve_x` | | ✓ | |
| 3 | `reserve_y` | | ✓ | |
| 4 | `user_token_in` | | ✓ | |
| 5 | `user_token_out` | | ✓ | |
| 6 | `token_x_mint` | | | |
| 7 | `token_y_mint` | | | |
| 8 | `oracle` | | ✓ | |
| 9 | `host_fee_in` | | ✓ | Optional (program ID for None) |
| 10 | `user` | ✓ | | |
| 11 | `token_x_program` | | | |
| 12 | `token_y_program` | | | |
| 13 | `memo_program` | | | `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr` |
| 14 | `event_authority` | | | |
| 15 | `program` | | | |
| 16.. | remaining: transfer-hook accounts per `slices`, **then** bin arrays | | bin arrays ✓ | Order from the official CLI (`swap_exact_in.rs:127-158`) |

**CPI.**

- Officially supported: Meteora publishes a DLMM swap CPI example
  (`cpi-examples@3326f57fb:programs/cpi-example/src/instructions/dlmm_cpi/swap.rs:1-102`), using
  `declare_program!(dlmm)` from the IDL (`src/lib.rs:8`). The example uses v1 `swap`; `swap2` has the
  same shape plus `memo_program` and the slices argument.
- The IDL defines error 6100 `CpiDisabled` ("CPI disabled"), and the devnet binary contains the
  string. The source is closed, so which instruction raises it is **unverified**. Evidence that it is
  not a swap guard:
  - The neighbouring IDL errors are zap protocol-fee errors (6099 `MintRestrictedFromZap`).
  - In DAMM v2 and DBC, the identically named error is marked `/// deprecated` next to the same zap
    errors (`damm-v2/.../error.rs:199-209`, `dynamic-bonding-curve/.../error.rs:200-210`).
  - The DLMM binary does not import `sol_get_stack_height` or
    `sol_get_processed_sibling_instruction` (§11).
  - The swap account list has no instructions sysvar and `AccountsType` has no slot for one.
  - **Must be confirmed by a LiteSVM CPI swap test** against `.ref/bins/devnet/meteora_dlmm.so`.
- The client must resolve the bin arrays to cross (the CLI uses 3, `swap_exact_in.rs:73-79`) and the
  bitmap extension, and pass them as remaining accounts. Limit orders (new in 0.12) are part of bin
  liquidity in the quote (`CHANGELOG.md:83`); they add no swap accounts in the IDL.

**Token-2022.** Supported, including transfer hooks via the slices (errors
`MissingRemainingAccountForTransferHook`, `InvalidTransferHookAuthority`; instruction
`initialize_token_badge`). The exact extension allowlist is in closed source: **unverified** beyond
the IDL error `UnsupportedMintExtension`.

**CPI crate.** None published (`dlmm` 404; `lb_clmm` 0.1.1 on crates.io dates from 2024-05 with no
repository and is not referenced by Meteora). The official route is `declare_program!` from
`idls/dlmm.json` (CHANGELOG `:277-278`).

**Devnet binary.** Dumped: `.ref/bins/devnet/meteora_dlmm.so` (also `dlmm-sdk/artifacts/lb_clmm.so`).

**Rating: (b)** implementable. Extra work: bin-array and transfer-hook account resolution off-chain,
a variable remaining tail, and a LiteSVM test to rule out `CpiDisabled` on the swap path.

---

## 9. Jupiter

### 9.1 Program and instructions

**Program ID.** `JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4` (`jupiter-cpi@12bc5f67b:src/lib.rs:3`;
on-chain IDL `address`). **Mainnet only**: on devnet the address is a non-executable system account
(RPC, 2026-10-20). LiteSVM tests must use the mainnet dump plus cloned venue accounts.

The `jupiter-cpi` IDL (2024-01) has `route`, `shared_accounts_route`, the exact-out and token-ledger
variants, and many per-DEX instructions. The **current on-chain IDL** (`idls/jupiter_v6.onchain.json`)
lists 19 instructions and adds `route_v2`, `shared_accounts_route_v2`, `exact_out_route_v2` and
`shared_accounts_exact_out_route_v2`. The mainnet binary's log strings include `RouteV2Instruction`
and `SharedAccountsRouteV2Instruction` (§11).

| Instruction | Discriminator | Args |
| --- | --- | --- |
| `route_v2` | `[187,100,250,204,49,196,175,20]` | `in_amount: u64, quoted_out_amount: u64, slippage_bps: u16, platform_fee_bps: u16, positive_slippage_bps: u16, route_plan: Vec<RoutePlanStepV2>` |
| `shared_accounts_route_v2` | `[209,152,83,147,124,254,216,233]` | `id: u8` + same as above |
| `route` (v1) | `[229,23,203,151,122,227,173,42]` | `route_plan: Vec<RoutePlanStep>, in_amount: u64, quoted_out_amount: u64, slippage_bps: u16, platform_fee_bps: u8` |
| `shared_accounts_route` (v1) | `[193,32,155,51,65,214,156,129]` | `id: u8` + v1 args |

**`route_v2` fixed accounts** (on-chain IDL). Every venue account for each hop follows as remaining
accounts, supplied by the API.

| # | Account | Signer | Writable |
| --- | --- | --- | --- |
| 0 | `user_transfer_authority` | ✓ | |
| 1 | `user_source_token_account` | | ✓ |
| 2 | `user_destination_token_account` | | ✓ |
| 3 | `source_mint` | | |
| 4 | `destination_mint` | | |
| 5 | `source_token_program` | | |
| 6 | `destination_token_program` | | |
| 7 | `destination_token_account` | | ✓ (optional) |
| 8 | `event_authority` (`D8cy77BBepLMngZx6ZukaTff5hCt1HrWyKk3Hnd9oitf`) | | |
| 9 | `program` | | |

### 9.2 Integration surfaces

| Surface | What it returns | Router CPI? | Source |
| --- | --- | --- | --- |
| `GET /swap/v2/order` + `/execute` (Meta-Aggregator: Metis, JupiterZ RFQ, Dflow, OKX) | A complete transaction for the user to sign, landed by Jupiter | **No.** A whole transaction cannot be wrapped in a router instruction, and RFQ fills are not a Jupiter-program instruction | developers.jup.ag/docs/swap |
| `GET /swap/v2/build` (Router, Metis only) | `computeBudgetInstructions`, `setupInstructions`, `swapInstruction`, `cleanupInstruction`, `addressesByLookupTableAddress`, `transactionVersion` | **Yes**: "use the swap instruction from `/build` as the inner instruction in a CPI call" | developers.jup.ag/docs/swap/build/common-instructions |
| Legacy `/quote` + `/swap-instructions` | Same idea (client in `jupiter-swap-api-client/src/lib.rs:82-90`) | Yes, historically | `jup-ag/sol-swap-cpi` README |
| Flash fill (no CPI) | Router instructions before/after a top-level Jupiter instruction | Not a CPI. Hooks could not bind to the swap atomically except by delta checks across instructions | `jup-ag/sol-swap-flash-fill` (referenced in the sol-swap-cpi README) |

### 9.3 Official CPI guidance (quoted from current docs)

- "For protocols that need to invoke the Jupiter swap from within their own onchain program, use
  the swap instruction from `/build` as the inner instruction in a CPI call."
- "CPI cannot use Address Lookup Tables (ALTs), which limits the number of accounts in the
  transaction. Jupiter's complex routing often requires many accounts."
- "Use `maxAccounts` on `/build` to control route complexity" (default 64, range 1–64).
- "Set your own compute budget, as CPI adds overhead."
- The old docs page says "As of January 2025, Jupiter Swap via CPI is recommended for most users",
  citing the "Loosen CPI restriction" feature.

### 9.4 Assessment

- **Depth.** Router (1) → Jupiter (2) → DEX (3) → Token / Token-2022 (4) → transfer hook (5). That fits
  the current limit of 5 frames (§10) with no headroom. A DEX that itself CPIs further (or an
  aggregator calling our router) runs out of depth.
- **Meteora rate limiters.** A Jupiter route through a DAMM v2 or DBC pool in its rate-limiter window
  runs at height 3 and fails (§6, §7).
- **Opaque route.** The router cannot validate `route_plan` semantics on-chain. Policy hooks can only
  constrain input amount, output mint, minimum out, and the observed token deltas. That is enough for
  our "observe deltas" model (research-pump.md §4.4), but hooks that reason about venue-specific
  state (reserves, curve phase) cannot apply to a Jupiter leg.
- **Authority.** `user_transfer_authority` must sign. Either the user signs and the router forwards
  the signature, or the router owns the source account via a PDA.
- **ALT claim.** In a v0 transaction, lookup-table addresses are loaded into the transaction's
  account list before execution, and a CPI references accounts by their position in that list. We
  therefore expect ALTs to work for accounts passed to a CPI. The real limits are the 64
  account-lock limit, 255 accounts per CPI instruction, and the 1232-byte size. This is an inference
  from the runtime model, not from Jupiter: **verify it in a test** before relying on it. Under v1
  transactions (no ALTs, 64 addresses) Jupiter's statement is moot.

**Rating: (b)** for `/build` + CPI (route built off-chain, account budget via `maxAccounts`, depth
headroom zero). **(c)** for `/order` (Meta-Aggregator / Ultra).

**Crates.** `jupiter-cpi` 4.0.3 on crates.io (2023-04) and 6.0.0 on git (2024-01) are both stale:
neither has `*_v2`. `jupiter-swap-api-client` 0.2.0 (crates.io, 2025-05). Use the on-chain IDL with
`declare_program!`, or pass the `/build` instruction through opaquely.

---

## 10. Solana runtime limits (verified)

Agave sources: `agave@1c12b3ab8` (partial, in `.ref/research/agave`), `runtime/src/bank.rs` from
`agave@322ec0a3c`, and crates in `~/.cargo/registry` (`agave-feature-set 4.2.2`, `solana-message 4.4.1`,
`agave-transaction-view 4.2.2`, `solana-transaction 4.1.6`, `solana-packet 4.2.0`, `solana-cpi 3.1.0`).
Feature status comes from `getMultipleAccounts` on the feature IDs (mainnet slot ~459,237,140,
devnet slot ~513,166,713, 2026-10-20). A missing feature account means the gate is neither activated
nor scheduled.

| Limit | Value | Source | Status |
| --- | --- | --- | --- |
| Instruction stack depth | **5 frames = 4 nested CPIs** | `program-runtime/src/execution_budget.rs:8` | In force |
| With SIMD-0268 | 9 frames = 8 nested CPIs | `execution_budget.rs:10-18`; gate `raise_cpi_nesting_limit_to_8` = `6TkHkRmP7JZy1fdM6fg5uXn76wChQBWGokHBJzrLB3mj` (`agave-feature-set-4.2.2/src/lib.rs:1339-1341`, `:2471-2472`) | **Not active on mainnet or devnet** (no feature account on either) |
| Re-entrancy | A→A allowed, A→B→A rejected | `program-runtime/src/invoke_context.rs:246-270` (already in research.md §2) | In force |
| Accounts locked per transaction | **64** (128 behind a gate) | `runtime/src/bank.rs:3858-3866`; `MAX_TX_ACCOUNT_LOCKS = 128` (`solana-transaction-4.1.6/src/sanitized.rs:23`); gate `increase_tx_account_lock_limit` = `9LZdXeKGeBV6hRLdxS1rHbHoEUsKqesCC2ZAPTPKJAbK` | Gate **not active** on either cluster. On mainnet the address holds 0.02 SOL as a system account, not a feature account |
| Accounts per instruction / per CPI instruction | 255 | `transaction-context/src/lib.rs:17`; `program-runtime/src/cpi.rs:186-200` | In force |
| Account infos per CPI | 255 | `program-runtime/src/cpi.rs:163`, `:203-213`; SIMD-0339 gate `H6iVbVaDZgDphcPbcZwc5LoznMPWQfnJ1AM7L1xzqvt5` | Active: mainnet slot 403,056,000, devnet 435,456,000 |
| CPI instruction data | 10 KiB | `transaction-context/src/lib.rs:18`; `cpi.rs:193-198` | In force |
| PDA signer seeds per CPI | 16 signers | `program-runtime/src/cpi.rs:62`, `:664` | In force |
| Instruction trace (top-level + CPIs) | 64 | `transaction-context/src/lib.rs:26` | In force |
| Transaction size, legacy / v0 | 1232 bytes (`PACKET_DATA_SIZE = 1280-40-8`) | `solana-packet-4.2.0/src/lib.rs:42`; `agave-transaction-view-4.2.2/src/sanitize.rs:39-52` | In force |
| Transaction size, **v1 (SIMD-0385)** | **4096 bytes**, ≤64 addresses, ≤64 instructions, ≤12 signatures, no ALTs | `solana-message-4.4.1/src/versions/v1/mod.rs:36-45`; `sanitize.rs:44-47`; gate `enable_tx_v1` = `txv1aq4pp281K9um3tnPgkfX8UqtFT6wcVW3hNezGLL` (`lib.rs:1514-1516`, `:2613`) | **Active: mainnet slot 447,120,000, devnet 492,480,000.** Jupiter `/build` already offers `transactionVersion=1` and warns that not all wallets support it |
| Compute per transaction | 1,400,000 CU | `execution_budget.rs:26` | In force |
| Default compute per instruction | 200,000 CU | `execution_budget.rs:31` | In force |
| CPI invoke cost | 946 CU + serialization | `execution_budget.rs:21` | In force |
| Return data | 1024 bytes | `solana-cpi-3.1.0/src/lib.rs:330`; `syscalls/src/lib.rs:1945-1946` | In force |
| Heap frame | 32 KiB default, 256 KiB max | `execution_budget.rs:35-36` | In force |
| SBF call depth (within a program) | 64 | `execution_budget.rs:24` | In force |

**Against `docs/research.md` §2.** Its CPI depth, SIMD-0268 status, return data, 946 CU invoke cost,
1.4M CU cap and re-entrancy rows are confirmed. **Its transaction-size row is out of date:** v1
4096-byte transactions are active on mainnet now, not just "listed for Agave 4.2". Its advice to
design for 1232 bytes with lookup tables is still sound, because v1 has no lookup tables, caps at
64 addresses, and wallet support is uneven (Jupiter docs).

**Router depth budget.** Router (1) → venue (2) → Token-2022 (3) → transfer hook (4) fits. If an
external program calls our router, it is one frame deeper: still fits for Pump, Raydium and DLMM
(4–5 frames). It **fails Meteora DAMM v2 and DBC rate-limiter checks** (height 3 > 2) and leaves
nothing for Jupiter.

---

## 11. Binaries dumped (`.ref/bins/`, gitignored)

All dumps used `solana -C /tmp/no-solana-config.yml program dump -u <explicit URL>` with no keypair.
The SHA-256 covers the dumped program-data bytes, including trailing padding. Use it to detect
upgrades between test runs, not as a build hash.

| File | Program ID | Cluster | Size (bytes) |
| --- | --- | --- | --- |
| `devnet/pump.so` | `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P` | devnet | 1,996,760 |
| `devnet/pump_amm.so` | `pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA` | devnet | 1,234,704 |
| `devnet/raydium_cpmm.so` | `DRaycpLY18LhpbydsBWbVJtxpNv9oXPgjRSfpF2bWpYb` | devnet | 742,640 |
| `devnet/raydium_clmm.so` | `DRayAUgENGQBKVaX8owNhgzkEDyoHTGVEGHVJT1E9pfH` | devnet | 2,528,752 |
| `devnet/raydium_launchlab.so` | `DRay6fNdQ5J82H7xV6uq2aV3mNrUZ1J4PgSKsWgptcm6` | devnet | 1,275,544 |
| `devnet/meteora_damm_v2.so` | `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG` | devnet | 1,559,448 |
| `devnet/meteora_dlmm.so` | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` | devnet | 2,198,048 |
| `devnet/meteora_dbc.so` | `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN` | devnet | 1,983,568 |
| `mainnet/jupiter_v6.so` | `JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4` | mainnet (not on devnet) | 2,892,224 |

SHA-256:

```text
bcee35d3eeb158090dd08764a8c5c03a2852c3cee553a0ce61dd5482469a5ec7  devnet/pump.so
494d2257ed4a9a2bc3eb210b22cdb5058d77e17f1ba5095b636e96c86b1fd4ef  devnet/pump_amm.so
87ef84634086209fc2c29b4163bbe5209a1e9383aff5c429781f5fff270d75fd  devnet/raydium_cpmm.so
60255f5ce990ef6ac6847e2316400d2dc2cb83968cb885bf44d6dfac2531ad43  devnet/raydium_clmm.so
7c666612612d4156f5e9db1213d8dc7973e095c6eff0d472dab0e51179fd20c7  devnet/raydium_launchlab.so
82bb9375921bb8007551cb65f9ca43b191597496cc9922926468b36671081ec2  devnet/meteora_damm_v2.so
d8fa183bb230525d815cfc8e4504a914e19f75def59c6d0e7d47b4837321ed38  devnet/meteora_dlmm.so
5edf76d972abaf355048db5d9003bc4dfa843cd98a5f93785430dac371678ad3  devnet/meteora_dbc.so
099da3a26d336aa7174960f057546f192fc1ccda8e3e9d1b4aa5c69fa8a79a5f  mainnet/jupiter_v6.so
```

Cluster presence (RPC `getMultipleAccounts`, 2026-10-20): every mainnet ID above is executable on
mainnet. Raydium mainnet IDs are non-executable system accounts on devnet, and Raydium devnet IDs
are absent on mainnet. Meteora and Pump use the same ID on both clusters. Jupiter is mainnet only.

**CPI-guard syscall scan** (byte search of each binary for the syscall names, which appear in the
ELF dynamic symbol table when imported). This is presence-only evidence: it says a check *can*
exist, not where.

| Binary | `sol_get_stack_height` | `sol_get_processed_sibling_instruction` | "CPI disabled" string |
| --- | --- | --- | --- |
| meteora_damm_v2 | yes | yes | yes (deprecated error) |
| meteora_dbc | yes | yes | yes (deprecated error) |
| meteora_dlmm | no | no | yes (error 6100, trigger unverified) |
| pump, pump_amm, raydium_cpmm, raydium_clmm, raydium_launchlab, jupiter_v6 | no | no | no |

A program can still detect CPI without these syscalls, by reading the instructions sysvar and
comparing the current top-level program ID. None of the swap account lists above include that
sysvar except DAMM v2 and DBC (optional remaining account).

---

## 12. Cross-venue notes for the adapter design

1. **Signer model.** Every venue's swap needs a signer that is the input account's transfer
   authority (`payer` / `user` / `user_transfer_authority`). For non-custodial routing the user
   signs the outer transaction and the router forwards that signer flag. Raydium CLMM additionally
   requires `input_token_account.owner == payer`, so a delegate does not work there.
2. **Optional Anchor accounts** (DAMM v2 `referral_token_account`, DLMM `bin_array_bitmap_extension`
   and `host_fee_in`, Jupiter `destination_token_account`) are passed as the venue's program ID to
   mean None.
3. **Variable remaining accounts** (CLMM tick arrays, DLMM bin arrays, transfer-hook extras) mean the
   router instruction must carry a per-venue tail length. Hooks must not assume a fixed account
   count.
4. **Observation by token deltas** (research-pump.md §4.4) works for every venue here and is the only
   option for Jupiter. Every venue handles transfer fees internally; deltas give the net amount.
5. **Top-level placement.** For DAMM v2 and DBC, the router's swap must be the only instruction in
   the transaction that references the pool, and the router must be top level. Separate
   "pre-hook" instructions that pass the pool account would break those pools during launch windows.
6. **Lifecycle states.** LaunchLab, DBC and Pump curves all graduate. Each needs a "complete →
   migrating → new venue" state in the adapter.

## 13. Open items (verification still required)

| Item | How to verify |
| --- | --- |
| DLMM `CpiDisabled` on other pairs | Verified not raised for mainnet SOL/USDC (`meteora_dlmm_mainnet_binary_routes_both_ways`); the trigger is still unknown, so other pairs may differ |
| LaunchLab accepts CPI `buy_exact_in` and the remaining-tail order | LiteSVM with `.ref/bins/devnet/raydium_launchlab.so` and a cloned devnet pool |
| DAMM v2 / DBC rate-limiter behaviour at height 2 vs 3 | LiteSVM: create a rate-limited pool (both repos have test fixtures), call from top-level router and from a wrapper program |
| ALT-loaded accounts usable in Jupiter CPI | LiteSVM or devnet transaction with a v0 message whose CPI accounts come from a lookup table |
| `pump-rust-client 0.5.0` changes vs 0.4.1 | Diff the crate; re-run `crates/pump-adapter` tests |
