"""Builds the README graphics in the pumphooks web app's visual language.

Paper, ink and one signal colour; Inter Tight + JetBrains Mono (subset from the web app's build
and embedded, so GitHub renders the real type); the dithered engine painting from
apps/web/public/art with the hook-shaped lens that reveals its colour.

    python3 .github/assets/src/build.py      # needs Pillow, numpy, fonttools, brotli
"""

import base64
import glob
import io
import os

import numpy as np
from fontTools import subset
from fontTools.ttLib import TTFont
from PIL import Image

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "../../.."))
OUT = os.path.join(ROOT, ".github/assets")

PAPER, PAPER2, PAPER3 = "#efede6", "#e7e4db", "#dedad0"
INK, INK2, MUTE, FAINT = "#0e0e0c", "#2c2b27", "#6f6d65", "#9a978d"
RULE, RULE2 = "#cdc9bd", "#b9b4a6"
SIGNAL, OK, DENY, WARN = "#ff4d00", "#17803d", "#c23317", "#a86a00"
ON_INK_MUTE = "#a29f95"

DOT, ARROW, LE, GT, GE, DOWN, DELTA = "\u00b7", "\u2192", "\u2264", "&gt;", "\u2265", "\u2193", "\u0394"
EXTRA = "\u00b7\u2192\u2264\u2265\u2193\u0394\u203a\u2014"


# ---------------------------------------------------------------- fonts

def font_face(family_name, family_css, weight_range):
    media = os.path.join(ROOT, "apps/web/.next/static/media")
    best = None
    for f in glob.glob(os.path.join(media, "*.p.*.woff2")):
        if TTFont(f)["name"].getDebugName(1) == family_name:
            best = f
    if best is None:
        raise SystemExit(f"{family_name} not found: build apps/web first (pnpm --filter @pump-hooks/web build)")
    opts = subset.Options()
    opts.flavor = "woff2"
    opts.layout_features = ["kern", "liga", "calt", "tnum"]
    font = subset.load_font(best, opts)
    s = subset.Subsetter(opts)
    s.populate(unicodes=list(range(0x20, 0x7F)) + [ord(c) for c in EXTRA])
    s.subset(font)
    buf = io.BytesIO()
    subset.save_font(font, buf, opts)
    b64 = base64.b64encode(buf.getvalue()).decode()
    return (f"@font-face{{font-family:'{family_css}';font-weight:{weight_range};"
            f"src:url(data:font/woff2;base64,{b64}) format('woff2')}}")


FONTS = None


def fonts():
    global FONTS
    if FONTS is None:
        FONTS = font_face("Inter Tight", "PH Display", "100 900") + font_face("JetBrains Mono", "PH Mono", "100 800")
    return FONTS


BASE_CSS = f"""
.d{{font-family:'PH Display','Inter Tight','Helvetica Neue',Arial,sans-serif}}
.m{{font-family:'PH Mono','JetBrains Mono',ui-monospace,Menlo,monospace}}
.eyebrow{{font-size:12px;letter-spacing:.14em;fill:{INK2}}}
"""


def doc(w, h, body, css="", defs=""):
    return f"""<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{w}" height="{h}" viewBox="0 0 {w} {h}">
<defs>
<style>{fonts()}{BASE_CSS}{css}</style>
<filter id="grain" x="0" y="0" width="100%" height="100%">
  <feTurbulence type="fractalNoise" baseFrequency="0.85" numOctaves="2" seed="7" stitchTiles="stitch"/>
  <feColorMatrix values="0 0 0 0 0.06  0 0 0 0 0.06  0 0 0 0 0.05  0 0 0 0.16 0"/>
</filter>
{defs}
</defs>
<rect width="{w}" height="{h}" fill="{PAPER}"/>
{body}
<rect width="{w}" height="{h}" filter="url(#grain)" pointer-events="none"/>
</svg>
"""


