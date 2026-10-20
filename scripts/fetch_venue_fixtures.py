#!/usr/bin/env python3
"""Clone the on-chain accounts the venue tests need into tests/fixtures/venues/<network>/.

Read-only JSON-RPC against public endpoints; no keys involved.

  raydium-cpmm  the AmmConfig (index 0) and the create-pool fee receiver, so tests can create pools
                with the real `initialize` instruction.
  damm-v2       one existing pool (SPL Token on both sides, enabled, funded, activated long ago so
                the anti-sniper rate limiter cannot apply), plus its vaults and mints.

Usage: scripts/fetch_venue_fixtures.py [devnet|mainnet ...]   (default: devnet mainnet)
"""

import base64
import hashlib
import json
import pathlib
import sys
import urllib.request

RPC = {"devnet": "https://api.devnet.solana.com", "mainnet": "https://api.mainnet-beta.solana.com"}
CPMM = {
    "devnet": ("DRaycpLY18LhpbydsBWbVJtxpNv9oXPgjRSfpF2bWpYb", "3oE58BKVt8KuYkGxx8zBojugnymWmBiyafWgMrnb6eYy"),
    "mainnet": ("CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C", "DNXgeM9EiiaAbaWvwjHj9fQQLAX5ZsfHyvmYUNRAdNC8"),
}
DAMM_V2 = "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG"
SPL_TOKEN = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
OUT = pathlib.Path(__file__).resolve().parent.parent / "tests/fixtures/venues"

B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def b58e(b: bytes) -> str:
    n, s = int.from_bytes(b, "big"), ""
    while n:
        n, r = divmod(n, 58)
        s = B58[r] + s
    return "1" * (len(b) - len(b.lstrip(b"\0"))) + s


def b58d(s: str) -> bytes:
    n = 0
    for c in s:
        n = n * 58 + B58.index(c)
    b = n.to_bytes((n.bit_length() + 7) // 8, "big")
    return b"\0" * (len(s) - len(s.lstrip("1"))) + b


def rpc(net, method, params):
    req = urllib.request.Request(
        RPC[net],
        data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(),
        headers={"Content-Type": "application/json"},
    )
    r = json.load(urllib.request.urlopen(req, timeout=120))
    if "error" in r:
        raise RuntimeError(f"{method}: {r['error']}")
    return r["result"]


def accounts(net, keys):
    out = []
    for i in range(0, len(keys), 100):
        out += rpc(net, "getMultipleAccounts", [keys[i : i + 100], {"encoding": "base64"}])["value"]
    return out


def save(dirpath: pathlib.Path, key: str, acct: dict):
    dirpath.mkdir(parents=True, exist_ok=True)
    doc = {
        "pubkey": key,
        "account": {
            "lamports": acct["lamports"],
            "data": acct["data"],
            "owner": acct["owner"],
            "executable": acct["executable"],
            "rentEpoch": 0,
            "space": len(base64.b64decode(acct["data"][0])),
        },
    }
    (dirpath / f"{key}.json").write_text(json.dumps(doc, indent=1) + "\n")


def pda(program: str, *seeds: bytes) -> str:
    """`find_program_address`: the first bump (from 255 down) whose hash is off the ed25519 curve."""
    prog = b58d(program)
    for bump in range(255, -1, -1):
        h = hashlib.sha256(b"".join(seeds) + bytes([bump]) + prog + b"ProgramDerivedAddress").digest()
        if not on_curve(h):
            return b58e(h)
    raise RuntimeError("no bump")


P = 2**255 - 19
D = (-121665 * pow(121666, P - 2, P)) % P


def on_curve(b: bytes) -> bool:
    y = int.from_bytes(b, "little") & ((1 << 255) - 1)
    if y >= P:
        return False
    u, v = (y * y - 1) % P, (D * y * y + 1) % P
    x2 = u * pow(v, P - 2, P) % P
    if x2 == 0:
        return True
    return pow(x2, (P - 1) // 2, P) == 1


def cpmm(net):
    program, fee_receiver = CPMM[net]
    config = pda(program, b"amm_config", (0).to_bytes(2, "little"))
    d = OUT / net / "raydium-cpmm"
    got = accounts(net, [config, fee_receiver])
    for k, a in zip([config, fee_receiver], got):
        assert a, f"{k} missing on {net}"
        save(d, k, a)
    (d / "manifest.json").write_text(
        json.dumps({"program": program, "amm_config": config, "create_pool_fee": fee_receiver, "source_rpc": RPC[net]}, indent=1) + "\n"
    )
    print(f"{net} cpmm: config {config}, fee receiver {fee_receiver}")


def damm(net, scan=3000):
    disc = hashlib.sha256(b"account:Pool").digest()[:8]
    pools = rpc(
        net,
        "getProgramAccounts",
        [DAMM_V2, {"encoding": "base64", "dataSlice": {"offset": 0, "length": 0}, "filters": [{"memcmp": {"offset": 0, "bytes": b58e(disc)}}]}],
    )
    keys = [p["pubkey"] for p in pools][:scan]
    best = None
    for k, a in zip(keys, accounts(net, keys)):
        if not a:
            continue
        b = base64.b64decode(a["data"][0])
        if len(b) != 1112:
            continue
        liquidity = int.from_bytes(b[360:376], "little")
        status, flag_a, flag_b = b[481], b[482], b[483]
        if status != 0 or flag_a != 0 or flag_b != 0 or liquidity == 0:
            continue
        mint_a, mint_b = b58e(b[168:200]), b58e(b[200:232])
        vault_a, vault_b = b58e(b[232:264]), b58e(b[264:296])
        best = best or []
        best.append((k, a, mint_a, mint_b, vault_a, vault_b))
        if len(best) >= 40:
            break
    assert best, f"no candidate DAMM v2 pool on {net}"
    for k, a, mint_a, mint_b, vault_a, vault_b in best:
        extra = accounts(net, [vault_a, vault_b, mint_a, mint_b])
        if not all(extra) or any(e["owner"] != SPL_TOKEN for e in extra):
            continue
        bal = [int.from_bytes(base64.b64decode(e["data"][0])[64:72], "little") for e in extra[:2]]
        if min(bal) < 10_000_000:
            continue
        d = OUT / net / "damm-v2"
        if d.exists():
            for f in d.glob("*.json"):
                f.unlink()
        save(d, k, a)
        for key, e in zip([vault_a, vault_b, mint_a, mint_b], extra):
            save(d, key, e)
        (d / "manifest.json").write_text(
            json.dumps(
                {"program": DAMM_V2, "pool": k, "token_a_mint": mint_a, "token_b_mint": mint_b, "token_a_vault": vault_a, "token_b_vault": vault_b, "vault_balances": bal, "source_rpc": RPC[net]},
                indent=1,
            )
            + "\n"
        )
        print(f"{net} damm-v2: pool {k}, vault balances {bal}")
        return
    raise RuntimeError(f"no funded SPL/SPL DAMM v2 pool among candidates on {net}")


if __name__ == "__main__":
    nets = sys.argv[1:] or ["devnet", "mainnet"]
    for net in nets:
        cpmm(net)
        try:
            damm(net)
        except Exception as e:  # mainnet getProgramAccounts is often refused by public RPC
            print(f"{net} damm-v2: skipped ({e})")
