# Pump Hooks — Architecture

> A programmable execution layer for Pump markets.

Pump.fun made markets permissionless. Pump Hooks makes those markets programmable. The primary
object in this protocol is a **hooked Pump market**: an existing Pump coin (bonding curve or
PumpSwap pool) plus a `MarketConfig` and a Hook Stack. Token creation is one convenience entrance
that ends in the same `create_market` flow; it is not a launchpad and has no curve of its own.

## 1. The honest boundary

PumpSwap and the Pump bonding curve do not call external programs during a trade. Pump Hooks
therefore runs hooks **at its own router boundary**:

```
Wallet ─▶ pump_hooks::route_* ─▶ BEFORE hooks ─▶ CPI Pump / PumpSwap ─▶ AFTER hooks ─▶ events
```

Anyone can trade the same coin directly on Pump or PumpSwap and no Pump Hooks rule applies to that
trade. Every rule in this system is described as applying to **routed execution**. The UI, SDK,
README and docs repeat this. Pump's own features (fees, holder rewards, migration) are enforced by
Pump, not by us.

What the router gives users that direct trading does not:
- protections and limits they (or the market's community) chose, enforced atomically;
- loyalty accounting (Holder XP) only earned through routed flow;
- programmable execution (TWAP, conditional orders, bounded strategies) built on the same path;
- a fully decoded execution trace for every routed trade.

## 2. Components

```
                          ┌───────────────────── off-chain ─────────────────────┐
  apps/web  ─┐            │  apps/indexer  ──▶ Postgres/SQLite ──▶ apps/web API │
  packages/cli ─▶ packages/sdk ──▶ RPC         │  SENTIENT engine ──▶ signed StrategySignal │
  hook devs ─▶ packages/hook-sdk               └──────────────────────────────────────────┘
                              │
                              ▼  on-chain
  ┌──────────────────────── programs/pump-hooks ────────────────────────┐
  │ registry: GlobalConfig, HookDefinition, MarketConfig, Policy          │
  │ router:   route_amm / route_curve (mode: buy or sell)                 │
  │ runtime:  crates/hook-runtime (first-party modules, compiled in)      │
  │ policy:   crates/policy-vm (bounded evaluator)                        │
  │ adapter:  crates/pump-adapter (only place that knows Pump accounts)   │
  └─────────────────────────────────────────────────────────────────────┘
                 │ CPI (user signs)               ▲ CPI (vault PDA signs)
                 ▼                                │
         Pump (v3) / PumpSwap (v2)        programs/strategy-vault
                                          StrategyVault, TwapOrder, signal verification
```

| Path | Responsibility |
| --- | --- |
| `crates/hook-types` | Stable ABI: `HookCallback`, `HookContext`, `HookDecision`, capabilities, module IDs, reason codes, Pod layouts. `no_std`-friendly, no Solana dependency. |
| `crates/hook-runtime` | `PumpHook` trait, the first-party modules, stack execution (ordering, veto short-circuit, capability enforcement). Host-testable; the program and the simulator call the same code. |
| `crates/policy-vm` | Policy encoding, validation, and evaluation. No loops, bounded size. |
| `crates/pump-adapter` | Pump/PumpSwap program IDs, account ordering, discriminators, account readers, quote math, CPI builders. Verified against `pump-rust-client`. |
| `programs/pump-hooks` | Registry + router (one program, see §3). |
| `programs/strategy-vault` | Explicitly authorized funds, SENTIENT signal verification, TWAP orders. |
| `hooks/*` | Hook manifests (`hook.json`), docs and per-module tests. Module code lives in `hook-runtime` for V1. |
| `packages/sdk` | `PumpHooksClient`: market discovery, quotes, tx building, simulation, traces. |
| `packages/hook-sdk` | Manifest schema, ABI types for TS, `create-pump-hook` scaffolder. |
| `packages/cli` | `pump-hooks` CLI over the SDK. |
| `apps/indexer` | Resumable, idempotent indexer for routed swaps, hook executions, policies, signals. |
| `apps/web` | Marketing, `/hook`, `/hooks`, `/build`, `/market/[mint]`, `/tx/[sig]`, `/simulate`. |

## 3. Why registry and router are one program

The brief proposed four programs. Research changed that:

1. **State ownership.** After-hooks write hook state (Holder XP, rolling volume, EMA). Only the
   owning program may write an account. If the registry owned `MarketConfig` and the router ran the
   hooks, every after-hook write would need a CPI back into the registry.
2. **Compute.** A CPI costs ~1,000 CU base plus account serialisation. With four hooks that is
   avoidable overhead on the hottest path in the protocol.
3. **CPI depth.** Solana allows instruction stack height 5. Strategy execution is
   `strategy_vault → pump_hooks → PumpSwap → Token`, which already uses four levels. A separate
   registry or policy program would leave no headroom.
4. **Policy engine** is a library evaluated in-process; it does not need its own program.

`strategy-vault` stays separate because it custodies explicitly deposited funds and should have a
small, independently reviewable surface and separate upgrade authority.

## 4. Accounts (PDAs)

All PDAs use fixed seeds; no variable-length strings. Hot-path accounts are `zero_copy` (Pod) to
avoid Borsh cost.

| Account | Seeds | Owner | Purpose |
| --- | --- | --- | --- |
| `GlobalConfig` | `["global"]` | pump-hooks | admin, pause flags, `disabled_modules` bitmask (kill switch), fee-free protocol |
| `HookDefinition` | `["hook", module_id u16 LE]` | pump-hooks | metadata: version, callbacks, capabilities, `source_hash`, `verified`, `audit_status`, `security_status`, counters |
| `MarketConfig` | `["market", mint, authority]` | pump-hooks | the hooked market: venue, pool, flags, stats, sequence, up to 6 inline hook slots |
| `UserMarketState` | `["user", market, user]` | pump-hooks | per-wallet state shared by modules: XP, vesting, cooldowns, routed volume, holding checkpoints |
| `Policy` | `["policy", market, index u8]` | pump-hooks | one compiled Composer policy |
| `StrategyVault` | `["vault", market]` | strategy-vault | allocation buckets, SENTIENT signer and bounds, nonce, regime |
| `VaultAuthority` | `["vault-authority", vault]` | system (no data) | holds lamports, signs CPIs; must be data-less so it can pay rent inside Pump CPIs |
| `TwapOrder` | `["twap", vault, id u32]` | strategy-vault | bounded scheduled execution of vault funds |

`HookInstallation` from the brief is represented by **inline slots** in `MarketConfig` (one account
read for the whole stack instead of one per hook). External hooks, when enabled, will get a
separate `HookInstallation` PDA because they own their own state.

### 4.1 Market authority (who may hook a token they did not create)

Hooks only affect routed flow, so hooking a market is not a privilege over other people's tokens.
Still, a community needs to know whose rules they are trading under. Model:

- `MarketConfig` is namespaced by `authority`: anyone can create a config for any Pump mint.
- At creation the program reads Pump's `BondingCurve.creator` (it survives migration). If the
  signer equals it, the market is flagged `CREATOR_VERIFIED`.
- The SDK and UI resolve **the creator-verified market first**; otherwise they list community
  markets with their authority shown. Holder-rewards coins record a pump.fun-controlled creator, so
  they can only have community markets.
- This avoids squatting (no first-come canonical PDA) and avoids takeover logic.

This is an architectural decision that could be revisited; it is documented, not hidden.

## 5. Routed trade lifecycle

`route_amm(mode, amount, limit)`; `route_curve` is symmetric, and `mode` selects buy or sell:

1. **Validate protocol**: `GlobalConfig` not paused for routing.
2. **Validate market**: `MarketConfig` PDA matches seeds, `ENABLED`, venue = PumpSwap,
   `market.pool == pool`, `market.mint == base_mint`, quote mint matches.
3. **Validate venue accounts** (adapter): PumpSwap program ID, `pool` owned by PumpSwap with the
   `Pool` discriminator, `pool.base_mint/quote_mint/vaults` equal the passed accounts, user token
   accounts owned by `user` with the right mints, token programs are SPL Token or Token-2022,
   not a cashback pool. Pump validates its own PDAs; we validate everything that binds the trade to
   *this* market.
4. **Load user state**: `UserMarketState` (`init_if_needed`, payer = user, ~0.0017 SOL once).
5. **Build `HookContext`** from account data only: reserves (effective quote reserves with signed
   virtual reserves), estimated quote size for the trade, clock, market sequence.
6. **Before hooks**: run `BeforeBuy` then `BeforeSwap` subscribers in priority order. First denial
   aborts with `HookDenied` (transaction reverts, no swap). Each execution produces a trace entry.
7. **Snapshot** user base/quote token balances.
8. **CPI** into PumpSwap `buy_v2`/`buy_exact_quote_in_v2` with the user as signer. The router never
   takes custody; tokens move user ↔ pool directly.
9. **Observe**: re-read balances, compute actual `base_delta` / `quote_delta`; sanity-check against
   the user's limit (fail closed if Pump's result is inconsistent).
