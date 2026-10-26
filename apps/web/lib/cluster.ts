import { PUMP_ALT } from "@pump-hooks/sdk";

export type Cluster = "devnet" | "mainnet-beta";

export const CLUSTERS: Record<Cluster, { label: string; rpc: string; alt: typeof PUMP_ALT.devnet; writes: boolean; indexer?: string }> = {
  devnet: {
    label: "Devnet",
    rpc: process.env.NEXT_PUBLIC_DEVNET_RPC_URL ?? "https://api.devnet.solana.com",
    alt: PUMP_ALT.devnet,
    writes: true,
    indexer: process.env.NEXT_PUBLIC_DEVNET_INDEXER_URL || undefined,
  },
  "mainnet-beta": {
    label: "Mainnet",
    rpc: process.env.NEXT_PUBLIC_MAINNET_RPC_URL || "/api/rpc",
    alt: PUMP_ALT.mainnet,
    // Writes also need the program detected live and unpaused (Providers); "0" is an off switch.
    writes: process.env.NEXT_PUBLIC_ALLOW_MAINNET_WRITES !== "0",
    indexer: process.env.NEXT_PUBLIC_MAINNET_INDEXER_URL || undefined,
  },
};

export const DEFAULT_CLUSTER: Cluster = process.env.NEXT_PUBLIC_DEFAULT_CLUSTER === "devnet" ? "devnet" : "mainnet-beta";

/** Absolute RPC URL for `Connection` (relative entries resolve against this site). */
export function rpcEndpoint(cluster: Cluster) {
  const rpc = CLUSTERS[cluster].rpc;
  return rpc.startsWith("/") ? new URL(rpc, typeof window === "undefined" ? "http://localhost" : window.location.origin).toString() : rpc;
}

export function explorerUrl(cluster: Cluster, kind: "address" | "tx", id: string) {
  return `https://explorer.solana.com/${kind}/${id}${cluster === "devnet" ? "?cluster=devnet" : ""}`;
}
