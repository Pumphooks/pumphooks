import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { address, type Instruction } from "@solana/kit";
import { Clock } from "litesvm";
import { ExecutionLab, fromB64, sha256, toB64, type AccountState, type ExecutionTrace, type HookResolver } from "./lab.js";

/**
 * A self-contained reproduction: program hashes, the exact pre-state of every account in the
 * message, the clock, the instructions and the expected outcome. It holds no secret keys; replays
 * run with signature verification off.
 */
export interface ReproBundle {
  format: "pumphooks-repro/1";
  label: string;
  runtime: { engine: "litesvm"; version: string };
  clock: ExecutionTrace["replay"]["clock"];
  programs: { id: string; name?: string; sha256: string; path: string; embedded?: string }[];
  accounts: { address: string; lamports: string; owner: string; executable: boolean; data: string }[];
  message: ExecutionTrace["replay"]["message"];
  expected: { status: ExecutionTrace["status"]; error?: string; computeUnits: string; logsSha256: string };
}

export const LITESVM_VERSION = "1.6.1";

export function logsDigest(logs: string[]): string {
  return sha256(new TextEncoder().encode(logs.join("\n")));
}

export function exportRepro(lab: ExecutionLab, trace: ExecutionTrace, opts: { embedPrograms?: boolean; pathRoot?: string } = {}): ReproBundle {
  const used = new Set<string>([...trace.instructions.map((i) => i.programId), ...trace.invocations.map((i) => i.programId)]);
  const programs = [...lab.programs.entries()]
    .filter(([id]) => used.has(id))
    .map(([id, p]) => ({
      id,
      name: p.name,
      sha256: p.sha256,
      path: opts.pathRoot ? relativeTo(opts.pathRoot, p.path) : p.path,
      embedded: opts.embedPrograms ? toB64(readFileSync(p.path)) : undefined,
    }));
  return {
    format: "pumphooks-repro/1",
    label: trace.label,
    runtime: { engine: "litesvm", version: LITESVM_VERSION },
    clock: trace.replay.clock,
    programs,
    accounts: trace.replay.pre.map((a) => ({ address: a.address, lamports: a.lamports.toString(), owner: a.owner, executable: a.executable, data: toB64(a.data) })),
    message: trace.replay.message,
    expected: { status: trace.status, error: trace.error, computeUnits: trace.computeUnits.total.toString(), logsSha256: logsDigest(trace.logs) },
  };
}

export interface ReplayResult {
  trace: ExecutionTrace;
  matches: { status: boolean; error: boolean; computeUnits: boolean; logs: boolean };
  identical: boolean;
}

/** Rebuilds the state in a fresh LiteSVM, re-executes the message and compares the outcome. */
export async function replay(bundle: ReproBundle, opts: { pathRoot?: string; resolvers?: HookResolver[] } = {}): Promise<ReplayResult> {
  if (bundle.format !== "pumphooks-repro/1") throw new Error(`unsupported bundle format ${bundle.format}`);
  const lab = new ExecutionLab({ sigverify: false, resolvers: opts.resolvers });
  for (const p of bundle.programs) {
    const bytes = p.embedded ? fromB64(p.embedded) : readFileSync(opts.pathRoot ? resolve(opts.pathRoot, p.path) : p.path);
    const digest = sha256(bytes);
    if (digest !== p.sha256) throw new Error(`program ${p.id}: binary hash ${digest} does not match bundle ${p.sha256}`);
    lab.svm.addProgram(address(p.id), bytes);
    lab.programs.set(p.id, { path: p.path, name: p.name, sha256: digest });
    if (p.name) lab.label(p.id, p.name);
  }
  for (const a of bundle.accounts) {
    const s: AccountState = { address: a.address, lamports: BigInt(a.lamports), owner: a.owner, executable: a.executable, data: fromB64(a.data) };
    lab.setAccount(s);
  }
  const c = bundle.clock;
  lab.svm.setClock(new Clock(BigInt(c.slot), BigInt(c.epochStartTimestamp), BigInt(c.epoch), BigInt(c.leaderScheduleEpoch), BigInt(c.unixTimestamp)));
  const instructions: Instruction[] = bundle.message.instructions.map((ix) => ({
    programAddress: address(ix.programId),
    accounts: ix.accounts.map((a) => ({ address: address(a.address), role: a.role })),
    data: fromB64(ix.data),
  }));
  const trace = await lab.execute({ instructions, payer: address(bundle.message.feePayer), label: `${bundle.label} (replay)` });
  const matches = {
    status: trace.status === bundle.expected.status,
    error: (trace.error ?? "") === (bundle.expected.error ?? ""),
    computeUnits: trace.computeUnits.total.toString() === bundle.expected.computeUnits,
    logs: logsDigest(trace.logs) === bundle.expected.logsSha256,
  };
  return { trace, matches, identical: Object.values(matches).every(Boolean) };
}

/** JSON with bigints as strings and byte arrays as base64. */
export function toJson(value: unknown): string {
  return JSON.stringify(value, (_, v) => (typeof v === "bigint" ? v.toString() : v instanceof Uint8Array ? toB64(v) : v), 2);
}

function relativeTo(root: string, p: string): string {
  const abs = resolve(p);
  const r = resolve(root);
  return abs.startsWith(r + "/") ? abs.slice(r.length + 1) : abs;
}
