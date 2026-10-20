//! PUMP HOOKS ROUTER engine.
//!
//! `route` forwards a user-signed swap to an allowlisted venue instruction ([`venues`]), runs a
//! route policy's hook stack around it, measures what actually left and arrived in the user's
//! token accounts, enforces `max_in` / `min_out` / `deadline` on those measurements, and writes a
//! receipt keyed by `(user, nonce)` so a submission cannot be applied twice.
//!
//! Enforcement class: ROUTER. Anyone can call the venue directly and skip every hook here; route
//! policies are opt-in guardrails for users and integrators, not protocol rules.

#![allow(unexpected_cfgs, deprecated)]

use anchor_lang::{
    prelude::*,
    solana_program::{
        instruction::{AccountMeta, Instruction},
        program::invoke,
    },
};
use hook_kernel::{carry_state, cpi::CpiPolicyHooks, reason, requires_guard, validate_stack, Op, Record};
use hook_standard::{loader, state::HEADER_LEN, Environment, HookStack, STACK_LEN};

pub mod venues;

declare_id!("2a3wWdS8z9g7vBcYWSjdETyEzo7DEjV3iYRZFsM52q4U");

pub const CONFIG_SEED: &[u8] = b"router-config";
pub const POLICY_SEED: &[u8] = b"route-policy";
pub const RECEIPT_SEED: &[u8] = b"receipt";
pub const MAX_VENUE_DATA: usize = 256;
pub const MAX_POLICY_DELAY: i64 = 30 * 86_400;

pub mod callback {
    pub const BEFORE_EXECUTE: u8 = 0;
    pub const AFTER_EXECUTE: u8 = 1;
    pub const BEFORE_SWAP: u8 = 2;
    pub const AFTER_SWAP: u8 = 3;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct RouteParams {
    /// Number of venue-instruction accounts at the start of the remaining accounts; the venue
    /// program follows them, then hook accounts.
    pub venue_accounts: u8,
    pub max_in: u64,
    pub min_out: u64,
    /// Cap on SOL the venue may take from the user's wallet (rent for venue-side accounts).
    pub max_lamports: u64,
    pub deadline: i64,
    pub nonce: u64,
    pub data: Vec<u8>,
}

#[program]
pub mod hook_router {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        let info = loader::read_program(&ctx.accounts.router_program, Some(&ctx.accounts.router_program_data))?;
        require!(info.upgrade_authority == Some(ctx.accounts.admin.key()), RouterError::Unauthorized);
        let c = &mut ctx.accounts.config;
        c.admin = ctx.accounts.admin.key();
        c.enabled_venues = 0;
        c.bump = ctx.bumps.config;
        Ok(())
    }

    /// Venues start disabled. Enabling an unverified venue is allowed (for devnet testing) and
    /// recorded in the event and on every receipt via the venue id.
    pub fn set_venue(ctx: Context<AdminOnly>, venue: u8, enabled: bool) -> Result<()> {
        let v = venues::VENUES.iter().find(|v| v.id == venue).ok_or(error!(RouterError::UnknownVenue))?;
        let c = &mut ctx.accounts.config;
        if enabled {
            c.enabled_venues |= 1 << venue;
        } else {
            c.enabled_venues &= !(1 << venue);
        }
        emit!(VenueSet { venue, enabled, verified: v.verified });
        Ok(())
    }

