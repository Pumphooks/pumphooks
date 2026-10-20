//! PUMP HOOKS TOKEN engine.
//!
//! A Token-2022 transfer-hook program. A mint whose `TransferHook` extension names this program
//! gets a [`TokenPolicy`] (a token-environment hook stack) and the interface's extra-account-metas
//! list. Token-2022 calls [`token_hooks::execute`] on every `transfer_checked` of that mint, and
//! the kernel evaluates the stack's `before_transfer` entries; a rejection fails the transfer.
//!
//! Enforcement class: NATIVE, for mints that name this program only. Existing SPL mints cannot
//! gain a transfer hook; the wrapper (`token-wrapper`) issues a new Token-2022 mint backed 1:1 by
//! the original, and its wrap / unwrap callbacks reach the same stack via [`token_hooks::check_operation`].
//!
//! Token stacks hold builtins only: a program hook would add a CPI level inside every transfer,
//! which breaks transfers made from inside other programs. Every account a builtin needs must be
//! derivable by Token-2022 from the extra-account-metas list, so record-sourced allowlists are
//! refused and credential entries declare the holder's gateway-token PDA.

#![allow(unexpected_cfgs, deprecated)]

use anchor_lang::{
    prelude::*,
    solana_program::{program::invoke_signed, system_instruction},
};
use anchor_spl::token_2022::spl_token_2022::{
    self,
    extension::{
        transfer_hook::{TransferHook, TransferHookAccount},
        BaseStateWithExtensions, StateWithExtensions,
    },
    state::{Account as Token2022Account, Mint as Token2022Mint},
};
use hook_kernel::{
    builtin::{self, CredentialConfig, GATEWAY_PROGRAM, TARGET_ACTOR},
    carry_state, reason, requires_guard, validate_stack, NoProgramHooks, Op, Record,
};
use hook_standard::{Environment, HookStack, IMPL_BUILTIN};
use spl_tlv_account_resolution::{account::ExtraAccountMeta, seeds::Seed, state::ExtraAccountMetaList};
use spl_transfer_hook_interface::instruction::ExecuteInstruction;

declare_id!("FZBBkQQamgfMCAoNk6PAdZi9nYZGMsGw9gDyTMNC3v54");

pub const POLICY_SEED: &[u8] = b"token-policy";
pub const EXTRA_METAS_SEED: &[u8] = b"extra-account-metas";
pub const MAX_POLICY_DELAY: i64 = 30 * 86_400;
/// Credential entries per token stack (each declares a network and a gateway-token PDA).
pub const MAX_CREDENTIALS: usize = 2;

pub mod callback {
    pub const BEFORE_TRANSFER: u8 = 0;
    pub const BEFORE_WRAP: u8 = 2;
    pub const BEFORE_UNWRAP: u8 = 3;
}

/// Index of the first extra account in `Execute` (source, mint, destination, owner, metas list).
const FIRST_EXTRA: u8 = 5;

#[program]
pub mod token_hooks {
    use super::*;

