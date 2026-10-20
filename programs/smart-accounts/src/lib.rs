//! PUMP HOOKS ACCOUNT engine: smart accounts.
//!
//! A smart account is a zero-copy state account plus a system-owned vault PDA that holds SOL and
//! owns token accounts. Every operation runs the account's hook stack through the shared kernel:
//!
//! | operation | callbacks | authority |
//! | --- | --- | --- |
//! | deposit | before/after_deposit | anyone |
//! | withdraw | before/after_withdraw | owner, scoped session, or an approved proposal |
//! | execute (CPI signed by the vault) | before/after_execute | owner, scoped session, or an approved proposal |
//! | govern (policy, owner, recovery, members, delays, unlock) | before/after_authority_change | owner (+ proposal when loosening), or recovery after its delay |
//!
//! Spending policies on `execute` see *measured* outflows (vault lamports and vault-owned token
//! accounts before vs after), and the CPI may not change a tracked token account's owner,
//! delegate or close authority, close it, or reassign the vault.

#![allow(unexpected_cfgs, deprecated)]

use anchor_lang::{
    prelude::*,
    solana_program::{
        hash::hash,
        instruction::{AccountMeta, Instruction},
        program::invoke_signed,
        system_program,
    },
};
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};
use hook_kernel::{
    carry_state, cpi::CpiPolicyHooks, reason, requires_guard, validate_stack, Op, Record,
};
use hook_standard::{state::HEADER_LEN, Environment, HookStack, STACK_LEN};

pub mod state;
use state::*;

declare_id!("6AEiwvifk4TVJ8g3UoVXqhaK8AqYy43A4DzWCgMmrtQw");

pub mod callback {
    pub const BEFORE_DEPOSIT: u8 = 0;
    pub const AFTER_DEPOSIT: u8 = 1;
    pub const BEFORE_WITHDRAW: u8 = 2;
    pub const AFTER_WITHDRAW: u8 = 3;
    pub const BEFORE_EXECUTE: u8 = 4;
    pub const AFTER_EXECUTE: u8 = 5;
    pub const BEFORE_AUTHORITY_CHANGE: u8 = 6;
    pub const AFTER_AUTHORITY_CHANGE: u8 = 7;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct CpiMeta {
    pub pubkey: Pubkey,
    pub is_signer: bool,
    pub is_writable: bool,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub enum Governance {
    SetPolicy { stack: Vec<u8> },
    SetOwner { owner: Pubkey },
    SetRecovery { recovery: Pubkey, delay: i64 },
    SetMembers { members: Vec<Pubkey> },
    SetPolicyDelay { delay: i64 },
    Unlock,
}

/// What a proposal commits to. The hash binds every account and flag of an `Execute`, so an
/// approved proposal cannot be replayed with substituted accounts.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Withdraw {
        asset: Pubkey,
        amount: u64,
        destination: Pubkey,
    },
    Execute {
        program: Pubkey,
        accounts: Vec<CpiMeta>,
        data: Vec<u8>,
    },
    Govern(Governance),
}

