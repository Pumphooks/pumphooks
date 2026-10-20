// Mirror of `hook_kernel::validate_stack` and `builtin::validate_config`. The result strings equal
// the kernel's `StackError` Debug output; test/vectors.test.ts checks them case by case.

import { v1 } from "@pump-hooks/sdk";

const { Cap } = v1;
export const ABI_VERSION = 1;
export const MAX_PROGRAM_HOOKS_PER_CALLBACK = 2;
export const POLICY_PROGRAM_CAPS =
  Cap.READ_ACCOUNT_STATE | Cap.READ_TOKEN_BALANCE | Cap.VALIDATE_OPERATION | Cap.VETO_OPERATION | Cap.UPDATE_OWN_STATE | Cap.PRODUCE_STATE | Cap.CONSUME_STATE;

export const CALLBACK_COUNT: Record<number, number> = { 0: 4, 1: 4, 2: 11, 3: 8 };
export const callbackMask = (env: number) => (1 << CALLBACK_COUNT[env]) - 1;

export type StackError =
  | "Version"
  | "Environment"
  | "TooLong"
  | `UnknownImpl(${number})`
  | `Callbacks(${number})`
  | `Capabilities(${number})`
  | `Dependency(${number})`
  | `Config(${number})`
  | `Program(${number})`
  | `TooManyProgramHooks(${number})`
  | `Duplicate(${number})`
  | `DirtySlot(${number})`;

const zero = (b: Uint8Array) => b.every((x) => x === 0);
const eq = (a: Uint8Array, b: Uint8Array) => a.length === b.length && a.every((x, i) => x === b[i]);
const popcount = (n: number) => n.toString(2).split("1").length - 1;

export function validateConfig(id: number, c: Uint8Array, members: number): boolean {
  const v = new DataView(c.buffer, c.byteOffset, c.byteLength);
  const u64 = (o: number) => v.getBigUint64(o, true);
  const i64 = (o: number) => v.getBigInt64(o, true);
  const u32 = (o: number) => v.getUint32(o, true);
  const u16 = (o: number) => v.getUint16(o, true);
  switch (id) {
    case 1: {
      const [target, source] = [c[0], c[1]];
      if (target > 1) return false;
      if (source === 0) return !zero(c.slice(16, 48)) && u64(8) === 0n;
      if (source === 1) return !zero(c.slice(16, 48)) && !zero(c.slice(48, 80));
      return false;
    }
    case 2:
      return u32(40) > 0;
    case 3:
      return u32(0) > 0 || i64(8) > 0n;
    case 4: {
      const [threshold, mask] = [c[0], c[1]];
      const membersMask = members >= 8 ? 0xff : (1 << members) - 1;
      return threshold > 0 && (mask & ~membersMask) === 0 && threshold <= popcount(mask);
    }
    case 5: {
      const used = [0, 1, 2, 3].map((i) => c.slice(i * 16, i * 16 + 16)).filter((x) => x[0] !== 0);
      return (
        used.some((x) => x[2] === 1) &&
        used.every(([field, op, role]) => role <= 1 && op <= 5 && (field === 7 || field === 8 ? op >= 4 : field <= 8))
      );
    }
    case 6:
      return u64(32) > 0n;
    case 7: {
      const [nb, na, ds, de] = [i64(0), i64(8), u32(16), u32(20)];
      const daily = ds !== 0 || de !== 0;
      const dailyOk = !daily || (ds < 86_400 && de <= 86_400 && ds !== de);
      return (na === 0n || na > nb) && dailyOk && (daily || nb !== 0n || na !== 0n);
    }
    case 8:
      return zero(c);
    case 9:
      return c[0] <= 1 && (!zero(c.slice(8, 40)) || !zero(c.slice(40, 72)));
    case 10: {
      const [mode, base, max, pivot, alpha, sens] = [c[0], u16(4), u16(6), u64(8), u16(16), u32(20)];
      return base <= max && max <= 10_000 && (mode === 0 ? pivot > 0n : mode === 1 ? alpha > 0 && alpha <= 10_000 && sens > 0 : false);
    }
    case 11: {
      const m = u16(0);
      return m > 0 && m < 10_000;
    }
    case 12: {
      const [minNum, minDen, maxNum, maxDen, mode] = [u64(0), u64(8), u64(16), u64(24), c[64]];
      const lower = minDen > 0n;
      const upper = maxNum > 0n && maxDen > 0n;
      const ordered = !(lower && upper) || minNum * maxDen <= maxNum * minDen;
      return (mode & ~3) === 0 && mode !== 0 && (lower || upper) && (maxNum === 0n) === (maxDen === 0n) && ordered;
    }
    case 13:
      return c[0] <= 1 && (c[1] & ~1) === 0 && !zero(c.slice(8, 40)) && zero(c.slice(40, 72));
    case 14:
      return u32(0) > 0 && (c[4] & ~1) === 0;
    default:
      return false;
  }
}

