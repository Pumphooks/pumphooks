# Architecture: programmable markets (internal codename HOOKS)

**Shared market infrastructure + a developer hook program = a programmable market.**

The core program (`hooks-core`) provides pools, vaults, accounting, settlement, LP shares, hook
invocation and permissions, and the hook and market registries. A hook is any Solana program that
implements the hook interface ([hook-interface.md](hook-interface.md)). It supplies market behavior
the core never anticipated, without changes to the core.

The core has no dependency on Pump. Pump tokens are ordinary SPL / Token-2022 mints. The earlier
Pump router is an optional adapter documented in
[architecture-pump-router.md](architecture-pump-router.md).

The design follows [research.md](research.md). The numbered implications there (R1–R11) are cited
as `[R#]`.

---

## 1. Components

```
                 ┌───────────────────────── hooks-core (one program) ─────────────────────────┐
 trader / LP ──▶ │ Protocol · HookRegistration · Attestation · Pool (+ vaults, LP mint)        │
 aggregator      │ swap · add/remove liquidity · donate · initialize                          │
                 │ hook credits · credit swaps · fee accounting · invariants · events          │
                 └───────────────┬─────────────────────────────────────────────────────────────┘
                                 │ CPI: callback-authority PDA signs; no user signature forwarded
                                 ▼
                 ┌──────────── hook program (anyone) ─────────────┐
                 │ before/after initialize, liquidity, swap,      │
                 │ donate; quote (custom curve)                   │
                 │ → HookResult via return data                   │
                 └────────────────────────────────────────────────┘
```

| Piece | Path |
| --- | --- |
| Interface crate (types, layouts, discriminators, caller verification) | `crates/hooks-interface` |
| Core program | `programs/hooks-core` |
| Reference hooks | `hooks-programs/*` |
| Hook developer kit (scaffolder, test harness) | `packages/create-hook`, `crates/hooks-test` |
| TypeScript SDK | `packages/sdk` |
| Indexer, CLI, web | `services/indexer`, `packages/cli`, `apps/web` |
| Branding (name, logo, domain) | `branding.json`, the single source |

## 2. Accounts

All PDAs are under `hooks-core`.

| Account | Seeds | Purpose |
| --- | --- | --- |
| `Protocol` | `["protocol"]` | Admin, pause flags, protocol fee share (≤ 25% of LP fee; **0 at launch**), fee authority |
| `HookRegistration` | `["hook", hook_program]` | Declared permission mask, interface version, metadata URI, upgrade authority and deploy slot observed at registration, registrant, pool count |
| `Attestation` | `["attest", hook_program, attester]` | Named attester's statement (kind, URI, content hash). There is no protocol-level "audited" flag [R11]. |
| `Pool` | `["pool", mint_0, mint_1, hook_program, salt_le_u64]` | Identity includes the hook [§1.1 of research]. Mints are sorted (`mint_0 < mint_1`). |
| Vaults | `["vault", pool, mint]` | Token accounts owned by the pool PDA |
| LP mint | `["lp", pool]` | SPL Token mint, authority = pool PDA |
| Callback authority | `["callback", hook_program]` | Signs every core→hook CPI. Holds no other authority [R5]. |

`hook_program = Pubkey::default()` means a plain pool with no hook.

### Pool state (key fields)

- `mint_0`, `mint_1`, `vault_0`, `vault_1`, `token_program_0/1`, `lp_mint`, `hook_program`,
  `permissions: u32`.
- `fee_ppm: u32` (current LP fee, parts per million), `max_fee_ppm: u32` (cap a dynamic-fee hook
  cannot exceed; ≤ 500 000).
- `reserve_0`, `reserve_1`: the LP-owned amounts priced by the curve.
- `credit_0`, `credit_1`: **the hook's balance in this pool** [R4].
- `protocol_fee_0`, `protocol_fee_1`.
- `lp_supply`, `locked_lp` (minimum liquidity), `sequence` (incremented per state-changing op),
  `flags` (`HOOK_PINNED`, `PAUSED`), `pinned_deploy_slot`, `created_at`.

**Vault invariant**, checked at the end of every state-changing instruction:

```
vault_i.amount >= reserve_i + credit_i + protocol_fee_i      (i = 0, 1)
```

It is `>=` because anyone can transfer tokens straight into a vault. An equality check would let a
1-token transfer brick the pool. Unsolicited tokens are ignored.

Token-2022 transfer fees are handled by measuring vault balance deltas, never by trusting inputs.
Exact-out swaps whose input mint currently charges a transfer fee are rejected.

## 3. Hook permissions

A `u32` mask. Wire values are frozen and listed in [hook-interface.md](hook-interface.md).

- Callbacks: `BEFORE_INITIALIZE`, `AFTER_INITIALIZE`, `BEFORE_ADD_LIQUIDITY`, `AFTER_ADD_LIQUIDITY`,
  `BEFORE_REMOVE_LIQUIDITY`, `AFTER_REMOVE_LIQUIDITY`, `BEFORE_SWAP`, `AFTER_SWAP`, `BEFORE_DONATE`,
  `AFTER_DONATE`.
