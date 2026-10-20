// token-hooks (Token-2022 transfer-hook engine) and token-wrapper encoders / decoders.

import { anchorDiscriminator, b58, bytes32, concat, findPda, i64, meta, seed, u64, u8, vecU8, type InstructionV1, type KeyLike } from "./ix.js";
import { decodeStack, Environment, type DecodedStack } from "./stack.js";

export const TOKEN_HOOKS_PROGRAM_ID = "FZBBkQQamgfMCAoNk6PAdZi9nYZGMsGw9gDyTMNC3v54";
export const TOKEN_WRAPPER_PROGRAM_ID = "841vHKyD8HXQKsNSp2A2pzNjDHuFDBu9JPt6xsJc7vbh";
export const TOKEN_2022_PROGRAM_ID = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
export const SPL_TOKEN_PROGRAM_ID = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
export const SYSTEM_PROGRAM_ID = "11111111111111111111111111111111";
export const GATEWAY_PROGRAM_ID = "gatem74V238djXdzWnJf94Wo1DcnuGkfijbf3AuBhfs";

export const TokenCallback = { BeforeTransfer: 0, AfterTransfer: 1, BeforeWrap: 2, BeforeUnwrap: 3 } as const;

export const tokenHooksPda = {
  policy: (mint: KeyLike, program: KeyLike = TOKEN_HOOKS_PROGRAM_ID) => findPda([seed("token-policy"), bytes32(mint)], program),
  extraAccountMetas: (mint: KeyLike, program: KeyLike = TOKEN_HOOKS_PROGRAM_ID) => findPda([seed("extra-account-metas"), bytes32(mint)], program),
  /** Civic gateway token of `holder` on `network`. */
  gatewayToken: (holder: KeyLike, network: KeyLike) =>
    findPda([bytes32(holder), seed("gateway"), new Uint8Array(8), bytes32(network)], GATEWAY_PROGRAM_ID),
};

export const wrapperPda = {
  wrapper: (underlying: KeyLike) => findPda([seed("wrapper"), bytes32(underlying)], TOKEN_WRAPPER_PROGRAM_ID),
  wrappedMint: (underlying: KeyLike) => findPda([seed("wrapped-mint"), bytes32(underlying)], TOKEN_WRAPPER_PROGRAM_ID),
  vault: (underlying: KeyLike) => findPda([seed("vault"), bytes32(underlying)], TOKEN_WRAPPER_PROGRAM_ID),
};

/**
 * `token_hooks::create_policy`. The signer `authority` must be the mint's transfer-hook authority.
 * `versions`: the registry `HookVersion` account of every entry (remaining accounts).
 */
export function createTokenPolicy(a: { payer: KeyLike; authority: KeyLike; mint: KeyLike; delaySecs?: bigint; stack: Uint8Array; versions?: KeyLike[] }): InstructionV1 {
  if (a.stack[1] !== Environment.Token) throw new Error("token policies take token-environment stacks");
  return {
    programId: TOKEN_HOOKS_PROGRAM_ID,
    accounts: [
      meta(a.payer, true, true),
      meta(a.authority, false, true),
      meta(a.mint),
      meta(tokenHooksPda.policy(a.mint)[0], true),
      meta(tokenHooksPda.extraAccountMetas(a.mint)[0], true),
      meta(SYSTEM_PROGRAM_ID),
      ...(a.versions ?? []).map((v) => meta(v)),
    ],
    data: concat(anchorDiscriminator("create_policy"), i64(a.delaySecs ?? 0n), vecU8(a.stack)),
  };
}

