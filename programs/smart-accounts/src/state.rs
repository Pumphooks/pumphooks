use anchor_lang::prelude::*;
use hook_standard::HookStack;

pub const ACCOUNT_SEED: &[u8] = b"smart-account";
pub const VAULT_SEED: &[u8] = b"vault";
pub const PROPOSAL_SEED: &[u8] = b"proposal";
pub const SESSION_SEED: &[u8] = b"session";
pub const MAX_MEMBERS: usize = 8;
/// Recovery must wait at least this long, so the owner can cancel a hostile recovery.
pub const MIN_RECOVERY_DELAY: i64 = 86_400;
pub const MAX_DELAY: i64 = 90 * 86_400;
pub const MAX_SESSION_SECS: i64 = 7 * 86_400;
pub const MAX_CPI_ACCOUNTS: usize = 24;
pub const MAX_CPI_DATA: usize = 512;

#[account(zero_copy)]
#[repr(C)]
pub struct SmartAccount {
    pub owner: Pubkey,
    /// `Pubkey::default()` = no recovery authority.
    pub recovery: Pubkey,
    pub creator: Pubkey,
    /// Approvers for multisig policies (bit i of `approvals` = `members[i]`).
    pub members: [Pubkey; MAX_MEMBERS],
    pub seed: u64,
    pub sequence: u64,
    /// Bumped on every governance change and lock; proposals and sessions from an older epoch
    /// are void.
    pub policy_epoch: u64,
    pub proposal_count: u64,
    pub recovery_delay: i64,
    /// Minimum age of a proposal that loosens policy (removes/changes a mandatory hook, changes
    /// owner, recovery or members, shortens this delay, or unlocks without the recovery's
    /// co-signature).
    pub policy_delay: i64,
    pub member_count: u8,
    pub locked: u8,
    pub bump: u8,
    pub vault_bump: u8,
    pub _pad: [u8; 4],
    pub stack: HookStack,
}

impl SmartAccount {
    pub fn has_recovery(&self) -> bool {
        self.recovery != Pubkey::default()
    }
    pub fn member_index(&self, key: &Pubkey) -> Option<usize> {
        self.members[..self.member_count as usize]
            .iter()
            .position(|m| m == key)
    }
}

pub const PROPOSAL_PENDING: u8 = 0;

#[account]
#[derive(InitSpace)]
pub struct Proposal {
    pub account: Pubkey,
    pub index: u64,
    pub proposer: Pubkey,
    /// sha256 of the Borsh-encoded [`crate::Action`].
    pub action_hash: [u8; 32],
    pub created_at: i64,
    pub epoch: u64,
    pub approvals: u8,
    pub status: u8,
    pub rent_payer: Pubkey,
    pub bump: u8,
}

pub const SESSION_WITHDRAW: u8 = 1;
pub const SESSION_EXECUTE: u8 = 2;

#[account]
#[derive(InitSpace)]
pub struct Session {
    pub account: Pubkey,
    pub key: Pubkey,
    pub expires_at: i64,
    pub ops: u8,
    /// `Pubkey::default()` = native SOL. Withdraw sessions are limited to this asset.
    pub asset: Pubkey,
    pub max_amount: u64,
    /// Execute sessions may only call this program.
    pub target_program: Pubkey,
    pub epoch: u64,
    pub bump: u8,
}
