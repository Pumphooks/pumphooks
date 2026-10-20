import {
  AccountInfo,
  AddressLookupTableAccount,
  ComputeBudgetProgram,
  Connection,
  GetProgramAccountsResponse,
  PublicKey,
  Signer,
  TransactionInstruction,
  TransactionMessage,
  VersionedTransaction,
} from "@solana/web3.js";
import {
  decodeHookDefinition,
  decodeMarketConfig,
  decodePolicy,
  decodeUserMarketState,
  HookDefinition,
  MARKET_CONFIG_SIZE,
  MARKET_MINT_OFFSET,
  MarketConfig,
  PolicyAccount,
  UserMarketState,
} from "./accounts.js";
import { ZERO_KEY } from "./codec.js";
import { encodeConfig, ModuleConfig } from "./configs.js";
import {
  MARKET_FLAGS,
  ModuleId,
  moduleInfo,
  NATIVE_MINT,
  PUMP_HOOKS_PROGRAM_ID,
  PUMP_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
  TOKEN_2022_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  TradeMode,
  Venue,
} from "./constants.js";
import { decodeEvents, decodeProgramError, eventsOf, HookDeniedEvent, HookExecutionEvent, PolicyExecutionEvent, ProgramErrorInfo, PumpHooksEvent, RoutedSwapEvent } from "./events.js";
import { createMarketIx, createPolicyIx, installHookIx, removeHookIx, routeIx, submitSignalIx } from "./instructions.js";
import { ammPda, associatedToken, hooksPda, pumpPda } from "./pda.js";
import { compilePolicy, PolicySpec } from "./policy.js";
import {
  ammV2Metas,
  cp,
  CurveView,
  curveV3Metas,
  decodeAmmBuybackRecipients,
  decodeCurve,
  decodeCurveBuybackRecipients,
  decodePool,
  decodeTokenAccount,
  isBuy,
  PoolView,
} from "./pump.js";
import { signalVerifyIx } from "./signal.js";
import { closeAccountIx, createAtaIdempotentIx, wrapSolIxs } from "./spl.js";

/** The subset of `Connection` the client uses. Read paths need only the first two methods. */
export type RpcLike = Pick<Connection, "getAccountInfo" | "getMultipleAccountsInfo"> &
  Partial<Pick<Connection, "getProgramAccounts" | "simulateTransaction" | "getTransaction" | "getLatestBlockhash" | "sendRawTransaction" | "confirmTransaction" | "getAddressLookupTable">>;

export interface ClientOptions {
  programId?: PublicKey;
  /** Address lookup tables used when building v0 transactions (e.g. `PUMP_ALT.devnet`). */
  lookupTables?: PublicKey[];
  /** Compute unit limit for routed trades. Measured 88k CU for 4 hooks; default leaves headroom. */
  computeUnits?: number;
  /** Optional priority fee in micro-lamports per CU. */
  microLamportsPerCu?: number;
  /** Choose among Pump's configured buyback fee recipients (default: uniform random). */
  pickBuybackRecipient?: (candidates: PublicKey[], venue: Venue) => PublicKey;
}

/** What the SDK detects about an existing Pump token before any hook is installed. */
export interface PumpTokenInfo {
  mint: PublicKey;
  baseTokenProgram: PublicKey;
  isToken2022: boolean;
  decimals: number;
  bondingCurve: PublicKey;
  curve: CurveView;
  creator: PublicKey;
  quoteMint: PublicKey;
  quoteTokenProgram: PublicKey;
  /** Where routed trades execute now. `undefined` while a completed curve awaits migration. */
  venue: Venue | undefined;
  pool: PublicKey;
  poolView?: PoolView;
  reserves: { base: bigint; quote: bigint };
  isCashback: boolean;
  isHolderReward: boolean;
  isMayhem: boolean;
  routable: boolean;
  notRoutableReason?: string;
}

export interface StackHook {
  module: ModuleId | string;
  config?: ModuleConfig;
  priority?: number;
  callbacks?: number;
}

export interface HookedMarket {
  address: PublicKey;
  config: MarketConfig;
}

export interface TradeParams {
  market: HookedMarket | PublicKey;
  user: PublicKey;
  mode: TradeMode;
  amount: bigint;
  limit: bigint;
  /** Close the wallet's wSOL account after a PumpSwap trade. Default: only if this tx created it. */
  unwrapSol?: boolean;
}

