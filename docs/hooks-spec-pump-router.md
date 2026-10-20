# Pump Hooks — Hook ABI Specification (v1)

This is the stable interface between the router and hooks. Discriminants and layouts are
**append-only**: never reorder, never reuse a retired value. The source of truth is
`crates/hook-types`; this document mirrors it and tests assert the sizes and offsets below.

All integers are little-endian. All structs are `#[repr(C)]`, `Pod`, 8-byte aligned.

## 1. Callbacks — `HookCallback: u8`

| Value | Name | Phase | Status |
| --- | --- | --- | --- |
| 0 | `BeforeBuy` | before | v1 |
| 1 | `AfterBuy` | after | v1 |
| 2 | `BeforeSell` | before | v1 |
| 3 | `AfterSell` | after | v1 |
| 4 | `BeforeSwap` | before (both sides) | v1 |
| 5 | `AfterSwap` | after (both sides) | v1 |
| 6 | `BeforeLiquidity` | before | reserved |
| 7 | `AfterLiquidity` | after | reserved |
| 8 | `BeforeFeeAction` | before | reserved |
| 9 | `AfterFeeAction` | after | reserved |
| 10 | `OnGraduation` | event | reserved |
| 11 | `OnStrategySignal` | event | reserved |

A hook subscribes with a `u16` mask `1 << callback`. For a buy the router dispatches `BeforeBuy`,
`BeforeSwap`, CPI, `AfterBuy`, `AfterSwap`. Each subscribed hook runs once per dispatched callback
it subscribes to.

## 2. `HookContext` (272 bytes)

| Offset | Field | Type | Notes |
| --- | --- | --- | --- |
| 0 | `version` | u8 | `1` |
| 1 | `callback` | u8 | `HookCallback` |
| 2 | `venue` | u8 | `0` BondingCurve, `1` PumpSwap |
| 3 | `side` | u8 | `0` Buy, `1` Sell |
| 4 | `mode` | u8 | `0` ExactBaseOut (buy), `1` ExactQuoteIn (buy), `2` ExactBaseIn (sell) |
| 5 | `flags` | u8 | see below |
| 6 | `slot_index` | u8 | index of the executing hook slot |
| 7 | `_reserved` | u8 | 0 |
| 8 | `market` | [u8;32] | `MarketConfig` address |
| 40 | `user` | [u8;32] | trader (or vault authority for strategy execution) |
| 72 | `base_mint` | [u8;32] | Pump coin |
| 104 | `quote_mint` | [u8;32] | WSOL mint for SOL-paired |
| 136 | `pool` | [u8;32] | PumpSwap pool, or bonding curve PDA |
| 168 | `amount` | u64 | user-specified exact amount (base out / quote in / base in) |
| 176 | `limit` | u64 | slippage bound (max quote in / min base out / min quote out) |
| 184 | `quote_amount` | u64 | before: estimated quote leg; after: observed |
| 192 | `base_amount` | u64 | before: estimated base leg; after: observed |
| 200 | `pre_base_reserves` | u64 | from venue accounts |
| 208 | `pre_quote_reserves` | u64 | effective (signed virtual reserves applied) |
| 216 | `post_base_reserves` | u64 | after only, else 0 |
| 224 | `post_quote_reserves` | u64 | after only, else 0 |
| 232 | `timestamp` | i64 | `Clock::unix_timestamp` |
| 240 | `slot` | u64 | `Clock::slot` |
| 248 | `sequence` | u64 | market sequence number of this routed trade |
| 256 | `market_activated_at` | i64 | `MarketConfig.created_at` |
| 264 | `user_base_balance` | u64 | before: pre-trade; after: post-trade |

`flags`: bit0 `QUOTE_NATIVE_SOL`, bit1 `BASE_TOKEN_2022`, bit2 `HOLDER_REWARD_COIN`,
bit3 `MAYHEM`, bit4 `CREATOR_VERIFIED_MARKET`, bit5 `STRATEGY_EXECUTION`, bit6 `AMOUNTS_OBSERVED`.

Estimates in before-callbacks use constant-product math on the pre-trade reserves without fees.
They are deterministic and derived only from on-chain account data.

## 3. `HookDecision` (8 bytes)

| Offset | Field | Type |
| --- | --- | --- |
| 0 | `allow` | u8 (0/1) |
| 1 | `_pad` | u8 |
| 2 | `reason_code` | u16 |
| 4 | `flags` | u32 (bit0 `STATE_WRITTEN`, bit1 `SIGNAL_EMITTED`, bit2 `POINTS_AWARDED`) |

After-callbacks must return `allow = 1`; a hook that wants to fail an after-callback returns an
error, which reverts the transaction.

## 4. Reason codes — `u16`

| Range | Owner |
| --- | --- |
| 0 | OK |
| 100–999 | router / adapter |
| `module_id × 1000 + n` | first-party module `module_id` (e.g. Launch Shield 1001…) |
| 9000–9999 | Composer policies (`9000 + policy-defined code`) |
| 10000+ | external hooks (`10000 + hook-defined`) |

## 5. Module IDs — `u16`

