import { Buffer } from "buffer";
import { PublicKey, type AccountInfo, type Connection } from "@solana/web3.js";
import { TOKEN_2022_PROGRAM_ID } from "@pump-hooks/sdk";

const METAPLEX = new PublicKey("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");
const TOKEN_METADATA_EXT = 19;

export interface TokenMeta {
  name: string;
  symbol: string;
  uri: string;
  source: "metaplex" | "token-2022";
}

const clean = (s: string) => s.replace(/\0/g, "").trim();

function borshStr(d: Uint8Array, o: number): [string, number] {
  const len = new DataView(d.buffer, d.byteOffset + o, 4).getUint32(0, true);
  return [new TextDecoder().decode(d.subarray(o + 4, o + 4 + len)), o + 4 + len];
}

/** Metaplex metadata: key(1) update_authority(32) mint(32) name symbol uri … */
function decodeMetaplex(d: Uint8Array): TokenMeta {
  let o = 65;
  const [name, a] = borshStr(d, o);
  const [symbol, b] = borshStr(d, (o = a));
  const [uri] = borshStr(d, (o = b));
  return { name: clean(name), symbol: clean(symbol), uri: clean(uri), source: "metaplex" };
}

/** Token-2022 TokenMetadata extension (TLV type 19) stored on the mint. */
function decodeToken2022(d: Uint8Array): TokenMeta | undefined {
  if (d.length <= 166) return undefined;
  const dv = new DataView(d.buffer, d.byteOffset, d.length);
  let o = 166;
  while (o + 4 <= d.length) {
    const type = dv.getUint16(o, true);
    const len = dv.getUint16(o + 2, true);
    if (type === TOKEN_METADATA_EXT) {
      let p = o + 4 + 64;
      const [name, a] = borshStr(d, p);
      const [symbol, b] = borshStr(d, (p = a));
      const [uri] = borshStr(d, (p = b));
      return { name: clean(name), symbol: clean(symbol), uri: clean(uri), source: "token-2022" };
    }
    if (type === 0 && len === 0) break;
    o += 4 + len;
  }
  return undefined;
}

export async function fetchTokenMeta(connection: Connection, mint: PublicKey, mintInfo?: AccountInfo<Buffer> | null): Promise<TokenMeta | undefined> {
  try {
    const pda = PublicKey.findProgramAddressSync([Buffer.from("metadata"), METAPLEX.toBuffer(), mint.toBuffer()], METAPLEX)[0];
    const [meta, m] = await connection.getMultipleAccountsInfo([pda, mintInfo === undefined ? mint : PublicKey.default]);
    const mintAi = mintInfo ?? m;
    if (mintAi?.owner.equals(TOKEN_2022_PROGRAM_ID)) {
      const t = decodeToken2022(mintAi.data);
      if (t) return t;
    }
    if (meta) return decodeMetaplex(meta.data);
  } catch {
    /* metadata is cosmetic */
  }
  return undefined;
}