export interface BuiltTransaction {
  instructions: TransactionInstruction[];
  /** Build a v0 transaction once a blockhash is known. */
  toTransaction(recentBlockhash: string, payer?: PublicKey): VersionedTransaction;
}

export interface ExecutionResult {
  ok: boolean;
  error?: ProgramErrorInfo;
  rawError?: unknown;
  logs: string[];
  unitsConsumed?: number;
  events: PumpHooksEvent[];
  trace?: HookExecutionEvent;
  swap?: RoutedSwapEvent;
  denied?: HookDeniedEvent;
  policies: PolicyExecutionEvent[];
}

export function executionResult(logs: string[], err: unknown, unitsConsumed?: number, programId = PUMP_HOOKS_PROGRAM_ID): ExecutionResult {
  const events = decodeEvents(logs, programId);
  return {
    ok: !err,
    error: err ? decodeProgramError(err, logs) : undefined,
    rawError: err ?? undefined,
    logs,
    unitsConsumed,
    events,
    trace: eventsOf(events, "HookExecutionEvent")[0],
    swap: eventsOf(events, "RoutedSwapEvent")[0],
    denied: eventsOf(events, "HookDeniedEvent")[0],
    policies: eventsOf(events, "PolicyExecutionEvent"),
  };
}

const must = <T>(v: T | null | undefined, msg: string): T => {
  if (v === null || v === undefined) throw new Error(msg);
  return v;
};

export class PumpHooksClient {
  readonly programId: PublicKey;
  private alts?: AddressLookupTableAccount[];

  constructor(
    readonly rpc: RpcLike,
    readonly opts: ClientOptions = {},
  ) {
    this.programId = opts.programId ?? PUMP_HOOKS_PROGRAM_ID;
  }

  // ---------------------------------------------------------------- reads

  /** Inspect an existing Pump token: curve or pool, quote mint, token program, routability. */
  async detect(mint: PublicKey): Promise<PumpTokenInfo> {
    const bondingCurve = pumpPda.bondingCurve(mint);
    const [mintAi, curveAi] = await this.rpc.getMultipleAccountsInfo([mint, bondingCurve]);
    if (!mintAi) throw new Error(`mint ${mint.toBase58()} not found`);
    const baseTokenProgram = mintAi.owner;
    if (!baseTokenProgram.equals(TOKEN_PROGRAM_ID) && !baseTokenProgram.equals(TOKEN_2022_PROGRAM_ID)) throw new Error("account is not a token mint");
    if (!curveAi || !curveAi.owner.equals(PUMP_PROGRAM_ID)) throw new Error("not a Pump token: no Pump bonding curve for this mint");
    const curve = decodeCurve(curveAi.data);
    const quoteMint = curve.quoteMint;
    const quoteTokenProgram = quoteMint.equals(NATIVE_MINT) ? TOKEN_PROGRAM_ID : must(await this.rpc.getAccountInfo(quoteMint), "quote mint missing").owner;
    const pool = ammPda.canonicalPool(mint, quoteMint);

    let venue: Venue | undefined = Venue.BondingCurve;
    let poolView: PoolView | undefined;
    let reserves = { base: curve.virtualTokenReserves, quote: curve.virtualQuoteReserves };
    if (curve.complete) {
      const poolAi = await this.rpc.getAccountInfo(pool);
      if (poolAi) {
        poolView = decodePool(poolAi.data);
        venue = Venue.PumpSwap;
        const [b, q] = await this.rpc.getMultipleAccountsInfo([poolView.poolBaseTokenAccount, poolView.poolQuoteTokenAccount]);
        const base = decodeTokenAccount(must(b, "pool base vault missing").data).amount;
        const quote = decodeTokenAccount(must(q, "pool quote vault missing").data).amount + poolView.virtualQuoteReserves;
        reserves = { base, quote: quote < 0n ? 0n : quote };
      } else {
        venue = undefined;
      }
    }
    const isCashback = poolView?.isCashback ?? curve.isCashback;
    const notRoutableReason = isCashback
      ? "cashback coins are not routable through Pump v2/v3 instructions"
      : venue === undefined
        ? "bonding curve complete; waiting for migration to PumpSwap"
        : undefined;
    return {
      mint,
      baseTokenProgram,
      isToken2022: baseTokenProgram.equals(TOKEN_2022_PROGRAM_ID),
      decimals: mintAi.data[44],
      bondingCurve,
      curve,
      creator: curve.creator,
      quoteMint,
      quoteTokenProgram,
      venue,
      pool,
      poolView,
      reserves,
      isCashback,
      isHolderReward: poolView?.isHolderReward ?? curve.isHolderReward,
      isMayhem: poolView?.isMayhem ?? curve.isMayhem,
      routable: !notRoutableReason,
      notRoutableReason,
    };
  }

