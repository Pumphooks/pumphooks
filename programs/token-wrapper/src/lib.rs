//! PUMP HOOKS token wrapper.
//!
//! Existing SPL mints cannot gain a transfer hook. This program issues a *new* Token-2022 mint
//! (for example BONK -> phBONK) whose `TransferHook` names `token-hooks`, backed 1:1 by the
//! original asset held in a program-owned vault:
//!
//! - `wrap` moves the underlying into the vault and mints exactly the measured vault increase.
//! - `unwrap` burns first, then releases the same amount from the vault.
//! - Both run the mint's `before_wrap` / `before_unwrap` hook stack first.
//! - After every operation the vault balance must cover the wrapped supply.
//! - There is no admin, withdraw or upgrade path for the vault in this program; the wrapped mint
//!   has no freeze authority, and only this program's PDA can mint.
//!
//! The wrapped token is a different asset: it does not inherit the original's liquidity, listings
//! or price feeds. An underlying freeze authority (e.g. USDC) can still freeze the vault; it is
//! recorded on the wrapper so clients can show it.

#![allow(unexpected_cfgs, deprecated)]

use anchor_lang::{
    prelude::*,
    solana_program::{program::invoke_signed, system_instruction},
};
use anchor_spl::{
    token_2022::spl_token_2022::{
        self,
        extension::{BaseStateWithExtensions, ExtensionType, StateWithExtensions},
        state::{Account as TokenAccountState, Mint as MintState},
    },
    token_interface::{self, BurnChecked, MintTo, TransferChecked},
};

declare_id!("841vHKyD8HXQKsNSp2A2pzNjDHuFDBu9JPt6xsJc7vbh");

pub const WRAPPER_SEED: &[u8] = b"wrapper";
pub const MINT_SEED: &[u8] = b"wrapped-mint";
pub const VAULT_SEED: &[u8] = b"vault";

pub const SPL_TOKEN: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

/// Underlying Token-2022 extensions the vault can hold safely. Anything else (permanent
/// delegate, transfer hook, non-transferable, confidential transfers, default-frozen accounts,
/// pausable, scaled UI amount, extensions added after this list) is refused.
pub const ALLOWED_UNDERLYING_EXTENSIONS: &[ExtensionType] = &[
    ExtensionType::TransferFeeConfig,
    ExtensionType::MintCloseAuthority,
    ExtensionType::InterestBearingConfig,
    ExtensionType::MetadataPointer,
    ExtensionType::TokenMetadata,
    ExtensionType::GroupPointer,
    ExtensionType::TokenGroup,
    ExtensionType::GroupMemberPointer,
    ExtensionType::TokenGroupMember,
];

#[program]
pub mod token_wrapper {
    use super::*;

