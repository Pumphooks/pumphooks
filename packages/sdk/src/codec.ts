import { Buffer } from "buffer";
import { PublicKey } from "@solana/web3.js";

/** Little-endian writer for Borsh / fixed-layout instruction data. */
export class Writer {
  private buf: number[] = [];

  u8(v: number): this {
    this.buf.push(v & 0xff);
    return this;
  }
  bool(v: boolean): this {
    return this.u8(v ? 1 : 0);
  }
  u16(v: number): this {
    return this.u8(v).u8(v >>> 8);
  }
  u32(v: number): this {
    return this.u16(v & 0xffff).u16(v >>> 16);
  }
  u64(v: bigint | number): this {
    let x = BigInt.asUintN(64, BigInt(v));
    for (let i = 0; i < 8; i++) {
      this.buf.push(Number(x & 0xffn));
      x >>= 8n;
    }
    return this;
  }
  i64(v: bigint | number): this {
    return this.u64(BigInt.asUintN(64, BigInt(v)));
  }
  bytes(b: Uint8Array | readonly number[]): this {
    for (const x of b) this.buf.push(x);
    return this;
  }
  fixed(b: Uint8Array, len: number): this {
    if (b.length !== len) throw new Error(`expected ${len} bytes, got ${b.length}`);
    return this.bytes(b);
  }
  pubkey(k: PublicKey): this {
    return this.bytes(k.toBytes());
  }
  option<T>(v: T | null | undefined, f: (w: this, v: T) => void): this {
    if (v === null || v === undefined) return this.u8(0);
    this.u8(1);
    f(this, v);
    return this;
  }
  vec<T>(items: readonly T[], f: (w: this, v: T) => void): this {
    this.u32(items.length);
    for (const it of items) f(this, it);
    return this;
  }
  done(): Buffer {
    return Buffer.from(this.buf);
  }
}

/** Little-endian reader. Reads past the end return zeros, like the program's legacy-account views. */
export class Reader {
  constructor(
    readonly data: Uint8Array,
    public off = 0,
  ) {}

  private take(n: number): Uint8Array {
    const out = new Uint8Array(n);
    const end = Math.min(this.off + n, this.data.length);
    if (this.off < end) out.set(this.data.subarray(this.off, end));
    this.off += n;
    return out;
  }
  skip(n: number): this {
    this.off += n;
    return this;
  }
  u8(): number {
    return this.take(1)[0];
  }
  bool(): boolean {
    return this.u8() !== 0;
  }
  u16(): number {
    const b = this.take(2);
    return b[0] | (b[1] << 8);
  }
  u32(): number {
    const b = this.take(4);
    return (b[0] | (b[1] << 8) | (b[2] << 16)) + b[3] * 0x1000000;
  }
  u64(): bigint {
    const b = this.take(8);
    let x = 0n;
    for (let i = 7; i >= 0; i--) x = (x << 8n) | BigInt(b[i]);
    return x;
  }
  i64(): bigint {
    return BigInt.asIntN(64, this.u64());
  }
  i128(): bigint {
    const lo = this.u64();
    const hi = this.u64();
    return BigInt.asIntN(128, (hi << 64n) | lo);
  }
  bytes(n: number): Uint8Array {
    return this.take(n);
  }
  pubkey(): PublicKey {
    return new PublicKey(this.take(32));
  }
  vec<T>(f: (r: this) => T): T[] {
    const n = this.u32();
    const out: T[] = [];
    for (let i = 0; i < n; i++) out.push(f(this));
    return out;
  }
}

export function u16le(v: number): Buffer {
  return new Writer().u16(v).done();
}

export const ZERO_KEY = new PublicKey(new Uint8Array(32));