    /// Remaining accounts: the registry `HookVersion` of every entry.
    ///
    /// A private policy (`shared = false`) only applies to routes signed by its authority. A shared
    /// policy can be used by anyone, so its stateful hooks (spending limits, accounting) are
    /// shared by all of its users.
    pub fn create_policy<'info>(ctx: Context<'_, '_, 'info, 'info, CreatePolicy<'info>>, seed: u64, delay: i64, shared: bool, stack: Vec<u8>) -> Result<()> {
        require!((0..=MAX_POLICY_DELAY).contains(&delay), RouterError::InvalidParams);
        let mut s = parse_stack(&stack)?;
        let empty: Box<HookStack> = bytemuck::zeroed_box();
        check_install(&empty, &mut s, ctx.remaining_accounts)?;
        let mut p = ctx.accounts.policy.load_init()?;
        p.authority = ctx.accounts.authority.key();
        p.seed = seed;
        p.delay = delay;
        p.shared = shared as u8;
        p.bump = ctx.bumps.policy;
        p.stack = *s;
        emit!(PolicyUpdated { policy: ctx.accounts.policy.key(), staged: false });
        Ok(())
    }

    /// Applies a stack immediately when every mandatory entry is kept unchanged; otherwise stages
    /// it. A staged stack is applied by calling again with the same bytes after `delay` (the
    /// guarded path for mandatory entries).
    pub fn update_policy<'info>(ctx: Context<'_, '_, 'info, 'info, UpdatePolicy<'info>>, stack: Vec<u8>) -> Result<()> {
        let mut s = parse_stack(&stack)?;
        let now = Clock::get()?.unix_timestamp;
        let hash = anchor_lang::solana_program::hash::hash(&stack).to_bytes();
        let (old, delay, pending_hash, pending_at) = {
            let p = ctx.accounts.policy.load()?;
            (boxed(&p.stack), p.delay, p.pending_hash, p.pending_at)
        };
        check_install(&old, &mut s, ctx.remaining_accounts)?;
        let mut p = ctx.accounts.policy.load_mut()?;
        if requires_guard(&old, &s) && delay > 0 {
            if pending_hash != hash {
                p.pending_hash = hash;
                p.pending_at = now;
                emit!(PolicyUpdated { policy: ctx.accounts.policy.key(), staged: true });
                return Ok(());
            }
            require!(now >= pending_at.saturating_add(delay), RouterError::TimelockPending);
        }
        p.stack = *s;
        p.pending_hash = [0; 32];
        p.pending_at = 0;
        p.sequence += 1;
        emit!(PolicyUpdated { policy: ctx.accounts.policy.key(), staged: false });
        Ok(())
    }

    pub fn route<'info>(ctx: Context<'_, '_, 'info, 'info, Route<'info>>, p: RouteParams) -> Result<()> {
        let clock = Clock::get()?;
        require!(clock.unix_timestamp <= p.deadline, RouterError::DeadlineExceeded);
        require!(p.data.len() <= MAX_VENUE_DATA, RouterError::InvalidParams);
        let n = p.venue_accounts as usize;
        let rem = ctx.remaining_accounts;
        require!(rem.len() > n, RouterError::AccountMismatch);
        let (venue_infos, rest) = rem.split_at(n);
        let venue_program = &rest[0];
        let hook_accounts = &rest[1..];

        // Adapter checks: allowlisted program and instruction, the user is the venue's signer, and
        // the measured accounts are the ones the venue debits and credits.
        let venue = venues::by_program(venue_program.key).ok_or(error!(RouterError::UnknownVenue))?;
        require!(ctx.accounts.config.enabled_venues & (1 << venue.id) != 0, RouterError::VenueDisabled);
        let swap = venue.swap(&p.data).ok_or(error!(RouterError::UnknownInstruction))?;
        require!((venue.min_accounts..=venue.max_accounts).contains(&n), RouterError::AccountMismatch);
        let user = ctx.accounts.user.key();
        require_keys_eq!(*venue_infos[venue.signer].key, user, RouterError::AccountMismatch);
        require_keys_eq!(*venue_infos[swap.input].key, ctx.accounts.user_input.key(), RouterError::AccountMismatch);
        require_keys_eq!(*venue_infos[swap.output].key, ctx.accounts.user_output.key(), RouterError::AccountMismatch);
        let policy_key = ctx.accounts.policy.as_ref().map(|p| p.key());
        for i in venue_infos {
            require!(Some(*i.key) != policy_key && *i.key != ctx.accounts.receipt.key(), RouterError::ProtectedAccount);
        }

        let (in_mint, in_before) = token_state(&ctx.accounts.user_input)?;
        let (out_mint, out_before) = token_state(&ctx.accounts.user_output)?;

        let mut stack = match &ctx.accounts.policy {
            Some(pl) => {
                let pol = pl.load()?;
                require!(pol.shared != 0 || pol.authority == user, RouterError::PolicyNotShared);
                Some(boxed(&pol.stack))
            }
            None => None,
        };
        let sequence = ctx.accounts.policy.as_ref().map(|pl| pl.load().map(|x| x.sequence)).transpose()?.unwrap_or(0);
        let records = records(hook_accounts);
        let recs: Vec<Record> = records.iter().map(|(o, d)| Record { owner: *o, data: d }).collect();
        let mut op = Op {
            subject: policy_key.unwrap_or_default(),
            actor: user,
            asset: in_mint,
            counterparty: venue.program,
            amount: p.max_in,
            amount_b: p.min_out,
            slot: clock.slot,
            now: clock.unix_timestamp,
            sequence,
            records: &recs,
            ..Op::default()
        };
        if let (Some(s), Some(pl)) = (stack.as_mut(), &ctx.accounts.policy) {
            let subject = pl.to_account_info();
            run(s, callback::BEFORE_EXECUTE, &op, &subject, hook_accounts)?;
            run(s, callback::BEFORE_SWAP, &op, &subject, hook_accounts)?;
        }

        // Only the venue's signer slot carries a signature.
        let metas = venue_infos
            .iter()
            .enumerate()
            .map(|(i, a)| AccountMeta { pubkey: *a.key, is_signer: i == venue.signer, is_writable: a.is_writable })
            .collect();
        let ix = Instruction { program_id: venue.program, accounts: metas, data: p.data.clone() };
        let mut infos = venue_infos.to_vec();
        infos.push(venue_program.clone());
        let lamports_before = ctx.accounts.user.lamports();
        invoke(&ix, &infos)?;
        let lamports_spent = lamports_before.saturating_sub(ctx.accounts.user.lamports());
        require!(lamports_spent <= p.max_lamports, RouterError::LamportsExceeded);

        let (_, in_after) = token_state(&ctx.accounts.user_input)?;
        let (_, out_after) = token_state(&ctx.accounts.user_output)?;
        let spent = in_before.saturating_sub(in_after);
        let received = out_after.saturating_sub(out_before);
        require!(in_after <= in_before && spent <= p.max_in, RouterError::MaxInExceeded);
        require!(out_after >= out_before && received >= p.min_out, RouterError::SlippageExceeded);

        op.amount = spent;
        op.amount_b = received;
        if let (Some(s), Some(pl)) = (stack.as_mut(), &ctx.accounts.policy) {
            let subject = pl.to_account_info();
            run(s, callback::AFTER_SWAP, &op, &subject, hook_accounts)?;
            run(s, callback::AFTER_EXECUTE, &op, &subject, hook_accounts)?;
            let mut pm = pl.load_mut()?;
            pm.stack = **s;
            pm.sequence += 1;
        }

        let r = &mut ctx.accounts.receipt;
        r.user = user;
        r.policy = policy_key.unwrap_or_default();
        r.venue = venue.id;
        r.venue_verified = venue.verified;
        r.input_mint = in_mint;
        r.output_mint = out_mint;
        r.spent = spent;
        r.received = received;
        r.lamports_spent = lamports_spent;
        r.max_in = p.max_in;
        r.min_out = p.min_out;
        r.slot = clock.slot;
        r.timestamp = clock.unix_timestamp;
        r.nonce = p.nonce;
        r.bump = ctx.bumps.receipt;
        emit!(Routed {
            receipt: r.key(),
            user,
            policy: r.policy,
            venue: venue.id,
            input_mint: in_mint,
            output_mint: out_mint,
            spent,
            received,
            nonce: p.nonce
        });
        Ok(())
    }

    /// Reclaims a receipt's rent. The nonce stays used only while the receipt exists, so clients
    /// should use fresh nonces (e.g. a timestamp) rather than reuse closed ones.
    pub fn close_receipt(_ctx: Context<CloseReceipt>) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------- helpers

