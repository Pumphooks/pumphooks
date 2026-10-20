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


CLMM = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK"
WHIRLPOOL = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc"
DLMM = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo"
# Liquid mainnet SOL/USDC pools (owner and layout checked below).
CONCENTRATED = {
    "raydium-clmm": (CLMM, "3ucNos4NbumPLZNWztqGHNFFgkHeRMBQAVemeeomsUxv"),
    "whirlpool": (WHIRLPOOL, "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE"),
    "dlmm": (DLMM, "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6"),
}


def i32(b, o):
    return int.from_bytes(b[o : o + 4], "little", signed=True)


def u16(b, o):
    return int.from_bytes(b[o : o + 2], "little")


def key_at(b, o):
    return b58e(b[o : o + 32])


def save_all(d, keys, required):
    """Saves every existing account in `keys`; `required` must all exist. Returns the saved keys."""
    got = accounts("mainnet", keys)
    saved = []
    for k, a in zip(keys, got):
        if a is None:
            assert k not in required, f"{k} missing"
            continue
        save(d, k, a)
        saved.append(k)
    return saved


def concentrated(name):
    program, pool = CONCENTRATED[name]
    d = OUT / "mainnet" / name
    if d.exists():
        for f in d.glob("*.json"):
            f.unlink()
    (a,) = accounts("mainnet", [pool])
    assert a and a["owner"] == program, f"{pool} is not a {name} pool"
    b = base64.b64decode(a["data"][0])
    save(d, pool, a)
    m = {"program": program, "pool": pool, "source_rpc": RPC["mainnet"]}
    if name == "raydium-clmm":
        # PoolState: amm_config 9, mints 73/105, vaults 137/169, observation 201, tick_spacing 235,
        # tick_current 269 (8-byte discriminator included).
        spacing, tick = u16(b, 235), i32(b, 269)
        size = spacing * 60
        start = (tick // size) * size
        arrays = [start + i * size for i in range(-3, 4)]
        array_keys = [pda(program, b"tick_array", b58d(pool), s.to_bytes(4, "big", signed=True)) for s in arrays]
        ext = pda(program, b"pool_tick_array_bitmap_extension", b58d(pool))
        m.update(
            amm_config=key_at(b, 9), mint_0=key_at(b, 73), mint_1=key_at(b, 105), vault_0=key_at(b, 137), vault_1=key_at(b, 169),
            observation=key_at(b, 201), tick_spacing=spacing, tick_current=tick, bitmap_extension=ext,
        )
        base = [m["amm_config"], m["mint_0"], m["mint_1"], m["vault_0"], m["vault_1"], m["observation"]]
        save_all(d, base, base)
        m["tick_arrays"] = [(s, k) for s, k in zip(arrays, array_keys) if k in save_all(d, [k], [])]
        m["bitmap_extension_exists"] = bool(save_all(d, [ext], []))
    elif name == "whirlpool":
        # Whirlpool: tick_spacing 41, tick_current_index 81, mint_a 101, vault_a 133, mint_b 181,
        # vault_b 213.
        spacing, tick = u16(b, 41), i32(b, 81)
        size = spacing * 88
        start = (tick // size) * size
        arrays = [start + i * size for i in range(-3, 4)]
        array_keys = [pda(program, b"tick_array", b58d(pool), str(s).encode()) for s in arrays]
        oracle = pda(program, b"oracle", b58d(pool))
        m.update(
            mint_a=key_at(b, 101), vault_a=key_at(b, 133), mint_b=key_at(b, 181), vault_b=key_at(b, 213),
            tick_spacing=spacing, tick_current=tick, oracle=oracle, whirlpools_config=key_at(b, 8),
        )
        base = [m["mint_a"], m["mint_b"], m["vault_a"], m["vault_b"], m["whirlpools_config"]]
        save_all(d, base, base)
        m["tick_arrays"] = [(s, k) for s, k in zip(arrays, array_keys) if k in save_all(d, [k], [])]
        m["oracle_exists"] = bool(save_all(d, [oracle], []))
    else:
        # LbPair (IDL lb_clmm 0.12): active_id 76, bin_step 80, mints 88/120, reserves 152/184,
        # oracle 552.
        active = i32(b, 76)
        index = active // 70
        indexes = [index + i for i in range(-3, 4)]
        array_keys = [pda(program, b"bin_array", b58d(pool), i.to_bytes(8, "little", signed=True)) for i in indexes]
        ext = pda(program, b"bitmap", b58d(pool))
        m.update(
            active_id=active, bin_step=u16(b, 80), mint_x=key_at(b, 88), mint_y=key_at(b, 120),
            reserve_x=key_at(b, 152), reserve_y=key_at(b, 184), oracle=key_at(b, 552), bitmap_extension=ext,
            event_authority=pda(program, b"__event_authority"),
        )
        base = [m["mint_x"], m["mint_y"], m["reserve_x"], m["reserve_y"], m["oracle"]]
        save_all(d, base, base)
        m["bin_arrays"] = [(i, k) for i, k in zip(indexes, array_keys) if k in save_all(d, [k], [])]
        m["bitmap_extension_exists"] = bool(save_all(d, [ext], []))
    (d / "manifest.json").write_text(json.dumps(m, indent=1) + "\n")
    print(f"mainnet {name}: pool {pool}, {len(m.get('tick_arrays', m.get('bin_arrays', [])))} arrays")


if __name__ == "__main__":
    args = sys.argv[1:]
    if args == ["concentrated"]:
        for name in CONCENTRATED:
            concentrated(name)
        sys.exit()
    nets = args or ["devnet", "mainnet"]
    for net in nets:
        cpmm(net)
        try:
            damm(net)
        except Exception as e:  # mainnet getProgramAccounts is often refused by public RPC
            print(f"{net} damm-v2: skipped ({e})")