pub fn action_hash(a: &Action) -> [u8; 32] {
    hash(&a.try_to_vec().unwrap()).to_bytes()
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct CreateParams {
    pub seed: u64,
    pub owner: Pubkey,
    pub recovery: Pubkey,
    pub recovery_delay: i64,
    pub policy_delay: i64,
    pub members: Vec<Pubkey>,
    pub stack: Vec<u8>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SessionParams {
    pub key: Pubkey,
    pub expires_at: i64,
    pub ops: u8,
    pub asset: Pubkey,
    pub max_amount: u64,
    pub target_program: Pubkey,
}

#[program]
pub mod smart_accounts {
    use super::*;

    /// Remaining accounts: the registry `HookVersion` of every stack entry.
    pub fn create_account<'info>(
        ctx: Context<'_, '_, 'info, 'info, CreateAccount<'info>>,
        p: CreateParams,
    ) -> Result<()> {
        require!(p.owner != Pubkey::default(), AccountError::InvalidParams);
        require!(p.members.len() <= MAX_MEMBERS, AccountError::InvalidParams);
        require!(
            (0..=MAX_DELAY).contains(&p.policy_delay),
            AccountError::InvalidParams
        );
        if p.recovery != Pubkey::default() {
            require!(
                (MIN_RECOVERY_DELAY..=MAX_DELAY).contains(&p.recovery_delay),
                AccountError::InvalidParams
            );
        }
        let mut stack = parse_stack(&p.stack)?;
        let empty: Box<HookStack> = bytemuck::zeroed_box();
        check_install(
            &empty,
            &mut stack,
            p.members.len() as u8,
            ctx.remaining_accounts,
        )?;

        let mut a = ctx.accounts.account.load_init()?;
        a.owner = p.owner;
        a.recovery = p.recovery;
        a.creator = ctx.accounts.payer.key();
        for (i, m) in p.members.iter().enumerate() {
            a.members[i] = *m;
        }
        a.member_count = p.members.len() as u8;
        a.seed = p.seed;
        a.recovery_delay = p.recovery_delay;
        a.policy_delay = p.policy_delay;
        a.bump = ctx.bumps.account;
        a.vault_bump = ctx.bumps.vault;
        a.stack = *stack;
        emit!(AccountCreated {
            account: ctx.accounts.account.key(),
            owner: p.owner,
            vault: ctx.accounts.vault.key()
        });
        Ok(())
    }

    /// Anyone may deposit. Token deposits must land in a token account owned by the vault.
    pub fn deposit<'info>(
        ctx: Context<'_, '_, 'info, 'info, Deposit<'info>>,
        amount: u64,
    ) -> Result<()> {
        require!(amount > 0, AccountError::InvalidParams);
        let a = &ctx.accounts;
        let asset = a.mint.as_ref().map(|m| m.key()).unwrap_or_default();
        let (mut stack, sequence, _) = snapshot(&a.account)?;
        let records = records(ctx.remaining_accounts);
        let recs: Vec<Record> = records
            .iter()
            .map(|(o, d)| Record { owner: *o, data: d })
            .collect();
        let clock = Clock::get()?;
        let op = Op {
            subject: a.account.key(),
            actor: a.depositor.key(),
            asset,
            counterparty: a.depositor.key(),
            amount,
            slot: clock.slot,
            now: clock.unix_timestamp,
            sequence,
            records: &recs,
            ..Op::default()
        };
        let subject = a.account.to_account_info();
        run(
            &mut stack,
            callback::BEFORE_DEPOSIT,
            &op,
            &subject,
            ctx.remaining_accounts,
        )?;
        match (
            &a.mint,
            &a.depositor_token,
            &a.vault_token,
            &a.token_program,
        ) {
            (None, None, None, None) => {
                system_program_transfer(
                    &a.depositor.to_account_info(),
                    &a.vault.to_account_info(),
                    &a.system_program,
                    amount,
                    None,
                )?;
            }
            (Some(mint), Some(from), Some(to), Some(tp)) => {
                require_keys_eq!(to.owner, a.vault.key(), AccountError::NotVaultTokenAccount);
                token_interface::transfer_checked(
                    CpiContext::new(
                        tp.to_account_info(),
                        TransferChecked {
                            from: from.to_account_info(),
                            mint: mint.to_account_info(),
                            to: to.to_account_info(),
                            authority: a.depositor.to_account_info(),
                        },
                    ),
                    amount,
                    mint.decimals,
                )?;
            }
            _ => return err!(AccountError::InvalidParams),
        }
        run(
            &mut stack,
            callback::AFTER_DEPOSIT,
            &op,
            &subject,
            ctx.remaining_accounts,
        )?;
        commit(&a.account, &stack)?;
        emit!(Deposited {
            account: a.account.key(),
            depositor: a.depositor.key(),
            asset,
            amount
        });
        Ok(())
    }

    /// Remaining accounts: hook programs, their callback authorities, state records, and member
    /// signers (counted as approvals).
    pub fn withdraw<'info>(
        ctx: Context<'_, '_, 'info, 'info, Withdraw<'info>>,
        amount: u64,
    ) -> Result<()> {
        require!(amount > 0, AccountError::InvalidParams);
        let a = &ctx.accounts;
        let asset = a.mint.as_ref().map(|m| m.key()).unwrap_or_default();
        let destination = a.destination.key();
        let action = Action::Withdraw {
            asset,
            amount,
            destination,
        };
        let (mut stack, sequence, acct) = snapshot(&a.account)?;
        require!(acct.locked == 0, AccountError::Locked);
        let clock = Clock::get()?;

        let auth = authorize(
            &acct,
            &a.authority.key(),
            a.proposal.as_deref(),
            &action,
            ctx.remaining_accounts,
            clock.unix_timestamp,
        )?;
        if !auth.via_proposal && a.authority.key() != acct.owner {
            let s = check_session(
                &a.account.key(),
                &acct,
                a.session.as_deref(),
                &a.authority.key(),
                clock.unix_timestamp,
                SESSION_WITHDRAW,
            )?;
            require!(
                s.asset == asset && amount <= s.max_amount,
                AccountError::SessionScope
            );
        }
        consume(&mut ctx.accounts.proposal);
        let a = &ctx.accounts;
        let records = records(ctx.remaining_accounts);
        let recs: Vec<Record> = records
            .iter()
            .map(|(o, d)| Record { owner: *o, data: d })
            .collect();
        let op = Op {
            subject: a.account.key(),
            actor: a.authority.key(),
            asset,
            counterparty: destination,
            amount,
            slot: clock.slot,
            now: clock.unix_timestamp,
            sequence,
            approvals: auth.approvals,
            proposal_created_at: auth.proposal_created_at,
            records: &recs,
            ..Op::default()
        };
        let subject = a.account.to_account_info();
        run(
            &mut stack,
            callback::BEFORE_WITHDRAW,
            &op,
            &subject,
            ctx.remaining_accounts,
        )?;

        let account_key = a.account.key();
        let seeds: &[&[u8]] = &[VAULT_SEED, account_key.as_ref(), &[acct.vault_bump]];
        match (&a.mint, &a.vault_token, &a.token_program) {
            (None, None, None) => {
                system_program_transfer(
                    &a.vault.to_account_info(),
                    &a.destination.to_account_info(),
                    &a.system_program,
                    amount,
                    Some(seeds),
                )?;
            }
            (Some(mint), Some(from), Some(tp)) => {
                require_keys_eq!(
                    from.owner,
                    a.vault.key(),
                    AccountError::NotVaultTokenAccount
                );
                token_interface::transfer_checked(
                    CpiContext::new_with_signer(
                        tp.to_account_info(),
                        TransferChecked {
                            from: from.to_account_info(),
                            mint: mint.to_account_info(),
                            to: a.destination.to_account_info(),
                            authority: a.vault.to_account_info(),
                        },
                        &[seeds],
                    ),
                    amount,
                    mint.decimals,
                )?;
            }
            _ => return err!(AccountError::InvalidParams),
        }
        run(
            &mut stack,
            callback::AFTER_WITHDRAW,
            &op,
            &subject,
            ctx.remaining_accounts,
        )?;
        commit(&a.account, &stack)?;
        emit!(Withdrawn {
            account: account_key,
            authority: a.authority.key(),
            asset,
            amount,
            destination
        });
        Ok(())
    }

    /// Calls `program` with the vault as signer. Remaining accounts: exactly the CPI accounts in
    /// order, then `program`, then hook accounts.
    pub fn execute<'info>(
        ctx: Context<'_, '_, 'info, 'info, Execute<'info>>,
        program: Pubkey,
        accounts: Vec<CpiMeta>,
        data: Vec<u8>,
    ) -> Result<()> {
        require!(
            accounts.len() <= MAX_CPI_ACCOUNTS && data.len() <= MAX_CPI_DATA,
            AccountError::InvalidParams
        );
        require!(program != crate::ID, AccountError::ProtectedAccount);
        let a = &ctx.accounts;
        let account_key = a.account.key();
        let vault_key = a.vault.key();
        let rem = ctx.remaining_accounts;
        require!(rem.len() > accounts.len(), AccountError::AccountMismatch);
        let (cpi_infos, rest) = rem.split_at(accounts.len());
        require_keys_eq!(*rest[0].key, program, AccountError::AccountMismatch);
        let mut protected = vec![account_key];
        if let Some(p) = &a.proposal {
            protected.push(p.key());
        }
        if let Some(s) = &a.session {
            protected.push(s.key());
        }
        for (m, info) in accounts.iter().zip(cpi_infos) {
            require_keys_eq!(m.pubkey, *info.key, AccountError::AccountMismatch);
            require!(
                !protected.contains(&m.pubkey),
                AccountError::ProtectedAccount
            );
        }

        let action = Action::Execute {
            program,
            accounts: accounts.clone(),
            data: data.clone(),
        };
        let (mut stack, sequence, acct) = snapshot(&a.account)?;
        require!(acct.locked == 0, AccountError::Locked);
        let clock = Clock::get()?;
        let auth = authorize(
            &acct,
            &a.authority.key(),
            a.proposal.as_deref(),
            &action,
            rest,
            clock.unix_timestamp,
        )?;
        if !auth.via_proposal && a.authority.key() != acct.owner {
            let s = check_session(
                &account_key,
                &acct,
                a.session.as_deref(),
                &a.authority.key(),
                clock.unix_timestamp,
                SESSION_EXECUTE,
            )?;
            require_keys_eq!(s.target_program, program, AccountError::SessionScope);
        }
        consume(&mut ctx.accounts.proposal);
        let a = &ctx.accounts;
        let records = records(rest);
        let recs: Vec<Record> = records
            .iter()
            .map(|(o, d)| Record { owner: *o, data: d })
            .collect();
        let mut op = Op {
            subject: account_key,
            actor: a.authority.key(),
            counterparty: program,
            slot: clock.slot,
            now: clock.unix_timestamp,
            sequence,
            approvals: auth.approvals,
            proposal_created_at: auth.proposal_created_at,
            records: &recs,
            ..Op::default()
        };
        let subject = a.account.to_account_info();
        run(&mut stack, callback::BEFORE_EXECUTE, &op, &subject, rest)?;

        // Snapshot the vault and every writable vault-owned token account in the CPI.
        let vault_info = a.vault.to_account_info();
        let vault_before = vault_info.lamports();
        let tracked: Vec<TokenSnapshot> = accounts
            .iter()
            .zip(cpi_infos)
            .filter(|(m, _)| m.is_writable)
            .filter_map(|(_, info)| TokenSnapshot::take(info, &vault_key))
            .collect();

        let ix = Instruction {
            program_id: program,
            accounts: accounts
                .iter()
                .map(|m| AccountMeta {
                    pubkey: m.pubkey,
                    is_signer: m.is_signer,
                    is_writable: m.is_writable,
                })
                .collect(),
            data,
        };
        let mut infos: Vec<AccountInfo<'info>> = cpi_infos.to_vec();
        infos.push(rest[0].clone());
        invoke_signed(
            &ix,
            &infos,
            &[&[VAULT_SEED, account_key.as_ref(), &[acct.vault_bump]]],
        )?;

        // Side channels: the vault must stay a plain system account; tracked token accounts keep
        // their owner, delegate and close authority, and are not closed.
        require!(
            *vault_info.owner == system_program::ID && vault_info.data_is_empty(),
            AccountError::SideChannel
        );
        let mut outflows: Vec<(Pubkey, u64)> = vec![];
        let sol_out = vault_before.saturating_sub(vault_info.lamports());
        if sol_out > 0 {
            outflows.push((Pubkey::default(), sol_out));
        }
        for t in &tracked {
            let spent = t.verify_after(cpi_infos.iter().find(|i| *i.key == t.key).unwrap())?;
            if spent > 0 {
                match outflows.iter_mut().find(|(m, _)| *m == t.mint) {
                    Some((_, v)) => *v = v.checked_add(spent).ok_or(AccountError::Overflow)?,
                    None => outflows.push((t.mint, spent)),
                }
            }
        }
        if outflows.is_empty() {
            run(&mut stack, callback::AFTER_EXECUTE, &op, &subject, rest)?;
        }
        for (asset, amount) in &outflows {
            op.asset = *asset;
            op.amount = *amount;
            run(&mut stack, callback::AFTER_EXECUTE, &op, &subject, rest)?;
        }
        commit(&a.account, &stack)?;
        emit!(Executed {
            account: account_key,
            authority: a.authority.key(),
            program,
            outflows: outflows.len() as u8
        });
        Ok(())
    }

    /// Governance. Loosening changes require an approved proposal at least `policy_delay` old;
    /// the recovery authority may replace the owner through a proposal at least
    /// `recovery_delay` old (bypassing the stack, so a lost owner key is recoverable).
    /// Remaining accounts: registry versions for a new stack, hook accounts, member signers.
    pub fn govern<'info>(
        ctx: Context<'_, '_, 'info, 'info, Govern<'info>>,
        g: Governance,
    ) -> Result<()> {
        let a = &ctx.accounts;
        let action = Action::Govern(g.clone());
        let (mut stack, sequence, acct) = snapshot(&a.account)?;
        let clock = Clock::get()?;
        let now = clock.unix_timestamp;
        let authority = a.authority.key();
        let auth = authorize(
            &acct,
            &authority,
            a.proposal.as_deref(),
            &action,
            ctx.remaining_accounts,
            now,
        )?;

        let by_recovery =
            acct.has_recovery() && authority == acct.recovery && authority != acct.owner;
        let mut new_stack = None;
        if by_recovery {
            let p = a
                .proposal
                .as_ref()
                .ok_or(error!(AccountError::Unauthorized))?;
            require!(
                matches!(g, Governance::SetOwner { .. }),
                AccountError::Unauthorized
            );
            require_keys_eq!(p.proposer, acct.recovery, AccountError::Unauthorized);
            require!(
                now >= p.created_at.saturating_add(acct.recovery_delay),
                AccountError::TimelockPending
            );
        } else {
            require_keys_eq!(authority, acct.owner, AccountError::Unauthorized);
            let recovery_cosigned = acct.has_recovery()
                && ctx
                    .remaining_accounts
                    .iter()
                    .any(|i| i.is_signer && *i.key == acct.recovery);
            let guarded = match &g {
                Governance::SetPolicy { stack: bytes } => {
                    let mut s = parse_stack(bytes)?;
                    let members = acct.member_count;
                    check_install(&stack, &mut s, members, ctx.remaining_accounts)?;
                    let guarded = requires_guard(&stack, &s);
                    new_stack = Some(s);
                    guarded
                }
                Governance::SetOwner { .. }
                | Governance::SetRecovery { .. }
                | Governance::SetMembers { .. } => true,
                Governance::SetPolicyDelay { delay } => *delay < acct.policy_delay,
                Governance::Unlock => !recovery_cosigned,
            };
            if guarded {
                require!(auth.via_proposal, AccountError::ProposalRequired);
                require!(
                    now >= auth
                        .proposal_created_at
                        .unwrap()
                        .saturating_add(acct.policy_delay),
                    AccountError::TimelockPending
                );
            }
            let records = records(ctx.remaining_accounts);
            let recs: Vec<Record> = records
                .iter()
                .map(|(o, d)| Record { owner: *o, data: d })
                .collect();
            let op = Op {
                subject: a.account.key(),
                actor: authority,
                counterparty: match &g {
                    Governance::SetOwner { owner } => *owner,
                    Governance::SetRecovery { recovery, .. } => *recovery,
                    _ => Pubkey::default(),
                },
                slot: clock.slot,
                now,
                sequence,
                approvals: auth.approvals,
                proposal_created_at: auth.proposal_created_at,
                records: &recs,
                ..Op::default()
            };
            let subject = a.account.to_account_info();
            run(
                &mut stack,
                callback::BEFORE_AUTHORITY_CHANGE,
                &op,
                &subject,
                ctx.remaining_accounts,
            )?;
            run(
                &mut stack,
                callback::AFTER_AUTHORITY_CHANGE,
                &op,
                &subject,
                ctx.remaining_accounts,
            )?;
        }

        if let Some(s) = new_stack.as_mut() {
            carry_state(&stack, s);
        }
        consume(&mut ctx.accounts.proposal);
        let a = &ctx.accounts;
        let mut acc = a.account.load_mut()?;
        acc.stack = *new_stack.unwrap_or(stack);
        match g {
            Governance::SetPolicy { .. } => {}
            Governance::SetOwner { owner } => {
                require!(owner != Pubkey::default(), AccountError::InvalidParams);
                acc.owner = owner;
            }
            Governance::SetRecovery { recovery, delay } => {
                if recovery != Pubkey::default() {
                    require!(
                        (MIN_RECOVERY_DELAY..=MAX_DELAY).contains(&delay),
                        AccountError::InvalidParams
                    );
                }
                acc.recovery = recovery;
                acc.recovery_delay = delay;
            }
            Governance::SetMembers { members } => {
                require!(members.len() <= MAX_MEMBERS, AccountError::InvalidParams);
                acc.members = [Pubkey::default(); MAX_MEMBERS];
                for (i, m) in members.iter().enumerate() {
                    acc.members[i] = *m;
                }
                acc.member_count = members.len() as u8;
                validate_stack(&acc.stack, acc.member_count)
                    .map_err(|_| error!(AccountError::InvalidStack))?;
            }
            Governance::SetPolicyDelay { delay } => {
                require!(
                    (0..=MAX_DELAY).contains(&delay),
                    AccountError::InvalidParams
                );
                acc.policy_delay = delay;
            }
            Governance::Unlock => acc.locked = 0,
        }
        acc.policy_epoch += 1;
        acc.sequence += 1;
        emit!(Governed {
            account: a.account.key(),
            authority,
            epoch: acc.policy_epoch
        });
        Ok(())
    }

    /// Owner, members and the recovery authority may propose. A member proposer approves.
    pub fn propose(ctx: Context<Propose>, action_hash: [u8; 32]) -> Result<()> {
        let mut acct = ctx.accounts.account.load_mut()?;
        let proposer = ctx.accounts.proposer.key();
        let member = acct.member_index(&proposer);
        require!(
            proposer == acct.owner
                || member.is_some()
                || (acct.has_recovery() && proposer == acct.recovery),
            AccountError::Unauthorized
        );
        let p = &mut ctx.accounts.proposal;
        p.account = ctx.accounts.account.key();
        p.index = acct.proposal_count;
        p.proposer = proposer;
        p.action_hash = action_hash;
        p.created_at = Clock::get()?.unix_timestamp;
        p.epoch = acct.policy_epoch;
        p.approvals = member.map(|i| 1u8 << i).unwrap_or(0);
        p.status = PROPOSAL_PENDING;
        p.rent_payer = proposer;
        p.bump = ctx.bumps.proposal;
        acct.proposal_count += 1;
        emit!(Proposed {
            account: p.account,
            proposal: p.key(),
            proposer,
            action_hash
        });
        Ok(())
    }

    pub fn approve(ctx: Context<Approve>) -> Result<()> {
        let acct = ctx.accounts.account.load()?;
        let p = &mut ctx.accounts.proposal;
        require!(p.epoch == acct.policy_epoch, AccountError::StaleProposal);
        let i = acct
            .member_index(&ctx.accounts.member.key())
            .ok_or(error!(AccountError::Unauthorized))?;
        p.approvals |= 1 << i;
        Ok(())
    }

    /// The owner or the proposer may cancel; rent returns to the proposer.
    pub fn cancel(ctx: Context<Cancel>) -> Result<()> {
        let acct = ctx.accounts.account.load()?;
        let s = ctx.accounts.signer.key();
        require!(
            s == acct.owner || s == ctx.accounts.proposal.proposer,
            AccountError::Unauthorized
        );
        Ok(())
    }

    pub fn create_session(ctx: Context<CreateSession>, p: SessionParams) -> Result<()> {
        let acct = ctx.accounts.account.load()?;
        require_keys_eq!(
            ctx.accounts.owner.key(),
            acct.owner,
            AccountError::Unauthorized
        );
        require!(acct.locked == 0, AccountError::Locked);
        let now = Clock::get()?.unix_timestamp;
        require!(
            p.expires_at > now && p.expires_at <= now + MAX_SESSION_SECS,
            AccountError::InvalidParams
        );
        require!(
            p.ops != 0 && p.ops & !(SESSION_WITHDRAW | SESSION_EXECUTE) == 0,
            AccountError::InvalidParams
        );
        require!(
            p.ops & SESSION_EXECUTE == 0
                || (p.target_program != Pubkey::default() && p.target_program != crate::ID),
            AccountError::InvalidParams
        );
        let s = &mut ctx.accounts.session;
        s.account = ctx.accounts.account.key();
        s.key = p.key;
        s.expires_at = p.expires_at;
        s.ops = p.ops;
        s.asset = p.asset;
        s.max_amount = p.max_amount;
        s.target_program = p.target_program;
        s.epoch = acct.policy_epoch;
        s.bump = ctx.bumps.session;
        Ok(())
    }

    pub fn revoke_session(ctx: Context<RevokeSession>) -> Result<()> {
        let acct = ctx.accounts.account.load()?;
        require_keys_eq!(
            ctx.accounts.owner.key(),
            acct.owner,
            AccountError::Unauthorized
        );
        Ok(())
    }

    /// Emergency lock by the owner or the recovery authority. Blocks withdraw, execute and new
    /// sessions, and voids every pending proposal and session (epoch bump). Deposits continue.
    pub fn lock(ctx: Context<Lock>) -> Result<()> {
        let mut acct = ctx.accounts.account.load_mut()?;
        let s = ctx.accounts.authority.key();
        require!(
            s == acct.owner || (acct.has_recovery() && s == acct.recovery),
            AccountError::Unauthorized
        );
        acct.locked = 1;
        acct.policy_epoch += 1;
        emit!(Locked {
            account: ctx.accounts.account.key(),
            by: s
        });
        Ok(())
    }
}

