/** Instruction builders for every Pump Hooks instruction (Anchor wire format). */
import { Buffer } from "buffer";
import { AccountMeta, PublicKey, SYSVAR_INSTRUCTIONS_PUBKEY, TransactionInstruction } from "@solana/web3.js";
import { sha256 } from "@noble/hashes/sha256";
import { Writer } from "./codec.js";
import { PUMP_HOOKS_PROGRAM_ID, SLOT_CONFIG_LEN, SYSTEM_PROGRAM_ID, TradeMode, Venue } from "./constants.js";
import { hooksPda } from "./pda.js";
import { PolicyArgs } from "./policy.js";

const BPF_LOADER_UPGRADEABLE = new PublicKey("BPFLoaderUpgradeab1e11111111111111111111111");

export const ixDiscriminator = (name: string) => sha256(Buffer.from(`global:${name}`)).subarray(0, 8);

const meta = (pubkey: PublicKey, isWritable = false, isSigner = false): AccountMeta => ({ pubkey, isSigner, isWritable });

function ix(name: string, keys: AccountMeta[], args: (w: Writer) => void = () => {}, programId = PUMP_HOOKS_PROGRAM_ID) {
  const w = new Writer().bytes(ixDiscriminator(name));
  args(w);
  return new TransactionInstruction({ programId, keys, data: w.done() });
}

const config48 = (w: Writer, c: Uint8Array) => w.fixed(c, SLOT_CONFIG_LEN);

function policyArgs(w: Writer, a: PolicyArgs) {
  w.u8(a.trigger).u8(a.combinator).u32(a.cooldownSecs).u16(a.denyReason);
  w.vec(a.conditions, (w, c) => w.u8(c.field).u8(c.op).u8(c.arg).u64(c.value));
  w.vec(a.actions, (w, x) => w.u8(x.kind).u8(x.arg).u16(x.arg16).u64(x.value));
  w.fixed(a.labelHash, 32);
}

export interface ProgramOpt {
  programId?: PublicKey;
}

// ---- protocol admin

/** Only the program's upgrade authority can initialize (no front-running the global config). */
export function initializeIx(p: { authority: PublicKey } & ProgramOpt) {
  const pid = p.programId ?? PUMP_HOOKS_PROGRAM_ID;
  const programData = PublicKey.findProgramAddressSync([pid.toBuffer()], BPF_LOADER_UPGRADEABLE)[0];
  return ix(
    "initialize",
    [meta(p.authority, true, true), meta(hooksPda.global(pid), true), meta(SYSTEM_PROGRAM_ID), meta(pid), meta(programData)],
    undefined,
    pid,
  );
}

export function setKillSwitchIx(p: { authority: PublicKey; disabledModules: bigint; paused: boolean } & ProgramOpt) {
  const pid = p.programId ?? PUMP_HOOKS_PROGRAM_ID;
  return ix("set_kill_switch", [meta(p.authority, false, true), meta(hooksPda.global(pid), true)], (w) => w.u64(p.disabledModules).bool(p.paused), pid);
}

export function registerHookIx(p: { authority: PublicKey; moduleId: number; sourceHash: Uint8Array } & ProgramOpt) {
  const pid = p.programId ?? PUMP_HOOKS_PROGRAM_ID;
  return ix(
    "register_hook",
    [meta(p.authority, true, true), meta(hooksPda.global(pid)), meta(hooksPda.hookDefinition(p.moduleId, pid), true), meta(SYSTEM_PROGRAM_ID)],
    (w) => w.u16(p.moduleId).fixed(p.sourceHash, 32),
    pid,
  );
}

// ---- markets

export interface CreateMarketAccounts {
  authority: PublicKey;
  mint: PublicKey;
  bondingCurve: PublicKey;
  /** Resolved quote mint (wSOL for SOL markets). */
  quoteMint: PublicKey;
  /** Canonical PumpSwap pool, or any account (e.g. the system program) before migration. */
  pool: PublicKey;
}

export function createMarketIx(p: CreateMarketAccounts & ProgramOpt) {
  const pid = p.programId ?? PUMP_HOOKS_PROGRAM_ID;
  return ix(
    "create_market",
    [
      meta(p.authority, true, true),
      meta(hooksPda.global(pid), true),
      meta(p.mint),
      meta(p.bondingCurve),
      meta(p.quoteMint),
      meta(p.pool),
      meta(hooksPda.market(p.mint, p.authority, pid), true),
      meta(SYSTEM_PROGRAM_ID),
    ],
    undefined,
    pid,
  );
}

export function syncMarketIx(p: { market: PublicKey; bondingCurve: PublicKey; quoteMint: PublicKey; pool: PublicKey } & ProgramOpt) {
  return ix("sync_market", [meta(p.market, true), meta(p.bondingCurve), meta(p.quoteMint), meta(p.pool)], undefined, p.programId);
}

