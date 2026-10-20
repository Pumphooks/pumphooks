use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, MintTo, Token};
use hooks_interface::{math, Callback};

use crate::{
    engine::{self, apply_lp_delta, invoke, Book, SwapArgs},
    errors::CoreError,
    events::*,
    hook::{cpi_hooks, CpiHooks, HookAccounts},
    policy::{self, PoolOp},
    state::*,
    token as tok,
};

/// The pool and its token plumbing, validated against the pool's stored keys.
#[derive(Accounts)]
pub struct PoolIo<'info> {
    #[account(
        mut,
        has_one = vault_0 @ CoreError::PoolAccountMismatch,
        has_one = vault_1 @ CoreError::PoolAccountMismatch,
        has_one = mint_0 @ CoreError::PoolAccountMismatch,
        has_one = mint_1 @ CoreError::PoolAccountMismatch,
        has_one = token_program_0 @ CoreError::PoolAccountMismatch,
        has_one = token_program_1 @ CoreError::PoolAccountMismatch,
    )]
    pub pool: Box<Account<'info, Pool>>,
    /// CHECK: pool vault (has_one).
    #[account(mut)]
    pub vault_0: UncheckedAccount<'info>,
    /// CHECK: pool vault (has_one).
    #[account(mut)]
    pub vault_1: UncheckedAccount<'info>,
    /// CHECK: pool mint (has_one).
    pub mint_0: UncheckedAccount<'info>,
    /// CHECK: pool mint (has_one).
    pub mint_1: UncheckedAccount<'info>,
    /// CHECK: token program recorded at pool creation (has_one).
    pub token_program_0: UncheckedAccount<'info>,
    /// CHECK: token program recorded at pool creation (has_one).
    pub token_program_1: UncheckedAccount<'info>,
}

/// Hook accounts; required when the pool has a hook, omitted otherwise.
#[derive(Accounts)]
pub struct HookIo<'info> {
    /// CHECK: must equal `pool.hook_program`.
    pub hook_program: Option<UncheckedAccount<'info>>,
    /// CHECK: must be the callback-authority PDA for the hook.
    pub callback_authority: Option<UncheckedAccount<'info>>,
    /// CHECK: required for pinned pools; validated by `read_program`.
    pub hook_program_data: Option<UncheckedAccount<'info>>,
}

impl<'info> HookIo<'info> {
    pub fn accounts(&self) -> HookAccounts<'_, 'info> {
        HookAccounts {
            hook_program: self.hook_program.as_ref().map(|a| a.as_ref()),
            callback_authority: self.callback_authority.as_ref().map(|a| a.as_ref()),
            hook_program_data: self.hook_program_data.as_ref().map(|a| a.as_ref()),
        }
    }
}

impl<'info> PoolIo<'info> {
    pub fn vault(&self, i: usize) -> AccountInfo<'info> {
        if i == 0 { self.vault_0.to_account_info() } else { self.vault_1.to_account_info() }
    }
    pub fn mint(&self, i: usize) -> AccountInfo<'info> {
        if i == 0 { self.mint_0.to_account_info() } else { self.mint_1.to_account_info() }
    }
    pub fn token_program(&self, i: usize) -> AccountInfo<'info> {
        if i == 0 { self.token_program_0.to_account_info() } else { self.token_program_1.to_account_info() }
    }

    /// Pays `amount` of side `i` out of the vault, signed by the pool.
    pub fn pay_out(&self, i: usize, to: &AccountInfo<'info>, amount: u64) -> Result<()> {
        let p: &Pool = &self.pool;
        let seeds = p.signer_seeds();
        tok::transfer(
            &self.token_program(i),
            &self.vault(i),
            to,
            &self.pool.to_account_info(),
            &self.mint(i),
            amount,
            Some(&seeds),
        )
    }

    pub fn pull_in(&self, i: usize, from: &AccountInfo<'info>, authority: &AccountInfo<'info>, amount: u64) -> Result<u64> {
        tok::transfer_in(&self.token_program(i), from, &self.vault(i), authority, &self.mint(i), amount)
    }

    /// Commits `book`, bumps the sequence and checks every vault covers what the pool owes.
    pub fn commit(&mut self, book: &Book) -> Result<u64> {
        self.pool.store(book);
        self.pool.sequence += 1;
        for i in 0..2 {
            let owed = book.owed(i).ok_or(error!(CoreError::MathOverflow))?;
            require!(tok::amount(&self.vault(i))? >= owed, CoreError::VaultInvariant);
        }
        Ok(self.pool.sequence)
    }
}

