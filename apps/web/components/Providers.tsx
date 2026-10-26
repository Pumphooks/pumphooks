"use client";

import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { Connection, PublicKey } from "@solana/web3.js";
import { WalletProvider } from "@solana/wallet-adapter-react";
import { decodeGlobalConfig, hooksPda, PUMP_HOOKS_PROGRAM_ID, PumpHooksClient, type GlobalConfig } from "@pump-hooks/sdk";
import { CLUSTERS, DEFAULT_CLUSTER, rpcEndpoint, type Cluster } from "@/lib/cluster";
import { PRIORITY_MICRO_LAMPORTS } from "@/lib/tx";

export interface ProgramStatus {
  state: "loading" | "not-deployed" | "uninitialized" | "live" | "error";
  global?: GlobalConfig;
  error?: string;
}

interface Ctx {
  cluster: Cluster;
  setCluster: (c: Cluster) => void;
  connection: Connection;
  client: PumpHooksClient;
  program: ProgramStatus;
  refreshProgram: () => void;
  writesEnabled: boolean;
}

const NetworkContext = createContext<Ctx | null>(null);

export function useNetwork(): Ctx {
  const c = useContext(NetworkContext);
  if (!c) throw new Error("useNetwork outside <Providers>");
  return c;
}

export function Providers({ children }: { children: ReactNode }) {
  const [cluster, setClusterState] = useState<Cluster>(DEFAULT_CLUSTER);
  useEffect(() => {
    const saved = window.localStorage.getItem("ph.cluster");
    if (saved === "devnet" || saved === "mainnet-beta") setClusterState(saved);
  }, []);
  const setCluster = useCallback((c: Cluster) => {
    window.localStorage.setItem("ph.cluster", c);
    setClusterState(c);
  }, []);

  const cfg = CLUSTERS[cluster];
  const connection = useMemo(() => new Connection(rpcEndpoint(cluster), "confirmed"), [cluster]);
  const client = useMemo(
    () => new PumpHooksClient(connection, { lookupTables: [cfg.alt], microLamportsPerCu: PRIORITY_MICRO_LAMPORTS }),
    [connection, cfg.alt],
  );

  const [program, setProgram] = useState<ProgramStatus>({ state: "loading" });
  const [tick, setTick] = useState(0);
  useEffect(() => {
    let live = true;
    setProgram({ state: "loading" });
    (async () => {
      try {
        const [prog, global] = await connection.getMultipleAccountsInfo([PUMP_HOOKS_PROGRAM_ID, hooksPda.global()]);
        if (!live) return;
        if (!prog?.executable) return setProgram({ state: "not-deployed" });
        if (!global) return setProgram({ state: "uninitialized" });
        setProgram({ state: "live", global: decodeGlobalConfig(global.data) });
      } catch (e) {
        if (live) setProgram({ state: "error", error: (e as Error).message });
      }
    })();
    return () => {
      live = false;
    };
  }, [connection, tick]);

  const value: Ctx = {
    cluster,
    setCluster,
    connection,
    client,
    program,
    refreshProgram: () => setTick((t) => t + 1),
    writesEnabled: cfg.writes && program.state === "live" && !program.global?.paused,
  };

  return (
    <NetworkContext.Provider value={value}>
      <WalletProvider wallets={[]} autoConnect>
        {children}
      </WalletProvider>
    </NetworkContext.Provider>
  );
}

export const isPubkey = (s: string) => {
  try {
    return new PublicKey(s.trim()).toBase58() === s.trim();
  } catch {
    return false;
  }
};
