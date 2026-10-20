# PROGRESS

## STATUS

The build order is done locally through the SDK, CLI, indexer and web app:

- Hook ABI, runtime, registry, Pump adapter and router;
- existing-token integration (curve and PumpSwap) and the first-party hooks;
- Policy VM, SDK, CLI and indexer;
- the marketplace, Composer, simulator, market page and transaction explorer.

Routed trades on existing Pump tokens work end to end against the real devnet Pump binaries
in LiteSVM.

The devnet deployment is blocked because the deployer has no devnet SOL. The web app reads live
devnet state, such as Pump token detection, and shows "not deployed" wherever it needs the
program.

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
  - `dump_programs.sh` dumps the Pump binaries;
  - `build_program.sh` builds the program;
  - `shot.mjs` takes visual-check screenshots.
- `packages/sdk` (`PumpHooksClient`):
  - `detect` for existing tokens;
  - `createMarket`, `installHook` and `createPolicy`;
  - `buy` / `sell` routing with the Pump ALT;
  - `previewStack`: create + install + routed buy in one simulated transaction (≤ 1232 bytes
    with the ALT);
  - `simulate` and `getExecutionTrace`;
  - config schemas, `CONFIG_PRESETS` and `validateConfig` (on-chain parity test);
  - the Composer compiler (`compilePolicy` / `describePolicy`).
- `packages/cli` (`pump-hooks`): status, inspect, hooks, market, install/uninstall, policy, quote,
  simulate, trade and tx. It never reads the Solana CLI config.
- `services/indexer`:
  - resumable and idempotent: an append-only JSONL store whose cursor is the last record, so a
    torn write is dropped and fetched again;
  - finalized commitment by default;
  - per-market and per-module stats;
  - a read-only HTTP API.
- `hooks/*/hook.json`: marketplace manifests. A test checks them against the SDK module table.
- `apps/web` (Next.js 16):
  - dithered WebGL stage with the hook-shaped colour lens.
  - Pages:
    - `/` home;
    - `/hook`, the primary flow: paste CA, detect, categories, stack, configure, simulate,
      activate;
    - `/hooks` marketplace and `/hooks/[slug]`, with callbacks, capabilities, config schema, the
      on-chain registry and limitations;
    - `/build` Composer;
    - `/market/[address]`: stack, policies, recent transactions from chain, and a routed buy
      with simulation;
    - `/tx/[signature]`: an execution trace decoded from logs, which flags unrouted transactions
      as bypassing the router;
    - `/simulate`;
    - `/docs`.
  - Executions are shown only when an indexer is configured.
  - Every audit field reads "unaudited".
- `README.md`, `SECURITY.md`, `LICENSE` (Apache-2.0).

## TESTS

107 passing: `cargo test --workspace` (77) + `pnpm -r test` (30). `next build` passes for all
18 routes.

| Suite | Tests |
| --- | ---: |
| hook-types | 13 |
| policy-vm | 20 |
| hook-runtime | 22 |
| pump-adapter (unit + equivalence vs pump-rust-client) | 6 |
| program unit | 1 |
| integration: PumpSwap route (LiteSVM, real `pump_amm`) | 11 |
| integration: curve route (LiteSVM, real `pump`) | 4 |
| sdk (unit, PumpSwap e2e, curve e2e, previewStack, presets) | 22 |
| cli (parsing, e2e on a cloned devnet token) | 3 |
| indexer (real executions: decode, pagination/gaps, torn write, HTTP, backfill) | 5 |

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

## BLOCKERS

- **Devnet SOL.** The deployer `41L6nJ9wrsCK3RuXzvZafNcTWuSEBouvtWqanVdAKi6b` holds 0 SOL. The
  public faucet is rate-limited and the devnet-pow faucets are empty. About 4 devnet SOL covers
  deploying the 507 KB program, initializing, and the milestone trades. The key lives in
  gitignored `.keys/`.

## NEXT

1. Fund the deployer, then deploy to devnet. Initialize, register modules, and create a market on
   the existing devnet token `44yV5xk8…`. Install Volume Guard, TWAP, Holder XP and SENTIENT, and
   send a routed `buy_v2` from the web app. Decode the transaction in `/tx` and in the indexer.
2. Strategy vault program (TWAP execution, treasury intents), behind explicit deposits.
3. CU optimisations listed in `docs/benchmarks.md`.
4. SENTIENT reference engine (off-chain signer service with bounded signals).
5. Pump token creation as an alternate entrance (last; links to Pump today).

## KNOWN RISKS

- Hooks bind only routed trades. Direct Pump trades bypass them, and a test pins this.
- Pump program upgrades can change layouts or account sets. Mitigations: the adapter is verified
  against the official client, tests run against the deployed devnet binaries, and router reads
  fail closed on unexpected layouts.
- Pre-trade sizes are constant-product estimates. After-hooks use observed deltas, and Volume
  Guard re-checks after the trade.
- Holder XP is anti-wash rather than sybil-proof. Many wallets can each earn capped XP.
- A synthetic migration test fills the whole curve. Partial-fill behaviour on the final buy is
  covered only by Pump's own logic.
- SENTIENT flags default to both restrictions. Markets must set the engine signer, and there is no
  default signer.
- Pump picks the buyback fee recipient at random from its configured set, and the SDK picks
  uniformly. That recipient must already hold an ATA for the quote mint, or the trade fails
  closed.
- The indexer trusts its RPC. Run it against a provider you control for anything beyond display.
- Unaudited. Nothing here claims an audit.