    /// Creates the wrapped mint (Token-2022 + transfer hook -> `token-hooks`, mint authority =
    /// the wrapper PDA, no freeze authority) and the vault. `hook_authority` controls the
    /// wrapped mint's token policy; install it with `token_hooks::create_policy` in the same
    /// transaction, since wrapped tokens cannot move until a policy exists.
    pub fn create_wrapper(ctx: Context<CreateWrapper>, hook_authority: Pubkey) -> Result<()> {
        let underlying_program = *ctx.accounts.underlying_mint.owner;
        require_keys_eq!(underlying_program, ctx.accounts.underlying_program.key(), WrapperError::WrongTokenProgram);
        let (decimals, freeze_authority, account_len) = inspect_underlying(&ctx.accounts.underlying_mint)?;

        let underlying = ctx.accounts.underlying_mint.key();
        let wrapper_key = ctx.accounts.wrapper.key();
        let rent = Rent::get()?;
        let payer = ctx.accounts.payer.to_account_info();
        let system = ctx.accounts.system_program.to_account_info();

        let mint_len = ExtensionType::try_calculate_account_len::<MintState>(&[ExtensionType::TransferHook])?;
        let mint_info = ctx.accounts.wrapped_mint.to_account_info();
        let mint_seeds: &[&[u8]] = &[MINT_SEED, underlying.as_ref(), &[ctx.bumps.wrapped_mint]];
        invoke_signed(
            &system_instruction::create_account(payer.key, mint_info.key, rent.minimum_balance(mint_len), mint_len as u64, &spl_token_2022::ID),
            &[payer.clone(), mint_info.clone(), system.clone()],
            &[mint_seeds],
        )?;
        let t22 = ctx.accounts.token_2022_program.to_account_info();
        invoke_signed(
            &spl_token_2022::extension::transfer_hook::instruction::initialize(&spl_token_2022::ID, mint_info.key, Some(hook_authority), Some(token_hooks::ID))?,
            &[mint_info.clone(), t22.clone()],
            &[],
        )?;
        invoke_signed(
            &spl_token_2022::instruction::initialize_mint2(&spl_token_2022::ID, mint_info.key, &wrapper_key, None, decimals)?,
            &[mint_info.clone(), t22],
            &[],
        )?;

        let vault_info = ctx.accounts.vault.to_account_info();
        let vault_seeds: &[&[u8]] = &[VAULT_SEED, underlying.as_ref(), &[ctx.bumps.vault]];
        invoke_signed(
            &system_instruction::create_account(payer.key, vault_info.key, rent.minimum_balance(account_len), account_len as u64, &underlying_program),
            &[payer.clone(), vault_info.clone(), system],
            &[vault_seeds],
        )?;
        invoke_signed(
            &spl_token_2022::instruction::initialize_account3(&underlying_program, vault_info.key, &underlying, &wrapper_key)?,
            &[vault_info.clone(), ctx.accounts.underlying_mint.to_account_info(), ctx.accounts.underlying_program.to_account_info()],
            &[],
        )?;

        let w = &mut ctx.accounts.wrapper;
        w.underlying_mint = underlying;
        w.underlying_program = underlying_program;
        w.wrapped_mint = mint_info.key();
        w.vault = vault_info.key();
        w.decimals = decimals;
        w.underlying_freeze_authority = freeze_authority.unwrap_or_default();
        w.bump = ctx.bumps.wrapper;
        emit!(WrapperCreated { wrapper: wrapper_key, underlying_mint: underlying, wrapped_mint: w.wrapped_mint, vault: w.vault });
        Ok(())
    }

