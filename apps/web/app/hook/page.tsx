import type { Metadata } from "next";
import { Suspense } from "react";
import { HookFlow } from "@/components/HookFlow";

export const metadata: Metadata = { title: "Hook a token" };

export default function HookPage() {
  return (
    <Suspense>
      <HookFlow />
    </Suspense>
  );
}
