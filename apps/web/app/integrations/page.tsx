import type { Metadata } from "next";
import { PageHead, Section } from "@/components/Provenance";
import { DeploymentStatus } from "@/components/DeploymentStatus";

export const metadata: Metadata = { title: "Integrations" };

// Each "verified" row names the test that exercises the real venue binary (tests/hooks/tests/venues.rs, router.rs).
const VENUES = [
  { name: "PumpSwap", id: "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA", status: "verified", how: "real binary, cloned devnet pool, buy and sell" },
  { name: "Raydium CPMM", id: "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C", status: "verified", how: "mainnet binary, pool made by its own initialize, both swap instructions" },
  { name: "Raydium CPMM (devnet)", id: "DRaycpLY18LhpbydsBWbVJtxpNv9oXPgjRSfpF2bWpYb", status: "verified", how: "devnet binary, same coverage" },
  { name: "Meteora DAMM v2", id: "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG", status: "verified", how: "devnet and mainnet binaries, cloned funded pools, swap2 and swap" },
  { name: "Raydium CLMM", id: "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK", status: "verified", how: "mainnet binary, cloned SOL/USDC pool with tick arrays, swap_v2 both ways" },
  { name: "Raydium CLMM (devnet)", id: "DRayAUgENGQBKVaX8owNhgzkEDyoHTGVEGHVJT1E9pfH", status: "not verified", how: "no devnet pool exercised yet" },
  { name: "Orca Whirlpools", id: "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc", status: "verified", how: "mainnet binary, cloned SOL/USDC whirlpool, swap_v2 both directions" },
  { name: "Meteora DLMM", id: "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo", status: "verified", how: "mainnet binary, cloned SOL/USDC pair with bin arrays, swap2 both ways" },
];

const ENGINES = [
  { engine: "TOKEN", program: "token-hooks + token-wrapper", enforcement: "Native on Token-2022 transfers of a hooked mint; wrapper for wrap / unwrap", bypass: "Not bypassable for the hooked mint. The original SPL token is unaffected." },
  { engine: "ROUTER", program: "hook-router", enforcement: "Router: only trades routed through hook-router", bypass: "Bypassable: the venue stays directly callable. No AMM above has native hooks." },
  { engine: "POOL", program: "hooks-core", enforcement: "Program: every swap and liquidity change of pools it manages", bypass: "Not bypassable for its own pools." },
  { engine: "ACCOUNT", program: "smart-accounts", enforcement: "Program: every withdrawal and execute of a smart account", bypass: "Not bypassable for funds held in the account." },
];

const PROGRAMS = [
  { name: "hook-registry", id: "9gKUqwnUa6ZJkCvNT4bnLY7A5czcJFuSV1dpYeosHHs4" },
  { name: "hook-router", id: "2a3wWdS8z9g7vBcYWSjdETyEzo7DEjV3iYRZFsM52q4U" },
  { name: "smart-accounts", id: "6AEiwvifk4TVJ8g3UoVXqhaK8AqYy43A4DzWCgMmrtQw" },
  { name: "token-hooks", id: "FZBBkQQamgfMCAoNk6PAdZi9nYZGMsGw9gDyTMNC3v54" },
  { name: "token-wrapper", id: "841vHKyD8HXQKsNSp2A2pzNjDHuFDBu9JPt6xsJc7vbh" },
  { name: "hooks-core", id: "8L8VFMpzP7S3ZfSyr9vu5vDVy93jgfWVfEzY4RTadFzw" },
  { name: "hook-state", id: "AZ3DwmvABEwrLJF25r94yyA92W4zvHgAwuhxBi8QJHRW" },
  { name: "pump-hooks (legacy)", id: "91kRNewiEDrKVtUXpahsDXYiURuKQKwhCt2hnaKNfkgD" },
];

export default function IntegrationsPage() {
  return (
    <main className="page">
      <PageHead n="10" crumb="Integrations" title="Where hooks actually run.">
        Which venues the router can call, how each engine enforces its hooks, and whether each program is deployed, checked live against public RPC.
      </PageHead>

      <Section n="01" title="Engines and enforcement">
        <table className="table">
          <thead>
            <tr>
              <th>Engine</th>
              <th>Program</th>
              <th>Enforced on</th>
              <th>Bypass</th>
            </tr>
          </thead>
          <tbody>
            {ENGINES.map((e) => (
              <tr key={e.engine}>
                <td>{e.engine}</td>
                <td>{e.program}</td>
                <td style={{ fontFamily: "var(--display)", fontSize: 13 }}>{e.enforcement}</td>
                <td style={{ fontFamily: "var(--display)", fontSize: 13 }}>{e.bypass}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </Section>

      <Section n="02" title="Router venues" aside="every venue starts disabled; the router admin enables each">
        <table className="table">
          <thead>
            <tr>
              <th>Venue</th>
              <th>Program</th>
              <th>Status</th>
              <th>How</th>
            </tr>
          </thead>
          <tbody>
            {VENUES.map((v) => (
              <tr key={v.id}>
                <td>{v.name}</td>
                <td title={v.id}>{v.id}</td>
                <td>
                  <span className={`tag ${v.status === "verified" ? "ok" : "warn"}`}>{v.status}</span>
                </td>
                <td style={{ fontFamily: "var(--display)", fontSize: 13 }}>{v.how}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <p className="mute" style={{ fontSize: 13 }}>
          Verified means the router traded through the venue&apos;s real program in LiteSVM, measuring the trader&apos;s actual balance changes
          (not the venue&apos;s report), enforcing its own max-in / min-out on them, and requiring the pool account to be owned by the venue program.
        </p>
      </Section>

      <Section n="03" title="Deployments" aside="live">
        <DeploymentStatus programs={PROGRAMS} />
        <p className="mute" style={{ fontSize: 13 }}>
          Checked from your browser with <code>getMultipleAccountsInfo</code> against the site&apos;s configured RPC. The public mainnet endpoint refuses
          browser requests; a deployment sets <code>NEXT_PUBLIC_MAINNET_RPC_URL</code> to a browser-capable endpoint.
        </p>
        <div className="notice warn" style={{ marginTop: 14 }}>
          <b>Pre-release.</b> The ABI v1 programs are built and tested on localnet. They are not on mainnet and will not be without an external
          audit. Their devnet deployment is pending.
        </div>
      </Section>
    </main>
  );
}
