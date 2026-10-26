import type { Metadata } from "next";
import Link from "next/link";
import { Callback, POLICY_LIMITS, PUMP_HOOKS_PROGRAM_ID } from "@pump-hooks/sdk";
import { DitherStage } from "@/components/DitherStage";
import { CALLBACK_NOTES, CAPABILITY_INFO, CATALOG } from "@/lib/catalog";

export const metadata: Metadata = { title: "Docs" };

const TOC = [
  ["routing", "Routing boundary"],
  ["abi", "Hook ABI"],
  ["execution", "Execution order"],
  ["policy", "Policy VM"],
  ["sentient", "SENTIENT"],
  ["sdk", "SDK"],
  ["cli", "CLI"],
  ["security", "Security model"],
] as const;

const SDK = `import { Connection, PublicKey } from "@solana/web3.js";
import { PumpHooksClient, PUMP_ALT, ModuleId, CONFIG_PRESETS } from "@pump-hooks/sdk";

const client = new PumpHooksClient(new Connection("https://api.devnet.solana.com"), {
  lookupTables: [PUMP_ALT.devnet],
});

// 1. Detect an existing Pump token: curve or PumpSwap, Token / Token-2022, reserves.
const token = await client.detect(new PublicKey(MINT));

// 2. Preview create + install + a routed buy as one simulated transaction.
const preview = await client.previewStack({
  mint: token.mint,
  authority: wallet,
  hooks: [
    { module: ModuleId.VolumeGuard, config: CONFIG_PRESETS[ModuleId.VolumeGuard] },
    { module: ModuleId.HolderXp, config: CONFIG_PRESETS[ModuleId.HolderXp] },
  ],
  quoteIn: 100_000_000n, // 0.1 SOL
});
const result = await client.simulate(preview, wallet);
result.trace?.entries; // [{ callback, moduleId, allowed, reasonCode }, ...]

// 3. Route a buy through an existing hooked market.
const market = await client.getMarket(MARKET);
const tx = await client.buy({ market, user: wallet, quoteIn: 100_000_000n });

// 4. Decode any confirmed transaction.
const trace = await client.getExecutionTrace(signature);`;

const CLI = `pump-hooks inspect <mint> --url https://api.devnet.solana.com
pump-hooks market create <mint> --keypair ./authority.json
pump-hooks install <market> volume-guard --config maxBuyQuote=5,windowSecs=3600 --dry-run
pump-hooks policy create <market> --spec policy.json
pump-hooks simulate <market> --buy 0.1
pump-hooks tx <signature>`;

const DEV_CLI = `pumphooks init my-hook --env router          # Rust program hook (hook-sdk) with tests and hook.json
pumphooks build && pumphooks test
pumphooks simulate --program target/deploy/my_hook.so --env router --callback before_swap --amount 5
pumphooks simulate --stack <hex>                  # kernel validation, conflicts, compute estimate
pumphooks bench                                    # measured CU vs bench/baseline.json
pumphooks security-check                           # diagnostics + stateful fuzzing (not an audit)
pumphooks demo [token|router|pool|account]         # the four engine demos on localnet
pumphooks deploy --cluster devnet --program <file.so> --keypair <file>      # prints; runs with --yes
pumphooks register --cluster devnet --manifest hook.json --keypair <file>   # simulates; sends with --yes`;

const HOOK = `pub trait PumpHook {
    const MODULE_ID: u16;
    fn validate_config(config: &[u8; 48]) -> Result<(), HookError>;
    fn init_state(_config: &[u8; 48], _state: &mut [u8; 48], _now: i64) {}
    fn on_callback(env: &mut HookEnv<'_>) -> Result<HookDecision, HookError>;
}

// 8 bytes, deterministic
pub struct HookDecision { pub allow: u8, pub _pad: u8, pub reason_code: u16, pub flags: u32 }`;

