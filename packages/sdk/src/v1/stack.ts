// ABI v1 hook stacks (crates/hook-standard, crates/hook-kernel). Byte-for-byte mirrors; checked
// against the kernel by test/vectors/kernel.json.

export const Environment = { Token: 0, Router: 1, Pool: 2, Account: 3 } as const;
export type EnvironmentId = (typeof Environment)[keyof typeof Environment];

export const ENVIRONMENT_NAMES = ["token", "router", "pool", "account"] as const;

export const CALLBACK_NAMES: Record<EnvironmentId, readonly string[]> = {
  0: ["before_transfer", "after_transfer", "before_wrap", "before_unwrap"],
  1: ["before_execute", "after_execute", "before_swap", "after_swap"],
  2: [
    "before_initialize", "after_initialize", "before_add_liquidity", "after_add_liquidity", "before_remove_liquidity",
    "after_remove_liquidity", "before_swap", "after_swap", "before_donate", "after_donate", "quote",
  ],
  3: [
    "before_deposit", "after_deposit", "before_withdraw", "after_withdraw", "before_execute", "after_execute",
    "before_authority_change", "after_authority_change",
  ],
};

export const Builtin = {
  Allowlist: 1, SpendingLimit: 2, Timelock: 3, Multisig: 4, Conditional: 5, MaxSize: 6, TimeWindow: 7, Accounting: 8,
  Denylist: 9, DynamicFee: 10, PriceImpact: 11, PriceLimit: 12, Credential: 13, SlotLimit: 14,
} as const;

export const BUILTIN_SLUGS: Record<number, string> = {
  1: "allowlist", 2: "spending-limit", 3: "timelock", 4: "multisig", 5: "conditional", 6: "max-size", 7: "time-window",
  8: "accounting", 9: "denylist", 10: "dynamic-fee", 11: "price-impact", 12: "price-limit", 13: "credential", 14: "slot-limit",
};

export const Cap = {
  READ_ACCOUNT_STATE: 1 << 0, READ_TOKEN_BALANCE: 1 << 1, VALIDATE_OPERATION: 1 << 2, VETO_OPERATION: 1 << 3,
  UPDATE_OWN_STATE: 1 << 4, REQUEST_AUTHORIZED_TRANSFER: 1 << 5, REQUEST_AUTHORIZED_SWAP: 1 << 6,
  REQUEST_AUTHORIZED_FEE_ALLOCATION: 1 << 7, CUSTOM_PRICING: 1 << 8, PRODUCE_STATE: 1 << 9, CONSUME_STATE: 1 << 10,
} as const;

const VETO = Cap.VALIDATE_OPERATION | Cap.VETO_OPERATION;

/** `builtin::required_caps`: installs must grant exactly these. */
export function requiredCaps(id: number): number {
  switch (id) {
    case 1: case 13: return VETO | Cap.READ_ACCOUNT_STATE | Cap.CONSUME_STATE;
    case 2: case 14: return VETO | Cap.UPDATE_OWN_STATE;
    case 3: case 4: case 6: case 7: case 9: return VETO;
    case 5: case 11: case 12: return VETO | Cap.READ_ACCOUNT_STATE;
    case 8: return Cap.UPDATE_OWN_STATE;
    case 10: return Cap.CUSTOM_PRICING | Cap.UPDATE_OWN_STATE;
    default: return 0xffffffff;
  }
}

/** `builtin::compute_estimate` (static; see docs/benchmarks/report.md for measured costs). */
export function computeEstimate(id: number): number {
  if (id === 1) return 2500;
  if (id === 5) return 800;
  if (id === 13) return 1500;
  if (id === 10 || id === 11 || id === 12) return 600;
  return 300;
}

const ENV_MASK: Record<EnvironmentId, number> = { 0: 0b1111, 1: 0b1111, 2: (1 << 11) - 1, 3: 0xff };
const ODD_AFTER: Record<EnvironmentId, number> = { 0: 0b0010, 1: 0b1010, 2: 0, 3: 0b1010_1010 };
const POOL_ENTRY = (1 << 2) | (1 << 6) | (1 << 8);
const POOL_AFTER = (1 << 3) | (1 << 5) | (1 << 7) | (1 << 9);

