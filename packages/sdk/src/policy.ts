/**
 * Composer: `WHEN trigger IF conditions THEN actions`, compiled to the Policy VM's fixed record.
 * Validation mirrors `policy_vm::PolicySpec::validate` so a policy that compiles here is accepted
 * on-chain (the program re-validates; it is the authority).
 */
import { Buffer } from "buffer";
import { sha256 } from "@noble/hashes/sha256";
import { Callback } from "./constants.js";

export const POLICY_LIMITS = { maxConditions: 8, maxActions: 4, maxRegisters: 4, maxDenyReason: 999, maxPointsPerAction: 10_000n } as const;

export const Field = {
  TRADE_SIZE: 1,
  TRADE_SIZE_BASE: 2,
  ROUTED_VOLUME: 3,
  VOLATILITY: 4,
  TREASURY_BALANCE: 5,
  TIME_SINCE_LAUNCH: 6,
  TIME_SINCE_LAST_ACTION: 7,
  BUY_SELL_RATIO: 8,
  HOLDER_DURATION: 9,
  HOOK_STATE: 10,
  USER_ROUTED_VOLUME: 11,
  USER_XP: 12,
  USER_BASE_BALANCE: 13,
} as const;
export type FieldName = keyof typeof Field;

export const FIELD_INFO: Record<FieldName, { label: string; unit: string; available: boolean }> = {
  TRADE_SIZE: { label: "Trade size (quote)", unit: "lamports", available: true },
  TRADE_SIZE_BASE: { label: "Trade size (base)", unit: "base", available: true },
  ROUTED_VOLUME: { label: "Market routed volume", unit: "lamports", available: true },
  VOLATILITY: { label: "Deviation from routed EMA (needs Volatility Guard)", unit: "bps", available: true },
  TREASURY_BALANCE: { label: "Treasury balance (not available in v1)", unit: "lamports", available: false },
  TIME_SINCE_LAUNCH: { label: "Time since market activation", unit: "seconds", available: true },
  TIME_SINCE_LAST_ACTION: { label: "Time since this policy last acted", unit: "seconds", available: true },
  BUY_SELL_RATIO: { label: "Routed buy/sell ratio", unit: "bps", available: true },
  HOLDER_DURATION: { label: "Wallet holding duration", unit: "seconds", available: true },
  HOOK_STATE: { label: "Policy register", unit: "value", available: true },
  USER_ROUTED_VOLUME: { label: "Wallet routed volume", unit: "lamports", available: true },
  USER_XP: { label: "Wallet Holder XP", unit: "xp", available: true },
  USER_BASE_BALANCE: { label: "Wallet token balance", unit: "base", available: true },
};

export const Op = { "==": 0, "!=": 1, ">": 2, ">=": 3, "<": 4, "<=": 5 } as const;
export type OpName = keyof typeof Op;

export const ActionKind = {
  ALLOW: 1,
  DENY: 2,
  RECORD: 3,
  AWARD_POINTS: 4,
  UPDATE_STATE: 5,
  QUEUE_TWAP: 6,
  ALLOCATE_AUTHORIZED_TREASURY: 7,
  PAUSE_STRATEGY: 8,
  RESUME_STRATEGY: 9,
  EMIT_SIGNAL: 10,
  EXECUTE_APPROVED_STRATEGY: 11,
} as const;
export type ActionName = keyof typeof ActionKind;

export const StateOp = { SET: 0, ADD: 1, SUB: 2 } as const;

/** v1 triggers are the routed-trade callbacks; crank-driven triggers are reserved. */
export type Trigger = "BeforeBuy" | "AfterBuy" | "BeforeSell" | "AfterSell" | "BeforeSwap" | "AfterSwap";
const TRIGGERS: Record<Trigger, number> = {
  BeforeBuy: Callback.BeforeBuy,
  AfterBuy: Callback.AfterBuy,
  BeforeSell: Callback.BeforeSell,
  AfterSell: Callback.AfterSell,
  BeforeSwap: Callback.BeforeSwap,
  AfterSwap: Callback.AfterSwap,
};
const isBefore = (t: number) => t === Callback.BeforeBuy || t === Callback.BeforeSell || t === Callback.BeforeSwap;

export interface ConditionSpec {
  field: FieldName;
  op: OpName;
  value: bigint | number;
  /** Register index for HOOK_STATE. */
  register?: number;
}

export type ActionSpec =
  | { kind: "ALLOW" }
  | { kind: "DENY" }
  | { kind: "RECORD"; tag: bigint | number }
  | { kind: "AWARD_POINTS"; points: bigint | number }
  | { kind: "UPDATE_STATE"; register: number; op: keyof typeof StateOp; value: bigint | number }
  | { kind: "EMIT_SIGNAL" }
  | { kind: "QUEUE_TWAP" | "ALLOCATE_AUTHORIZED_TREASURY" | "PAUSE_STRATEGY" | "RESUME_STRATEGY" | "EXECUTE_APPROVED_STRATEGY"; value?: bigint | number };

export interface PolicySpec {
  when: Trigger;
  if?: ConditionSpec[];
  match?: "ALL" | "ANY";
  then: ActionSpec[];
  cooldownSecs?: number;
  /** Reason offset (0..=999); the trade is denied with reason 9000 + denyReason. */
  denyReason?: number;
  label?: string;
}