fn hooks_for<'a, 'info>(
    snapshot: &'a Pool,
    pool_info: &'a AccountInfo<'info>,
    hooks: &'a HookIo<'info>,
    remaining: &'a [AccountInfo<'info>],
    hook_data: &'a [u8],
    caller: Pubkey,
) -> Result<Option<CpiHooks<'a, 'info>>> {
    cpi_hooks(snapshot, pool_info, hooks.accounts(), remaining, hook_data, caller)
}

// ---------------------------------------------------------------- swap

#[derive(Accounts)]
pub struct Swap<'info> {
    pub trader: Signer<'info>,
    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Box<Account<'info, Protocol>>,
    pub io: PoolIo<'info>,
    /// CHECK: trader's token account for mint 0; the token program enforces mint and authority.
    #[account(mut)]
    pub trader_token_0: UncheckedAccount<'info>,
    /// CHECK: trader's token account for mint 1.
    #[account(mut)]
    pub trader_token_1: UncheckedAccount<'info>,
    pub hooks: HookIo<'info>,
    /// Required when the pool has a policy (`pool.policy`).
    #[account(mut)]
    pub policy: Option<AccountLoader<'info, PoolPolicy>>,
}

pub fn swap<'info>(
    ctx: Context<'_, '_, 'info, 'info, Swap<'info>>,
    zero_for_one: bool,
    exact_in: bool,
    amount: u64,
    limit: u64,
    hook_data: Vec<u8>,
) -> Result<()> {
    let a = ctx.accounts;
    require!(a.protocol.paused & PAUSE_SWAPS == 0, CoreError::Paused);
    let (i, o) = if zero_for_one { (0, 1) } else { (1, 0) };
    let user = [a.trader_token_0.to_account_info(), a.trader_token_1.to_account_info()];
    let trader = a.trader.to_account_info();

    let snapshot: Pool = (*a.io.pool).clone().into_inner();
    let pool_info = a.io.pool.to_account_info();
    let mut hooks = hooks_for(&snapshot, &pool_info, &a.hooks, ctx.remaining_accounts, &hook_data, trader.key())?;
    let (max_in, min_out) = if exact_in { (amount, limit) } else { (limit, amount) };
    let mut op = PoolOp { actor: trader.key(), asset: a.io.mint(i).key(), amount: max_in, amount_b: min_out, sequence: snapshot.sequence };
    let policy = a.policy.as_ref();
    policy::run(pool_info.key(), snapshot.policy, policy, Callback::BeforeSwap as u8, &op, ctx.remaining_accounts)?;

    let amount_eff = if exact_in {
        a.io.pull_in(i, &user[i], &trader, amount)?
    } else {
        require!(!tok::transfer_fee_active(&a.io.mint(i))?, CoreError::ExactOutTransferFee);
        amount
    };
    let args = SwapArgs { zero_for_one, exact_in, amount: amount_eff, limit, protocol_fee_bps: a.protocol.protocol_fee_bps };
    let mut book = snapshot.book();
    let r = engine::swap(&mut book, &args, &mut hooks)?;
    drop(hooks);

    if !exact_in {
        let got = a.io.pull_in(i, &user[i], &trader, r.user_in)?;
        require!(got == r.user_in, CoreError::ExactOutTransferFee);
    }
    a.io.pay_out(o, &user[o], r.user_out)?;
    (op.amount, op.amount_b) = (r.user_in, r.user_out);
    policy::run(pool_info.key(), snapshot.policy, policy, Callback::AfterSwap as u8, &op, ctx.remaining_accounts)?;
    let sequence = a.io.commit(&book)?;
    emit!(Swapped {
        pool: pool_info.key(),
        trader: trader.key(),
        zero_for_one,
        exact_in,
        from_credit: false,
        amount_in: r.user_in,
        amount_out: r.user_out,
        curve_in: r.curve_in,
        curve_out: r.curve_out,
        fee: r.fee,
        protocol_fee: r.protocol_fee,
        fee_ppm: r.fee_ppm,
        delta_specified: r.delta_specified,
        delta_unspecified: r.delta_unspecified,
        reserve_0: book.reserve[0],
        reserve_1: book.reserve[1],
        sequence,
    });
    Ok(())
}

