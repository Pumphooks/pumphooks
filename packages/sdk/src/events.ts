/**
 * Event and error decoding from transaction logs. Events are Anchor `emit!` records
 * (`Program data: base64(disc ‖ borsh)`); only lines emitted while the Pump Hooks program is the
 * executing program are decoded, so Pump's own events are never misread as ours.
 */
import { Buffer } from "buffer";
import { PublicKey } from "@solana/web3.js";
import { sha256 } from "@noble/hashes/sha256";
import { Reader } from "./codec.js";
import { Callback, describeReason, PUMP_HOOKS_PROGRAM_ID } from "./constants.js";
import { PROGRAM_ERRORS } from "./generated/errors.js";

export const eventDiscriminator = (name: string) => sha256(Buffer.from(`event:${name}`)).subarray(0, 8);

export interface TraceItem {
  callback: Callback;
  slotIndex: number;
  moduleId: number;
  allowed: boolean;
  reasonCode: number;
  flags: number;
}

export interface HookExecutionEvent {
  market: PublicKey;
  user: PublicKey;
  sequence: bigint;
  entries: TraceItem[];
}

export interface HookDeniedEvent {
  market: PublicKey;
  user: PublicKey;
  sequence: bigint;
  moduleId: number;
  callback: Callback;
  slotIndex: number;
  reasonCode: number;
}

export interface RoutedSwapEvent {
  market: PublicKey;
  user: PublicKey;
  mint: PublicKey;
  venue: number;
  side: number;
  mode: number;
  amount: bigint;
  limit: bigint;
  /** Observed from the wallet's balances (net of rent Pump charged inside the trade). */
  quoteAmount: bigint;
  baseAmount: bigint;
  preBaseReserves: bigint;
  preQuoteReserves: bigint;
  postBaseReserves: bigint;
  postQuoteReserves: bigint;
  sequence: bigint;
  slot: bigint;
  timestamp: bigint;
  hooksRun: number;
  xp: bigint;
  pendingXp: bigint;
  policyPoints: bigint;
}

export interface PolicyExecutionEvent {
  market: PublicKey;
  sequence: bigint;
  policyIndex: number;
  trigger: number;
  matched: boolean;
  /** 0 allow, 1 deny, 2 record, 3 signal/intent. */
  result: number;
  reasonCode: number;
  tag: bigint;
}

export interface StrategySignalApplied {
  market: PublicKey;
  signer: PublicKey;
  nonce: bigint;
  action: number;
  param: number;
  validUntil: bigint;
  confidenceBps: number;
}

type Decoders = {
  MarketCreated: { market: PublicKey; mint: PublicKey; authority: PublicKey; venue: number; creatorVerified: boolean; bondingCurve: PublicKey; pool: PublicKey; quoteMint: PublicKey };
  MarketSynced: { market: PublicKey; venue: number; pool: PublicKey };
  MarketPausedSet: { market: PublicKey; paused: boolean };
  HookInstalled: { market: PublicKey; moduleId: number; version: number; slotIndex: number; priority: number; callbacks: number; capabilities: number; config: Uint8Array };
  HookRemoved: { market: PublicKey; moduleId: number };
  HookConfigured: { market: PublicKey; moduleId: number; enabled: boolean; config: Uint8Array };
  PolicyUpdated: { market: PublicKey; index: number; trigger: number; nConditions: number; nActions: number; enabled: boolean; closed: boolean };
  HookExecutionEvent: HookExecutionEvent;
  HookDeniedEvent: HookDeniedEvent;
  RoutedSwapEvent: RoutedSwapEvent;
  PolicyExecutionEvent: PolicyExecutionEvent;
  StrategySignalApplied: StrategySignalApplied;
  KillSwitchSet: { disabledModules: bigint; paused: boolean };
};
export type EventName = keyof Decoders;
export type PumpHooksEvent = { [K in EventName]: { name: K; data: Decoders[K] } }[EventName];