    /// Installs the mint's policy and its extra-account-metas list. Only the mint's transfer-hook
    /// authority can do this, and only for a mint whose hook program is this program. Remaining
    /// accounts: the registry `HookVersion` of every entry.
    pub fn create_policy<'info>(ctx: Context<'_, '_, 'info, 'info, CreatePolicy<'info>>, delay: i64, stack: Vec<u8>) -> Result<()> {
        require!((0..=MAX_POLICY_DELAY).contains(&delay), TokenHooksError::InvalidParams);
        let mint = ctx.accounts.mint.key();
        require_keys_eq!(transfer_hook_authority(&ctx.accounts.mint)?, ctx.accounts.authority.key(), TokenHooksError::Unauthorized);
        let mut s = parse_stack(&stack)?;
        let empty: Box<HookStack> = bytemuck::zeroed_box();
        check_install(&empty, &mut s, ctx.remaining_accounts)?;
        write_metas(
            &ctx.accounts.extra_account_metas,
            &ctx.accounts.payer,
            &ctx.accounts.system_program,
            &mint,
            ctx.bumps.extra_account_metas,
            &build_metas(&s),
        )?;
        let mut p = ctx.accounts.policy.load_init()?;
        p.mint = mint;
        p.authority = ctx.accounts.authority.key();
        p.delay = delay;
        p.bump = ctx.bumps.policy;
        p.metas_bump = ctx.bumps.extra_account_metas;
        p.stack = *s;
        emit!(PolicyUpdated { mint, staged: false });
        Ok(())
    }

    /// Applies a stack immediately when every mandatory entry is kept; otherwise stages it, and a
    /// second call with the same bytes after `delay` applies it. The extra-account-metas list is
    /// rebuilt from the applied stack.
    pub fn update_policy<'info>(ctx: Context<'_, '_, 'info, 'info, UpdatePolicy<'info>>, stack: Vec<u8>) -> Result<()> {
        let mut s = parse_stack(&stack)?;
        let now = Clock::get()?.unix_timestamp;
        let hash = anchor_lang::solana_program::hash::hash(&stack).to_bytes();
        let (old, delay, pending_hash, pending_at) = {
            let p = ctx.accounts.policy.load()?;
            (boxed(&p.stack), p.delay, p.pending_hash, p.pending_at)
        };
        check_install(&old, &mut s, ctx.remaining_accounts)?;
        let mint = ctx.accounts.mint.key();
        {
            let mut p = ctx.accounts.policy.load_mut()?;
            if requires_guard(&old, &s) && delay > 0 {
                if pending_hash != hash {
                    p.pending_hash = hash;
                    p.pending_at = now;
                    emit!(PolicyUpdated { mint, staged: true });
                    return Ok(());
                }
                require!(now >= pending_at.saturating_add(delay), TokenHooksError::TimelockPending);
            }
            p.stack = *s;
            p.pending_hash = [0; 32];
            p.pending_at = 0;
            p.sequence += 1;
        }
        write_metas(
            &ctx.accounts.extra_account_metas,
            &ctx.accounts.authority,
            &ctx.accounts.system_program,
            &mint,
            ctx.bumps.extra_account_metas,
            &build_metas(&s),
        )?;
        emit!(PolicyUpdated { mint, staged: false });
        Ok(())
    }

    /// The transfer-hook interface's `Execute`, called by Token-2022 during `transfer_checked`.
    /// Refused unless the source account is mid-transfer, so direct calls cannot consume
    /// spending windows or rate limits.
    #[instruction(discriminator = [105, 37, 101, 197, 75, 251, 102, 26])]
    pub fn execute<'info>(ctx: Context<'_, '_, 'info, 'info, Execute<'info>>, amount: u64) -> Result<()> {
        let mint = ctx.accounts.mint.key();
        let (source_mint, actor, transferring) = token_account(&ctx.accounts.source)?;
        let (dest_mint, counterparty, _) = token_account(&ctx.accounts.destination)?;
        require!(transferring, TokenHooksError::NotTransferring);
        require!(source_mint == mint && dest_mint == mint, TokenHooksError::MintMismatch);
        run(&ctx.accounts.policy, callback::BEFORE_TRANSFER, actor, counterparty, amount, ctx.remaining_accounts)
    }

    /// Runs the wrap / unwrap callbacks for the mint's issuer. The signer must be the mint's
    /// mint authority (the wrapper's PDA for wrapped assets).
    pub fn check_operation<'info>(
        ctx: Context<'_, '_, 'info, 'info, CheckOperation<'info>>,
        callback: u8,
        actor: Pubkey,
        counterparty: Pubkey,
        amount: u64,
    ) -> Result<()> {
        require!(matches!(callback, callback::BEFORE_WRAP | callback::BEFORE_UNWRAP), TokenHooksError::InvalidParams);
        let data = ctx.accounts.mint.try_borrow_data()?;
        let m = StateWithExtensions::<Token2022Mint>::unpack(&data)?;
        let authority: Option<Pubkey> = m.base.mint_authority.into();
        require!(authority == Some(ctx.accounts.mint_authority.key()), TokenHooksError::Unauthorized);
        drop(data);
        run(&ctx.accounts.policy, callback, actor, counterparty, amount, ctx.remaining_accounts)
    }
}

