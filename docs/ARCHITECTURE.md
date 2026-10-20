# PUMP HOOKS architecture

> The programmable execution layer for Solana: hooks for **tokens, transactions, liquidity pools
> and smart accounts**, under one standard ([STANDARD.md](STANDARD.md)).

## 1. System map

```
                         ┌──────────────── hook-registry (program) ────────────────┐
  developers ──publish──▶│ Hook · HookVersion (pinned, append-only) · Review        │
                         └───────────────▲───────────────────────▲─────────────────┘
                                         │ installs reference     │ reads
                                         │ versions               │
  ┌───────────── crates/hook-standard + crates/hook-kernel (shared, compiled into each engine) ──┐
  │ environments · callbacks · capabilities · HookStack · monotone evaluation ·                  │
  │ builtin policies · PolicyCall · StateHeader                                                   │
  └───────┬──────────────────┬──────────────────────┬───────────────────────┬────────────────────┘
          │                  │                      │                       │
   TOKEN engine        ROUTER engine          POOL engine             ACCOUNT engine
   token-wrapper +     pump-hooks router      hooks-core              smart-accounts
   token-hooks         (PumpSwap / Pump       (PoolManager, our       (vault PDAs, hook stack,
   (Token-2022         curve adapters)        own pools only)         proposals, sessions)
   TransferHook)       ROUTER-enforced,       PROGRAM-enforced        PROGRAM-enforced
   NATIVE + WRAPPER    bypassable
          │                  │                      │                       │
          └──────── cross-environment state records (owner-verified, typed, fresh) ───────────┘

  off-chain: @pumphooks/sdk · hook-types (Zod) · composer · CLI · indexer (Postgres) · web
```

## 2. Components

| Component | Path | Status |
| --- | --- | --- |
| ABI v1 types | `crates/hook-standard`, `packages/hook-types` | built, 6 unit tests |
| Composition kernel and builtin policies | `crates/hook-kernel` | built, 14 unit and property tests |
| Hook registry | `programs/hook-registry` | built, 7 LiteSVM tests |
| ROUTER engine | `programs/hook-router` (registry + kernel) | built, 11 LiteSVM tests against the real PumpSwap binary |
| POOL engine (PoolManager) | `programs/hooks-core` (registry + kernel) | built, 25 LiteSVM tests + 11 engine unit and property tests |
| ACCOUNT engine | `programs/smart-accounts` (registry + kernel) | built, 13 LiteSVM tests |
| TOKEN engine | `programs/token-wrapper`, `programs/token-hooks` | **not built** |
| POOL example hooks | `programs/example-hooks/pool-*` | planned; `programs/test-hook` is a scripted test hook |
| Legacy Pump router (v0) | `programs/pump-hooks` (own policy VM, not ABI v1) | built, LiteSVM-tested against the real Pump binaries |
| Policy VM (bounded WHEN/IF/THEN) | `crates/policy-vm` | built |
| SDK, composer, CLI | `packages/sdk`, `packages/hook-types`, `packages/cli` | SDK and CLI exist for the router; extending |
| Indexer | `services/indexer` | built (file store); Postgres store pending |
| Web | `apps/web` | built for the router; expanding to the full page set |

## 3. Key decisions

1. **Builtins first for security policies.**
   - Spending limits, timelocks, multisig, access control and transaction-size caps are
     *builtins*: restricted policies evaluated inside the environment program, with no CPI.
   - Program hooks exist for extensibility, but they only ever get a restricted, privilege-dropped
     interface.
   - This follows the brief: "prefer a restricted, audited policy instruction set over arbitrary
     CPI".
2. **Monotone composition.** Hook stacks can only restrict operations. Fail-closed. Mandatory
   entries are protected at the lifecycle level, by timelocked removal.