10. **After hooks**: `AfterBuy`, `AfterSwap` subscribers. Any error reverts the whole transaction.
11. **Commit**: increment `market.sequence`, stats; emit `HookExecutionEvent` per hook and one
    `RoutedSwapEvent` with the packed trace.

### Atomicity

Solana transactions are atomic: any error after the CPI rolls back the swap and all hook state.
The router therefore needs no compensation logic. Fail-closed rules for V1:

| Failure | Result |
| --- | --- |
| Required before-hook denies or errors | No swap, tx reverts |
| Pump CPI fails (slippage, paused, complete curve…) | tx reverts, no hook state persists |
| After-hook errors | tx reverts, swap undone |
| Disabled module in `disabled_modules` | routed trades on markets using it revert with `ModuleDisabled` until the market removes it (fail closed) |

Optional telemetry-only modules (non-reverting) are a later feature.

## 6. Venues and graduation

`MarketConfig.venue` is `BondingCurve` or `PumpSwap`. When a curve completes and migrates, anyone
can call `sync_market`, which verifies the canonical pool (PumpSwap PDA
`["pool", 0u16, pool_authority(mint), mint, quote_mint]`) exists with matching mints and switches the
venue. Between completion and migration, routed trades deny with `MarketMigrating`.

## 7. Strategy execution path (TWAP, SENTIENT, conditional orders)