    /// Deposits `amount` of the underlying and mints the vault's measured increase (lower than
    /// `amount` for transfer-fee assets) to `user_wrapped`. Remaining accounts are forwarded to
    /// the hook stack (e.g. gateway tokens).
    pub fn wrap<'info>(ctx: Context<'_, '_, 'info, 'info, Operate<'info>>, amount: u64, min_out: u64) -> Result<()> {
        require!(amount > 0, WrapperError::ZeroAmount);
        let recipient = token_owner(&ctx.accounts.user_wrapped)?;
        check(&ctx, token_hooks::callback::BEFORE_WRAP, recipient, amount)?;

        let before = token_amount(&ctx.accounts.vault)?;
        token_interface::transfer_checked(
            CpiContext::new(
                ctx.accounts.underlying_program.to_account_info(),
                TransferChecked {
                    from: ctx.accounts.user_underlying.to_account_info(),
                    mint: ctx.accounts.underlying_mint.to_account_info(),
                    to: ctx.accounts.vault.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            amount,
            ctx.accounts.wrapper.decimals,
        )?;
        let received = token_amount(&ctx.accounts.vault)?.checked_sub(before).ok_or(error!(WrapperError::ReserveShortfall))?;
        require!(received > 0 && received >= min_out, WrapperError::SlippageExceeded);

        let underlying = ctx.accounts.wrapper.underlying_mint;
        let seeds: &[&[u8]] = &[WRAPPER_SEED, underlying.as_ref(), &[ctx.accounts.wrapper.bump]];
        token_interface::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_2022_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.wrapped_mint.to_account_info(),
                    to: ctx.accounts.user_wrapped.to_account_info(),
                    authority: ctx.accounts.wrapper.to_account_info(),
                },
                &[seeds],
            ),
            received,
        )?;
        let (reserves, supply) = verify(&ctx.accounts.vault, &ctx.accounts.wrapped_mint)?;
        let w = &mut ctx.accounts.wrapper;
        w.total_wrapped = w.total_wrapped.saturating_add(received);
        emit!(Wrapped { wrapper: w.key(), user: ctx.accounts.user.key(), deposited: amount, minted: received, reserves, supply });
        Ok(())
    }

    /// Burns `amount` wrapped tokens, then releases `amount` of the underlying from the vault.
    /// `min_out` bounds what `user_underlying` measurably receives (transfer fees apply).
    pub fn unwrap<'info>(ctx: Context<'_, '_, 'info, 'info, Operate<'info>>, amount: u64, min_out: u64) -> Result<()> {
        require!(amount > 0, WrapperError::ZeroAmount);
        let recipient = token_owner(&ctx.accounts.user_underlying)?;
        check(&ctx, token_hooks::callback::BEFORE_UNWRAP, recipient, amount)?;

        token_interface::burn_checked(
            CpiContext::new(
                ctx.accounts.token_2022_program.to_account_info(),
                BurnChecked {
                    mint: ctx.accounts.wrapped_mint.to_account_info(),
                    from: ctx.accounts.user_wrapped.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            amount,
            ctx.accounts.wrapper.decimals,
        )?;
        let before = token_amount(&ctx.accounts.user_underlying)?;
        let underlying = ctx.accounts.wrapper.underlying_mint;
        let seeds: &[&[u8]] = &[WRAPPER_SEED, underlying.as_ref(), &[ctx.accounts.wrapper.bump]];
        token_interface::transfer_checked(
            CpiContext::new_with_signer(
                ctx.accounts.underlying_program.to_account_info(),
                TransferChecked {
                    from: ctx.accounts.vault.to_account_info(),
                    mint: ctx.accounts.underlying_mint.to_account_info(),
                    to: ctx.accounts.user_underlying.to_account_info(),
                    authority: ctx.accounts.wrapper.to_account_info(),
                },
                &[seeds],
            ),
            amount,
            ctx.accounts.wrapper.decimals,
        )?;
        let received = token_amount(&ctx.accounts.user_underlying)?.saturating_sub(before);
        require!(received >= min_out, WrapperError::SlippageExceeded);
        let (reserves, supply) = verify(&ctx.accounts.vault, &ctx.accounts.wrapped_mint)?;
        let w = &mut ctx.accounts.wrapper;
        w.total_unwrapped = w.total_unwrapped.saturating_add(amount);
        emit!(Unwrapped { wrapper: w.key(), user: ctx.accounts.user.key(), burned: amount, released: received, reserves, supply });
        Ok(())
    }

    /// Fails unless the vault covers the wrapped supply; emits both. Anyone can call it.
    pub fn verify_reserves(ctx: Context<VerifyReserves>) -> Result<()> {
        let (reserves, supply) = verify(&ctx.accounts.vault, &ctx.accounts.wrapped_mint)?;
        emit!(ReservesVerified { wrapper: ctx.accounts.wrapper.key(), reserves, supply });
        Ok(())
    }
}

// ---------------------------------------------------------------- helpers

/// `(decimals, freeze authority, vault account length)`; refuses unsafe Token-2022 extensions.
fn inspect_underlying(mint: &AccountInfo) -> Result<(u8, Option<Pubkey>, usize)> {
    let data = mint.try_borrow_data()?;
    if *mint.owner == SPL_TOKEN {
        let m = StateWithExtensions::<MintState>::unpack(&data[..data.len().min(82)])?;
        require!(m.base.is_initialized, WrapperError::UnsupportedMint);
        return Ok((m.base.decimals, m.base.freeze_authority.into(), 165));
    }
    require_keys_eq!(*mint.owner, spl_token_2022::ID, WrapperError::WrongTokenProgram);
    let m = StateWithExtensions::<MintState>::unpack(&data)?;
    let types = m.get_extension_types()?;
    for t in &types {
        if !ALLOWED_UNDERLYING_EXTENSIONS.contains(t) {
            msg!("unsupported underlying extension: {:?}", t);
            return err!(WrapperError::UnsupportedExtension);
        }
    }
    let account_exts = ExtensionType::get_required_init_account_extensions(&types);
    let len = ExtensionType::try_calculate_account_len::<TokenAccountState>(&account_exts)?;
    Ok((m.base.decimals, m.base.freeze_authority.into(), len))
}

fn token_amount(info: &AccountInfo) -> Result<u64> {
    let d = info.try_borrow_data()?;
    require!(d.len() >= 165, WrapperError::NotTokenAccount);
    Ok(u64::from_le_bytes(d[64..72].try_into().unwrap()))
}

fn token_owner(info: &AccountInfo) -> Result<Pubkey> {
    require!(*info.owner == SPL_TOKEN || *info.owner == spl_token_2022::ID, WrapperError::NotTokenAccount);
    let d = info.try_borrow_data()?;
    require!(d.len() >= 165, WrapperError::NotTokenAccount);
    Ok(Pubkey::try_from(&d[32..64]).unwrap())
}

/// `(vault balance, wrapped supply)`; the vault must cover the supply.
fn verify(vault: &AccountInfo, wrapped_mint: &AccountInfo) -> Result<(u64, u64)> {
    let reserves = token_amount(vault)?;
    let supply = {
        let d = wrapped_mint.try_borrow_data()?;
        u64::from_le_bytes(d[36..44].try_into().unwrap())
    };
    require!(reserves >= supply, WrapperError::ReserveShortfall);
    Ok((reserves, supply))
}

fn check<'info>(ctx: &Context<'_, '_, 'info, 'info, Operate<'info>>, callback: u8, counterparty: Pubkey, amount: u64) -> Result<()> {
    let underlying = ctx.accounts.wrapper.underlying_mint;
    let seeds: &[&[u8]] = &[WRAPPER_SEED, underlying.as_ref(), &[ctx.accounts.wrapper.bump]];
    token_hooks::cpi::check_operation(
        CpiContext::new_with_signer(
            ctx.accounts.token_hooks_program.to_account_info(),
            token_hooks::cpi::accounts::CheckOperation {
                mint_authority: ctx.accounts.wrapper.to_account_info(),
                mint: ctx.accounts.wrapped_mint.to_account_info(),
                policy: ctx.accounts.policy.to_account_info(),
            },
            &[seeds],
        )
        .with_remaining_accounts(ctx.remaining_accounts.to_vec()),
        callback,
        ctx.accounts.user.key(),
        counterparty,
        amount,
    )
}

