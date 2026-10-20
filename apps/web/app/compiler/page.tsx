import type { Metadata } from "next";
import { compiler, short } from "@/lib/artifacts";
import { PageHead, Provenance, Section } from "@/components/Provenance";

export const metadata: Metadata = { title: "Transaction compiler" };

const MAX_TX_SIZE = 1232;
const PROGRAMS: Record<string, string> = {
  ComputeBudget111111111111111111111111111111: "compute-budget",
  TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb: "token-2022",
  TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA: "spl-token",
  "841vHKyD8HXQKsNSp2A2pzNjDHuFDBu9JPt6xsJc7vbh": "token-wrapper",
  FZBBkQQamgfMCAoNk6PAdZi9nYZGMsGw9gDyTMNC3v54: "token-hooks",
};
const fmt = (n: number | string) => Number(n).toLocaleString("en-US");

export default function CompilerPage() {
  const c = compiler.compiled;
  const role = (a: { signer: boolean; writable: boolean }) => `${a.signer ? "signer" : ""}${a.signer && a.writable ? " · " : ""}${a.writable ? "writable" : ""}` || "readonly";
  return (
    <main className="page">
      <PageHead n="09" crumb="Compiler" title="One plan, one transaction.">
        <code>@pump-hooks/transaction-compiler</code> turns operations across engines into a single atomic v0 transaction: dependency order, merged
        account roles, required signers, exact byte size and a compute budget. A plan that does not fit is refused with a reason; it is never split.
      </PageHead>

      <Section n="01" title="Plan → order" aside="wrap, hooked transfer, unwrap">
        <div className="grid-2">
          <div className="col">
            <table className="table">
              <thead>
                <tr>
                  <th>As written</th>
                  <th>Kind</th>
                  <th>After</th>
                </tr>
              </thead>
              <tbody>
                {compiler.plan.map((o) => (
                  <tr key={o.id}>
                    <td>{o.id}</td>
                    <td>{o.kind}</td>
                    <td className="mute">{o.after.join(", ") || "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="col">
            <table className="table">
              <thead>
                <tr>
                  <th>Compiled order</th>
                </tr>
              </thead>
              <tbody>
                {c.order.map((id, i) => (
                  <tr key={id}>
                    <td>
                      {i + 1}. {id}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            <table className="table" style={{ marginTop: 14 }}>
              <thead>
                <tr>
                  <th>Instruction</th>
                  <th>Program</th>
                  <th>Accounts</th>
                  <th>Data</th>
                </tr>
              </thead>
              <tbody>
                {c.instructions.map((ix, i) => (
                  <tr key={i}>
                    <td>{i}</td>
                    <td title={ix.programId}>{PROGRAMS[ix.programId] ?? short(ix.programId)}</td>
                    <td>{ix.accounts}</td>
                    <td>{ix.dataLen} B</td>
                  </tr>
                ))}
              </tbody>
            </table>
            <p className="mute" style={{ fontSize: 13 }}>
              The hooked transfer&apos;s extra accounts were resolved from the live token policy, not supplied by hand.
            </p>
          </div>
        </div>
        <div className="kv">
          <div>
            <div className="k">Size</div>
            <div className="v">
              {c.size} / {MAX_TX_SIZE} bytes
            </div>
          </div>
          <div>
            <div className="k">Instructions</div>
            <div className="v">{c.instructions.length} (incl. compute budget)</div>
          </div>
          <div>
            <div className="k">Accounts</div>
            <div className="v">{c.accounts.length}</div>
          </div>
          <div>
            <div className="k">Signers</div>
            <div className="v">{c.requiredSigners.length}</div>
          </div>
          <div>
            <div className="k">Static CU limit</div>
            <div className="v">{fmt(c.computeUnitLimit)}</div>
          </div>
          <div>
            <div className="k">Executed CU</div>
            <div className={`v ${compiler.executed.status === "success" ? "ok" : "deny"}`}>
              {fmt(compiler.executed.computeUnits)} ({compiler.executed.status})
            </div>
          </div>
          <div>
            <div className="k">Limit after simulation</div>
            <div className="v">{fmt(compiler.executed.measuredLimit)}</div>
          </div>
        </div>
        <div style={{ height: 10, background: "var(--paper-3)", marginTop: 14, position: "relative" }}>
          <div style={{ position: "absolute", inset: 0, width: `${(100 * c.size) / MAX_TX_SIZE}%`, background: "var(--ink)" }} />
        </div>
        <details>
          <summary>Account table ({c.accounts.length})</summary>
          <table className="table">
            <tbody>
              {c.accounts.map((a) => (
                <tr key={a.address}>
                  <td title={a.address}>{a.address}</td>
                  <td>{role(a)}</td>
                  <td className="mute">{a.lookupTable ? `via ${short(a.lookupTable)}` : "static"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      </Section>

      <Section n="02" title="Atomic failure">
        <p style={{ marginTop: 0 }}>
          A wrap followed by a transfer of more than the user holds. The transfer fails, so the wrap before it is undone too: only the fee payer&apos;s
          balance changes.
        </p>
        <div className="kv">
          <div>
            <div className="k">Status</div>
            <div className="v deny">{compiler.atomicFailure.status}</div>
          </div>
          <div style={{ gridColumn: "span 2" }}>
            <div className="k">Error</div>
            <div className="v" title={compiler.atomicFailure.error}>{compiler.atomicFailure.error}</div>
          </div>
          <div>
            <div className="k">Accounts changed</div>
            <div className="v">{compiler.atomicFailure.accountsChanged.join(", ")}</div>
          </div>
        </div>
      </Section>

      <Section n="03" title="Refusals" aside="typed errors, never a partial plan">
        <table className="table">
          <thead>
            <tr>
              <th>Plan</th>
              <th>Code</th>
              <th>Message</th>
            </tr>
          </thead>
          <tbody>
            {compiler.errors.map((e) => (
              <tr key={e.case}>
                <td>{e.case}</td>
                <td className="deny">{e.code}</td>
                <td style={{ fontFamily: "var(--display)", fontSize: 13 }}>{e.message}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <Provenance p={compiler} />
      </Section>
    </main>
  );
}
