use anchor_lang::prelude::*;
use anchor_spl::token::Token;
use anchor_spl::token_interface::{Mint, TokenInterface};
use hooks_interface::{perm, Callback, CALLBACK_SEED, MAX_FEE_PPM};

use crate::{
    engine::invoke,
    errors::CoreError,
    events::{PolicyAttached, PoolInitialized},
    hook::{cpi_hooks, read_program, HookAccounts},
    policy,
    state::*,
    token,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct InitPoolParams {
    /// `Pubkey::default()` for a plain pool.
    pub hook_program: Pubkey,
    pub permissions: u32,
    pub fee_ppm: u32,
    pub max_fee_ppm: u32,
    pub salt: u64,
    /// Freeze the hook at the deploy slot its registry version observed; every later operation
    /// fails if it is redeployed.
    pub pin_hook: bool,
}

#[derive(Accounts)]
#[instruction(params: InitPoolParams)]
pub struct InitializePool<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(mut, seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Box<Account<'info, Protocol>>,
    pub mint_0: Box<InterfaceAccount<'info, Mint>>,
    pub mint_1: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        init,
        payer = creator,
        space = 8 + Pool::INIT_SPACE,
        seeds = [
            POOL_SEED,
            mint_0.key().as_ref(),
            mint_1.key().as_ref(),
            params.hook_program.as_ref(),
            &params.salt.to_le_bytes(),
        ],
        bump
    )]
    pub pool: Box<Account<'info, Pool>>,
    /// CHECK: created here as a PDA token account.
    #[account(mut, seeds = [VAULT_SEED, pool.key().as_ref(), mint_0.key().as_ref()], bump)]
    pub vault_0: UncheckedAccount<'info>,
    /// CHECK: created here as a PDA token account.
    #[account(mut, seeds = [VAULT_SEED, pool.key().as_ref(), mint_1.key().as_ref()], bump)]
    pub vault_1: UncheckedAccount<'info>,
    #[account(
        init,
        payer = creator,
        seeds = [LP_SEED, pool.key().as_ref()],
        bump,
        mint::decimals = LP_DECIMALS,
        mint::authority = pool,
        mint::token_program = lp_token_program,
    )]
    pub lp_mint: Box<InterfaceAccount<'info, Mint>>,
    /// `hook-registry` version the hook is installed from (owner-checked by `Account`).
    pub hook_version: Option<Box<Account<'info, hook_registry::HookVersion>>>,
    /// CHECK: must equal `params.hook_program`.
    pub hook_program: Option<UncheckedAccount<'info>>,
    /// CHECK: seeds checked in the handler.
    pub callback_authority: Option<UncheckedAccount<'info>>,
    /// CHECK: validated by `read_program` when pinning.
    pub hook_program_data: Option<UncheckedAccount<'info>>,
    pub token_program_0: Interface<'info, TokenInterface>,
    pub token_program_1: Interface<'info, TokenInterface>,
    pub lp_token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn initialize_pool<'info>(
    ctx: Context<'_, '_, 'info, 'info, InitializePool<'info>>,
    params: InitPoolParams,
    hook_data: Vec<u8>,
) -> Result<()> {
    let a = &ctx.accounts;
    require!(a.protocol.paused & PAUSE_POOLS == 0, CoreError::Paused);
    require!(a.mint_0.key() < a.mint_1.key(), CoreError::MintOrder);
    require_keys_eq!(*a.mint_0.to_account_info().owner, a.token_program_0.key(), CoreError::PoolAccountMismatch);
    require_keys_eq!(*a.mint_1.to_account_info().owner, a.token_program_1.key(), CoreError::PoolAccountMismatch);
    token::check_mint(&a.mint_0.to_account_info())?;
    token::check_mint(&a.mint_1.to_account_info())?;
    require!(
        params.fee_ppm <= params.max_fee_ppm && params.max_fee_ppm <= MAX_FEE_PPM,
        CoreError::FeeTooHigh
    );
    require!(perm::is_valid(params.permissions), CoreError::InvalidPermissions);

    let has_hook = params.hook_program != Pubkey::default();
    let (mut callback_bump, mut flags, mut pinned_slot) = (0u8, 0u8, 0u64);
    if has_hook {
        let version = a.hook_version.as_ref().ok_or(error!(CoreError::HookVersionMismatch))?;
        policy::check_hook_version(version, &params.hook_program, params.permissions)?;
        let program = a.hook_program.as_ref().ok_or(error!(CoreError::HookAccountMismatch))?;
        require_keys_eq!(program.key(), params.hook_program, CoreError::HookAccountMismatch);
        let (cb, bump) =
            Pubkey::find_program_address(&[CALLBACK_SEED, params.hook_program.as_ref()], &crate::ID);
        let given = a.callback_authority.as_ref().ok_or(error!(CoreError::HookAccountMismatch))?;
        require_keys_eq!(given.key(), cb, CoreError::HookAccountMismatch);
        callback_bump = bump;
        if params.pin_hook {
            let info = read_program(program, a.hook_program_data.as_ref().map(|p| p.as_ref()))?;
            require!(info.deploy_slot == version.observed_deploy_slot, CoreError::HookRedeployed);
            flags |= POOL_HOOK_PINNED;
            pinned_slot = info.deploy_slot;
        }
    } else {
        require!(params.permissions == 0 && !params.pin_hook, CoreError::InvalidPermissions);
    }

    let pool_key = a.pool.key();
    let (m0, m1) = (a.mint_0.key(), a.mint_1.key());
    for (vault, mint, tp, bump, mk) in [
        (&a.vault_0, &a.mint_0, &a.token_program_0, ctx.bumps.vault_0, m0),
        (&a.vault_1, &a.mint_1, &a.token_program_1, ctx.bumps.vault_1, m1),
    ] {
        token::create_vault(
            &a.creator.to_account_info(),
            &vault.to_account_info(),
            &mint.to_account_info(),
            &pool_key,
            &tp.to_account_info(),
            &a.system_program.to_account_info(),
            &[VAULT_SEED, pool_key.as_ref(), mk.as_ref(), &[bump]],
        )?;
    }

    let now = Clock::get()?.unix_timestamp;
    let pool = &mut ctx.accounts.pool;
    pool.bump = ctx.bumps.pool;
    pool.callback_bump = callback_bump;
    pool.flags = flags;
    pool.permissions = params.permissions;
    pool.fee_ppm = params.fee_ppm;
    pool.max_fee_ppm = params.max_fee_ppm;
    pool.salt = params.salt;
    pool.mint_0 = m0;
    pool.mint_1 = m1;
    pool.vault_0 = ctx.accounts.vault_0.key();
    pool.vault_1 = ctx.accounts.vault_1.key();
    pool.token_program_0 = ctx.accounts.token_program_0.key();
    pool.token_program_1 = ctx.accounts.token_program_1.key();
    pool.lp_mint = ctx.accounts.lp_mint.key();
    pool.hook_program = params.hook_program;
    pool.hook_version = ctx.accounts.hook_version.as_ref().filter(|_| has_hook).map(|v| v.key()).unwrap_or_default();
    pool.policy = Pubkey::default();
    pool.creator = ctx.accounts.creator.key();
    pool.pinned_deploy_slot = pinned_slot;
    pool.created_at = now;

    let snapshot = (**pool).clone();
    let pool_info = pool.to_account_info();
    let accts = HookAccounts {
        hook_program: ctx.accounts.hook_program.as_ref().map(|a| a.as_ref()),
        callback_authority: ctx.accounts.callback_authority.as_ref().map(|a| a.as_ref()),
        hook_program_data: ctx.accounts.hook_program_data.as_ref().map(|a| a.as_ref()),
    };
    let caller = ctx.accounts.creator.key();
    let mut hooks = cpi_hooks(&snapshot, &pool_info, accts, ctx.remaining_accounts, &hook_data, caller)?;
    let book = snapshot.book();
    let args = [params.fee_ppm as u64, params.max_fee_ppm as u64, params.salt, 0, 0, 0, 0];
    if let Some(h) = hooks.as_mut() {
        invoke(h, Callback::BeforeInitialize, &book, 0, args)?;
        invoke(h, Callback::AfterInitialize, &book, 0, args)?;
    }

    ctx.accounts.protocol.pool_count += 1;
    emit!(PoolInitialized {
        pool: pool_key,
        creator: caller,
        mint_0: m0,
        mint_1: m1,
        hook_program: params.hook_program,
        hook_version: ctx.accounts.pool.hook_version,
        permissions: params.permissions,
        fee_ppm: params.fee_ppm,
        max_fee_ppm: params.max_fee_ppm,
        salt: params.salt,
        pinned: flags & POOL_HOOK_PINNED != 0,
    });
    Ok(())
}

