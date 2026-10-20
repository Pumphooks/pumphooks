# Research: EVM hook architectures beyond Uniswap v4

Companion to [../research.md](../research.md), whose §1 covers Uniswap v4. This file does not repeat
that material; it cross-references it as "research.md §1.x".

Sources were read on 2026-10-20 from source tarballs (GitHub `codeload`, `HEAD`) unpacked under
`.ref/research/<owner>_<repo>`. Citations use `repo: path:line`.

| Source | Commit | What was read |
| --- | --- | --- |
| `balancer/balancer-v3-monorepo` | `2b9ce4b1b` (2026-09-13) | `pkg/interfaces/contracts/vault/{IHooks,VaultTypes,ISenderGuard}.sol`, `pkg/vault/contracts/{Vault,VaultExtension,VaultCommon,BaseHooks,VaultGuard,SenderGuard}.sol`, `lib/HooksConfigLib.sol`, `pkg/pool-hooks/contracts/*`, `pkg/standalone-utils/contracts/BalancerContractRegistry.sol`, `audits/` |
| `pancakeswap/infinity-core` | `3c5ef1be1` (2026-10-01) | `Vault.sol`, `libraries/{Hooks,LPFeeLibrary,SettlementGuard}.sol`, `libraries/math/ParametersHelper.sol`, `types/PoolKey.sol`, `pool-cl/*`, `pool-bin/*` |
| `pancakeswap/infinity-periphery` | `56a7a4f50` (2026-10-01) | `base/BaseActionsRouter.sol`, `pool-cl/CLPositionManager.sol`, `InfinityRouter.sol` |
| `pancakeswap/infinity-hooks` | `644c30f1c` (2025-03-05) | `pool-cl/CLBaseHook.sol`, `pool-cl/vecake-exclusive/CLVeCakeExclusiveHook.sol` |

Web sources are listed in §5.

---

## 1. Balancer v3 hooks

### 1.1 Model in one paragraph

Balancer v3 separates the **pool** (a contract that implements only the invariant math via
`IBasePool.onSwap`, etc.) from an optional **hooks contract** chosen when the pool is registered with
the Vault. The Vault holds all tokens, runs operations inside a transient `unlock` session (as v4
does), and calls the hooks contract at fixed points. One hooks contract can serve many pools. A
pool's hooks contract is set once at registration and never changes: the only production write to
`_hooksContracts[pool]` is in `_registerPool`
(`balancer-v3-monorepo: pkg/vault/contracts/VaultExtension.sol:325`).

### 1.2 `IHooks` callbacks

All from `pkg/interfaces/contracts/vault/IHooks.sol`. Every callback returns a `bool success`; the
Vault reverts with a hook-specific error when it is `false` (`HooksConfigLib.sol:186-189, 206-210,
260-263, 295-307, 346-348, 383-395, 434-436, 467-469, 485-487`).

| Callback | Lines | Inputs (abridged) | Returns |
| --- | --- | --- | --- |
| `onRegister` | 42-47 | `factory, pool, TokenConfig[], LiquidityManagement` | `bool` (false ⇒ registration reverts) |
| `getHookFlags` | 57 | — | `HookFlags` |
| `onBeforeInitialize` | 72 | `exactAmountsIn, userData` | `bool` |
| `onAfterInitialize` | 84-88 | `exactAmountsIn, bptAmountOut, userData` | `bool` |
| `onBeforeAddLiquidity` | 108-116 | `router, pool, kind, maxAmountsInScaled18, minBptAmountOut, balancesScaled18, userData` | `bool` |
| `onAfterAddLiquidity` | 135-144 | `router, pool, kind, amountsInScaled18, amountsInRaw, bptAmountOut, balancesScaled18, userData` | `(bool, uint256[] hookAdjustedAmountsInRaw)` |
| `onBeforeRemoveLiquidity` | 164-172 | `router, pool, kind, maxBptAmountIn, minAmountsOutScaled18, balancesScaled18, userData` | `bool` |
| `onAfterRemoveLiquidity` | 191-200 | `router, pool, kind, bptAmountIn, amountsOutScaled18, amountsOutRaw, balancesScaled18, userData` | `(bool, uint256[] hookAdjustedAmountsOutRaw)` |
| `onBeforeSwap` | 215 | `PoolSwapParams, pool` | `bool` |
| `onAfterSwap` | 227-229 | `AfterSwapParams` | `(bool, uint256 hookAdjustedAmountCalculatedRaw)` |
| `onComputeDynamicSwapFeePercentage` | 247-251 | `PoolSwapParams, pool, staticSwapFeePercentage` | `(bool, uint256 dynamicSwapFeePercentage)` — declared **`view`** |

`PoolSwapParams` carries `kind, amountGivenScaled18, balancesScaled18, indexIn, indexOut, router,
userData`; `AfterSwapParams` adds token addresses, in/out amounts, post-swap balances,
`amountCalculatedScaled18/Raw` and `pool` (`VaultTypes.sol:307-345`). The **router address is an
explicit argument to every liquidity and swap hook**. `userData` is the hook-data equivalent.

