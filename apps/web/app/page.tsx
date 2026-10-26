import Link from "next/link";
import { DitherStage } from "@/components/DitherStage";
import { Footer } from "@/components/Footer";
import { CaBox, LiveStatus } from "@/components/HomeClient";

const STACK = [
  ["Launch Shield", "risk"],
  ["Anti-dump", "max wallet · sell limit · hours"],
  ["Dynamic Execution", "volume · volatility"],
  ["Holder XP", "rewards"],
  ["Treasury", "allocation"],
  ["TWAP", "execution"],
  ["SENTIENT", "autonomous"],
  ["Custom Logic", "policy vm"],
];

export default function Home() {
  return (
    <>
      <DitherStage />
      <main className="home">
        <section className="hero" data-frame="hero">
          <h1 className="display">The programmable layer for Pump.</h1>
          <p className="lede">
            Give any Pump market new behavior. Compose execution, liquidity, treasury, trading, risk and autonomous logic.
          </p>
          <div className="hero-cta">
            <Link className="btn" href="/hook">
              Hook a token <span className="chev">›</span>
            </Link>
            <Link className="btn outline" href="/hooks">
              Explore hooks
            </Link>
            <Link className="btn outline" href="/build">
              Build a hook
            </Link>
          </div>
          <CaBox />
          <div className="scroll-hint" data-stage-top>
            Any existing Pump token · bonding curve or PumpSwap ↓
          </div>
        </section>

        <Chapter frame="s1" n="01" eyebrow="For token communities" title="Hook any Pump token">
          <p className="lede">
            Paste a contract address. Pump Hooks reads what already exists — bonding curve or PumpSwap pool, quote mint,
            Token or Token-2022, creator, reserves, any hook configs — and lets you program the market on top of it. No new
            token. No migration. The market keeps trading on Pump.
          </p>
          <div className="btn-row">
            <Link className="btn" href="/hook">
              Hook a token <span className="chev">›</span>
            </Link>
            <Link className="link-u" href="/docs#routing">How routing works</Link>
          </div>
        </Chapter>

        <Chapter frame="s2" n="02" eyebrow="The hook stack" title="Compose market behavior">
          <p className="lede">
            Every routed trade runs the market&apos;s stack in priority order: before hooks, the Pump CPI, after hooks,
            settlement. Atomic and fail-closed — one denial reverts the whole transaction.
          </p>
          <div className="stack-diagram" aria-label="Any Pump token, through the hook stack, to Pump or PumpSwap">
            <div className="row head">
              <span>ANY PUMP TOKEN</span>
              <span>CA</span>
            </div>
            <div className="arrow">↓</div>
            <div className="row" style={{ fontWeight: 600 }}>
              <span>HOOK STACK</span>
              <span className="mute">before · after</span>
            </div>
            {STACK.map(([n, k]) => (
              <div className="row hook" key={n}>
                <span>{n}</span>
                <span className="mute">{k}</span>
              </div>
            ))}
            <div className="arrow">↓</div>
            <div className="row head">
              <span>PUMP / PUMPSWAP</span>
              <span>CPI</span>
            </div>
          </div>
          <div className="btn-row" style={{ marginTop: 26 }}>
            <Link className="btn" href="/hooks">
              Explore hooks <span className="chev">›</span>
            </Link>
            <Link className="link-u" href="/simulate">Simulate a trade</Link>
          </div>
        </Chapter>

        <Chapter frame="s3" n="03" eyebrow="For builders" title="Build a hook">
          <p className="lede">
            Write WHEN / IF / THEN in the Composer and compile it to the bounded Policy VM — no loops, no arbitrary CPI,
            fixed cost. Or build a module against the frozen v1 ABI: six trade callbacks, capability-scoped state,
            priority namespaces, a typed SDK and a local simulator against Pump&apos;s real programs.
          </p>
          <div className="btn-row">
            <Link className="btn" href="/build">
              Open the Composer <span className="chev">›</span>
            </Link>
            <Link className="link-u" href="/docs#abi">Hook ABI</Link>
            <Link className="link-u" href="/docs#sdk">SDK</Link>
          </div>
        </Chapter>

        <Chapter frame="s4" n="04" eyebrow="Autonomous" title="SENTIENT, bounded">
          <p className="lede">
            An off-chain engine reads routed flow and proposes a market regime. The program verifies its signature, nonce,
            freshness and the market&apos;s own TTL ceiling. SENTIENT can only tighten routed execution — it never signs
            for users, never trades and never moves funds.
          </p>
          <div className="btn-row">
            <Link className="btn" href="/hooks/sentient">
              SENTIENT <span className="chev">›</span>
            </Link>
            <Link className="link-u" href="/docs#security">Security model</Link>
          </div>
        </Chapter>

        <section className="slab" data-frame="end">
          <div className="slab-inner">
            <div>
              <div className="eyebrow">
                <span className="n">05</span>
                <span>The boundary</span>
              </div>
              <h2 className="h2">What hooks can and can&apos;t do.</h2>
              <p className="lede">Stated plainly, everywhere: in the program, the SDK, the CLI and here.</p>
            </div>
            <div className="boundary-list">
              <div>
                <b>Router boundary</b>
                <p>
                  PumpSwap and the bonding curve don&apos;t call external hooks. Pump Hooks runs before and after its own
                  CPI into Pump. Trades sent to Pump directly bypass the stack.
                </p>
              </div>
              <div>
                <b>No custody</b>
                <p>
                  Tokens move between the wallet and Pump inside one transaction the wallet signs. No delegate, no vault
                  for user funds, no redirection of fees Pump Hooks doesn&apos;t control.
                </p>
              </div>
              <div>
                <b>Fail-closed</b>
                <p>
                  A denial, a hook error or an after-check that Pump executed more than allowed reverts everything — the
                  Pump trade included.
                </p>
              </div>
            </div>
          </div>
          <div style={{ padding: "0 var(--gutter) 80px" }}>
            <LiveStatus />
          </div>
        </section>
      </main>
      <Footer />
    </>
  );
}

function Chapter({ frame, n, eyebrow, title, children }: { frame: string; n: string; eyebrow: string; title: string; children: React.ReactNode }) {
  return (
    <section className="chapter" data-frame={frame}>
      <div className="chapter-copy">
        <div className="eyebrow">
          <span className="n">{n}</span>
          <span>{eyebrow}</span>
        </div>
        <h2 className="h2">{title}</h2>
        {children}
      </div>
    </section>
  );
}