// ---------------------------------------------------------------- liquidity

#[derive(Accounts)]
pub struct Liquidity<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Box<Account<'info, Protocol>>,
    pub io: PoolIo<'info>,
    /// CHECK: LP mint of this pool.
    #[account(mut, address = io.pool.lp_mint @ CoreError::PoolAccountMismatch)]
    pub lp_mint: UncheckedAccount<'info>,
    /// CHECK: owner's token account for mint 0.
    #[account(mut)]
    pub owner_token_0: UncheckedAccount<'info>,
    /// CHECK: owner's token account for mint 1.
    #[account(mut)]
    pub owner_token_1: UncheckedAccount<'info>,
    /// CHECK: owner's LP token account; the token program enforces mint and authority.
    #[account(mut)]
    pub owner_lp: UncheckedAccount<'info>,
    pub lp_token_program: Program<'info, Token>,
    pub hooks: HookIo<'info>,
    /// Required when the pool has a policy (`pool.policy`).
    #[account(mut)]
    pub policy: Option<AccountLoader<'info, PoolPolicy>>,
}

pub fn add_liquidity<'info>(
    ctx: Context<'_, '_, 'info, 'info, Liquidity<'info>>,
    max_0: u64,
    max_1: u64,
    min_shares: u64,
    hook_data: Vec<u8>,
) -> Result<()> {
    let a = ctx.accounts;
    require!(a.protocol.paused & PAUSE_ADDS == 0, CoreError::Paused);
    require!(max_0 > 0 && max_1 > 0, CoreError::ZeroAmount);
    let user = [a.owner_token_0.to_account_info(), a.owner_token_1.to_account_info()];
    let owner = a.owner.to_account_info();
    let snapshot: Pool = (*a.io.pool).clone().into_inner();
    let pool_info = a.io.pool.to_account_info();
    let mut hooks = hooks_for(&snapshot, &pool_info, &a.hooks, ctx.remaining_accounts, &hook_data, owner.key())?;
    let mut book = snapshot.book();

    let mut op = PoolOp { actor: owner.key(), asset: snapshot.mint_0, amount: max_0, amount_b: max_1, sequence: snapshot.sequence };
    let policy = a.policy.as_ref();
    policy::run(pool_info.key(), snapshot.policy, policy, Callback::BeforeAddLiquidity as u8, &op, ctx.remaining_accounts)?;
    invoke(&mut hooks, Callback::BeforeAddLiquidity, &book, 0, [max_0, max_1, min_shares, 0, 0, 0, 0])?;

    let want = if book.lp_supply == 0 {
        [max_0, max_1]
    } else {
        let sh = math::shares_for_deposit(max_0, max_1, book.reserve[0], book.reserve[1], book.lp_supply)
            .ok_or(error!(CoreError::MathOverflow))?;
        let (w0, w1) = math::amounts_for_shares_up(sh, book.reserve[0], book.reserve[1], book.lp_supply)
            .ok_or(error!(CoreError::MathOverflow))?;
        [w0, w1]
    };
    require!(want[0] > 0 && want[1] > 0, CoreError::ZeroAmount);
    let got = [a.io.pull_in(0, &user[0], &owner, want[0])?, a.io.pull_in(1, &user[1], &owner, want[1])?];

    let shares = if book.lp_supply == 0 {
        let (total, user_shares) =
            math::initial_shares(got[0], got[1]).ok_or(error!(CoreError::InsufficientLiquidity))?;
        book.lp_supply = total;
        user_shares
    } else {
        let s = math::shares_for_deposit(got[0], got[1], book.reserve[0], book.reserve[1], book.lp_supply)
            .ok_or(error!(CoreError::MathOverflow))?;
        book.lp_supply = book.lp_supply.checked_add(s).ok_or(error!(CoreError::MathOverflow))?;
        s
    };
    require!(shares > 0, CoreError::InsufficientLiquidity);
    for (k, g) in got.iter().enumerate() {
        book.reserve[k] = book.reserve[k].checked_add(*g).ok_or(error!(CoreError::MathOverflow))?;
    }

    let d = invoke(&mut hooks, Callback::AfterAddLiquidity, &book, 0, [got[0], got[1], shares, 0, 0, 0, 0])?
        .map(|r| [r.delta_0, r.delta_1])
        .unwrap_or([0, 0]);
    drop(hooks);

    let max = [max_0, max_1];
    for k in 0..2 {
        // Positive: the hook charges the LP extra into its credit. Negative: a rebate from credit.
        if d[k] > 0 {
            let extra = a.io.pull_in(k, &user[k], &owner, d[k] as u64)?;
            book.credit[k] = book.credit[k].checked_add(extra).ok_or(error!(CoreError::MathOverflow))?;
        } else if d[k] < 0 {
            let rebate = d[k].unsigned_abs();
            book.credit[k] = book.credit[k].checked_sub(rebate).ok_or(error!(CoreError::InsufficientCredit))?;
            a.io.pay_out(k, &user[k], rebate)?;
        }
        require!(want[k] as i128 + d[k] as i128 <= max[k] as i128, CoreError::Slippage);
    }
    require!(shares >= min_shares, CoreError::Slippage);
    (op.amount, op.amount_b) = (got[0], got[1]);
    policy::run(pool_info.key(), snapshot.policy, policy, Callback::AfterAddLiquidity as u8, &op, ctx.remaining_accounts)?;

    {
        let p: &Pool = &a.io.pool;
        let seeds = p.signer_seeds();
        token::mint_to(
            CpiContext::new_with_signer(
                a.lp_token_program.to_account_info(),
                MintTo {
                    mint: a.lp_mint.to_account_info(),
                    to: a.owner_lp.to_account_info(),
                    authority: a.io.pool.to_account_info(),
                },
                &[&seeds],
            ),
            shares,
        )?;
    }
    let sequence = a.io.commit(&book)?;
    emit!(LiquidityChanged {
        pool: pool_info.key(),
        owner: owner.key(),
        added: true,
        amount_0: got[0],
        amount_1: got[1],
        shares,
        hook_delta_0: d[0],
        hook_delta_1: d[1],
        reserve_0: book.reserve[0],
        reserve_1: book.reserve[1],
        lp_supply: book.lp_supply,
        sequence,
    });
    Ok(())
}