```
DATA ─▶ MODEL (off-chain) ─▶ StrategySignal (ed25519) ─▶ strategy-vault verifies bounds
     ─▶ TwapOrder / state change ─▶ keeper crank ─▶ vault PDA signs ─▶ pump_hooks::route_* ─▶ Pump
```

Strategy trades go through the same hook stack, so a market's own guards apply to its own
treasury. The vault can only spend its `strategy` bucket, only through the router, only within
per-slice size, slippage, cooldown and epoch budget limits.

Policies created in the Composer cannot CPI. Policy actions that affect a vault (`QUEUE_TWAP`,
`ALLOCATE_AUTHORIZED_TREASURY`, `PAUSE_STRATEGY`, …) write a pending **intent** into the `Policy`
account; the vault program's permissionless `apply_policy_intent` validates the intent against the
vault's own bounds before acting.

## 8. Transactions built by the SDK

A routed SOL buy on PumpSwap:

```
ComputeBudget::SetComputeUnitLimit
ComputeBudget::SetComputeUnitPrice           (optional)
ATA::CreateIdempotent (user base ATA)
ATA::CreateIdempotent (user WSOL ATA)
System::Transfer + Token::SyncNative          (wrap)
pump_hooks::route_amm                         (5 + 17 + policy/strategy accounts)
Token::CloseAccount (WSOL)                    (unwrap)
```

v0 transactions with Pump's published lookup table plus our own (global config, programs).
Bonding-curve SOL trades use native SOL (no wrap).

## 9. Off-chain

- **Indexer**: subscribes to `pump_hooks` logs and polls signatures; decodes Anchor events;
  stores last processed slot + signature; upserts by `(signature, ix_index, event_index)` so
  replays are idempotent. Pump trade events for hooked markets are stored for simulation input.
- **Simulator**: replays historical Pump trades (from the indexer or RPC) through
  `hook-runtime` compiled to WASM/Node (same code as on-chain), reporting allowed/denied/reason
  codes and estimated CU. It says "routed-equivalent" because historical trades were not routed.
- **SENTIENT engine**: computes signals from indexed data and signs `StrategySignal`s with a key
  that the vault bounds. It never holds funds.

## 10. Upgradeability

- Programs are upgradeable during devnet; mainnet upgrade authority goes to a multisig with a
  documented timelock before any mainnet deployment (not done without explicit approval).
- Account layouts carry `version` and `reserved` bytes; ABI discriminants are append-only.