// ---------------------------------------------------------------- state

#[account]
#[derive(InitSpace)]
pub struct Wrapper {
    pub underlying_mint: Pubkey,
    pub underlying_program: Pubkey,
    pub wrapped_mint: Pubkey,
    pub vault: Pubkey,
    /// Default (all zero) when the underlying has none.
    pub underlying_freeze_authority: Pubkey,
    pub total_wrapped: u64,
    pub total_unwrapped: u64,
    pub decimals: u8,
    pub bump: u8,
}

// ---------------------------------------------------------------- accounts

#[derive(Accounts)]
pub struct CreateWrapper<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: SPL Token or Token-2022 mint; parsed and screened in the handler.
    pub underlying_mint: UncheckedAccount<'info>,
    #[account(init, payer = payer, space = 8 + Wrapper::INIT_SPACE, seeds = [WRAPPER_SEED, underlying_mint.key().as_ref()], bump)]
    pub wrapper: Account<'info, Wrapper>,
    /// CHECK: created here as a Token-2022 mint.
    #[account(mut, seeds = [MINT_SEED, underlying_mint.key().as_ref()], bump)]
    pub wrapped_mint: UncheckedAccount<'info>,
    /// CHECK: created here as the underlying token account owned by `wrapper`.
    #[account(mut, seeds = [VAULT_SEED, underlying_mint.key().as_ref()], bump)]
    pub vault: UncheckedAccount<'info>,
    /// CHECK: must own `underlying_mint`; checked in the handler.
    #[account(constraint = underlying_program.key() == SPL_TOKEN || underlying_program.key() == spl_token_2022::ID @ WrapperError::WrongTokenProgram)]
    pub underlying_program: UncheckedAccount<'info>,
    /// CHECK: Token-2022.
    #[account(address = spl_token_2022::ID)]
    pub token_2022_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Operate<'info> {
    pub user: Signer<'info>,
    #[account(
        mut,
        seeds = [WRAPPER_SEED, underlying_mint.key().as_ref()],
        bump = wrapper.bump,
        has_one = underlying_mint,
        has_one = wrapped_mint,
        has_one = vault,
        has_one = underlying_program,
    )]
    pub wrapper: Account<'info, Wrapper>,
    /// CHECK: bound by `wrapper`.
    pub underlying_mint: UncheckedAccount<'info>,
    /// CHECK: bound by `wrapper`.
    #[account(mut)]
    pub wrapped_mint: UncheckedAccount<'info>,
    /// CHECK: bound by `wrapper`.
    #[account(mut)]
    pub vault: UncheckedAccount<'info>,
    /// CHECK: the token program checks mint and authority.
    #[account(mut)]
    pub user_underlying: UncheckedAccount<'info>,
    /// CHECK: the token program checks mint and authority.
    #[account(mut)]
    pub user_wrapped: UncheckedAccount<'info>,
    /// CHECK: the wrapped mint's token policy; `token-hooks` checks its seeds.
    #[account(mut)]
    pub policy: UncheckedAccount<'info>,
    /// CHECK: bound by `wrapper`.
    pub underlying_program: UncheckedAccount<'info>,
    /// CHECK: Token-2022.
    #[account(address = spl_token_2022::ID)]
    pub token_2022_program: UncheckedAccount<'info>,
    /// CHECK: token-hooks.
    #[account(address = token_hooks::ID)]
    pub token_hooks_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct VerifyReserves<'info> {
    #[account(has_one = wrapped_mint, has_one = vault)]
    pub wrapper: Account<'info, Wrapper>,
    /// CHECK: bound by `wrapper`.
    pub wrapped_mint: UncheckedAccount<'info>,
    /// CHECK: bound by `wrapper`.
    pub vault: UncheckedAccount<'info>,
}

