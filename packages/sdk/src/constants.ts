import { PublicKey } from "@solana/web3.js";

/** Pump Hooks program (devnet deployment key; mainnet is not deployed). */
export const PUMP_HOOKS_PROGRAM_ID = new PublicKey("91kRNewiEDrKVtUXpahsDXYiURuKQKwhCt2hnaKNfkgD");

export const PUMP_PROGRAM_ID = new PublicKey("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");
export const PUMP_AMM_PROGRAM_ID = new PublicKey("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA");
export const PUMP_FEES_PROGRAM_ID = new PublicKey("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");
export const MAYHEM_PROGRAM_ID = new PublicKey("MAyhSmzXzV1pTf7LsNkrNwkWKTo4ougAJ1PPg47MD4e");

export const TOKEN_PROGRAM_ID = new PublicKey("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
export const TOKEN_2022_PROGRAM_ID = new PublicKey("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
export const ATA_PROGRAM_ID = new PublicKey("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
export const NATIVE_MINT = new PublicKey("So11111111111111111111111111111111111111112");
export const SYSTEM_PROGRAM_ID = new PublicKey("11111111111111111111111111111111");

/** Address lookup tables Pump publishes for its static accounts. */
export const PUMP_ALT = {
  mainnet: new PublicKey("Hyif6eWb8x88RVrvjPfabsgRYnwkVnyByEXTVTXbUcyP"),
  devnet: new PublicKey("7Je3GuLHSXZT6oC96EuAKTH8LsMhvC32z72cqVxUnvR7"),
} as const;

export const ABI_VERSION = 1;
export const MAX_HOOK_SLOTS = 6;
export const MAX_RUNTIME_HOOKS = 6;
export const MAX_POLICIES = 4;
export const SLOT_CONFIG_LEN = 48;

/** Hook callbacks. Wire values are frozen; 6..11 are reserved for future releases. */
export enum Callback {
  BeforeBuy = 0,
  AfterBuy = 1,
  BeforeSell = 2,
  AfterSell = 3,
  BeforeSwap = 4,
  AfterSwap = 5,
  BeforeLiquidity = 6,
  AfterLiquidity = 7,
  BeforeFeeAction = 8,
  AfterFeeAction = 9,
  OnGraduation = 10,
  OnStrategySignal = 11,
}
export const V1_CALLBACK_MASK = 0b11_1111;
export const callbackMask = (...cbs: Callback[]) => cbs.reduce((m, c) => m | (1 << c), 0);
export const callbacksOf = (mask: number): Callback[] =>
  Array.from({ length: 12 }, (_, i) => i).filter((i) => mask & (1 << i)) as Callback[];

export enum Venue {
  BondingCurve = 0,
  PumpSwap = 1,
}

export enum Side {
  Buy = 0,
  Sell = 1,
}

/** How a routed trade is specified; maps 1:1 onto Pump instructions. */
export enum TradeMode {
  /** Buy exactly `amount` base for at most `limit` quote. */
  ExactBaseOut = 0,
  /** Spend exactly `amount` quote for at least `limit` base. */
  ExactQuoteIn = 1,
  /** Sell exactly `amount` base for at least `limit` quote. */
  ExactBaseIn = 2,
}

export const Capability = {
  READ_MARKET: 1 << 0,
  VETO_ROUTED_SWAP: 1 << 1,
  WRITE_HOOK_STATE: 1 << 2,
  WRITE_USER_STATE: 1 << 3,
  AWARD_POINTS: 1 << 4,
  TREASURY_SIGNAL: 1 << 5,
  STRATEGY_EXECUTION: 1 << 6,
  ORACLE_READ: 1 << 7,
  EXTERNAL_CPI: 1 << 8,
  EMIT_SIGNAL: 1 << 9,
} as const;
export type CapabilityName = keyof typeof Capability;
export const capabilitiesOf = (mask: number): CapabilityName[] =>
  (Object.keys(Capability) as CapabilityName[]).filter((k) => mask & Capability[k]);

export enum ModuleId {
  LaunchShield = 1,
  VolumeGuard = 2,
  VolatilityGuard = 3,
  HolderXp = 4,
  TreasuryRouter = 5,
  Twap = 6,
  ConditionalOrders = 7,
  Sentient = 8,
  Policy = 9,
  MaxWallet = 10,
  SellLimit = 11,
  TradingHours = 12,
}

export interface ModuleInfo {
  id: ModuleId;
  slug: string;
  name: string;
  version: number;
  /** Callbacks the module can subscribe to. */
  callbacks: number;
  defaultCallbacks: number;
  capabilities: number;
  priorityMin: number;
  priorityMax: number;
  defaultPriority: number;
  /** Runtime modules consume one of MAX_RUNTIME_HOOKS. */
  runtime: boolean;
  /** Implemented and installable in this release. */
  available: boolean;
}

const C = Capability;
const TRADE_BEFORE = callbackMask(Callback.BeforeBuy, Callback.BeforeSell, Callback.BeforeSwap);

/** Mirror of `hook_types::MODULES`; checked against registered HookDefinitions in tests. */
export const MODULES: readonly ModuleInfo[] = [
  {
    id: ModuleId.LaunchShield, slug: "launch-shield", name: "Launch Shield", version: 1,
    callbacks: TRADE_BEFORE, defaultCallbacks: callbackMask(Callback.BeforeBuy, Callback.BeforeSell),
    capabilities: C.READ_MARKET | C.VETO_ROUTED_SWAP | C.WRITE_HOOK_STATE,
    priorityMin: 0, priorityMax: 999, defaultPriority: 100, runtime: true, available: true,
  },
  {
    id: ModuleId.VolumeGuard, slug: "volume-guard", name: "Volume Guard", version: 1,
    callbacks: V1_CALLBACK_MASK, defaultCallbacks: callbackMask(Callback.BeforeSwap, Callback.AfterSwap),
    capabilities: C.READ_MARKET | C.VETO_ROUTED_SWAP | C.WRITE_HOOK_STATE | C.WRITE_USER_STATE,
    priorityMin: 1000, priorityMax: 1999, defaultPriority: 1100, runtime: true, available: true,
  },
  {
    id: ModuleId.VolatilityGuard, slug: "volatility-guard", name: "Volatility Guard", version: 1,
    callbacks: V1_CALLBACK_MASK, defaultCallbacks: callbackMask(Callback.BeforeSwap, Callback.AfterSwap),
    capabilities: C.READ_MARKET | C.VETO_ROUTED_SWAP | C.WRITE_HOOK_STATE,
    priorityMin: 1000, priorityMax: 1999, defaultPriority: 1200, runtime: true, available: true,
  },
  {
    id: ModuleId.HolderXp, slug: "holder-xp", name: "Holder XP", version: 1,
    callbacks: callbackMask(Callback.AfterBuy, Callback.AfterSell),
    defaultCallbacks: callbackMask(Callback.AfterBuy, Callback.AfterSell),
    capabilities: C.READ_MARKET | C.WRITE_HOOK_STATE | C.WRITE_USER_STATE | C.AWARD_POINTS,
    priorityMin: 2000, priorityMax: 2999, defaultPriority: 2100, runtime: true, available: true,
  },
  {
    id: ModuleId.TreasuryRouter, slug: "treasury-router", name: "Treasury Router", version: 1,
    callbacks: 0, defaultCallbacks: 0, capabilities: C.READ_MARKET | C.TREASURY_SIGNAL,
    priorityMin: 2000, priorityMax: 2999, defaultPriority: 2500, runtime: false, available: true,
  },
  {
    id: ModuleId.Twap, slug: "twap", name: "TWAP", version: 1,
    callbacks: 0, defaultCallbacks: 0, capabilities: C.READ_MARKET | C.STRATEGY_EXECUTION,
    priorityMin: 3000, priorityMax: 3999, defaultPriority: 3100, runtime: false, available: true,
  },
  {
    id: ModuleId.ConditionalOrders, slug: "conditional-orders", name: "Conditional Orders", version: 1,
    callbacks: 0, defaultCallbacks: 0, capabilities: C.READ_MARKET | C.STRATEGY_EXECUTION,
    priorityMin: 3000, priorityMax: 3999, defaultPriority: 3200, runtime: false, available: false,
  },
  {
    id: ModuleId.Sentient, slug: "sentient", name: "SENTIENT", version: 1,
    callbacks: V1_CALLBACK_MASK, defaultCallbacks: callbackMask(Callback.BeforeSwap, Callback.AfterSwap),
    capabilities: C.READ_MARKET | C.VETO_ROUTED_SWAP | C.WRITE_HOOK_STATE | C.STRATEGY_EXECUTION | C.EMIT_SIGNAL,
    priorityMin: 3000, priorityMax: 3999, defaultPriority: 3500, runtime: true, available: true,
  },
  {
    id: ModuleId.Policy, slug: "policy", name: "Custom Policy", version: 1,
    callbacks: V1_CALLBACK_MASK, defaultCallbacks: 0,
    capabilities:
      C.READ_MARKET | C.VETO_ROUTED_SWAP | C.WRITE_HOOK_STATE | C.WRITE_USER_STATE |
      C.AWARD_POINTS | C.TREASURY_SIGNAL | C.EMIT_SIGNAL,
    priorityMin: 0, priorityMax: 4999, defaultPriority: 4000, runtime: true, available: true,
  },
  {
    id: ModuleId.MaxWallet, slug: "max-wallet", name: "Max Wallet", version: 1,
    callbacks: callbackMask(Callback.BeforeBuy, Callback.AfterBuy),
    defaultCallbacks: callbackMask(Callback.BeforeBuy, Callback.AfterBuy),
    capabilities: C.READ_MARKET | C.VETO_ROUTED_SWAP,
    priorityMin: 0, priorityMax: 999, defaultPriority: 200, runtime: true, available: true,
  },
  {
    id: ModuleId.SellLimit, slug: "sell-limit", name: "Sell Limit", version: 1,
    callbacks: callbackMask(Callback.BeforeSell), defaultCallbacks: callbackMask(Callback.BeforeSell),
    capabilities: C.READ_MARKET | C.VETO_ROUTED_SWAP,
    priorityMin: 1000, priorityMax: 1999, defaultPriority: 1300, runtime: true, available: true,
  },
  {
    id: ModuleId.TradingHours, slug: "trading-hours", name: "Trading Hours", version: 1,
    callbacks: TRADE_BEFORE, defaultCallbacks: callbackMask(Callback.BeforeSwap),
    capabilities: C.READ_MARKET | C.VETO_ROUTED_SWAP,
    priorityMin: 0, priorityMax: 999, defaultPriority: 50, runtime: true, available: true,
  },
];

export function moduleInfo(id: number | string): ModuleInfo {
  const m = MODULES.find((x) => x.id === id || x.slug === id);
  if (!m) throw new Error(`unknown module ${id}`);
  return m;
}

/** Reason codes: module_id * 1000 + n; 9000+ Composer policies; 40000+ external hooks. */
export const REASONS: Record<number, string> = {
  0: "ok",
  1001: "Launch Shield: routed buy above the launch-phase limit",
  1002: "Launch Shield: wallet cooldown active",
  1003: "Launch Shield: routed buys per slot exhausted",
  1004: "Launch Shield: buy and sell by one wallet in one slot",
  1005: "Launch Shield: routed sell above the launch-phase limit",
  2001: "Volume Guard: routed buy above max trade size",
  2002: "Volume Guard: routed sell above max trade size",
  2003: "Volume Guard: rolling market routed volume limit",
  2004: "Volume Guard: rolling wallet routed volume limit",
  3001: "Volatility Guard: price deviates from routed EMA (buy)",
  3002: "Volatility Guard: price deviates from routed EMA (sell)",
  8001: "SENTIENT: defensive regime size cap",
  8002: "SENTIENT: circuit breaker regime",
  10001: "Max Wallet: buy would take the wallet above the max balance",
  11001: "Sell Limit: sell is above the allowed share of the wallet's balance",
  11002: "Sell Limit: wallet sold too recently",
  12001: "Trading Hours: outside trading hours",
};

export function describeReason(code: number): string {
  if (REASONS[code]) return REASONS[code];
  if (code >= 9000 && code <= 9999) return "Custom policy denied the routed trade";
  if (code >= 40000) return "External hook denied the routed trade";
  return "unknown";
}

export enum Regime {
  Normal = 0,
  Cautious = 1,
  Defensive = 2,
  CircuitBreaker = 3,
}

export enum SignalAction {
  NoOp = 0,
  ChangeStrategyState = 1,
  QueueBuyback = 2,
  QueueTwap = 3,
  IncreaseReserveTarget = 4,
  DecreaseReserveTarget = 5,
  PauseStrategy = 6,
  ResumeStrategy = 7,
}

export const MARKET_FLAGS = {
  CREATOR_VERIFIED: 1 << 0,
  PAUSED: 1 << 1,
  MIGRATION_PENDING: 1 << 2,
  BASE_TOKEN_2022: 1 << 3,
  HOLDER_REWARD_COIN: 1 << 4,
  MAYHEM: 1 << 5,
} as const;

export const LAMPORTS_PER_SOL = 1_000_000_000n;
