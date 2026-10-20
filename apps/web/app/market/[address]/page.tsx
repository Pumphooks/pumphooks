import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { PublicKey } from "@solana/web3.js";
import { MarketView } from "@/components/MarketView";

export const metadata: Metadata = { title: "Market" };

export default async function MarketPage({ params }: { params: Promise<{ address: string }> }) {
  const { address } = await params;
  try {
    new PublicKey(address);
  } catch {
    notFound();
  }
  return <MarketView address={address} />;
}