/** Validates a full (1736-byte) or compact stack. `members`: the environment's member count (multisig). */
export function validateStack(bytes: Uint8Array, members = 0): "ok" | StackError {
  if (bytes[0] !== ABI_VERSION) return "Version";
  const env = bytes[1];
  if (env > 3) return "Environment";
  const len = bytes[2];
  if (len > v1.MAX_STACK) return "TooLong";
  if (bytes.length < v1.STACK_HEADER_LEN + len * v1.ENTRY_LEN) return "TooLong";
  const mask = callbackMask(env);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const at = (i: number) => v1.STACK_HEADER_LEN + i * v1.ENTRY_LEN;
  for (let i = 0; i < len; i++) {
    const o = at(i);
    const e = bytes.slice(o, o + v1.ENTRY_LEN);
    const [kind, builtin, mandatory, deps] = [e[64], e[65], e[66], e[67]];
    const callbacks = view.getUint16(o + 68, true);
    const pad = view.getUint16(o + 70, true);
    const granted = view.getUint32(o + 72, true);
    const reserved = view.getUint32(o + 76, true);
    const pinned = view.getBigUint64(o + 80, true);
    const version = e.slice(0, 32);
    const program = e.slice(32, 64);
    if (callbacks === 0 || (callbacks & ~mask) !== 0) return `Callbacks(${i})`;
    if (deps >> i !== 0) return `Dependency(${i})`;
    if (!zero(version)) {
      for (let j = 0; j < i; j++) if (eq(bytes.slice(at(j), at(j) + 32), version)) return `Duplicate(${i})`;
    }
    if (pad !== 0 || reserved !== 0 || mandatory > 1) return `Config(${i})`;
    if (kind === v1.IMPL_BUILTIN) {
      if (!v1.BUILTIN_SLUGS[builtin]) return `UnknownImpl(${i})`;
      if (!zero(program) || pinned !== 0n) return `Program(${i})`;
      if ((callbacks & ~v1.supportedCallbacks(builtin, env as v1.EnvironmentId)) !== 0) return `Callbacks(${i})`;
      if (granted >>> 0 !== v1.requiredCaps(builtin) >>> 0) return `Capabilities(${i})`;
      if (!validateConfig(builtin, e.slice(88, 184), members)) return `Config(${i})`;
    } else if (kind === v1.IMPL_PROGRAM) {
      if (builtin !== 0 || env === v1.Environment.Pool) return `UnknownImpl(${i})`;
      if (zero(program)) return `Program(${i})`;
      if ((granted & ~POLICY_PROGRAM_CAPS) !== 0 || (granted & (Cap.VALIDATE_OPERATION | Cap.VETO_OPERATION)) === 0) return `Capabilities(${i})`;
    } else {
      return `UnknownImpl(${i})`;
    }
  }
  for (let c = 0; c < 16; c++) {
    let n = 0;
    for (let i = 0; i < len; i++) if (bytes[at(i) + 64] === v1.IMPL_PROGRAM && (view.getUint16(at(i) + 68, true) >> c) & 1) n++;
    if (n > MAX_PROGRAM_HOOKS_PER_CALLBACK) return `TooManyProgramHooks(${c})`;
  }
  if (bytes.length >= v1.STACK_HEADER_LEN + v1.MAX_STACK * v1.ENTRY_LEN) {
    for (let i = len; i < v1.MAX_STACK; i++) if (!zero(bytes.slice(at(i), at(i) + v1.ENTRY_LEN))) return `DirtySlot(${i})`;
  }
  return "ok";
}