/** `builtin::supported_callbacks`: the callback mask a builtin may run on in an environment. */
export function supportedCallbacks(id: number, env: EnvironmentId): number {
  const beforeAccount = 0b0101_0101;
  const tokenGates = 0b1101;
  const [T, R, P, A] = [0, 1, 2, 3];
  const gate = [1, 9, 13];
  if ([1, 5, 7, 9, 13].includes(id) && env === P) return POOL_ENTRY;
  if (id === 2 && env === P) return 1 << 6;
  if (id === 6 && env === P) return (1 << 2) | (1 << 6);
  if (id === 8 && env === P) return POOL_AFTER;
  if (gate.includes(id) && env === A) return beforeAccount;
  if (gate.includes(id) && env === R) return 0b0101;
  if (gate.includes(id) && env === T) return tokenGates;
  if (id === 2) return env === A ? (1 << 2) | (1 << 5) : env === R ? 1 << 2 : 1 << 0;
  if (id === 3) return env === A ? (1 << 2) | (1 << 4) | (1 << 6) : env === T ? 0b1001 : 0;
  if (id === 4) return env === A ? (1 << 2) | (1 << 4) | (1 << 6) : 0;
  if (id === 5) return env === A ? beforeAccount | (1 << 5) : env === R ? 0b0101 : tokenGates;
  if (id === 6) return env === A ? (1 << 2) | (1 << 5) : env === R ? 0b0101 : tokenGates;
  if (id === 7) return ENV_MASK[env] & ~ODD_AFTER[env];
  if (id === 8) return env === T ? tokenGates : ENV_MASK[env];
  if ((id === 10 || id === 11) && env === P) return 1 << 6;
  if (id === 12) return env === P ? 1 << 7 : env === R ? 1 << 3 : 0;
  if (id === 14) return env === P ? (1 << 2) | (1 << 6) : env === R ? 1 << 2 : 0;
  return 0;
}

export const STACK_HEADER_LEN = 8;
export const ENTRY_LEN = 216;
export const CONFIG_LEN = 96;
export const MAX_STACK = 8;
export const IMPL_BUILTIN = 0;
export const IMPL_PROGRAM = 1;

export const Target = { Actor: 0, Counterparty: 1 } as const;

export interface EntrySpec {
  builtin: number;
  /** Callback ids (bit positions); defaults to every callback the builtin supports in `env`. */
  callbacks?: number[];
  mandatory?: boolean;
  dependsOn?: number;
  /** Registry `HookVersion` address bytes; defaults to 32 bytes of the builtin id (tests). */
  hookVersion?: Uint8Array;
  config?: Uint8Array;
}

class W {
  readonly b: Uint8Array;
  private v: DataView;
  constructor(n: number) {
    this.b = new Uint8Array(n);
    this.v = new DataView(this.b.buffer);
  }
  u8(o: number, x: number) { this.b[o] = x; return this; }
  u16(o: number, x: number) { this.v.setUint16(o, x, true); return this; }
  u32(o: number, x: number) { this.v.setUint32(o, x >>> 0, true); return this; }
  u64(o: number, x: bigint) { this.v.setBigUint64(o, x, true); return this; }
  i64(o: number, x: bigint) { this.v.setBigInt64(o, x, true); return this; }
  key(o: number, k: Uint8Array) {
    if (k.length !== 32) throw new Error("key must be 32 bytes");
    this.b.set(k, o);
    return this;
  }
}