- Powers:
  - `BEFORE_SWAP_RETURNS_DELTA`, `AFTER_SWAP_RETURNS_DELTA`;
  - `AFTER_ADD_LIQUIDITY_RETURNS_DELTA`, `AFTER_REMOVE_LIQUIDITY_RETURNS_DELTA`;
  - `DYNAMIC_FEE`: may override the fee per swap, within `max_fee_ppm`;
  - `CUSTOM_CURVE`: the `quote` callback replaces the constant-product price over the pool's reserves.

Rules [R3]:

- The hook declares a mask when registering. A pool's mask is chosen at creation, must be a subset
  of the declared mask, and **never changes**.
- Each returns-delta power requires its base callback.
- `DYNAMIC_FEE` alone allows `set_fee`. A per-swap fee override additionally requires `BEFORE_SWAP`.
  `CUSTOM_CURVE` enables the `quote` callback and needs nothing else.
- The core invokes only permitted callbacks and ignores result fields a pool is not permitted to use.
  For example, a fee override without `DYNAMIC_FEE` is an error, not silently applied.

## 4. Operations

Every operation runs in one instruction: callbacks, accounting, token movement and invariant
checks. If anything fails, everything reverts [R6]. Multi-hop routes compose several swap
instructions in one transaction. Each settles independently and the transaction is atomic.

### 4.1 Initialize pool

- **Who:** anyone.
- **Inputs:** mints, hook program (registered, or none), `permissions ⊆ declared`, `fee_ppm`,
  `max_fee_ppm`, `salt`, `pin_hook`.
- **Token policy:** mints must pass [R9].
- **Steps:**
  1. Run `before_initialize`. A rejection reverts.
  2. Create the pool, vaults and LP mint.
  3. Run `after_initialize`.
- Liquidity is added separately.

### 4.2 Swap

- **Inputs:** `zero_for_one`, `exact_in`, `amount`, `limit` (min out / max in), `hook_data`.
- Remaining accounts are the hook's accounts.

Exact-in (input token `I`, output `O`):

1. Transfer `amount` from the trader to `vault_I` and measure the received amount `a`.
2. `before_swap`. Returns may include:
   - a fee override (needs `DYNAMIC_FEE`);
   - `delta_specified` (s, token `I`) and `delta_unspecified` (u₁, token `O`), which need
     `BEFORE_SWAP_RETURNS_DELTA`.
   - Positive deltas mean **the hook takes**; negative mean **the hook pays from its credit**.
3. `curve_in = a − s`. It must be ≥ 0. `curve_in = 0` is a full takeover (NoOp).
4. `fee = ceil(curve_in · fee_ppm / 1e6)`; the protocol share comes out of `fee`.
5. `curve_out` is either:
   - the constant product `(curve_in − fee) · R_O / (R_I + curve_in − fee)`; or
   - the hook's `quote` (needs `CUSTOM_CURVE`), checked `< R_O`.
6. `after_swap` may return u₂ (token `O`), which needs `AFTER_SWAP_RETURNS_DELTA`.
7. `out = curve_out − u₁ − u₂`. It must satisfy `0 ≤ out` and `out ≥ limit`.
8. Accounting:
   - `R_I += curve_in − protocol_fee`;
   - `R_O −= curve_out`;
   - `credit_I += s`;
   - `credit_O += u₁ + u₂`;
   - every credit must stay ≥ 0.
9. Transfer `out` from `vault_O` to the trader, then check the invariants.

Exact-out mirrors this:

- The specified token is `O`, and `curve_out = amount + s`.
- `curve_in` is the inverse constant product rounded up (or the hook quote), grossed up for the fee.
- The trader pays `curve_in + u₁ + u₂ ≤ limit`.
- If the input mint charges a transfer fee, exact-out is rejected in V1.

In both directions the **trader's slippage limit, enforced by the core after every delta, is the
trader's protection** against any hook [R4].

### 4.3 Add / remove liquidity

The core provides fungible, proportional LP shares.

- **First deposit:** shares = `isqrt(a₀·a₁)`. `MIN_LIQUIDITY = 1000` shares are locked forever.
- **Later deposits:**
  - shares = `min(a₀·S/R₀, a₁·S/R₁)`;
  - the amounts are recomputed so they are exactly proportional, rounded in the pool's favour.
- **Remove:** amounts = `shares·R/S`, rounded down.
- Hooks:
  - `before_*` may reject;
  - `after_*` may return `(d₀, d₁)` when permitted. Positive means the hook takes from the LP: an
    extra deposit charge, or a withdrawal fee into the hook's credit. Negative means the hook pays
    the LP a rebate from its credit.
- The LP's `max_0/max_1` (add) or `min_0/min_1` (remove) bound the result.

Specialised LP accounting, such as vesting, JIT defense or per-position rewards, is built by hooks
on these callbacks plus the hook's own accounts.

### 4.4 Donate

