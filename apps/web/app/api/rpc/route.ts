import { PUMP_HOOKS_PROGRAM_ID } from "@pump-hooks/sdk";

// api.mainnet-beta.solana.com answers 403 to any request carrying a browser Origin, so mainnet reads go through here.
// MAINNET_RPC_URL (server-only) keeps a provider key out of the bundle.
const UPSTREAM = process.env.MAINNET_RPC_URL || "https://api.mainnet-beta.solana.com";
const MAX_BODY = 64 * 1024;
const MAX_BATCH = 20;

const ALLOWED = new Set([
  "getAccountInfo",
  "getMultipleAccounts",
  "getProgramAccounts",
  "getBalance",
  "getTokenAccountBalance",
  "getTokenAccountsByOwner",
  "getTokenSupply",
  "getLatestBlockhash",
  "isBlockhashValid",
  "getBlockHeight",
  "getSlot",
  "getEpochInfo",
  "getFeeForMessage",
  "getMinimumBalanceForRentExemption",
  "getRecentPrioritizationFees",
  "getSignatureStatuses",
  "getSignaturesForAddress",
  "getTransaction",
  "simulateTransaction",
  "sendTransaction",
  "getVersion",
  "getGenesisHash",
  "getHealth",
]);

interface RpcRequest {
  jsonrpc?: string;
  id?: unknown;
  method?: unknown;
  params?: unknown;
}

function rejection(r: RpcRequest): string | undefined {
  if (typeof r?.method !== "string" || !ALLOWED.has(r.method)) return `method not allowed: ${String(r?.method)}`;
  // Unfiltered scans of large programs are the expensive call; only Pump Hooks' own accounts are scannable.
  if (r.method === "getProgramAccounts" && (!Array.isArray(r.params) || r.params[0] !== PUMP_HOOKS_PROGRAM_ID.toBase58())) {
    return "getProgramAccounts is limited to the Pump Hooks program";
  }
}

export async function POST(req: Request) {
  const text = await req.text();
  if (text.length > MAX_BODY) return Response.json({ error: "request too large" }, { status: 413 });
  let body: RpcRequest | RpcRequest[];
  try {
    body = JSON.parse(text);
  } catch {
    return Response.json({ jsonrpc: "2.0", id: null, error: { code: -32700, message: "parse error" } }, { status: 400 });
  }
  const list = Array.isArray(body) ? body : [body];
  if (list.length === 0 || list.length > MAX_BATCH) return Response.json({ error: "bad batch size" }, { status: 400 });
  for (const r of list) {
    const why = rejection(r);
    if (why) return Response.json({ jsonrpc: "2.0", id: r?.id ?? null, error: { code: -32601, message: why } }, { status: 403 });
  }
  const upstream = await fetch(UPSTREAM, { method: "POST", headers: { "content-type": "application/json" }, body: text, cache: "no-store" });
  return new Response(upstream.body, { status: upstream.status, headers: { "content-type": "application/json" } });
}
