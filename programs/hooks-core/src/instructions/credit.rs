//! Hook credit and hook-initiated operations (`docs/engines/pool-manager.md` §4.5), and protocol fees.

use anchor_lang::prelude::*;
use hooks_interface::{perm, Callback, HookResult};

use crate::{
    engine::{self, Book, Hooks, SwapArgs},
    errors::CoreError,
    events::*,
    hook::require_hook_authority,
    instructions::trade::*,
    state::*,
};

#[derive(Accounts)]
pub struct CreditDeposit<'info> {
    pub depositor: Signer<'info>,
    pub io: PoolIo<'info>,
    /// CHECK: depositor's token account for mint 0.
    #[account(mut)]
    pub depositor_token_0: UncheckedAccount<'info>,
    /// CHECK: depositor's token account for mint 1.
    #[account(mut)]
    pub depositor_token_1: UncheckedAccount<'info>,
}

/// Anyone may fund a hook's credit in a pool (e.g. a solver or the hook's operator).
pub fn credit_deposit(ctx: Context<CreditDeposit>, amount_0: u64, amount_1: u64) -> Result<()> {
    let a = ctx.accounts;
    require!(a.io.pool.has_hook(), CoreError::Unauthorized);
    let from = [a.depositor_token_0.to_account_info(), a.depositor_token_1.to_account_info()];
    let auth = a.depositor.to_account_info();
    let mut book = a.io.pool.book();
    let got = [a.io.pull_in(0, &from[0], &auth, amount_0)?, a.io.pull_in(1, &from[1], &auth, amount_1)?];
    for k in 0..2 {
        book.credit[k] = book.credit[k].checked_add(got[k]).ok_or(error!(CoreError::MathOverflow))?;
    }
    a.io.commit(&book)?;
    emit_credit(&a.io, auth.key(), true, got);
    Ok(())
}

#[derive(Accounts)]
pub struct HookOp<'info> {
    /// The hook's `["hook-authority"]` PDA, signed by the hook program.
    pub hook_authority: Signer<'info>,
    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Box<Account<'info, Protocol>>,
    pub io: PoolIo<'info>,
    /// CHECK: destination for credit withdrawals of mint 0 (unused by other operations).
    #[account(mut)]
    pub destination_0: Option<UncheckedAccount<'info>>,
    /// CHECK: destination for credit withdrawals of mint 1.
    #[account(mut)]
    pub destination_1: Option<UncheckedAccount<'info>>,
}

pub fn credit_withdraw(ctx: Context<HookOp>, amount_0: u64, amount_1: u64) -> Result<()> {
    let a = ctx.accounts;
    require_hook_authority(&a.io.pool, a.hook_authority.key)?;
    let mut book = a.io.pool.book();
    for (k, amt, dest) in [(0, amount_0, &a.destination_0), (1, amount_1, &a.destination_1)] {
        if amt == 0 {
            continue;
        }
        let dest = dest.as_ref().ok_or(error!(CoreError::PoolAccountMismatch))?;
        book.credit[k] = book.credit[k].checked_sub(amt).ok_or(error!(CoreError::InsufficientCredit))?;
        a.io.pay_out(k, &dest.to_account_info(), amt)?;
    }
    a.io.commit(&book)?;
    emit_credit(&a.io, a.hook_authority.key(), false, [amount_0, amount_1]);
    Ok(())
}

/// Hook-initiated swaps cannot call back into the hook (that would be re-entrancy), so on a
/// custom-curve pool the hook supplies the curve amount itself; it is the pricing authority.
struct SelfQuote(u64);
impl Hooks for SelfQuote {
    fn call(&mut self, cb: Callback, _: &Book, _: u8, _: [u64; 7]) -> Result<HookResult> {
        Ok(if cb == Callback::Quote { HookResult::ok().with_quote(self.0) } else { HookResult::ok() })
    }
}

