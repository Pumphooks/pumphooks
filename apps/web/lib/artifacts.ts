// Artifacts from real runs. data/*.json come from scripts/export-data.mts (LiteSVM + compiled
// programs); bench/*.json come from the Rust bench (Mollusk + LiteSVM). Nothing here is
// hand-written.

import labJson from "@/data/lab.json";
import compilerJson from "@/data/compiler.json";
import securityJson from "@/data/security.json";
import benchResults from "../../../bench/results.json";
import benchBaseline from "../../../bench/baseline.json";

export interface Provenance {
  commit: string;
  generatedAt: string;
  runtime: string;
  programs: Record<string, string>;
  command: string;
}

export interface TraceJson {
  label: string;
  status: "success" | "failed";
  error?: string;
  note?: string;
  feePayer: string;
  invocations: { program: string; programId: string; depth: number; consumed?: number; result: string }[];
  cpiTree: { program: string; programId: string; stackHeight: number; children?: unknown[] }[][];
  computeUnits: { total: string; perInstruction: (number | null)[] };
  accountDiffs: {
    address: string;
    label?: string;
    signer: boolean;
    writable: boolean;
    created: boolean;
    closed: boolean;
    lamportsBefore: string;
    lamportsAfter: string;
    dataLenBefore: number;
    dataLenAfter: number;
    changedRanges: [number, number][];
  }[];
  tokenBalances: { account: string; label?: string; mint: string; before: string; after: string; delta: string }[];
  hookSteps: { engine: string; callback: string; entry: number; name: string; kind: string; mandatory: boolean; result: string; reason?: string }[];
  logs: string[];
  returnData?: { programId: string; data: string };
  provenance: Record<string, { provenance: string; source: string }>;
}

export const lab = labJson as unknown as Provenance & { traces: TraceJson[] };

export interface CompiledAccount {
  address: string;
  signer: boolean;
  writable: boolean;
  lookupTable?: string;
}

export const compiler = compilerJson as unknown as Provenance & {
  plan: { id: string; kind: string; after: string[] }[];
  compiled: {
    order: string[];
    size: number;
    computeUnitLimit: number;
    computeProvenance: string;
    requiredSigners: string[];
    warnings: string[];
    lookupTablesUsed: string[];
    accounts: CompiledAccount[];
    instructions: { programId: string; accounts: number; dataLen: number }[];
  };
  executed: { status: string; computeUnits: string; measuredLimit: number };
  atomicFailure: { plan: { id: string; kind: string; after: string[] }[]; status: string; error?: string; accountsChanged: string[] };
  errors: { case: string; code: string; message: string }[];
};

export interface Evidence {
  test: string;
  kind: string;
  level: string;
}

export const security = securityJson as unknown as Provenance & {
  disclaimer: string;
  fuzz: {
    target: string;
    seed: number;
    runs: number;
    steps: number;
    executed: number;
    coverage: Record<string, number>;
    outcomes: { ok: number; failed: number };
    failure?: { invariant: string; message: string };
  };
  invariants: string[];
  attacks: Record<string, string>;
  categories: { id: string; title: string; threat: string; evidence: Evidence[]; gaps?: string }[];
};

export interface BenchMeasurement {
  name: string;
  compute_units: number;
  ok: boolean;
  kernel_estimate?: number | null;
}
export interface BenchResults {
  runtime: string;
  programs: [string, string][];
  unit: BenchMeasurement[];
  transaction: BenchMeasurement[];
}

export const bench = benchResults as unknown as BenchResults;
export const baseline = benchBaseline as unknown as BenchResults;

export const short = (s: string, n = 4) => (s.length > 2 * n + 1 ? `${s.slice(0, n)}…${s.slice(-n)}` : s);
