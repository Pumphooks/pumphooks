import { ModuleId, moduleInfo, type ModuleInfo } from "@pump-hooks/sdk";
import launchShield from "../../../hooks/launch-shield/hook.json";
import volumeGuard from "../../../hooks/volume-guard/hook.json";
import volatilityGuard from "../../../hooks/volatility-guard/hook.json";
import holderXp from "../../../hooks/holder-xp/hook.json";
import treasuryRouter from "../../../hooks/treasury-router/hook.json";
import twap from "../../../hooks/twap/hook.json";
import conditionalOrders from "../../../hooks/conditional-orders/hook.json";
import sentient from "../../../hooks/sentient/hook.json";
import policy from "../../../hooks/policy/hook.json";

export const CATEGORIES = ["EXECUTION", "RISK", "ORDERS", "LIQUIDITY", "TREASURY", "REWARDS", "AUTONOMOUS", "ACCESS", "EXPERIMENTAL"] as const;
export type Category = (typeof CATEGORIES)[number];

export const CATEGORY_NOTES: Partial<Record<Category, string>> = {
  LIQUIDITY:
    "BeforeLiquidity / AfterLiquidity are reserved in the ABI (wire values 6 and 7). No V1 module ships for them: liquidity actions on PumpSwap do not pass through the router yet.",
};

export interface HookManifest {
  slug: string;
  moduleId: number;
  name: string;
  version: number;
  kind: string;
  developer: string;
  categories: Category[];
  summary: string;
  behavior: string[];
  callbacks: string[];
  defaultCallbacks: string[];
  capabilities: string[];
  priority: { min: number; max: number; default: number };
  runtime: boolean;
  available: boolean;
  compatibleMarkets: string[];
  source: string;
  audit: string;
  verification: string;
  limitations: string[];
}

export interface CatalogEntry extends HookManifest {
  info: ModuleInfo;
}

const manifests = [launchShield, volumeGuard, volatilityGuard, holderXp, sentient, twap, treasuryRouter, policy, conditionalOrders] as HookManifest[];

export const CATALOG: CatalogEntry[] = manifests.map((m) => ({ ...m, info: moduleInfo(m.moduleId as ModuleId) }));

export const catalogEntry = (slugOrId: string | number) => CATALOG.find((c) => c.slug === slugOrId || c.moduleId === slugOrId);

export const REPO_URL = process.env.NEXT_PUBLIC_REPO_URL ?? "";
export const sourceUrl = (path: string) => (REPO_URL ? `${REPO_URL.replace(/\/$/, "")}/${path}` : undefined);
