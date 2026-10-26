import { createHash } from "node:crypto";
import { put } from "@vercel/blob";
import { COIN_LIMITS } from "@pump-hooks/sdk";

// Coin metadata for /launch. pump.fun's own upload endpoint only answers its own frontend, so the
// image and the metadata JSON are stored in this project's Blob store under content-hash names.
const MAX_IMAGE = 2 * 1024 * 1024;
const MAX_DESCRIPTION = 1000;
const MAX_LINK = 200;

const IMAGE_TYPES: { type: string; ext: string; magic: (b: Uint8Array) => boolean }[] = [
  { type: "image/png", ext: "png", magic: (b) => b[0] === 0x89 && b[1] === 0x50 && b[2] === 0x4e && b[3] === 0x47 },
  { type: "image/jpeg", ext: "jpg", magic: (b) => b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff },
  { type: "image/gif", ext: "gif", magic: (b) => b[0] === 0x47 && b[1] === 0x49 && b[2] === 0x46 },
  { type: "image/webp", ext: "webp", magic: (b) => String.fromCharCode(...b.subarray(0, 4)) === "RIFF" && String.fromCharCode(...b.subarray(8, 12)) === "WEBP" },
];

const bad = (error: string, status = 400) => Response.json({ error }, { status });
const sha = (b: Uint8Array | string) => createHash("sha256").update(b).digest("hex");

function text(form: FormData, key: string, max: number, required = false): string | Error {
  const v = form.get(key);
  const s = typeof v === "string" ? v.trim() : "";
  if (required && !s) return new Error(`${key} is required`);
  if (Buffer.byteLength(s) > max) return new Error(`${key} is too long (max ${max} bytes)`);
  return s;
}

function link(form: FormData, key: string): string | undefined | Error {
  const s = text(form, key, MAX_LINK);
  if (s instanceof Error || !s) return s || undefined;
  try {
    const u = new URL(s);
    if (u.protocol !== "https:" && u.protocol !== "http:") throw new Error();
    return u.toString();
  } catch {
    return new Error(`${key} must be a http(s) link`);
  }
}

export async function POST(req: Request) {
  if (!process.env.BLOB_READ_WRITE_TOKEN) return bad("metadata storage is not configured", 503);
  if (Number(req.headers.get("content-length") ?? 0) > MAX_IMAGE + 16 * 1024) return bad("upload too large (image max 2 MB)", 413);
  let form: FormData;
  try {
    form = await req.formData();
  } catch {
    return bad("expected multipart form data");
  }

  const fields = {
    name: text(form, "name", COIN_LIMITS.name, true),
    symbol: text(form, "symbol", COIN_LIMITS.symbol, true),
    description: text(form, "description", MAX_DESCRIPTION),
    twitter: link(form, "twitter"),
    telegram: link(form, "telegram"),
    website: link(form, "website"),
  };
  for (const v of Object.values(fields)) if (v instanceof Error) return bad(v.message);

  const file = form.get("image");
  if (!(file instanceof File) || file.size === 0) return bad("image is required");
  if (file.size > MAX_IMAGE) return bad("image too large (max 2 MB)", 413);
  const bytes = new Uint8Array(await file.arrayBuffer());
  const kind = IMAGE_TYPES.find((t) => t.magic(bytes));
  if (!kind) return bad("image must be PNG, JPEG, GIF or WebP");

  const opts = { access: "public" as const, addRandomSuffix: false, allowOverwrite: true };
  const image = await put(`coins/${sha(bytes)}.${kind.ext}`, Buffer.from(bytes), { ...opts, contentType: kind.type });
  const meta: Record<string, unknown> = {
    name: fields.name,
    symbol: fields.symbol,
    description: fields.description,
    image: image.url,
    showName: true,
    createdOn: "https://pumphooks.fun",
  };
  for (const k of ["twitter", "telegram", "website"] as const) if (fields[k]) meta[k] = fields[k];
  const json = JSON.stringify(meta);
  const uri = await put(`coins/${sha(json)}.json`, json, { ...opts, contentType: "application/json" });
  if (Buffer.byteLength(uri.url) > COIN_LIMITS.uri) return bad("metadata link too long", 500);
  return Response.json({ uri: uri.url, image: image.url });
}
