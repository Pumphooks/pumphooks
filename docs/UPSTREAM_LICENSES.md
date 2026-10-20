# Upstream licenses

Checked 2026-10-20 by reading each repository's `LICENSE` file and the `license` fields in its
`Cargo.toml` / `package.json`. Pump Hooks is Apache-2.0 (`LICENSE`). None of the repositories below
ships a `NOTICE` file.

| Repository | License | How Pump Hooks uses it |
| --- | --- | --- |
| [LiteSVM/litesvm](https://github.com/LiteSVM/litesvm) | Apache-2.0 (Node binding crate: MIT) | Dependency: Rust `litesvm` 0.16 in tests; npm `litesvm` in `packages/execution-lab`. No code copied. |
| [anza-xyz/mollusk](https://github.com/anza-xyz/mollusk) | Apache-2.0 | Dependency: `mollusk-svm` / `mollusk-svm-bencher` 0.15.1 in `bench/`. No code copied. |
| [Ackee-Blockchain/trident](https://github.com/Ackee-Blockchain/trident) | MIT | Optional dependency in a separate workspace (`trident-tests/`, Agave 2.x). No code copied. |
| [patcha-fi/patcha](https://github.com/patcha-fi/patcha) | MIT, © 2026 Patcha Core | Studied. No code copied. The size-ramped dynamic fee (`fee = base + (max − base)·min(amount, pivot)/pivot`) is reimplemented as one mode of our `dynamic-fee` builtin; attributed in `crates/hook-kernel/src/builtin.rs`. |
| [solana-foundation/program-examples](https://github.com/solana-developers/program-examples) | MIT | Studied (Token-2022 transfer-hook examples). No code copied. |
| [solana-program/transfer-hook](https://github.com/solana-program/transfer-hook) | Apache-2.0 | Interface facts (discriminators, validation PDA seeds) used; crate `spl-transfer-hook-interface` used as a dependency. |
| [solana-program/token-2022](https://github.com/solana-program/token-2022) | Apache-2.0 | Dependency (`spl-token-2022` via `anchor-spl`). |
| `spl-tlv-account-resolution` | Apache-2.0 | Dependency. |
| `solana-gateway` 0.5.0 (crates.io, Identity.com) | Apache-2.0 | Gateway-token account layout used to implement credential checks (`credential` builtin). No code copied. |
| [civicteam/token-extensions-transfer-hook](https://github.com/civicteam/token-extensions-transfer-hook) | **No license file** (all rights reserved by default) | Studied only. Nothing copied. |
| [Squads-Protocol/smart-account-program](https://github.com/Squads-Protocol/smart-account-program) | **AGPL-3.0** | Studied only. Nothing copied; `programs/smart-accounts` is an independent implementation. |
| [Squads-Protocol/v4](https://github.com/Squads-Protocol/v4) | **AGPL-3.0** (Rust SDK and CLI: MIT OR Apache-2.0) | Studied only. Nothing copied. |
| [orca-so/whirlpools](https://github.com/orca-so/whirlpools) | **Orca License** since 2025-02-27: non-commercial, excludes competitors (DEXs, AMMs, aggregators, routers) without written consent. Apache-2.0 before. | No code, crate or SDK used. The router adapter only builds the on-chain `swap_v2` instruction from its public account order. Whether that needs Orca's consent is a legal question; the adapter ships **disabled** until that review happens. |
| [raydium-io/raydium-clmm](https://github.com/raydium-io/raydium-clmm), [raydium-cp-swap](https://github.com/raydium-io/raydium-cp-swap) | Apache-2.0 | Interface facts only; no code copied. |
| [MeteoraAg/dlmm-sdk](https://github.com/MeteoraAg/dlmm-sdk) | **No license file** (TS client: ISC) | IDL facts only (program interface); no code copied. |
| [MeteoraAg/damm-v2](https://github.com/MeteoraAg/damm-v2) | **Meteora Noncommercial Licence** (`license.md`) | Interface facts only (account order, discriminators, pool account offsets); no code copied. Tests load the deployed binary dumped from the public cluster. |
| [jup-ag/jupiter-cpi](https://github.com/jup-ag/jupiter-cpi) | No license file; Cargo says Apache-2.0; archived | Not used. |

## Rules we follow

- Copyleft (AGPL) and unlicensed code is never copied into this repository.
- Program interfaces (account orders, discriminators, account layouts) are used to build
  instructions and decode accounts; the source of each is cited next to the code.
- Anything adapted from a permissive project names the project and license next to the code and in
  this file.