// ---------------------------------------------------------------- helpers

/// Heap copy of a large Pod value (SBF stack frames are 4 KiB; a stack is 1.7 KiB and an
/// account 2.1 KiB).
fn boxed<T: bytemuck::Pod>(v: &T) -> Box<T> {
    let mut b: Box<T> = bytemuck::zeroed_box();
    bytemuck::bytes_of_mut(&mut *b).copy_from_slice(bytemuck::bytes_of(v));
    b
}

fn parse_stack(bytes: &[u8]) -> Result<Box<HookStack>> {
    require!(bytes.len() == STACK_LEN, AccountError::InvalidStack);
    let mut s: Box<HookStack> = bytemuck::zeroed_box();
    bytemuck::bytes_of_mut(&mut *s).copy_from_slice(bytes);
    require!(
        s.environment == Environment::Account as u8,
        AccountError::InvalidStack
    );
    Ok(s)
}

/// Validates `new`, checks each entry against its registry version, and carries builtin state
/// over from `old` for unchanged entries.
fn check_install<'info>(
    old: &HookStack,
    new: &mut HookStack,
    members: u8,
    remaining: &'info [AccountInfo<'info>],
) -> Result<()> {
    if let Err(e) = validate_stack(new, members) {
        msg!("invalid stack: {:?}", e);
        return err!(AccountError::InvalidStack);
    }
    for e in new.active() {
        let key = Pubkey::new_from_array(e.hook_version);
        let info = remaining
            .iter()
            .find(|i| *i.key == key)
            .ok_or(error!(AccountError::MissingVersion))?;
        let version: Account<hook_registry::HookVersion> = Account::try_from(info)?;
        hook_registry::verify_install(&key, &version, e, Environment::Account)?;
    }
    carry_state(old, new);
    Ok(())
}