def text(x, y, s, size, cls="d", fill=INK, anchor="start", weight=None, ls=None, extra=""):
    a = f' text-anchor="{anchor}"' if anchor != "start" else ""
    wgt = f' font-weight="{weight}"' if weight else ""
    l = f' letter-spacing="{ls}"' if ls is not None else ""
    return f'<text x="{x}" y="{y}" class="{cls}" font-size="{size}" fill="{fill}"{a}{wgt}{l} {extra}>{s}</text>'


def mono_w(s, size, ls_em=0.0):
    n = len(s.replace("&gt;", ">").replace("&lt;", "<").replace("&amp;", "&"))
    return n * size * 0.6 + n * size * ls_em


def tag(x, y, label, kind="plain", anchor="start", dashed=False):
    """`.tag` from globals.css: 22px, mono 10.5px, .08em, 1px border. `y` is the top."""
    size, ls = 10.5, 0.08
    dot = kind in ("ok", "warn", "deny")
    w = mono_w(label, size, ls) + 16 + (13 if dot else 0)
    if anchor == "end":
        x -= w
    stroke = {"plain": RULE2, "ink": INK, "ok": OK, "warn": WARN, "deny": DENY}[kind]
    color = {"plain": INK, "ink": PAPER, "ok": OK, "warn": WARN, "deny": DENY}[kind]
    fill = INK if kind == "ink" else "none"
    so = "0.45" if kind in ("ok", "warn", "deny") else "1"
    dash = ' stroke-dasharray="3 3"' if dashed else ""
    out = f'<rect x="{x + .5}" y="{y + .5}" width="{w - 1}" height="21" fill="{fill}" stroke="{stroke}" stroke-opacity="{so}"{dash}/>'
    tx = x + 8
    if dot:
        out += f'<circle cx="{x + 11.5}" cy="{y + 11}" r="3.5" fill="{color}" class="pulse"/>'
        tx += 13
    out += text(tx, y + 15, label.upper(), size, "m", color, ls=f"{ls}em")
    return out, w


def hook_mark(x, y, size, stroke=INK):
    s = size / 1.27
    return (f'<g transform="translate({x + 0.27 * s} {y + 0.86 * s}) scale({s})" fill="none" stroke="{stroke}" '
            f'stroke-width="0.15" stroke-linecap="round" stroke-linejoin="round">'
            f'<circle cx="0" cy="-0.62" r="0.15"/><path d="M0 -0.47 V0.08 A0.24 0.24 0 0 0 0.48 0.08 V-0.1 L0.4 -0.03"/></g>')


def b64img(img, fmt, **kw):
    buf = io.BytesIO()
    img.save(buf, fmt, **kw)
    mime = "image/png" if fmt == "PNG" else "image/jpeg"
    return f"data:{mime};base64," + base64.b64encode(buf.getvalue()).decode()


def write(name, svg):
    with open(os.path.join(OUT, name), "w", encoding="utf-8") as f:
        f.write(svg)
    print(f"{name}: {len(svg.encode()) // 1024} KB")


# ---------------------------------------------------------------- art

BAYER8 = np.array([
    [0, 32, 8, 40, 2, 34, 10, 42], [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38], [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41], [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37], [63, 31, 55, 23, 61, 29, 53, 21]], dtype=np.float32)


