"use client";

import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { useMemo, useState } from "react";
import { PublicKey } from "@solana/web3.js";
import { useWallet } from "@solana/wallet-adapter-react";
import {
  compilePolicy,
  describePolicy,
  FIELD_INFO,
  ModuleId,
  POLICY_LIMITS,
  type ActionSpec,
  type FieldName,
  type OpName,
  type PolicyArgs,
  type PolicySpec,
  type Trigger,
} from "@pump-hooks/sdk";
import { DitherStage } from "./DitherStage";
import { isPubkey, useNetwork } from "./Providers";
import { parseSol } from "@/lib/format";
import { sendBatches } from "@/lib/tx";

const TRIGGERS: Trigger[] = ["BeforeBuy", "BeforeSell", "BeforeSwap", "AfterBuy", "AfterSell", "AfterSwap"];
const OPS: OpName[] = [">", ">=", "<", "<=", "==", "!="];
const FIELDS = (Object.keys(FIELD_INFO) as FieldName[]).filter((f) => FIELD_INFO[f].available);
type ActKind = "DENY" | "AWARD_POINTS" | "RECORD" | "UPDATE_STATE" | "EMIT_SIGNAL" | "ALLOW";
const ACTIONS: { kind: ActKind; label: string; when: "before" | "after" | "any" }[] = [
  { kind: "DENY", label: "DENY the routed trade", when: "before" },
  { kind: "AWARD_POINTS", label: "AWARD_POINTS to the wallet", when: "after" },
  { kind: "RECORD", label: "RECORD a tagged event", when: "any" },
  { kind: "UPDATE_STATE", label: "UPDATE_STATE register", when: "any" },
  { kind: "EMIT_SIGNAL", label: "EMIT_SIGNAL", when: "any" },
  { kind: "ALLOW", label: "ALLOW (explicit)", when: "any" },
];

interface Cond {
  field: FieldName;
  op: OpName;
  value: string;
  register: string;
}
interface Act {
  kind: ActKind;
  value: string;
  register: string;
  stateOp: "SET" | "ADD" | "SUB";
}
interface Draft {
  when: Trigger;
  match: "ALL" | "ANY";
  conds: Cond[];
  acts: Act[];
  cooldown: string;
  denyReason: string;
  label: string;
}

const TEMPLATES: { name: string; draft: Draft }[] = [
  {
    name: "Early whale brake",
    draft: {
      when: "BeforeBuy",
      match: "ALL",
      conds: [
        { field: "TIME_SINCE_LAUNCH", op: "<", value: "900", register: "0" },
        { field: "TRADE_SIZE", op: ">", value: "2", register: "0" },
      ],
      acts: [{ kind: "DENY", value: "", register: "0", stateOp: "SET" }],
      cooldown: "0",
      denyReason: "1",
      label: "deny > 2 SOL routed buys in the first 15 minutes",
    },
  },
  {
    name: "Reward diamond hands",
    draft: {
      when: "AfterBuy",
      match: "ALL",
      conds: [{ field: "HOLDER_DURATION", op: ">=", value: "86400", register: "0" }],
      acts: [{ kind: "AWARD_POINTS", value: "100", register: "0", stateOp: "SET" }],
      cooldown: "3600",
      denyReason: "0",
      label: "+100 points for wallets holding ≥ 1 day that buy again",
    },
  },
  {
    name: "Sell-pressure brake",
    draft: {
      when: "BeforeSell",
      match: "ALL",
      conds: [
        { field: "BUY_SELL_RATIO", op: "<", value: "4000", register: "0" },
        { field: "TRADE_SIZE", op: ">", value: "5", register: "0" },
      ],
      acts: [{ kind: "DENY", value: "", register: "0", stateOp: "SET" }],
      cooldown: "0",
      denyReason: "2",
      label: "deny > 5 SOL routed sells while routed flow is sell-heavy",
    },
  },
];

const unitOf = (f: FieldName) => (FIELD_INFO[f].unit === "lamports" ? "SOL" : FIELD_INFO[f].unit);