fn boxed<T: bytemuck::Pod>(v: &T) -> Box<T> {
    let mut b: Box<T> = bytemuck::zeroed_box();
    bytemuck::bytes_of_mut(&mut *b).copy_from_slice(bytemuck::bytes_of(v));
    b
}

fn parse_stack(bytes: &[u8]) -> Result<Box<HookStack>> {
    require!(bytes.len() == STACK_LEN, RouterError::InvalidStack);
    let mut s: Box<HookStack> = bytemuck::zeroed_box();
    bytemuck::bytes_of_mut(&mut *s).copy_from_slice(bytes);
    require!(s.environment == Environment::Router as u8, RouterError::InvalidStack);
    Ok(s)
}

fn check_install<'info>(old: &HookStack, new: &mut HookStack, remaining: &'info [AccountInfo<'info>]) -> Result<()> {
    if let Err(e) = validate_stack(new, 0) {
        msg!("invalid stack: {:?}", e);
        return err!(RouterError::InvalidStack);
    }
    for e in new.active() {
        let key = Pubkey::new_from_array(e.hook_version);
        let info = remaining.iter().find(|i| *i.key == key).ok_or(error!(RouterError::MissingVersion))?;
        let version: Account<hook_registry::HookVersion> = Account::try_from(info)?;
        hook_registry::verify_install(&key, &version, e, Environment::Router)?;
    }
    carry_state(old, new);
    Ok(())
}