// ---------------------------------------------------------------- helpers

fn boxed<T: bytemuck::Pod>(v: &T) -> Box<T> {
    let mut b: Box<T> = bytemuck::zeroed_box();
    bytemuck::bytes_of_mut(&mut *b).copy_from_slice(bytemuck::bytes_of(v));
    b
}

fn parse_stack(bytes: &[u8]) -> Result<Box<HookStack>> {
    let mut s: Box<HookStack> = bytemuck::zeroed_box();
    require!(HookStack::read_compact(bytes, &mut s), TokenHooksError::InvalidStack);
    require!(s.environment == Environment::Token as u8, TokenHooksError::InvalidStack);
    Ok(s)
}

/// Token-specific install rules on top of `validate_stack`; see the module docs.
pub fn check_token_stack(s: &HookStack) -> core::result::Result<(), &'static str> {
    let mut credentials = 0;
    for e in s.active() {
        if e.impl_kind != IMPL_BUILTIN {
            return Err("token stacks hold builtins only");
        }
        if e.builtin_id == builtin::ALLOWLIST && e.config[1] != builtin::SOURCE_INLINE {
            return Err("token allowlists must be inline");
        }
        if e.builtin_id == builtin::CREDENTIAL {
            credentials += 1;
        }
    }
    if credentials > MAX_CREDENTIALS {
        return Err("too many credential entries");
    }
    Ok(())
}

fn check_install<'info>(old: &HookStack, new: &mut HookStack, remaining: &'info [AccountInfo<'info>]) -> Result<()> {
    if let Err(e) = validate_stack(new, 0) {
        msg!("invalid stack: {:?}", e);
        return err!(TokenHooksError::InvalidStack);
    }
    if let Err(e) = check_token_stack(new) {
        msg!("invalid token stack: {}", e);
        return err!(TokenHooksError::InvalidStack);
    }
    for e in new.active() {
        let key = Pubkey::new_from_array(e.hook_version);
        let info = remaining.iter().find(|i| *i.key == key).ok_or(error!(TokenHooksError::MissingVersion))?;
        let version: Account<hook_registry::HookVersion> = Account::try_from(info)?;
        hook_registry::verify_install(&key, &version, e, Environment::Token)?;
    }
    carry_state(old, new);
    Ok(())
}

/// The accounts Token-2022 resolves for `Execute`: the policy (writable, for builtin state),
/// then per credential entry the gatekeeper network and the holder's gateway-token PDA
/// (`[wallet, "gateway", [0; 8], network]` under the gateway program).
pub fn build_metas(s: &HookStack) -> Vec<ExtraAccountMeta> {
    let seeds = [
        Seed::Literal { bytes: POLICY_SEED.to_vec() },
        Seed::AccountKey { index: 1 },
    ];
    let mut metas = vec![ExtraAccountMeta::new_with_seeds(&seeds, false, true).unwrap()];
    let mut gateway: Option<u8> = None;
    for e in s.active().iter().filter(|e| e.builtin_id == builtin::CREDENTIAL) {
        let c: CredentialConfig = bytemuck::pod_read_unaligned(&e.config);
        let program = *gateway.get_or_insert_with(|| {
            metas.push(ExtraAccountMeta::new_with_pubkey(&GATEWAY_PROGRAM, false, false).unwrap());
            FIRST_EXTRA + metas.len() as u8 - 1
        });
        metas.push(ExtraAccountMeta::new_with_pubkey(&Pubkey::new_from_array(c.gatekeeper_network), false, false).unwrap());
        let network = FIRST_EXTRA + metas.len() as u8 - 1;
        // Token account owner is bytes 32..64 of the source (0) or destination (2) account.
        let holder = if c.target == TARGET_ACTOR { 0 } else { 2 };
        let pda = [
            Seed::AccountData { account_index: holder, data_index: 32, length: 32 },
            Seed::Literal { bytes: b"gateway".to_vec() },
            Seed::Literal { bytes: vec![0; 8] },
            Seed::AccountKey { index: network },
        ];
        metas.push(ExtraAccountMeta::new_external_pda_with_seeds(program, &pda, false, false).unwrap());
    }
    metas
}

