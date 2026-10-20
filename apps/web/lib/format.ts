import type { PublicKey } from "@solana/web3.js";

const LAMPORTS = 1_000_000_000n;

export function sol(lamports: bigint | number, digits = 4): string {
  const v = BigInt(lamports);
  const neg = v < 0n;
  const a = neg ? -v : v;
  const whole = a / LAMPORTS;
  const frac = (a % LAMPORTS).toString().padStart(9, "0").slice(0, digits).replace(/0+$/, "");
  return `${neg ? "−" : ""}${whole.toLocaleString("en-US")}${frac ? "." + frac : ""}`;
}

export function parseSol(input: string): bigint | undefined {
  const s = input.trim();
  if (!/^\d*(\.\d{0,9})?$/.test(s) || s === "" || s === ".") return undefined;
  const [w, f = ""] = s.split(".");
  return BigInt(w || "0") * LAMPORTS + BigInt((f + "000000000").slice(0, 9));
}

export function units(raw: bigint, decimals: number, digits = 2): string {
  const d = 10n ** BigInt(decimals);
  const whole = raw / d;
  const frac = (raw % d).toString().padStart(decimals, "0").slice(0, digits).replace(/0+$/, "");
  return `${whole.toLocaleString("en-US")}${frac ? "." + frac : ""}`;
}

export function parseUnits(input: string, decimals: number): bigint | undefined {
  const s = input.trim();
  if (!new RegExp(`^\\d*(\\.\\d{0,${decimals}})?$`).test(s) || s === "" || s === ".") return undefined;
  const [w, f = ""] = s.split(".");
  return BigInt(w || "0") * 10n ** BigInt(decimals) + BigInt((f + "0".repeat(decimals)).slice(0, decimals) || "0");
}

export function short(k: PublicKey | string, n = 4): string {
  const s = typeof k === "string" ? k : k.toBase58();
  return s.length <= n * 2 + 1 ? s : `${s.slice(0, n)}…${s.slice(-n)}`;
}

export function duration(secs: number | bigint): string {
  let s = Number(secs);
  if (s === 0) return "0s";
  const parts: string[] = [];
  for (const [u, n] of [["d", 86_400], ["h", 3_600], ["m", 60], ["s", 1]] as const) {
    if (s >= n) {
      parts.push(`${Math.floor(s / n)}${u}`);
      s %= n;
    }
  }
  return parts.slice(0, 2).join(" ");
}

export function ago(unix: number | null | undefined): string {
  if (!unix) return "—";
  const d = Math.max(0, Math.floor(Date.now() / 1000) - unix);
  if (d < 60) return `${d}s ago`;
  if (d < 3600) return `${Math.floor(d / 60)}m ago`;
  if (d < 86_400) return `${Math.floor(d / 3600)}h ago`;
  return `${Math.floor(d / 86_400)}d ago`;
}

export const pad = (n: number, w = 2) => n.toString().padStart(w, "0");
