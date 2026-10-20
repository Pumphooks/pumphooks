// smart-accounts (ACCOUNT engine) encoders. Funds sit in a system-owned vault PDA that only this
// program signs for; every withdrawal and execute runs the account's hook stack.

import { sha256 } from "@noble/hashes/sha256";
import { anchorDiscriminator, b58, bytes32, concat, findPda, i64, meta, seed, u32, u64, u8, vecU8, type InstructionV1, type KeyLike } from "./ix.js";
import { Environment } from "./stack.js";
import { SYSTEM_PROGRAM_ID } from "./tokens.js";

export const SMART_ACCOUNTS_PROGRAM_ID = "6AEiwvifk4TVJ8g3UoVXqhaK8AqYy43A4DzWCgMmrtQw";

export const AccountCallback = {
  BeforeDeposit: 0, AfterDeposit: 1, BeforeWithdraw: 2, AfterWithdraw: 3, BeforeExecute: 4, AfterExecute: 5, BeforeAuthorityChange: 6, AfterAuthorityChange: 7,
} as const;

export const smartAccountPda = {
  account: (payer: KeyLike, accountSeed: bigint) => findPda([seed("smart-account"), bytes32(payer), u64(accountSeed)], SMART_ACCOUNTS_PROGRAM_ID)[0],
  vault: (account: KeyLike) => findPda([seed("vault"), bytes32(account)], SMART_ACCOUNTS_PROGRAM_ID)[0],
  proposal: (account: KeyLike, index: bigint) => findPda([seed("proposal"), bytes32(account), u64(index)], SMART_ACCOUNTS_PROGRAM_ID)[0],
  session: (account: KeyLike, key: KeyLike) => findPda([seed("session"), bytes32(account), bytes32(key)], SMART_ACCOUNTS_PROGRAM_ID)[0],
};

const NONE = meta(SMART_ACCOUNTS_PROGRAM_ID);
const ZERO = new Uint8Array(32);

export interface CreateAccountArgs {
  /** Pays rent and keys the account address with `seed`. */
  payer: KeyLike;
  seed: bigint;
  owner: KeyLike;
  recovery?: KeyLike;
  recoveryDelaySecs?: bigint;
  /** Delay before a mandatory-entry change applies. */
  policyDelaySecs: bigint;
  members?: KeyLike[];
  stack: Uint8Array;
  /** Registry `HookVersion` of every stack entry. */
  versions: KeyLike[];
}

export function createSmartAccount(a: CreateAccountArgs): InstructionV1 {
  if (a.stack[1] !== Environment.Account) throw new Error("smart accounts take account-environment stacks");
  const account = smartAccountPda.account(a.payer, a.seed);
  const members = a.members ?? [];
  return {
    programId: SMART_ACCOUNTS_PROGRAM_ID,
    accounts: [meta(a.payer, true, true), meta(account, true), meta(smartAccountPda.vault(account)), meta(SYSTEM_PROGRAM_ID), ...[...new Set(a.versions.map(b58))].map((v) => meta(v))],
    data: concat(
      anchorDiscriminator("create_account"),
      u64(a.seed),
      bytes32(a.owner),
      a.recovery ? bytes32(a.recovery) : ZERO,
      i64(a.recoveryDelaySecs ?? 0n),
      i64(a.policyDelaySecs),
      u32(members.length),
      ...members.map(bytes32),
      vecU8(a.stack),
    ),
  };
}

/** SOL deposit; anyone may deposit. */
export function depositSol(a: { depositor: KeyLike; account: KeyLike; amount: bigint }): InstructionV1 {
  return {
    programId: SMART_ACCOUNTS_PROGRAM_ID,
    accounts: [meta(a.depositor, true, true), meta(a.account, true), meta(smartAccountPda.vault(a.account), true), NONE, NONE, NONE, NONE, meta(SYSTEM_PROGRAM_ID)],
    data: concat(anchorDiscriminator("deposit"), u64(a.amount)),
  };
}

/**
 * SOL withdrawal by the owner (or a session key with `session`). Timelocked stacks need an aged
 * `proposal` whose action hash matches exactly this withdrawal. `extra`: hook programs, their
 * callback authorities, state records and member signers.
 */
export function withdrawSol(a: { authority: KeyLike; account: KeyLike; destination: KeyLike; amount: bigint; proposal?: KeyLike; session?: KeyLike; extra?: InstructionV1["accounts"] }): InstructionV1 {
  return {
    programId: SMART_ACCOUNTS_PROGRAM_ID,
    accounts: [
      meta(a.authority, false, true),
      meta(a.account, true),
      meta(smartAccountPda.vault(a.account), true),
      meta(a.destination, true),
      a.session ? meta(a.session) : NONE,
      a.proposal ? meta(a.proposal, true) : NONE,
      NONE,
      NONE,
      NONE,
      meta(SYSTEM_PROGRAM_ID),
      ...(a.extra ?? []),
    ],
    data: concat(anchorDiscriminator("withdraw"), u64(a.amount)),
  };
}

/** `sha256(borsh(Action::Withdraw { asset, amount, destination }))`; asset = zero key for SOL. */
export function withdrawActionHash(a: { asset?: KeyLike; amount: bigint; destination: KeyLike }): Uint8Array {
  return sha256(concat(u8(0), a.asset ? bytes32(a.asset) : ZERO, u64(a.amount), bytes32(a.destination)));
}

/** Proposal at the account's current `proposalCount`; the timelock counts from its creation. */
export function propose(a: { proposer: KeyLike; account: KeyLike; index: bigint; actionHash: Uint8Array }): InstructionV1 {
  if (a.actionHash.length !== 32) throw new Error("action hash must be 32 bytes");
  return {
    programId: SMART_ACCOUNTS_PROGRAM_ID,
    accounts: [meta(a.proposer, true, true), meta(a.account, true), meta(smartAccountPda.proposal(a.account, a.index), true), meta(SYSTEM_PROGRAM_ID)],
    data: concat(anchorDiscriminator("propose"), a.actionHash),
  };
}
