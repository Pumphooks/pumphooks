import type { Provenance as P } from "@/lib/artifacts";

const REPO = "https://github.com/Pumphooks/pumphooks";

export function Provenance({ p, title = "Provenance" }: { p: P; title?: string }) {
  return (
    <details>
      <summary>{title}: commit {p.commit.slice(0, 7)}, generated {p.generatedAt.slice(0, 10)}</summary>
      <div className="kv">
        <div>
          <div className="k">Commit</div>
          <div className="v">
            <a href={`${REPO}/commit/${p.commit}`} target="_blank" rel="noreferrer">
              {p.commit.slice(0, 12)}
            </a>
          </div>
        </div>
        <div>
          <div className="k">Generated</div>
          <div className="v">{p.generatedAt}</div>
        </div>
        <div style={{ gridColumn: "span 2" }}>
          <div className="k">Runtime</div>
          <div className="v" title={p.runtime}>{p.runtime}</div>
        </div>
        {Object.entries(p.programs).map(([name, hash]) => (
          <div key={name} style={{ gridColumn: "span 2" }}>
            <div className="k">{name} sha256</div>
            <div className="v" title={hash}>{hash}</div>
          </div>
        ))}
      </div>
      <pre className="code">{p.command}</pre>
    </details>
  );
}

export function PageHead({ n, crumb, title, children }: { n: string; crumb: string; title: string; children: React.ReactNode }) {
  return (
    <div className="page-head" style={{ minHeight: 200 }}>
      <div className="crumb">
        <b>{n}</b> / {crumb}
      </div>
      <h1 className="h2">{title}</h1>
      <div className="lede">{children}</div>
    </div>
  );
}

export function Section({ n, title, aside, children }: { n: string; title: string; aside?: React.ReactNode; children: React.ReactNode }) {
  return (
    <section className="section">
      <div className="section-h">
        <span className="n">{n}</span>
        <h3>{title}</h3>
        {aside ? <span className="aside">{aside}</span> : null}
      </div>
      {children}
    </section>
  );
}