/** Config encoders for the builtins usable in token stacks (and elsewhere). Layouts: `builtin.rs`. */
export const configs = {
  allowlistInline: (target: number, keys: Uint8Array[]) => {
    if (keys.length < 1 || keys.length > 2) throw new Error("inline allowlist holds 1 or 2 keys");
    const w = new W(CONFIG_LEN).u8(0, target).u8(1, 0);
    keys.forEach((k, i) => w.key(16 + 32 * i, k));
    return w.b;
  },
  denylist: (target: number, keys: Uint8Array[]) => {
    if (keys.length < 1 || keys.length > 2) throw new Error("denylist holds 1 or 2 keys");
    const w = new W(CONFIG_LEN).u8(0, target);
    keys.forEach((k, i) => w.key(8 + 32 * i, k));
    return w.b;
  },
  maxSize: (asset: Uint8Array, maxAmount: bigint) => new W(CONFIG_LEN).key(0, asset).u64(32, maxAmount).b,
  spendingLimit: (asset: Uint8Array, limit: bigint, periodSecs: number) => new W(CONFIG_LEN).key(0, asset).u64(32, limit).u32(40, periodSecs).b,
  timeWindow: (o: { notBefore?: bigint; notAfter?: bigint; dailyStart?: number; dailyEnd?: number }) =>
    new W(CONFIG_LEN).i64(0, o.notBefore ?? 0n).i64(8, o.notAfter ?? 0n).u32(16, o.dailyStart ?? 0).u32(20, o.dailyEnd ?? 0).b,
  timelock: (delaySecs: number, notBefore: bigint) => new W(CONFIG_LEN).u32(0, delaySecs).i64(8, notBefore).b,
  accounting: () => new Uint8Array(CONFIG_LEN),
  credential: (target: number, gatekeeperNetwork: Uint8Array) => new W(CONFIG_LEN).u8(0, target).key(8, gatekeeperNetwork).b,
  slotLimit: (maxPerSlot: number) => new W(CONFIG_LEN).u32(0, maxPerSlot).b,
  multisig: (threshold: number, memberMask: number) => new W(CONFIG_LEN).u8(0, threshold).u8(1, memberMask).b,
  priceImpact: (maxBps: number) => new W(CONFIG_LEN).u16(0, maxBps).b,
  /** Size mode: `baseBps` for small swaps rising to `maxBps` at `pivot` input; the pool's own max fee still caps it. */
  dynamicFee: (o: { mode?: number; baseBps: number; maxBps: number; pivot: bigint; alphaBps?: number; sensitivityBps?: number }) =>
    new W(CONFIG_LEN).u8(0, o.mode ?? 0).u16(4, o.baseBps).u16(6, o.maxBps).u64(8, o.pivot).u16(16, o.alphaBps ?? 0).u32(20, o.sensitivityBps ?? 0).b,
};

export function encodeEntry(env: EnvironmentId, spec: EntrySpec): Uint8Array {
  const supported = supportedCallbacks(spec.builtin, env);
  const mask = spec.callbacks ? spec.callbacks.reduce((m, c) => m | (1 << c), 0) : supported;
  if (mask === 0 || (mask & ~supported) !== 0) {
    throw new Error(`${BUILTIN_SLUGS[spec.builtin] ?? spec.builtin} cannot run on callbacks 0b${mask.toString(2)} in ${ENVIRONMENT_NAMES[env]}`);
  }
  const w = new W(ENTRY_LEN);
  w.b.set(spec.hookVersion ?? new Uint8Array(32).fill(spec.builtin), 0);
  w.u8(64, IMPL_BUILTIN).u8(65, spec.builtin).u8(66, spec.mandatory ? 1 : 0).u8(67, spec.dependsOn ?? 0);
  w.u16(68, mask).u32(72, requiredCaps(spec.builtin));
  const cfg = spec.config ?? new Uint8Array(CONFIG_LEN);
  if (cfg.length > CONFIG_LEN) throw new Error("config too long");
  w.b.set(cfg, 88);
  return w.b;
}