fn write_metas<'info>(
    info: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    system: &AccountInfo<'info>,
    mint: &Pubkey,
    bump: u8,
    metas: &[ExtraAccountMeta],
) -> Result<()> {
    let size = ExtraAccountMetaList::size_of(metas.len())?;
    let rent = Rent::get()?.minimum_balance(size);
    if info.data_is_empty() {
        let seeds: &[&[u8]] = &[EXTRA_METAS_SEED, mint.as_ref(), &[bump]];
        invoke_signed(
            &system_instruction::create_account(payer.key, info.key, rent, size as u64, &crate::ID),
            &[payer.clone(), info.clone(), system.clone()],
            &[seeds],
        )?;
    } else {
        require_keys_eq!(*info.owner, crate::ID, TokenHooksError::InvalidParams);
        if info.lamports() < rent {
            anchor_lang::solana_program::program::invoke(
                &system_instruction::transfer(payer.key, info.key, rent - info.lamports()),
                &[payer.clone(), info.clone(), system.clone()],
            )?;
        }
        info.realloc(size, false)?;
    }
    let mut d = info.try_borrow_mut_data()?;
    d.fill(0);
    ExtraAccountMetaList::init::<ExecuteInstruction>(&mut d, metas)?;
    Ok(())
}

fn transfer_hook_authority(mint: &AccountInfo) -> Result<Pubkey> {
    let data = mint.try_borrow_data()?;
    let m = StateWithExtensions::<Token2022Mint>::unpack(&data)?;
    let hook = m.get_extension::<TransferHook>().map_err(|_| error!(TokenHooksError::NotHookedMint))?;
    let program: Option<Pubkey> = hook.program_id.into();
    require!(program == Some(crate::ID), TokenHooksError::NotHookedMint);
    let authority: Option<Pubkey> = hook.authority.into();
    authority.ok_or(error!(TokenHooksError::Unauthorized))
}

/// `(mint, owner, transferring)` of a Token-2022 account.
fn token_account(info: &AccountInfo) -> Result<(Pubkey, Pubkey, bool)> {
    require_keys_eq!(*info.owner, spl_token_2022::ID, TokenHooksError::MintMismatch);
    let data = info.try_borrow_data()?;
    let a = StateWithExtensions::<Token2022Account>::unpack(&data)?;
    let transferring = a.get_extension::<TransferHookAccount>().map(|e| bool::from(e.transferring)).unwrap_or(false);
    Ok((a.base.mint, a.base.owner, transferring))
}

fn run<'info>(
    policy: &AccountLoader<'info, TokenPolicy>,
    cb: u8,
    actor: Pubkey,
    counterparty: Pubkey,
    amount: u64,
    remaining: &[AccountInfo<'info>],
) -> Result<()> {
    let clock = Clock::get()?;
    let (mut stack, mint, sequence) = {
        let p = policy.load()?;
        (boxed(&p.stack), p.mint, p.sequence)
    };
    let records = hook_kernel::collect_records(remaining);
    let recs: Vec<Record> = records.iter().map(|(o, d)| Record { owner: *o, data: d }).collect();
    let op = Op {
        subject: policy.key(),
        actor,
        asset: mint,
        counterparty,
        amount,
        slot: clock.slot,
        now: clock.unix_timestamp,
        sequence,
        records: &recs,
        ..Op::default()
    };
    hook_kernel::evaluate(&mut stack, cb, &op, &mut NoProgramHooks).map_err(|r| {
        msg!("hook stack rejected: callback {} entry {} reason {} ({})", cb, r.index, r.reason, reason::describe(r.reason));
        emit!(TransferRejected { mint, callback: cb, entry: r.index, reason: r.reason });
        error!(TokenHooksError::PolicyRejected)
    })?;
    let mut p = policy.load_mut()?;
    p.stack = *stack;
    p.sequence += 1;
    Ok(())
}

// ---------------------------------------------------------------- state

