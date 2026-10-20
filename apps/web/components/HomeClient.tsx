"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { hooksPda, MODULES } from "@pump-hooks/sdk";
import { isPubkey, useNetwork } from "./Providers";
import { CLUSTERS } from "@/lib/cluster";

export function CaBox() {
  const [v, setV] = useState("");
  const router = useRouter();
  const ok = isPubkey(v);
  return (
    <form
      className="ca-box"
      onSubmit={(e) => {
        e.preventDefault();
        if (ok) router.push(`/hook?mint=${v.trim()}`);
      }}
    >
      <input
        value={v}
        onChange={(e) => setV(e.target.value)}
        placeholder="Paste a Pump token address"
        spellCheck={false}
        autoComplete="off"
        aria-label="Pump token contract address"
      />
      <button type="submit" disabled={!ok} aria-label="Hook this token">
        ↑
      </button>
    </form>
  );
}

export function LiveStatus() {
  const { cluster, connection, program } = useNetwork();
  const [registered, setRegistered] = useState<number | undefined>();
  useEffect(() => {
    setRegistered(undefined);
    if (program.state !== "live") return;
    const ids = MODULES.filter((m) => m.available).map((m) => m.id);
    connection
      .getMultipleAccountsInfo(ids.map((id) => hooksPda.hookDefinition(id)))
      .then((a) => setRegistered(a.filter(Boolean).length))
      .catch(() => undefined);
  }, [connection, program.state]);

  const label = {
    loading: "Checking…",
    "not-deployed": "Not deployed",
    uninitialized: "Uninitialized",
    live: program.global?.paused ? "Paused" : "Live",
    error: "RPC error",
  }[program.state];
  const dot = program.state === "live" ? "live" : program.state === "loading" ? "wait pulse" : "off";

  return (
    <div className="status-strip">
      <div>
        <div className="k">Program · {CLUSTERS[cluster].label}</div>
        <div className="v">
          <span className={`dot ${dot}`} />
          {label}
        </div>
      </div>
      <div>
        <div className="k">Hooked markets</div>
        <div className="v mono">{program.global ? program.global.marketCount.toString() : "—"}</div>
      </div>
      <div>
        <div className="k">Registered hooks</div>
        <div className="v mono">{registered ?? "—"}</div>
      </div>
      <div>
        <div className="k">Audit</div>
        <div className="v">None yet</div>
      </div>
    </div>
  );
}
