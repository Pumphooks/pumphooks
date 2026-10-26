#!/usr/bin/env bash
# Dump the deployed third-party programs the LiteSVM tests run against (read-only, public RPC).
#   scripts/dump_programs.sh [devnet_rpc] [mainnet_rpc]
set -euo pipefail
DEVNET=${1:-https://api.devnet.solana.com}
MAINNET=${2:-https://api.mainnet-beta.solana.com}
OUT="$(cd "$(dirname "$0")/.." && pwd)/tests/fixtures/programs"
mkdir -p "$OUT"
dump() { solana program dump "$1" "$OUT/$2.so" --url "$3"; }
dump pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA pump_amm "$DEVNET"
dump 6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P pump "$DEVNET"
dump pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ pump_fees "$DEVNET"
dump pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA pump_amm_mainnet "$MAINNET"
dump 6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P pump_mainnet "$MAINNET"
dump pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ pump_fees_mainnet "$MAINNET"
dump DRaycpLY18LhpbydsBWbVJtxpNv9oXPgjRSfpF2bWpYb raydium_cpmm_devnet "$DEVNET"
dump CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C raydium_cpmm_mainnet "$MAINNET"
dump cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG meteora_damm_v2_devnet "$DEVNET"
dump cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG meteora_damm_v2_mainnet "$MAINNET"
dump CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK raydium_clmm_mainnet "$MAINNET"
dump whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc orca_whirlpool_mainnet "$MAINNET"
dump LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo meteora_dlmm_mainnet "$MAINNET"
dump MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr spl_memo "$MAINNET"
shasum -a 256 "$OUT"/*.so
