// Resolves a coin metadata URI to its image link. Many metadata hosts (ipfs.io included) refuse
// cross-origin fetches, so the JSON is read here; the image itself loads directly in an <img>.
const MAX_BYTES = 64 * 1024;
const TIMEOUT_MS = 5000;
// ipfs.io, dweb.link and w3s.link now only serve a service-worker gateway (HTTP 429 for plain fetches).
const IPFS_GATEWAY = "https://ipfs.filebase.io/ipfs/";
const RETIRED_GATEWAYS = new Set(["ipfs.io", "dweb.link", "w3s.link", "cloudflare-ipfs.com"]);

function allowed(raw: string | null): URL | undefined {
  if (!raw) return undefined;
  try {
    let u = new URL(raw.startsWith("ipfs://") ? `${IPFS_GATEWAY}${raw.slice(7)}` : raw);
    if (RETIRED_GATEWAYS.has(u.hostname) && u.pathname.startsWith("/ipfs/")) u = new URL(`${IPFS_GATEWAY}${u.pathname.slice(6)}${u.search}`);
    if (u.protocol !== "https:" || u.port) return undefined;
    const h = u.hostname;
    if (h === "localhost" || h.endsWith(".local") || h.endsWith(".internal") || /^[\d.]+$/.test(h) || h.includes(":")) return undefined;
    return u;
  } catch {
    return undefined;
  }
}

export async function GET(req: Request) {
  const uri = allowed(new URL(req.url).searchParams.get("uri"));
  if (!uri) return Response.json({ error: "uri must be a public https link" }, { status: 400 });
  try {
    const res = await fetch(uri, { signal: AbortSignal.timeout(TIMEOUT_MS), redirect: "follow", headers: { accept: "application/json" } });
    if (!res.ok || !res.body) return Response.json({ error: `metadata host answered ${res.status}` }, { status: 502 });
    const reader = res.body.getReader();
    const chunks: Uint8Array[] = [];
    let size = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.length;
      if (size > MAX_BYTES) {
        await reader.cancel();
        return Response.json({ error: "metadata too large" }, { status: 502 });
      }
      chunks.push(value);
    }
    const j = JSON.parse(Buffer.concat(chunks).toString("utf8")) as { image?: unknown };
    const image = typeof j.image === "string" ? allowed(j.image)?.toString() : undefined;
    return Response.json({ image: image ?? null }, { headers: { "cache-control": "public, s-maxage=86400, stale-while-revalidate=604800" } });
  } catch {
    return Response.json({ error: "could not read metadata" }, { status: 502 });
  }
}
