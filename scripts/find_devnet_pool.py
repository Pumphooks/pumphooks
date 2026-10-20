#!/usr/bin/env python3
"""List existing devnet PumpSwap pools quoted in wSOL, ranked by quote-vault liquidity.

Read-only. Usage: scripts/find_devnet_pool.py [rpc_url] [limit]
"""
import base64
import json
import struct
import sys
import time
import urllib.error
import urllib.request

RPC = sys.argv[1] if len(sys.argv) > 1 else "https://api.devnet.solana.com"
LIMIT = int(sys.argv[2]) if len(sys.argv) > 2 else 15
AMM = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA"
WSOL = "So11111111111111111111111111111111111111112"
POOL_DISC_B58 = "hQrXeCntzbV"
ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def b58(b: bytes) -> str:
    n = int.from_bytes(b, "big")
    s = ""
    while n:
        n, r = divmod(n, 58)
        s = ALPHABET[r] + s
    return "1" * (len(b) - len(b.lstrip(b"\0"))) + s


def rpc(method, params):
    req = urllib.request.Request(
        RPC,
        json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(),
        {"content-type": "application/json"},
    )
    for attempt in range(8):
        try:
            out = json.load(urllib.request.urlopen(req, timeout=120))
            break
        except urllib.error.HTTPError as e:
            if e.code != 429 or attempt == 7:
                raise
            time.sleep(2 * (attempt + 1))
    if "error" in out:
        raise SystemExit(out["error"])
    return out["result"]


pools = rpc(
    "getProgramAccounts",
    [
        AMM,
        {
            "encoding": "base64",
            "filters": [{"memcmp": {"offset": 0, "bytes": POOL_DISC_B58}}, {"memcmp": {"offset": 75, "bytes": WSOL}}],
        },
    ],
)
rows = []
for p in pools:
    d = base64.b64decode(p["account"]["data"][0])
    is_cashback = len(d) > 244 and d[244] != 0
    vqr = struct.unpack_from("<q", d, 245)[0] if len(d) >= 261 else 0  # low 8 bytes of i128 is enough for ranking
    rows.append(
        {
            "pool": p["pubkey"],
            "index": struct.unpack_from("<H", d, 9)[0],
            "creator": b58(d[11:43]),
            "base_mint": b58(d[43:75]),
            "base_vault": b58(d[139:171]),
            "quote_vault": b58(d[171:203]),
            "cashback": is_cashback,
            "mayhem": len(d) > 243 and d[243] != 0,
            "len": len(d),
            "vqr": vqr,
        }
    )
rows = [r for r in rows if not r["cashback"] and r["index"] == 0]

balances = {}
vaults = [r["quote_vault"] for r in rows]
for i in range(0, len(vaults), 100):
    res = rpc("getMultipleAccounts", [vaults[i : i + 100], {"encoding": "base64", "dataSlice": {"offset": 64, "length": 8}}])
    for k, a in zip(vaults[i : i + 100], res["value"]):
        balances[k] = struct.unpack("<Q", base64.b64decode(a["data"][0]))[0] if a else 0
for r in rows:
    r["quote_vault_lamports"] = balances.get(r["quote_vault"], 0)
rows.sort(key=lambda r: r["quote_vault_lamports"], reverse=True)
print(json.dumps({"total_wsol_pools": len(pools), "candidates": rows[:LIMIT]}, indent=1))
