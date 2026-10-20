import type { Metadata } from "next";
import { Suspense } from "react";
import { Composer } from "@/components/Composer";

export const metadata: Metadata = { title: "Composer" };

export default function BuildPage() {
  return (
    <Suspense>
      <Composer />
    </Suspense>
  );
}
