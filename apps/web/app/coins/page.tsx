import type { Metadata } from "next";
import { CoinsDirectory } from "@/components/CoinsDirectory";

export const metadata: Metadata = { title: "Coins" };

export default function CoinsPage() {
  return <CoinsDirectory />;
}
