# Mainnet launch: `pump-hooks`

`pump-hooks` hooks coins on the real pump.fun bonding curve (and PumpSwap after graduation). The
engines below (registry, pool engine, smart accounts, token hooks + wrapper) followed the same day.
`hook-router` and `hook-state` stay on devnet. **Not audited.**

## Engines (2026-10-26)

All upgradeable, upgrade authority `5xiBZPdRatPFgvGYY55vTsC1efwwkf1PzyktpVRwsJ77`, built from commit
`4f73878` with `bash scripts/build_programs.sh`, deployed with the keypairs in `.keys/`. Setup:
`packages/demos/scripts/mainnet-init.mts` (only the upgrade authority can initialize either program).

| Program | ID | Deploy | Setup |
| --- | --- | --- | --- |
| hook-registry | `9gKUqwnUa6ZJkCvNT4bnLY7A5czcJFuSV1dpYeosHHs4` | [`3tQ2Juy2…`](https://solscan.io/tx/3tQ2Juy2T6YVVAS4o1B1CXNVw5nymL3PWQ9jBMtCEqsbxfDbonoUqpdjSG7KRERYSYHxSQnxnZH35CaAkfvSqzyi) | [`3Tvwofi…`](https://solscan.io/tx/3TvwofiQNCyStE3rVjBhorFEWPmPaFF1m6UhS9X83Q4MKbzRgj7ykXdGgUW4HtHWTojko7gBbgnit7wqFa2naoMr) |
| hooks-core (pool engine) | `8L8VFMpzP7S3ZfSyr9vu5vDVy93jgfWVfEzY4RTadFzw` | [`66QG84a4…`](https://solscan.io/tx/66QG84a4uKTs19JzqCPPgokC2e1NFLDdfk3HnRpx7B6DEVLTom1cpiy1CgyKoZvWmhYsHTHAQmYZBKb7pqXxxYBV) | [`2ZpmTmKE…`](https://solscan.io/tx/2ZpmTmKEQBCnJCHMvbn8VL2pR5qVoak648VS211C93pgckUKUEqTANTqw4XFwPVcdvdzG2oxvRqhDbdEuB6sx7NT) |
| smart-accounts | `6AEiwvifk4TVJ8g3UoVXqhaK8AqYy43A4DzWCgMmrtQw` | [`4zCEPNiM…`](https://solscan.io/tx/4zCEPNiMiKNSjUjyBCzE6NJXaw36p36sQd7LrrY8sodpmaJC3JK3qSZyvKZyhcPJ56WATqJT2nLmQLhQVS1sMWLZ) | — |
| token-hooks | `FZBBkQQamgfMCAoNk6PAdZi9nYZGMsGw9gDyTMNC3v54` | [`5ZajfQgk…`](https://solscan.io/tx/5ZajfQgkVaSedYLYtspeULEFAqkRP9XB27UbPU1SjDZVX5soLDwPJsKpYAFjQGUEJkg3xCXdA8SgcczESf7tEzJk) | — |
| token-wrapper | `841vHKyD8HXQKsNSp2A2pzNjDHuFDBu9JPt6xsJc7vbh` | [`5q9JFNXq…`](https://solscan.io/tx/5q9JFNXqbtChhfcE8CDPkGmjHGEjdgdCX1nzNk8tCikPrR3TX79KMxa369XvSFyUacjLoc9CJHLsxJUmrss2b9Kz) | — |

Total rent 10.75 SOL.

| | |
| --- | --- |
| Program ID | `91kRNewiEDrKVtUXpahsDXYiURuKQKwhCt2hnaKNfkgD` (`target/deploy/pump_hooks-keypair.json`) |
| Deployer / upgrade authority | `5xiBZPdRatPFgvGYY55vTsC1efwwkf1PzyktpVRwsJ77` (`.keys/mainnet-deployer.json`, git-ignored) |
| Binary | `target/deploy/pump_hooks.so`, 528,696 bytes (`bash scripts/build_program.sh`) |

## Deployed (2026-10-26)

| Step | Transaction |
| --- | --- |
| Deploy (slot 461684222), ProgramData `6TPEShQtpLey3TSGS4kf448RgEakRPM1Cdge9hvKERk4` | [`4EpEWseG…Gn21eD`](https://solscan.io/tx/4EpEWseGks6WwdfceibThQUHS7ivPwZv9Ypr8A3m8jCVLyueBCXF5MrREpWvZ5dXWcAhNWSCpxpuxYoSN4Gn21eD) |
| Initialize (upgrade authority only) | [`59bskkPb…msS1R9`](https://solscan.io/tx/59bskkPbEB6DpSRgXB1HnXHmzaM2dve2YRRpBuHRjs1PqUpcCNZLgVD5yxMgYFZvkcdDt2AZB11gwKFNe3msS1R9) |
| Register launch-shield, volume-guard, volatility-guard, holder-xp | [`21wmgp4s…AjfnyuG7`](https://solscan.io/tx/21wmgp4scmSPizuZYRSFL4L5BUgzEW3PeQU7qF1e6ceutfD2geSjfYwV4jS8hhqHsWgbvs6ECrfkoLEYAjfnyuG7) |
| Register treasury-router, twap, sentient, policy | [`3YM2Ngba…GxYU67bh`](https://solscan.io/tx/3YM2NgbaYQSSHq4tkz3MM9nRTxAfdhkjvp11Ltnq5sPzWNP3auTEhpyAQADaGxYU67bhZzmRi6XiwaB7CUjXmLYM) |
| Register max-wallet, sell-limit, trading-hours | [`2jXW57Cc…C9zma3th`](https://solscan.io/tx/2jXW57CcufAQQ9E4qSwH46BXNWxymzXtCqHKSJBPK5h5LbSkLNJnBkoQjNA6BXgkcM8d2Ue8hCY72vpYC9zma3th) |

Binary sha256 `5118e979de833832310fa817ac07964d99764ab9f40d5092df3b937380a814c6`, rebuilt from commit
`fca58bd` with `bash scripts/build_program.sh`. Mainnet rent came out lower than the local estimate
below: the deploy cost 2.69 SOL in total.

## Cost (measured with `solana rent` on a local validator)

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
node packages/cli/dist/bin.js status --url "$RPC"
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