There is no donate hook. Donation is a liquidity kind (`AddLiquidityKind.DONATION`) gated by
`LiquidityManagement.enableDonation` (`VaultTypes.sol:17-22`).

### 1.3 `HookFlags` and how they are stored

```text
struct HookFlags {                       // VaultTypes.sol:59-70
    bool enableHookAdjustedAmounts;
    bool shouldCallBeforeInitialize;
    bool shouldCallAfterInitialize;
    bool shouldCallComputeDynamicSwapFee;
    bool shouldCallBeforeSwap;
    bool shouldCallAfterSwap;
    bool shouldCallBeforeAddLiquidity;
    bool shouldCallAfterAddLiquidity;
    bool shouldCallBeforeRemoveLiquidity;
    bool shouldCallAfterRemoveLiquidity;
}
```

- The hook **self-declares** its flags through `getHookFlags()`. The Vault reads them **once, at
  registration**, and copies them into the pool's packed config word `_poolConfigBits[pool]`
  (`VaultExtension.sol:290-321`; packing helpers in `HooksConfigLib.sol:37-148`).
- After that, the hook contract's `getHookFlags()` is never consulted again. Changing what the
  function returns later does not change any registered pool. The stored flags are readable as
  `HooksConfig` via `getHooksConfig(pool)` (`VaultExtension.sol:590`, `HooksConfigLib.sol:150-165`).
- So permissions are **data in Vault storage, per pool**, not address bits (contrast research.md §1.2).
  No address mining is needed.

### 1.4 Registration and factory allowlisting

`registerPool` (`VaultExtension.sol:150-172`) has no caller authorization beyond
`onlyVaultDelegateCall nonReentrant whenVaultNotPaused`. The caller (`msg.sender`) is recorded as the
pool's factory. When `poolHooksContract != 0` (`:277-322`):

1. The Vault calls `onRegister(msg.sender /*factory*/, pool, tokenConfig, liquidityManagement)`.
   If it returns `false`, registration reverts with `HookRegistrationFailed` (`:279-288`). The
   source comment says this is to "make sure that hook trusts the pool factory" (`:278`).
2. It reads `getHookFlags()` (`:292`).
3. It rejects `enableHookAdjustedAmounts == true` unless the pool sets
   `disableUnbalancedLiquidity` (`:294-304`). See §1.5.
4. It packs the flags and stores the hooks address (`:306-325`).

The hook decides who may attach it. The default in `BaseHooks.onRegister` is **deny all**
(`pkg/vault/contracts/BaseHooks.sol:24-32`). `IHooks.sol:32-34` tells implementers to "check the
factory, and check that the given pool is from the factory". Examples:

- `DirectionalFeeHookExample.onRegister` returns
  `factory == _allowedStablePoolFactory && IBasePoolFactory(factory).isPoolFromFactory(pool)`
  (`pkg/pool-hooks/contracts/DirectionalFeeHookExample.sol:58-67`).
- `VeBALFeeDiscountHookExample` does the same with `_allowedFactory`
  (`VeBALFeeDiscountHookExample.sol:59-72`).
- `ExitFeeHookExample`, `LotteryHookExample` and `FeeTakingHookExample` return `true` for any pool and
  carry a NOTICE that real hooks must not (`ExitFeeHookExample.sol:98-100`,
  `LotteryHookExample.sol:104-106`, `FeeTakingHookExample.sol:112-114`).
- `onRegister` can also validate pool shape. For example, `ExitFeeHookExample` reverts unless
  `enableDonation` is set (`:103-105`). `NftLiquidityPositionExample` requires donation and
  disabled unbalanced liquidity (`NftLiquidityPositionExample.sol:205-210`).

### 1.5 `enableHookAdjustedAmounts`: what after-hooks may change

- Without the flag, the Vault **calls** the after-hook but **ignores** the returned amounts
  (`HooksConfigLib.sol:265-268, 350-353, 438-441`; documented at `VaultTypes.sol:53-58` and
  `IHooks.sol:120-122, 176-178, 219-221`).
- With the flag, an after-hook may replace **only the calculated side** of the operation:
  - swap: `amountCalculated` (amount out for EXACT_IN, amount in for EXACT_OUT);
  - add liquidity: `amountsInRaw`;
  - remove liquidity: `amountsOutRaw`.

  The registration comment explains why: "the Vault only supports modifying the calculated part of
  the operation. As such, when a hook supports adjusted amounts, it cannot support unbalanced
  liquidity operations, as this would introduce instances where the amount calculated is the input
  amount (EXACT_OUT)" (`VaultExtension.sol:294-304`).
- **The user's limits are re-checked against the hook-adjusted values**:
  - swap: `HookAdjustedSwapLimit` if EXACT_IN output < `limitRaw` or EXACT_OUT input > `limitRaw`
    (`HooksConfigLib.sol:270-275`);
  - add: each adjusted amount in must be ≤ `maxAmountsIn[i]` (`:355-363`);
  - remove: each adjusted amount out must be ≥ `minAmountsOut[i]` (`:443-451`);
  - the adjusted array length must equal the original (`:346, :434`).