/// Never blocked by the protocol pause. A hook with `BEFORE_REMOVE_LIQUIDITY` can block it.
pub fn remove_liquidity<'info>(
    ctx: Context<'_, '_, 'info, 'info, Liquidity<'info>>,
    shares: u64,
    min_0: u64,
    min_1: u64,
    hook_data: Vec<u8>,
) -> Result<()> {
    let a = ctx.accounts;
    require!(shares > 0, CoreError::ZeroAmount);
    let user = [a.owner_token_0.to_account_info(), a.owner_token_1.to_account_info()];
    let owner = a.owner.to_account_info();
    let snapshot: Pool = (*a.io.pool).clone().into_inner();
    let pool_info = a.io.pool.to_account_info();
    let mut hooks = hooks_for(&snapshot, &pool_info, &a.hooks, ctx.remaining_accounts, &hook_data, owner.key())?;
    let mut book = snapshot.book();

    invoke(&mut hooks, Callback::BeforeRemoveLiquidity, &book, 0, [shares, min_0, min_1, 0, 0, 0, 0])?;

    let (a0, a1) = math::amounts_for_shares_down(shares, book.reserve[0], book.reserve[1], book.lp_supply)
        .ok_or(error!(CoreError::InsufficientLiquidity))?;
    token::burn(
        CpiContext::new(
            a.lp_token_program.to_account_info(),
            Burn { mint: a.lp_mint.to_account_info(), from: a.owner_lp.to_account_info(), authority: owner.clone() },
        ),
        shares,
    )?;
    book.reserve[0] -= a0;
    book.reserve[1] -= a1;
    book.lp_supply -= shares;

    let d = invoke(&mut hooks, Callback::AfterRemoveLiquidity, &book, 0, [a0, a1, shares, 0, 0, 0, 0])?
        .map(|r| [r.delta_0, r.delta_1])
        .unwrap_or([0, 0]);
    drop(hooks);

    // Positive: withheld from the LP into the hook's credit. Negative: a bonus from credit.
    let out = [
        apply_lp_delta(&mut book, 0, a0, d[0], false)?,
        apply_lp_delta(&mut book, 1, a1, d[1], false)?,
    ];
    require!(out[0] >= min_0 && out[1] >= min_1, CoreError::Slippage);
    a.io.pay_out(0, &user[0], out[0])?;
    a.io.pay_out(1, &user[1], out[1])?;
    // Withdrawals never depend on the policy: accounting runs only if the LP supplies it.
    if a.policy.is_some() {
        let op = PoolOp { actor: owner.key(), asset: snapshot.mint_0, amount: out[0], amount_b: out[1], sequence: snapshot.sequence };
        policy::run(pool_info.key(), snapshot.policy, a.policy.as_ref(), Callback::AfterRemoveLiquidity as u8, &op, ctx.remaining_accounts)?;
    }
    let sequence = a.io.commit(&book)?;
    emit!(LiquidityChanged {
        pool: pool_info.key(),
        owner: owner.key(),
        added: false,
        amount_0: out[0],
        amount_1: out[1],
        shares,
        hook_delta_0: d[0],
        hook_delta_1: d[1],
        reserve_0: book.reserve[0],
        reserve_1: book.reserve[1],
        lp_supply: book.lp_supply,
        sequence,
    });
    Ok(())
}

