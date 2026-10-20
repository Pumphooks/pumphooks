# PUMP HOOKS ABI v1

The shared standard for hooks in four execution environments: **tokens, transactions (router),
liquidity pools and smart accounts**.

The environments share a model, not an instruction format. Every hook has:

- one manifest format;
- one capability vocabulary;
- one configuration format;
- one registry;
- one composition and validation model.

Each environment then has its own **execution adapter**, because Solana gives each one a different
real enforcement point. A Token-2022 transfer hook and a pool swap hook have different on-chain
interfaces, and the standard makes that explicit.

Rust types live in `crates/hook-standard`; TypeScript types and Zod schemas live in
`packages/hook-types`. They are kept in sync by golden-vector tests.

---

## 1. Environments and lifecycle callbacks

Every callback carries an **enforcement class**:

| Class | Meaning |
| --- | --- |
| `NATIVE` | Invoked by another protocol's runtime. For example, Token-2022 invokes the transfer-hook program during `TransferChecked`. It cannot be bypassed for that asset. |
| `PROGRAM` | Invoked by our own program, which owns the state being changed (pools, smart accounts). It cannot be bypassed for that state. |
| `ROUTER` | Invoked by our router around an operation on a third-party venue. **Bypassable:** anyone can call the venue directly. |
| `WRAPPER` | Invoked by our token-wrapper program around wrap and unwrap. It cannot be bypassed for the wrapped asset, but it has no effect on the underlying token. |
| `UNAVAILABLE` | The environment has no enforceable point for this callback. The standard says so, rather than faking it. |

Off-chain observation is **not** a hook class. Indexers and keepers may observe events, but nothing
observed off-chain is called "enforced".

### TOKEN (wrapped Token-2022 assets)

| id | callback | class | notes |
| --- | --- | --- | --- |
| 0 | `beforeTransfer` | NATIVE | The Token-2022 `Execute` CPI, issued during `TransferChecked` / `TransferCheckedWithFee`. Balances have already moved inside the same instruction, and a failure reverts the transfer. Accounts arrive read-only and non-signer. |
| 1 | `afterTransfer` | UNAVAILABLE | Token-2022 has a single callback point. Post-transfer logic must run inside that same `Execute`, or in a router or account flow that performs the transfer. |
| 2 | `beforeWrap` | WRAPPER | Deposit of the underlying token and mint of the wrapped asset. |
| 3 | `beforeUnwrap` | WRAPPER | Burn of the wrapped asset and release of the underlying token. |

### ROUTER (voluntary routing to third-party venues)

| id | callback | class |
| --- | --- | --- |
| 0 | `beforeExecute` | ROUTER |
| 1 | `afterExecute` | ROUTER |
| 2 | `beforeSwap` | ROUTER |
| 3 | `afterSwap` | ROUTER |

### POOL (pools created by our PoolManager only)

| id | callback | class |
| --- | --- | --- |
| 0 | `beforeInitialize` | PROGRAM |
| 1 | `afterInitialize` | PROGRAM |
| 2 | `beforeAddLiquidity` | PROGRAM |
| 3 | `afterAddLiquidity` | PROGRAM |
| 4 | `beforeRemoveLiquidity` | PROGRAM |
| 5 | `afterRemoveLiquidity` | PROGRAM |
| 6 | `beforeSwap` | PROGRAM |
| 7 | `afterSwap` | PROGRAM |
| 8 | `beforeDonate` | PROGRAM |
| 9 | `afterDonate` | PROGRAM |
| 10 | `quote` | PROGRAM |

The pool wire format is defined in [engines/pool-hook-interface.md](engines/pool-hook-interface.md).

### ACCOUNT (our smart accounts)

| id | callback | class |
| --- | --- | --- |
| 0 | `beforeDeposit` | PROGRAM |
| 1 | `afterDeposit` | PROGRAM |
| 2 | `beforeWithdraw` | PROGRAM |
| 3 | `afterWithdraw` | PROGRAM |
| 4 | `beforeExecute` | PROGRAM |
| 5 | `afterExecute` | PROGRAM |
| 6 | `beforeAuthorityChange` | PROGRAM |
| 7 | `afterAuthorityChange` | PROGRAM |

Account deposit hooks apply only when assets arrive through the account's `deposit` instruction. A
plain SPL transfer into the vault cannot be intercepted by anyone, so it is not a hooked deposit.

## 2. Capabilities

A capability is **requested** in the manifest and **granted** per installation. A request alone
grants nothing. Enforcement comes only from:

- which accounts the environment forwards;
- the privileges it forwards them with;
- which result fields it honors.

