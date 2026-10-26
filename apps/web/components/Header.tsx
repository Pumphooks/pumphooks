"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useState } from "react";
import { Wordmark } from "./Logo";
import { useNetwork } from "./Providers";
import { WalletButton } from "./WalletButton";
import { CLUSTERS } from "@/lib/cluster";
import { REPO } from "@/lib/repo";

const NAV = [
  { href: "/launch", label: "Launch" },
  { href: "/coins", label: "Coins" },
  { href: "/hook", label: "Hook a token" },
  { href: "/hooks", label: "Hooks" },
  { href: "/build", label: "Composer" },
  { href: "/simulate", label: "Simulate" },
  { href: "/demos", label: "Demos" },
  { href: "/lab", label: "Lab" },
  { href: "/docs", label: "Docs" },
];

export function Header() {
  const path = usePathname();
  const [scrolled, setScrolled] = useState(false);
  useEffect(() => {
    const f = () => setScrolled(window.scrollY > 8);
    f();
    window.addEventListener("scroll", f, { passive: true });
    return () => window.removeEventListener("scroll", f);
  }, []);
  return (
    <header className={`header${scrolled ? " scrolled" : ""}`}>
      <Link href="/" aria-label="Pump Hooks home">
        <Wordmark />
      </Link>
      <nav className="nav">
        {NAV.map((n) => (
          <Link key={n.href} href={n.href} aria-current={path === n.href || path.startsWith(n.href + "/") ? "page" : undefined}>
            {n.label}
          </Link>
        ))}
      </nav>
      <div className="header-tools">
        <a className="cluster gh" href={REPO} target="_blank" rel="noopener noreferrer" aria-label="Source on GitHub" title="Source on GitHub">
          <svg width="15" height="15" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
            <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
          </svg>
        </a>
        <ClusterSwitch />
        <WalletButton />
      </div>
    </header>
  );
}

function ClusterSwitch() {
  const { cluster, setCluster, program } = useNetwork();
  const next = cluster === "devnet" ? "mainnet-beta" : "devnet";
  const dot =
    program.state === "live" ? "live" : program.state === "loading" ? "wait pulse" : program.state === "uninitialized" ? "wait" : "off";
  const title =
    program.state === "live"
      ? "Pump Hooks program is live on this cluster"
      : program.state === "not-deployed"
        ? "Pump Hooks is not deployed on this cluster — detection and quotes only"
        : program.state === "uninitialized"
          ? "Program deployed but not initialized"
          : program.state === "error"
            ? `RPC error: ${program.error}`
            : "Checking program…";
  return (
    <button className="cluster" onClick={() => setCluster(next)} title={`${title}. Click to switch to ${CLUSTERS[next].label}.`}>
      <span className={`dot ${dot}`} />
      {CLUSTERS[cluster].label}
    </button>
  );
}
