import { v1 } from "@pump-hooks/sdk";

export const COMPUTE_BUDGET_PROGRAM_ID = "ComputeBudget111111111111111111111111111111";
export const MAX_TX_SIZE = 1232;
export const MAX_CU = 1_400_000;
export const MAX_ACCOUNTS = 64;

/** Reads raw account data (LiteSVM, RPC, fixtures). */
export type AccountReader = (address: string) => Uint8Array | null | Promise<Uint8Array | null>;

export type Operation =
  | { kind: "instruction"; id: string; ix: v1.InstructionV1; cu?: number; after?: string[] }
  /** Token-2022 transfer of a token-hooks mint; the hook's extra accounts are resolved from the live policy. */
  | { kind: "hooked-transfer"; id: string; source: string; destination: string; sourceOwner: string; destinationOwner: string; mint: string; amount: bigint; decimals: number; after?: string[] }
  | { kind: "wrap" | "unwrap"; id: string; user: string; underlyingMint: string; underlyingProgram: string; userUnderlying: string; userWrapped: string; amount: bigint; minOut: bigint; after?: string[] };

export interface LookupTable {
  address: string;
  addresses: string[];
}

export interface Plan {
  feePayer: string;
  operations: Operation[];
  /** Signers the caller can provide (fee payer included implicitly). */
  signers: string[];
  lookupTables?: LookupTable[];
  /** micro-lamports per CU; omitted = no price instruction. */
  computeUnitPrice?: bigint;
  /** Fraction added to the CU estimate (default 0.1). */
  cuMargin?: number;
}

export interface AccountEntry {
  address: string;
  signer: boolean;
  writable: boolean;
  /** Set when the account is loaded through a lookup table instead of the static key list. */
  lookupTable?: string;
  usedBy: string[];
}

export interface CompiledTransaction {
  order: string[];
  instructions: v1.InstructionV1[];
  accounts: AccountEntry[];
  requiredSigners: string[];
  /** Exact serialized size (signatures + v0 message). */
  size: number;
  computeUnitLimit: number;
  /** "static" until `refineWithSimulation` replaces it with a measured value. */
  computeProvenance: "static" | "simulated";
  lookupTablesUsed: string[];
  warnings: string[];
}

export class CompileError extends Error {
  constructor(
    readonly code: "cycle" | "unknown-dependency" | "duplicate-id" | "missing-signer" | "too-large" | "too-many-accounts" | "compute" | "unresolved",
    message: string,
    readonly detail: Record<string, unknown> = {},
  ) {
    super(message);
  }
}

/** Static per-instruction estimates, used until a simulation measures the real cost. */
const STATIC_CU: Record<string, number> = {
  [v1.SYSTEM_PROGRAM_ID]: 3_000,
  [v1.SPL_TOKEN_PROGRAM_ID]: 6_000,
  [v1.TOKEN_2022_PROGRAM_ID]: 8_000,
  [COMPUTE_BUDGET_PROGRAM_ID]: 150,
};
/** Measured in docs/benchmarks/report.md (hooked transfer, 7 builtins) plus headroom for larger stacks. */
const HOOKED_TRANSFER_CU = 30_000;
const WRAPPER_CU = 60_000;
const DEFAULT_CU = 200_000;

function compactLen(n: number): number {
  return n < 0x80 ? 1 : n < 0x4000 ? 2 : 3;
}

function topo(ops: Operation[]): Operation[] {
  const byId = new Map<string, Operation>();
  for (const o of ops) {
    if (byId.has(o.id)) throw new CompileError("duplicate-id", `duplicate operation id "${o.id}"`);
    byId.set(o.id, o);
  }
  const state = new Map<string, 1 | 2>();
  const out: Operation[] = [];
  const visit = (o: Operation, path: string[]) => {
    if (state.get(o.id) === 2) return;
    if (state.get(o.id) === 1) throw new CompileError("cycle", `dependency cycle: ${[...path, o.id].join(" -> ")}`, { path: [...path, o.id] });
    state.set(o.id, 1);
    for (const d of o.after ?? []) {
      const dep = byId.get(d);
      if (!dep) throw new CompileError("unknown-dependency", `"${o.id}" depends on unknown "${d}"`);
      visit(dep, [...path, o.id]);
    }
    state.set(o.id, 2);
    out.push(o);
  };
  for (const o of ops) visit(o, []);
  return out;
}

