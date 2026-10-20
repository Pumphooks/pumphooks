//! Routed execution: Wallet → route_* → before hooks → Pump CPI → after hooks → settlement.
//!
//! The Pump trade accounts are passed as the first 17 remaining accounts in Pump's own order and
//! forwarded unchanged; the wallet signs the Pump instruction itself (no custody, no delegate).
//! Policy accounts follow, one per active policy bit, in index order.
//!
//! Hooks only govern trades that go through this router. Direct Pump trades bypass them.

use anchor_lang::{
    prelude::*,
    Discriminator,
    solana_program::{
        instruction::{AccountMeta, Instruction},
        program::invoke,
    },
};
use bytemuck::Zeroable;
use hook_runtime::{commit_trade, run_phase, PhaseOutcome, Trace};
use hook_types::{
    context_flags, HookContext, HookSlot, MarketStats, ModuleId, Side, UserState, Venue, ABI_VERSION, MAX_HOOK_SLOTS,
    MAX_POLICIES,
};
use policy_vm::PolicyData;
use pump_adapter::{
    accounts::{amm_v2, curve_v3},
    ix::{amm_v2_data, curve_v3_data, Mode},
    layout::{CurveView, PoolView, TokenAccountView},
    math, PUMP_AMM_PROGRAM_ID, PUMP_PROGRAM_ID,
};

use crate::{
    errors::{hook_err, PumpHooksError as E},
    events::*,
    state::*,
    ID,
};

#[derive(Accounts)]
pub struct Route<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(seeds = [GLOBAL_SEED], bump = global.bump)]
    pub global: Account<'info, GlobalConfig>,
    #[account(mut)]
    pub market: AccountLoader<'info, MarketConfig>,
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + UserMarketState::SIZE,
        seeds = [USER_SEED, market.key().as_ref(), user.key().as_ref()],
        bump
    )]
    pub user_state: AccountLoader<'info, UserMarketState>,
    pub system_program: Program<'info, System>,
}

/// Balances and reserves read before and after the Pump CPI.
#[derive(Clone, Copy, Default)]
struct Snapshot {
    user_base: u64,
    user_quote: u64,
    base_reserve: u64,
    quote_reserve: u64,
    /// Data lengths of the Pump accounts Pump may create or grow at the wallet's expense inside the
    /// trade: the user volume accumulator and the market account (legacy curve/pool resize).
    rent_lens: [usize; 2],
}

/// Rent the wallet paid inside the CPI for accounts Pump created or resized.
fn rent_outlay(pre: &Snapshot, post: &Snapshot) -> Result<u64> {
    let rent = Rent::get()?;
    let at = |len: usize| if len == 0 { 0 } else { rent.minimum_balance(len) };
    Ok(pre
        .rent_lens
        .iter()
        .zip(post.rent_lens)
        .map(|(&a, b)| if b > a { at(b).saturating_sub(at(a)) } else { 0 })
        .sum())
}

fn token(ai: &AccountInfo, owner: &Pubkey, mint: &Pubkey) -> Result<u64> {
    require!(pump_adapter::is_token_program(ai.owner), E::InvalidUserTokenAccount);
    let t = TokenAccountView::parse(&ai.try_borrow_data()?).map_err(|_| E::InvalidUserTokenAccount)?;
    require_keys_eq!(t.owner, *owner, E::InvalidUserTokenAccount);
    require_keys_eq!(t.mint, *mint, E::InvalidUserTokenAccount);
    Ok(t.amount)
}

fn vault_amount(ai: &AccountInfo) -> Result<u64> {
    Ok(TokenAccountView::parse(&ai.try_borrow_data()?).map_err(|_| E::PumpAccountMismatch)?.amount)
}

fn check_key(ai: &AccountInfo, k: &Pubkey) -> Result<()> {
    require_keys_eq!(ai.key(), *k, E::PumpAccountMismatch);
    Ok(())
}

