import { sha256 } from "@noble/hashes/sha256";
import { PublicKey } from "@solana/web3.js";

/** A library-neutral instruction: base58 addresses, explicit signer / writable flags. */
export interface InstructionV1 {
  programId: string;
  accounts: { address: string; signer: boolean; writable: boolean }[];
  data: Uint8Array;
}

export type KeyLike = string | PublicKey | Uint8Array;

export const pk = (k: KeyLike): PublicKey => (k instanceof PublicKey ? k : new PublicKey(k));
export const b58 = (k: KeyLike): string => pk(k).toBase58();
export const bytes32 = (k: KeyLike): Uint8Array => pk(k).toBytes();

export const meta = (address: KeyLike, writable = false, signer = false) => ({ address: b58(address), signer, writable });

export function anchorDiscriminator(name: string, ns = "global"): Uint8Array {
  return sha256(new TextEncoder().encode(`${ns}:${name}`)).slice(0, 8);
}

export const u8 = (n: number) => Uint8Array.of(n);
export function u16(n: number): Uint8Array {
  const b = new Uint8Array(2);
  new DataView(b.buffer).setUint16(0, n, true);
  return b;
}
export function u32(n: number): Uint8Array {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setUint32(0, n, true);
  return b;
}
export function u64(n: bigint): Uint8Array {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, n, true);
  return b;
}
export function i64(n: bigint): Uint8Array {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigInt64(0, n, true);
  return b;
}
export const vecU8 = (b: Uint8Array) => concat(u32(b.length), b);

export function concat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
}

export function findPda(seeds: Uint8Array[], program: KeyLike): [string, number] {
  const [a, bump] = PublicKey.findProgramAddressSync(seeds, pk(program));
  return [a.toBase58(), bump];
}

export const seed = (s: string) => new TextEncoder().encode(s);