/// Swaps the hook's own credit against the pool. No callbacks run (v4's `noSelfCall`). This is
/// how TWAMM, limit orders, batch clearing and rebalancing execute.
pub fn credit_swap(
    ctx: Context<HookOp>,
    zero_for_one: bool,
    exact_in: bool,
    amount: u64,
    limit: u64,
    quote: u64,
) -> Result<()> {
    let a = ctx.accounts;
    require!(a.protocol.paused & PAUSE_SWAPS == 0, CoreError::Paused);
    require_hook_authority(&a.io.pool, a.hook_authority.key)?;
    let (i, o) = if zero_for_one { (0, 1) } else { (1, 0) };
    let mut book = a.io.pool.book();
    book.permissions &= perm::CUSTOM_CURVE;
    let args = SwapArgs { zero_for_one, exact_in, amount, limit, protocol_fee_bps: a.protocol.protocol_fee_bps, policy_fee_ppm: None };
    let r = engine::swap(&mut book, &args, &mut SelfQuote(quote))?;
    book.credit[i] = book.credit[i].checked_sub(r.user_in).ok_or(error!(CoreError::InsufficientCredit))?;
    book.credit[o] = book.credit[o].checked_add(r.user_out).ok_or(error!(CoreError::MathOverflow))?;
    book.permissions = a.io.pool.permissions;
    let sequence = a.io.commit(&book)?;
    emit!(Swapped {
        pool: a.io.pool.key(),
        trader: a.hook_authority.key(),
        zero_for_one,
        exact_in,
        from_credit: true,
        amount_in: r.user_in,
        amount_out: r.user_out,
        curve_in: r.curve_in,
        curve_out: r.curve_out,
        fee: r.fee,
        protocol_fee: r.protocol_fee,
        fee_ppm: r.fee_ppm,
        delta_specified: 0,
        delta_unspecified: 0,
        reserve_0: book.reserve[0],
        reserve_1: book.reserve[1],
        sequence,
    });
    Ok(())
}

/// Moves hook credit into the reserves, i.e. pays the pool's LPs (e.g. JIT penalties).
pub fn credit_donate(ctx: Context<HookOp>, amount_0: u64, amount_1: u64) -> Result<()> {
    let a = ctx.accounts;
    require_hook_authority(&a.io.pool, a.hook_authority.key)?;
    let mut book = a.io.pool.book();
    require!(book.lp_supply > 0, CoreError::InsufficientLiquidity);
    for (k, amt) in [(0, amount_0), (1, amount_1)] {
        book.credit[k] = book.credit[k].checked_sub(amt).ok_or(error!(CoreError::InsufficientCredit))?;
        book.reserve[k] = book.reserve[k].checked_add(amt).ok_or(error!(CoreError::MathOverflow))?;
    }
    let sequence = a.io.commit(&book)?;
    emit!(Donated {
        pool: a.io.pool.key(),
        donor: a.hook_authority.key(),
        from_credit: true,
        amount_0,
        amount_1,
        sequence,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct SetFee<'info> {
    pub hook_authority: Signer<'info>,
    #[account(mut)]
    pub pool: Box<Account<'info, Pool>>,
}

pub fn set_fee(ctx: Context<SetFee>, fee_ppm: u32) -> Result<()> {
    let pool = &mut ctx.accounts.pool;
    require_hook_authority(pool, ctx.accounts.hook_authority.key)?;
    require!(pool.permissions & perm::DYNAMIC_FEE != 0, CoreError::HookResultNotPermitted);
    require!(fee_ppm <= pool.max_fee_ppm, CoreError::FeeTooHigh);
    pool.fee_ppm = fee_ppm;
    emit!(FeeChanged { pool: pool.key(), fee_ppm });
    Ok(())
}

fn emit_credit(io: &PoolIo, actor: Pubkey, deposit: bool, amounts: [u64; 2]) {
    emit!(CreditMoved {
        pool: io.pool.key(),
        actor,
        deposit,
        amount_0: amounts[0],
        amount_1: amounts[1],
        credit_0: io.pool.credit_0,
        credit_1: io.pool.credit_1,
    });
}

// ---------------------------------------------------------------- protocol fees

#[derive(Accounts)]
pub struct WithdrawProtocolFees<'info> {
    pub fee_authority: Signer<'info>,
    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump, has_one = fee_authority @ CoreError::Unauthorized)]
    pub protocol: Box<Account<'info, Protocol>>,
    pub io: PoolIo<'info>,
    /// CHECK: destination token account for mint 0.
    #[account(mut)]
    pub destination_0: UncheckedAccount<'info>,
    /// CHECK: destination token account for mint 1.
    #[account(mut)]
    pub destination_1: UncheckedAccount<'info>,
}

pub fn withdraw_protocol_fees(ctx: Context<WithdrawProtocolFees>) -> Result<()> {
    let a = ctx.accounts;
    let mut book = a.io.pool.book();
    let fees = book.protocol_fee;
    book.protocol_fee = [0, 0];
    a.io.pay_out(0, &a.destination_0.to_account_info(), fees[0])?;
    a.io.pay_out(1, &a.destination_1.to_account_info(), fees[1])?;
    a.io.commit(&book)?;
    emit!(ProtocolFeesWithdrawn { pool: a.io.pool.key(), amount_0: fees[0], amount_1: fees[1] });
    Ok(())
}