fn snapshot_amm(m: &MarketConfig, user: &Pubkey, a: &[AccountInfo]) -> Result<(Snapshot, PoolView)> {
    let pool_ai = &a[amm_v2::POOL];
    check_key(pool_ai, &m.pool)?;
    require_keys_eq!(*pool_ai.owner, PUMP_AMM_PROGRAM_ID, E::PumpAccountMismatch);
    let pool = PoolView::parse(&pool_ai.try_borrow_data()?).map_err(|_| E::PumpAccountMismatch)?;
    require!(!pool.is_cashback, E::CashbackCoinUnsupported);
    check_key(&a[amm_v2::USER], user)?;
    check_key(&a[amm_v2::BASE_MINT], &m.mint)?;
    check_key(&a[amm_v2::QUOTE_MINT], &m.quote_mint)?;
    check_key(&a[amm_v2::POOL_BASE_TOKEN_ACCOUNT], &pool.pool_base_token_account)?;
    check_key(&a[amm_v2::POOL_QUOTE_TOKEN_ACCOUNT], &pool.pool_quote_token_account)?;
    check_key(&a[amm_v2::BASE_TOKEN_PROGRAM], &m.base_token_program)?;
    check_key(&a[amm_v2::QUOTE_TOKEN_PROGRAM], &m.quote_token_program)?;
    check_key(&a[amm_v2::PROGRAM], &PUMP_AMM_PROGRAM_ID)?;
    let quote_vault = vault_amount(&a[amm_v2::POOL_QUOTE_TOKEN_ACCOUNT])?;
    Ok((
        Snapshot {
            user_base: token(&a[amm_v2::USER_BASE_TOKEN_ACCOUNT], user, &m.mint)?,
            user_quote: token(&a[amm_v2::USER_QUOTE_TOKEN_ACCOUNT], user, &m.quote_mint)?,
            base_reserve: vault_amount(&a[amm_v2::POOL_BASE_TOKEN_ACCOUNT])?,
            quote_reserve: pool.effective_quote_reserve(quote_vault).ok_or(E::MathOverflow)?,
            rent_lens: [a[amm_v2::USER_VOLUME_ACCUMULATOR].data_len(), pool_ai.data_len()],
        },
        pool,
    ))
}

fn snapshot_curve(m: &MarketConfig, user: &Pubkey, a: &[AccountInfo]) -> Result<(Snapshot, CurveView)> {
    let bc_ai = &a[curve_v3::BONDING_CURVE];
    check_key(bc_ai, &m.bonding_curve)?;
    require_keys_eq!(*bc_ai.owner, PUMP_PROGRAM_ID, E::PumpAccountMismatch);
    let curve = CurveView::parse(&bc_ai.try_borrow_data()?).map_err(|_| E::PumpAccountMismatch)?;
    require!(!curve.is_cashback, E::CashbackCoinUnsupported);
    check_key(&a[curve_v3::USER], user)?;
    check_key(&a[curve_v3::BASE_MINT], &m.mint)?;
    check_key(&a[curve_v3::QUOTE_MINT], &m.quote_mint)?;
    check_key(&a[curve_v3::BASE_TOKEN_PROGRAM], &m.base_token_program)?;
    check_key(&a[curve_v3::QUOTE_TOKEN_PROGRAM], &m.quote_token_program)?;
    check_key(&a[curve_v3::PROGRAM], &PUMP_PROGRAM_ID)?;
    let user_quote = if curve.is_sol_quoted() {
        a[curve_v3::USER].lamports()
    } else {
        token(&a[curve_v3::ASSOCIATED_QUOTE_USER], user, &m.quote_mint)?
    };
    Ok((
        Snapshot {
            user_base: token(&a[curve_v3::ASSOCIATED_BASE_USER], user, &m.mint)?,
            user_quote,
            base_reserve: curve.virtual_token_reserves,
            quote_reserve: curve.virtual_quote_reserves,
            rent_lens: [a[curve_v3::USER_VOLUME_ACCUMULATOR].data_len(), bc_ai.data_len()],
        },
        curve,
    ))
}

fn cpi(program: Pubkey, meta: &[(bool, bool)], accts: &[AccountInfo], data: Vec<u8>) -> Result<()> {
    let metas = accts
        .iter()
        .zip(meta)
        .map(|(ai, &(s, w))| if w { AccountMeta::new(ai.key(), s) } else { AccountMeta::new_readonly(ai.key(), s) })
        .collect();
    invoke(&Instruction { program_id: program, accounts: metas, data }, accts)?;
    Ok(())
}

/// Constant-product pre-trade estimate of `(quote_amount, base_amount)`.
fn estimate(mode: Mode, amount: u64, limit: u64, base_r: u64, quote_r: u64) -> (u64, u64) {
    match mode {
        Mode::ExactQuoteIn => (amount, math::cp_base_out(base_r, quote_r, amount)),
        Mode::ExactBaseOut => (math::cp_quote_in(base_r, quote_r, amount).unwrap_or(limit), amount),
        Mode::ExactBaseIn => (math::cp_quote_out(base_r, quote_r, amount), amount),
    }
}

