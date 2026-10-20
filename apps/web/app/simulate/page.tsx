import type { Metadata } from "next";
import { SimulateLab } from "@/components/SimulateLab";

export const metadata: Metadata = { title: "Simulate" };

export default function SimulatePage() {
  return <SimulateLab />;
}
