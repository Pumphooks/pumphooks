import type { NextConfig } from "next";
import { join } from "node:path";

const root = join(__dirname, "../..");

const config: NextConfig = {
  reactStrictMode: true,
  turbopack: { root },
  outputFileTracingRoot: root,
  poweredByHeader: false,
  agentRules: false,
};

export default config;
