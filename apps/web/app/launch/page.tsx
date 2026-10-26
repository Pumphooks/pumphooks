import type { Metadata } from "next";
import { LaunchFlow } from "@/components/LaunchFlow";

export const metadata: Metadata = { title: "Launch a coin" };

export default function LaunchPage() {
  return <LaunchFlow />;
}
