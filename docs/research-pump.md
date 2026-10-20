# Pump Hooks — Research

Status: Phase 0. Sources inspected on 2026-10-20. Every claim about Pump interfaces below comes from
`pump-fun/pump-public-docs` (commit `639b87f`, 2026-10-13), its IDLs, and the published
`pump-rust-client 0.4.1`, `@pump-fun/pump-sdk 4.1.0` and `@pump-fun/pump-swap-sdk 2.1.0` packages.
Where this document disagrees with the original product brief, the documentation wins.

---

## 1. Pump: what is actually available today

### 1.1 Programs

| Program | ID | Notes |
| --- | --- | --- |
| Pump (bonding curve) | `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P` | Mainnet and devnet |
| PumpSwap (pump_amm) | `pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA` | Mainnet and devnet |
| Pump Fees | `pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ` | Fee schedule (`fee_config`) |
| Mayhem | `MAyhSmzXzV1pTf7LsNkrNwkWKTo4ougAJ1PPg47MD4e` | Required accounts on `create_v2` |

`pump-rust-client 0.4.1` ships the compiled `pump.so`, `pump_amm.so`, `pump_fees.so`,
`mayhem_program.so` and `pump_agent_payments.so` in `artifacts/`, plus a tool that clones the
admin-initialised PDAs (`Global`, `GlobalConfig`, `FeeConfig`, mayhem params, lookup tables) from a
cluster. **This is the single most important finding for testing:** we can run our router against
Pump's real bytecode on a local validator instead of mocks.

### 1.2 Trade instructions we route

The brief named PumpSwap `buy_v2 / sell_v2 / buy_exact_quote_in_v2`. Those exist. In addition, the
bonding-curve program now has a *smaller* generation:

| Venue | Instructions | Accounts | Fee behaviour |
| --- | --- | --- | --- |
| Bonding curve | `buy_v3`, `buy_exact_quote_in_v3`, `sell_v3` | 17, no remaining accounts | Protocol + creator fee kept on the curve, swept later |
| Bonding curve | `buy_v2`, `buy_exact_quote_in_v2`, `sell_v2` | 27 / 26 | Fees paid per trade (needed only for cashback coins) |
| PumpSwap | `buy_v2`, `buy_exact_quote_in_v2`, `sell_v2` | 17, no remaining accounts | Protocol + creator fee kept in the pool, swept later |
| PumpSwap | `buy`, `sell`, `buy_exact_quote_in` | 23+ with remaining | Legacy |

**Decision:** the router CPIs into bonding-curve **v3** and PumpSwap **v2** only. Both are 17
accounts with a fixed order and no optional accounts, which is what makes a hooked trade fit in one
transaction. Cashback coins (deprecated; v3/v2 fail with `CashbackCoinNotSupported`) are explicitly
unsupported in V1 and the adapter rejects them before CPI.

Discriminators (from the IDLs, verified again in adapter tests against `pump-rust-client`):

| Instruction | Discriminator |
| --- | --- |
| amm `buy_v2` | `[184,23,238,97,103,197,211,61]` |
| amm `buy_exact_quote_in_v2` | `[194,171,28,70,104,77,91,47]` |
| amm `sell_v2` | `[93,246,130,60,231,233,64,178]` |
| pump `buy_v3` | `[7,5,29,196,245,23,101,80]` |
| pump `buy_exact_quote_in_v3` | `[225,247,80,30,213,179,132,136]` |
| pump `sell_v3` | `[28,146,222,119,38,196,105,213]` |
| pump `create_v2` | `[214,144,76,236,95,139,49,180]` |

### 1.3 State we read on-chain

- `Pool` (pump_amm): `base_mint@43`, `quote_mint@75`, `pool_base_token_account@139`,
  `pool_quote_token_account@171`, `coin_creator@211`, `is_mayhem_mode@243`, `is_cashback_coin@244`,
  `virtual_quote_reserves: i128@245` (**can be negative since 2026-09-30**), `is_holder_reward@270`,
  `protocol_fees@271`, `creator_fees@279`. Older pools are shorter; missing trailing fields read as 0.
- Effective quote reserves = `pool_quote_token_account.amount + virtual_quote_reserves` (signed add,
  Pump guarantees the result is non-negative and does not overflow). Base reserves are the raw vault
  balance.
- `BondingCurve`: `virtual_token_reserves@8`, `virtual_quote_reserves@16`, `real_token_reserves@24`,
  `real_quote_reserves@32`, `complete@48`, `creator@49`, `is_cashback_coin@82`, `quote_mint@83`
  (`Pubkey::default()` = SOL), `is_holder_reward@124`.
- The official CPI guide recommends `pump_rust_client::state` wrappers because they zero-pad short
  legacy accounts. Our on-chain adapter reads fixed offsets with the same zero-pad rule (cheaper than
  full Borsh deserialisation in the hot path); host tests assert equality with the official wrappers.

