#!/usr/bin/env bash
# Snapshot the accounts a routed bonding-curve (v3) trade touches for one existing, un-migrated
# Pump coin.
#
#   scripts/fetch_curve_fixtures.sh <bonding_curve> [rpc_url]
#
# Read-only. Output: tests/fixtures/curve-<bonding_curve>/<pubkey>.json + manifest.json
set -euo pipefail

CURVE=${1:?bonding curve address}
RPC=${2:-https://api.devnet.solana.com}
PUMP=6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P
FEES=pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ
WSOL=So11111111111111111111111111111111111111112
ATA=ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL

DIR="$(cd "$(dirname "$0")/.." && pwd)/tests/fixtures/curve-$CURVE"
mkdir -p "$DIR"
pda() { solana find-program-derived-address "$@" | head -1 | awk '{print $1}'; }
rpc() { curl -s "$RPC" -X POST -H 'content-type: application/json' -d "$1"; }
dump() {
  local key=$1 label=$2
  for i in 1 2 3 4 5; do
    if solana account "$key" --url "$RPC" --output json-compact > "$DIR/$key.json" 2>/dev/null; then
      echo "  $label $key"; echo "$key $label" >> "$DIR/.manifest"; return 0
    fi
    sleep $((i * 2))
  done
  rm -f "$DIR/$key.json"; echo "  (missing) $label $key"
}

rm -f "$DIR/.manifest"
dump "$CURVE" bonding_curve
# The curve's base token account reveals the mint.
read -r CURVE_ATA MINT TOKEN_PROGRAM <<<"$(for prog in TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA; do
  rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getTokenAccountsByOwner\",\"params\":[\"$CURVE\",{\"programId\":\"$prog\"},{\"encoding\":\"jsonParsed\"}]}" |
    python3 -c "import json,sys; v=json.load(sys.stdin)['result']['value']; [print(a['pubkey'], a['account']['data']['parsed']['info']['mint'], a['account']['owner']) for a in v[:1]]"
done | head -1)"
[ -n "${MINT:-}" ] || { echo "could not resolve mint"; exit 1; }
[ "$(pda "$PUMP" string:bonding-curve pubkey:$MINT)" = "$CURVE" ] || { echo "curve is not the canonical PDA of $MINT"; exit 1; }
dump "$MINT" base_mint
dump "$WSOL" quote_mint
dump "$CURVE_ATA" associated_base_bonding_curve
GLOBAL=$(pda "$PUMP" string:global)
dump "$GLOBAL" pump_global
dump "$(pda "$FEES" string:fee_config pubkey:$PUMP)" pump_fee_config
RECIPIENT=$(python3 - "$DIR/$GLOBAL.json" <<'EOF'
import base64, json, sys
A = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
def b58(b):
    n = int.from_bytes(b, "big"); s = ""
    while n: n, r = divmod(n, 58); s = A[r] + s
    return "1" * (len(b) - len(b.lstrip(b"\0"))) + s
d = base64.b64decode(json.load(open(sys.argv[1]))["account"]["data"][0])
print(b58(d[741:773]))
EOF
)
dump "$RECIPIENT" buyback_fee_recipient

python3 - "$DIR" "$CURVE" "$MINT" "$TOKEN_PROGRAM" "$RECIPIENT" "$RPC" <<'EOF'
import json, sys
d, curve, mint, tp, rec, rpc = sys.argv[1:]
rows = [l.split() for l in open(f"{d}/.manifest")]
json.dump({"bonding_curve": curve, "mint": mint, "base_token_program": tp, "buyback_fee_recipient": rec,
           "source_rpc": rpc.split("?")[0], "accounts": {label: key for key, label in rows}}, open(f"{d}/manifest.json", "w"), indent=1)
EOF
rm -f "$DIR/.manifest"
echo "wrote $DIR"
