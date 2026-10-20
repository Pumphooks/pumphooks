import { anchorDiscriminator, b58, concat, findPda, InstructionV1, KeyLike, meta, pk, seed, u16, u32, u8, vecU8 } from "./ix.js";

export const REGISTRY_PROGRAM_ID = "9gKUqwnUa6ZJkCvNT4bnLY7A5czcJFuSV1dpYeosHHs4";
export const BPF_UPGRADEABLE_LOADER = "BPFLoaderUpgradeab1e11111111111111111111111";
const SYSTEM = "11111111111111111111111111111111";

export const MAX_SLUG = 32;

export const registryPda = {
  config: () => findPda([seed("config")], REGISTRY_PROGRAM_ID)[0],
  hook: (slug: string) => findPda([seed("hook"), seed(slug)], REGISTRY_PROGRAM_ID)[0],
  version: (hook: KeyLike, index: number) => findPda([seed("version"), pk(hook).toBytes(), u32(index)], REGISTRY_PROGRAM_ID)[0],
};

export const programData = (program: KeyLike) => findPda([pk(program).toBytes()], BPF_UPGRADEABLE_LOADER)[0];

export function validSlug(slug: string): boolean {
  return slug.length > 0 && slug.length <= MAX_SLUG && /^[a-z0-9-]+$/.test(slug);
}

export interface PublishParams {
  semver: [number, number, number];
  implKind: number;
  builtinId: number;
  /** Environment bitmask (bit = `Environment`). */
  environments: number;
  /** Callback mask per environment, index = `Environment`. */
  callbacks: [number, number, number, number];
  capabilities: number;
  configSchemaHash: Uint8Array;
  manifestHash: Uint8Array;
  manifestUri: string;
  artifactHash: Uint8Array;
}

function hash32(h: Uint8Array, what: string): Uint8Array {
  if (h.length !== 32) throw new Error(`${what} must be 32 bytes`);
  return h;
}

export function encodePublishParams(p: PublishParams): Uint8Array {
  return concat(
    ...p.semver.map(u16),
    u8(p.implKind),
    u8(p.builtinId),
    u8(p.environments),
    ...p.callbacks.map(u16),
    u32(p.capabilities),
    hash32(p.configSchemaHash, "configSchemaHash"),
    hash32(p.manifestHash, "manifestHash"),
    vecU8(new TextEncoder().encode(p.manifestUri)),
    hash32(p.artifactHash, "artifactHash"),
  );
}

export function createHook(authority: KeyLike, slug: string): InstructionV1 {
  if (!validSlug(slug)) throw new Error(`invalid slug "${slug}" (a-z, 0-9, '-', at most ${MAX_SLUG})`);
  return {
    programId: REGISTRY_PROGRAM_ID,
    accounts: [meta(authority, true, true), meta(registryPda.hook(slug), true), meta(SYSTEM)],
    data: concat(anchorDiscriminator("create_hook"), vecU8(new TextEncoder().encode(slug))),
  };
}

/**
 * `index` must be the hook's current `version_count`. Program versions name the hook program;
 * the publisher must be its upgrade authority (the registry reads the loader to check).
 */
export function publishVersion(o: { publisher: KeyLike; hook: KeyLike; index: number; program?: KeyLike; params: PublishParams }): InstructionV1 {
  const none = meta(REGISTRY_PROGRAM_ID);
  return {
    programId: REGISTRY_PROGRAM_ID,
    accounts: [
      meta(o.publisher, true, true),
      meta(o.hook, true),
      meta(registryPda.config()),
      meta(registryPda.version(o.hook, o.index), true),
      o.program ? meta(o.program) : none,
      o.program ? meta(programData(o.program)) : none,
      meta(SYSTEM),
    ],
    data: concat(anchorDiscriminator("publish_version"), encodePublishParams(o.params)),
  };
}

export interface HookAccount {
  authority: string;
  slug: string;
  versionCount: number;
  status: number;
}

export function decodeHook(data: Uint8Array): HookAccount {
  const v = new DataView(data.buffer, data.byteOffset, data.byteLength);
  let o = 8;
  const authority = b58(data.slice(o, o + 32));
  o += 32;
  o += data[o] === 1 ? 33 : 1;
  const len = v.getUint32(o, true);
  o += 4;
  const slug = new TextDecoder().decode(data.slice(o, o + len));
  o += len;
  const versionCount = v.getUint32(o, true);
  o += 4;
  o += data[o] === 1 ? 7 : 1;
  return { authority, slug, versionCount, status: data[o] };
}

export interface HookVersionAccount {
  hook: string;
  index: number;
  semver: [number, number, number];
  implKind: number;
  builtinId: number;
  program: string;
  environments: number;
  callbacks: [number, number, number, number];
  capabilities: number;
  manifestHash: Uint8Array;
  manifestUri: string;
  artifactHash: Uint8Array;
  observedDeploySlot: bigint;
  status: number;
  publisher: string;
}

export function decodeHookVersion(data: Uint8Array): HookVersionAccount {
  const v = new DataView(data.buffer, data.byteOffset, data.byteLength);
  let o = 8;
  const key = () => b58(data.slice(o, (o += 32)));
  const hook = key();
  const index = v.getUint32(o, true);
  o += 4;
  const semver = [0, 1, 2].map((i) => v.getUint16(o + 2 * i, true)) as [number, number, number];
  o += 6;
  const implKind = data[o++];
  const builtinId = data[o++];
  const program = key();
  const environments = data[o++];
  const callbacks = [0, 1, 2, 3].map((i) => v.getUint16(o + 2 * i, true)) as [number, number, number, number];
  o += 8;
  const capabilities = v.getUint32(o, true);
  o += 4;
  o += 32;
  const manifestHash = data.slice(o, (o += 32));
  const len = v.getUint32(o, true);
  o += 4;
  const manifestUri = new TextDecoder().decode(data.slice(o, (o += len)));
  const artifactHash = data.slice(o, (o += 32));
  o += 1;
  o += data[o] === 1 ? 33 : 1;
  const observedDeploySlot = v.getBigUint64(o, true);
  o += 8;
  const status = data[o++];
  const publisher = key();
  return { hook, index, semver, implKind, builtinId, program, environments, callbacks, capabilities, manifestHash, manifestUri, artifactHash, observedDeploySlot, status, publisher };
}