// ---------------------------------------------------------------- events & errors

#[event]
pub struct WrapperCreated {
    pub wrapper: Pubkey,
    pub underlying_mint: Pubkey,
    pub wrapped_mint: Pubkey,
    pub vault: Pubkey,
}
#[event]
pub struct Wrapped {
    pub wrapper: Pubkey,
    pub user: Pubkey,
    pub deposited: u64,
    pub minted: u64,
    pub reserves: u64,
    pub supply: u64,
}
#[event]
pub struct Unwrapped {
    pub wrapper: Pubkey,
    pub user: Pubkey,
    pub burned: u64,
    pub released: u64,
    pub reserves: u64,
    pub supply: u64,
}
#[event]
pub struct ReservesVerified {
    pub wrapper: Pubkey,
    pub reserves: u64,
    pub supply: u64,
}

#[error_code]
pub enum WrapperError {
    #[msg("Token program does not own this mint")]
    WrongTokenProgram,
    #[msg("Underlying mint is not initialized")]
    UnsupportedMint,
    #[msg("Underlying mint has an extension the vault cannot hold safely")]
    UnsupportedExtension,
    #[msg("Not a token account")]
    NotTokenAccount,
    #[msg("Amount must be positive")]
    ZeroAmount,
    #[msg("Received less than min_out")]
    SlippageExceeded,
    #[msg("Vault does not cover the wrapped supply")]
    ReserveShortfall,
}
