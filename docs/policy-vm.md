# Pump Hooks — Policy VM (v1)

The Composer (`/build`) compiles visual rules into a **Policy**: a fixed-size, loop-free record
evaluated by `crates/policy-vm` inside the router. It is a policy engine, not a programming
language. It cannot CPI, cannot move funds, cannot loop, and cannot grow.

```
WHEN  trigger
IF    condition₁ AND/OR … conditionₙ      (n ≤ 8)
THEN  action₁ … actionₘ                    (m ≤ 4)
```

## 1. Account layout — `Policy` (PDA `["policy", market, index]`, index < 4)

| Field | Type | Notes |
| --- | --- | --- |
| `market` | Pubkey | |
| `index` | u8 | 0–3 |
| `version` | u8 | `1` |
| `trigger` | u8 | see §2 |
| `combinator` | u8 | `0` ALL (AND), `1` ANY (OR) |
| `n_conditions` | u8 | ≤ 8 (0 = always) |
| `n_actions` | u8 | 1–4 |
| `enabled` | u8 | |
| `bump` | u8 | |
| `cooldown_secs` | u32 | non-DENY actions skipped inside cooldown |
| `deny_reason` | u16 | reported as `9000 + deny_reason` (≤ 999) |
| `conditions` | [Condition; 8] | 16 bytes each |
| `actions` | [Action; 4] | 16 bytes each |
| `registers` | [u64; 4] | `HOOK_STATE` storage |
| `exec_count` | u64 | |
| `last_exec_ts` | i64 | |
| `intent` | Intent | pending treasury/strategy intent for the vault |
| `label_hash` | [u8; 32] | sha256 of the off-chain label/description |

`Condition { field: u8, op: u8, arg: u8, _pad: [u8;5], value: u64 }`
`Action    { kind: u8, arg: u8, arg16: u16, _pad: u32, value: u64 }`

## 2. Triggers

| Value | Trigger | Status |
| --- | --- | --- |
| 0 | `BEFORE_BUY` | v1 |
| 1 | `AFTER_BUY` | v1 |
| 2 | `BEFORE_SELL` | v1 |
| 3 | `AFTER_SELL` | v1 |
| 4 | `BEFORE_SWAP` | v1 |
| 5 | `AFTER_SWAP` | v1 |
| 16 | `VOLUME_THRESHOLD` | reserved (crank) |
| 17 | `VOLATILITY_THRESHOLD` | reserved (crank) |
| 18 | `TIME_INTERVAL` | reserved (crank) |
| 19 | `TREASURY_THRESHOLD` | reserved (crank) |
| 20 | `GRADUATION` | reserved |
| 21 | `FEE_EVENT` | reserved |
| 22 | `STRATEGY_SIGNAL` | reserved |

Triggers 0–5 equal the matching `HookCallback` values.

## 3. Condition fields

Only fields whose data the router can obtain deterministically from accounts in the transaction.

| Value | Field | Unit | Source |
| --- | --- | --- | --- |
| 1 | `TRADE_SIZE` | quote base units | context (estimate before, observed after) |
| 2 | `TRADE_SIZE_BASE` | base units | context |
| 3 | `ROUTED_VOLUME` | quote units | `MarketConfig` lifetime routed volume |
| 4 | `VOLATILITY` | bps deviation of spot from EMA | Volatility Guard slot state (validation requires the module) |
| 5 | `TREASURY_BALANCE` | lamports | Strategy vault authority (requires vault account) — reserved v1.1 |
| 6 | `TIME_SINCE_LAUNCH` | seconds | **since market activation** (`MarketConfig.created_at`) — Pump does not store a creation time on the curve |
| 7 | `TIME_SINCE_LAST_ACTION` | seconds | this policy's `last_exec_ts` |
| 8 | `BUY_SELL_RATIO` | bps (`buy*10000/max(sell,1)`) | routed volume stats |
| 9 | `HOLDER_DURATION` | seconds | `now - UserMarketState.first_buy_ts` (0 if never bought) |
| 10 | `HOOK_STATE` | raw u64 | `registers[arg]` |
| 11 | `USER_ROUTED_VOLUME` | quote units | `UserMarketState` |
| 12 | `USER_XP` | XP | `UserMarketState` |
| 13 | `USER_BASE_BALANCE` | base units | context |

## 4. Operators

`0 EQ, 1 NEQ, 2 GT, 3 GTE, 4 LT, 5 LTE`. Unsigned comparison on u64. Anything else fails validation.

## 5. Actions

| Value | Action | Allowed triggers | Effect |
| --- | --- | --- | --- |
| 1 | `ALLOW` | any | stop processing further actions of this policy |
| 2 | `DENY` | before-* only | reject routed trade, reason `9000 + deny_reason` |
| 3 | `RECORD` | any | emit `PolicyExecutionEvent` with tag `value` |
| 4 | `AWARD_POINTS` | after-* only | `user.policy_points += min(value, 10_000)` |
| 5 | `UPDATE_STATE` | any | `registers[arg] = SET/ADD/SUB(value)` (`arg16`: 0 SET, 1 ADD sat, 2 SUB sat) |
| 6 | `QUEUE_TWAP` | any | write intent (vault applies within its bounds) |
| 7 | `ALLOCATE_AUTHORIZED_TREASURY` | any | write intent |
| 8 | `PAUSE_STRATEGY` | any | write intent |
| 9 | `RESUME_STRATEGY` | any | write intent |
| 10 | `EMIT_SIGNAL` | any | emit `PolicyExecutionEvent` with `result = SIGNAL` |
| 11 | `EXECUTE_APPROVED_STRATEGY` | any | write intent |

There is no CPI action. Intent actions do nothing by themselves: the vault program's permissionless
`apply_policy_intent` reads the intent, checks it against the vault's configured bounds and
`allowed_actions`, and either applies it or ignores it. The Composer labels them "requires a
Strategy Vault".

## 6. Evaluation semantics

```
eval(policy, ctx):
  if !enabled or trigger != ctx.callback: skip
  matched = (n_conditions == 0) or combine(combinator, conditions[0..n])
  if !matched: return Allow (no effects)
  in_cooldown = cooldown_secs > 0 and now - last_exec_ts < cooldown_secs
  for a in actions[0..n_actions]:
      DENY      -> return Deny(9000 + deny_reason)          (applies even in cooldown)
      ALLOW     -> break
      others    -> if !in_cooldown: apply
  if any non-DENY action applied: exec_count += 1, last_exec_ts = now
```

Policies installed on a market run in ascending `index`. The first DENY ends the before-phase.
Cost is bounded by `4 policies × (8 conditions + 4 actions)`, each O(1).

## 7. Validation (on `create_policy` / `update_policy`)

- `version == 1`, `index < 4`, trigger in the v1 set.
- `n_conditions ≤ 8`, `1 ≤ n_actions ≤ 4`; unused entries must be zeroed.
- Every field/op/action kind known; `HOOK_STATE` register `< 4`; `UPDATE_STATE` op `< 3`.
- `DENY` only in before-triggers; `AWARD_POINTS` only in after-triggers; at most one `DENY`.
- `VOLATILITY` requires an installed Volatility Guard slot; `TREASURY_BALANCE` rejected in v1.
- `deny_reason ≤ 999`, `AWARD_POINTS.value ≤ 10_000`.

## 8. Example

"WHEN Before Buy IF Trade Size > 5 SOL THEN Reject Routed Trade"

```
trigger = 0 (BEFORE_BUY), combinator = ALL
conditions[0] = { field: TRADE_SIZE, op: GT, value: 5_000_000_000 }
actions[0]    = { kind: DENY }, deny_reason = 1
```
