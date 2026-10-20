# Research: programmable markets on Solana

Sources were inspected on 2026-10-20 from source code, not summaries. Pump-specific research for the
earlier router lives in [research-pump.md](research-pump.md).

| Source | Commit | What was read |
| --- | --- | --- |
| `Uniswap/v4-core` | `46c683469` (2026-04-02) | `PoolManager.sol`, `libraries/Hooks.sol`, `LPFeeLibrary.sol`, `types/*`, `IHooks.sol` |
| `Uniswap/v4-periphery` | `9969eec44` (2026-09-18) | `base/SafeCallback.sol`, `base/ImmutableState.sol`, `V4Router`, `PositionManager`, `hooks/permissionedPools` |
| `Uniswap/v4-template` | `1fbf95547` (2025-10-28) | `src/Counter.sol`, deploy scripts, submodules |
| `patcha-fi/patcha` | `3d4123dec` (2026-06-13) | Program, runtime crate, adapters, README |
| `solana-program/transfer-hook` | `3ca0dbcbb` (2026-10-07) | Interface crate (instruction, on-chain helpers, extra account metas) |
| `igneous-labs/S` (Sanctum Infinity) | `66de438b9` (2025-11-19) | Pricing-program and SOL-value-calculator interfaces, controller CPI and verification |
| `Ellipsis-Labs/plasma` | `86fb3503e` (2026-10-16) | `plasma_state` (AMM, LP vesting) |
| `raydium-io/raydium-cp-swap` | `f4b9b2634` (2026-10-20) | Token-2022 handling (`utils/token.rs`, withdraw) |
| `MeteoraAg/damm-v2` | `5398b5c7c` (2026-10-15) | Base-fee modules and dynamic fee parameters |
| `solana-labs/twamm` | `a3be22284` (2023-10-17) | README / design |
| `PropAMMForge/PropAMM-Forge` | `fedb79fee` (2026-10-20) | README / design |
| SPL token-swap (`solana-program-library`, master) | — | `curve/calculator.rs`, `base.rs`, `constant_product.rs`, `offset.rs` |
| `anza-xyz/agave` | `1c12b3ab8` (2026-10-20) | `program-runtime` (invoke context, execution budget), `syscalls`, `transaction-context` |

GitHub search went beyond high-star projects, with queries for hook frameworks, programmable or
modular AMMs, dynamic fees, TWAMMs, batch auctions, oracle AMMs and shared vaults. The search API
rate-limited one query (oracle AMM). §4 lists what was found.

---

## 1. What Uniswap v4 actually does

### 1.1 Singleton plus flash accounting

- One `PoolManager` holds every pool's tokens.
- A pool is identified by
  `PoolKey { currency0, currency1, fee, tickSpacing, hooks }`, and its id is the hash of that key.
  The **hook address is part of the pool's identity**: the same pair with a different hook is a
  different pool.
- State-changing calls happen only inside `unlock(data)`, which calls back
  `IUnlockCallback(msg.sender).unlockCallback(data)`.
- Inside that callback the caller can perform any sequence of `swap`, `modifyLiquidity`, `donate`,
  `take`, `settle`, `mint` and `burn` calls.
- Each call adjusts a per-(currency, address) **delta** in transient storage. `NonzeroDeltaCount`
  tracks how many are non-zero.
- When the callback returns, `unlock` reverts with `CurrencyNotSettled` unless every delta is zero
  (`PoolManager.sol:104-114`).
- Token movement is decoupled from accounting:
  - `sync(currency)` snapshots the manager's balance, and `settle()` credits
    `balanceNow - reservesBefore` to the caller (`_settle`, `:349-365`).
  - `take` debits and transfers out.
  - `mint` / `burn` convert deltas to and from ERC-6909 claim balances held inside the manager.
  - `clear` forgives an exact positive delta (dust).
  - Multi-hop routes therefore move tokens once, at the end.

### 1.2 Hook permissions live in the hook's address

- `Hooks.sol` reserves the low 14 bits of the hook address. There are 10 callback flags (before/after
  × initialize, add liquidity, remove liquidity, swap, donate) and 4 "returns delta" flags:
  `beforeSwapReturnDelta`, `afterSwapReturnDelta`, `afterAddLiquidityReturnDelta` and
  `afterRemoveLiquidityReturnDelta` (`Hooks.sol:27-47`).