export default function DocsPage() {
  return (
    <main className="page">
      <div className="page-head">
        <DitherStage mode="banner" />
        <div className="crumb">
          <b>05</b> / Docs
        </div>
        <h1 className="h2">How Pump Hooks works.</h1>
        <p className="lede">
          A router program that wraps Pump and PumpSwap trades with a deterministic, composable hook stack. Program{" "}
          <span className="mono" style={{ fontSize: 13 }}>
            {PUMP_HOOKS_PROGRAM_ID.toBase58()}
          </span>
          .
        </p>
      </div>

      <div className="docs">
        <nav className="docs-toc">
          {TOC.map(([id, label], i) => (
            <a key={id} href={`#${id}`}>
              <span className="faint">{String(i + 1).padStart(2, "0")}</span> {label}
            </a>
          ))}
        </nav>

        <article className="docs-body">
          <section id="routing">
            <h2>01 Routing boundary</h2>
            <p>
              Pump and PumpSwap do not call external programs before or after a swap. There is no Uniswap-v4-style callback inside the venue. Pump Hooks
              therefore runs hooks at <b>its own router boundary</b>: a trader calls <code>route_curve</code> or <code>route_amm</code> to buy or sell, the router runs
              before-callbacks, performs the trade by CPI into Pump (bonding curve) or PumpSwap (after migration), observes the executed amounts and runs
              after-callbacks — all in one atomic transaction.
            </p>
            <div className="notice warn">
              <b>Bypass is possible.</b> Anyone can trade the same token directly on Pump or PumpSwap. Those trades do not run hooks. Guards protect
              routed flow, and rewards accrue only to routed flow; neither can restrict the venue itself.
            </div>
            <p>
              A hooked market is a <code>MarketConfig</code> PDA at <code>[&quot;market&quot;, mint, namespace]</code> for an existing Pump token. Many
              namespaces can hook the same token; the creator&apos;s market is marked <i>creator-owned</i> when the namespace is the Pump creator.
            </p>
          </section>

          <section id="abi">
            <h2>02 Hook ABI</h2>
            <p>
              Callback wire values are frozen. Reserved values are never reused or reordered; V1 subscribes to the six trade callbacks only.
            </p>
            <table className="table">
              <thead>
                <tr>
                  <th>#</th>
                  <th>Callback</th>
                  <th>Semantics</th>
                </tr>
              </thead>
              <tbody>
                {Array.from({ length: 12 }, (_, i) => (
                  <tr key={i} className={i >= 6 ? "faint" : undefined}>
                    <td>{i}</td>
                    <td>{Callback[i]}</td>
                    <td style={{ fontFamily: "var(--display)", fontSize: 13 }}>{CALLBACK_NOTES[Callback[i]]}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            <p>Each slot holds 48 bytes of config and 48 bytes of state. Modules implement one interface and return an 8-byte decision:</p>
            <pre className="code">{HOOK}</pre>
            <p>Capabilities are granted at install and enforced by the executor, not trusted from the module:</p>
            <table className="table">
              <tbody>
                {Object.entries(CAPABILITY_INFO).map(([k, v]) => (
                  <tr key={k}>
                    <td style={{ width: 230 }}>{k}</td>
                    <td style={{ fontFamily: "var(--display)", fontSize: 13 }}>{v}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            <p>
              V1 ships {CATALOG.length} first-party modules (see <Link className="link-u" href="/hooks">Hooks</Link>). External programs as hooks
              (CPI hooks) are specified but <b>not enabled</b> in V1.
            </p>
          </section>

          <section id="execution">
            <h2>03 Execution order</h2>
            <ol>
              <li>Validate the market, venue accounts and kill switch. Paused markets and disabled modules fail closed.</li>
              <li>Run before-callbacks in ascending priority. The first DENY reverts the whole transaction with <code>HookDenied</code> (6018).</li>
              <li>CPI into Pump or PumpSwap with the trader&apos;s own signature and slippage limit.</li>
              <li>Measure executed amounts from balance deltas, not from inputs.</li>
              <li>Run after-callbacks and active policies; commit state only for modules holding the write capability.</li>
              <li>
                Emit <code>RoutedSwapEvent</code>, <code>HookExecutionEvent</code> and <code>PolicyExecutionEvent</code>s, decodable by the SDK,
                CLI, explorer and indexer.
              </li>
            </ol>
            <p>
              Limits: 6 hook slots, 4 runtime hooks, 4 policies per market. Measured overhead on top of a direct PumpSwap
              buy is about 21–32k CU for 0–4 hooks (see <code>docs/benchmarks.md</code>).
            </p>
          </section>

          <section id="policy">
            <h2>04 Policy VM</h2>
            <p>
              Policies are <code>WHEN trigger IF conditions THEN actions</code> programs evaluated by a bounded interpreter: at most{" "}
              {POLICY_LIMITS.maxConditions} conditions, {POLICY_LIMITS.maxActions} actions and {POLICY_LIMITS.maxRegisters} registers; no loops,
              no external calls, at most {POLICY_LIMITS.maxPointsPerAction.toString()} points per action. DENY is valid only on Before triggers and
              reports error <code>9000 + reason</code>. Vault actions only record an intent; no vault is deployed in V1. Build policies in the{" "}
              <Link className="link-u" href="/build">
                Composer
              </Link>
              .
            </p>
          </section>

          <section id="sentient">
            <h2>05 SENTIENT</h2>
            <p>
              The on-chain half of an autonomous risk engine. An off-chain engine signs a <code>StrategySignal</code> with Ed25519; the program checks
              the market-configured signer, nonce, freshness and TTL, then moves the market between NORMAL, CAUTIOUS, DEFENSIVE and CIRCUIT_BREAKER.
              Signals can only <b>restrict</b> routed trades: the engine never holds user funds, never signs for users and never trades. Every regime
              expires.
            </p>
          </section>

          <section id="sdk">
            <h2>06 SDK</h2>
            <p>
              <code>@pump-hooks/sdk</code> builds every instruction, decodes every account and event, and validates hook configs and policies with the
              same rules as the program.
            </p>
            <pre className="code">{SDK}</pre>
          </section>

          <section id="cli">
            <h2>07 CLI</h2>
            <p>The CLI never reads your Solana CLI config; pass <code>--url</code> and <code>--keypair</code> explicitly. Write commands accept <code>--dry-run</code>.</p>
            <pre className="code">{CLI}</pre>
            <p>
              <code>pumphooks</code> is the ABI v1 developer CLI from the same package. <code>deploy</code> and <code>register</code> refuse mainnet.
            </p>
            <pre className="code">{DEV_CLI}</pre>
          </section>

          <section id="security">
            <h2>08 Security model</h2>
            <ul>
              <li>No token delegate and no persistent custody: users sign each routed trade; the router moves only what that trade moves.</li>
              <li>Hooks cannot transfer user assets; they can deny, record, and update their own state and per-wallet ledgers.</li>
              <li>Fail-closed: any hook error, bad config or unexpected venue state reverts the transaction.</li>
              <li>Fees Pump Hooks does not control are never redirected. Treasury Router only describes how explicitly deposited funds would be split; the vault that would enforce it is not deployed.</li>
              <li>A global kill switch can pause routing or disable individual modules; it cannot move funds.</li>
              <li>
                Status: <b>not audited</b>. On mainnet only the <code>pump-hooks</code> program is launched; the deployment badge checks it live. Read <code>docs/threat-model-pump-router.md</code> before using real funds.
              </li>
            </ul>
          </section>
        </article>
      </div>
    </main>
  );
}
