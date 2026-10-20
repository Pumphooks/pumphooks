# Pump Hooks — Threat Model

Assume every account, argument, signer, signal and hook is hostile until validated.

## Assets

| Asset | Where | Who may move it |
| --- | --- | --- |
| User tokens / SOL | user wallets | only the user, by signing; the router passes the user signature to Pump and never holds funds |
| Strategy vault funds | `VaultAuthority` PDA | vault authority (reserve/ops buckets); bounded strategy actions (strategy bucket) |
| Market configuration | `MarketConfig`, `Policy` | market authority |
| Module enablement / pause | `GlobalConfig` | protocol admin (multisig before mainnet) |
| Holder XP / user state | `UserMarketState` | only the router, during a successful routed trade |

Pump Hooks never holds a token delegate and never custodies user funds between transactions.

## Trust boundaries

1. Wallet → router (untrusted args and accounts).
2. Router → Pump/PumpSwap (trusted program IDs, untrusted accounts until bound to the market).
3. Off-chain SENTIENT engine → vault (signed, bounded, replay-protected).
4. Market authority → its own market (trusted for its market only).
5. Protocol admin → global switches (cannot touch user funds or vault funds).

## Threats and controls

| # | Threat | Control | Test |
| --- | --- | --- | --- |
| T1 | **Fake Pump program**: attacker passes a look-alike program as `program` | Adapter requires exact program ID constants; CPI target is the constant, not the passed key | `wrong_pump_program` |
| T2 | **Fake pool / pool for another mint** | `pool` owner == PumpSwap, discriminator == `Pool`, `pool == market.pool`, `pool.base_mint == market.mint` | `wrong_pool`, `wrong_mint` |
| T3 | **Vault substitution**: pass a different token account as pool vault so our reserve read is fake | `pool_base/quote_token_account` must equal the keys stored in `Pool` | `wrong_vault` |
| T4 | **User token account substitution** to fake balance deltas | Token account owner == user, mint == expected mint, owned by the declared token program | `wrong_user_token_account` |
| T5 | **MarketConfig substitution** (pass a lax config) | PDA re-derived from `[market, mint, authority]`; trace records which market was used; UI shows authority and verified flag | `wrong_market_config` |
| T6 | **Malicious creator** sets rules that hurt routed users | Rules can only *deny* or *account*; no module can move user funds; users can see the stack before trading and can always trade directly | design |
| T7 | **Malicious hook** (future external hooks) | V1: only first-party modules compiled into the program. Capabilities checked at install and dispatch. `EXTERNAL_CPI` capability is rejected in V1 | `capability_denied` |
| T8 | **Replayed / expired / future strategy signal** | Strictly increasing nonce per vault, `valid_until >= now`, `issued_at <= now + MAX_SKEW`, max validity window | `signal_*` tests |
| T9 | **Forged signal** | Ed25519 precompile instruction must precede, and its pubkey and message must byte-equal the vault signer and the signal being applied (instruction sysvar introspection) | `invalid_signer` |
| T10 | **Signal tries to exceed bounds** | `allowed_actions` bitmask, `max_amount_bps`, epoch budget, cooldown, per-slice slippage cap | `signal_bounds` |
| T11 | **Unauthorized vault withdrawal** | Only vault authority for reserve/ops; strategy bucket only spendable via TWAP slices through the router | `unauthorized_withdraw` |
| T12 | **Integer overflow / underflow** | `checked_*` everywhere on value paths; `overflow-checks = true` in release; u128 intermediates for price math; property tests at u64 boundaries | `overflow`, `u64_bounds` |
| T13 | **Compute DoS** (max hooks, max policy size) | ≤ 4 runtime hooks, ≤ 8 conditions, ≤ 4 actions, no loops over unbounded data, benchmarked | `max_hooks`, `policy_limits` |
| T14 | **Malicious remaining accounts** | Remaining accounts are only accepted in declared positions with owner + PDA checks (policies, vault); unexpected extras are rejected | `extra_remaining_accounts` |
| T15 | **Configuration takeover** | All config mutations require `market.authority` signer; authority transfer is two-step (propose/accept) | `unauthorized_config` |
| T16 | **Unauthorized hook install / removal** | `install_hook`/`remove_hook` require market authority; module must be registered and enabled | `unauthorized_install`, `unauthorized_remove` |
| T17 | **Disabled hook still executing** | Router checks `GlobalConfig.disabled_modules` and slot `ENABLED` flag on every dispatch | `disabled_hook` |
| T18 | **Paused protocol** | `GlobalConfig.paused_routing` checked first | `paused_protocol` |
| T19 | **Wash-trading Holder XP** | XP only from routed buys, vests after a hold period, forfeited on any routed sell before vesting, per-trade and per-day caps, never from sells; holding XP uses `min(balance_then, balance_now) × elapsed` | `xp_*` tests |
| T20 | **Holding-XP double counting across wallets** | Residual risk: two wallets can each hold during overlapping intervals only if each holds at both checkpoints; bounded by caps and only checkpointed on routed trades. Documented | doc |
| T21 | **Delegate abuse** | The router requires `user` to be the signer and token account owner; it never accepts delegates and never approves | `delegate_rejected` |
| T22 | **Upgrade-authority compromise** | Devnet: single key. Mainnet: multisig + timelock, published in `SECURITY.md`. Not deployed to mainnet | process |
| T23 | **Fake oracle** | V1 has no external oracle. Volatility uses on-chain reserves only. Off-chain signals are labelled as such and only reach the chain as bounded signed signals | design |
| T24 | **Price manipulation before a routed trade** (attacker trades directly to trip a guard or to sandwich a routed user) | Out of our control for direct flow. Guards deny rather than execute at bad prices; user slippage limits always forwarded to Pump | doc |
| T25 | **Strategy used for manipulation** ("buy before users", pulling liquidity) | Not implemented by design. Actions are treasury/risk actions with size/time bounds; no liquidity withdrawal action exists | design |
| T26 | **Cashback coin / unsupported venue** | Adapter rejects before CPI with explicit error | `cashback_rejected` |
| T27 | **Curve complete, not migrated** | Deny `MarketMigrating` | `migrating_market` |
| T28 | **Log truncation hides events** | Events are compact; indexer cross-checks `RoutedSwapEvent.hook_count` against decoded `HookExecutionEvent`s and marks traces incomplete instead of inventing data | indexer |
| T29 | **Same-slot routed sandwich** | Launch Shield can deny a routed buy and sell by the same wallet in one slot and cap routed buys per slot — protecting routed users from routed actors only | `shield_slot` |

## Explicit non-goals

- Preventing trades that bypass the router.
- Preventing MEV on Pump directly.
- Guaranteeing any price outcome.
