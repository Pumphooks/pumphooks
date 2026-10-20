# Benchmarks

Measured by `tests/integration/tests/amm_route.rs::compute_benchmarks` in LiteSVM 0.16 with the
`pump_amm` binary dumped from devnet and the cloned state of existing devnet pool
`9Nhddo1PcCBms54wcLYbxoRcAJbpmSvHWMcbdMf9tdmZ` (mint `44yV5xk8C1QVRGz82Q8KLcpft5gqbNd3SG8Lt1NrhLo9`).
Trade: `buy_exact_quote_in` 0.1 SOL. Program built with `cargo build-sbf` (platform-tools v1.53).

"first" is a wallet's first trade on that market (creates `UserMarketState`, rent paid by the
wallet); "steady" is a later trade. Numbers are compute units for the whole instruction including
the Pump CPI. Re-run with:

```bash
cargo build-sbf --manifest-path programs/pump-hooks/Cargo.toml --sbf-out-dir target/deploy
cargo test -p pump-hooks-integration compute_benchmarks -- --nocapture   # writes target/benchmarks.json
```

| Case | CU first | CU steady | Accounts | Ix data |
| --- | ---: | ---: | ---: | ---: |
| Direct PumpSwap `buy_exact_quote_in_v2` | 59,477 | 56,567 | 17 | 24 B |
| Routed, 0 hooks | 80,436 | 77,475 | 22 | 25 B |
| Routed, 1 hook (Volume Guard) | 81,395 | 78,416 | 22 | 25 B |
| Routed, 2 hooks (+ Holder XP) | 83,542 | 80,571 | 22 | 25 B |
| Routed, 4 hooks (+ SENTIENT, Policy with 0 policies) | 91,168 | 88,189 | 22 | 25 B |

The router's fixed cost is about 21k CU over a direct trade at steady state. Each hook adds about
1–4k CU. Policies add 4 accounts at most, one per active policy.

Where the fixed cost goes (not yet profiled in detail): Anchor account validation and
`init_if_needed` on `UserMarketState`, PDA re-derivation for the user state, two snapshots of five
token accounts plus the pool, building and emitting three events (`HookExecutionEvent`,
`RoutedSwapEvent`, `PolicyExecutionEvent`s), and heap copies of the 2.7 KB working set.

Planned reductions:

- precompute bumps;
- cache `find_program_address` results for policies;
- skip `HookExecutionEvent` when the trace is empty;
- move snapshot parsing to fixed offsets only.

A full routed trade with 4 hooks stays under 100k CU, well inside the 200k default and the 1.4M
cap. That leaves room for the SDK's compute budget, ATA creation and wSOL wrap/unwrap in the same
transaction.

Transaction size: the routed instruction adds 5 accounts (user state, global, market, our program
id, system program; the Pump block is the same 17 accounts). With the Pump devnet/mainnet ALT, a
v0 transaction stays well under the 1232-byte limit.
