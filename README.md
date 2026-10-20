# Pump Hooks

**The programmable layer for Pump.** Give any existing Pump market new behavior: compose
execution, risk, rewards, treasury and autonomous logic as a hook stack that runs around every
trade routed through Pump Hooks.

```
ANY PUMP TOKEN ──▶ HOOK STACK ──────────────────────────────▶ PUMP / PUMPSWAP
                   Launch Shield · Volume Guard · Volatility
                   Holder XP · Treasury · TWAP · SENTIENT · Policies
```

Pump Hooks is not a launchpad. The primary object is a **hooked market**: an existing Pump token
(on its bonding curve or migrated to PumpSwap), plus a hook stack and policies owned by a market
authority.

## The routing boundary

Pump and PumpSwap do not call external programs before or after swaps. Pump Hooks therefore runs
hooks **at its own router boundary**. In one atomic transaction:

1. The wallet calls `route_curve` (bonding curve) or `route_amm` (PumpSwap) with a buy or sell
   mode.
2. The before-hooks run.
3. The router makes the CPI into Pump or PumpSwap.
4. It measures the executed amounts.
5. The after-hooks run.
6. Events are emitted.

**Trades sent to Pump or PumpSwap directly bypass the hook stack.** Guards protect routed flow and
rewards accrue to routed flow; neither can restrict the venue itself. A test pins this
behavior.

## Repository

| Path | What |
| --- | --- |
| `crates/hook-types` | Frozen hook ABI: callbacks, context, decision, capabilities, slots, signals |
| `crates/hook-runtime` | Phase executor and first-party modules, shared by program and simulator |
| `crates/policy-vm` | Bounded WHEN/IF/THEN interpreter (8 conditions, 4 actions, no loops, no CPI) |
| `crates/pump-adapter` | Pump / PumpSwap layouts, PDAs, trade builders (byte-checked against `pump-rust-client`) |
| `programs/pump-hooks` | Anchor program: registry, markets on existing tokens, hook stack, policies, router, SENTIENT |
| `packages/sdk` | `PumpHooksClient`: detect, hook, route, simulate, decode |
| `packages/cli` | `pump-hooks` CLI |
| `services/indexer` | Resumable, idempotent event indexer with a read-only HTTP API |
| `apps/web` | Web app: `/hook`, `/hooks`, `/build`, `/market/[address]`, `/tx/[signature]`, `/simulate`, `/docs` |
| `hooks/*/hook.json` | Marketplace manifests for every module |
| `docs/` | Research, architecture, threat model, hooks spec, policy VM, benchmarks |

## Status

- Not audited.
- Not deployed on mainnet.
- Devnet deployment is pending funding of the deployer key.

See [PROGRESS.md](PROGRESS.md) and [SECURITY.md](SECURITY.md).

## Build and test

Requirements: Rust with the Solana/Agave toolchain, Node 20 and pnpm 10.

```sh
pnpm install
scripts/build_program.sh            # builds target/deploy/pump_hooks.so
cargo test --workspace              # ABI, runtime, policy VM, adapter, LiteSVM integration
pnpm -r test                        # SDK, CLI, indexer (LiteSVM against real Pump binaries)
pnpm --filter @pump-hooks/web dev   # http://localhost:3000
```

The integration suites run the real devnet `pump_amm` / `pump` binaries in LiteSVM against cloned
state of existing devnet tokens (`tests/fixtures`).

## Using it

```ts
import { Connection, PublicKey } from "@solana/web3.js";
import { PumpHooksClient, PUMP_ALT, ModuleId, CONFIG_PRESETS } from "@pump-hooks/sdk";

const client = new PumpHooksClient(new Connection("https://api.devnet.solana.com"), { lookupTables: [PUMP_ALT.devnet] });
const token = await client.detect(new PublicKey(MINT)); // curve or PumpSwap, Token-2022, reserves
const { market, ix } = await client.createMarket(token.mint, authority);
const install = client.installHook({ market, authority, module: ModuleId.VolumeGuard, config: CONFIG_PRESETS[ModuleId.VolumeGuard] });
```

```sh
pump-hooks inspect <mint> --url https://api.devnet.solana.com
pump-hooks simulate <market> --buy 0.1
pump-hooks tx <signature>
```

The CLI and indexer never read your Solana CLI config. Pass `--url` and `--keypair` (CLI) or
`RPC_URL` (indexer) explicitly.

### Indexer

```sh
RPC_URL=https://api.devnet.solana.com DATA_DIR=./data PORT=8787 pnpm --filter @pump-hooks/indexer start
```

Set `NEXT_PUBLIC_DEVNET_INDEXER_URL` for the web app to show per-hook execution counts.

## License

Apache-2.0. Pump Hooks is not affiliated with Pump.fun.
