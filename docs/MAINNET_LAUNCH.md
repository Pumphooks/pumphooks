# Mainnet launch: `pump-hooks`

Only the `pump-hooks` program goes to mainnet. It hooks coins on the real pump.fun bonding curve
(and PumpSwap after graduation). Every other program stays on localnet/devnet. **Not audited.**

| | |
| --- | --- |
| Program ID | `91kRNewiEDrKVtUXpahsDXYiURuKQKwhCt2hnaKNfkgD` (`target/deploy/pump_hooks-keypair.json`) |
| Deployer / upgrade authority | `5xiBZPdRatPFgvGYY55vTsC1efwwkf1PzyktpVRwsJ77` (`.keys/mainnet-deployer.json`, git-ignored) |
| Binary | `target/deploy/pump_hooks.so`, 528,696 bytes (`bash scripts/build_program.sh`) |

## Cost (measured with `solana rent`)

| Item | SOL | |
| --- | --- | --- |
| ProgramData account (528,696 + 45 bytes) | 3.68092824 | locked while the program exists |
| Program account (36 bytes) | 0.00114144 | locked |
| Deploy buffer (528,696 + 37 bytes) | 3.68087256 | refunded when the deploy completes |
| Initialize + register 11 modules | ~0.017 | measured on a local run |
| Write transactions (~530) + priority fees | < 0.05 | |
| **Peak balance needed** | **~7.43** | |
| **Net spend** | **~3.75** | |

## Steps

The RPC URL lives only in an environment variable. Never pass `-u mainnet-beta` defaults from a
CLI config.

```sh
export RPC="https://mainnet.helius-rpc.com/?api-key=..."   # not written to any file

# 1. Deploy. If it stops midway, rerun the same command with --buffer <printed buffer keypair>,
#    or reclaim the buffer with `solana program close --buffers`.
solana program deploy target/deploy/pump_hooks.so \
  --program-id target/deploy/pump_hooks-keypair.json \
  --keypair .keys/mainnet-deployer.json --fee-payer .keys/mainnet-deployer.json \
  --upgrade-authority .keys/mainnet-deployer.json \
  --url "$RPC" --with-compute-unit-price 200000 --max-sign-attempts 50 --use-rpc

# 2. Initialize and register the modules. Only the upgrade authority can initialize.
pnpm --filter @pump-hooks/cli exec tsx scripts/bootstrap-pump-hooks.ts \
  --url "$RPC" --keypair ../../.keys/mainnet-deployer.json --dry-run
pnpm --filter @pump-hooks/cli exec tsx scripts/bootstrap-pump-hooks.ts \
  --url "$RPC" --keypair ../../.keys/mainnet-deployer.json

# 3. Check
solana program show 91kRNewiEDrKVtUXpahsDXYiURuKQKwhCt2hnaKNfkgD --url "$RPC"
pnpm --filter @pump-hooks/cli exec pump-hooks status --url "$RPC"
```

4. In Vercel project settings set `MAINNET_RPC_URL` to the Helius URL (server-only; no
   `NEXT_PUBLIC_` prefix) and redeploy. The site sends browser RPC through `/api/rpc`, which only
   forwards an allowlist of read/simulate/send methods. Without the variable it falls back to the
   public RPC, which is rate-limited.

The site turns mainnet writes on by itself once it sees the program live and unpaused.
`NEXT_PUBLIC_ALLOW_MAINNET_WRITES=0` is an off switch.

## Verified before launch

On a `solana-test-validator` running pump.fun's mainnet `pump`, `pump_amm` and `pump_fees`
binaries with the live global, fee config and fee recipients cloned, through the site and its RPC
proxy: bootstrap 11/11 (idempotent), wallet connect, Max Wallet rule simulated and activated in one
transaction, a 15 SOL buy refused by the rule (10001) before the pump.fun CPI, a 0.1 SOL routed buy
confirmed (681,680.62 tokens, ~106.9k CU), and the trace page decoding the events. Routed buys set
a 5% slippage floor from the simulation; a floor above the curve's output reverts.