/** `token_hooks::update_policy`: applies now, or stages behind the policy delay when a mandatory entry changes. */
export function updateTokenPolicy(a: { authority: KeyLike; mint: KeyLike; stack: Uint8Array; versions?: KeyLike[] }): InstructionV1 {
  return {
    programId: TOKEN_HOOKS_PROGRAM_ID,
    accounts: [
      meta(a.authority, true, true),
      meta(a.mint),
      meta(tokenHooksPda.policy(a.mint)[0], true),
      meta(tokenHooksPda.extraAccountMetas(a.mint)[0], true),
      meta(SYSTEM_PROGRAM_ID),
      ...(a.versions ?? []).map((v) => meta(v)),
    ],
    data: concat(anchorDiscriminator("update_policy"), vecU8(a.stack)),
  };
}

/**
 * Accounts Token-2022 needs appended to `transfer_checked` for a mint hooked by token-hooks:
 * the resolved extra metas (policy; per credential entry: gateway program, network, gateway
 * token), then the hook program and the metas list.
 */
export function transferHookAccounts(mint: KeyLike, stack: DecodedStack, source: { owner: KeyLike }, destination: { owner: KeyLike }) {
  const out = [meta(tokenHooksPda.policy(mint)[0], true)];
  let gateway = false;
  for (const e of stack.entries.filter((x) => x.builtinId === 13)) {
    if (!gateway) {
      out.push(meta(GATEWAY_PROGRAM_ID));
      gateway = true;
    }
    const network = e.config.slice(8, 40);
    out.push(meta(network));
    out.push(meta(tokenHooksPda.gatewayToken(e.config[0] === 0 ? source.owner : destination.owner, network)[0]));
  }
  out.push(meta(TOKEN_HOOKS_PROGRAM_ID), meta(tokenHooksPda.extraAccountMetas(mint)[0]));
  return out;
}

/** Token-2022 `transfer_checked` with `extra` appended. */
export function transferChecked(a: { source: KeyLike; mint: KeyLike; destination: KeyLike; owner: KeyLike; amount: bigint; decimals: number; extra?: InstructionV1["accounts"]; program?: string }): InstructionV1 {
  return {
    programId: a.program ?? TOKEN_2022_PROGRAM_ID,
    accounts: [meta(a.source, true), meta(a.mint), meta(a.destination, true), meta(a.owner, false, true), ...(a.extra ?? [])],
    data: concat(u8(12), u64(a.amount), u8(a.decimals)),
  };
}

export interface TokenPolicy {
  mint: string;
  authority: string;
  pendingHash: Uint8Array;
  sequence: bigint;
  delay: bigint;
  pendingAt: bigint;
  bump: number;
  metasBump: number;
  stack: DecodedStack;
}

export const TOKEN_POLICY_STACK_OFFSET = 136;

export function decodeTokenPolicy(data: Uint8Array): TokenPolicy {
  const disc = anchorDiscriminator("TokenPolicy", "account");
  if (data.length < TOKEN_POLICY_STACK_OFFSET + 8 || disc.some((b, i) => data[i] !== b)) throw new Error("not a TokenPolicy");
  const v = new DataView(data.buffer, data.byteOffset, data.byteLength);
  return {
    mint: b58(data.slice(8, 40)),
    authority: b58(data.slice(40, 72)),
    pendingHash: data.slice(72, 104),
    sequence: v.getBigUint64(104, true),
    delay: v.getBigInt64(112, true),
    pendingAt: v.getBigInt64(120, true),
    bump: data[128],
    metasBump: data[129],
    stack: decodeStack(data.slice(TOKEN_POLICY_STACK_OFFSET)),
  };
}

// ---------------------------------------------------------------- wrapper

/** `token_wrapper::create_wrapper`; follow with `createTokenPolicy` for the wrapped mint (same tx). */
export function createWrapper(a: { payer: KeyLike; underlyingMint: KeyLike; underlyingProgram: KeyLike; hookAuthority: KeyLike }): InstructionV1 {
  const u = a.underlyingMint;
  return {
    programId: TOKEN_WRAPPER_PROGRAM_ID,
    accounts: [
      meta(a.payer, true, true),
      meta(u),
      meta(wrapperPda.wrapper(u)[0], true),
      meta(wrapperPda.wrappedMint(u)[0], true),
      meta(wrapperPda.vault(u)[0], true),
      meta(a.underlyingProgram),
      meta(TOKEN_2022_PROGRAM_ID),
      meta(SYSTEM_PROGRAM_ID),
    ],
    data: concat(anchorDiscriminator("create_wrapper"), bytes32(a.hookAuthority)),
  };
}