async function lower(op: Operation, read: AccountReader): Promise<{ ix: v1.InstructionV1; cu: number }> {
  switch (op.kind) {
    case "instruction":
      return { ix: op.ix, cu: op.cu ?? STATIC_CU[op.ix.programId] ?? DEFAULT_CU };
    case "hooked-transfer": {
      const [policy] = v1.tokenHooksPda.policy(op.mint);
      const data = await read(policy);
      if (!data) throw new CompileError("unresolved", `"${op.id}": mint ${op.mint} has no token-hooks policy (${policy})`, { policy });
      const stack = v1.decodeTokenPolicy(data).stack;
      const extra = v1.transferHookAccounts(op.mint, stack, { owner: op.sourceOwner }, { owner: op.destinationOwner });
      return {
        ix: v1.transferChecked({ source: op.source, mint: op.mint, destination: op.destination, owner: op.sourceOwner, amount: op.amount, decimals: op.decimals, extra }),
        cu: HOOKED_TRANSFER_CU,
      };
    }
    case "wrap":
    case "unwrap": {
      const f = op.kind === "wrap" ? v1.wrap : v1.unwrap;
      return { ix: f(op), cu: WRAPPER_CU };
    }
  }
}

export function computeBudget(limit: number, price?: bigint): v1.InstructionV1[] {
  const out: v1.InstructionV1[] = [{ programId: COMPUTE_BUDGET_PROGRAM_ID, accounts: [], data: v1.concat(v1.u8(2), v1.u32(limit)) }];
  if (price !== undefined) out.push({ programId: COMPUTE_BUDGET_PROGRAM_ID, accounts: [], data: v1.concat(v1.u8(3), v1.u64(price)) });
  return out;
}

/** Merged account list in v0 order: fee payer, writable signers, readonly signers, writable, readonly. */
export function mergeAccounts(feePayer: string, ixs: { id: string; ix: v1.InstructionV1 }[]): AccountEntry[] {
  const m = new Map<string, AccountEntry>();
  const touch = (address: string, signer: boolean, writable: boolean, by: string) => {
    const e = m.get(address) ?? { address, signer: false, writable: false, usedBy: [] };
    e.signer ||= signer;
    e.writable ||= writable;
    if (!e.usedBy.includes(by)) e.usedBy.push(by);
    m.set(address, e);
  };
  touch(feePayer, true, true, "fee-payer");
  for (const { id, ix } of ixs) {
    touch(ix.programId, false, false, id);
    for (const a of ix.accounts) touch(a.address, a.signer, a.writable, id);
  }
  const rank = (e: AccountEntry) => (e.address === feePayer ? 0 : e.signer ? (e.writable ? 1 : 2) : e.writable ? 3 : 4);
  return [...m.values()].sort((a, b) => rank(a) - rank(b));
}

/** Exact v0 transaction size for these accounts and instructions. */
export function transactionSize(accounts: AccountEntry[], ixs: v1.InstructionV1[]): number {
  const signers = accounts.filter((a) => a.signer).length;
  const staticKeys = accounts.filter((a) => !a.lookupTable).length;
  let n = compactLen(signers) + 64 * signers;
  n += 1 + 3 + compactLen(staticKeys) + 32 * staticKeys + 32;
  n += compactLen(ixs.length);
  for (const ix of ixs) n += 1 + compactLen(ix.accounts.length) + ix.accounts.length + compactLen(ix.data.length) + ix.data.length;
  const tables = new Map<string, { w: number; r: number }>();
  for (const a of accounts) {
    if (!a.lookupTable) continue;
    const t = tables.get(a.lookupTable) ?? { w: 0, r: 0 };
    if (a.writable) t.w++;
    else t.r++;
    tables.set(a.lookupTable, t);
  }
  n += compactLen(tables.size);
  for (const t of tables.values()) n += 32 + compactLen(t.w) + t.w + compactLen(t.r) + t.r;
  return n;
}