| ID | Module | Runtime? | Default callbacks | Priority namespace |
| --- | --- | --- | --- | --- |
| 1 | Launch Shield | yes | BeforeBuy, BeforeSell | 0–999 security |
| 2 | Volume Guard | yes | BeforeSwap, AfterSwap | 1000–1999 risk |
| 3 | Volatility Guard | yes | BeforeSwap, AfterSwap | 1000–1999 risk |
| 4 | Holder XP | yes | AfterBuy, AfterSell | 2000–2999 accounting |
| 5 | Treasury Router | config-only | — | 2000–2999 accounting |
| 6 | TWAP | config-only (executes via vault) | — | 3000–3999 strategies |
| 7 | Conditional Orders | reserved | — | 3000–3999 strategies |
| 8 | SENTIENT | yes | BeforeSwap, AfterSwap | 3000–3999 strategies |
| 9 | Policy (Composer) | yes | from policy triggers | any (authority chooses) |

Execution order: ascending `(priority, slot_index)`. Slots are stored sorted at install time so the
router iterates linearly. Ties are impossible because `slot_index` is unique.

Runtime cap: **4 runtime hooks** per market (`MAX_RUNTIME_HOOKS`), 6 slots total
(`MAX_HOOK_SLOTS`) so config-only modules fit. Revisit after `docs/benchmarks.md`.

## 6. Capabilities — `u32` bitmask

| Bit | Capability | Meaning |
| --- | --- | --- |
| 0 | `READ_MARKET` | read context and market stats |
| 1 | `VETO_ROUTED_SWAP` | may return `allow = 0` in before-callbacks |
| 2 | `WRITE_HOOK_STATE` | may write its slot state |
| 3 | `WRITE_USER_STATE` | may write `UserMarketState` |
| 4 | `AWARD_POINTS` | may increase XP / points |
| 5 | `TREASURY_SIGNAL` | may emit treasury intents |
| 6 | `STRATEGY_EXECUTION` | may be invoked for vault-signed execution |
| 7 | `ORACLE_READ` | may read declared external accounts |
| 8 | `EXTERNAL_CPI` | runs as a separate program (rejected in v1) |
| 9 | `EMIT_SIGNAL` | may emit strategy signals/events |

`HookDefinition.capabilities` is the maximum; the slot stores the granted subset. The runtime
enforces it: a veto from a hook without `VETO_ROUTED_SWAP` is a protocol error
(`CapabilityViolation`), not a denial — fail closed.

## 7. Hook slot (112 bytes, inline in `MarketConfig`)

| Offset | Field | Type |
| --- | --- | --- |
| 0 | `module_id` | u16 |
| 2 | `priority` | u16 |
| 4 | `callbacks` | u16 |
| 6 | `flags` | u16 (bit0 `ENABLED`, bit1 `REQUIRED`) |
| 8 | `capabilities` | u32 |
| 12 | `version` | u16 |
| 14 | `_pad` | u16 |
| 16 | `config` | [u8;48] module-defined, validated at install |
| 64 | `state` | [u8;48] module-defined, written by the module |

## 8. Rust interface

```rust
pub trait PumpHook {
    const MODULE_ID: u16;
    const CAPABILITIES: u32;
    fn validate_config(config: &[u8; 48]) -> Result<(), HookError>;
    fn on_callback(env: &mut HookEnv<'_>) -> Result<HookDecision, HookError>;
}

pub struct HookEnv<'a> {
    pub ctx: &'a HookContext,
    pub config: &'a [u8; 48],
    pub state: &'a mut [u8; 48],
    pub user: &'a mut UserMarketState,
    pub market: &'a MarketStats,
    pub granted: u32,           // capabilities
    pub ext: &'a ExternalReads, // e.g. SENTIENT regime, policy accounts
}
```

## 9. External hook CPI ABI (defined, disabled in v1)

- Instruction data: `sha256("global:pump_hook_callback")[..8] ‖ HookContext ‖ config[48]`.
- Accounts: `market` (read-only), `hook_state` (writable, owned by the hook program, PDA
  `["state", market]` under the hook program), then declared read-only accounts.
- Never passed: user signer, user token accounts, vaults.
- Return: `HookDecision` via `set_return_data`; router checks the returning program ID.
- Compute: the router sets a per-hook CU budget check via `sol_remaining_compute_units`.

## 10. Events (Anchor `emit!`, borsh)

```
HookExecutionEvent { market, pool, mint, user, callback: u8, module_id: u16, slot_index: u8,
                     sequence: u64, allowed: bool, reason_code: u16, flags: u32, slot: u64 }
RoutedSwapEvent    { market, pool, mint, user, venue: u8, side: u8, mode: u8,
                     amount_in: u64, amount_out: u64, quote_amount: u64, base_amount: u64,
                     hook_count: u8, sequence: u64, timestamp: i64, slot: u64 }
HookDeniedEvent    { market, user, callback, module_id, reason_code, sequence }   // simulation logs only; a denial reverts
PolicyExecutionEvent { market, policy_index: u8, trigger: u8, matched: bool, result: u8,
                       reason_code: u16, sequence: u64 }
StrategyActionEvent  { market, vault, action: u8, amount: u64, reason: u16, nonce: u64, timestamp: i64 }
MarketCreatedEvent / HookInstalledEvent / HookRemovedEvent / HookConfiguredEvent / MarketSyncedEvent
```

A denied routed trade reverts, so its events do not land on-chain. Denials are surfaced through
transaction simulation (the SDK decodes the `HookDenied` error: module, reason code) and the
simulator. The explorer never shows a denial it did not observe.