function toSpec(d: Draft): PolicySpec {
  const num = (s: string, what: string) => {
    if (!/^\d+$/.test(s.trim())) throw new Error(`${what}: whole number required`);
    return BigInt(s.trim());
  };
  return {
    when: d.when,
    match: d.match,
    if: d.conds.map((c, i) => {
      const value = FIELD_INFO[c.field].unit === "lamports" ? parseSol(c.value) : /^\d+$/.test(c.value.trim()) ? BigInt(c.value.trim()) : undefined;
      if (value === undefined) throw new Error(`condition ${i + 1}: ${unitOf(c.field)} value required`);
      return { field: c.field, op: c.op, value, register: Number(c.register) };
    }),
    then: d.acts.map((a, i): ActionSpec => {
      switch (a.kind) {
        case "DENY":
        case "ALLOW":
        case "EMIT_SIGNAL":
          return { kind: a.kind };
        case "AWARD_POINTS":
          return { kind: "AWARD_POINTS", points: num(a.value, `action ${i + 1}`) };
        case "RECORD":
          return { kind: "RECORD", tag: num(a.value || "0", `action ${i + 1}`) };
        case "UPDATE_STATE":
          return { kind: "UPDATE_STATE", register: Number(a.register), op: a.stateOp, value: num(a.value || "0", `action ${i + 1}`) };
      }
    }),
    cooldownSecs: Number(num(d.cooldown || "0", "cooldown")),
    denyReason: Number(num(d.denyReason || "0", "deny reason")),
    label: d.label || undefined,
  };
}

