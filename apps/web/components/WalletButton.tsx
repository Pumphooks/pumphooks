"use client";

import { useEffect, useRef, useState } from "react";
import { WalletReadyState } from "@solana/wallet-adapter-base";
import { useWallet } from "@solana/wallet-adapter-react";
import { short } from "@/lib/format";

export function WalletButton() {
  const { wallets, wallet, publicKey, connected, connecting, select, connect, disconnect } = useWallet();
  const [open, setOpen] = useState(false);
  const [pending, setPending] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const close = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, []);

  useEffect(() => {
    if (pending && wallet?.adapter.name === pending && !connected && !connecting) {
      connect().catch(() => undefined).finally(() => setPending(null));
    }
  }, [pending, wallet, connected, connecting, connect]);

  const installed = wallets.filter((w) => w.readyState === WalletReadyState.Installed || w.readyState === WalletReadyState.Loadable);

  return (
    <div ref={ref} style={{ position: "relative" }}>
      <button className="btn small" onClick={() => setOpen((o) => !o)}>
        {connecting ? <span className="spin" /> : null}
        {connected && publicKey ? <span className="mono">{short(publicKey)}</span> : "Connect"}
      </button>
      {open && (
        <div className="wallet-menu">
          {connected && publicKey ? (
            <>
              <button onClick={() => navigator.clipboard.writeText(publicKey.toBase58())}>Copy address</button>
              <button
                onClick={() => {
                  disconnect();
                  setOpen(false);
                }}
              >
                Disconnect {wallet?.adapter.name}
              </button>
            </>
          ) : installed.length === 0 ? (
            <div className="empty">No Solana wallet detected. Install a Wallet Standard wallet (Phantom, Solflare, Backpack) and reload.</div>
          ) : (
            installed.map((w) => (
              <button
                key={w.adapter.name}
                onClick={() => {
                  select(w.adapter.name);
                  setPending(w.adapter.name);
                  setOpen(false);
                }}
              >
                {/* eslint-disable-next-line @next/next/no-img-element */}
                <img src={w.adapter.icon} alt="" />
                {w.adapter.name}
              </button>
            ))
          )}
        </div>
      )}
    </div>
  );
}
