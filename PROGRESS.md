# PROGRESS

## STATUS

Phases 0–5 of the build order are done locally: the Hook ABI, runtime, registry, Pump adapter
and router exist, and routed trades on an existing Pump token work end to end. They are tested
against the real devnet `pump_amm` binary and the cloned state of an existing devnet pool.

The devnet deployment is blocked because the deployer has no devnet SOL.

## COMPLETED

- `docs/`: research, architecture, threat model (T1–T29), hooks spec, policy VM, benchmarks.
- `crates/hook-types`: the frozen ABI.
  - `HookCallback`, `HookContext` (272 B), `HookDecision`, capabilities, `HookSlot` (112 B),
    `UserState` (160 B), `StrategySignal` (96 B, domain-separated), reason codes and the module
    table.
- `crates/policy-vm`: bounded WHEN/IF/THEN evaluator.
  - No loops, no CPI, no arbitrary code.
  - At most 8 conditions and 4 actions per policy.
  - Validated at write time.
- `crates/hook-runtime`: phase executor shared by the program and the simulator.
  - Runs callbacks in priority order and short-circuits on the first veto.
  - Enforces capabilities; a hook acting beyond them is an error, not a denial.
  - Kill switch fails closed.
  - After-phase post-conditions revert the trade.
  - Modules: Launch Shield, Volume Guard, Volatility Guard, Holder XP (anti-wash vesting and
    forfeit), Treasury Router (config), TWAP (bounded config), SENTIENT (signed regimes) and Policy.
- `crates/pump-adapter`: Pump/PumpSwap IDs, PDAs, account layouts, and curve v3 / AMM v2
  instruction builders.
  - Byte-identical to `pump-rust-client` 0.4.1, checked by tests.
- `programs/pump-hooks` (Anchor 0.31.1, program id `91kRNewiEDrKVtUXpahsDXYiURuKQKwhCt2hnaKNfkgD`):
  - global config;
  - kill switch;
  - two-step authority transfer;
  - hook registry;
  - `create_market` on an existing Pump token, which auto-detects curve vs. migrated, the
    canonical pool, the quote mint, Token-2022, holder-reward, mayhem and creator-verified, and
    rejects cashback coins;
  - `sync_market` on migration;
  - install, remove and configure hooks;
  - create, update and close policies;
  - `route_amm` / `route_curve`: before hooks, then Pump CPI signed by the wallet, then observed
    deltas, then after hooks, then settlement and events;
  - `submit_signal` (ed25519 precompile verification).
- `tests/integration`: LiteSVM against the devnet `pump_amm` binary with a cloned existing pool.
- Scripts:
  - `find_devnet_pool.py` lists existing pools;
  - `fetch_fixtures.sh` snapshots the accounts;
  - `dump_programs.sh` dumps the Pump binaries.

## TESTS

`cargo test --workspace`: 73 passing.

| Suite | Tests |
| --- | ---: |
| hook-types | 13 |
| policy-vm | 20 |
| hook-runtime | 22 |
| pump-adapter (unit + equivalence vs pump-rust-client) | 6 |
| integration (LiteSVM, real Pump AMM) | 11 |

Integration coverage:

- the demo stack on an existing token;
- the exact trace order;
- `buy_v2` and `buy_exact_quote_in_v2`;
- a routed sell that forfeits XP;
- a Volume Guard denial that reverts everything without ever calling Pump;
- the kill switch;
- direct Pump trades bypassing hooks (the documented limitation, pinned by a test);
- substituted, foreign or missing accounts;
- Composer policies (deny and award);
- the SENTIENT signal path (unsigned, wrong signer, replay, defensive cap, expiry);
- compute benchmarks.

## CURRENT

- Devnet deployment and the milestone transaction on existing devnet pool
  `9Nhddo1PcCBms54wcLYbxoRcAJbpmSvHWMcbdMf9tdmZ`.
- Next phases: SDK, CLI, indexer and the web app.

## BLOCKERS

- **Devnet SOL.** The deployer `41L6nJ9wrsCK3RuXzvZafNcTWuSEBouvtWqanVdAKi6b` holds 0 SOL. The
  public faucet is rate-limited and the devnet-pow faucets are empty. About 4 devnet SOL covers
  deploying the 507 KB program, initializing, and the milestone trades. The key lives in
  gitignored `.keys/`.

## NEXT

1. Fund the deployer, then deploy to devnet. Initialize, register modules, and create a market on
   the existing devnet token `44yV5xk8…`. Install Volume Guard, TWAP, Holder XP and SENTIENT, and
   send a routed `buy_v2`. Decode the transaction with the indexer.
2. LiteSVM curve-route test on an existing un-migrated devnet curve.
3. `packages/sdk` (PumpHooksClient), `packages/cli`, `services/indexer`.
4. `apps/web`: `/hook` primary flow, `/hooks`, `/build`, `/market/[mint]`, `/tx/[sig]`.
5. Strategy vault (TWAP execution, treasury intents), simulator, marketplace manifests.

## KNOWN RISKS

- Hooks bind only routed trades. Direct Pump trades bypass them, and a test pins this.
- Pump program upgrades can change layouts or account sets. Mitigations: the adapter is verified
  against the official client, tests run against the deployed devnet binaries, and router reads
  fail closed on unexpected layouts.
- Pre-trade sizes are constant-product estimates. After-hooks use observed deltas, and Volume
  Guard re-checks after the trade.
- Holder XP is anti-wash rather than sybil-proof. Many wallets can each earn capped XP.
- Unaudited. Nothing here claims an audit.