/** A program-hook entry (an `IMPL_PROGRAM` registry version). */
export interface ProgramEntrySpec {
  hookVersion: Uint8Array;
  program: Uint8Array;
  callbacks: number[];
  granted: number;
  mandatory?: boolean;
  dependsOn?: number;
  /** The version's observed deploy slot, or 0 to accept upgrades. */
  pinnedSlot?: bigint;
  config?: Uint8Array;
}

export function encodeProgramEntry(env: EnvironmentId, spec: ProgramEntrySpec): Uint8Array {
  const mask = spec.callbacks.reduce((m, c) => m | (1 << c), 0);
  const count = CALLBACK_NAMES[env].length;
  if (mask === 0 || mask >> count !== 0) throw new Error(`invalid callbacks for ${ENVIRONMENT_NAMES[env]}`);
  const w = new W(ENTRY_LEN);
  w.b.set(spec.hookVersion, 0);
  w.b.set(spec.program, 32);
  w.u8(64, IMPL_PROGRAM).u8(65, 0).u8(66, spec.mandatory ? 1 : 0).u8(67, spec.dependsOn ?? 0);
  w.u16(68, mask).u32(72, spec.granted).u64(80, spec.pinnedSlot ?? 0n);
  const cfg = spec.config ?? new Uint8Array(CONFIG_LEN);
  if (cfg.length > CONFIG_LEN) throw new Error("config too long");
  w.b.set(cfg, 88);
  return w.b;
}

/** Compact stack bytes (header + entries), as accepted by `create_policy` / `update_policy`. */
export function encodeStack(env: EnvironmentId, entries: (EntrySpec | Uint8Array)[]): Uint8Array {
  if (entries.length > MAX_STACK) throw new Error(`at most ${MAX_STACK} entries`);
  const out = new Uint8Array(STACK_HEADER_LEN + entries.length * ENTRY_LEN);
  out[0] = 1;
  out[1] = env;
  out[2] = entries.length;
  entries.forEach((e, i) => out.set(e instanceof Uint8Array ? e : encodeEntry(env, e), STACK_HEADER_LEN + i * ENTRY_LEN));
  return out;
}

export interface DecodedEntry {
  index: number;
  hookVersion: Uint8Array;
  program: Uint8Array;
  kind: "builtin" | "program";
  builtinId: number;
  slug: string;
  mandatory: boolean;
  dependsOn: number;
  callbacks: number;
  granted: number;
  pinnedSlot: bigint;
  config: Uint8Array;
  state: Uint8Array;
}

export interface DecodedStack {
  version: number;
  environment: EnvironmentId;
  entries: DecodedEntry[];
}

export function decodeStack(bytes: Uint8Array): DecodedStack {
  if (bytes.length < STACK_HEADER_LEN) throw new Error("stack too short");
  const env = bytes[1];
  if (env > 3) throw new Error(`unknown environment ${env}`);
  const len = bytes[2];
  if (len > MAX_STACK || bytes.length < STACK_HEADER_LEN + len * ENTRY_LEN) throw new Error("bad stack length");
  const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const entries: DecodedEntry[] = [];
  for (let i = 0; i < len; i++) {
    const o = STACK_HEADER_LEN + i * ENTRY_LEN;
    const kind = bytes[o + 64] === IMPL_BUILTIN ? "builtin" : "program";
    entries.push({
      index: i,
      hookVersion: bytes.slice(o, o + 32),
      program: bytes.slice(o + 32, o + 64),
      kind,
      builtinId: bytes[o + 65],
      slug: kind === "builtin" ? (BUILTIN_SLUGS[bytes[o + 65]] ?? `builtin-${bytes[o + 65]}`) : "program-hook",
      mandatory: bytes[o + 66] !== 0,
      dependsOn: bytes[o + 67],
      callbacks: v.getUint16(o + 68, true),
      granted: v.getUint32(o + 72, true),
      pinnedSlot: v.getBigUint64(o + 80, true),
      config: bytes.slice(o + 88, o + 184),
      state: bytes.slice(o + 184, o + 216),
    });
  }
  return { version: bytes[0], environment: env as EnvironmentId, entries };
}