The kernel never claims to sandbox a program's own powers. A hook program can always do whatever
its own keys and PDAs allow; it just gets nothing extra from us.

| bit | capability | what the environment actually does |
| --- | --- | --- |
| 0 | `READ_ACCOUNT_STATE` | Forwards the operation's state accounts read-only. |
| 1 | `READ_TOKEN_BALANCE` | Forwards the relevant token accounts read-only. |
| 2 | `VALIDATE_OPERATION` | Calls the hook and requires a well-formed result. |
| 3 | `VETO_OPERATION` | Honors a `REJECT` decision. Without it, a reject is a capability violation, and the operation still fails (fail-closed). |
| 4 | `UPDATE_OWN_STATE` | Forwards hook-declared accounts as writable **only if they are owned by the hook program**. |
| 5 | `REQUEST_AUTHORIZED_TRANSFER` | Honors a transfer request, but only from the hook's own escrow or credit. Never from user funds. |
| 6 | `REQUEST_AUTHORIZED_SWAP` | Pools only: honors swap deltas, bounded by the trader's limits. |
| 7 | `REQUEST_AUTHORIZED_FEE_ALLOCATION` | Honors a fee override within the installation's bounds. |
| 8 | `CUSTOM_PRICING` | Pools only: the `quote` callback replaces the curve. |
| 9 | `PRODUCE_STATE` | May write typed state records ([§7](#7-cross-environment-state)) under its own program. |
| 10 | `CONSUME_STATE` | Forwards declared state records from other producers read-only. |

**Hooks never receive:**

- a user's signature (`is_signer` is cleared on every forwarded account);
- a custody authority (vault and pool PDAs never sign for a hook);
- the ability to change ownership or remove another hook.

## 3. Hook manifest

The manifest is JSON, validated by `HookManifestSchema` (Zod). Its sha256 over the canonical JSON
form (sorted keys) is stored on-chain in the registry, so clients can detect a manifest that was
swapped behind the URI.

```json
{
  "standard": "pump-hooks/v1",
  "slug": "spending-limit",
  "name": "Daily spending limit",
  "version": "1.0.0",
  "implementation": { "kind": "builtin", "environment": "account", "builtinId": 1 },
  "environments": { "account": ["beforeWithdraw", "beforeExecute", "afterExecute"] },
  "capabilities": ["VALIDATE_OPERATION", "VETO_OPERATION", "READ_TOKEN_BALANCE"],
  "config": { "schema": { "dailyLimit": "u64", "mint": "pubkey" } },
  "outputs": [{ "type": "SPEND_REMAINING", "valueType": "u64" }],
  "dependsOn": [],
  "conflictsWith": [],
  "requiredAccounts": [],
  "compute": { "beforeWithdraw": 2500 },
  "upgradePolicy": "immutable",
  "source": { "repo": "…", "path": "…", "commit": "…" }
}
```

`implementation.kind` is one of:

- **`builtin`**: a restricted, audited-by-construction policy evaluated *inside* the environment
  program, with no CPI. This is the default for security policies, which follows the brief's rule
  of preferring a restricted policy instruction set where isolation can't be guaranteed.
- **`program`**: an external hook program invoked through the environment adapter, with a
  restricted interface.

## 4. Registry (on-chain, `programs/hook-registry`)

- **`Hook`** `["hook", slug]`: developer authority, latest version, status.
- **`HookVersion`** `["version", hook, version_u32]`. Fields:
  - implementation (builtin id or program id), environments bitmask, per-environment callback
    masks, requested capabilities;
  - config schema hash, manifest hash and URI, artifact hash (sha256 of the `.so`);
  - upgrade policy, with the program's upgrade authority and deploy slot as observed at
    publication;
  - status: `Active`, `Deprecated` or `Revoked`.

  Versions are append-only; a version's fields never change after publication except its status.
- **`Review`** `["review", version, reviewer]`. A signed statement by a named reviewer, with a kind
  and evidence hash and URI:
  - `CompatibilityTested`;
  - `IndependentAudit`;
  - `SecurityWarning`.

**Registration is not a review.** The UI shows three separate tiers:

1. **Community published**: anyone can publish.
2. **Compatibility tested**: a test-run attestation exists, with its evidence.
3. **Independently audited**: an `IndependentAudit` review exists from a named reviewer, with a
   report hash.

Nothing renders an audit badge without a review account and its evidence.

## 5. Installation and composition (the hook stack)

Every environment stores an installation as a **hook stack**. Composition is defined once, in
`crates/hook-kernel`, and every environment that runs stacks uses it.

```
HookStack { version, entries[≤8] }
Entry {
  hook_version: Pubkey   // registry HookVersion, which pins the version
  impl: Builtin(id) | Program(program_id)
  callbacks: u16         // ⊆ the version's callbacks for this environment
  granted: u32           // ⊆ the version's requested capabilities
  mandatory: bool
  depends_on: u8         // bitmask of EARLIER entry indices whose outputs this entry consumes
  config: [u8; 96]       // fixed layout per hook; larger configs use a config account
  state:  [u8; 32]       // builtin-owned mutable state (e.g. spent in window)
  pinned_slot: u64       // program hooks: required ProgramData deploy slot (0 = unpinned)
}
```

### Semantics

**Monotone restriction.** A stack can only *restrict* an operation. Every applicable entry runs in
order, and any `REJECT`, error or malformed result fails the whole operation (fail-closed). No
entry can turn another entry's rejection into an approval, so **no hook can override a mandatory
restriction**.

**Mandatory versus optional.** This difference concerns *lifecycle*, not evaluation:

- A mandatory entry can only be removed, reordered, or have its config loosened through the
  environment's guarded path. Smart accounts, for example, require a timelocked policy change,
  signed by the owner and, if one is set, the recovery authority.
- An optional entry can be removed by the installation authority directly.

**Ordering and dependencies.** An entry may depend only on earlier entries, so the dependency graph
is acyclic by construction.

**Typed outputs.** A hook may emit `(output_type, value)`. Builtins emit outputs directly; program
hooks emit them in an authenticated result (the return-data program id must equal the hook). A
dependent entry reads a typed output only from the producer index it declared. An output from any
other source is never trusted.

**Conflicts.** The manifest's `conflictsWith` list is checked at install time, both in the SDK and
on-chain where versions are known.

**Bounds.** At most 8 entries, at most 2 program-hook CPIs per callback, and a per-callback compute
estimate summed from the manifests.

### Plan determinism

`composeHooks()` produces an `InstallationPlan`. Its canonical encoding is the exact bytes of the
on-chain stack, and the plan hash is the sha256 of those bytes, so the same input always yields the
same plan, the same bytes and the same hash.

## 6. Program-hook call format (policy environments)

The pool environment uses its own frozen format
([engines/pool-hook-interface.md](engines/pool-hook-interface.md)), because a pool hook can return
deltas and quotes.

The token, router and account environments share a **policy call**:

```
data     = sha256("pump-hooks-v1:<environment>:<callback>")[..8] ‖ PolicyContext ‖ op_data
accounts = [0] environment's callback-authority PDA ["callback", hook_program] (signer)
           [1] subject (pool / smart account / wrapper / route market), read-only
           [2..] forwarded accounts per granted capabilities (never a signer)
result   = PolicyResult (40 bytes) via set_return_data:
           version u8, decision u8, reason u16, output_type u16, _pad u16,
           output_value u64, _reserved[24] (zero)
```

The token environment's `beforeTransfer` is the exception: Token-2022 defines that interface, with
the `spl-transfer-hook-interface:execute` discriminator and its own account order. Token hooks
therefore implement that interface directly, and the registry records them with
`environment = token`.

## 7. Cross-environment state

A **state record** is an account owned by its **producer program**, with a fixed header:

```
StateHeader {
  magic "PHST", version u8, schema u16,
  namespace [u8; 32],   // e.g. sha256("allowlist:v1")
  producer: Pubkey,     // must equal the account owner
  subject: Pubkey,      // what the record is about (wallet, mint, account)
  issuer: Pubkey,       // who caused it (e.g. the list authority)
  slot u64, timestamp i64, sequence u64
}
followed by a schema-specific body.
```

A consumer checks four things:

1. The account's **owner** is the expected producer program. The program id comes from the
   consumer's configuration, never from the record.
2. The **schema** and **namespace** match.
3. The record is **fresh**: `now − slot ≤ max_age`, when configured.
4. **Replay**, where relevant: the consumer stores the last sequence it consumed and requires a
   strictly greater one.

Composition can happen in two ways:

- **Synchronously**, in one transaction: the producer writes, then the consumer reads, as long as
  CPI and account limits allow.
- **Asynchronously**: a later transaction consumes previously committed records. Nothing runs
  automatically. A keeper may submit that transaction, but the consumer re-validates everything
  on-chain.

## 8. Versioning

- `standard` is `pump-hooks/v1`. Callback ids, capability bits and wire layouts are frozen; later
  versions only append.
- Every installation pins a `HookVersion`. Program hooks may also pin a **deploy slot**: the
  environment refuses to call a redeployed program until the installation is re-pinned.
