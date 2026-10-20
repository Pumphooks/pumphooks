// hook-router (ROUTER engine) encoders / decoders. Router hooks run only on trades routed
// through this program; the venue itself stays directly callable.

import { ammV2Metas } from "../pump.js";
import { anchorDiscriminator, b58, bytes32, compareBytes, concat, findPda, i64, meta, pk, seed, u64, u8, vecU8, type InstructionV1, type KeyLike } from "./ix.js";
import { Environment } from "./stack.js";
import { programData } from "./registry.js";
import { SPL_TOKEN_PROGRAM_ID, SYSTEM_PROGRAM_ID } from "./tokens.js";

export const ROUTER_PROGRAM_ID = "2a3wWdS8z9g7vBcYWSjdETyEzo7DEjV3iYRZFsM52q4U";
export const PUMP_AMM_PROGRAM_ID = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
export const NATIVE_MINT = "So11111111111111111111111111111111111111112";

/** Venue indices (`hook_router::venues`). Every venue starts disabled. */
export const Venue = {
  PumpSwap: 0, RaydiumCpmm: 1, RaydiumCpmmDevnet: 2, MeteoraDammV2: 3, RaydiumClmm: 4, RaydiumClmmDevnet: 5, OrcaWhirlpool: 6, MeteoraDlmm: 7,
} as const;

export const RouterCallback = { BeforeExecute: 0, AfterExecute: 1, BeforeSwap: 2, AfterSwap: 3 } as const;

export const routerPda = {
  config: () => findPda([seed("router-config")], ROUTER_PROGRAM_ID)[0],
  policy: (authority: KeyLike, policySeed: bigint) => findPda([seed("route-policy"), bytes32(authority), u64(policySeed)], ROUTER_PROGRAM_ID)[0],
  receipt: (user: KeyLike, nonce: bigint) => findPda([seed("receipt"), bytes32(user), u64(nonce)], ROUTER_PROGRAM_ID)[0],
};

/** Only the router's upgrade authority can initialize (no front-running the config). */
export function initializeRouter(admin: KeyLike): InstructionV1 {
  return {
    programId: ROUTER_PROGRAM_ID,
    accounts: [meta(admin, true, true), meta(routerPda.config(), true), meta(ROUTER_PROGRAM_ID), meta(programData(ROUTER_PROGRAM_ID)), meta(SYSTEM_PROGRAM_ID)],
    data: anchorDiscriminator("initialize"),
  };
}

export function setVenue(admin: KeyLike, venue: number, enabled: boolean): InstructionV1 {
  return {
    programId: ROUTER_PROGRAM_ID,
    accounts: [meta(admin, false, true), meta(routerPda.config(), true)],
    data: concat(anchorDiscriminator("set_venue"), u8(venue), u8(enabled ? 1 : 0)),
  };
}

const sortedVersions = (versions: KeyLike[]) => [...new Set(versions.map(b58))].sort((a, b) => compareBytes(bytes32(a), bytes32(b))).map((v) => meta(v));

/** `versions`: the registry `HookVersion` of every stack entry. `shared`: whether other users may route under it. */
export function createRoutePolicy(a: { authority: KeyLike; seed: bigint; delaySecs?: bigint; shared?: boolean; stack: Uint8Array; versions: KeyLike[] }): InstructionV1 {
  if (a.stack[1] !== Environment.Router) throw new Error("route policies take router-environment stacks");
  return {
    programId: ROUTER_PROGRAM_ID,
    accounts: [meta(a.authority, true, true), meta(routerPda.policy(a.authority, a.seed), true), meta(SYSTEM_PROGRAM_ID), ...sortedVersions(a.versions)],
    data: concat(anchorDiscriminator("create_policy"), u64(a.seed), i64(a.delaySecs ?? 0n), u8(a.shared === false ? 0 : 1), vecU8(a.stack)),
  };
}

export interface RouteArgs {
  user: KeyLike;
  /** Route policy whose hook stack runs; omit for an unhooked route. */
  policy?: KeyLike;
  userInput: KeyLike;
  userOutput: KeyLike;
  /** The venue instruction's accounts, in order. */
  venueAccounts: InstructionV1["accounts"];
  venueProgram: KeyLike;
  venueData: Uint8Array;
  /** Accounts forwarded to program hooks (after the venue program). */
  hookAccounts?: InstructionV1["accounts"];
  maxIn: bigint;
  minOut: bigint;
  /** Cap on SOL the venue may take from the user's wallet. */
  maxLamports: bigint;
  deadline?: bigint;
  nonce: bigint;
}