### 1.4 Fees, sweeps, holder rewards

- v2/v3 trades keep protocol and creator fees in the curve/pool; anyone can call
  `sweep_protocol_fee` / `sweep_creator_fee`. The destination is fixed by Pump. **Pump Hooks never
  redirects these fees.** A creator may voluntarily deposit fees they have already collected into a
  Strategy Vault; that is the only way protocol-controlled funds come into existence.
- Holder rewards coins (`is_holder_reward`) route the creator fee to holders and are paid by
  pump.fun. There is no holder claim instruction. Our "Holder First" preset can *display* and
  *recommend* holder rewards (at creation time it is a `create_v2` flag), but we do not claim to
  distribute them.
- Buyback part of the protocol fee is still paid per trade to one of 8 `buyback_fee_recipients`
  (must exist; for WSOL/USDC it does).

### 1.5 Quote mints, Token-2022, migration

- `create_v2` coins are Token-2022 (6 decimals). Older coins are SPL Token. The adapter carries both
  token program IDs and reads token-account balances at the common offset (amount @64), which is
  identical for SPL Token and Token-2022 base layouts.
- Quote mint: SOL (wrapped SOL mint passed, native SOL moved on the curve; WSOL token account on
  PumpSwap), USDC, or another pump coin (depth ≤ 2 chains). V1 supports SOL and USDC quotes; pump-coin
  quotes are allowed by the adapter but treated as "unpriced" (no SOL value conversions).
- Synthetic migration: the v3 buy that empties a curve can continue into the not-yet-created pool.
  Router consequence: post-trade observation must use **token-account deltas**, not curve reserves,
  because that buy emits three events and the curve completes mid-instruction.
- After completion, curve trades fail with `BondingCurveComplete` until `migrate_v2`. The router
  treats "complete && no pool" as `MarketMigrating` and denies with a clear reason instead of letting
  Pump fail.
- Permissionless pools: PumpSwap pools with any `index`/`creator`. A market may point at a
  non-canonical pool only if the market authority explicitly selects it, and the UI labels it.

### 1.6 Compute and transaction size

Pump's FAQ recommends a static ~100k CU for a buy/sell and notes CU varies with bump derivation and
whether the curve completes. The v2/v3 instructions were explicitly introduced so "more fits in one
transaction". Pump publishes address lookup tables (`Hyif6eW…` mainnet, `7Je3GuL…` devnet) which we
use for v0 transactions.

---

## 2. Patcha (`patcha-fi/patcha`, commit `3d4123d`)

| | |
| --- | --- |
| Architecture | Toolchain-free Rust runtime (`hook-runtime`) + Anchor executor (`register_hook`, `install_hook`, `trigger_hook`) + TS metadata library. Targets Orca/Raydium CLMMs. |
| Interesting primitive | Same decision logic in a host crate and in the program, so simulation and on-chain agree. Folding rules: install order, first veto short-circuits, last fee override wins. |
| Reusable conceptually | Host-testable runtime crate shared with the program and the simulator; stable callback tags; params blob schema-checked at install. |
| Does not translate | `trigger_hook` is a **standalone instruction** whose `amount_in` and `tick` are caller-supplied. Nothing binds a hook evaluation to an actual swap, so a caller can trigger hooks with fabricated context or skip them. Fee overrides have no effect on venues that do not consume them. |
| Security observations | `install_hook` has no pool-authority check (anyone can install on any pool). Builtin hooks are marked `audited = true` by default. PDAs use variable strings. The caller is used as the gated identity. |
| PumpHooks adaptation | Keep the shared host/on-chain runtime idea. Reject the standalone trigger: in Pump Hooks, hooks only run **inside** the router instruction that performs the CPI, with context derived from on-chain account data, and post-hooks see observed token deltas. Market authority is explicit; "verified/audited" are separate fields that default to false. |

---

## 3. Uniswap v4 hook ecosystem — ideas, not code

Uniswap v4 hooks are called by the PoolManager itself, so they are unbypassable for that pool. On
Pump there is no such callback; everything below is translated to the **router boundary**, which
means "applies to routed execution only".

