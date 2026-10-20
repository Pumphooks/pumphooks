/** The few SPL Token / ATA instructions routed trades need (wSOL wrap/unwrap, idempotent ATAs). */
import { Buffer } from "buffer";
import { PublicKey, SystemProgram, TransactionInstruction } from "@solana/web3.js";
import { ATA_PROGRAM_ID, NATIVE_MINT, SYSTEM_PROGRAM_ID, TOKEN_PROGRAM_ID } from "./constants.js";
import { associatedToken } from "./pda.js";

export function createAtaIdempotentIx(payer: PublicKey, owner: PublicKey, mint: PublicKey, tokenProgram: PublicKey) {
  return new TransactionInstruction({
    programId: ATA_PROGRAM_ID,
    keys: [
      { pubkey: payer, isSigner: true, isWritable: true },
      { pubkey: associatedToken(owner, tokenProgram, mint), isSigner: false, isWritable: true },
      { pubkey: owner, isSigner: false, isWritable: false },
      { pubkey: mint, isSigner: false, isWritable: false },
      { pubkey: SYSTEM_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: tokenProgram, isSigner: false, isWritable: false },
    ],
    data: Buffer.from([1]),
  });
}

export function syncNativeIx(account: PublicKey) {
  return new TransactionInstruction({
    programId: TOKEN_PROGRAM_ID,
    keys: [{ pubkey: account, isSigner: false, isWritable: true }],
    data: Buffer.from([17]),
  });
}

export function closeAccountIx(account: PublicKey, destination: PublicKey, owner: PublicKey, tokenProgram = TOKEN_PROGRAM_ID) {
  return new TransactionInstruction({
    programId: tokenProgram,
    keys: [
      { pubkey: account, isSigner: false, isWritable: true },
      { pubkey: destination, isSigner: false, isWritable: true },
      { pubkey: owner, isSigner: true, isWritable: false },
    ],
    data: Buffer.from([9]),
  });
}

/** Create the wallet's wSOL ATA (idempotent) and fund it with `lamports`. */
export function wrapSolIxs(owner: PublicKey, lamports: bigint): TransactionInstruction[] {
  const ata = associatedToken(owner, TOKEN_PROGRAM_ID, NATIVE_MINT);
  const out = [createAtaIdempotentIx(owner, owner, NATIVE_MINT, TOKEN_PROGRAM_ID)];
  if (lamports > 0n) {
    out.push(SystemProgram.transfer({ fromPubkey: owner, toPubkey: ata, lamports }), syncNativeIx(ata));
  }
  return out;
}