/// Heap-held working set of one routed trade. SBF stack frames are 4 KiB, so nothing large lives
/// on the stack of `route`.
struct Work {
    ctx: HookContext,
    slots: [HookSlot; MAX_HOOK_SLOTS],
    policies: [PolicyData; MAX_POLICIES],
    policy_mask: u8,
    user: UserState,
    stats: MarketStats,
    trace: Trace,
}

#[inline(never)]
fn new_work() -> Box<Work> {
    Box::new(Work {
        ctx: Zeroable::zeroed(),
        slots: [Zeroable::zeroed(); MAX_HOOK_SLOTS],
        policies: [Zeroable::zeroed(); MAX_POLICIES],
        policy_mask: 0,
        user: UserState::default(),
        stats: MarketStats::default(),
        trace: Trace::default(),
    })
}

/// Active policies follow the Pump block in the remaining accounts, one per mask bit, in index
/// order.
#[inline(never)]
fn load_policies<'a>(w: &mut Work, market_key: &Pubkey, extra: &'a [AccountInfo<'a>]) -> Result<()> {
    let mut it = extra.iter();
    for i in 0..MAX_POLICIES {
        if w.policy_mask & (1 << i) == 0 {
            continue;
        }
        let ai = it.next().ok_or(E::PolicyAccountMissing)?;
        let expected = Pubkey::find_program_address(&[POLICY_SEED, market_key.as_ref(), &[i as u8]], &ID).0;
        require_keys_eq!(ai.key(), expected, E::PolicyAccountMissing);
        require!(ai.is_writable, E::PolicyAccountMissing);
        w.policies[i] = AccountLoader::<Policy>::try_from(ai)?.load()?.read();
    }
    Ok(())
}

#[inline(never)]
fn store_policies<'a>(w: &Work, extra: &'a [AccountInfo<'a>]) -> Result<()> {
    let mut it = extra.iter();
    for i in 0..MAX_POLICIES {
        if w.policy_mask & (1 << i) == 0 {
            continue;
        }
        let ai = it.next().ok_or(E::PolicyAccountMissing)?;
        AccountLoader::<Policy>::try_from(ai)?.load_mut()?.write(&w.policies[i]);
    }
    Ok(())
}

#[inline(never)]
fn phase(w: &mut Work, side: Side, before: bool, disabled: u64) -> Result<Box<PhaseOutcome>> {
    let mask = w.policy_mask;
    let [p0, p1, p2, p3] = &mut w.policies;
    let mut refs = [
        (mask & 1 != 0).then_some(p0),
        (mask & 2 != 0).then_some(p1),
        (mask & 4 != 0).then_some(p2),
        (mask & 8 != 0).then_some(p3),
    ];
    let out = run_phase(side, before, &mut w.slots, &mut w.ctx, &w.stats, &mut w.user, &mut refs, disabled, &mut w.trace)
        .map_err(hook_err)?;
    Ok(Box::new(out))
}

#[inline(never)]
fn fill_context(c: &mut HookContext, m: &MarketConfig, venue: Venue, side: Side, mode: Mode, quote_native: bool) {
    c.version = ABI_VERSION;
    c.venue = venue as u8;
    c.side = side as u8;
    c.mode = mode as u8;
    let mut f = 0u8;
    if quote_native {
        f |= context_flags::QUOTE_NATIVE_SOL;
    }
    if m.has_flag(market_flags::BASE_TOKEN_2022) {
        f |= context_flags::BASE_TOKEN_2022;
    }
    if m.has_flag(market_flags::HOLDER_REWARD_COIN) {
        f |= context_flags::HOLDER_REWARD_COIN;
    }
    if m.has_flag(market_flags::MAYHEM) {
        f |= context_flags::MAYHEM;
    }
    if m.has_flag(market_flags::CREATOR_VERIFIED) {
        f |= context_flags::CREATOR_VERIFIED_MARKET;
    }
    c.flags = f;
    c.base_mint = m.mint.to_bytes();
    c.quote_mint = m.quote_mint.to_bytes();
    c.pool = if venue == Venue::PumpSwap { m.pool.to_bytes() } else { m.bonding_curve.to_bytes() };
    c.market_activated_at = m.activated_at;
}

fn trace_items(t: &Trace) -> Vec<TraceItem> {
    t.as_slice()
        .iter()
        .map(|e| TraceItem {
            callback: e.callback,
            slot_index: e.slot_index,
            module_id: e.module_id,
            allowed: e.allowed,
            reason_code: e.reason_code,
            flags: e.flags,
        })
        .collect()
}