- The PoolManager calls only callbacks whose bit is set. The check is a mask on the address, so it
  costs nothing.
- `isValidHookAddress` rejects a returns-delta flag without its base flag (`:109-127`).
- Developers mine a CREATE2 salt so the deployed address carries the right bits (`HookMiner` in the
  template's deploy script). Permissions are therefore immutable for the life of the hook and of
  every pool keyed to it.

### 1.3 Callback contract

- `callHook` does a raw `call`, bubbles reverts, and checks that the returned selector equals the
  called selector (`:131-155`).
- Delta-returning callbacks must return exactly 64 bytes, and `beforeSwap` exactly 96 bytes.
- `IHooks` return types:
  - `beforeSwap → (bytes4, BeforeSwapDelta, uint24 lpFeeOverride)`;
  - `afterSwap → (bytes4, int128)`;
  - `afterAdd/RemoveLiquidity → (bytes4, BalanceDelta)`;
  - the rest return `bytes4`.
- **`noSelfCall`**: hooks are not invoked for actions the hook itself initiates (`msg.sender ==
  hook`). This is how a hook can trade against its own pool without recursion.
- **`hookData`**: an opaque `bytes` from the swapper or LP, forwarded verbatim. The hook must treat it
  as untrusted.

### 1.4 Return deltas (custom accounting)

- `BeforeSwapDelta` packs `int128 specified` (upper 128 bits) and `int128 unspecified` (lower).
- The specified delta is **added to the amount being swapped** before the pool's curve runs. A hook
  that returns `-amountSpecified` consumes the whole input, so the AMM's own curve does nothing.
  This is how custom curves, NoOp pools and limit-order or RFQ hooks are built.
- The swap type must not flip. `HookDeltaExceedsSwapAmount` reverts if exact-in becomes exact-out
  or vice versa (`:270-279`).
- `afterSwap` may return an extra unspecified delta.
- The PoolManager credits the combined hook delta to **the hook's address** and subtracts it from the
  caller's delta: "the caller has to pay for (or receive) the hook's delta" (`:304-313`). The hook
  then settles inside the same `unlock` through `take` / `settle`.
- The same pattern applies to liquidity: `afterAdd/RemoveLiquidity` return a `BalanceDelta` that
  moves value between the LP and the hook (`:216-244`).
- **Consequence: a hook with a returns-delta permission can take arbitrary value from swappers and
  LPs of its pools.** The only user protection is the caller's slippage limit and the decision not
  to use that pool. v4 makes this explicit by encoding it in the address. It does not prevent it.

### 1.5 Fees

- LP fee: `uint24` in hundredths of a bip, capped at `MAX_LP_FEE = 1_000_000` (100%).
- A pool keyed with `fee = DYNAMIC_FEE_FLAG (0x800000)` has a dynamic fee:
  - only the hook may call `updateDynamicLPFee` (`PoolManager.sol:339-346`);
  - `beforeSwap` may also return a per-swap override with `OVERRIDE_FEE_FLAG (0x400000)` set.
- Protocol fees are separate and taken on the input currency (`_swap`, `:230-253`).

### 1.6 Periphery and template

- `PositionManager` wraps liquidity as ERC-721 positions. `V4Router` encodes multi-hop routes as
  action lists executed inside one `unlock`.
- `SafeCallback` / `ImmutableState.onlyPoolManager` restrict callbacks to the PoolManager.
- `BaseHook` now lives in OpenZeppelin's `uniswap-hooks` (the template imports
  `@openzeppelin/uniswap-hooks/src/base/BaseHook.sol`).
- The template's `Counter` hook shows the intended developer experience:
  - declare permissions in `getHookPermissions()`;
  - implement `_beforeSwap` and friends;
  - mine an address;
  - deploy, create a pool, add liquidity and swap via scripts.
- A single hook contract is expected to serve many pools and to key its state by `PoolId`.

### 1.7 Failure mode that matters: unauthenticated callbacks (Cork, May 2025)