/** Wire form accepted by `create_policy` / `update_policy`. */
export interface PolicyArgs {
  trigger: number;
  combinator: number;
  cooldownSecs: number;
  denyReason: number;
  conditions: { field: number; op: number; arg: number; value: bigint }[];
  actions: { kind: number; arg: number; arg16: number; value: bigint }[];
  labelHash: Uint8Array;
}

export class PolicyCompileError extends Error {}

export function compilePolicy(spec: PolicySpec, env: { hasVolatilityGuard?: boolean } = {}): PolicyArgs {
  const fail = (m: string): never => {
    throw new PolicyCompileError(m);
  };
  const trigger = TRIGGERS[spec.when] ?? fail(`unsupported trigger ${spec.when}`);
  const conds = spec.if ?? [];
  if (conds.length > POLICY_LIMITS.maxConditions) fail(`at most ${POLICY_LIMITS.maxConditions} conditions`);
  if (spec.then.length === 0) fail("a policy needs at least one action");
  if (spec.then.length > POLICY_LIMITS.maxActions) fail(`at most ${POLICY_LIMITS.maxActions} actions`);
  const denyReason = spec.denyReason ?? 0;
  if (denyReason < 0 || denyReason > POLICY_LIMITS.maxDenyReason) fail("denyReason must be 0..=999");

  const conditions = conds.map((c) => {
    const field = Field[c.field] ?? fail(`unknown field ${c.field}`);
    const op = Op[c.op] ?? fail(`unknown operator ${c.op}`);
    if (!FIELD_INFO[c.field].available) fail(`${c.field} is not available in v1`);
    if (c.field === "VOLATILITY" && !env.hasVolatilityGuard) fail("VOLATILITY needs Volatility Guard installed");
    const arg = c.field === "HOOK_STATE" ? (c.register ?? 0) : 0;
    if (arg >= POLICY_LIMITS.maxRegisters) fail("register out of range");
    return { field, op, arg, value: BigInt(c.value) };
  });

  const before = isBefore(trigger);
  let denies = 0;
  const actions = spec.then.map((a) => {
    const kind = ActionKind[a.kind] ?? fail(`unknown action ${(a as { kind: string }).kind}`);
    switch (a.kind) {
      case "DENY":
        if (!before) fail("DENY is only allowed on Before* triggers");
        denies++;
        return { kind, arg: 0, arg16: 0, value: 0n };
      case "AWARD_POINTS": {
        if (before) fail("AWARD_POINTS is only allowed on After* triggers");
        const v = BigInt(a.points);
        if (v > POLICY_LIMITS.maxPointsPerAction) fail("at most 10000 points per action");
        return { kind, arg: 0, arg16: 0, value: v };
      }
      case "UPDATE_STATE":
        if (a.register >= POLICY_LIMITS.maxRegisters) fail("register out of range");
        return { kind, arg: a.register, arg16: StateOp[a.op], value: BigInt(a.value) };
      case "RECORD":
        return { kind, arg: 0, arg16: 0, value: BigInt(a.tag) };
      case "ALLOW":
      case "EMIT_SIGNAL":
        return { kind, arg: 0, arg16: 0, value: 0n };
      default:
        return { kind, arg: 0, arg16: 0, value: BigInt(a.value ?? 0) };
    }
  });
  if (denies > 1) fail("at most one DENY per policy");

  return {
    trigger,
    combinator: spec.match === "ANY" ? 1 : 0,
    cooldownSecs: spec.cooldownSecs ?? 0,
    denyReason,
    conditions,
    actions,
    labelHash: spec.label ? sha256(Buffer.from(spec.label)) : new Uint8Array(32),
  };
}

/** Human-readable rendering of a compiled policy (Composer preview, explorer). */
export function describePolicy(p: Pick<PolicyArgs, "trigger" | "combinator" | "conditions" | "actions" | "denyReason">): string {
  const trig = (Object.keys(TRIGGERS) as Trigger[]).find((k) => TRIGGERS[k] === p.trigger) ?? `trigger(${p.trigger})`;
  const fname = (f: number) => (Object.keys(Field) as FieldName[]).find((k) => Field[k] === f) ?? `field(${f})`;
  const oname = (o: number) => (Object.keys(Op) as OpName[]).find((k) => Op[k] === o) ?? "?";
  const aname = (k: number) => (Object.keys(ActionKind) as ActionName[]).find((n) => ActionKind[n] === k) ?? `action(${k})`;
  const conds = p.conditions.map((c) => `${fname(c.field)}${c.field === Field.HOOK_STATE ? `[${c.arg}]` : ""} ${oname(c.op)} ${c.value}`);
  const acts = p.actions.map((a) =>
    a.kind === ActionKind.DENY ? `DENY(${9000 + p.denyReason})` : a.value ? `${aname(a.kind)}(${a.value})` : aname(a.kind),
  );
  const cond = conds.length ? ` IF ${conds.join(p.combinator === 1 ? " OR " : " AND ")}` : "";
  return `WHEN ${trig}${cond} THEN ${acts.join(", ")}`;
}
