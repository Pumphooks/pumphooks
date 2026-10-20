#!/usr/bin/env bash
# Snapshot the on-chain accounts a routed PumpSwap trade touches for one existing pool, in the
# `solana account --output json` format (usable by LiteSVM tests and `solana-test-validator --account`).
#
#   scripts/fetch_fixtures.sh <pool> [rpc_url]
#
# Read-only. Output: tests/fixtures/<pool>/<pubkey>.json + manifest.json
set -euo pipefail

POOL=${1:?pool address}
RPC=${2:-https://api.devnet.solana.com}
PUMP=6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P
AMM=pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA
FEES=pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ
WSOL=So11111111111111111111111111111111111111112
TOKEN=TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA
ATA=ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL

DIR="$(cd "$(dirname "$0")/.." && pwd)/tests/fixtures/$POOL"
mkdir -p "$DIR"

pda() { solana find-program-derived-address "$@" | head -1 | awk '{print $1}'; }
dump() {
  local key=$1 label=$2
  for i in 1 2 3 4 5; do
    if solana account "$key" --url "$RPC" --output json-compact > "$DIR/$key.json" 2>/dev/null; then
      echo "  $label $key"
      echo "$key $label" >> "$DIR/.manifest"
      return 0
    fi
    sleep $((i * 2))
  done
  rm -f "$DIR/$key.json"
  echo "  (missing) $label $key"
}

rm -f "$DIR/.manifest"
dump "$POOL" pool
read -r MINT BASE_VAULT QUOTE_VAULT <<<"$(python3 - "$DIR/$POOL.json" <<'EOF'
import base64, json, sys
A = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
def b58(b):
    n = int.from_bytes(b, "big"); s = ""
    while n: n, r = divmod(n, 58); s = A[r] + s
    return "1" * (len(b) - len(b.lstrip(b"\0"))) + s
d = base64.b64decode(json.load(open(sys.argv[1]))["account"]["data"][0])
print(b58(d[43:75]), b58(d[139:171]), b58(d[171:203]))
EOF
)"
dump "$MINT" base_mint
dump "$WSOL" quote_mint
dump "$BASE_VAULT" pool_base_token_account
dump "$QUOTE_VAULT" pool_quote_token_account
GC=$(pda "$AMM" string:global_config)
dump "$GC" amm_global_config
dump "$(pda "$FEES" string:fee_config pubkey:$AMM)" amm_fee_config
dump "$(pda "$PUMP" string:bonding-curve pubkey:$MINT)" bonding_curve
dump "$(pda "$PUMP" string:global)" pump_global
dump "$(pda "$FEES" string:fee_config pubkey:$PUMP)" pump_fee_config

RECIPIENT=$(python3 - "$DIR/$GC.json" <<'EOF'
import base64, json, sys
A = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
def b58(b):
    n = int.from_bytes(b, "big"); s = ""
    while n: n, r = divmod(n, 58); s = A[r] + s
    return "1" * (len(b) - len(b.lstrip(b"\0"))) + s
d = base64.b64decode(json.load(open(sys.argv[1]))["account"]["data"][0])
print(b58(d[643:675]))
EOF
)
dump "$(pda "$ATA" pubkey:$RECIPIENT pubkey:$TOKEN pubkey:$WSOL)" buyback_fee_recipient_wsol_ata

python3 - "$DIR" "$POOL" "$MINT" "$RECIPIENT" "$RPC" <<'EOF'
import json, sys
d, pool, mint, rec, rpc = sys.argv[1:]
rows = [l.split() for l in open(f"{d}/.manifest")]
json.dump({"pool": pool, "mint": mint, "buyback_fee_recipient": rec, "source_rpc": rpc.split("?")[0],
           "accounts": {label: key for key, label in rows}}, open(f"{d}/manifest.json", "w"), indent=1)
EOF
rm -f "$DIR/.manifest"
echo "wrote $DIR"
