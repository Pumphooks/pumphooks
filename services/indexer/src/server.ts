import { createServer, type Server } from "node:http";
import type { Indexer } from "./indexer.js";

const json = (v: unknown) => JSON.stringify(v, (_, x) => (typeof x === "bigint" ? x.toString() : x));

/**
 * Read-only HTTP API:
 *   GET /health                 cursor, last slot, transaction count
 *   GET /modules                per-module executions, denials, net installs
 *   GET /markets                per-market routed flow
 *   GET /markets/:address       one market + its 50 most recent records
 *   GET /tx/:signature          one record
 */
export function serve(indexer: Indexer, port: number, startedAt = Date.now()): Server {
  const { stats, store } = indexer;
  const server = createServer((req, res) => {
    const url = new URL(req.url ?? "/", "http://x");
    const send = (code: number, body: unknown) => {
      res.writeHead(code, { "content-type": "application/json", "access-control-allow-origin": "*" });
      res.end(json(body));
    };
    if (req.method !== "GET") return send(405, { error: "read-only" });
    const parts = url.pathname.split("/").filter(Boolean);
    switch (parts[0]) {
      case undefined:
      case "health":
        return send(200, { ok: true, cursor: store.cursor() ?? null, lastSlot: stats.lastSlot, transactions: stats.transactions, uptimeSecs: Math.round((Date.now() - startedAt) / 1000) });
      case "modules":
        return send(200, [...stats.modules.values()].sort((a, b) => a.moduleId - b.moduleId));
      case "markets": {
        if (!parts[1]) return send(200, [...stats.markets.values()].sort((a, b) => b.lastSlot - a.lastSlot));
        const m = stats.markets.get(parts[1]);
        if (!m) return send(404, { error: "market not indexed" });
        const recent = [...store.records()]
          .filter((r) => r.swap?.market === parts[1] || r.denied?.market === parts[1] || r.hooks[0]?.market === parts[1] || r.installs.some((i) => i.market === parts[1]))
          .slice(-50)
          .reverse();
        return send(200, { ...m, recent });
      }
      case "tx": {
        const r = parts[1] ? store.get(parts[1]) : undefined;
        return r ? send(200, r) : send(404, { error: "transaction not indexed" });
      }
      default:
        return send(404, { error: "not found" });
    }
  });
  server.listen(port);
  return server;
}
