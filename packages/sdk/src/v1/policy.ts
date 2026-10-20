// Program-hook policy calls (hook_standard::PolicyContext / PolicyResult), for simulating a hook
// program the way an environment calls it.

import { sha256 } from "@noble/hashes/sha256";
import { concat, findPda, InstructionV1, KeyLike, meta, pk } from "./ix.js";
import { CALLBACK_NAMES, ENVIRONMENT_NAMES, EnvironmentId } from "./stack.js";

export const ABI_VERSION = 1;
export const POLICY_CONTEXT_LEN = 208;
export const POLICY_RESULT_LEN = 40;
export const DECISION_ALLOW = 0;
export const DECISION_REJECT = 1;
/** Hook-specific rejection reasons start here (hook_sdk::CUSTOM_REASON_BASE). */
export const CUSTOM_REASON_BASE = 1000;

export const ENVIRONMENT_PROGRAMS: Record<EnvironmentId, string> = {
  0: "FZBBkQQamgfMCAoNk6PAdZi9nYZGMsGw9gDyTMNC3v54",
  1: "2a3wWdS8z9g7vBcYWSjdETyEzo7DEjV3iYRZFsM52q4U",
  2: "",
  3: "6AEiwvifk4TVJ8g3UoVXqhaK8AqYy43A4DzWCgMmrtQw",
};

export function policyDiscriminator(env: EnvironmentId, callback: number): Uint8Array {
  const name = CALLBACK_NAMES[env][callback];
  if (name === undefined) throw new Error(`no callback ${callback} in ${ENVIRONMENT_NAMES[env]}`);
  return sha256(new TextEncoder().encode(`pump-hooks-v1:${ENVIRONMENT_NAMES[env]}:${name}`)).slice(0, 8);
}

export const callbackAuthority = (environmentProgram: KeyLike, hook: KeyLike) =>
  findPda([new TextEncoder().encode("callback"), pk(hook).toBytes()], environmentProgram);

export interface PolicyContextSpec {
  environment: EnvironmentId;
  callback: number;
  authorityBump: number;
  entryIndex?: number;
  subject: KeyLike;
  actor: KeyLike;
  asset?: KeyLike;
  counterparty?: KeyLike;
  amount?: bigint;
  amountB?: bigint;
  slot?: bigint;
  unixTimestamp?: bigint;
  sequence?: bigint;
  inputs?: { kind: number; value: bigint }[];
}

export function encodePolicyContext(c: PolicyContextSpec): Uint8Array {
  const b = new Uint8Array(POLICY_CONTEXT_LEN);
  const v = new DataView(b.buffer);
  b[0] = ABI_VERSION;
  b[1] = c.environment;
  b[2] = c.callback;
  b[3] = c.authorityBump;
  b[4] = c.entryIndex ?? 0;
  b.set(pk(c.subject).toBytes(), 8);
  b.set(pk(c.actor).toBytes(), 40);
  if (c.asset) b.set(pk(c.asset).toBytes(), 72);
  if (c.counterparty) b.set(pk(c.counterparty).toBytes(), 104);
  v.setBigUint64(136, c.amount ?? 0n, true);
  v.setBigUint64(144, c.amountB ?? 0n, true);
  v.setBigUint64(152, c.slot ?? 0n, true);
  v.setBigInt64(160, c.unixTimestamp ?? 0n, true);
  v.setBigUint64(168, c.sequence ?? 0n, true);
  (c.inputs ?? []).slice(0, 2).forEach((o, i) => {
    v.setUint16(176 + 16 * i, o.kind, true);
    v.setBigUint64(176 + 16 * i + 8, o.value, true);
  });
  return b;
}

/**
 * The instruction an environment sends to a program hook: `[callback authority (signer), subject,
 * forwarded...]`. On chain the authority signs via `invoke_signed`; off chain only a simulator with
 * signature verification disabled can reproduce that.
 */
export function policyCall(o: {
  hook: KeyLike;
  context: Omit<PolicyContextSpec, "authorityBump">;
  forwarded?: { address: KeyLike; writable?: boolean }[];
  opData?: Uint8Array;
}): InstructionV1 {
  const env = o.context.environment;
  const envProgram = ENVIRONMENT_PROGRAMS[env];
  if (!envProgram) throw new Error(`${ENVIRONMENT_NAMES[env]} has no program-hook caller`);
  const [authority, bump] = callbackAuthority(envProgram, o.hook);
  return {
    programId: pk(o.hook).toBase58(),
    accounts: [meta(authority, false, true), meta(o.context.subject), ...(o.forwarded ?? []).map((a) => meta(a.address, a.writable ?? false))],
    data: concat(policyDiscriminator(env, o.context.callback), encodePolicyContext({ ...o.context, authorityBump: bump }), o.opData ?? new Uint8Array()),
  };
}

export interface PolicyResult {
  version: number;
  decision: "allow" | "reject";
  reason: number;
  outputKind: number;
  outputValue: bigint;
}

export function decodePolicyResult(data: Uint8Array): PolicyResult | null {
  if (data.length !== POLICY_RESULT_LEN || data[0] !== ABI_VERSION || data[1] > DECISION_REJECT) return null;
  const v = new DataView(data.buffer, data.byteOffset, data.byteLength);
  if (v.getUint16(6, true) !== 0 || data.slice(16).some((x) => x !== 0)) return null;
  return {
    version: data[0],
    decision: data[1] === DECISION_ALLOW ? "allow" : "reject",
    reason: v.getUint16(2, true),
    outputKind: v.getUint16(4, true),
    outputValue: v.getBigUint64(8, true),
  };
}
