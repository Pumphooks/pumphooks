"use client";

import { capabilitiesOf } from "@pump-hooks/sdk";
import { definitionAddress, useRegistry } from "./Registry";
import { useNetwork } from "./Providers";
import { explorerUrl, CLUSTERS } from "@/lib/cluster";
import { short } from "@/lib/format";

const hex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");

export function RegistryPanel({ moduleId }: { moduleId: number }) {
  const reg = useRegistry([moduleId]);
  const { cluster } = useNetwork();
  const addr = definitionAddress(moduleId);
  if (reg.status === "loading") return <div className="skeleton" />;
  if (reg.status === "unavailable")
    return (
      <div className="notice">
        Pump Hooks is {reg.reason} on {CLUSTERS[cluster].label}. HookDefinition PDA: <span className="mono">{short(addr, 6)}</span>
      </div>
    );
  const d = reg.defs.get(moduleId);
  if (!d) return <div className="notice warn">Not registered on {CLUSTERS[cluster].label}.</div>;
  return (
    <div className="kv">
      <div>
        <div className="k">HookDefinition</div>
        <div className="v">
          <a href={explorerUrl(cluster, "address", addr)} target="_blank" rel="noreferrer">
            {short(addr, 6)}
          </a>
        </div>
      </div>
      <div>
        <div className="k">Status</div>
        <div className={`v ${d.enabled ? "ok" : "deny"}`}>{d.enabled ? "enabled" : "disabled"}</div>
      </div>
      <div>
        <div className="k">Installs</div>
        <div className="v">{d.installs.toString()}</div>
      </div>
      <div>
        <div className="k">Registered version</div>
        <div className="v">v{d.version}</div>
      </div>
      <div>
        <div className="k">Capabilities</div>
        <div className="v" title={capabilitiesOf(d.capabilities).join(", ")}>{capabilitiesOf(d.capabilities).length}</div>
      </div>
      <div>
        <div className="k">Audit status</div>
        <div className="v warn">{d.auditStatus === 0 ? "unaudited" : `code ${d.auditStatus}`}</div>
      </div>
      <div style={{ gridColumn: "1 / -1" }}>
        <div className="k">Source hash</div>
        <div className="v" style={{ whiteSpace: "normal", wordBreak: "break-all", fontSize: 11 }}>{hex(d.sourceHash)}</div>
      </div>
    </div>
  );
}