fn snapshot(
    loader: &AccountLoader<SmartAccount>,
) -> Result<(Box<HookStack>, u64, Box<SmartAccount>)> {
    let a = loader.load()?;
    Ok((boxed(&a.stack), a.sequence, boxed(&*a)))
}

fn commit(loader: &AccountLoader<SmartAccount>, stack: &HookStack) -> Result<()> {
    let mut a = loader.load_mut()?;
    a.stack = *stack;
    a.sequence += 1;
    Ok(())
}

/// Copies state-record headers out of the remaining accounts (copies, so no borrow is held
/// across hook CPIs).
fn records(remaining: &[AccountInfo]) -> Vec<(Pubkey, Vec<u8>)> {
    remaining
        .iter()
        .filter(|i| !i.executable && *i.owner != system_program::ID)
        .filter_map(|i| {
            let d = i.try_borrow_data().ok()?;
            (d.len() >= HEADER_LEN && d[..4] == hook_standard::state::MAGIC)
                .then(|| (*i.owner, d[..HEADER_LEN].to_vec()))
        })
        .collect()
}

fn run<'info>(
    stack: &mut HookStack,
    cb: u8,
    op: &Op,
    subject: &AccountInfo<'info>,
    remaining: &[AccountInfo<'info>],
) -> Result<()> {
    let mut hooks = CpiPolicyHooks {
        environment: Environment::Account,
        environment_program: &crate::ID,
        subject,
        accounts: remaining,
        op_data: &[],
    };
    hook_kernel::evaluate(stack, cb, op, &mut hooks).map_err(|r| {
        msg!(
            "hook stack rejected: callback {} entry {} reason {} ({}) hook_reason {}",
            cb,
            r.index,
            r.reason,
            reason::describe(r.reason),
            r.hook_reason
        );
        emit!(PolicyRejected {
            account: op.subject,
            callback: cb,
            entry: r.index,
            reason: r.reason,
            hook_reason: r.hook_reason
        });
        error!(AccountError::PolicyRejected)
    })?;
    Ok(())
}