def painting(display_w, crop_frac, cell=3, scale=2):
    """(dithered PNG data URI, colour JPEG data URI, display height)."""
    src = Image.open(os.path.join(ROOT, "apps/web/public/art/hook-engine.jpg")).convert("RGB")
    W, H = src.size
    src = src.crop((0, 0, W, int(H * crop_frac)))
    display_h = round(display_w * src.size[1] / src.size[0])

    gw, gh = display_w // cell, display_h // cell
    g = np.asarray(src.convert("L").resize((gw, gh), Image.LANCZOS), dtype=np.float32)
    lum = np.clip((g - 18) / (232 - 18), 0, 1) ** 1.15
    thr = (np.tile(BAYER8, (gh // 8 + 1, gw // 8 + 1))[:gh, :gw] + 0.5) / 64
    ink = lum < thr
    rgba = np.zeros((gh, gw, 4), dtype=np.uint8)
    rgba[ink] = (14, 14, 12, 255)
    d = Image.fromarray(rgba, "RGBA").resize((gw * cell * scale, gh * cell * scale), Image.NEAREST)
    dither = b64img(d.quantize(2), "PNG", optimize=True)

    c = np.asarray(src.resize((int(display_w * 1.5), int(display_h * 1.5)), Image.LANCZOS), dtype=np.float32)
    white = np.clip((c.min(axis=2) - 200) / 40, 0, 1)[..., None]
    paper = np.array([0xef, 0xed, 0xe6], dtype=np.float32)
    c = c * (1 - white) + paper * white
    colour = b64img(Image.fromarray(c.astype(np.uint8)), "JPEG", quality=78, optimize=True, progressive=True)
    return dither, colour, display_h, gw * cell


# ---------------------------------------------------------------- hero

def hero():
    W, H = 1280, 920
    body = []
    # header, as on the site
    body.append(hook_mark(48, 21, 22))
    body.append(text(72, 38, "Pump Hooks", 17, "d", INK, weight=600, ls="-0.02em"))
    t, w = tag(0, 21, "devnet", "warn")
    body.append(f'<g transform="translate({W - 48 - w} 0)">{t}</g>')
    body.append(text(W - 48 - w - 20, 36, f"SOLANA {DOT} HOOK STANDARD V1", 11, "m", MUTE, "end", ls="0.14em"))
    body.append(f'<line x1="0" y1="64.5" x2="{W}" y2="64.5" stroke="{RULE}"/>')

    body.append(text(W / 2, 150, f'<tspan fill="{MUTE}">00</tspan>   /   PUMP HOOKS', 12, "m eyebrow", INK2, "middle"))
    body.append(text(W / 2, 252, "Make Solana", 104, "d", INK, "middle", 500, "-0.048em"))
    body.append(text(W / 2, 352, "programmable.", 104, "d", INK, "middle", 500, "-0.048em"))
    body.append(text(W / 2, 410, "One hook standard for tokens, transactions, liquidity pools and smart accounts.", 20, "d", INK2, "middle", ls="-0.005em"))

    # chips: the selected one walks across, like the registry filter on /hooks
    chips = [("TOKENS", "01"), ("TRANSACTIONS", "02"), ("LIQUIDITY POOLS", "03"), ("SMART ACCOUNTS", "04")]
    cw, ch, cx0, cy = 214, 46, (W - 4 * 214) / 2, 448
    plain, inverse = [], []
    for i, (name, n) in enumerate(chips):
        x = cx0 + i * cw
        plain.append(f'<rect x="{x + .5}" y="{cy + .5}" width="{cw}" height="{ch}" fill="none" stroke="{RULE}"/>')
        plain.append(text(x + 14, cy + 28, name, 11.5, "m", INK, ls="0.1em"))
        plain.append(text(x + cw - 14, cy + 28, n, 11.5, "m", FAINT, "end"))
        inverse.append(text(x + 14, cy + 28, name, 11.5, "m", PAPER, ls="0.1em"))
        inverse.append(text(x + cw - 14, cy + 28, n, 11.5, "m", ON_INK_MUTE, "end"))
    xs = ";".join(str(cx0 + i * cw) for i in range(4))
    sel = (f'<rect x="{cx0}" y="{cy}" width="{cw + 1}" height="{ch + 1}">'
           f'<animate attributeName="x" values="{xs}" dur="10s" calcMode="discrete" repeatCount="indefinite"/></rect>')
    body.append("".join(plain))
    body.append(f'<clipPath id="sel">{sel}</clipPath>')
    body.append(f'<g fill="{INK}">{sel}</g>')
    body.append(f'<g clip-path="url(#sel)">{"".join(inverse)}</g>')
    body.append(text(W / 2, 534, f"OPEN SOURCE   {DOT}   DEVNET   {DOT}   NOT AUDITED   {DOWN}", 11, "m", MUTE, "middle", ls="0.12em"))

    # painting with the hook lens
    dw = 1000
    dither, colour, dh, _ = painting(dw, 0.39)
    px, py = (W - dw) / 2, H - dh + 6
    S = 110
    lens = (f'<g transform="scale({S})" fill="none" stroke="#fff" stroke-width="0.34" stroke-linecap="round" stroke-linejoin="round">'
            f'<circle cx="0" cy="-0.62" r="0.15"/><path d="M0 -0.47 V0.08 A0.24 0.24 0 0 0 0.48 0.08 V-0.1 L0.4 -0.03"/></g>')
    def p(fx, fy):
        return f"{px + dw * fx:.0f} {py + dh * fy:.0f}"
    path = (f"M{p(.40, .62)} C{p(.46, .40)} {p(.56, .80)} {p(.60, .58)} "
            f"S{p(.52, .25)} {p(.47, .40)} S{p(.34, .85)} {p(.40, .62)}")
    defs = f"""
<filter id="soft" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="9"/></filter>
<mask id="lens" maskUnits="userSpaceOnUse" x="0" y="0" width="{W}" height="{H}">
  <g filter="url(#soft)"><g>{lens}<animateMotion dur="16s" repeatCount="indefinite" path="{path}"/></g></g>
</mask>"""
    body.append(f'<image x="{px}" y="{py}" width="{dw}" height="{dh}" xlink:href="{dither}" href="{dither}"/>')
    body.append(f'<image x="{px}" y="{py}" width="{dw}" height="{dh}" xlink:href="{colour}" href="{colour}" mask="url(#lens)"/>')
    css = ".pulse{animation:pulse 1.6s ease-in-out infinite}@keyframes pulse{50%{opacity:.35}}"
    write("hero.svg", doc(W, H, "\n".join(body), css, defs))


# ---------------------------------------------------------------- system map

def system():
    W, X0, X1 = 1280, 48, 1232
    BW = X1 - X0
    body, css = [], []
    body.append(text(X0, 74, f'<tspan fill="{MUTE}">01</tspan>   /   SYSTEM', 12, "m eyebrow"))
    body.append(text(X0, 140, "One standard. Four engines.", 56, "d", INK, weight=500, ls="-0.04em"))
    body.append(text(X0, 182, "Router, pool and account engines install from one registry and run through one kernel. Tokens are next.", 17, "d", INK2))

    y = 222
    top = y
    # A: build
    cells = [("SDK", "@pumphooks/sdk"), ("CLI", "pumphooks"), ("COMPOSER", "Visual composer")]
    cw = BW / 3
    for i, (k, v) in enumerate(cells):
        x = X0 + i * cw
        if i:
            body.append(f'<line x1="{x + .5}" y1="{y}" x2="{x + .5}" y2="{y + 84}" stroke="{RULE}"/>')
        body.append(text(x + 20, y + 30, k, 11, "m", MUTE, ls="0.12em"))
        body.append(text(x + 20, y + 64, v, 24, "d", INK, weight=500, ls="-0.025em"))
        t, _ = tag(x + cw - 20, y + 16, "in progress", "warn", "end")
        body.append(t)
    rows = [(y, 84)]
    y += 84

    def arrow_row(y, xs):
        out = f'<line x1="{X0}" y1="{y + .5}" x2="{X1}" y2="{y + .5}" stroke="{RULE}"/>'
        for x in xs:
            out += text(x, y + 20, DOWN, 13, "m", MUTE, "middle")
        return out + f'<line x1="{X0}" y1="{y + 28.5}" x2="{X1}" y2="{y + 28.5}" stroke="{RULE}"/>'

    body.append(arrow_row(y, [X0 + BW / 2]))
    y += 28

    # B: registry
    LBL = 210
    body.append(text(X0 + 20, y + 32, f'<tspan fill="{MUTE}">02</tspan>  REGISTRY', 11, "m", INK2, ls="0.12em"))
    body.append(text(X0 + LBL, y + 38, "Hook registry", 26, "d", INK, weight=500, ls="-0.025em"))
    body.append(text(X0 + LBL, y + 66, "Append-only versions pinned to a program and its upgrade authority.", 15, "d", INK2))
    body.append(text(X0 + LBL, y + 86, "Deprecation, named reviewers and trust tiers that only say what is true.", 15, "d", INK2))
    tx = X1 - 20
    t, w = tag(tx, y + 24, "audited", "plain", "end", dashed=True)
    body.append(t.replace(f'fill="{INK}"', f'fill="{FAINT}"', 1))
    tx -= w + 6
    t, w = tag(tx, y + 24, "tested", "ok", "end")
    body.append(t)
    tx -= w + 6
    t, w = tag(tx, y + 24, "community", "plain", "end")
    body.append(t)
    body.append(text(X1 - 20, y + 74, "AUDITED APPEARS ONLY WITH A REAL AUDIT", 10, "m", FAINT, "end", ls="0.1em"))
    rows.append((y, 108))
    y += 108
    body.append(arrow_row(y, [X0 + BW / 2]))
    y += 28

    # C: kernel (ink row, like .pipe-row.cpi)
    KH = 128
    body.append(f'<rect x="{X0}" y="{y}" width="{BW}" height="{KH}" fill="{INK}"/>')
    body.append(text(X0 + 20, y + 32, f'<tspan fill="{ON_INK_MUTE}">03</tspan>  KERNEL', 11, "m", PAPER, ls="0.12em"))
    body.append(text(X0 + LBL, y + 40, "Execution kernel", 26, "d", PAPER, weight=500, ls="-0.025em"))
    body.append(text(X0 + LBL, y + 68, "ABI v1 hook stacks, capability grants, monotone fail-closed composition.", 15, "d", "#d8d5cb"))
    body.append(text(X0 + LBL, y + 88, "Builtins run inline. Program hooks run by CPI with no signers and only", 15, "d", "#d8d5cb"))
    body.append(text(X0 + LBL, y + 108, "the accounts their capabilities allow.", 15, "d", "#d8d5cb"))
    builtins = ["ALLOWLIST", "SPENDING_LIMIT", "TIMELOCK", "MULTISIG", "CONDITIONAL", "MAX_SIZE", "TIME_WINDOW", "ACCOUNTING"]
    gx, gy = X1 - 20 - 2 * 170, y + 22
    for i, b in enumerate(builtins):
        bx, by = gx + (i % 2) * 170, gy + (i // 2) * 22
        body.append(text(bx, by + 14, f'<tspan fill="{ON_INK_MUTE}">0{i + 1}</tspan>  {b}', 11, "m", PAPER, ls="0.06em"))
    rows.append((y, KH))
    y += KH

    ew = BW / 4
    centers = [X0 + ew * (i + .5) for i in range(4)]
    body.append(arrow_row(y, centers))
    y += 28

    # D: engines (hook-card)
    EH = 236
    engines = [
        ("0x00", "TOKEN", "Tokens", ["SPL in, Token-2022 out. In the", "standard and the kernel; the", "wrapper engine is not built."], "wrapper", "not built", "warn", "\u2014"),
        ("0x01", "ROUTER", "Transactions", ["Allowlisted venue swaps with", "measured slippage, deadline", "and replay-proof receipts."], "router", "tested", "ok", "15"),
        ("0x02", "POOL", "Liquidity pools", ["Registry-pinned pool hooks for", "fees, deltas and quotes, plus", "builtin stacks on the kernel."], "program", "tested", "ok", "25"),
        ("0x03", "ACCOUNT", "Smart accounts", ["Vault PDA with limits, timelocks,", "multisig, sessions, recovery", "and an emergency lock."], "program", "tested", "ok", "13"),
    ]
    hl = (f'<rect x="{X0}" y="{y}" width="{ew}" height="{EH}" fill="{PAPER2}">'
          f'<animate attributeName="x" values="{";".join(str(X0 + ew * i) for i in range(4))}" dur="8s" calcMode="discrete" repeatCount="indefinite"/></rect>'
          f'<rect x="{X0}" y="{y}" width="{ew}" height="3" fill="{SIGNAL}">'
          f'<animate attributeName="x" values="{";".join(str(X0 + ew * i) for i in range(4))}" dur="8s" calcMode="discrete" repeatCount="indefinite"/></rect>')
    body.append(hl)
    for i, (hid, env, name, desc, enf, status, kind, tests) in enumerate(engines):
        x = X0 + i * ew
        if i:
            body.append(f'<line x1="{x + .5}" y1="{y}" x2="{x + .5}" y2="{y + EH}" stroke="{INK}"/>')
        body.append(text(x + 20, y + 32, f"{hid} {DOT} {env}", 11, "m", MUTE))
        t, _ = tag(x + ew - 20, y + 17, enf, "ink", "end")
        body.append(t)
        body.append(text(x + 20, y + 76, name, 28, "d", INK, weight=500, ls="-0.03em"))
        for j, line in enumerate(desc):
            body.append(text(x + 20, y + 106 + j * 20, line, 14.5, "d", INK2))
        body.append(f'<line x1="{x + 20}" y1="{y + 180.5}" x2="{x + ew - 20}" y2="{y + 180.5}" stroke="{RULE}"/>')
        body.append(text(x + 20, y + 206, "STATUS", 11, "m", MUTE, ls="0.06em"))
        body.append(f'<circle cx="{x + 20 + 3.5}" cy="{y + 222}" r="3.5" fill="{OK if kind == "ok" else WARN}" class="pulse"/>')
        body.append(text(x + 32, y + 226, status, 12, "m", INK))
        body.append(text(x + ew / 2 + 10, y + 206, "LITESVM TESTS", 11, "m", MUTE, ls="0.06em"))
        body.append(text(x + ew / 2 + 10, y + 226, tests, 12, "m", INK))
    rows.append((y, EH))
    y += EH

    # E: state records
    body.append(f'<line x1="{X0}" y1="{y + .5}" x2="{X1}" y2="{y + .5}" stroke="{INK}"/>')
    body.append(text(X0 + 20, y + 30, f'<tspan fill="{MUTE}">04</tspan>  STATE', 11, "m", INK2, ls="0.12em"))
    body.append(text(X0 + LBL, y + 31, f"Typed records shared across environments: producer {DOT} schema {DOT} namespace {DOT} subject {DOT} freshness {DOT} replay guard", 14.5, "d", INK2))
    rows.append((y, 48))
    y += 48
    body.append(f'<rect x="{X0 + .5}" y="{top + .5}" width="{BW - 1}" height="{y - top - 1}" fill="none" stroke="{INK}"/>')

    # signal marker walking down the stack
    ys = ";".join(str(r[0]) for r in rows)
    hs = ";".join(str(r[1]) for r in rows)
    body.append(f'<rect x="{X0 - 6}" y="{rows[0][0]}" width="3" height="{rows[0][1]}" fill="{SIGNAL}">'
                f'<animate attributeName="y" values="{ys}" dur="6s" calcMode="discrete" repeatCount="indefinite"/>'
                f'<animate attributeName="height" values="{hs}" dur="6s" calcMode="discrete" repeatCount="indefinite"/></rect>')
    H = y + 48
    css.append(".pulse{animation:pulse 1.6s ease-in-out infinite}@keyframes pulse{50%{opacity:.35}}")
    write("system.svg", doc(W, H, "\n".join(body), "".join(css)))


# ---------------------------------------------------------------- router pipeline

def pipeline(x0, y0, width, title, verdict, rows, T, start, kf):
    """`.pipeline` from globals.css. rows: (index, stage, detail, result, kind). kind: plain | hook | cpi | denied | skipped."""
    out = []
    RH = 42
    out.append(text(x0, y0 - 16, title, 12, "m", INK2, ls="0.12em"))
    out.append(f'<rect x="{x0}" y="{y0}" width="{width}" height="{RH * len(rows)}" fill="#f3f1eb"/>')
    for i, (idx, stage, detail, result, kind) in enumerate(rows):
        y = y0 + i * RH
        t_on = start + i * 0.075
        cls = f"r{kf}{i}"
        if kind == "cpi":
            out.append(f'<rect x="{x0}" y="{y}" width="{width}" height="{RH}" fill="{INK}"/>')
        if kind == "denied":
            out.append(f'<rect x="{x0}" y="{y}" width="{width}" height="{RH}" fill="{DENY}" fill-opacity="0.09"/>')
        if i:
            out.append(f'<line x1="{x0}" y1="{y + .5}" x2="{x0 + width}" y2="{y + .5}" stroke="{RULE}"/>')
        fg = PAPER if kind == "cpi" else (FAINT if kind == "skipped" else INK)
        ix = ON_INK_MUTE if kind == "cpi" else FAINT
        sx = x0 + (44 if kind in ("hook", "denied", "skipped") and stage.isupper() and idx.startswith("h") else 40)
        indent = kind in ("hook", "denied") or (kind == "skipped" and idx.startswith("h"))
        num = idx[1:] if idx.startswith("h") else idx
        out.append(text(x0 + 12, y + 26, num, 11, "m", ix))
        if indent:
            out.append(f'<line x1="{x0 + 42}" y1="{y + 21.5}" x2="{x0 + 48}" y2="{y + 21.5}" stroke="{fg}"/>')
            sx = x0 + 54
        out.append(text(sx, y + 26, stage, 11, "m", fg, ls="0.08em"))
        out.append(text(x0 + 186, y + 26, detail, 12, "m", fg if kind != "plain" else INK2))
        rc = {"ok": OK, "deny": DENY}.get(result[1], fg)
        out.append(f'<g class="{cls}">' + text(x0 + width - 12, y + 26, result[0], 12, "m", rc, "end") + "</g>")
        kf_css = f".{cls}{{animation:{cls} {T}s linear infinite}}@keyframes {cls}{{0%,{t_on * 100:.1f}%{{opacity:0}}{t_on * 100 + 2:.1f}%,94%{{opacity:1}}98%,100%{{opacity:0}}}}"
        out.append(f"<style>{kf_css}</style>")
    yb = y0 + RH * len(rows)
    out.append(f'<rect x="{x0 + .5}" y="{y0 + .5}" width="{width - 1}" height="{yb - y0 - 1}" fill="none" stroke="{INK}"/>')
    vt = start + len([r for r in rows if r[4] != "skipped"]) * 0.075 + 0.04
    label, kind = verdict
    color = OK if kind == "ok" else DENY
    vcls = f"v{kf}"
    t, w = tag(x0, yb + 18, label, kind)
    out.append(f'<g class="{vcls}">{t}</g>')
    out.append(f"<style>.{vcls}{{animation:{vcls} {T}s linear infinite}}@keyframes {vcls}{{0%,{vt * 100:.1f}%{{opacity:0}}{vt * 100 + 2:.1f}%,94%{{opacity:1}}98%,100%{{opacity:0}}}}</style>")
    # signal marker
    n_live = len([r for r in rows if r[4] != "skipped"])
    keys = [0.0] + [start + i * 0.075 for i in range(n_live)] + [0.94]
    ys = [y0] + [y0 + i * RH for i in range(n_live)] + [y0]
    out.append(f'<rect x="{x0 - 6}" y="{y0}" width="3" height="{RH}" fill="{SIGNAL}" opacity="0">'
               f'<animate attributeName="y" values="{";".join(map(str, ys))}" keyTimes="{";".join(f"{k:.3f}" for k in keys)}" dur="{T}s" calcMode="discrete" repeatCount="indefinite"/>'
               f'<animate attributeName="opacity" values="0;1;0" keyTimes="0;{start:.3f};0.94" dur="{T}s" calcMode="discrete" repeatCount="indefinite"/></rect>')
    return "\n".join(out), yb + 40


def route():
    W, X0 = 1280, 48
    body = []
    body.append(text(X0, 74, f'<tspan fill="{MUTE}">02</tspan>   /   ROUTER', 12, "m eyebrow"))
    body.append(text(X0, 140, "Every hook can veto. None can override.", 56, "d", INK, weight=500, ls="-0.04em"))
    body.append(text(X0, 182, "Before hooks, the venue CPI, measured deltas, after hooks, receipt. One transaction: a single denial reverts all of it.", 17, "d", INK2))

    allowed = [
        ("01", "SIGN", f"max_in 0.20 SOL {DOT} min_out {DOT} deadline", ("user", ""), "plain"),
        ("02", "ADAPTER", "PumpSwap buy_exact_quote_in_v2", ("verified", "ok"), "plain"),
        ("h03", "ALLOWLIST", "before_swap: actor is listed", ("ok", "ok"), "hook"),
        ("h04", "MAX_SIZE", f"before_swap: 0.20 {LE} 0.25 SOL", ("ok", "ok"), "hook"),
        ("05", "CPI", f"pump_amm {DOT} the user's own signature", ("swap", ""), "cpi"),
        ("06", "MEASURE", f"{DELTA}in {LE} max_in {DOT} {DELTA}out {GE} min_out", ("ok", "ok"), "plain"),
        ("h07", "ACCOUNTING", f"after_swap: records measured {DELTA}in", ("recorded", "ok"), "hook"),
        ("08", "RECEIPT", "PDA (user, nonce): replay-proof", ("written", "ok"), "plain"),
    ]
    denied = [
        ("01", "SIGN", f"max_in 0.50 SOL {DOT} min_out {DOT} deadline", ("user", ""), "plain"),
        ("02", "ADAPTER", "PumpSwap buy_exact_quote_in_v2", ("verified", "ok"), "plain"),
        ("h03", "ALLOWLIST", "before_swap: actor is listed", ("ok", "ok"), "hook"),
        ("h04", "MAX_SIZE", f"before_swap: 0.50 {GT} 0.25 SOL", ("deny", "deny"), "denied"),
        ("05", "CPI", "never called", ("", ""), "skipped"),
        ("06", "MEASURE", "skipped", ("", ""), "skipped"),
        ("h07", "ACCOUNTING", "skipped", ("", ""), "skipped"),
        ("08", "RECEIPT", "not written", ("", ""), "skipped"),
    ]
    gap = 32
    cw = (W - 2 * X0 - gap) / 2
    T = 10
    a, yb = pipeline(X0, 262, cw, f"ROUTE 0.20 SOL {DOT} STACK: ALLOWLIST, MAX_SIZE 0.25, ACCOUNTING", ("committed", "ok"), allowed, T, 0.05, "a")
    d, _ = pipeline(X0 + cw + gap, 262, cw, "SAME STACK, ROUTE 0.50 SOL", ("reverted, nothing moved", "deny"), denied, T, 0.05, "b")
    body += [a, d]
    note = "Direct venue calls skip router hooks by design. Pool and account hooks bind every operation on those programs."
    body.append(f'<line x1="{X0}" y1="{yb + 20.5}" x2="{W - X0}" y2="{yb + 20.5}" stroke="{RULE}"/>')
    body.append(text(X0, yb + 50, f'<tspan fill="{SIGNAL}">NOTE</tspan>   {note}', 12, "m", INK2))
    H = yb + 82
    css = ".pulse{animation:pulse 1.6s ease-in-out infinite}@keyframes pulse{50%{opacity:.35}}"
    write("route.svg", doc(W, H, "\n".join(body), css))


if __name__ == "__main__":
    hero()
    system()
    route()
