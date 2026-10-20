# Hook interface v1 (frozen wire format)

This is the complete contract between the core program and a hook program. Rust types live in
`crates/hooks-interface`. Any language that can produce a Solana program can implement a hook.

All integers are little-endian. Offsets are frozen for interface version 1; later versions only
append fields.

## 1. Callbacks

| id | callback | instruction name | needs permission |
| --- | --- | --- | --- |
| 0 | BeforeInitialize | `before_initialize` | `1 << 0` |
| 1 | AfterInitialize | `after_initialize` | `1 << 1` |
| 2 | BeforeAddLiquidity | `before_add_liquidity` | `1 << 2` |
| 3 | AfterAddLiquidity | `after_add_liquidity` | `1 << 3` |
| 4 | BeforeRemoveLiquidity | `before_remove_liquidity` | `1 << 4` |
| 5 | AfterRemoveLiquidity | `after_remove_liquidity` | `1 << 5` |
| 6 | BeforeSwap | `before_swap` | `1 << 6` |
| 7 | AfterSwap | `after_swap` | `1 << 7` |
| 8 | BeforeDonate | `before_donate` | `1 << 8` |
| 9 | AfterDonate | `after_donate` | `1 << 9` |
| 10 | Quote | `quote` | `CUSTOM_CURVE` (`1 << 10`) |

Power bits:

| Bit | Power |
| --- | --- |
| `1 << 16` | `BEFORE_SWAP_RETURNS_DELTA` |
| `1 << 17` | `AFTER_SWAP_RETURNS_DELTA` |
| `1 << 18` | `AFTER_ADD_LIQUIDITY_RETURNS_DELTA` |
| `1 << 19` | `AFTER_REMOVE_LIQUIDITY_RETURNS_DELTA` |
| `1 << 20` | `DYNAMIC_FEE` |

All other bits are reserved and must be zero.

**Discriminator:** the first 8 bytes of `sha256("market-hooks-v1:" + instruction name)`. The
namespace is protocol-neutral on purpose, and the values are pinned by tests in `hooks-interface`.

## 2. Instruction

```
data     = discriminator[8] ‖ HookContext[248] ‖ hook_data[hook_data_len]   (hook_data_len ≤ 512)
accounts = [0] callback authority  PDA ["callback", hook_program_id] under the core   signer, read-only
           [1] pool                                                                     read-only
           [2..] the pool operation's remaining accounts, in order; is_signer = false
```

A hook **must** verify, before anything else:

- account 0 is a signer, and its key equals `create_program_address(["callback", own_program_id,
  [context.authority_bump]], CORE_PROGRAM_ID)`. Taking the bump from the context is safe because
  only the core can sign for its PDAs. It costs a fixed ~1.5k CU, whereas `find_program_address`
  costs vary by program id;
- account 1's key equals `context.pool`, and it is owned by `CORE_PROGRAM_ID`.

`hooks_interface::verify_core_call` does both. Skipping this check is how the Cork hook lost about
$12M.

`hook_data` is supplied by whoever built the transaction. Treat it as untrusted input.

The pool account's data is written when the core instruction ends, so during a callback it still
shows the state before the operation. Read reserves, supply and fee from the context.

## 3. HookContext (248 bytes)