- Cork's v4 hook exposed `beforeSwap` without a `msg.sender == poolManager` check.
- The attacker called it directly, with forged `hookData`, from inside its own `unlock` context. It
  then combined this with an economic flaw in Cork's rollover pricing to extract about $12M.
- Sources: Cork post-mortem, BlockSec and CertiK analyses, and the DeFiHackLabs reproduction
  `src/test/2025-05/Corkprotocol_exp.sol`.
- Lesson: **a hook interface must make "this call really comes from the core, for this pool, in this
  operation" verifiable by default**, not left to each hook author. Hook data is attacker-controlled.

---

## 2. What does not port to Solana, and why

These are runtime facts, checked in Agave at `1c12b3ab8`.

| Fact | Source | Consequence |
| --- | --- | --- |
| **No re-entrancy across programs.** A program may call itself (A→A), but A→B→A fails with `ReentrancyNotAllowed`. | `program-runtime/src/invoke_context.rs:246-270` | A hook cannot call back into the core during a callback. v4's "hook settles via `take`/`settle` inside the callback" is impossible. The hook must **return** its intent and the core must apply it. This also removes the whole class of reentrancy bugs. |
| **CPI depth 4** (stack height 5). SIMD-0268 raises it to 8, but its gate `6TkHk…` is **not activated on mainnet** (planned for Agave 4.4). | `execution_budget.rs:8-17`, solana.com/upgrades | Wallet → aggregator → core → hook → (one more level) is the worst case. Hooks called through an aggregator get at most one further CPI level until SIMD-0268 activates. |
| **Return data ≤ 1024 bytes**, tagged with the program id that set it. | `syscalls/src/lib.rs:1945`, `MAX_RETURN_DATA` | Hook results travel as return data. The core must check that the returning program id is the hook. |
| **CPI cost** is 946 CU per invoke, plus serialization of every account passed. The transaction cap is 1.4M CU. | `execution_budget.rs:21,26` | Each callback costs real compute. Callbacks must be opt-in per pool (v4's bitmask), and account lists must stay small. |
| **Privileges can be dropped, never escalated.** In a CPI the callee sees `is_signer` / `is_writable` as set in the caller's `AccountMeta`, capped by the caller's own privileges. | Solana CPI model | The core can call a hook **without** forwarding the user's signature. A hook then cannot spend the user's tokens even though the user signed the outer transaction. Token-2022 does the same thing for transfer hooks. |
| **Only the owner program may write an account's data; PDAs sign only for their own program.** | Solana account model | A hook cannot touch pool state or vaults owned by the core even if they are passed to it writable. Vault custody stays with the core's PDA. |
| **No transient storage.** | — | v4-style flash accounting across separate instructions has no native home. Net settlement must happen **within one instruction** (a batch `execute(actions[])`) or be persisted. |
| **Transaction size.** 1232 bytes for legacy/v0. solana.com/upgrades lists 4096-byte transactions in Agave 4.2 (Aug 2026). | solana.com/upgrades | Design for 1232 bytes with lookup tables until wallet support for larger transactions is confirmed. |

There is also no address-bit trick. Program ids can be ground for vanity, but that is not a
reasonable requirement. Permissions must be **declared data**, fixed into the pool at creation.

---

## 3. Patcha (`patcha-fi/patcha`)

The README claims to bring v4 hooks to Solana CLMMs (Orca, Raydium, Meteora), "mainnet-live",
"8/8 hooks · 59/59 tests". The code shows:

- **No CPI anywhere.** `trigger_hook(callback_kind, amount_in, tick)` is a standalone instruction.
  - It dispatches a slug to one of six compiled-in functions and emits an event.
  - Nothing places it on a swap's execution path, so a swap simply does not call it.
- **Caller-supplied inputs.** `amount_in` and `tick` are instruction arguments and are never read
  from the pool. The Orca adapter just stamps a venue tag.
- **The decision changes nothing.** `fee_override_bps` is emitted, not applied: Orca and Raydium do
  not read it.
- **Unknown slugs allow.** `dispatch` ends with `_ => Ok(HookOutcome::allow())`. "Community" hooks
  are slugs with no code.
- **"Audited" is self-assigned.** `register_hook` sets `audited = kind == HOOK_KIND_BUILTIN`.
- **Docs and code disagree.** The README table lists 8 modules (PriceImpactCap, JIT-Defense); the
  dispatcher has 6.

What to learn from it:

- The hook-at-the-integration-boundary pattern, without a core that **owns settlement**, gives
  advisory signals, not market behavior.
- Self-reported audit flags and fallthrough-allow defaults are anti-patterns.

Our earlier Pump router had the same structural limit (bypassable boundary), stated honestly. This
protocol removes it by owning the pool.

---

## 4. Solana prior art

| Project | Relevant mechanism | Takeaway |
| --- | --- | --- |
| **Token-2022 transfer hook interface** | Token-2022 CPIs into a program named by the mint, using `Execute { amount }` with an 8-byte namespaced discriminator (`spl-transfer-hook-interface:execute`). Extra accounts are resolved from a validation PDA `["extra-account-metas", mint]` holding a TLV `ExtraAccountMetaList`. Accounts from the transfer are passed read-only. | The canonical Solana hook pattern: a **fixed interface, namespaced discriminators, declared extra accounts resolvable off-chain, and de-escalated privileges.** We adopt all four. |
| **Sanctum Infinity** (`igneous-labs/S`) | One controller holds every pool's reserves. Swap pricing is delegated by CPI to a **pricing program** configured in pool state. `verify_correct_pricing_program` checks the passed program, and the controller reads a `u64` from return data (`cpi/pricing_program.rs:113-116`, `FaultyPricingProgram` on missing data). | Production precedent for **core owns custody and settlement; an external program supplies the number via return data.** Interface documented per instruction with account tables. |
| **SPL token-swap** | Pluggable curves behind a `CurveCalculator` trait (constant product, constant price, offset). The curves are **compiled into the program.** | A fixed menu. New curves need a new program. That is what we are removing. |
| **Meteora DAMM v2** | Fee modules in the AMM (time scheduler, market-cap scheduler, rate limiter, volatility-based dynamic fee). | Rich, but a closed feature list. Each is a natural *hook*, not a core feature. |
| **Raydium CP-Swap** | Token-2022 support with an **allowlist of mint extensions** (TransferFeeConfig, MetadataPointer, TokenMetadata, InterestBearingConfig, ScaledUiAmount). Others need explicit permission. Received amounts are net of transfer fees. | Copy the extension policy: default-deny, account for transfer fees by measuring vault deltas. |
| **Ellipsis Plasma** | Sandwich resistance by **slot snapshots**: a virtual limit order priced at the slot-start snapshot. Also an LP-share vesting window against JIT liquidity. | A real mechanism a generic hook system must express. It needs per-pool hook state, the ability to replace pricing, and LP-action gating. |
| **Solana TWAMM** (`solana-labs`) | Orders pooled and executed over time by permissionless cranks, netted at an oracle price, residual routed through Jupiter. | TWAMM needs scheduled execution (crank instruction) plus hook-held inventory. That argues for **hook-owned balances inside the core** (v4's ERC-6909 claims). |
| **PropAMM Forge** | "One program, many vaults". Pricing is posted by an off-chain signer, and quotes expire in slots. | Oracle- or quote-priced markets are hooks that own pricing and read signed or posted state. |
| Batch-auction projects (Epoch, BeatEx, unwind, lattice, CrossBar, mev-shield) | Mostly 2026 hackathon builds, often on MagicBlock ephemeral rollups or TEEs. | Batch settlement needs **deferred settlement**: collect intents, clear later. A hook must be able to hold the user's input and pay out on a later crank. |
| Token-2022 hook AMMs (`mitgajera/Token2022-Hook-AMM` and others, 0★) | AMMs that support transfer-hook *mints*. | Different problem (mint hooks, not market hooks). Confirms demand for transfer-hook mint support as an opt-in. |

**Finding:** no Solana project we found has an AMM core invoke **third-party** programs on the
swap path with **custom accounting** under core-enforced invariants:

- Sanctum is the closest, but it has one pricing program per pool, set by an admin, and no
  before/after lifecycle or LP hooks.
- Patcha is advisory.
- Everything else is a closed feature list.

---

## 5. Design implications (inputs to the architecture)

1. **The core owns custody, accounting and settlement.** Hooks never hold the vault authority, are
   never invoked with the user's signature, and cannot write core-owned accounts (Solana enforces
   the last). Unlike Patcha and our earlier router, nothing about the market is bypassable: the
   pool *is* the market.
2. **Hooks return intents and never call back.** Re-entrancy is impossible, so every callback
   returns a fixed-layout result via return data:
   - allow, or reject with a code;
   - a fee override;
   - deltas.
   The core checks the returning program id (as Sanctum does) and the length, and applies the
   result.
3. **Permissions are data, fixed at pool creation.**
   - The pool stores `(hook_program, permissions)` as part of its identity. The pool address is
     derived from the mints, hook program, fee config and salt.
   - Permissions are a strict subset of what the hook declared at registration. The core calls only
     permitted callbacks.
   - Returns-delta permissions require their base callback, as in v4.
4. **Custom accounting is bounded by conservation.** A hook delta moves value between the trader /
   LP and the **hook's balance in the core**: a per-(hook, pool, mint) credit, like ERC-6909 claims.
   - It can never draw on other pools, other hooks, or LP principal it was not given.
   - The user's slippage limit is enforced by the core after all deltas.
   - Exact-in / exact-out cannot flip.
   - Vault balances must equal accounted reserves plus hook credits plus protocol fees after every
     instruction.
5. **The caller is authenticated for hooks by construction.**
   - The core passes a context (pool, operation, caller, amounts, sequence) and calls the hook with
     a **callback-authority PDA as a signer**.
   - That PDA is derived per hook program: `["callback", hook_program_id]` under the core.
   - It holds no other authority: no vaults, no mints, no admin role.
   - Because it is per hook, hook A cannot forward its signer to hook B and impersonate the core:
     B accepts only the PDA derived with B's id.
   - The hook SDK checks the signer before anything else (an `only_core` check generated, not
     hand-written).
   - A top-level call to a hook cannot forge it, which closes the Cork class of bugs.
6. **Flash accounting is per instruction.** `execute(actions[])` runs swaps and liquidity
   operations across pools with net deltas and settles once at the end. That covers multi-hop and
   hook-plus-swap compositions without transient storage.
7. **Every callback is opt-in, and accounts are declared.**
   - A hook's extra accounts come from a hook-owned config the core does not interpret. It follows
     the transfer-hook `ExtraAccountMetaList` idea, so SDKs resolve accounts automatically.
   - Caller-supplied `hook_data` is opaque and untrusted.
8. **Depth budget.** Core → hook is one CPI. Through an aggregator, hooks get one further CPI until
   SIMD-0268 activates. The SDK and simulator must report stack height.
9. **Token policy.**
   - SPL Token and Token-2022 with an extension allowlist (Raydium's list plus transfer-fee
     accounting by vault-balance deltas).
   - Transfer-hook mints, permanent delegate, non-transferable, default-frozen and confidential
     transfers are rejected in V1.
   - No dependency on Pump: Pump tokens are ordinary SPL / Token-2022 mints.
10. **Hook-owned balances enable deferred mechanisms.** TWAMM, batch auctions, limit and range orders
    take user input into the hook's credit in one transaction and settle later via the hook's own
    crank instruction, which calls the core as a top-level caller.
11. **No fake trust signals.**
    - Registry entries carry *verifiable* facts (program id, upgrade authority, executable data
      hash, declared permissions) and evidence-backed attestations from named attesters.
    - Never a self-assigned "audited" flag.
    - The UI shows upgradeable hooks as upgradeable.

## 6. Open risks

- **Upgradeable hooks.**
  - A pool's hook program can change behavior after LPs deposit.
  - Mitigation: surface the upgrade authority prominently.
  - Optionally let pools require an immutable hook, or pin a program-data hash checked on every
    call. Hashing the program data per call is too expensive, so pin at creation and check the
    `ProgramData` last-deployed slot.
- **Malicious custom-accounting hooks** can still extract value up to user slippage, exactly as in
  v4. This must be visible in the UI and the SDK quote (simulated, not assumed).
- **Compute.** A hook with 4+ callbacks may exceed router budgets. Benchmark per callback.
- **Oracle-priced hooks** depend on oracle freshness. That is the hook's responsibility, but the
  registry should expose which oracles a hook reads.