export function Composer() {
  const params = useSearchParams();
  const { client, connection, writesEnabled, program } = useNetwork();
  const wallet = useWallet();
  const [d, setD] = useState<Draft>(TEMPLATES[0].draft);
  const [hasVG, setHasVG] = useState(false);
  const [marketIn, setMarketIn] = useState(params.get("market") ?? "");
  const [inst, setInst] = useState<{ status: "idle" | "loading" | "ok" | "error"; msg?: string; sig?: string }>({ status: "idle" });

  const compiled = useMemo((): { ok: true; args: PolicyArgs; spec: PolicySpec } | { ok: false; error: string } => {
    try {
      const spec = toSpec(d);
      return { ok: true, args: compilePolicy(spec, { hasVolatilityGuard: hasVG }), spec };
    } catch (e) {
      return { ok: false, error: (e as Error).message };
    }
  }, [d, hasVG]);

  const before = d.when.startsWith("Before");
  const set = (p: Partial<Draft>) => {
    setD({ ...d, ...p });
    setInst({ status: "idle" });
  };
  const setCond = (i: number, p: Partial<Cond>) => set({ conds: d.conds.map((c, j) => (j === i ? { ...c, ...p } : c)) });
  const setAct = (i: number, p: Partial<Act>) => set({ acts: d.acts.map((a, j) => (j === i ? { ...a, ...p } : a)) });

  const install = async () => {
    if (!compiled.ok || !wallet.publicKey || !isPubkey(marketIn)) return;
    setInst({ status: "loading", msg: "Loading market" });
    try {
      const market = await client.getMarket(new PublicKey(marketIn.trim()));
      if (!market.config.authority.equals(wallet.publicKey)) throw new Error("connected wallet is not this market's authority");
      const { ix, index } = client.createPolicy({ market, authority: wallet.publicKey, spec: compiled.spec });
      setInst({ status: "loading", msg: `Creating policy #${index}` });
      const [sent] = await sendBatches(connection, wallet, [[ix]]);
      if (!sent.result.ok) throw new Error(sent.result.error ? `${sent.result.error.name}: ${sent.result.error.msg}` : "transaction failed");
      setInst({ status: "ok", msg: `Policy #${index} is live on this market.`, sig: sent.signature });
    } catch (e) {
      setInst({ status: "error", msg: (e as Error).message });
    }
  };

  return (
    <main className="page">
      <div className="page-head">
        <DitherStage mode="banner" />
        <div className="crumb">
          <b>03</b> / Composer
        </div>
        <h1 className="h2">WHEN · IF · THEN</h1>
        <p className="lede">
          Compose market logic and compile it to the bounded Policy VM: at most {POLICY_LIMITS.maxConditions} conditions and{" "}
          {POLICY_LIMITS.maxActions} actions, no loops, no external calls. The compiler here is the same one the SDK and CLI use; the program
          re-validates everything on-chain.
        </p>
        <div className="btn-row" style={{ marginTop: 16, gap: 8 }}>
          {TEMPLATES.map((t) => (
            <button key={t.name} className="btn small outline" onClick={() => set(t.draft)}>
              {t.name}
            </button>
          ))}
        </div>
      </div>

      <div className="grid-main">
        <div className="col">
          <div className="composer-line">
            <span className="kw when">WHEN</span>
            <div>
              <select className="input" value={d.when} onChange={(e) => set({ when: e.target.value as Trigger })} style={{ maxWidth: 260 }}>
                {TRIGGERS.map((t) => (
                  <option key={t}>{t}</option>
                ))}
              </select>
              <p className="faint" style={{ fontSize: 12, margin: "6px 0 0" }}>
                {before ? "Runs before the Pump CPI — can deny the routed trade." : "Runs after the Pump CPI on executed amounts — can award and record."}
              </p>
            </div>
          </div>

          <div className="composer-line">
            <span className="kw">IF</span>
            <div>
              {d.conds.map((c, i) => (
                <div className="cond-row" key={i}>
                  <select className="input" value={c.field} onChange={(e) => setCond(i, { field: e.target.value as FieldName })}>
                    {FIELDS.map((f) => (
                      <option key={f} value={f}>
                        {f} — {FIELD_INFO[f].label}
                      </option>
                    ))}
                  </select>
                  <select className="input" value={c.op} onChange={(e) => setCond(i, { op: e.target.value as OpName })}>
                    {OPS.map((o) => (
                      <option key={o}>{o}</option>
                    ))}
                  </select>
                  <div className="input-unit">
                    <input className="input" value={c.value} onChange={(e) => setCond(i, { value: e.target.value })} />
                    <span>{c.field === "HOOK_STATE" ? `r${c.register}` : unitOf(c.field)}</span>
                  </div>
                  <button className="icon-btn" onClick={() => set({ conds: d.conds.filter((_, j) => j !== i) })} aria-label="Remove condition">
                    ×
                  </button>
                  {c.field === "HOOK_STATE" && (
                    <select className="input" style={{ gridColumn: "1 / 2" }} value={c.register} onChange={(e) => setCond(i, { register: e.target.value })}>
                      {[0, 1, 2, 3].map((r) => (
                        <option key={r} value={r}>
                          register {r}
                        </option>
                      ))}
                    </select>
                  )}
                </div>
              ))}
              <div style={{ display: "flex", gap: 18, alignItems: "center" }}>
                {d.conds.length < POLICY_LIMITS.maxConditions && (
                  <button className="add" onClick={() => set({ conds: [...d.conds, { field: "TRADE_SIZE", op: ">", value: "1", register: "0" }] })}>
                    + condition
                  </button>
                )}
                {d.conds.length > 1 && (
                  <label className="mono" style={{ fontSize: 11.5, display: "flex", gap: 6, alignItems: "center" }}>
                    match
                    <select className="input" style={{ height: 28, width: 80 }} value={d.match} onChange={(e) => set({ match: e.target.value as "ALL" | "ANY" })}>
                      <option>ALL</option>
                      <option>ANY</option>
                    </select>
                  </label>
                )}
                {d.conds.some((c) => c.field === "VOLATILITY") && (
                  <label className="mono" style={{ fontSize: 11.5, display: "flex", gap: 6, alignItems: "center" }}>
                    <input type="checkbox" checked={hasVG} onChange={(e) => setHasVG(e.target.checked)} /> market has Volatility Guard
                  </label>
                )}
              </div>
            </div>
          </div>

          <div className="composer-line">
            <span className="kw">THEN</span>
            <div>
              {d.acts.map((a, i) => (
                <div className="act-row" key={i}>
                  <select className="input" value={a.kind} onChange={(e) => setAct(i, { kind: e.target.value as ActKind })}>
                    {ACTIONS.map((x) => (
                      <option key={x.kind} value={x.kind} disabled={(x.when === "before" && !before) || (x.when === "after" && before)}>
                        {x.label}
                      </option>
                    ))}
                  </select>
                  {a.kind === "AWARD_POINTS" || a.kind === "RECORD" ? (
                    <div className="input-unit">
                      <input className="input" value={a.value} onChange={(e) => setAct(i, { value: e.target.value })} />
                      <span>{a.kind === "AWARD_POINTS" ? "pts" : "tag"}</span>
                    </div>
                  ) : a.kind === "UPDATE_STATE" ? (
                    <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 6 }}>
                      <select className="input" value={a.register} onChange={(e) => setAct(i, { register: e.target.value })}>
                        {[0, 1, 2, 3].map((r) => (
                          <option key={r} value={r}>
                            r{r}
                          </option>
                        ))}
                      </select>
                      <select className="input" value={a.stateOp} onChange={(e) => setAct(i, { stateOp: e.target.value as Act["stateOp"] })}>
                        <option>SET</option>
                        <option>ADD</option>
                        <option>SUB</option>
                      </select>
                      <input className="input" value={a.value} onChange={(e) => setAct(i, { value: e.target.value })} />
                    </div>
                  ) : a.kind === "DENY" ? (
                    <div className="input-unit">
                      <input className="input" value={d.denyReason} onChange={(e) => set({ denyReason: e.target.value })} />
                      <span>reason 9000+</span>
                    </div>
                  ) : (
                    <span />
                  )}
                  <button className="icon-btn" onClick={() => set({ acts: d.acts.filter((_, j) => j !== i) })} aria-label="Remove action">
                    ×
                  </button>
                </div>
              ))}
              {d.acts.length < POLICY_LIMITS.maxActions && (
                <button className="add" onClick={() => set({ acts: [...d.acts, { kind: before ? "RECORD" : "AWARD_POINTS", value: "10", register: "0", stateOp: "ADD" }] })}>
                  + action
                </button>
              )}
              <p className="faint" style={{ fontSize: 12, marginTop: 10 }}>
                Vault actions (QUEUE_TWAP, ALLOCATE_AUTHORIZED_TREASURY, strategy pause/resume) compile, but only emit intents: the strategy vault that
                would act on them is not deployed, so they are not offered here.
              </p>
            </div>
          </div>

          <div className="composer-line">
            <span className="kw">WITH</span>
            <div className="grid-2" style={{ gap: 12 }}>
              <div className="field" style={{ borderTop: 0, gridTemplateColumns: "1fr 120px" }}>
                <label>
                  cooldown
                  <small>Seconds between actions of this policy</small>
                </label>
                <input className="input" value={d.cooldown} onChange={(e) => set({ cooldown: e.target.value })} />
              </div>
              <div className="field" style={{ borderTop: 0, gridTemplateColumns: "1fr" }}>
                <label>
                  label
                  <small>Stored as a hash on-chain</small>
                </label>
                <input className="input" value={d.label} onChange={(e) => set({ label: e.target.value })} />
              </div>
            </div>
          </div>
        </div>

        <div className="col" style={{ position: "sticky", top: "var(--header)" }}>
          <section className="section">
            <div className="section-h">
              <span className="n">01</span>
              <h3>Compiled</h3>
              <span className={`aside ${compiled.ok ? "ok" : "deny"}`}>{compiled.ok ? "valid" : "invalid"}</span>
            </div>
            {compiled.ok ? (
              <>
                <div className="code">
                  <span className="hl">{describePolicy(compiled.args)}</span>
                </div>
                <details>
                  <summary>Wire format · create_policy args</summary>
                  <div className="code" style={{ fontSize: 11 }}>
                    {JSON.stringify(
                      { ...compiled.args, labelHash: Array.from(compiled.args.labelHash.slice(0, 8), (b) => b.toString(16).padStart(2, "0")).join("") + "…" },
                      (_, v) => (typeof v === "bigint" ? v.toString() : v),
                      2,
                    )}
                  </div>
                </details>
              </>
            ) : (
              <div className="notice deny">{compiled.error}</div>
            )}
          </section>
          <section className="section">
            <div className="section-h">
              <span className="n">02</span>
              <h3>Install</h3>
            </div>
            <p className="mute" style={{ fontSize: 13, marginTop: 0 }}>
              Policies run inside the Custom Policy module. Install it on your market from{" "}
              <Link className="link-u" href="/hook">
                Hook a token
              </Link>
              , then add this policy as the market authority.
            </p>
            <input className="input" placeholder="Market address" value={marketIn} onChange={(e) => setMarketIn(e.target.value)} spellCheck={false} />
            <button
              className="btn"
              style={{ width: "100%", marginTop: 10 }}
              disabled={!compiled.ok || !wallet.publicKey || !isPubkey(marketIn) || !writesEnabled || inst.status === "loading"}
              onClick={install}
            >
              {inst.status === "loading" ? (
                <>
                  <span className="spin" /> {inst.msg}
                </>
              ) : (
                "Add policy to market"
              )}
            </button>
            {program.state !== "live" && <p className="faint" style={{ fontSize: 12 }}>Pump Hooks isn&apos;t deployed on this cluster yet; compiling still works.</p>}
            {inst.status === "error" && <div className="notice deny" style={{ marginTop: 10 }}>{inst.msg}</div>}
            {inst.status === "ok" && (
              <div className="notice ok" style={{ marginTop: 10 }}>
                {inst.msg}{" "}
                {inst.sig && (
                  <Link className="link-u" href={`/tx/${inst.sig}`}>
                    View transaction
                  </Link>
                )}
              </div>
            )}
            <p className="faint" style={{ fontSize: 12 }}>Requires the Custom Policy module (ID {ModuleId.Policy}) and at most 4 policies per market.</p>
          </section>
        </div>
      </div>
    </main>
  );
}