export function setMarketPausedIx(p: { authority: PublicKey; market: PublicKey; paused: boolean } & ProgramOpt) {
  return ix("set_market_paused", [meta(p.authority, false, true), meta(p.market, true)], (w) => w.bool(p.paused), p.programId);
}

// ---- hook stack

export interface InstallHookArgs {
  authority: PublicKey;
  market: PublicKey;
  moduleId: number;
  priority: number;
  /** 0 = the module's default callbacks. */
  callbacks: number;
  capabilities: number;
  config: Uint8Array;
}

export function installHookIx(p: InstallHookArgs & ProgramOpt) {
  const pid = p.programId ?? PUMP_HOOKS_PROGRAM_ID;
  return ix(
    "install_hook",
    [meta(p.authority, false, true), meta(hooksPda.global(pid)), meta(hooksPda.hookDefinition(p.moduleId, pid), true), meta(p.market, true)],
    (w) => {
      w.u16(p.moduleId).u16(p.priority).u16(p.callbacks).u32(p.capabilities);
      config48(w, p.config);
    },
    pid,
  );
}

export function removeHookIx(p: { authority: PublicKey; market: PublicKey; moduleId: number } & ProgramOpt) {
  const pid = p.programId ?? PUMP_HOOKS_PROGRAM_ID;
  return ix(
    "remove_hook",
    [meta(p.authority, false, true), meta(hooksPda.hookDefinition(p.moduleId, pid), true), meta(p.market, true)],
    (w) => w.u16(p.moduleId),
    pid,
  );
}

export function configureHookIx(p: { authority: PublicKey; market: PublicKey; moduleId: number; enabled: boolean; config?: Uint8Array } & ProgramOpt) {
  return ix(
    "configure_hook",
    [meta(p.authority, false, true), meta(p.market, true)],
    (w) => w.u16(p.moduleId).bool(p.enabled).option(p.config, config48),
    p.programId,
  );
}

// ---- composer policies

export function createPolicyIx(p: { authority: PublicKey; market: PublicKey; index: number; args: PolicyArgs } & ProgramOpt) {
  const pid = p.programId ?? PUMP_HOOKS_PROGRAM_ID;
  return ix(
    "create_policy",
    [meta(p.authority, true, true), meta(p.market, true), meta(hooksPda.policy(p.market, p.index, pid), true), meta(SYSTEM_PROGRAM_ID)],
    (w) => {
      w.u8(p.index);
      policyArgs(w, p.args);
    },
    pid,
  );
}

export function updatePolicyIx(p: { authority: PublicKey; market: PublicKey; index: number; enabled: boolean; args?: PolicyArgs } & ProgramOpt) {
  const pid = p.programId ?? PUMP_HOOKS_PROGRAM_ID;
  return ix(
    "update_policy",
    [meta(p.authority, false, true), meta(p.market, true), meta(hooksPda.policy(p.market, p.index, pid), true)],
    (w) => w.u8(p.index).bool(p.enabled).option(p.args, policyArgs),
    pid,
  );
}

export function closePolicyIx(p: { authority: PublicKey; market: PublicKey; index: number } & ProgramOpt) {
  const pid = p.programId ?? PUMP_HOOKS_PROGRAM_ID;
  return ix(
    "close_policy",
    [meta(p.authority, true, true), meta(p.market, true), meta(hooksPda.policy(p.market, p.index, pid), true)],
    (w) => w.u8(p.index),
    pid,
  );
}

// ---- routed execution

export interface RouteArgs {
  user: PublicKey;
  market: PublicKey;
  venue: Venue;
  mode: TradeMode;
  amount: bigint;
  limit: bigint;
  /** The 17 Pump trade accounts (see `ammV2Metas` / `curveV3Metas`). */
  pumpAccounts: AccountMeta[];
  /** Active policies of the market, in index order. */
  policyIndexes?: number[];
}

export function routeIx(p: RouteArgs & ProgramOpt) {
  const pid = p.programId ?? PUMP_HOOKS_PROGRAM_ID;
  const policies = (p.policyIndexes ?? []).map((i) => meta(hooksPda.policy(p.market, i, pid), true));
  return ix(
    p.venue === Venue.PumpSwap ? "route_amm" : "route_curve",
    [
      meta(p.user, true, true),
      meta(hooksPda.global(pid)),
      meta(p.market, true),
      meta(hooksPda.userState(p.market, p.user, pid), true),
      meta(SYSTEM_PROGRAM_ID),
      ...p.pumpAccounts,
      ...policies,
    ],
    (w) => w.u8(p.mode).u64(p.amount).u64(p.limit),
    pid,
  );
}

// ---- SENTIENT

/** Must be preceded in the same transaction by the Ed25519 verification of the signal. */
export function submitSignalIx(p: { market: PublicKey; signal: Uint8Array } & ProgramOpt) {
  return ix("submit_signal", [meta(p.market, true), meta(SYSVAR_INSTRUCTIONS_PUBKEY)], (w) => w.fixed(p.signal, 96), p.programId);
}
