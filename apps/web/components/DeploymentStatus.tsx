"use client";

import { useEffect, useState } from "react";
import { Connection, PublicKey } from "@solana/web3.js";
import { CLUSTERS, explorerUrl, type Cluster } from "@/lib/cluster";

type State = "checking" | "deployed" | "absent" | `error:${string}`;

/** Live `getAccountInfo` per program and cluster: nothing here is assumed. */
export function DeploymentStatus({ programs }: { programs: { name: string; id: string }[] }) {
  const [state, setState] = useState<Record<string, State>>({});
  useEffect(() => {
    let live = true;
    for (const cluster of Object.keys(CLUSTERS) as Cluster[]) {
      const conn = new Connection(CLUSTERS[cluster].rpc, "confirmed");
      conn
        .getMultipleAccountsInfo(programs.map((p) => new PublicKey(p.id)))
        .then((infos) => {
          if (!live) return;
          setState((s) => ({ ...s, ...Object.fromEntries(programs.map((p, i) => [`${cluster}:${p.id}`, infos[i]?.executable ? "deployed" : "absent"])) }));
        })
        .catch((e: Error) => {
          const why = /403/.test(e.message) ? "RPC refused (403)" : "RPC error";
          if (live) setState((s) => ({ ...s, ...Object.fromEntries(programs.map((p) => [`${cluster}:${p.id}`, `error:${why}`])) }));
        });
    }
    return () => {
      live = false;
    };
  }, [programs]);
  const cell = (cluster: Cluster, id: string) => {
    const v = state[`${cluster}:${id}`] ?? "checking";
    const cls = v === "deployed" ? "ok" : v === "absent" ? "mute" : v.startsWith("error:") ? "warn" : "faint";
    const text = v === "deployed" ? "deployed" : v === "absent" ? "not deployed" : v.startsWith("error:") ? v.slice(6) : "checking…";
    return v === "deployed" ? (
      <a className={cls} href={explorerUrl(cluster, "address", id)} target="_blank" rel="noreferrer">
        {text}
      </a>
    ) : (
      <span className={cls}>{text}</span>
    );
  };
  return (
    <table className="table">
      <thead>
        <tr>
          <th>Program</th>
          <th>Address</th>
          <th>Devnet</th>
          <th>Mainnet</th>
        </tr>
      </thead>
      <tbody>
        {programs.map((p) => (
          <tr key={p.id}>
            <td>{p.name}</td>
            <td title={p.id}>{p.id}</td>
            <td>{cell("devnet", p.id)}</td>
            <td>{cell("mainnet-beta", p.id)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