  marketAddress(mint: PublicKey, namespace: PublicKey): PublicKey {
    return hooksPda.market(mint, namespace, this.programId);
  }

  async getMarket(market: PublicKey): Promise<HookedMarket> {
    const ai = must(await this.rpc.getAccountInfo(market), `market ${market.toBase58()} not found`);
    return { address: market, config: decodeMarketConfig(ai.data) };
  }

  async getMarketFor(mint: PublicKey, namespace: PublicKey): Promise<HookedMarket | undefined> {
    const address = this.marketAddress(mint, namespace);
    const ai = await this.rpc.getAccountInfo(address);
    return ai ? { address, config: decodeMarketConfig(ai.data) } : undefined;
  }

  /** All hooked markets for a mint (one per namespace). Requires `getProgramAccounts`. */
  async findMarkets(mint: PublicKey): Promise<HookedMarket[]> {
    const gpa = must(this.rpc.getProgramAccounts, "RPC does not expose getProgramAccounts").bind(this.rpc);
    const res = (await gpa(this.programId, {
      filters: [{ dataSize: MARKET_CONFIG_SIZE }, { memcmp: { offset: MARKET_MINT_OFFSET, bytes: mint.toBase58() } }],
    })) as GetProgramAccountsResponse;
    return res.map(({ pubkey, account }) => ({ address: pubkey, config: decodeMarketConfig(account.data) }));
  }

  async getUserState(market: PublicKey, user: PublicKey): Promise<UserMarketState | undefined> {
    const ai = await this.rpc.getAccountInfo(hooksPda.userState(market, user, this.programId));
    return ai ? decodeUserMarketState(ai.data) : undefined;
  }

  async getPolicies(market: HookedMarket): Promise<PolicyAccount[]> {
    const idx = policyIndexes(market.config.policyMask);
    const ais = await this.rpc.getMultipleAccountsInfo(idx.map((i) => hooksPda.policy(market.address, i, this.programId)));
    return ais.filter((a): a is NonNullable<typeof a> => !!a).map((a) => decodePolicy(a.data));
  }

  async getHookDefinition(moduleId: number): Promise<HookDefinition | undefined> {
    const ai = await this.rpc.getAccountInfo(hooksPda.hookDefinition(moduleId, this.programId));
    return ai ? decodeHookDefinition(ai.data) : undefined;
  }

  /**
   * Pre-trade estimate from current reserves (constant product, before Pump fees). Use
   * `simulate` for the exact amount Pump will execute.
   */
  async quote(mint: PublicKey, mode: TradeMode, amount: bigint) {
    const t = await this.detect(mint);
    const { base, quote } = t.reserves;
    switch (mode) {
      case TradeMode.ExactQuoteIn:
        return { baseOut: cp.baseOut(amount, base, quote), quoteIn: amount, method: "constant-product, pre-fee" as const };
      case TradeMode.ExactBaseOut:
        return { baseOut: amount, quoteIn: cp.quoteIn(amount, base, quote), method: "constant-product, pre-fee" as const };
      case TradeMode.ExactBaseIn:
        return { baseIn: amount, quoteOut: cp.quoteOut(amount, base, quote), method: "constant-product, pre-fee" as const };
    }
  }

  quoteBuy(mint: PublicKey, quoteIn: bigint) {
    return this.quote(mint, TradeMode.ExactQuoteIn, quoteIn);
  }

  // ---------------------------------------------------------------- market setup

  /** Turn an existing Pump token into a programmable market owned by `authority`. */
  async createMarket(mint: PublicKey, authority: PublicKey): Promise<{ market: PublicKey; ix: TransactionInstruction; token: PumpTokenInfo }> {
    const token = await this.detect(mint);
    if (!token.routable) throw new Error(token.notRoutableReason);
    const ix = createMarketIx({
      authority,
      mint,
      bondingCurve: token.bondingCurve,
      quoteMint: token.quoteMint,
      pool: token.venue === Venue.PumpSwap ? token.pool : SYSTEM_PROGRAM_ID,
      programId: this.programId,
    });
    return { market: this.marketAddress(mint, authority), ix, token };
  }