// ---------------------------------------------------------------- donate

#[derive(Accounts)]
pub struct Donate<'info> {
    pub donor: Signer<'info>,
    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Box<Account<'info, Protocol>>,
    pub io: PoolIo<'info>,
    /// CHECK: donor's token account for mint 0.
    #[account(mut)]
    pub donor_token_0: UncheckedAccount<'info>,
    /// CHECK: donor's token account for mint 1.
    #[account(mut)]
    pub donor_token_1: UncheckedAccount<'info>,
    pub hooks: HookIo<'info>,
    /// Required when the pool has a policy (`pool.policy`).
    #[account(mut)]
    pub policy: Option<AccountLoader<'info, PoolPolicy>>,
}

/// Adds tokens to the reserves without minting shares, i.e. pays current LPs.
pub fn donate<'info>(
    ctx: Context<'_, '_, 'info, 'info, Donate<'info>>,
    amount_0: u64,
    amount_1: u64,
    hook_data: Vec<u8>,
) -> Result<()> {
    let a = ctx.accounts;
    require!(a.protocol.paused & PAUSE_ADDS == 0, CoreError::Paused);
    require!(amount_0 > 0 || amount_1 > 0, CoreError::ZeroAmount);
    let user = [a.donor_token_0.to_account_info(), a.donor_token_1.to_account_info()];
    let donor = a.donor.to_account_info();
    let snapshot: Pool = (*a.io.pool).clone().into_inner();
    require!(snapshot.lp_supply > 0, CoreError::InsufficientLiquidity);
    let pool_info = a.io.pool.to_account_info();
    let mut hooks = hooks_for(&snapshot, &pool_info, &a.hooks, ctx.remaining_accounts, &hook_data, donor.key())?;
    let mut book = snapshot.book();

    let mut op = PoolOp { actor: donor.key(), asset: snapshot.mint_0, amount: amount_0, amount_b: amount_1, sequence: snapshot.sequence };
    let policy = a.policy.as_ref();
    policy::run(pool_info.key(), snapshot.policy, policy, Callback::BeforeDonate as u8, &op, ctx.remaining_accounts)?;
    invoke(&mut hooks, Callback::BeforeDonate, &book, 0, [amount_0, amount_1, 0, 0, 0, 0, 0])?;
    let got = [a.io.pull_in(0, &user[0], &donor, amount_0)?, a.io.pull_in(1, &user[1], &donor, amount_1)?];
    for k in 0..2 {
        book.reserve[k] = book.reserve[k].checked_add(got[k]).ok_or(error!(CoreError::MathOverflow))?;
    }
    invoke(&mut hooks, Callback::AfterDonate, &book, 0, [got[0], got[1], 0, 0, 0, 0, 0])?;
    drop(hooks);
    (op.amount, op.amount_b) = (got[0], got[1]);
    policy::run(pool_info.key(), snapshot.policy, policy, Callback::AfterDonate as u8, &op, ctx.remaining_accounts)?;
    let sequence = a.io.commit(&book)?;
    emit!(Donated { pool: pool_info.key(), donor: donor.key(), from_credit: false, amount_0: got[0], amount_1: got[1], sequence });
    Ok(())
}
