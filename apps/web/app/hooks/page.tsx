import type { Metadata } from "next";
import { HooksMarket } from "@/components/HooksMarket";

export const metadata: Metadata = { title: "Hooks" };

export default function HooksPage() {
  return <HooksMarket />;
}