| Repository / source | Primitive | Portable idea | Does not translate | Security notes | Pump Hooks adaptation |
| --- | --- | --- | --- | --- | --- |
| `CryptoGnome/v4hooks` catalog (52 entries: first-party teaching hooks + pinned links to Clanker, Zora, Doppler, Flaunch, OpenZeppelin, EulerSwap…) | Machine-readable hook catalog with permission flags, callbacks, audit/license metadata | A registry entry schema with callbacks + capabilities + audit status + source, and an explicit "catalog is not an audit" warning | Address-bit permission flags (v4 encodes permissions in the hook address) | Catalog explicitly separates "source confirmed" from "safe" | `hook.json` manifest and on-chain `HookDefinition` carry callbacks, capabilities, source hash, `verified` and a separate `audit_status` that defaults to `NONE` |
| `v4hooks/SniperTax`, `MevWindow`, Clanker `ClankerMevDescendingFees` | Fee that decays (linear / parabolic) after pool init; same-block trading disallowed | Time-since-activation phases; quadratic decay schedule | We cannot change Pump's fee. A router-side tax would just push traders to direct Pump | Fee taxes on routed flow are bypassable and create adverse selection against our own users | **Launch Shield** uses phases for *limits* (max routed size, per-wallet cooldown, per-slot buy cap) instead of taxes. No fee extraction. |
| Clanker `ClankerSniperAuctionV0/V2` | Recurring sniper auction: the right to trade in an early block is auctioned via gas-price peg; proceeds split | Auctioning early access instead of letting priority fees decide | Requires the venue to enforce who trades first; Pump does not | Auction only meaningful if unbypassable | Recorded as **not implementable** on Pump. Documented so nobody claims it. |
| `akshatmittal/v4-twamm-hook` (Uniswap Labs + Zaha, audited by ABDK & Certora) | Rate-based long-term orders executed before any other swap; opposing flows net out | Rate accounting instead of per-order loops; bounded intervals; explicit MEV and information-leakage disclosure | Netting opposing orders across users would make us a custodial matcher; "execute before other swaps" is not enforceable on Pump | Orders are public → information leakage; slices are MEV targets | **TWAP** executes only for *authorized vault funds*, one bounded slice per crank, with per-slice slippage and cooldown. Slices route through the same hook stack. No user-order netting in V1. |
| `saucepoint/v4-stoploss`, `v4hooks/StopLoss`, `TakeProfit`, `TrailingStop`; `impetus82/airbag-hook`, `il-aware-limit-order-hook` | Orders filled in `afterSwap` when the tick crosses; receipt tokens; Airbag compensates makers for overshoot measured inside the same tx without an oracle | Measuring price only from in-transaction state (no oracle); claimable receipts | Orders filled by the AMM's own swap loop. Pump has no ticks and will not call us | A user's order filled by a keeper needs pre-committed bounds so the keeper cannot grief | **Conditional Orders**: user escrows only the amount for that order in a per-order PDA; a permissionless keeper executes it through the router when on-chain reserves cross the trigger price; min-out is fixed at order creation. Phase 13. |
| `shift0x/uniswap-v4-prediction-market-hook`, `bithookdev/bithook_mono`, `ParsaAbolhasani/v4-prophecy-hook` | Prediction markets settled on the pool's own price; Bithook uses commit–reveal on the pool's own TWAP | Settling on the market's own observable price; commit–reveal to avoid copying | Settlement against a single Pump pool price is manipulable by a direct trade right before settlement | Price manipulation near expiry is the main risk; the v4 hook mitigates via arbitrage incentives we cannot give | Listed as **EXPERIMENTAL** only. If built: TWAP-of-observations window, commit–reveal, and a loud manipulation disclaimer. Not in V1. |
| `launch-fi/launchpad` | Exponential and gradual Dutch-auction curves with decaying LP fee in a single v4 pool | Dutch-auction price discovery | We do not build curves or launch mechanics — Pump does | — | Not adopted. Out of scope by product principle. |
| `Aperture-Finance/uniswap-v4-hooks` | JIT-LP fee redistribution; hook-native LP automation | Separating passive and JIT liquidity incentives | PumpSwap LP is not hook-aware; JIT on PumpSwap is not observable from our router | — | Recorded under LIQUIDITY as research only. |
| OpenZeppelin `AntiSandwichHook`, `LiquidityPenaltyHook` (via catalog) | Anti-sandwich by constraining same-block price movement; JIT penalty | Per-slot movement caps | Sandwichers will not use our router | Claiming MEV protection for bypassable flow is dishonest | Launch Shield's per-slot cap and the "no buy and sell by the same wallet in one slot through the router" rule protect **routed users from each other only**. Docs say exactly that. |
| `Powellgraham5/Karma-protocol` | Reputation score → cheaper fees, scored by an off-chain agent written to an on-chain registry | Loyalty as a first-class market primitive | Fee discounts impossible on Pump | Off-chain scorer is a trusted oracle | **Holder XP** is computed on-chain from routed activity only; no off-chain scorer can mint XP. |
| `v4hooks/NthBuyPot`, `ReferralFee`, `VolumeTier`, `LpRewards`, `Super-DCA` gauge | Deterministic per-counter rewards, tiered volume programs | Counter advances at most once per slot (anti-spam) | Pot/referral funding comes from swap fees we do not control | Wash trading is the dominant exploit of volume rewards | XP accrual is vested and forfeited on sell, capped per day, and never rewards sells. |
| `v4hooks/TradingDays`, `bennoprice/univ4 TradingHours`, `AllowlistSwap`, Uniswap `PermissionedHooks`, `purefi VerifierHook` | Time windows, allowlists, attestation-gated swaps | Policy conditions on time and identity | Gating only applies to routed flow | A gated router market can be traded freely on Pump directly | ACCESS hooks are available through the Policy VM (time conditions) but documented as "router-only". Allowlists via Merkle root are Phase 13+. |
| `v4hooks/VolatilityFee`, `SurgeFee`, `MedianOracle`, Uniswap `VolatilityOracle`, `GeomeanOracle` | Volatility estimated from in-pool observations | EMA / median of observed prices; surge detection | No callback on every Pump trade, so our observations are sparse | Sparse observations can lag; a manipulator can move the pool then route | **Volatility Guard** compares current on-chain spot (from reserves) to an EMA of routed observations plus permissionless `observe` cranks; documents sparsity. |
| `v4hooks/Buyback`, `AutoBurn`, `FeeRouter`, `atj3097/buyback-hook` | Fee-funded buybacks | Rules-bound buybacks from an explicit treasury | Fees are Pump's to route, not ours | Buybacks timed to front-run users are manipulation | Buybacks only from a Strategy Vault funded voluntarily; executed as bounded TWAP slices through the router. Never timed against users. |
| `Paracausal-Labs/Agentropolis`, `tuanhoaterraria1/aura`, `luisfernandomendozav/fiducia-solana`, `Tobiinsaurralde/turnkey-agent-wallet`, `Rzbyte/safehands-solana` | AI agents propose, a deterministic policy layer decides; agent wallets that cannot be drained | "Model proposes, policy disposes" | LLM council debate is theatre unless execution is bounded on-chain | Prompt-injected agents will sign anything they are allowed to | **SENTIENT**: off-chain engine signs a compact `StrategySignal`; the on-chain vault verifies signer, nonce, freshness, action bitmask, size, budget, cooldown and slippage. The key can only do what the vault already permits. |
| `marronjo/iceberg-cofhe`, `Najnomics/StealthAuction`, `vrag99/dBay` | FHE-encrypted orders / Dutch auctions | Hidden order sizes | No FHE coprocessor on Solana mainnet | — | Not adopted. |
| `hensha256/v2-on-v4`, `saucepoint/v4-constant-sum`, Doppler | Custom curves inside v4 | — | We do not replace Pump's AMM | — | Not adopted. |

