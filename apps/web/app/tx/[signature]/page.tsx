import type { Metadata } from "next";
import { TxView } from "@/components/TxView";

export const metadata: Metadata = { title: "Transaction" };

export default async function TxPage({ params }: { params: Promise<{ signature: string }> }) {
  const { signature } = await params;
  return <TxView signature={signature} />;
}