const DECODE: { [K in EventName]: (r: Reader) => Decoders[K] } = {
  MarketCreated: (r) => ({ market: r.pubkey(), mint: r.pubkey(), authority: r.pubkey(), venue: r.u8(), creatorVerified: r.bool(), bondingCurve: r.pubkey(), pool: r.pubkey(), quoteMint: r.pubkey() }),
  MarketSynced: (r) => ({ market: r.pubkey(), venue: r.u8(), pool: r.pubkey() }),
  MarketPausedSet: (r) => ({ market: r.pubkey(), paused: r.bool() }),
  HookInstalled: (r) => ({ market: r.pubkey(), moduleId: r.u16(), version: r.u16(), slotIndex: r.u8(), priority: r.u16(), callbacks: r.u16(), capabilities: r.u32(), config: r.bytes(48) }),
  HookRemoved: (r) => ({ market: r.pubkey(), moduleId: r.u16() }),
  HookConfigured: (r) => ({ market: r.pubkey(), moduleId: r.u16(), enabled: r.bool(), config: r.bytes(48) }),
  PolicyUpdated: (r) => ({ market: r.pubkey(), index: r.u8(), trigger: r.u8(), nConditions: r.u8(), nActions: r.u8(), enabled: r.bool(), closed: r.bool() }),
  HookExecutionEvent: (r) => ({
    market: r.pubkey(),
    user: r.pubkey(),
    sequence: r.u64(),
    entries: r.vec((r) => ({ callback: r.u8() as Callback, slotIndex: r.u8(), moduleId: r.u16(), allowed: r.bool(), reasonCode: r.u16(), flags: r.u32() })),
  }),
  HookDeniedEvent: (r) => ({ market: r.pubkey(), user: r.pubkey(), sequence: r.u64(), moduleId: r.u16(), callback: r.u8() as Callback, slotIndex: r.u8(), reasonCode: r.u16() }),
  RoutedSwapEvent: (r) => ({
    market: r.pubkey(), user: r.pubkey(), mint: r.pubkey(), venue: r.u8(), side: r.u8(), mode: r.u8(),
    amount: r.u64(), limit: r.u64(), quoteAmount: r.u64(), baseAmount: r.u64(),
    preBaseReserves: r.u64(), preQuoteReserves: r.u64(), postBaseReserves: r.u64(), postQuoteReserves: r.u64(),
    sequence: r.u64(), slot: r.u64(), timestamp: r.i64(), hooksRun: r.u8(), xp: r.u64(), pendingXp: r.u64(), policyPoints: r.u64(),
  }),
  PolicyExecutionEvent: (r) => ({ market: r.pubkey(), sequence: r.u64(), policyIndex: r.u8(), trigger: r.u8(), matched: r.bool(), result: r.u8(), reasonCode: r.u16(), tag: r.u64() }),
  StrategySignalApplied: (r) => ({ market: r.pubkey(), signer: r.pubkey(), nonce: r.u64(), action: r.u8(), param: r.u8(), validUntil: r.i64(), confidenceBps: r.u16() }),
  KillSwitchSet: (r) => ({ disabledModules: r.u64(), paused: r.bool() }),
};

const BY_DISC = new Map<string, EventName>(
  (Object.keys(DECODE) as EventName[]).map((n) => [Buffer.from(eventDiscriminator(n)).toString("hex"), n]),
);

const INVOKE = /^Program (\w+) invoke \[\d+\]$/;
const EXIT = /^Program (\w+) (success|failed)/;

/** Decode Pump Hooks events from a transaction's log messages (successful or failed). */
export function decodeEvents(logs: readonly string[], programId: PublicKey = PUMP_HOOKS_PROGRAM_ID): PumpHooksEvent[] {
  const pid = programId.toBase58();
  const stack: string[] = [];
  const out: PumpHooksEvent[] = [];
  for (const line of logs) {
    const inv = INVOKE.exec(line);
    if (inv) {
      stack.push(inv[1]);
      continue;
    }
    if (EXIT.test(line)) {
      stack.pop();
      continue;
    }
    if (!line.startsWith("Program data: ") || stack[stack.length - 1] !== pid) continue;
    const raw = Buffer.from(line.slice("Program data: ".length), "base64");
    const name = BY_DISC.get(Buffer.from(raw.subarray(0, 8)).toString("hex"));
    if (!name) continue;
    const data = DECODE[name](new Reader(raw, 8));
    out.push({ name, data } as PumpHooksEvent);
  }
  return out;
}

export function eventsOf<K extends EventName>(events: PumpHooksEvent[], name: K): Decoders[K][] {
  return events.filter((e) => e.name === name).map((e) => e.data as Decoders[K]);
}

export interface ProgramErrorInfo {
  code: number;
  name: string;
  msg: string;
}

/** Find the Pump Hooks error (if any) in logs or a `{ InstructionError: [i, { Custom: n }] }` value. */
export function decodeProgramError(err: unknown, logs: readonly string[] = []): ProgramErrorInfo | undefined {
  const fromLogs = logs.map((l) => /Error Number: (\d+)/.exec(l)).find(Boolean);
  let code = fromLogs ? Number(fromLogs[1]) : undefined;
  if (code === undefined && err && typeof err === "object" && "InstructionError" in err) {
    const ie = (err as { InstructionError: [number, unknown] }).InstructionError[1];
    if (ie && typeof ie === "object" && "Custom" in ie) code = Number((ie as { Custom: number }).Custom);
  }
  if (code === undefined) return undefined;
  return PROGRAM_ERRORS.find((e) => e.code === code);
}

/** One line per trace entry, e.g. `BeforeSwap  Volume Guard  ALLOW`. */
export function formatTrace(entries: TraceItem[], moduleName: (id: number) => string): string[] {
  return entries.map(
    (e) => `${Callback[e.callback].padEnd(11)} ${moduleName(e.moduleId).padEnd(18)} ${e.allowed ? "ALLOW" : `DENY ${e.reasonCode} ${describeReason(e.reasonCode)}`}`,
  );
}
