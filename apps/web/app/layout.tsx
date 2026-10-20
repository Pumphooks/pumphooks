import type { Metadata, Viewport } from "next";
import { Inter_Tight, JetBrains_Mono } from "next/font/google";
import "./globals.css";
import { Providers } from "@/components/Providers";
import { Header } from "@/components/Header";

const display = Inter_Tight({ subsets: ["latin"], variable: "--font-display", weight: ["400", "500", "600"] });
const mono = JetBrains_Mono({ subsets: ["latin"], variable: "--font-mono", weight: ["400", "500", "600"] });

export const metadata: Metadata = {
  title: { default: "Pump Hooks — the programmable layer for Pump", template: "%s · Pump Hooks" },
  description:
    "Give any existing Pump market new behavior. Compose execution, risk, rewards, treasury and autonomous logic that runs on every trade routed through Pump Hooks.",
};

export const viewport: Viewport = { themeColor: "#efede6" };

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`${display.variable} ${mono.variable}`}>
      <body>
        <Providers>
          <Header />
          {children}
        </Providers>
      </body>
    </html>
  );
}
