"use client";

import { useEffect, useState } from "react";
import { hooksPda, type HookDefinition } from "@pump-hooks/sdk";
import { useNetwork } from "./Providers";
import { CLUSTERS } from "@/lib/cluster";

export type RegistryState = { status: "loading" } | { status: "unavailable"; reason: string } | { status: "ok"; defs: Map<number, HookDefinition> };

/** On-chain HookDefinitions for every module in the catalog, on the selected cluster. */
export function useRegistry(ids: number[]): RegistryState {
  const { client, connection, program } = useNetwork();
  const [s, setS] = useState<RegistryState>({ status: "loading" });
  const key = ids.join(",");
  useEffect(() => {
    if (program.state === "loading") return setS({ status: "loading" });
    if (program.state !== "live") return setS({ status: "unavailable", reason: program.state === "not-deployed" ? "not deployed on this cluster" : program.state });
    let live = true;
    (async () => {
      const defs = new Map<number, HookDefinition>();
      await Promise.all(
        ids.map(async (id) => {
          const d = await client.getHookDefinition(id).catch(() => undefined);
          if (d) defs.set(id, d);
        }),
      );
      if (live) setS({ status: "ok", defs });
    })();
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, connection, program.state, key]);
  return s;
}

export interface ModuleActivity {
  executions: number;
  denials: number;
}

/** Per-module activity from the indexer service, when one is configured for this cluster. */
export function useModuleActivity(): Map<number, ModuleActivity> | undefined {
  const { cluster } = useNetwork();
  const url = CLUSTERS[cluster].indexer;
  const [m, setM] = useState<Map<number, ModuleActivity>>();
  useEffect(() => {
    setM(undefined);
    if (!url) return;
    let live = true;
    fetch(`${url.replace(/\/$/, "")}/modules`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((rows: ({ moduleId: number } & ModuleActivity)[]) => live && setM(new Map(rows.map((r) => [r.moduleId, r]))))
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [url]);
  return m;
}

export const definitionAddress = (id: number) => hooksPda.hookDefinition(id).toBase58();
