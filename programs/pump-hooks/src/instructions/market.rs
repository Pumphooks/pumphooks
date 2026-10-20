use anchor_lang::prelude::*;
use hook_types::Venue;
use pump_adapter::{
    is_token_program,
    layout::{CurveView, PoolView},
    pda, resolve_quote_mint, PUMP_AMM_PROGRAM_ID, PUMP_PROGRAM_ID, TOKEN_2022_PROGRAM_ID,
};

use crate::{
    errors::PumpHooksError as E,
    events::{MarketCreated, MarketPausedSet, MarketSynced},
    state::*,
};

#[derive(Accounts)]
pub struct CreateMarket<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(mut, seeds = [GLOBAL_SEED], bump = global.bump)]
    pub global: Account<'info, GlobalConfig>,
    /// CHECK: must be owned by SPL Token or Token-2022 (checked in handler).
    pub mint: UncheckedAccount<'info>,
    /// CHECK: must be the mint's Pump `bonding-curve` PDA, owned by Pump (checked in handler).
    pub bonding_curve: UncheckedAccount<'info>,
    /// CHECK: resolved quote mint (wSOL for SOL markets); owner gives the quote token program.
    pub quote_mint: UncheckedAccount<'info>,
    /// CHECK: canonical PumpSwap pool if the coin has migrated; ignored otherwise.
    pub pool: UncheckedAccount<'info>,
    #[account(
        init,
        payer = authority,
        space = 8 + MarketConfig::SIZE,
        seeds = [MARKET_SEED, mint.key().as_ref(), authority.key().as_ref()],
        bump
    )]
    pub market: AccountLoader<'info, MarketConfig>,
    pub system_program: Program<'info, System>,
}

pub struct Detected {
    pub curve: CurveView,
    pub quote_mint: Pubkey,
    pub pool: Option<PoolView>,
}

/// Read the Pump state of `mint` from its curve (and pool if migrated).
pub fn detect(mint: &Pubkey, curve_ai: &AccountInfo, quote_ai: &AccountInfo, pool_ai: &AccountInfo) -> Result<Detected> {
    require_keys_eq!(curve_ai.key(), pda::pump::bonding_curve(mint).0, E::InvalidBondingCurve);
    require_keys_eq!(*curve_ai.owner, PUMP_PROGRAM_ID, E::InvalidBondingCurve);
    let curve = CurveView::parse(&curve_ai.try_borrow_data()?).map_err(|_| E::InvalidBondingCurve)?;
    let quote_mint = resolve_quote_mint(&curve.quote_mint);
    require_keys_eq!(quote_ai.key(), quote_mint, E::InvalidQuoteMint);
    require!(is_token_program(quote_ai.owner), E::InvalidQuoteMint);
    let pool = if curve.complete {
        require_keys_eq!(pool_ai.key(), pda::amm::canonical_pool(mint, &quote_mint), E::InvalidPool);
        require_keys_eq!(*pool_ai.owner, PUMP_AMM_PROGRAM_ID, E::InvalidPool);
        let p = PoolView::parse(&pool_ai.try_borrow_data()?).map_err(|_| E::InvalidPool)?;
        require_keys_eq!(p.base_mint, *mint, E::InvalidPool);
        require_keys_eq!(p.quote_mint, quote_mint, E::InvalidPool);
        Some(p)
    } else {
        None
    };
    Ok(Detected { curve, quote_mint, pool })
}