- The hook settles the difference itself through the Vault's session accounting. For example,
  `FeeTakingHookExample.onAfterSwap` lowers the output by `hookFee` (EXACT_IN) or raises the input by
  `hookFee` (EXACT_OUT). It then calls `_vault.sendTo(feeToken, address(this), hookFee)`, which books
  matching debt so the session still settles (`FeeTakingHookExample.sol:134-178`).
  `ExitFeeHookExample` instead donates the withheld amounts back to the pool with
  `_vault.addLiquidity(... kind: DONATION ...)` (`ExitFeeHookExample.sol:133-172`).
- Compared with v4 (research.md §1.4): v4 return deltas can rewrite the **specified** amount before
  the curve runs (NoOp / custom curve). Balancer has no before-swap delta: `onBeforeSwap` returns only
  `bool`. **Balancer hooks cannot replace the curve.** Custom curves live in the pool contract.

### 1.6 Dynamic swap fees and bounds

- `onComputeDynamicSwapFeePercentage` runs after `onBeforeSwap` and before `_swap`
  (`Vault.sol:243-256`). It is declared `view` (`IHooks.sol:247-251`), so it cannot change state
  while computing a fee.
- The only bound the Vault applies to the **dynamic** fee is `MAX_FEE_PERCENTAGE = 99.9999%`
  (`VaultTypes.sol:439`; enforced at `HooksConfigLib.sol:191-195`). The reason given is that EXACT_OUT
  divides by the fee complement. Spearbit's review flagged the earlier 100% case as a
  division-by-zero (finding 5.4.9, `audits/spearbit/2024-10-04.pdf` p.22).
- The pool's own min/max (`ISwapFeePercentageBounds`) bound only the **static** fee
  (`VaultCommon.sol:354-368`). Examples: weighted pools allow 0.001%–10%
  (`pkg/pool-weighted/contracts/WeightedPool.sol:52-53`); stable pools allow 0.0001%–10%
  (`pkg/pool-stable/contracts/StablePool.sol:80-81`).
- `IHooks.sol:235-239` warns that pool minimum fees "represent an important security guardrail
  against attacks employing flash loans and approximations resulting from rounding". It says a
  dynamic fee below the minimum should be used "with extreme caution". The Vault itself does **not**
  enforce that floor.
- Hooks add their own caps. `MevCaptureHook` caps its maximum MEV fee at the Vault max
  (`MevCaptureHook.sol:31, 82, 213-214`). `ExitFeeHookExample` caps its exit fee at 10%
  (`ExitFeeHookExample.sol:47, 182-183`).

### 1.7 How the Vault limits hook abuse

1. **Caller authenticity is the hook's job, with a provided guard.** `VaultGuard.onlyVault` checks
   `msg.sender == _vault` (`pkg/vault/contracts/VaultGuard.sol:16-25`). `IHooks.sol:20-24` and every
   example hook use it.
2. **Opt-in at registration.** The hook must return `true` from `onRegister`, and the base default is
   deny (§1.4).
3. **Frozen flags.** Flags are snapshotted at registration (§1.3), and the hooks contract cannot be
   swapped (§1.1).
4. **Adjusted amounts are opt-in and bounded.** They need an explicit flag, affect only the
   calculated side, require balanced-only liquidity, and are re-checked against user limits (§1.5).
5. **Reentrancy is scoped.**
   - Hooks are reentrant by design. `swap` is `onlyWhenUnlocked`, not `nonReentrant`, and the source
     says "the swap hooks are reentrant and could do anything, including change these balances"
     (`Vault.sol:197-221`).
   - After `onBeforeSwap`, the Vault **reloads balances and rates** and recomputes the given amount
     (`Vault.sol:232-240`). Add and remove liquidity do the same (`:551, :824`).
   - The math-and-accounting core `_swap` is `nonReentrant` "to guard against read-only reentrancy"
     (`Vault.sol:367-400`), as are `settle` and `sendTo` (`:138, :158`).
6. **Sanity checks on hook output.** The Vault checks the returned bool and array lengths, and caps
   the fee (above).
7. **Hook-free exit.** `removeLiquidityRecovery` works only in Recovery Mode, computes proportional
   amounts directly, and calls no hooks (`VaultExtension.sol:749-790`). A broken or malicious hook
   therefore cannot trap LPs once governance or the pause manager enables Recovery Mode
   (`IVaultAdmin.sol:229-236`).

### 1.8 Router-trust model

- Inside a hook, `msg.sender` is the Vault. The Vault passes `router` (the contract that called the
  Vault) as an argument (`Vault.sol:270, 309`).
- Balancer routers implement `ISenderGuard.getSender()`. It returns the outermost caller, stored in
  transient storage by the `saveSender` modifier (`pkg/vault/contracts/SenderGuard.sol:33-69, 84-86`).
- **`getSender()` is only meaningful if the router is trusted.** Any contract can call the Vault and
  implement `getSender()` to return anything. `VeBALFeeDiscountHookExample` gives the discount only
  when `params.router == _trustedRouter`, because "`getSender` may be manipulated by a malicious
  router" (`VeBALFeeDiscountHookExample.sol:28-30, 76-94`). `LotteryHookExample` lets untrusted
  routers pay into the pot but makes them ineligible to win (`LotteryHookExample.sol:36-37, 127-132`).
