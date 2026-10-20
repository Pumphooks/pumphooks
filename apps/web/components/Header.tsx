"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useState } from "react";
import { Wordmark } from "./Logo";
import { useNetwork } from "./Providers";
import { WalletButton } from "./WalletButton";
import { CLUSTERS } from "@/lib/cluster";

const NAV = [
  { href: "/hook", label: "Hook a token" },
  { href: "/hooks", label: "Hooks" },
  { href: "/build", label: "Composer" },
  { href: "/simulate", label: "Simulate" },
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
