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
    rpc: process.env.NEXT_PUBLIC_MAINNET_RPC_URL ?? "https://api.mainnet-beta.solana.com",
    alt: PUMP_ALT.mainnet,
    // Pump Hooks is not deployed on mainnet. Detection is read-only there.
    writes: process.env.NEXT_PUBLIC_ALLOW_MAINNET_WRITES === "1",
    indexer: process.env.NEXT_PUBLIC_MAINNET_INDEXER_URL || undefined,
  },
};

export const DEFAULT_CLUSTER: Cluster = process.env.NEXT_PUBLIC_DEFAULT_CLUSTER === "mainnet-beta" ? "mainnet-beta" : "devnet";

export function explorerUrl(cluster: Cluster, kind: "address" | "tx", id: string) {
  return `https://explorer.solana.com/${kind}/${id}${cluster === "devnet" ? "?cluster=devnet" : ""}`;
}
