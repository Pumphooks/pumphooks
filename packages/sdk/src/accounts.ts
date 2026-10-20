/** Decoders for Pump Hooks program accounts. Offsets follow `programs/pump-hooks/src/state.rs`. */
import { Buffer } from "buffer";
import { PublicKey } from "@solana/web3.js";
import { sha256 } from "@noble/hashes/sha256";
import { Reader } from "./codec.js";
import { MAX_HOOK_SLOTS, ModuleId, MODULES, ModuleInfo, Venue } from "./constants.js";
import { decodeConfig, ModuleConfig } from "./configs.js";

export const accountDiscriminator = (name: string) => sha256(Buffer.from(`account:${name}`)).subarray(0, 8);

const DISC = {
  GlobalConfig: accountDiscriminator("GlobalConfig"),
  HookDefinition: accountDiscriminator("HookDefinition"),
  MarketConfig: accountDiscriminator("MarketConfig"),
  UserMarketState: accountDiscriminator("UserMarketState"),
  Policy: accountDiscriminator("Policy"),
};

function body(name: keyof typeof DISC, d: Uint8Array): Reader {
  const disc = DISC[name];
  if (d.length < 8 || !disc.every((x, i) => d[i] === x)) throw new Error(`account is not a ${name}`);
  return new Reader(d, 8);
}

export const MARKET_CONFIG_SIZE = 8 + 1176;
/** Byte offset of `MarketConfig.mint` in the account (for getProgramAccounts filters). */
export const MARKET_MINT_OFFSET = 8 + 96;

export interface GlobalConfig {
  authority: PublicKey;
  pendingAuthority: PublicKey;
  disabledModules: bigint;
  marketCount: bigint;
  paused: boolean;
}

export function decodeGlobalConfig(d: Uint8Array): GlobalConfig {
  const r = body("GlobalConfig", d);
  return { authority: r.pubkey(), pendingAuthority: r.pubkey(), disabledModules: r.u64(), marketCount: r.u64(), paused: r.bool() };
}

export interface HookDefinition {
  moduleId: number;
  version: number;
  capabilities: number;
  callbacks: number;
  priorityMin: number;
  priorityMax: number;
  runtime: boolean;
  enabled: boolean;
  installs: bigint;
  sourceHash: Uint8Array;
  /** 0 = unaudited. Nothing in this repository sets a non-zero value. */
  auditStatus: number;
}

export function decodeHookDefinition(d: Uint8Array): HookDefinition {
  const r = body("HookDefinition", d);
  return {
    moduleId: r.u16(),
    version: r.u16(),
    capabilities: r.u32(),
    callbacks: r.u16(),
    priorityMin: r.u16(),
    priorityMax: r.u16(),
    runtime: r.bool(),
    enabled: r.bool(),
    installs: r.u64(),
    sourceHash: r.bytes(32),
    auditStatus: r.u8(),
  };
}

export interface HookSlot {
  index: number;
  moduleId: number;
  module?: ModuleInfo;
  priority: number;
  callbacks: number;
  flags: number;
  enabled: boolean;
  capabilities: number;
  version: number;
  config: Uint8Array;
  decodedConfig: ModuleConfig;
  state: Uint8Array;
}

export interface MarketConfig {
  namespace: PublicKey;
  authority: PublicKey;
  pendingAuthority: PublicKey;
  mint: PublicKey;
  bondingCurve: PublicKey;
  pool: PublicKey;
  quoteMint: PublicKey;
  baseTokenProgram: PublicKey;
  quoteTokenProgram: PublicKey;
  strategyVault: PublicKey;
  createdAt: bigint;
  activatedAt: bigint;
  routedBuyQuote: bigint;
  routedSellQuote: bigint;
  tradeCount: bigint;
  sequence: bigint;
  flags: number;
  venue: Venue;
  policyMask: number;
  /** Installed hooks in execution (priority) order. */
  slots: HookSlot[];
}