- Trust can come from a governance registry. `MevCaptureHook` exempts senders only when
  `_registry.isTrustedRouter(params.router)` (`MevCaptureHook.sol:123-129`). `isTrustedRouter`
  returns whether the address is an active `ROUTER` entry in `BalancerContractRegistry`
  (`pkg/standalone-utils/contracts/BalancerContractRegistry.sol:301-303`). The constructor smoke-tests
  that the registry does not trust `address(0)` (`MevCaptureHook.sol:69-73`).
- A hook can also **be its own router**. `NftLiquidityPositionExample` rejects any router other than
  itself (`onlySelfRouter`, `NftLiquidityPositionExample.sol:109-112, 238, 253, 318-322`).
- Queries are a caveat. Query entry points store a caller-supplied `sender`, so "a hook that prices
  by sender identity will quote for the address passed in, and execute for the actual router caller"
  (`ISenderGuard.sol:13-20`).

### 1.9 Example hooks (paths under `pkg/pool-hooks/contracts/`)

| Hook | Flags used | What it shows |
| --- | --- | --- |
| `ExitFeeHookExample.sol` | `enableHookAdjustedAmounts`, `afterRemoveLiquidity` (`:114-120`) | Exit fee on proportional removes only. The fee is withheld from `amountsOutRaw` and donated to LPs (`:133-172`). |
| `DirectionalFeeHookExample.sol` | `computeDynamicSwapFee` (`:72`) | Higher fee for swaps that push the pool away from balance. Factory allowlist (`:58-67`). |
| `VeBALFeeDiscountHookExample.sol` | `computeDynamicSwapFee` (`:55`) | 50% fee discount for veBAL holders, only through a trusted router (`:76-94`). |
| `LotteryHookExample.sol` | `enableHookAdjustedAmounts`, `afterSwap` (`:116-120`) | Takes a fee into a pot. Trusted-router users can win it. Uses `block.prevrandao` as a toy RNG (`:246-250`). |
| `FeeTakingHookExample.sol` | adjusted amounts, `afterSwap/AddLiquidity/RemoveLiquidity` (`:124-130`) | Canonical "adjust then `sendTo`" settlement pattern (`:134-258`). |
| `NftLiquidityPositionExample.sol` | adjusted amounts, `beforeAddLiquidity`, `afterRemoveLiquidity` (`:220-225`) | Hook as router. Wraps LP positions as NFTs and applies a decaying exit fee. |
| `MevCaptureHook.sol` | `computeDynamicSwapFee`, `before{Add,Remove}Liquidity` (`:103-111`) | Priority-fee-based MEV tax with registry-trusted-router exemptions. |
| `StableSurgeHook.sol` / `ECLPSurgeHook.sol` (+ `SurgeHookCommon.sol`) | dynamic fee | Production "surge" fees. `audits/WONTFIX.md` notes the approximation breaks exact-in/exact-out equivalence for large swaps, but "any error computing a dynamic swap fee above the static swap fee percentage cannot lead to theft of funds". |

---

## 2. PancakeSwap Infinity (formerly v4)

### 2.1 Architecture

- **Vault and pool managers are separate contracts.** The `Vault` holds tokens and runs the
  transient settlement session (`lock` → `ILockCallback.lockAcquired` → revert unless all deltas are
  settled; `infinity-core: src/Vault.sol:74-86`).
- **Pool managers are "apps" registered by the Vault owner** (`registerApp`, `onlyOwner`,
  `Vault.sol:41-42`). Only registered apps may book deltas (`onlyRegisteredApp`, `:28-31, 96`).
- There are two managers today: `CLPoolManager` (concentrated liquidity) and `BinPoolManager`
  (Trader Joe-style liquidity bins).
- The Vault keeps per-app reserves (`reservesOfApp`, `Vault.sol:25`). One app cannot draw another's
  tokens. A mid-lock overdraft by an app is recorded as an `AppDeficit` and must be repaid before the
  lock is released (`Vault.sol:80-82, 208-235`).
- Only one locker may exist at a time: `setLocker` reverts on nested lock attempts
  (`libraries/SettlementGuard.sol:25-34`).

### 2.2 `PoolKey` and the hook registration bitmap

```text
struct PoolKey {                         // src/types/PoolKey.sol:12-25
    Currency currency0;
    Currency currency1;
    IHooks hooks;
    IPoolManager poolManager;            // which app
    uint24 fee;                          // 0x800000 = dynamic
    bytes32 parameters;                  // hooks bitmap + pool-type params
}
```

Layout of `parameters`:

| Bits | CL pools (`pool-cl/libraries/CLPoolParametersHelper.sol:10-20`) | Bin pools (`pool-bin/libraries/BinPoolParametersHelper.sol:9-17`) |
| --- | --- | --- |
| `[0,16)` | hooks registration bitmap (`ParametersHelper.OFFSET_HOOK = 0`, `getHooksRegistrationBitmap` decodes `uint16`; `libraries/math/ParametersHelper.sol:12-23`) | same |
| `[16,40)` | `tickSpacing` (24 bits) | `[16,32)` `binStep` (16 bits) |
| rest | must be zero (`checkUnusedBitsAllZero`, `ParametersHelper.sol:25-29`; enforced at `CLPoolManager.sol:101-103`, `BinPoolManager.sol:107-109`) | same |