pub fn create_market(ctx: Context<CreateMarket>) -> Result<()> {
    let a = &ctx.accounts;
    require!(!a.global.paused, E::ProtocolPaused);
    let mint = a.mint.key();
    require!(is_token_program(a.mint.owner), E::InvalidMint);
    let d = detect(&mint, &a.bonding_curve, &a.quote_mint, &a.pool)?;
    let cashback = d.curve.is_cashback || d.pool.map(|p| p.is_cashback).unwrap_or(false);
    require!(!cashback, E::CashbackCoinUnsupported);

    let authority = a.authority.key();
    let verified = d.curve.creator == authority || d.pool.map(|p| p.coin_creator == authority).unwrap_or(false);
    let now = Clock::get()?.unix_timestamp;

    let mut m = ctx.accounts.market.load_init()?;
    m.namespace = authority;
    m.authority = authority;
    m.mint = mint;
    m.bonding_curve = ctx.accounts.bonding_curve.key();
    m.pool = if d.pool.is_some() { ctx.accounts.pool.key() } else { Pubkey::default() };
    m.quote_mint = d.quote_mint;
    m.base_token_program = *ctx.accounts.mint.owner;
    m.quote_token_program = *ctx.accounts.quote_mint.owner;
    m.created_at = now;
    m.activated_at = now;
    m.venue = if d.pool.is_some() { Venue::PumpSwap as u8 } else { Venue::BondingCurve as u8 };
    m.bump = ctx.bumps.market;
    let mut flags = 0;
    if verified {
        flags |= market_flags::CREATOR_VERIFIED;
    }
    if m.base_token_program == TOKEN_2022_PROGRAM_ID {
        flags |= market_flags::BASE_TOKEN_2022;
    }
    if d.curve.is_holder_reward || d.pool.map(|p| p.is_holder_reward).unwrap_or(false) {
        flags |= market_flags::HOLDER_REWARD_COIN;
    }
    if d.curve.is_mayhem || d.pool.map(|p| p.is_mayhem).unwrap_or(false) {
        flags |= market_flags::MAYHEM;
    }
    m.flags = flags;
    let (venue, pool) = (m.venue, m.pool);
    drop(m);

    ctx.accounts.global.market_count = ctx.accounts.global.market_count.saturating_add(1);
    emit!(MarketCreated {
        market: ctx.accounts.market.key(),
        mint,
        authority,
        venue,
        creator_verified: verified,
        bonding_curve: ctx.accounts.bonding_curve.key(),
        pool,
        quote_mint: d.quote_mint,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct SyncMarket<'info> {
    #[account(mut)]
    pub market: AccountLoader<'info, MarketConfig>,
    /// CHECK: verified against the market.
    pub bonding_curve: UncheckedAccount<'info>,
    /// CHECK: verified against the market.
    pub quote_mint: UncheckedAccount<'info>,
    /// CHECK: canonical pool (verified).
    pub pool: UncheckedAccount<'info>,
}

/// Permissionless: switch a market to PumpSwap once its curve has completed and the canonical
/// pool exists.
pub fn sync_market(ctx: Context<SyncMarket>) -> Result<()> {
    let mut m = ctx.accounts.market.load_mut()?;
    require_keys_eq!(ctx.accounts.bonding_curve.key(), m.bonding_curve, E::InvalidBondingCurve);
    let d = detect(&m.mint, &ctx.accounts.bonding_curve, &ctx.accounts.quote_mint, &ctx.accounts.pool)?;
    require!(d.pool.is_some(), E::MarketNotMigrated);
    m.pool = ctx.accounts.pool.key();
    m.venue = Venue::PumpSwap as u8;
    m.flags &= !market_flags::MIGRATION_PENDING;
    emit!(MarketSynced { market: ctx.accounts.market.key(), venue: m.venue, pool: m.pool });
    Ok(())
}

#[derive(Accounts)]
pub struct MarketAdmin<'info> {
    pub authority: Signer<'info>,
    #[account(mut)]
    pub market: AccountLoader<'info, MarketConfig>,
}

pub fn check_market_authority(m: &MarketConfig, signer: &Pubkey) -> Result<()> {
    require_keys_eq!(m.authority, *signer, E::Unauthorized);
    Ok(())
}

pub fn set_market_paused(ctx: Context<MarketAdmin>, paused: bool) -> Result<()> {
    let mut m = ctx.accounts.market.load_mut()?;
    check_market_authority(&m, &ctx.accounts.authority.key())?;
    if paused {
        m.flags |= market_flags::PAUSED;
    } else {
        m.flags &= !market_flags::PAUSED;
    }
    emit!(MarketPausedSet { market: ctx.accounts.market.key(), paused });
    Ok(())
}

pub fn propose_market_authority(ctx: Context<MarketAdmin>, new_authority: Pubkey) -> Result<()> {
    let mut m = ctx.accounts.market.load_mut()?;
    check_market_authority(&m, &ctx.accounts.authority.key())?;
    m.pending_authority = new_authority;
    Ok(())
}

pub fn accept_market_authority(ctx: Context<MarketAdmin>) -> Result<()> {
    let mut m = ctx.accounts.market.load_mut()?;
    require_keys_eq!(m.pending_authority, ctx.accounts.authority.key(), E::NoPendingAuthority);
    m.authority = m.pending_authority;
    m.pending_authority = Pubkey::default();
    Ok(())
}