fn records(remaining: &[AccountInfo]) -> Vec<(Pubkey, Vec<u8>)> {
    remaining
        .iter()
        .filter(|i| !i.executable)
        .filter_map(|i| {
            let d = i.try_borrow_data().ok()?;
            (d.len() >= HEADER_LEN && d[..4] == hook_standard::state::MAGIC).then(|| (*i.owner, d[..HEADER_LEN].to_vec()))
        })
        .collect()
}

fn run<'info>(stack: &mut HookStack, cb: u8, op: &Op, subject: &AccountInfo<'info>, remaining: &[AccountInfo<'info>]) -> Result<()> {
    let mut hooks = CpiPolicyHooks { environment: Environment::Router, environment_program: &crate::ID, subject, accounts: remaining, op_data: &[] };
    hook_kernel::evaluate(stack, cb, op, &mut hooks).map_err(|r| {
        msg!("hook stack rejected: callback {} entry {} reason {} ({}) hook_reason {}", cb, r.index, r.reason, reason::describe(r.reason), r.hook_reason);
        emit!(RouteRejected { policy: op.subject, callback: cb, entry: r.index, reason: r.reason, hook_reason: r.hook_reason });
        error!(RouterError::PolicyRejected)
    })?;
    Ok(())
}

/// `(mint, amount)` of an SPL Token / Token-2022 account.
fn token_state(info: &AccountInfo) -> Result<(Pubkey, u64)> {
    let program = *info.owner;
    require!(program == TOKEN_PROGRAM || program == TOKEN_2022_PROGRAM, RouterError::NotTokenAccount);
    let d = info.try_borrow_data()?;
    require!(d.len() >= 165 && (d.len() == 165 || d[165] == 2), RouterError::NotTokenAccount);
    Ok((Pubkey::try_from(&d[..32]).unwrap(), u64::from_le_bytes(d[64..72].try_into().unwrap())))
}

pub const TOKEN_PROGRAM: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const TOKEN_2022_PROGRAM: Pubkey = pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

// ---------------------------------------------------------------- state

#[account]
#[derive(InitSpace)]
pub struct RouterConfig {
    pub admin: Pubkey,
    /// Bit `venue id` set = enabled.
    pub enabled_venues: u32,
    pub bump: u8,
}

#[account(zero_copy)]
#[repr(C)]
pub struct RoutePolicy {
    pub authority: Pubkey,
    pub pending_hash: [u8; 32],
    pub seed: u64,
    pub sequence: u64,
    pub delay: i64,
    pub pending_at: i64,
    pub bump: u8,
    pub shared: u8,
    pub _pad: [u8; 6],
    pub stack: HookStack,
}