  installHook(p: {
    market: PublicKey;
    authority: PublicKey;
    module: ModuleId | string;
    config?: ModuleConfig;
    priority?: number;
    /** Callback mask; 0/undefined = module defaults. */
    callbacks?: number;
    /** Granted capabilities; default = the module's full (registered) set. */
    capabilities?: number;
  }): TransactionInstruction {
    const info = moduleInfo(p.module);
    if (!info.available) throw new Error(`${info.name} is not available in this release`);
    return installHookIx({
      authority: p.authority,
      market: p.market,
      moduleId: info.id,
      priority: p.priority ?? info.defaultPriority,
      callbacks: p.callbacks ?? 0,
      capabilities: p.capabilities ?? info.capabilities,
      config: info.id === ModuleId.Policy ? new Uint8Array(48) : encodeConfig(info.id, p.config ?? {}),
      programId: this.programId,
    });
  }

  removeHook(p: { market: PublicKey; authority: PublicKey; module: ModuleId | string }): TransactionInstruction {
    return removeHookIx({ authority: p.authority, market: p.market, moduleId: moduleInfo(p.module).id, programId: this.programId });
  }

  /** Compile a Composer policy and build `create_policy`. The market needs the Policy module. */
  createPolicy(p: { market: HookedMarket; authority: PublicKey; spec: PolicySpec; index?: number }): { index: number; ix: TransactionInstruction } {
    const slots = p.market.config.slots;
    if (!slots.some((s) => s.moduleId === ModuleId.Policy)) throw new Error("install the Custom Policy module first");
    const index = p.index ?? [0, 1, 2, 3].find((i) => !(p.market.config.policyMask & (1 << i)));
    if (index === undefined) throw new Error("market already has 4 policies");
    const args = compilePolicy(p.spec, { hasVolatilityGuard: slots.some((s) => s.moduleId === ModuleId.VolatilityGuard) });
    return { index, ix: createPolicyIx({ authority: p.authority, market: p.market.address, index, args, programId: this.programId }) };
  }

  // ---------------------------------------------------------------- routed trades

  private recipients = new Map<string, PublicKey[]>();

  private async buybackRecipient(venue: Venue): Promise<PublicKey> {
    const key = venue === Venue.PumpSwap ? ammPda.globalConfig() : pumpPda.global();
    let list = this.recipients.get(key.toBase58());
    if (!list) {
      const ai = must(await this.rpc.getAccountInfo(key), "Pump global account missing");
      list = venue === Venue.PumpSwap ? decodeAmmBuybackRecipients(ai.data) : decodeCurveBuybackRecipients(ai.data);
      if (list.length === 0) throw new Error("Pump has no buyback fee recipients configured");
      this.recipients.set(key.toBase58(), list);
    }
    return this.opts.pickBuybackRecipient?.(list, venue) ?? list[Math.floor(Math.random() * list.length)];
  }