Bit offsets inside the 16-bit bitmap (`pool-cl/interfaces/ICLHooks.sol:11-24`,
`pool-bin/interfaces/IBinHooks.sol:10-23`):

| Bit | CL | Bin |
| --- | --- | --- |
| 0 / 1 | before / after Initialize | before / after Initialize |
| 2 / 3 | before / after AddLiquidity | before / after Mint |
| 4 / 5 | before / after RemoveLiquidity | before / after Burn |
| 6 / 7 | before / after Swap | before / after Swap |
| 8 / 9 | before / after Donate | before / after Donate |
| 10 | beforeSwapReturnsDelta | beforeSwapReturnsDelta |
| 11 | afterSwapReturnsDelta | afterSwapReturnsDelta |
| 12 | afterAddLiquidityReturnsDelta | afterMintReturnsDelta |
| 13 | afterRemoveLiquidityReturnsDelta | afterBurnReturnsDelta |
| 14–15 | unused | unused |

How the bitmap is validated and used:

- **Double declaration, checked at initialize.** `Hooks.validateHookConfig` requires
  `key.hooks.getHooksRegistrationBitmap() == key.parameters` bitmap. With `hooks == 0`, the bitmap
  must be 0 and the fee must not be dynamic (`libraries/Hooks.sol:47-61`). The hook-side declaration
  is the one-function `IHooks` interface (`interfaces/IHooks.sol:4-6`). The base contract builds it
  from a `Permissions` struct (`infinity-hooks: src/pool-cl/CLBaseHook.sol:49, 301-305`).
- **Conflict check.** A returns-delta bit without its base bit reverts with
  `HookPermissionsValidationError` (`pool-cl/libraries/CLHooks.sol:23-51`; Bin equivalent at
  `pool-bin/libraries/BinHooks.sol:24-51`).
- **Runtime source of truth.** The pool manager reads the bitmap from the **key's `parameters`**:
  `shouldCall(parameters, offset, hook) = bit set && hook != msg.sender` (`Hooks.sol:71-73`). The
  hook-side bitmap is consulted only once, at initialize.
- Because `parameters` is part of `PoolKey`, the bitmap is part of the pool's identity (`PoolId`),
  just as v4's hook address is.
- **Stale comment.** `ICLHooks.sol:26-28` still says the manager decides "by inspecting the leading
  bits of the hooks contract address". That is inherited Uniswap documentation and contradicts the
  code above.

### 2.3 Callbacks

CL (`pool-cl/interfaces/ICLHooks.sol`):

| Callback | Lines | Returns |
| --- | --- | --- |
| `beforeInitialize(sender, key, sqrtPriceX96)` | 36 | `bytes4` |
| `afterInitialize(sender, key, sqrtPriceX96, tick)` | 44-46 | `bytes4` |
| `beforeAddLiquidity(sender, key, params, hookData)` | 54-59 | `bytes4` |
| `afterAddLiquidity(sender, key, params, delta, feesAccrued, hookData)` | 70-77 | `(bytes4, BalanceDelta)` |
| `beforeRemoveLiquidity(...)` | 85-90 | `bytes4` |
| `afterRemoveLiquidity(sender, key, params, delta, feesAccrued, hookData)` | 101-108 | `(bytes4, BalanceDelta)` |
| `beforeSwap(sender, key, params, hookData)` | 121-126 | `(bytes4, BeforeSwapDelta, uint24 lpFeeOverride)` |
| `afterSwap(sender, key, params, delta, hookData)` | 136-142 | `(bytes4, int128)` |
| `beforeDonate` / `afterDonate(sender, key, amount0, amount1, hookData)` | 151-172 | `bytes4` |

Bin (`pool-bin/interfaces/IBinHooks.sol`). The shape is the same, with these differences:

- `beforeMint` returns `(bytes4, uint24 lpFeeOverride)` (`:55-60`). The fee override applies when a
  mint into the active bin at a different ratio triggers an internal swap (the "composition fee",
  `:50-54`).
- `afterMint` / `afterBurn` return `(bytes4, BalanceDelta)` (`:70-76, 99-105`).
- `beforeSwap(sender, key, swapForY, amountSpecified, hookData)` returns
  `(bytes4, BeforeSwapDelta, uint24)` (`:119-125`).
- `afterSwap` returns `(bytes4, int128)` (`:136-143`).

The call mechanics match v4 (research.md §1.3):

- raw `call`, bubble revert, and selector match (`Hooks.sol:77-101`);
- delta-returning calls must return exactly 64 bytes (`:104-114`), and `beforeSwap` exactly 96
  (`CLHooks.sol:140-141`);
- no self-call (`Hooks.sol:71-73`).

### 2.4 Return deltas

These mirror v4 (research.md §1.4):

- The `beforeSwap` specified delta is added to `amountToSwap`. `HookDeltaExceedsSwapAmount` reverts
  if exact-in/exact-out would flip (`CLHooks.sol:150-160`).
