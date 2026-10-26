"use client";

import { CONFIG_SCHEMAS, type ModuleId } from "@pump-hooks/sdk";
import { choicesFor, flagLabel, flagsFor, utcOffset, type FormValues, type ParsedConfig } from "@/lib/config-form";
import { duration } from "@/lib/format";

const UNIT_LABEL: Record<string, string> = {
  lamports: "SOL",
  seconds: "sec",
  bps: "bps",
  count: "#",
  xp: "XP",
  base: "tokens",
  timestamp: "unix",
  minutes: "min",
};

export function ConfigEditor({
  module,
  values,
  parsed,
  onChange,
  symbol,
}: {
  module: ModuleId;
  values: FormValues;
  parsed: ParsedConfig;
  onChange: (v: FormValues) => void;
  symbol?: string;
}) {
  const schema = CONFIG_SCHEMAS[module] ?? [];
  if (schema.length === 0) return <p className="mute" style={{ fontSize: 13, margin: "8px 0" }}>No configuration. Add rules in the Composer after activation.</p>;
  const set = (k: string, v: string) => onChange({ ...values, [k]: v });
  return (
    <div>
      {schema.map((f) => {
        const err = parsed.fieldErrors[f.name];
        const choices = choicesFor(module, f.name);
        if (choices) {
          return (
            <div className="field" key={f.name}>
              <label htmlFor={`${module}-${f.name}`}>
                {f.name}
                <small>{f.description}</small>
              </label>
              <select id={`${module}-${f.name}`} className="input" value={values[f.name] || "0"} onChange={(e) => set(f.name, e.target.value)}>
                {Object.entries(choices).map(([label, x]) => (
                  <option key={label} value={String(x)}>
                    {label}
                  </option>
                ))}
              </select>
            </div>
          );
        }
        if (f.unit === "flags") {
          const flags = flagsFor(module, f.name);
          const cur = Number(values[f.name] || 0);
          return (
            <div className="field" key={f.name}>
              <label>
                {f.name}
                <small>{f.description}</small>
              </label>
              <div style={{ display: "grid", gap: 4 }}>
                {Object.entries(flags).map(([name, bit]) => (
                  <label key={name} style={{ display: "flex", gap: 8, alignItems: "center", fontFamily: "var(--mono)", fontSize: 11.5 }}>
                    <input type="checkbox" checked={(cur & bit) !== 0} onChange={(e) => set(f.name, String(e.target.checked ? cur | bit : cur & ~bit))} />
                    {flagLabel(name)}
                  </label>
                ))}
              </div>
            </div>
          );
        }
        const v = values[f.name] ?? "";
        const hint =
          f.unit === "seconds" && /^\d+$/.test(v) && v !== "0"
            ? duration(Number(v))
            : f.unit === "bps" && /^\d+$/.test(v)
              ? `${(Number(v) / 100).toFixed(2)}%`
              : f.type === "i16" && f.unit === "minutes" && /^-?\d+$/.test(v)
                ? utcOffset(Number(v))
                : undefined;
        const clock = f.unit === "minutes" && f.type === "u16";
        return (
          <div className="field" key={f.name}>
            <label htmlFor={`${module}-${f.name}`}>
              {f.name}
              <small>
                {f.description}
                {hint ? <span className="mono"> · {hint}</span> : null}
              </small>
            </label>
            <div className="input-unit">
              <input
                id={`${module}-${f.name}`}
                className={`input${err ? " bad" : ""}`}
                value={v}
                spellCheck={false}
                placeholder={f.unit === "key" ? "engine public key" : clock ? "HH:MM" : "0"}
                onChange={(e) => set(f.name, e.target.value)}
                title={err}
                style={f.unit === "key" ? { fontSize: 11 } : undefined}
              />
              {f.unit !== "key" ? <span>{f.unit === "base" && symbol ? symbol : clock ? "local" : UNIT_LABEL[f.unit]}</span> : null}
            </div>
          </div>
        );
      })}
      {parsed.problems.length > 0 && (
        <ul className="problems">
          {parsed.problems.map((p) => (
            <li key={p}>✕ {p}</li>
          ))}
        </ul>
      )}
    </div>
  );
}