#[account]
#[derive(InitSpace)]
pub struct Receipt {
    pub user: Pubkey,
    pub policy: Pubkey,
    pub venue: u8,
    pub venue_verified: bool,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub spent: u64,
    pub received: u64,
    pub lamports_spent: u64,
    pub max_in: u64,
    pub min_out: u64,
    pub slot: u64,
    pub timestamp: i64,
    pub nonce: u64,
    pub bump: u8,
}

// ---------------------------------------------------------------- accounts

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(init, payer = admin, space = 8 + RouterConfig::INIT_SPACE, seeds = [CONFIG_SEED], bump)]
    pub config: Account<'info, RouterConfig>,
    /// CHECK: this program.
    #[account(address = crate::ID)]
    pub router_program: UncheckedAccount<'info>,
    /// CHECK: validated by `loader::read_program`.
    pub router_program_data: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct AdminOnly<'info> {
    pub admin: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ RouterError::Unauthorized)]
    pub config: Account<'info, RouterConfig>,
}

#[derive(Accounts)]
#[instruction(seed: u64)]
pub struct CreatePolicy<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(init, payer = authority, space = 8 + core::mem::size_of::<RoutePolicy>(), seeds = [POLICY_SEED, authority.key().as_ref(), &seed.to_le_bytes()], bump)]
    pub policy: AccountLoader<'info, RoutePolicy>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdatePolicy<'info> {
    pub authority: Signer<'info>,
    #[account(mut, has_one = authority @ RouterError::Unauthorized)]
    pub policy: AccountLoader<'info, RoutePolicy>,
}

#[derive(Accounts)]
#[instruction(p: RouteParams)]
pub struct Route<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, RouterConfig>,
    #[account(mut)]
    pub policy: Option<AccountLoader<'info, RoutePolicy>>,
    /// CHECK: token account debited by the venue; validated by `token_state` and the adapter.
    #[account(mut)]
    pub user_input: UncheckedAccount<'info>,
    /// CHECK: token account credited by the venue; validated by `token_state` and the adapter.
    #[account(mut)]
    pub user_output: UncheckedAccount<'info>,
    #[account(init, payer = user, space = 8 + Receipt::INIT_SPACE, seeds = [RECEIPT_SEED, user.key().as_ref(), &p.nonce.to_le_bytes()], bump)]
    pub receipt: Account<'info, Receipt>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CloseReceipt<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(mut, has_one = user @ RouterError::Unauthorized, close = user)]
    pub receipt: Account<'info, Receipt>,
}

// ---------------------------------------------------------------- events & errors

#[event]
pub struct VenueSet {
    pub venue: u8,
    pub enabled: bool,
    pub verified: bool,
}
#[event]
pub struct PolicyUpdated {
    pub policy: Pubkey,
    pub staged: bool,
}
#[event]
pub struct Routed {
    pub receipt: Pubkey,
    pub user: Pubkey,
    pub policy: Pubkey,
    pub venue: u8,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub spent: u64,
    pub received: u64,
    pub nonce: u64,
}
#[event]
pub struct RouteRejected {
    pub policy: Pubkey,
    pub callback: u8,
    pub entry: u8,
    pub reason: u16,
    pub hook_reason: u16,
}

#[error_code]
pub enum RouterError {
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
    #[msg("Deadline exceeded")]
    DeadlineExceeded,
    #[msg("Venue program is not in the adapter table")]
    UnknownVenue,
    #[msg("Venue is disabled")]
    VenueDisabled,
    #[msg("Instruction is not an allowlisted swap for this venue")]
    UnknownInstruction,
    #[msg("Accounts do not match the venue adapter")]
    AccountMismatch,
    #[msg("Router state may not be passed to the venue")]
    ProtectedAccount,
    #[msg("Not a token account")]
    NotTokenAccount,
    #[msg("Spent more than max_in")]
    MaxInExceeded,
    #[msg("Received less than min_out")]
    SlippageExceeded,
    #[msg("Hook stack rejected the route")]
    PolicyRejected,
    #[msg("Policy is private to its authority")]
    PolicyNotShared,
    #[msg("Venue took more SOL from the user than max_lamports")]
    LamportsExceeded,
}