- The `afterSwap` unspecified delta accumulates on top (`:172-181`).
- The combined hook delta is subtracted from the caller's delta: "the caller has to pay for (or
  receive) the hook's delta" (`:183-191`).
- The liquidity variant is in `CLHooks.sol:84-123`.
- The pool manager then books both deltas with
  `vault.accountAppDeltaWithHookDelta(key, delta, hookDelta)` (`CLPoolManager.sol:158, 211`). The
  Vault credits the hook's delta **to the hook address** (`Vault.sol:89-110`).
- If a delta bit is not set, the callback still runs, but its returned delta is ignored as zero
  (`Hooks.sol:108`).

### 2.5 Dynamic fee

- `fee == DYNAMIC_FEE_FLAG (0x800000)` makes the pool dynamic. The initial fee is 0, but the hook may
  set it in `afterInitialize` (`CLPoolManager.sol:107-109`; `LPFeeLibrary.sol:16-28`).
- Only the hook may call `updateDynamicLPFee`.
  - CL bounds it to `ONE_HUNDRED_PERCENT_FEE = 1_000_000` (`CLPoolManager.sol:254-260`).
  - Bin bounds it to **`TEN_PERCENT_FEE = 100_000`** (`BinPoolManager.sol:295-302`).
- Per-swap override: `beforeSwap` returns a fee with `OVERRIDE_FEE_FLAG (0x400000)` set. It is
  validated against the same caps:
  - CL: `CLPool.sol:227-228`;
  - Bin swap: `BinPool.sol:450-451`;
  - Bin mint composition fee: `BinPool.sol:123-124`.
- Bin pools also enforce the 10% cap on static fees at initialize (`BinPoolManager.sol:114-115`).
  **A hard cap that depends on the pool type** is a notable departure from v4's single 100% ceiling
  (research.md §1.5).

### 2.6 Pause behaviour

- `swap` and `donate` are `whenNotPaused` (`CLPoolManager.sol:162-167, 215-220`;
  `BinPoolManager.sol:133-137, 252-256`).
- CL `modifyLiquidity` blocks only **adds** while paused (`CLPoolManager.sol:131-132`). Bin `burn`
  has no pause modifier (`BinPoolManager.sol:224-227`).
- So withdrawals stay open when paused. **They still run the hooks**, so, unlike Balancer's Recovery
  Mode, a reverting hook can still block exits.

### 2.7 Router trust

- The `sender` passed to hooks is the pool manager's `msg.sender`, i.e. the locker or router
  (`CLHooks.sol:57, 77`).
- Periphery routers expose `msgSender()`, which returns the transient locker
  (`infinity-periphery: src/base/BaseActionsRouter.sol:55-58`;
  `pool-cl/CLPositionManager.sol:145-146`). This is the same convention as Uniswap's `IMsgSender`
  (`Uniswap/v4-periphery: src/interfaces/IMsgSender.sol:9-12`).
- Core has no trusted-router registry.
- The official sample `CLVeCakeExclusiveHook` gates swaps on `veCake.balanceOf(tx.origin)` and
  comments "Using tx.origin is a workaround for now. Do NOT use this in production"
  (`infinity-hooks: src/pool-cl/vecake-exclusive/CLVeCakeExclusiveHook.sol:51-63`).
- Caller authentication is provided by `CLBaseHook`: `poolManagerOnly`, `vaultOnly` (for
  `lockAcquired`) and `selfOnly` (`CLBaseHook.sol:78-93, 102, 114-118, 226-231`).

### 2.8 Differences from Uniswap v4

| Aspect | Uniswap v4 (research.md §1) | PancakeSwap Infinity |
| --- | --- | --- |
| Permission encoding | low 14 bits of hook address; CREATE2 salt mining | 16-bit bitmap in `PoolKey.parameters`, must equal `hook.getHooksRegistrationBitmap()` at initialize |
| Core layout | single `PoolManager` (pools + tokens) | `Vault` + owner-registered pool-manager apps with per-app reserves |
| Pool types | CL only | CL and Bin (different callback names and fee caps) |
| Pool-type params | `tickSpacing` field | packed into `parameters`; unused bits must be zero |
| Liquidity callbacks (Bin) | n/a | `beforeMint` may override the composition fee |
| Max dynamic LP fee | 100% | CL 100%, Bin 10% |
| Lock nesting | nested `unlock` reverts `AlreadyUnlocked` (`Uniswap/v4-core: src/PoolManager.sol:105`) | one locker at a time (`LockerAlreadySet`) |
| `PoolKey` includes manager | no | yes (`poolManagerMatch` check) |

---

## 3. Hook security incidents and recurring finding classes

Besides Cork (research.md §1.7):