export function decodeMarketConfig(d: Uint8Array): MarketConfig {
  const r = body("MarketConfig", d);
  const [namespace, authority, pendingAuthority, mint, bondingCurve, pool, quoteMint, baseTokenProgram, quoteTokenProgram, strategyVault] =
    Array.from({ length: 10 }, () => r.pubkey());
  const createdAt = r.i64();
  const activatedAt = r.i64();
  const routedBuyQuote = r.u64();
  const routedSellQuote = r.u64();
  const tradeCount = r.u64();
  const sequence = r.u64();
  const flags = r.u32();
  const venue = r.u8() as Venue;
  r.u8(); // bump
  const policyMask = r.u8();
  r.u8();
  const slots: HookSlot[] = [];
  for (let index = 0; index < MAX_HOOK_SLOTS; index++) {
    const moduleId = r.u16();
    const priority = r.u16();
    const callbacks = r.u16();
    const slotFlags = r.u16();
    const capabilities = r.u32();
    const version = r.u16();
    r.skip(2);
    const config = r.bytes(48);
    const state = r.bytes(48);
    if (moduleId === 0) continue;
    slots.push({
      index, moduleId, module: MODULES.find((x) => x.id === moduleId), priority, callbacks, flags: slotFlags,
      enabled: (slotFlags & 1) !== 0, capabilities, version, config,
      decodedConfig: decodeConfig(moduleId as ModuleId, config), state,
    });
  }
  return {
    namespace, authority, pendingAuthority, mint, bondingCurve, pool, quoteMint, baseTokenProgram, quoteTokenProgram,
    strategyVault, createdAt, activatedAt, routedBuyQuote, routedSellQuote, tradeCount, sequence, flags, venue,
    policyMask, slots,
  };
}

export interface UserState {
  xp: bigint;
  pendingXp: bigint;
  pendingVestAt: bigint;
  xpDay: number;
  xpToday: number;
  policyPoints: bigint;
  routedBuyQuote: bigint;
  routedSellQuote: bigint;
  tradeCount: number;
  lastSide: number;
  lastTradeTs: bigint;
  lastTradeSlot: bigint;
  firstBuyTs: bigint;
  holdCheckpointTs: bigint;
  holdCheckpointBalance: bigint;
  vgWindowStart: bigint;
  vgWindowVolume: bigint;
  forfeitedXp: bigint;
}

export interface UserMarketState {
  market: PublicKey;
  user: PublicKey;
  createdAt: bigint;
  state: UserState;
}

export function decodeUserMarketState(d: Uint8Array): UserMarketState {
  const r = body("UserMarketState", d);
  const market = r.pubkey();
  const user = r.pubkey();
  const createdAt = r.i64();
  r.skip(8);
  const s: UserState = {
    xp: r.u64(),
    pendingXp: r.u64(),
    pendingVestAt: r.i64(),
    xpDay: r.u32(),
    xpToday: r.u32(),
    policyPoints: r.u64(),
    routedBuyQuote: r.u64(),
    routedSellQuote: r.u64(),
    tradeCount: r.u32(),
    lastSide: (() => { const v = r.u8(); r.skip(3); return v; })(),
    lastTradeTs: r.i64(),
    lastTradeSlot: r.u64(),
    firstBuyTs: r.i64(),
    holdCheckpointTs: r.i64(),
    holdCheckpointBalance: r.u64(),
    vgWindowStart: r.i64(),
    vgWindowVolume: r.u64(),
    forfeitedXp: r.u64(),
  };
  return { market, user, createdAt, state: s };
}

export interface PolicyCondition {
  field: number;
  op: number;
  arg: number;
  value: bigint;
}
export interface PolicyAction {
  kind: number;
  arg: number;
  arg16: number;
  value: bigint;
}

export interface PolicyAccount {
  market: PublicKey;
  index: number;
  version: number;
  trigger: number;
  combinator: number;
  enabled: boolean;
  cooldownSecs: number;
  denyReason: number;
  conditions: PolicyCondition[];
  actions: PolicyAction[];
  registers: bigint[];
  execCount: bigint;
  lastExecTs: bigint;
  intent: { kind: number; value: bigint; createdAt: bigint; sequence: bigint };
  labelHash: Uint8Array;
}

export function decodePolicy(d: Uint8Array): PolicyAccount {
  const r = body("Policy", d);
  const market = r.pubkey();
  const index = r.u8();
  const version = r.u8();
  const trigger = r.u8();
  const combinator = r.u8();
  const nc = r.u8();
  const na = r.u8();
  const enabled = r.bool();
  r.u8();
  const cooldownSecs = r.u32();
  const denyReason = r.u16();
  r.skip(2);
  const conditions = Array.from({ length: 8 }, () => {
    const c = { field: r.u8(), op: r.u8(), arg: r.u8(), value: 0n };
    r.skip(5);
    c.value = r.u64();
    return c;
  }).slice(0, nc);
  const actions = Array.from({ length: 4 }, () => {
    const a = { kind: r.u8(), arg: r.u8(), arg16: r.u16(), value: 0n };
    r.skip(4);
    a.value = r.u64();
    return a;
  }).slice(0, na);
  const registers = Array.from({ length: 4 }, () => r.u64());
  const execCount = r.u64();
  const lastExecTs = r.i64();
  const kind = r.u8();
  r.skip(7);
  const intent = { kind, value: r.u64(), createdAt: r.i64(), sequence: r.u64() };
  return {
    market, index, version, trigger, combinator, enabled, cooldownSecs, denyReason,
    conditions, actions, registers, execCount, lastExecTs, intent, labelHash: r.bytes(32),
  };
}