function operate(name: "wrap" | "unwrap", a: { user: KeyLike; underlyingMint: KeyLike; underlyingProgram: KeyLike; userUnderlying: KeyLike; userWrapped: KeyLike; amount: bigint; minOut: bigint; remaining?: InstructionV1["accounts"] }): InstructionV1 {
  const u = a.underlyingMint;
  const wrapped = wrapperPda.wrappedMint(u)[0];
  return {
    programId: TOKEN_WRAPPER_PROGRAM_ID,
    accounts: [
      meta(a.user, false, true),
      meta(wrapperPda.wrapper(u)[0], true),
      meta(u),
      meta(wrapped, true),
      meta(wrapperPda.vault(u)[0], true),
      meta(a.userUnderlying, true),
      meta(a.userWrapped, true),
      meta(tokenHooksPda.policy(wrapped)[0], true),
      meta(a.underlyingProgram),
      meta(TOKEN_2022_PROGRAM_ID),
      meta(TOKEN_HOOKS_PROGRAM_ID),
      ...(a.remaining ?? []),
    ],
    data: concat(anchorDiscriminator(name), u64(a.amount), u64(a.minOut)),
  };
}

/** Deposits `amount` underlying; mints the vault's measured increase (≥ `minOut`). */
export const wrap = (a: Parameters<typeof operate>[1]) => operate("wrap", a);
/** Burns `amount` wrapped, then releases `amount` underlying (measured receipt ≥ `minOut`). */
export const unwrap = (a: Parameters<typeof operate>[1]) => operate("unwrap", a);

export function verifyReserves(underlyingMint: KeyLike): InstructionV1 {
  return {
    programId: TOKEN_WRAPPER_PROGRAM_ID,
    accounts: [meta(wrapperPda.wrapper(underlyingMint)[0]), meta(wrapperPda.wrappedMint(underlyingMint)[0]), meta(wrapperPda.vault(underlyingMint)[0])],
    data: anchorDiscriminator("verify_reserves"),
  };
}

export interface Wrapper {
  underlyingMint: string;
  underlyingProgram: string;
  wrappedMint: string;
  vault: string;
  /** null when the underlying has no freeze authority. */
  underlyingFreezeAuthority: string | null;
  totalWrapped: bigint;
  totalUnwrapped: bigint;
  decimals: number;
  bump: number;
}

export function decodeWrapper(data: Uint8Array): Wrapper {
  const disc = anchorDiscriminator("Wrapper", "account");
  if (data.length < 8 + 32 * 5 + 18 || disc.some((b, i) => data[i] !== b)) throw new Error("not a Wrapper");
  const v = new DataView(data.buffer, data.byteOffset, data.byteLength);
  const key = (o: number) => b58(data.slice(o, o + 32));
  const freeze = data.slice(136, 168);
  return {
    underlyingMint: key(8),
    underlyingProgram: key(40),
    wrappedMint: key(72),
    vault: key(104),
    underlyingFreezeAuthority: freeze.every((b) => b === 0) ? null : b58(freeze),
    totalWrapped: v.getBigUint64(168, true),
    totalUnwrapped: v.getBigUint64(176, true),
    decimals: data[184],
    bump: data[185],
  };
}

/** Token account amount (SPL Token / Token-2022 share bytes 64..72). */
export function tokenAmount(data: Uint8Array): bigint {
  return new DataView(data.buffer, data.byteOffset, data.byteLength).getBigUint64(64, true);
}
/** Mint supply (bytes 36..44). */
export function mintSupply(data: Uint8Array): bigint {
  return new DataView(data.buffer, data.byteOffset, data.byteLength).getBigUint64(36, true);
}