struct Auth {
    approvals: u8,
    proposal_created_at: Option<i64>,
    via_proposal: bool,
}

/// Marks the proposal executed before anything moves (consume-before-execute).
fn consume(proposal: &mut Option<Account<Proposal>>) {
    if let Some(p) = proposal.as_mut() {
        p.status = PROPOSAL_EXECUTED;
    }
}

pub const PROPOSAL_EXECUTED: u8 = 1;

/// Approvals = members who signed this transaction, plus (for proposals) members who approved.
fn authorize(
    acct: &SmartAccount,
    authority: &Pubkey,
    proposal: Option<&Proposal>,
    action: &Action,
    remaining: &[AccountInfo],
    _now: i64,
) -> Result<Auth> {
    let mut approvals = 0u8;
    for (i, m) in acct.members[..acct.member_count as usize]
        .iter()
        .enumerate()
    {
        if m == authority || remaining.iter().any(|r| r.is_signer && r.key == m) {
            approvals |= 1 << i;
        }
    }
    let Some(p) = proposal else {
        return Ok(Auth {
            approvals,
            proposal_created_at: None,
            via_proposal: false,
        });
    };
    require!(p.status == PROPOSAL_PENDING, AccountError::StaleProposal);
    require!(p.epoch == acct.policy_epoch, AccountError::StaleProposal);
    require!(
        p.action_hash == action_hash(action),
        AccountError::ActionMismatch
    );
    let executor_ok = *authority == acct.owner
        || acct.member_index(authority).is_some()
        || (acct.has_recovery() && *authority == acct.recovery);
    require!(executor_ok, AccountError::Unauthorized);
    Ok(Auth {
        approvals: approvals | p.approvals,
        proposal_created_at: Some(p.created_at),
        via_proposal: true,
    })
}