#[inline(never)]
fn emit_denied(w: &Work, market: Pubkey, user: Pubkey, d: &hook_runtime::TraceEntry) {
    emit!(HookExecutionEvent { market, user, sequence: w.ctx.sequence, entries: trace_items(&w.trace) });
    emit!(HookDeniedEvent {
        market,
        user,
        sequence: w.ctx.sequence,
        module_id: d.module_id,
        callback: d.callback,
        slot_index: d.slot_index,
        reason_code: d.reason_code,
    });
    msg!("denied by module {} reason {}: {}", d.module_id, d.reason_code, hook_types::reason::describe(d.reason_code));
}

#[inline(never)]
fn emit_policies(market: Pubkey, sequence: u64, o: &PhaseOutcome) {
    for e in o.policy_events() {
        emit!(PolicyExecutionEvent {
            market,
            sequence,
            policy_index: e.policy_index,
            trigger: e.trigger,
            matched: e.matched,
            result: e.result,
            reason_code: e.reason_code,
            tag: e.tag,
        });
    }
}

#[inline(never)]
fn emit_settled(w: &Work, market: Pubkey, user: Pubkey, pre: &Snapshot, post: &Snapshot) {
    let c = &w.ctx;
    emit!(HookExecutionEvent { market, user, sequence: c.sequence, entries: trace_items(&w.trace) });
    emit!(RoutedSwapEvent {
        market,
        user,
        mint: Pubkey::new_from_array(c.base_mint),
        venue: c.venue,
        side: c.side,
        mode: c.mode,
        amount: c.amount,
        limit: c.limit,
        quote_amount: c.quote_amount,
        base_amount: c.base_amount,
        pre_base_reserves: pre.base_reserve,
        pre_quote_reserves: pre.quote_reserve,
        post_base_reserves: post.base_reserve,
        post_quote_reserves: post.quote_reserve,
        sequence: c.sequence,
        slot: c.slot,
        timestamp: c.timestamp,
        hooks_run: w.trace.len as u8,
        xp: w.user.xp,
        pending_xp: w.user.pending_xp,
        policy_points: w.user.policy_points,
    });
}

#[inline(never)]
fn snapshot(venue: Venue, m: &MarketConfig, user: &Pubkey, pump: &[AccountInfo]) -> Result<(Snapshot, bool, bool)> {
    match venue {
        Venue::PumpSwap => Ok((snapshot_amm(m, user, pump)?.0, false, false)),
        Venue::BondingCurve => {
            let (s, c) = snapshot_curve(m, user, pump)?;
            Ok((s, c.is_sol_quoted(), c.complete))
        }
    }
}

/// Observed `(quote_amount, base_amount)` of the executed trade from the wallet's balances.
fn observed(side: Side, pre: &Snapshot, post: &Snapshot, acc_rent: u64) -> Result<(u64, u64)> {
    if side == Side::Buy {
        let spent = pre.user_quote.checked_sub(post.user_quote).ok_or(E::UnexpectedBalanceChange)?;
        let got = post.user_base.checked_sub(pre.user_base).ok_or(E::UnexpectedBalanceChange)?;
        Ok((spent.saturating_sub(acc_rent), got))
    } else {
        // Proceeds = quote delta + rent the wallet paid inside the trade.
        let net = post.user_quote as i128 - pre.user_quote as i128 + acc_rent as i128;
        let got = u64::try_from(net).map_err(|_| E::UnexpectedBalanceChange)?;
        let sold = pre.user_base.checked_sub(post.user_base).ok_or(E::UnexpectedBalanceChange)?;
        Ok((got, sold))
    }
}

