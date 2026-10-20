#!/usr/bin/env bash
# Dump the deployed Pump programs (default: devnet) for LiteSVM / local-validator tests.
#   scripts/dump_programs.sh [rpc_url]
set -euo pipefail
RPC=${1:-https://api.devnet.solana.com}
OUT="$(cd "$(dirname "$0")/.." && pwd)/tests/fixtures/programs"
mkdir -p "$OUT"
for p in pump_amm:pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA pump:6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P pump_fees:pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ; do
  solana program dump "${p##*:}" "$OUT/${p%%:*}.so" --url "$RPC"
done
shasum -a 256 "$OUT"/*.so