fn check_session<'a>(
    account: &Pubkey,
    acct: &SmartAccount,
    session: Option<&'a Session>,
    authority: &Pubkey,
    now: i64,
    op: u8,
) -> Result<&'a Session> {
    let s = session.ok_or(error!(AccountError::Unauthorized))?;
    require_keys_eq!(s.account, *account, AccountError::Unauthorized);
    require_keys_eq!(s.key, *authority, AccountError::Unauthorized);
    require!(now < s.expires_at, AccountError::SessionExpired);
    require!(s.epoch == acct.policy_epoch, AccountError::SessionExpired);
    require!(s.ops & op != 0, AccountError::SessionScope);
    Ok(s)
}

fn system_program_transfer<'info>(
    from: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    sp: &Program<'info, System>,
    amount: u64,
    seeds: Option<&[&[u8]]>,
) -> Result<()> {
    let accts = anchor_lang::system_program::Transfer {
        from: from.clone(),
        to: to.clone(),
    };
    match seeds {
        Some(s) => anchor_lang::system_program::transfer(
            CpiContext::new_with_signer(sp.to_account_info(), accts, &[s]),
            amount,
        ),
        None => anchor_lang::system_program::transfer(
            CpiContext::new(sp.to_account_info(), accts),
            amount,
        ),
    }
}