  /**
   * Build a routed trade: wallet → Pump Hooks (before hooks) → Pump/PumpSwap CPI signed by the
   * wallet → after hooks → settlement. Tokens move between the wallet and Pump directly.
   */
  async buildTrade(p: TradeParams): Promise<BuiltTransaction> {
    const market = p.market instanceof PublicKey ? await this.getMarket(p.market) : p.market;
    const m = market.config;
    if (m.flags & MARKET_FLAGS.PAUSED) throw new Error("market is paused");
    const user = p.user;
    const buy = isBuy(p.mode);
    const recipient = await this.buybackRecipient(m.venue);
    const ixs: TransactionInstruction[] = [ComputeBudgetProgram.setComputeUnitLimit({ units: this.opts.computeUnits ?? 300_000 })];
    if (this.opts.microLamportsPerCu) ixs.push(ComputeBudgetProgram.setComputeUnitPrice({ microLamports: this.opts.microLamportsPerCu }));
    ixs.push(createAtaIdempotentIx(user, user, m.mint, m.baseTokenProgram));

    const solQuote = m.quoteMint.equals(NATIVE_MINT);
    let pumpAccounts;
    let closeWsol = false;
    if (m.venue === Venue.PumpSwap) {
      const poolAi = must(await this.rpc.getAccountInfo(m.pool), "pool missing");
      const pool = decodePool(poolAi.data);
      pumpAccounts = ammV2Metas(
        {
          pool: m.pool,
          baseMint: m.mint,
          quoteMint: m.quoteMint,
          poolBaseTokenAccount: pool.poolBaseTokenAccount,
          poolQuoteTokenAccount: pool.poolQuoteTokenAccount,
          baseTokenProgram: m.baseTokenProgram,
          quoteTokenProgram: m.quoteTokenProgram,
        },
        user,
        recipient,
      );
      if (solQuote) {
        const wsol = associatedToken(user, TOKEN_PROGRAM_ID, NATIVE_MINT);
        const existed = !!(await this.rpc.getAccountInfo(wsol));
        const fund = buy ? (p.mode === TradeMode.ExactQuoteIn ? p.amount : p.limit) : 0n;
        ixs.push(...wrapSolIxs(user, fund));
        closeWsol = p.unwrapSol ?? !existed;
      } else {
        ixs.push(createAtaIdempotentIx(user, user, m.quoteMint, m.quoteTokenProgram));
      }
    } else {
      pumpAccounts = curveV3Metas(
        { baseMint: m.mint, quoteMint: m.quoteMint, baseTokenProgram: m.baseTokenProgram, quoteTokenProgram: m.quoteTokenProgram },
        user,
        recipient,
      );
      if (!solQuote) ixs.push(createAtaIdempotentIx(user, user, m.quoteMint, m.quoteTokenProgram));
    }

    ixs.push(
      routeIx({
        user,
        market: market.address,
        venue: m.venue,
        mode: p.mode,
        amount: p.amount,
        limit: p.limit,
        pumpAccounts,
        policyIndexes: policyIndexes(m.policyMask),
        programId: this.programId,
      }),
    );
    if (closeWsol) ixs.push(closeAccountIx(associatedToken(user, TOKEN_PROGRAM_ID, NATIVE_MINT), user, user));
    return this.built(ixs, user);
  }

  /** Spend exactly `quoteIn` (lamports for SOL markets), receiving at least `minBaseOut`. */
  buy(p: { market: HookedMarket | PublicKey; user: PublicKey; quoteIn: bigint; minBaseOut?: bigint }) {
    return this.buildTrade({ market: p.market, user: p.user, mode: TradeMode.ExactQuoteIn, amount: p.quoteIn, limit: p.minBaseOut ?? 1n });
  }

  sell(p: { market: HookedMarket | PublicKey; user: PublicKey; baseIn: bigint; minQuoteOut?: bigint }) {
    return this.buildTrade({ market: p.market, user: p.user, mode: TradeMode.ExactBaseIn, amount: p.baseIn, limit: p.minQuoteOut ?? 0n });
  }

  /**
   * One transaction that creates the market, installs `hooks` and routes a buy of `quoteIn`, so a
   * stack can be simulated before anything exists on-chain. Nothing is persisted by `simulate`.
   * Config-only modules are left out: they do not run during trades. The compute-budget
   * instruction is omitted so four runtime hooks fit in one 1232-byte v0 transaction with the
   * Pump lookup table; default per-instruction limits cover the measured cost.
   */
  async previewStack(p: { mint: PublicKey; authority: PublicKey; hooks: StackHook[]; quoteIn: bigint }): Promise<BuiltTransaction> {
    const { market, ix, token } = await this.createMarket(p.mint, p.authority);
    const runtime = p.hooks.filter((h) => moduleInfo(h.module).runtime);
    const installs = runtime.map((h) => this.installHook({ ...h, market, authority: p.authority }));
    const z = PublicKey.default;
    const config: MarketConfig = {
      namespace: p.authority, authority: p.authority, pendingAuthority: z, mint: p.mint, bondingCurve: token.bondingCurve,
      pool: token.venue === Venue.PumpSwap ? token.pool : SYSTEM_PROGRAM_ID, quoteMint: token.quoteMint,
      baseTokenProgram: token.baseTokenProgram, quoteTokenProgram: token.quoteTokenProgram, strategyVault: z,
      createdAt: 0n, activatedAt: 0n, routedBuyQuote: 0n, routedSellQuote: 0n, tradeCount: 0n, sequence: 0n,
      flags: 0, venue: must(token.venue, "market has no venue"), policyMask: 0, slots: [],
    };
    const trade = await this.buildTrade({ market: { address: market, config }, user: p.authority, mode: TradeMode.ExactQuoteIn, amount: p.quoteIn, limit: 1n });
    const rest = trade.instructions.filter((i) => !i.programId.equals(ComputeBudgetProgram.programId));
    return this.built([ix, ...installs, ...rest], p.authority);
  }