1. **Bunni v2 (Sept 2, 2025, ~$8.4M), Uniswap v4 hook with internal accounting.**
   - Root cause: a floor-rounding error when updating idle balances in `withdraw()`. It undervalued
     total liquidity and was exploited with a flash-loan tick push, many tiny withdrawals and a
     sandwich.
   - Losses: ~$2.4M from USDC/USDT on Ethereum and ~$5.9M from weETH/ETH on Unichain. Bunni later
     declared bankruptcy on Oct 23, 2025.
   - Sources: [BlockSec](https://blocksec.com/blog/bunni-incident-repeated-small-withdrawals-compound-a-rounding-error-into-an-8.4m-drain),
     [Coinspect](https://www.coinspect.com/learn-evm-attacks/cases/bunni/),
     [Trail of Bits](https://blog.trailofbits.com/2026/07/30/building-secure-uniswap-v4-hooks).
   - Lesson: "each transaction satisfied the PoolManager's settlement invariant because the bug was
     in BunniHook's internal accounting" (Trail of Bits). Settlement checks do not protect a hook's
     own books.
2. **Trail of Bits' seven recurring hook failure patterns**
   ([Building secure Uniswap v4 hooks](https://blog.trailofbits.com/2026/07/30/building-secure-uniswap-v4-hooks),
   Jul 30, 2026, drawn from their audits and Solodit):
   1. Anyone can call your hook (Cork).
   2. Treating any pool as legitimate. Pool creation is permissionless, so attackers can attach your
      hook to pools with attacker-chosen tokens. Example:
      [Semantic Layer SVFHook finding](https://solodit.cyfrin.io/issues/chat-points-manipulation-by-adding-liquidity-to-custom-pools-via-svfhook-contract-spearbit-none-semantic-layer-pdf).
   3. Custom accounting leaks value (Bunni). Includes NoOp swaps and unbounded dynamic fees.
   4. Right logic in the wrong callback, i.e. before-state vs after-state. Example:
      [OpenZeppelin LiquidityPenaltyHook JIT bypass](https://solodit.cyfrin.io/issues/jit-liquidity-penalty-can-be-bypassed-openzeppelin-none-openzeppelin-uniswap-hooks-v110-rc-1-audit-markdown).
   5. Address bits are part of the API. A missing return-delta bit means the delta is silently
      treated as zero; proxies keep the bits while changing the code. Example:
      [Sorella Angstrom finding](https://solodit.cyfrin.io/issues/all-swaps-will-revert-if-the-dynamic-protocol-fee-is-enabled-since-hook-configsol-does-not-encode-the-afterswapreturndelta-permission-cyfrin-none-sorella-l2-angstrom-markdown):
      the `afterSwapReturnDelta` bit was missing, so every swap reverted `CurrencyNotSettled` once
      the fee was enabled.
   6. Hook failures block pool actions. A reverting `afterRemoveLiquidity` traps LPs.
   7. State can change during a callback sequence. A shared hook across pools allows nested actions
      to mutate cached state.
3. **Uniswap's own v4 Security Framework**
   ([developers.uniswap.org](https://developers.uniswap.org/docs/protocols/v4/security)) lists these
   misconfiguration classes:
   - **Permission encoding and salt-grinding pitfalls**: a wrong CREATE2 salt can disable required
     callbacks or enable undesired ones.
   - **Upgradeability hazards**: prefer versioned redeploys, use timelocked multisig upgrades.
   - **Dynamic-fee abuse**: "Fees can be raised selectively after seeing a user's trade".
   - **NoOp / BeforeSwapDelta risks**.
   - **Flash-accounting transient-state manipulation**.
   - **Token-type hazards**: fee-on-transfer, rebasing, ERC-777, pausable or blacklistable tokens.
4. **Balancer v3 audit notes relevant to hooks** (from `balancer-v3-monorepo: audits/`):
   - Spearbit (Nov 2024) 5.4.9: a dynamic-fee hook could set 100%, making EXACT_OUT divide by zero.
     The current Vault caps the fee at 99.9999% (§1.6).
   - Spearbit 5.2.7: a single global `userData` in batch routes makes per-hop hook data hard to pass.
   - `audits/WONTFIX.md` records the surge-hook approximation as known and argues that fees above
     static cannot steal funds.
5. **Context, not a hook bug: Balancer v2 (Nov 3, 2025, ~$116–128M).** Rounding in the upscale path
   of EXACT_OUT swaps in Stable / Composable Stable pools was chained through BatchSwaps
   ([Cointelegraph, Nov 5, 2025](https://cointelegraph.com/news/balancer-post-mortem-report-116m-hack)).
   It is relevant because Balancer v3's `IHooks.sol:235-239` justifies minimum swap fees as a
   rounding guardrail, and a dynamic-fee hook can bypass that guardrail.

---

## 4. Comparison

| Dimension | Uniswap v4 | Balancer v3 | PancakeSwap Infinity |
| --- | --- | --- | --- |
| **Permissions encoding** | Address bits (14 low bits), mined via CREATE2; immutable per hook address (research.md §1.2) | `HookFlags` struct (10 bools) returned by `getHookFlags()`, snapshotted into `_poolConfigBits[pool]` at registration (`VaultExtension.sol:290-321`) | 16-bit bitmap in `PoolKey.parameters[0,16)`, must equal `hook.getHooksRegistrationBitmap()` at initialize (`Hooks.sol:47-61`); part of `PoolId` |
| **Who attaches a hook** | Anyone calling `initialize` with any hook; hook may refuse in `beforeInitialize` | Pool factory at `registerPool`; hook must opt in via `onRegister(factory, pool, …)`; default deny (`BaseHooks.sol:24-32`); hook fixed forever | Anyone calling `initialize`; hook may refuse in `beforeInitialize`; pool managers themselves are owner-registered apps (`Vault.sol:41-42`) |
| **Custom accounting** | Signed return deltas: before-swap specified + unspecified, after-swap unspecified, after add/remove `BalanceDelta`; can replace the curve (NoOp) | `enableHookAdjustedAmounts`: after-hooks replace **only the calculated amount**; re-checked against user limits; requires balanced-only liquidity; no before-swap delta, so no curve replacement (`HooksConfigLib.sol:264-276, 350-365, 438-453`) | Same as v4 (4 returns-delta bits); delta credited to hook address (`CLHooks.sol:150-191`, `Vault.sol:89-110`) |
| **Dynamic fees** | `fee = 0x800000`; hook-only `updateDynamicLPFee`; per-swap override from `beforeSwap`; cap 100% | Separate `view` callback `onComputeDynamicSwapFeePercentage`; Vault caps at 99.9999%; pool min/max bound static fee only (`HooksConfigLib.sol:175-198`, `VaultCommon.sol:354-368`) | Same flags as v4; cap 100% (CL) / 10% (Bin), also for overrides and Bin mint composition fee (`CLPool.sol:227-228`, `BinPool.sol:123-124, 450-451`) |
| **Re-entrancy model** | Hooks run inside caller's `unlock`; may call the manager; manager skips hooks for hook-initiated actions (`noSelfCall`) | Hooks reentrant by design; Vault reloads balances after before-hooks; core `_swap`, `settle`, `sendTo` are `nonReentrant` (`Vault.sol:215-240, 367-400`) | Same as v4 (`shouldCall` skips self, `Hooks.sol:71-73`); one locker per tx (`SettlementGuard.sol:25-34`) |
| **Hook data** | `hookData` bytes per call, untrusted | `userData` bytes per call (global per batch route; Spearbit 5.2.7) | `hookData` bytes per call |
| **Caller authentication** | `BaseHook.onlyPoolManager` (hook author's job) | `VaultGuard.onlyVault` (hook author's job) | `CLBaseHook.poolManagerOnly` / `vaultOnly` (hook author's job) |
| **User identity / router trust** | `sender` = router; `IMsgSender.msgSender()` convention, unauthenticated | `router` arg + `ISenderGuard.getSender()`; hooks must allowlist the router (`VeBALFeeDiscountHookExample.sol:80-85`) or check a governance `BalancerContractRegistry.isTrustedRouter` (`MevCaptureHook.sol:123-129`) | `sender` = router; `msgSender()` convention; official sample uses `tx.origin` with "do not use in production" |
| **How trust is conveyed to users** | Address bits readable by anyone; risky deltas visible in the address | `getHooksConfig(pool)` exposes frozen flags; `enableHookAdjustedAmounts` is a single explicit "may change your amounts" flag; Recovery Mode gives a hook-free exit | Bitmap visible in `PoolKey` / `Initialize` event (`CLPoolManager.sol:120`); per-type fee caps |
| **Escape hatch** | none at core | `removeLiquidityRecovery` bypasses hooks (`VaultExtension.sol:749-790`) | Paused state still allows withdrawals, but hooks still run |

---

## 5. Sources

Code (local copies under `.ref/research/`):

- `balancer/balancer-v3-monorepo` @ `2b9ce4b1b8ad49eafcad8d136f0872ea7c7d5d53`
- `pancakeswap/infinity-core` @ `3c5ef1be1f71442d8bc60cc6fd11f9c677b714df`
- `pancakeswap/infinity-periphery` @ `56a7a4f5076d57d5ed90c38a00421ecbac0d649c`
- `pancakeswap/infinity-hooks` @ `644c30f1cb09e4eee04becb6198da1414bdb0e77`
- `balancer-v3-monorepo: audits/spearbit/2024-10-04.pdf` (Spearbit / Cantina review, Nov 27, 2024)
- `balancer-v3-monorepo: audits/WONTFIX.md`

Web (raw search/extract output in `.ref/research/web/`):

- [Building secure Uniswap v4 hooks — Trail of Bits](https://blog.trailofbits.com/2026/07/30/building-secure-uniswap-v4-hooks) (Jul 30, 2026)
- [Security Framework — Uniswap Developers](https://developers.uniswap.org/docs/protocols/v4/security)
- [Bunni Incident — BlockSec](https://blocksec.com/blog/bunni-incident-repeated-small-withdrawals-compound-a-rounding-error-into-an-8.4m-drain) (Feb 12, 2026)
- [Bunni — Coinspect Learn EVM Attacks](https://www.coinspect.com/learn-evm-attacks/cases/bunni/)
- [Balancer releases preliminary post-mortem after $116M hack — Cointelegraph](https://cointelegraph.com/news/balancer-post-mortem-report-116m-hack) (Nov 5, 2025)
- Solodit findings linked in §3 item 2 (cited through the Trail of Bits post; not opened directly)