/** Moves every account that may be looked up (not a signer, not an invoked program) into the first table holding it. */
function applyLookups(accounts: AccountEntry[], ixs: v1.InstructionV1[], tables: LookupTable[]): string[] {
  const programs = new Set(ixs.map((i) => i.programId));
  const used = new Set<string>();
  for (const a of accounts) {
    if (a.signer || programs.has(a.address)) continue;
    const t = tables.find((t) => t.addresses.includes(a.address));
    if (t) {
      a.lookupTable = t.address;
      used.add(t.address);
    }
  }
  return [...used];
}

export async function compile(plan: Plan, read: AccountReader): Promise<CompiledTransaction> {
  const ordered = topo(plan.operations);
  const lowered: { id: string; ix: v1.InstructionV1; cu: number }[] = [];
  for (const op of ordered) lowered.push({ id: op.id, ...(await lower(op, read)) });

  const margin = plan.cuMargin ?? 0.1;
  const estimate = Math.ceil(lowered.reduce((s, l) => s + l.cu, 0) * (1 + margin)) + 300;
  if (estimate > MAX_CU) throw new CompileError("compute", `estimated ${estimate} CU exceeds the ${MAX_CU} transaction limit`, { estimate });

  const budget = computeBudget(estimate, plan.computeUnitPrice).map((ix, i) => ({ id: `compute-budget-${i}`, ix }));
  const all = [...budget, ...lowered.map(({ id, ix }) => ({ id, ix }))];
  const instructions = all.map((x) => x.ix);
  const accounts = mergeAccounts(plan.feePayer, all);

  const available = new Set([plan.feePayer, ...plan.signers]);
  const missing = accounts.filter((a) => a.signer && !available.has(a.address));
  if (missing.length) {
    throw new CompileError("missing-signer", `missing signatures: ${missing.map((m) => `${m.address} (${m.usedBy.join(", ")})`).join("; ")}`, { missing: missing.map((m) => m.address) });
  }
  const lookupTablesUsed = plan.lookupTables?.length ? applyLookups(accounts, instructions, plan.lookupTables) : [];
  if (accounts.length > MAX_ACCOUNTS) throw new CompileError("too-many-accounts", `${accounts.length} accounts exceed the ${MAX_ACCOUNTS}-account lock limit`);

  const size = transactionSize(accounts, instructions);
  if (size > MAX_TX_SIZE) {
    const lookupable = accounts.filter((a) => !a.signer && !a.lookupTable && !instructions.some((i) => i.programId === a.address)).length;
    throw new CompileError(
      "too-large",
      `transaction is ${size} bytes (limit ${MAX_TX_SIZE}). It is not split automatically, because a split would no longer be atomic. ` +
        (lookupable ? `${lookupable} accounts could move to an address lookup table (saves up to ${lookupable * 31} bytes).` : "Remove operations or send them as separate, non-atomic transactions."),
      { size, limit: MAX_TX_SIZE, lookupable },
    );
  }

  const warnings: string[] = [];
  const writableBy = accounts.filter((a) => a.writable && a.usedBy.filter((u) => !u.startsWith("compute-budget")).length > 1);
  for (const a of writableBy) if (a.address !== plan.feePayer) warnings.push(`${a.address} is written by ${a.usedBy.join(", ")}; later operations see earlier writes`);

  return {
    order: ordered.map((o) => o.id),
    instructions,
    accounts,
    requiredSigners: accounts.filter((a) => a.signer).map((a) => a.address),
    size,
    computeUnitLimit: estimate,
    computeProvenance: "static",
    lookupTablesUsed,
    warnings,
  };
}

/** Replaces the static compute limit with the simulated consumption plus `margin`. */
export function withMeasuredCompute(tx: CompiledTransaction, consumed: bigint | number, margin = 0.1, price?: bigint): CompiledTransaction {
  const limit = Math.min(MAX_CU, Math.ceil(Number(consumed) * (1 + margin)) + 300);
  const n = price === undefined ? 1 : 2;
  const instructions = [...computeBudget(limit, price), ...tx.instructions.slice(n)];
  return { ...tx, instructions, computeUnitLimit: limit, computeProvenance: "simulated", size: transactionSize(tx.accounts, instructions) };
}
