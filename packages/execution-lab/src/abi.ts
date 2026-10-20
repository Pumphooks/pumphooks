// ABI v1 layouts (crates/hook-standard) needed to decode installed hook stacks.

export const ENVIRONMENTS = ["token", "router", "pool", "account"] as const;
export type EnvironmentName = (typeof ENVIRONMENTS)[number];

export const CALLBACKS: Record<EnvironmentName, readonly string[]> = {
  token: ["before_transfer", "after_transfer", "before_wrap", "before_unwrap"],
  router: ["before_execute", "after_execute", "before_swap", "after_swap"],
  pool: [
    "before_initialize",
    "after_initialize",
    "before_add_liquidity",
    "after_add_liquidity",
    "before_remove_liquidity",
    "after_remove_liquidity",
    "before_swap",
    "after_swap",
    "before_donate",
    "after_donate",
    "quote",
  ],
  account: [
    "before_deposit",
    "after_deposit",
    "before_withdraw",
    "after_withdraw",
    "before_execute",
    "after_execute",
    "before_authority_change",
    "after_authority_change",
  ],
};

/** Builtin ids (crates/hook-kernel/src/builtin.rs). */
export const BUILTINS: Record<number, string> = {
  1: "allowlist",
  2: "spending-limit",
  3: "timelock",
  4: "multisig",
  5: "conditional",
  6: "max-size",
  7: "time-window",
  8: "accounting",
  9: "denylist",
  10: "dynamic-fee",
  11: "price-impact",
  12: "price-limit",
  13: "credential",
  14: "slot-limit",
};

export const IMPL_BUILTIN = 0;
export const IMPL_PROGRAM = 1;
export const STACK_HEADER_LEN = 8;
export const ENTRY_LEN = 216;
export const MAX_STACK = 8;
export const STACK_LEN = STACK_HEADER_LEN + MAX_STACK * ENTRY_LEN;

export interface DecodedEntry {
  index: number;
  hookVersion: Uint8Array;
  program: Uint8Array;
  kind: "builtin" | "program";
  builtinId: number;
  name: string;
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
  environment: EnvironmentName;
  entries: DecodedEntry[];
}

/** Decodes a full or compact `HookStack`. */
export function decodeStack(bytes: Uint8Array): DecodedStack {
  if (bytes.length < STACK_HEADER_LEN) throw new Error("stack too short");
  const env = ENVIRONMENTS[bytes[1]];
  if (!env) throw new Error(`unknown environment ${bytes[1]}`);
  const len = bytes[2];
  if (len > MAX_STACK || bytes.length < STACK_HEADER_LEN + len * ENTRY_LEN) throw new Error("bad stack length");
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const entries: DecodedEntry[] = [];
  for (let i = 0; i < len; i++) {
    const o = STACK_HEADER_LEN + i * ENTRY_LEN;
    const kind = bytes[o + 64] === IMPL_BUILTIN ? "builtin" : "program";
    const builtinId = bytes[o + 65];
    entries.push({
      index: i,
      hookVersion: bytes.slice(o, o + 32),
      program: bytes.slice(o + 32, o + 64),
      kind,
      builtinId,
      name: kind === "builtin" ? (BUILTINS[builtinId] ?? `builtin-${builtinId}`) : "program-hook",
      mandatory: bytes[o + 66] !== 0,
      dependsOn: bytes[o + 67],
      callbacks: view.getUint16(o + 68, true),
      granted: view.getUint32(o + 72, true),
      pinnedSlot: view.getBigUint64(o + 80, true),
      config: bytes.slice(o + 88, o + 184),
      state: bytes.slice(o + 184, o + 216),
    });
  }
  return { version: bytes[0], environment: env, entries };
}

/** Entries that run on `callback`, in the kernel's evaluation order (index order). */
export function entriesOn(stack: DecodedStack, callback: number): DecodedEntry[] {
  return stack.entries.filter((e) => (e.callbacks & (1 << callback)) !== 0);
}

/** Byte offset of the `HookStack` inside each engine's policy account (8-byte discriminator included). */
export const POLICY_STACK_OFFSET = {
  /** token-hooks `TokenPolicy`: mint, authority, pending_hash, sequence, delay, pending_at, bump, pad. */
  tokenPolicy: 8 + 32 + 32 + 32 + 8 + 8 + 8 + 8,
  /** hook-router `RoutePolicy`: authority, pending_hash, seed, sequence, delay, pending_at, bump, shared, pad. */
  routePolicy: 8 + 32 + 32 + 8 + 8 + 8 + 8 + 8,
} as const;
