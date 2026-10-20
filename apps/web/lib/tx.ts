import { Connection, PublicKey, TransactionInstruction, TransactionMessage, VersionedTransaction, type AddressLookupTableAccount } from "@solana/web3.js";
import type { WalletContextState } from "@solana/wallet-adapter-react";
import { executionResult, type ExecutionResult } from "@pump-hooks/sdk";

const MAX_TX = 1232;

function compile(ixs: TransactionInstruction[], payer: PublicKey, blockhash: string, alts: AddressLookupTableAccount[]) {
  return new VersionedTransaction(new TransactionMessage({ payerKey: payer, recentBlockhash: blockhash, instructions: ixs }).compileToV0Message(alts));
}

function fits(ixs: TransactionInstruction[], payer: PublicKey, alts: AddressLookupTableAccount[]) {
  try {
    return compile(ixs, payer, PublicKey.default.toBase58(), alts).serialize().length <= MAX_TX;
  } catch {
    return false;
  }
}

/** Greedily pack instructions, in order, into as few transactions as fit. Order is preserved. */
export function pack(ixs: TransactionInstruction[], payer: PublicKey, alts: AddressLookupTableAccount[] = []): TransactionInstruction[][] {
  const out: TransactionInstruction[][] = [];
  let cur: TransactionInstruction[] = [];
  for (const ix of ixs) {
    if (fits([...cur, ix], payer, alts)) cur.push(ix);
    else {
      if (cur.length === 0) throw new Error("a single instruction exceeds the transaction size limit");
      out.push(cur);
      cur = [ix];
    }
  }
  if (cur.length) out.push(cur);
  return out;
}

export interface SentTx {
  signature: string;
  result: ExecutionResult;
}

/** Sign with the connected wallet and send each batch sequentially, stopping at the first failure. */
export async function sendBatches(
  connection: Connection,
  wallet: WalletContextState,
  batches: TransactionInstruction[][],
  alts: AddressLookupTableAccount[] = [],
  onProgress?: (i: number, sig?: string) => void,
): Promise<SentTx[]> {
  if (!wallet.publicKey) throw new Error("connect a wallet first");
  const sent: SentTx[] = [];
  for (const [i, ixs] of batches.entries()) {
    onProgress?.(i);
    const { blockhash, lastValidBlockHeight } = await connection.getLatestBlockhash("confirmed");
    const tx = compile(ixs, wallet.publicKey, blockhash, alts);
    const signature = await wallet.sendTransaction(tx, connection, { preflightCommitment: "confirmed" });
    onProgress?.(i, signature);
    const conf = await connection.confirmTransaction({ signature, blockhash, lastValidBlockHeight }, "confirmed");
    const got = await connection.getTransaction(signature, { maxSupportedTransactionVersion: 0, commitment: "confirmed" });
    const result = executionResult(got?.meta?.logMessages ?? [], conf.value.err ?? got?.meta?.err, got?.meta?.computeUnitsConsumed ?? undefined);
    sent.push({ signature, result });
    if (!result.ok) break;
  }
  return sent;
}
