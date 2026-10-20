"use client";

import { useEffect, useState } from "react";
import { hooksPda, type HookDefinition } from "@pump-hooks/sdk";
import { useNetwork } from "./Providers";

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

export const definitionAddress = (id: number) => hooksPda.hookDefinition(id).toBase58();