#[account(zero_copy)]
#[repr(C)]
pub struct TokenPolicy {
    pub mint: Pubkey,
    pub authority: Pubkey,
    pub pending_hash: [u8; 32],
    pub sequence: u64,
    pub delay: i64,
    pub pending_at: i64,
    pub bump: u8,
    /// Canonical bump of this mint's extra-account-metas PDA, so `execute` verifies it at a fixed cost.
    pub metas_bump: u8,
    pub _pad: [u8; 6],
    pub stack: HookStack,
}

// ---------------------------------------------------------------- accounts

#[derive(Accounts)]
pub struct CreatePolicy<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    pub authority: Signer<'info>,
    /// CHECK: Token-2022 mint; its transfer-hook extension is checked in the handler.
    #[account(owner = spl_token_2022::ID)]
    pub mint: UncheckedAccount<'info>,
    #[account(init, payer = payer, space = 8 + core::mem::size_of::<TokenPolicy>(), seeds = [POLICY_SEED, mint.key().as_ref()], bump)]
    pub policy: AccountLoader<'info, TokenPolicy>,
    /// CHECK: created here with the interface's layout.
    #[account(mut, seeds = [EXTRA_METAS_SEED, mint.key().as_ref()], bump)]
    pub extra_account_metas: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdatePolicy<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    /// CHECK: key bound by the policy seeds.
    pub mint: UncheckedAccount<'info>,
    #[account(mut, seeds = [POLICY_SEED, mint.key().as_ref()], bump = policy.load()?.bump, has_one = authority @ TokenHooksError::Unauthorized, has_one = mint)]
    pub policy: AccountLoader<'info, TokenPolicy>,
    /// CHECK: rewritten with the interface's layout.
    #[account(mut, seeds = [EXTRA_METAS_SEED, mint.key().as_ref()], bump)]
    pub extra_account_metas: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Execute<'info> {
    /// CHECK: parsed in the handler (Token-2022 account, mid-transfer).
    pub source: UncheckedAccount<'info>,
    /// CHECK: key bound by the policy and metas seeds.
    pub mint: UncheckedAccount<'info>,
    /// CHECK: parsed in the handler.
    pub destination: UncheckedAccount<'info>,
    /// CHECK: the transfer authority (owner or delegate); Token-2022 has verified it.
    pub owner: UncheckedAccount<'info>,
    /// CHECK: the interface's validation account for this mint.
    #[account(seeds = [EXTRA_METAS_SEED, mint.key().as_ref()], bump = policy.load()?.metas_bump)]
    pub extra_account_metas: UncheckedAccount<'info>,
    #[account(mut, seeds = [POLICY_SEED, mint.key().as_ref()], bump = policy.load()?.bump)]
    pub policy: AccountLoader<'info, TokenPolicy>,
}

#[derive(Accounts)]
pub struct CheckOperation<'info> {
    pub mint_authority: Signer<'info>,
    /// CHECK: Token-2022 mint; its mint authority is checked in the handler.
    #[account(owner = spl_token_2022::ID)]
    pub mint: UncheckedAccount<'info>,
    #[account(mut, seeds = [POLICY_SEED, mint.key().as_ref()], bump = policy.load()?.bump)]
    pub policy: AccountLoader<'info, TokenPolicy>,
}

// ---------------------------------------------------------------- events & errors

#[event]
pub struct PolicyUpdated {
    pub mint: Pubkey,
    pub staged: bool,
}

#[event]
pub struct TransferRejected {
    pub mint: Pubkey,
    pub callback: u8,
    pub entry: u8,
    pub reason: u16,
}

#[error_code]
pub enum TokenHooksError {
    #[msg("Signer is not authorized")]
    Unauthorized,
    #[msg("Invalid parameters")]
    InvalidParams,
    #[msg("Invalid hook stack")]
    InvalidStack,
    #[msg("Registry version account missing for a stack entry")]
    MissingVersion,
    #[msg("Policy change is staged; apply it after the delay")]
    TimelockPending,
    #[msg("Mint's transfer hook does not name this program")]
    NotHookedMint,
    #[msg("Execute is only valid during a Token-2022 transfer")]
    NotTransferring,
    #[msg("Token account does not belong to this mint")]
    MintMismatch,
    #[msg("Hook stack rejected the operation")]
    PolicyRejected,
}