// ---------------------------------------------------------------- policy

#[derive(Accounts)]
pub struct AttachPolicy<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(mut, has_one = creator @ CoreError::PolicyLocked)]
    pub pool: Box<Account<'info, Pool>>,
    #[account(
        init,
        payer = creator,
        space = 8 + core::mem::size_of::<PoolPolicy>(),
        seeds = [POLICY_SEED, pool.key().as_ref()],
        bump
    )]
    pub policy: AccountLoader<'info, PoolPolicy>,
    pub system_program: Program<'info, System>,
}

/// Attaches a builtin policy stack (compact ABI v1 form; remaining accounts are the registry
/// versions of its entries). Only before the first deposit, so every LP sees it; immutable after.
pub fn attach_policy<'info>(ctx: Context<'_, '_, 'info, 'info, AttachPolicy<'info>>, stack: Vec<u8>) -> Result<()> {
    let pool = &mut ctx.accounts.pool;
    require!(pool.lp_supply == 0 && pool.policy == Pubkey::default(), CoreError::PolicyLocked);
    let mut s = policy::parse_stack(&stack)?;
    policy::check_install(&mut s, ctx.remaining_accounts)?;
    let mut p = ctx.accounts.policy.load_init()?;
    p.pool = pool.key();
    p.bump = ctx.bumps.policy;
    bytemuck::bytes_of_mut(&mut p.stack).copy_from_slice(bytemuck::bytes_of(&*s));
    pool.policy = ctx.accounts.policy.key();
    emit!(PolicyAttached { pool: pool.key(), policy: pool.policy, entries: s.len });
    Ok(())
}