| off | size | field |
| --- | --- | --- |
| 0 | 1 | `version` = 1 |
| 1 | 1 | `callback` id |
| 2 | 1 | `swap_flags`: bit0 `zero_for_one` (token 0 in), bit1 `exact_in` |
| 3 | 1 | `authority_bump`: canonical bump of the callback-authority PDA |
| 4 | 4 | `permissions` (pool mask) |
| 8 | 32 | `pool` |
| 40 | 32 | `caller`: the signer that initiated the operation |
| 72 | 32 | `mint_0` |
| 104 | 32 | `mint_1` |
| 136 | 8 | `reserve_0` (before\* callbacks: state before the operation; after\* callbacks: after the curve / liquidity change, before the hook's own deltas) |
| 144 | 8 | `reserve_1` |
| 152 | 8 | `lp_supply` |
| 160 | 8 | `sequence` (pool operation counter) |
| 168 | 8 | `slot` |
| 176 | 8 | `unix_timestamp` (i64) |
| 184 | 4 | `fee_ppm` (current LP fee) |
| 188 | 4 | `hook_data_len` |
| 192 | 56 | `args: [u64; 7]` (meaning below) |

| callback | args[0] | args[1] | args[2] | args[3] | args[4] | args[5] | args[6] |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Before/AfterInitialize | fee_ppm | max_fee_ppm | salt | | | | |
| BeforeSwap | amount specified (exact-in: measured received) | limit | | | | | |
| Quote | exact-in: net curve input (after fee); exact-out: curve output | exact-in: fee amount; exact-out: 0 (the fee is added after) | | | | | |
| AfterSwap | amount specified | limit | curve_in | curve_out | fee | before δ_specified (i64 bits) | before δ_unspecified (i64 bits) |
| BeforeAddLiquidity | max_0 | max_1 | min_shares | | | | |
| AfterAddLiquidity | amount_0 | amount_1 | shares | | | | |
| BeforeRemoveLiquidity | shares | min_0 | min_1 | | | | |
| AfterRemoveLiquidity | amount_0 | amount_1 | shares | | | | |
| Before/AfterDonate | amount_0 | amount_1 | | | | | |

## 4. HookResult (40 bytes, via `set_return_data`)

| off | size | field |
| --- | --- | --- |
| 0 | 1 | `version` = 1 |
| 1 | 1 | `decision`: 0 continue, 1 reject (the core reverts with `HookRejected(reason)`) |
| 2 | 2 | `reason` (hook-defined, surfaced in errors and events) |
| 4 | 4 | `flags`: bit0 `FEE_OVERRIDE` |
| 8 | 4 | `fee_ppm` (valid when `FEE_OVERRIDE`; BeforeSwap only; ≤ pool `max_fee_ppm`) |
| 12 | 4 | reserved (0) |
| 16 | 8 | `delta_0` (i64) |
| 24 | 8 | `delta_1` (i64) |
| 32 | 8 | `quote` (Quote only) |

The core rejects a result in any of these cases:

- it is missing;
- it was set by another program;
- its length is not 40 or its version is not 1;
- a reserved field is non-zero;
- a non-zero delta or fee override was not permitted for that callback.

Delta meaning: **positive = the hook takes (its credit grows); negative = the hook pays from its
credit.**

| callback | delta_0 | delta_1 |
| --- | --- | --- |
| BeforeSwap | δ_specified (specified token) | δ_unspecified (other token) |
| AfterSwap | must be 0 | δ_unspecified |
| AfterAddLiquidity | token 0 charged to LP (+) / rebated (−) | token 1 |
| AfterRemoveLiquidity | token 0 withheld from LP (+) / bonus (−) | token 1 |
| others | must be 0 | must be 0 |

Settlement rules are in [architecture.md §4](architecture.md). In summary, every delta moves value
between the trader or LP and this pool's hook credit, and nowhere else. Credits must stay ≥ 0. The
trader's or LP's limits are checked after all deltas. The exact-in or exact-out type cannot flip.

## 5. Quote (custom curve)

When the pool has `CUSTOM_CURVE`, the core calls `quote` instead of the constant product:

- **exact-in:** `args[0]` is the net input to the curve and the hook returns the output amount.
  The core requires `quote < reserve_out`.
- **exact-out:** `args[0]` is the requested curve output and the hook returns the net input
  required. The core requires `quote > 0`. The core then grosses it up by the fee.

The hook prices against the pool's reserves (in the context), its own accounts (oracle, schedule,
inventory model) and `hook_data`. LPs keep their core LP shares.

## 6. Hook-initiated operations

A hook acts as the PDA `["hook-authority"]` **derived under the hook program**, signed with
`invoke_signed`:

- `credit_withdraw`;
- `credit_swap` (no callbacks run);
- `set_fee` (needs `DYNAMIC_FEE`).

`credit_deposit` is open to anyone.

## 7. Budget

- Compute: each callback costs the hook's own CU plus about 1–2k for CPI and return data. The
  1.4M CU transaction limit applies to everything.
- Depth: core→hook uses one level. A hook may CPI two levels deeper when the core is top-level (the
  limit is 5 levels until SIMD-0268).
- Accounts: the core swap uses about 10 accounts. Hooks should keep extra accounts small or rely on
  address lookup tables.
