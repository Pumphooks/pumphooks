// ABI v1 encoders for the token engine (token-hooks, token-wrapper) and hook stacks of every
// environment. Instructions are library-neutral (`InstructionV1`); convert with `toWeb3`.

import { PublicKey, TransactionInstruction } from "@solana/web3.js";
import { Buffer } from "buffer";
import type { InstructionV1 } from "./ix.js";

export * from "./ix.js";
export * from "./stack.js";
export * from "./tokens.js";

export function toWeb3(ix: InstructionV1): TransactionInstruction {
  return new TransactionInstruction({
    programId: new PublicKey(ix.programId),
    keys: ix.accounts.map((a) => ({ pubkey: new PublicKey(a.address), isSigner: a.signer, isWritable: a.writable })),
    data: Buffer.from(ix.data),
  });
}