pub fn route<'info>(ctx: Context<'_, '_, 'info, 'info, Route<'info>>, venue: Venue, mode: u8, amount: u64, limit: u64) -> Result<()> {
    let mode = Mode::from_u8(mode).ok_or(E::InvalidMode)?;
    require!(amount > 0, E::ZeroAmount);
    require!(!ctx.accounts.global.paused, E::ProtocolPaused);
    let n = match venue {
        Venue::BondingCurve => curve_v3::LEN,
        Venue::PumpSwap => amm_v2::LEN,
    };
    require!(ctx.remaining_accounts.len() >= n, E::PumpAccountsMissing);
    let (pump, extra) = ctx.remaining_accounts.split_at(n);

    let market_key = ctx.accounts.market.key();
    let user_key = ctx.accounts.user.key();
    let clock = Clock::get()?;
    let disabled = ctx.accounts.global.disabled_modules;

    // User state is created by the wallet's first routed trade.
    let fresh = {
        let ai = ctx.accounts.user_state.to_account_info();
        let d = ai.try_borrow_data()?;
        d[..8] == [0u8; 8]
    };
    if fresh {
        {
            let mut us = ctx.accounts.user_state.load_init()?;
            us.market = market_key;
            us.user = user_key;
            us.created_at = clock.unix_timestamp;
            us.bump = ctx.bumps.user_state;
        }
        // Anchor writes zero-copy discriminators on exit; `load_mut` below needs it now.
        let ai = ctx.accounts.user_state.to_account_info();
        ai.try_borrow_mut_data()?[..8].copy_from_slice(UserMarketState::DISCRIMINATOR);
    }

    let mut m = ctx.accounts.market.load_mut()?;
    require!(!m.has_flag(market_flags::PAUSED), E::MarketPaused);
    match venue {
        Venue::PumpSwap => require!(m.venue == Venue::PumpSwap as u8, E::MarketNotMigrated),
        Venue::BondingCurve => require!(m.venue == Venue::BondingCurve as u8, E::MarketMigrated),
    }
    let (pre, quote_native, complete) = snapshot(venue, &m, &user_key, pump)?;
    require!(!complete, E::MarketMigrated);

    let side = if mode.is_buy() { Side::Buy } else { Side::Sell };
    let (q_est, b_est) = estimate(mode, amount, limit, pre.base_reserve, pre.quote_reserve);
    m.sequence = m.sequence.checked_add(1).ok_or(E::MathOverflow)?;

    let mut w = new_work();
    fill_context(&mut w.ctx, &m, venue, side, mode, quote_native);
    w.ctx.market = market_key.to_bytes();
    w.ctx.user = user_key.to_bytes();
    w.ctx.amount = amount;
    w.ctx.limit = limit;
    w.ctx.quote_amount = q_est;
    w.ctx.base_amount = b_est;
    w.ctx.pre_base_reserves = pre.base_reserve;
    w.ctx.pre_quote_reserves = pre.quote_reserve;
    w.ctx.post_base_reserves = pre.base_reserve;
    w.ctx.post_quote_reserves = pre.quote_reserve;
    w.ctx.timestamp = clock.unix_timestamp;
    w.ctx.slot = clock.slot;
    w.ctx.sequence = m.sequence;
    w.ctx.user_base_balance = pre.user_base;
    w.slots = m.read_slots();
    w.policy_mask = w.slots.iter().find(|s| s.module_id == ModuleId::Policy as u16).map(|s| s.config[0]).unwrap_or(0);
    w.stats = m.stats();
    load_policies(&mut w, &market_key, extra)?;
    let mut us = ctx.accounts.user_state.load_mut()?;
    w.user = us.read();

    // ---- before phase
    let before = phase(&mut w, side, true, disabled)?;
    if let Some(d) = before.denied {
        emit_denied(&w, market_key, user_key, &d);
        return err!(E::HookDenied);
    }

    // ---- Pump CPI: the wallet signs; tokens move wallet <-> Pump directly.
    match venue {
        Venue::PumpSwap => cpi(PUMP_AMM_PROGRAM_ID, &amm_v2::META, pump, amm_v2_data(mode, amount, limit))?,
        Venue::BondingCurve => cpi(PUMP_PROGRAM_ID, &curve_v3::META, pump, curve_v3_data(mode, amount, limit))?,
    }

    // ---- observe
    let (post, _, completed) = snapshot(venue, &m, &user_key, pump)?;
    let acc_rent = if quote_native { rent_outlay(&pre, &post)? } else { 0 };    let (quote_amount, base_amount) = observed(side, &pre, &post, acc_rent)?;
    w.ctx.quote_amount = quote_amount;
    w.ctx.base_amount = base_amount;
    w.ctx.post_base_reserves = post.base_reserve;
    w.ctx.post_quote_reserves = post.quote_reserve;
    w.ctx.user_base_balance = post.user_base;
    w.ctx.flags |= context_flags::AMOUNTS_OBSERVED;

    // ---- after phase
    let after = phase(&mut w, side, false, disabled)?;

    // ---- settle
    let Work { ctx: hctx, stats, user, .. } = &mut *w;
    commit_trade(hctx, stats, user);
    m.write_slots(&w.slots);
    m.set_stats(&w.stats);
    if completed {
        m.flags |= market_flags::MIGRATION_PENDING;
    }
    us.write(&w.user);
    drop(us);
    drop(m);
    store_policies(&w, extra)?;

    emit_settled(&w, market_key, user_key, &pre, &post);
    emit_policies(market_key, w.ctx.sequence, &before);
    emit_policies(market_key, w.ctx.sequence, &after);
    Ok(())
}