/** `hook_router::route`: measured max-in / min-out, receipt at `[receipt, user, nonce]`. */
export function route(a: RouteArgs): InstructionV1 {
  if (a.venueAccounts.length > 255) throw new Error("too many venue accounts");
  const venue = a.venueAccounts.map((m) => ({ ...m, signer: false }));
  return {
    programId: ROUTER_PROGRAM_ID,
    accounts: [
      meta(a.user, true, true),
      meta(routerPda.config()),
      a.policy ? meta(a.policy, true) : meta(ROUTER_PROGRAM_ID),
      meta(a.userInput, true),
      meta(a.userOutput, true),
      meta(routerPda.receipt(a.user, a.nonce), true),
      meta(SYSTEM_PROGRAM_ID),
      ...venue,
      meta(a.venueProgram),
      ...(a.hookAccounts ?? []),
    ],
    data: concat(
      anchorDiscriminator("route"),
      u8(a.venueAccounts.length),
      u64(a.maxIn),
      u64(a.minOut),
      u64(a.maxLamports),
      i64(a.deadline ?? 0x7fff_ffff_ffff_ffffn),
      u64(a.nonce),
      vecU8(a.venueData),
    ),
  };
}

export interface Receipt {
  user: string;
  policy: string;
  venue: number;
  venueVerified: boolean;
  inputMint: string;
  outputMint: string;
  spent: bigint;
  received: bigint;
  lamportsSpent: bigint;
  maxIn: bigint;
  minOut: bigint;
  slot: bigint;
  timestamp: bigint;
  nonce: bigint;
}

export function decodeReceipt(data: Uint8Array): Receipt {
  const v = new DataView(data.buffer, data.byteOffset, data.byteLength);
  const key = (o: number) => b58(data.slice(o, o + 32));
  return {
    user: key(8),
    policy: key(40),
    venue: data[72],
    venueVerified: data[73] === 1,
    inputMint: key(74),
    outputMint: key(106),
    spent: v.getBigUint64(138, true),
    received: v.getBigUint64(146, true),
    lamportsSpent: v.getBigUint64(154, true),
    maxIn: v.getBigUint64(162, true),
    minOut: v.getBigUint64(170, true),
    slot: v.getBigUint64(178, true),
    timestamp: v.getBigInt64(186, true),
    nonce: v.getBigUint64(194, true),
  };
}

/** PumpSwap `buy_v2` / `buy_exact_quote_in_v2` / `sell_v2` (discriminators: `pump_adapter::ix::disc`). */
export const pumpSwap = {
  /** `buy-exact-quote-in` spends `amount` quote for at least `limit` base; `sell` sells `amount` base for at least `limit` quote. */
  data(mode: "buy-exact-base-out" | "buy-exact-quote-in" | "sell", amount: bigint, limit: bigint): Uint8Array {
    const disc = {
      "buy-exact-base-out": [184, 23, 238, 97, 103, 197, 211, 61],
      "buy-exact-quote-in": [194, 171, 28, 70, 104, 77, 91, 47],
      sell: [93, 246, 130, 60, 231, 233, 64, 178],
    }[mode];
    return concat(Uint8Array.from(disc), u64(amount), u64(limit));
  },
  /** The 17 trade accounts for a SOL-quoted pool. */
  accounts(k: { pool: KeyLike; user: KeyLike; baseMint: KeyLike; poolBaseTokenAccount: KeyLike; poolQuoteTokenAccount: KeyLike; baseTokenProgram: KeyLike; buybackRecipient: KeyLike }): InstructionV1["accounts"] {
    const keys = {
      pool: pk(k.pool),
      baseMint: pk(k.baseMint),
      quoteMint: pk(NATIVE_MINT),
      poolBaseTokenAccount: pk(k.poolBaseTokenAccount),
      poolQuoteTokenAccount: pk(k.poolQuoteTokenAccount),
      baseTokenProgram: pk(k.baseTokenProgram),
      quoteTokenProgram: pk(SPL_TOKEN_PROGRAM_ID),
    };
    return ammV2Metas(keys, pk(k.user), pk(k.buybackRecipient)).map((m) => meta(m.pubkey, m.isWritable, m.isSigner));
  },
};