  /** Relay a SENTIENT signal signed off-chain by the market's configured engine key. */
  submitSignal(p: { market: PublicKey; signer: PublicKey; signal: Uint8Array; signature: Uint8Array; payer: PublicKey }): BuiltTransaction {
    return this.built([signalVerifyIx(p.signer, p.signal, p.signature), submitSignalIx({ market: p.market, signal: p.signal, programId: this.programId })], p.payer);
  }

  // ---------------------------------------------------------------- transactions

  async loadLookupTables(): Promise<AddressLookupTableAccount[]> {
    if (this.alts) return this.alts;
    const get = this.rpc.getAddressLookupTable?.bind(this.rpc);
    const out: AddressLookupTableAccount[] = [];
    for (const k of this.opts.lookupTables ?? []) {
      const r = get ? await get(k) : undefined;
      if (r?.value) out.push(r.value);
    }
    this.alts = out;
    return out;
  }

  private built(instructions: TransactionInstruction[], payer: PublicKey): BuiltTransaction {
    return {
      instructions,
      toTransaction: (recentBlockhash, feePayer) => bundle(instructions, payer, this.alts ?? []).toTransaction(recentBlockhash, feePayer),
    };
  }

  async transaction(b: BuiltTransaction, payer?: PublicKey): Promise<VersionedTransaction> {
    await this.loadLookupTables();
    const { blockhash } = await must(this.rpc.getLatestBlockhash, "RPC cannot fetch blockhash").call(this.rpc);
    return b.toTransaction(blockhash, payer);
  }

  /** Simulate without signatures; returns decoded hook trace, swap, denial and policy events. */
  async simulate(b: BuiltTransaction | VersionedTransaction, payer?: PublicKey): Promise<ExecutionResult> {
    const tx = b instanceof VersionedTransaction ? b : await this.transaction(b, payer);
    const sim = must(this.rpc.simulateTransaction, "RPC cannot simulate").bind(this.rpc);
    const r = await sim(tx, { sigVerify: false, replaceRecentBlockhash: true });
    return executionResult(r.value.logs ?? [], r.value.err, r.value.unitsConsumed, this.programId);
  }

  async send(b: BuiltTransaction, signers: Signer[]): Promise<{ signature: string; result: ExecutionResult }> {
    const tx = await this.transaction(b, signers[0].publicKey);
    tx.sign(signers);
    const signature = await must(this.rpc.sendRawTransaction, "RPC cannot send").call(this.rpc, tx.serialize(), { skipPreflight: false });
    await must(this.rpc.confirmTransaction, "RPC cannot confirm").call(this.rpc, signature, "confirmed");
    return { signature, result: await this.getExecutionTrace(signature) };
  }

  /** Decode the hook execution of a confirmed transaction from its logs. */
  async getExecutionTrace(signature: string): Promise<ExecutionResult & { slot?: number; blockTime?: number | null }> {
    const get = must(this.rpc.getTransaction, "RPC cannot fetch transactions").bind(this.rpc);
    const tx = must(await get(signature, { maxSupportedTransactionVersion: 0, commitment: "confirmed" }), `transaction ${signature} not found`);
    return { ...executionResult(tx.meta?.logMessages ?? [], tx.meta?.err, tx.meta?.computeUnitsConsumed, this.programId), slot: tx.slot, blockTime: tx.blockTime };
  }
}

/** Wrap instructions as a `BuiltTransaction` (v0 message, optional lookup tables). */
export function bundle(instructions: TransactionInstruction[], payer: PublicKey, alts: AddressLookupTableAccount[] = []): BuiltTransaction {
  return {
    instructions,
    toTransaction: (recentBlockhash, feePayer = payer) =>
      new VersionedTransaction(new TransactionMessage({ payerKey: feePayer, recentBlockhash, instructions }).compileToV0Message(alts)),
  };
}

export const policyIndexes = (mask: number) => [0, 1, 2, 3].filter((i) => mask & (1 << i));

export { ZERO_KEY };