/// A vault-owned token account, before a CPI.
struct TokenSnapshot {
    key: Pubkey,
    program: Pubkey,
    mint: Pubkey,
    amount: u64,
    owner_delegate_close: [u8; 32 + 36 + 36],
}

const TOKEN_ACCOUNT_LEN: usize = 165;

impl TokenSnapshot {
    fn fields(d: &[u8]) -> [u8; 104] {
        let mut f = [0u8; 104];
        f[..32].copy_from_slice(&d[32..64]);
        f[32..68].copy_from_slice(&d[72..108]);
        f[68..104].copy_from_slice(&d[129..165]);
        f
    }

    fn take(info: &AccountInfo, vault: &Pubkey) -> Option<Self> {
        let program = *info.owner;
        if program != anchor_spl::token::ID && program != anchor_spl::token_2022::ID {
            return None;
        }
        let d = info.try_borrow_data().ok()?;
        // Token-2022 accounts with extensions carry account type 2 at byte 165 (mints carry 1).
        if d.len() < TOKEN_ACCOUNT_LEN
            || &d[32..64] != vault.as_ref()
            || (d.len() > TOKEN_ACCOUNT_LEN && d[TOKEN_ACCOUNT_LEN] != 2)
        {
            return None;
        }
        Some(TokenSnapshot {
            key: *info.key,
            program,
            mint: Pubkey::try_from(&d[..32]).unwrap(),
            amount: u64::from_le_bytes(d[64..72].try_into().unwrap()),
            owner_delegate_close: Self::fields(&d),
        })
    }

    /// Returns the amount spent, or fails if the account was closed or re-authorized.
    fn verify_after(&self, info: &AccountInfo) -> Result<u64> {
        require!(
            *info.owner == self.program && info.lamports() > 0,
            AccountError::SideChannel
        );
        let d = info.try_borrow_data()?;
        require!(
            d.len() >= TOKEN_ACCOUNT_LEN && Pubkey::try_from(&d[..32]).unwrap() == self.mint,
            AccountError::SideChannel
        );
        require!(
            Self::fields(&d) == self.owner_delegate_close,
            AccountError::SideChannel
        );
        let amount = u64::from_le_bytes(d[64..72].try_into().unwrap());
        Ok(self.amount.saturating_sub(amount))
    }
}

// ---------------------------------------------------------------- accounts

#[derive(Accounts)]
#[instruction(p: CreateParams)]
pub struct CreateAccount<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(init, payer = payer, space = 8 + core::mem::size_of::<SmartAccount>(), seeds = [ACCOUNT_SEED, payer.key().as_ref(), &p.seed.to_le_bytes()], bump)]
    pub account: AccountLoader<'info, SmartAccount>,
    /// CHECK: system-owned PDA holding SOL; signs only inside this program.
    #[account(seeds = [VAULT_SEED, account.key().as_ref()], bump)]
    pub vault: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub depositor: Signer<'info>,
    #[account(mut)]
    pub account: AccountLoader<'info, SmartAccount>,
    /// CHECK: vault PDA.
    #[account(mut, seeds = [VAULT_SEED, account.key().as_ref()], bump)]
    pub vault: UncheckedAccount<'info>,
    pub mint: Option<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub depositor_token: Option<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut)]
    pub vault_token: Option<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Option<Interface<'info, TokenInterface>>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    pub authority: Signer<'info>,
    #[account(mut)]
    pub account: AccountLoader<'info, SmartAccount>,
    /// CHECK: vault PDA.
    #[account(mut, seeds = [VAULT_SEED, account.key().as_ref()], bump)]
    pub vault: UncheckedAccount<'info>,
    /// CHECK: SOL recipient or destination token account (validated by the token program).
    #[account(mut)]
    pub destination: UncheckedAccount<'info>,
    pub session: Option<Account<'info, Session>>,
    #[account(mut, has_one = account @ AccountError::Unauthorized)]
    pub proposal: Option<Account<'info, Proposal>>,
    pub mint: Option<InterfaceAccount<'info, Mint>>,
    #[account(mut)]
    pub vault_token: Option<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Option<Interface<'info, TokenInterface>>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Execute<'info> {
    pub authority: Signer<'info>,
    #[account(mut)]
    pub account: AccountLoader<'info, SmartAccount>,
    /// CHECK: vault PDA.
    #[account(mut, seeds = [VAULT_SEED, account.key().as_ref()], bump)]
    pub vault: UncheckedAccount<'info>,
    pub session: Option<Account<'info, Session>>,
    #[account(mut, has_one = account @ AccountError::Unauthorized)]
    pub proposal: Option<Account<'info, Proposal>>,
}