3. **Custody never leaves our programs.**
   - Pool vaults belong to pool PDAs; smart-account assets to vault PDAs; wrapper reserves to
     wrapper PDAs.
   - No hook ever receives a signer for these. The only asset movement a hook can request is from
     its own credit or escrow (pools), bounded by the user's limits.
4. **Honest enforcement classes.**
   - Router hooks are bypassable.
   - Token hooks are enforced only for the wrapped asset, at Token-2022's single callback point.
   - Pool hooks govern only our pools.
   - See [PROTOCOL_LIMITATIONS.md](PROTOCOL_LIMITATIONS.md).
5. **Versioned registry with explicit trust tiers.** Community published, compatibility tested and
   independently audited are distinct and evidence-backed.
6. **Localnet by default.** Devnet hosts the PoolManager only, for now. Mainnet is disabled until an
   independent review.

## 4. Engine summaries

### POOL ([engines/pool-manager.md](engines/pool-manager.md))

- Constant-product pools with fungible LP shares, keyed by mints, hook and salt.
- Hooks get v4-style callbacks, bounded return deltas against their own credit, dynamic fees
  within `max_fee_ppm`, and an optional custom-curve `quote`.
- A pool's hook is installed from a `hook-registry` Pool version that declares its callbacks and
  capabilities; pinning ties the pool to the binary that version observed.
- A pool may also carry a builtin ABI v1 stack (allowlist, max-size, time-window, accounting, ...)
  evaluated by `hook-kernel`. It can gate entry and never withdrawals.
- The settlement engine is property-tested for token conservation under arbitrary hook results.

### ACCOUNT

- A smart account is a PDA that owns a vault PDA.
- **Authorities:** an owner, an optional recovery authority, and sessions (scoped, expiring keys).
- **Operations:** `deposit`, `withdraw`, `execute` (CPI with the vault's signature, after policy
  approval) and `change_authority`. Each runs the account's hook stack.
- **Execution checks:** `execute` measures declared balances before and after, so spending policies
  hold even for arbitrary CPIs.
- **Proposals:** multisig and timelock policies use proposal accounts, which bind an action hash
  to approvals and a created-at time.

### TOKEN

- `token-wrapper` holds an existing SPL or Token-2022 asset in a reserve and mints a 1:1
  Token-2022 wrapped asset with a `TransferHook` extension.
- `token-hooks` implements `spl-transfer-hook-interface` with allowlist, timelock and accounting
  modes. The accounting mode writes typed state records.
- The original token is never modified.

### ROUTER

- The `hook-router` program routes voluntary trades to allowlisted venues via CPI. It runs the
  route policy's ABI v1 stack before and after, then measures the user's token and SOL deltas and
  enforces max-in, min-out and a SOL cap after the venue returns.
- The older `pump-hooks` program (v0, PumpSwap and the Pump bonding curve) predates ABI v1 and uses
  its own policy VM.
- Other venues are documented with their verified CPI surfaces and status in
  [RESEARCH.md](RESEARCH.md). Adapters are only added when they can be tested against the real
  program binary.

## 5. Trust boundaries

See [THREAT_MODEL.md](THREAT_MODEL.md).

Users trust:

- our program code: unaudited, so localnet and devnet only;
- the upgrade authority of our programs, which is disclosed on `/status`;
- any program hook they choose to install, within its granted capabilities.

Users never have to trust:

- the registry's publishers, since registration is not a review;
- off-chain indexers, for anything enforcement-related.

## 6. Build plan

The order follows the brief. Each step ends with passing tests and a commit.

1. Standard types and kernel, with unit and property tests.
2. Registry program.
3. Smart accounts with builtin hooks.
4. Router: deadline, receipts, kernel-backed stacks.
5. Token wrapper and token hooks.
6. Pool example hooks, and registry-pinned pool installs.
7. Cross-environment state records.
8. SDK, composer and CLI.
9. Web: all pages, composer UI, demos.
10. Security and property suites, benchmarks.
11. Docs and status.