### 3.1 Concepts that survive the trip to Solana/Pump

1. **Hook stack with deterministic order** and first-veto short-circuit (v4, Patcha).
2. **Capability flags** declared per hook and enforced by the runtime (v4 permission bits →
   explicit capability bitmask checked at install and at dispatch).
3. **Shared host/on-chain logic** so simulation equals execution (Patcha).
4. **Phase/time-based launch protection** expressed as limits, not taxes (Zora/Clanker decay).
5. **Rate-based TWAP** with bounded slices and honest MEV disclosure (Uniswap TWAMM).
6. **Oracle-free in-transaction measurement** (Airbag): post-hooks read actual token deltas.
7. **Model proposes, policy disposes** for autonomous strategies (SafeHands, Turnkey agents).
8. **Anti-wash loyalty**: vesting + forfeiture + caps rather than raw volume rewards.

### 3.2 Concepts that do not survive

- Anything that relies on the venue calling us (fee overrides, JIT detection inside the swap,
  sniper auctions, "execute before every swap").
- Anything that redirects fees we do not control.
- "MEV protection" for transactions that never touch our router.

---

## 4. What this means for the architecture

1. Hooks must run **inside** the routed trade instruction, in the same program that performs the
   CPI, so context cannot be fabricated and atomicity is free (Solana reverts the whole transaction
   on any error).
2. V1 hooks are **first-party modules compiled into the router** (no CPI per hook). This removes
   ~1–5k CU per hook of CPI overhead, removes the write-ownership problem (hook state is owned by the
   program that mutates it), and keeps CPI depth at router → Pump → Token. The Hook ABI is still
   defined as a CPI interface so external hooks can be enabled later behind the `EXTERNAL_CPI`
   capability.
3. Registry and router are one program (`pump-hooks`). See `architecture-pump-router.md` §3 for the reasoning.
4. The router must observe post-trade state from **token-account balances** (synthetic migration,
   kept fees and negative virtual reserves all make reserve-diffing unreliable).
5. The primary user object is an **existing Pump market**. Token creation is a thin convenience
   wrapper around `create_v2` that ends in the same `create_market` flow.
