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
import maxWallet from "../../../hooks/max-wallet/hook.json";
import sellLimit from "../../../hooks/sell-limit/hook.json";
import tradingHours from "../../../hooks/trading-hours/hook.json";

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

const manifests = [launchShield, maxWallet, sellLimit, tradingHours, volumeGuard, volatilityGuard, holderXp, sentient, twap, treasuryRouter, policy, conditionalOrders] as HookManifest[];

export const CATALOG: CatalogEntry[] = manifests.map((m) => ({ ...m, info: moduleInfo(m.moduleId as ModuleId) }));

export const catalogEntry = (slugOrId: string | number) => CATALOG.find((c) => c.slug === slugOrId || c.moduleId === slugOrId);

export const CAPABILITY_INFO: Record<string, string> = {
  READ_MARKET: "Read market config, venue reserves and the trade context",
  VETO_ROUTED_SWAP: "Deny a routed trade in a before-callback (reverts the transaction)",
  WRITE_HOOK_STATE: "Persist its 48-byte slot state on the market",
  WRITE_USER_STATE: "Update the trading wallet's per-market state",
  AWARD_POINTS: "Credit XP / points in the wallet's per-market ledger",
  TREASURY_SIGNAL: "Emit treasury allocation intents for the strategy vault",
  STRATEGY_EXECUTION: "Bound or request vault strategy execution (TWAP, orders)",
  ORACLE_READ: "Read an external oracle account (reserved; not granted in V1)",
  EXTERNAL_CPI: "Call an external hook program (reserved; no permissionless CPI hooks in V1)",
  EMIT_SIGNAL: "Emit or consume signed strategy signals",
};

export const CALLBACK_NOTES: Record<string, string> = {
  BeforeBuy: "Routed buy, before the Pump CPI. Can deny.",
  AfterBuy: "Routed buy, after the Pump CPI. Sees executed amounts; can still fail the transaction.",
  BeforeSell: "Routed sell, before the Pump CPI. Can deny.",
  AfterSell: "Routed sell, after the Pump CPI.",
  BeforeSwap: "Either side, before the Pump CPI. Can deny.",
  AfterSwap: "Either side, after the Pump CPI.",
  BeforeLiquidity: "Reserved (wire value 6).",
  AfterLiquidity: "Reserved (wire value 7).",
  BeforeFeeAction: "Reserved (wire value 8).",
  AfterFeeAction: "Reserved (wire value 9).",
  OnGraduation: "Reserved (wire value 10).",
  OnStrategySignal: "Reserved (wire value 11).",
};

export const REPO_URL = process.env.NEXT_PUBLIC_REPO_URL ?? "";
export const sourceUrl = (path: string) => (REPO_URL ? `${REPO_URL.replace(/\/$/, "")}/${path}` : undefined);
