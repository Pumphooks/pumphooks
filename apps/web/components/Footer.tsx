import Link from "next/link";
import { Wordmark } from "./Logo";

export function Footer() {
  return (
    <footer className="footer">
      <div>
        <Wordmark />
        <p style={{ color: "#a29f95", maxWidth: 380, marginTop: 14 }}>
          A programmable execution layer for existing Pump markets. Hooks run on trades routed through the Pump Hooks
          router; direct Pump trades are unaffected.
        </p>
      </div>
      <div>
        <h4>Product</h4>
        <Link href="/hook">Hook a token</Link>
        <Link href="/hooks">Hook registry</Link>
        <Link href="/build">Composer</Link>
        <Link href="/simulate">Simulator</Link>
      </div>
      <div>
        <h4>Developers</h4>
        <Link href="/docs">Docs</Link>
        <Link href="/docs#abi">Hook ABI</Link>
        <Link href="/docs#sdk">SDK &amp; CLI</Link>
        <Link href="/docs#security">Security model</Link>
        <Link href="/demos">Demos</Link>
        <Link href="/playground">Playground</Link>
        <Link href="/lab">Execution lab</Link>
        <Link href="/benchmarks">Benchmarks</Link>
        <Link href="/security-lab">Security lab</Link>
        <Link href="/compiler">Compiler</Link>
        <Link href="/integrations">Integrations</Link>
      </div>
      <div>
        <h4>Status</h4>
        <span style={{ display: "block", color: "#d8d5cb", margin: "5px 0" }}>Devnet: pre-release</span>
        <span style={{ display: "block", color: "#d8d5cb", margin: "5px 0" }}>Mainnet: not deployed</span>
        <span style={{ display: "block", color: "#d8d5cb", margin: "5px 0" }}>Audit: none yet</span>
      </div>
      <div className="fine">
        <span>Pump Hooks is independent software and is not affiliated with Pump.fun. Unaudited. Use at your own risk.</span>
        <span className="mono">ABI v1</span>
      </div>
    </footer>
  );
}
