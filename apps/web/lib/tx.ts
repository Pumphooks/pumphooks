import { ComputeBudgetProgram, Connection, PublicKey, TransactionInstruction, TransactionMessage, VersionedTransaction, type AddressLookupTableAccount } from "@solana/web3.js";
import type { WalletContextState } from "@solana/wallet-adapter-react";
import { executionResult, type ExecutionResult } from "@pump-hooks/sdk";

const MAX_TX = 1232;

/** Priority fee for every wallet-sent transaction: zero-priority transactions get dropped when Pump is busy. */
export const PRIORITY_MICRO_LAMPORTS = 200_000;

const SET_COMPUTE_UNIT_PRICE = 3;

function withPriority(ixs: TransactionInstruction[]) {
  const priced = ixs.some((i) => i.programId.equals(ComputeBudgetProgram.programId) && i.data[0] === SET_COMPUTE_UNIT_PRICE);
  return priced ? ixs : [ComputeBudgetProgram.setComputeUnitPrice({ microLamports: PRIORITY_MICRO_LAMPORTS }), ...ixs];
}

function compile(ixs: TransactionInstruction[], payer: PublicKey, blockhash: string, alts: AddressLookupTableAccount[]) {
  return new VersionedTransaction(
    new TransactionMessage({ payerKey: payer, recentBlockhash: blockhash, instructions: withPriority(ixs) }).compileToV0Message(alts),
  );
}

/** Polls signature status: the mainnet RPC is proxied over HTTP, so there is no websocket to subscribe on. */
async function confirm(connection: Connection, signature: string, lastValidBlockHeight: number) {
  for (;;) {
    const {
      value: [status],
    } = await connection.getSignatureStatuses([signature]);
    if (status?.err) return status.err;
    if (status?.confirmationStatus === "confirmed" || status?.confirmationStatus === "finalized") return null;
    if ((await connection.getBlockHeight("confirmed")) > lastValidBlockHeight) throw new Error(`transaction ${signature} expired before confirming; try again`);
    await new Promise((r) => setTimeout(r, 1500));
  }
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
    const err = await confirm(connection, signature, lastValidBlockHeight);
    const got = await connection.getTransaction(signature, { maxSupportedTransactionVersion: 0, commitment: "confirmed" });
    const result = executionResult(got?.meta?.logMessages ?? [], err ?? got?.meta?.err, got?.meta?.computeUnitsConsumed ?? undefined);
    sent.push({ signature, result });
    if (!result.ok) break;
  }
  return sent;
}