Adds tokens to the reserves without minting shares (for example, to pay LPs). `before/after_donate`
apply.

### 4.5 Hook-initiated operations (v4's `noSelfCall`)

A hook acts through its **hook-authority PDA** `["hook-authority"]`, derived under the *hook
program*. The hook signs with `invoke_signed` from its own instruction; the hook is the top-level
caller, so this is not re-entrancy. Operations:

- `credit_deposit`: anyone funds the hook's credit (tokens in, measured).
- `credit_withdraw`: the hook authority moves credit out to any token account.
- `credit_swap`: the hook authority swaps credit against the pool's curve at the current fee. **No
  callbacks run**, which is how TWAMM, limit orders, batch clearing and rebalancing execute on a
  crank [R10].
- `set_fee`: needs `DYNAMIC_FEE`, within `max_fee_ppm`.

### 4.6 Protocol

- `initialize_protocol`, `set_admin` (two-step), `set_pause` (pauses new pools, swaps and adds;
  **never removes**), `set_protocol_fee_share` (≤ 2500 bps of the LP fee),
  `withdraw_protocol_fees`.
- The admin cannot touch reserves, credits or hooks.

## 5. Calling a hook

See [hook-interface.md](hook-interface.md) for exact bytes.

- **Instruction data:** an 8-byte namespaced discriminator, a fixed 248-byte `HookContext`, then
  `hook_data` (≤ 512 bytes, opaque, untrusted).
- **Accounts:**
  - `[callback_authority (signer), pool (read-only)]`, then the hook's accounts from the remaining
    accounts in caller order.
  - `is_signer` is cleared on every forwarded account, so the user's signature never reaches a
    hook [R1].
  - `is_writable` is preserved. A hook can only write accounts it owns.
- **Result:**
  - a 40-byte `HookResult` from `get_return_data`;
  - the program id must equal the hook, and the length and version must match. Anything else is
    `HookResultInvalid`.
  - A missing result is invalid. There is no fallthrough-allow [§3 of research].
- **Pinning:** with `HOOK_PINNED`, the hook's `ProgramData` account must be supplied and its
  last-deploy slot must equal `pinned_deploy_slot`. LPs then get a hook that cannot silently
  upgrade under them [research §6].
- **Depth:** core→hook is one CPI. Hooks get 2 more levels when called from a top-level instruction
  and 1 more through an aggregator, until SIMD-0268 activates [R8].

## 6. Events

Anchor events are emitted via `emit!` (program log data):

- `PoolInitialized`, `Swap` (amounts, curve amounts, fee, deltas, reserves after, sequence),
  `LiquidityAdded`, `LiquidityRemoved`, `Donated`;
- `HookCalled` (callback, decision, reason, compute used is not observable on-chain so is not
  claimed);
- `CreditMoved`, `FeeChanged`, `HookRegistered`, `Attested`, `ProtocolUpdated`.

The indexer decodes these. Explorers render a swap as the ordered callback trace.

## 7. Token policy [R9]

| Accepted | Rejected in V1 |
| --- | --- |
| SPL Token | Token-2022 `TransferHook`, `PermanentDelegate`, `NonTransferable`, `DefaultAccountState`, `ConfidentialTransferMint`, `ConfidentialMintBurn`, and any extension not in the accepted list |
| Token-2022 with `TransferFeeConfig`, `MetadataPointer`, `TokenMetadata`, `InterestBearingConfig`, `MintCloseAuthority`, `GroupPointer`, `TokenGroup`, `GroupMemberPointer`, `TokenGroupMember` | Extensions newer than the core's Token-2022 parser (v6), including `ScaledUiAmount` and `Pausable`: these fail to parse and are rejected (fail closed) |

## 8. Security properties

| Property | Mechanism |
| --- | --- |
| Hooks cannot move vault tokens | Vault authority is the pool PDA. Hooks have no signer for it, and a hook cannot re-enter the core. |
| Hooks cannot spend user tokens | The user signature is never forwarded (`is_signer` cleared). |
| Hooks cannot impersonate the core to other hooks | The callback authority is per hook program. |
| Hook results are authentic | Return-data program id and length checks. |
| Hook value extraction is bounded | Deltas only move value between the trader or LP and *this* pool's hook credit. Credits stay ≥ 0, the trader's limit is enforced last, and swap type cannot flip. |
| Accounting is conserved | The vault invariant is checked after every instruction. |
| LPs can always exit the core's pause | Pause never blocks `remove_liquidity`. A hook with `BEFORE_REMOVE_LIQUIDITY` can, and the UI labels that power "can block withdrawals". |
| No silent upgrades (opt-in) | `HOOK_PINNED`. |
| No fake trust signals | Registry facts are observed on-chain, and attestations are signed by named attesters. |

What it does **not** protect against:

- A hook the trader or LP chose that is designed to extract value, up to the user's limits.
- An upgradeable, unpinned hook changing behavior.
- A hook that blocks withdrawals.

These are disclosed in the SDK quote, the registry and the UI.