#[derive(Accounts)]
pub struct Govern<'info> {
    pub authority: Signer<'info>,
    #[account(mut)]
    pub account: AccountLoader<'info, SmartAccount>,
    #[account(mut, has_one = account @ AccountError::Unauthorized)]
    pub proposal: Option<Account<'info, Proposal>>,
}

#[derive(Accounts)]
pub struct Propose<'info> {
    #[account(mut)]
    pub proposer: Signer<'info>,
    #[account(mut)]
    pub account: AccountLoader<'info, SmartAccount>,
    #[account(
        init,
        payer = proposer,
        space = 8 + Proposal::INIT_SPACE,
        seeds = [PROPOSAL_SEED, account.key().as_ref(), &account.load()?.proposal_count.to_le_bytes()],
        bump
    )]
    pub proposal: Account<'info, Proposal>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Approve<'info> {
    pub member: Signer<'info>,
    pub account: AccountLoader<'info, SmartAccount>,
    #[account(mut, has_one = account @ AccountError::Unauthorized, constraint = proposal.status == PROPOSAL_PENDING @ AccountError::StaleProposal)]
    pub proposal: Account<'info, Proposal>,
}

#[derive(Accounts)]
pub struct Cancel<'info> {
    pub signer: Signer<'info>,
    pub account: AccountLoader<'info, SmartAccount>,
    #[account(mut, has_one = account @ AccountError::Unauthorized, has_one = rent_payer @ AccountError::Unauthorized, close = rent_payer)]
    pub proposal: Account<'info, Proposal>,
    /// CHECK: receives the proposal's rent (address checked by `has_one`).
    #[account(mut)]
    pub rent_payer: UncheckedAccount<'info>,
}

#[derive(Accounts)]
#[instruction(p: SessionParams)]
pub struct CreateSession<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    pub account: AccountLoader<'info, SmartAccount>,
    #[account(init, payer = owner, space = 8 + Session::INIT_SPACE, seeds = [SESSION_SEED, account.key().as_ref(), p.key.as_ref()], bump)]
    pub session: Account<'info, Session>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RevokeSession<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    pub account: AccountLoader<'info, SmartAccount>,
    #[account(mut, has_one = account @ AccountError::Unauthorized, close = owner)]
    pub session: Account<'info, Session>,
}

#[derive(Accounts)]
pub struct Lock<'info> {
    pub authority: Signer<'info>,
    #[account(mut)]
    pub account: AccountLoader<'info, SmartAccount>,
}

// ---------------------------------------------------------------- events & errors

#[event]
pub struct AccountCreated {
    pub account: Pubkey,
    pub owner: Pubkey,
    pub vault: Pubkey,
}
#[event]
pub struct Deposited {
    pub account: Pubkey,
    pub depositor: Pubkey,
    pub asset: Pubkey,
    pub amount: u64,
}
#[event]
pub struct Withdrawn {
    pub account: Pubkey,
    pub authority: Pubkey,
    pub asset: Pubkey,
    pub amount: u64,
    pub destination: Pubkey,
}
#[event]
pub struct Executed {
    pub account: Pubkey,
    pub authority: Pubkey,
    pub program: Pubkey,
    pub outflows: u8,
}
#[event]
pub struct Governed {
    pub account: Pubkey,
    pub authority: Pubkey,
    pub epoch: u64,
}
#[event]
pub struct Proposed {
    pub account: Pubkey,
    pub proposal: Pubkey,
    pub proposer: Pubkey,
    pub action_hash: [u8; 32],
}
#[event]
pub struct Locked {
    pub account: Pubkey,
    pub by: Pubkey,
}
#[event]
pub struct PolicyRejected {
    pub account: Pubkey,
    pub callback: u8,
    pub entry: u8,
    pub reason: u16,
    pub hook_reason: u16,
}

#[error_code]
pub enum AccountError {
    #[msg("Signer is not authorized")]
    Unauthorized,
    #[msg("Invalid parameters")]
    InvalidParams,
    #[msg("Invalid hook stack")]
    InvalidStack,
    #[msg("Registry version account missing for a stack entry")]
    MissingVersion,
    #[msg("Hook stack rejected the operation")]
    PolicyRejected,
    #[msg("Account is locked")]
    Locked,
    #[msg("This change requires an approved proposal")]
    ProposalRequired,
    #[msg("Timelock has not elapsed")]
    TimelockPending,
    #[msg("Proposal is stale, executed, or from an older policy epoch")]
    StaleProposal,
    #[msg("Action does not match the proposal")]
    ActionMismatch,
    #[msg("Session expired or revoked")]
    SessionExpired,
    #[msg("Operation outside the session's scope")]
    SessionScope,
    #[msg("Token account is not owned by the vault")]
    NotVaultTokenAccount,
    #[msg("Accounts do not match the instruction")]
    AccountMismatch,
    #[msg("Smart-account state may not be passed to an executed program")]
    ProtectedAccount,
    #[msg("Executed program changed vault ownership, delegate, close authority, or closed a tracked account")]
    SideChannel,
    #[msg("Arithmetic overflow")]
    Overflow,
}
