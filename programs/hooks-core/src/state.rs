use anchor_lang::prelude::*;

pub const PROTOCOL_SEED: &[u8] = b"protocol";
pub const HOOK_SEED: &[u8] = b"hook";
pub const ATTEST_SEED: &[u8] = b"attest";
pub const POOL_SEED: &[u8] = b"pool";
pub const VAULT_SEED: &[u8] = b"vault";
pub const LP_SEED: &[u8] = b"lp";

pub const MAX_PROTOCOL_FEE_BPS: u16 = 2_500;
pub const MAX_URI_LEN: usize = 200;
pub const LP_DECIMALS: u8 = 9;

pub const PAUSE_POOLS: u8 = 1 << 0;
pub const PAUSE_SWAPS: u8 = 1 << 1;
pub const PAUSE_ADDS: u8 = 1 << 2;

pub const POOL_HOOK_PINNED: u8 = 1 << 0;

#[account]
#[derive(InitSpace)]
pub struct Protocol {
    pub admin: Pubkey,
    pub pending_admin: Pubkey,
    pub fee_authority: Pubkey,
    /// Share of every LP fee taken by the protocol, in bps of the fee. 0 at launch.
    pub protocol_fee_bps: u16,
    pub paused: u8,
    pub bump: u8,
    pub pool_count: u64,
    pub hook_count: u64,
}

#[account]
#[derive(InitSpace)]
pub struct HookRegistration {
    pub program: Pubkey,
    pub registrant: Pubkey,
    /// Upgrade authority observed at registration (or last update); `None` = immutable.
    pub upgrade_authority: Option<Pubkey>,
    /// Last-deploy slot observed at registration (or last update).
    pub deploy_slot: u64,
    pub permissions: u32,
    pub interface_version: u8,
    pub bump: u8,
    pub pool_count: u64,
    pub registered_at: i64,
    pub updated_at: i64,
    #[max_len(MAX_URI_LEN)]
    pub metadata_uri: String,
}

pub const ATTEST_AUDIT: u8 = 0;
pub const ATTEST_REVIEW: u8 = 1;
pub const ATTEST_VERIFIED_BUILD: u8 = 2;
pub const ATTEST_WARNING: u8 = 3;

/// A statement about a hook, signed by whoever makes it. The protocol does not judge attesters;
/// clients decide whom to trust.
#[account]
#[derive(InitSpace)]
pub struct Attestation {
    pub hook_program: Pubkey,
    pub attester: Pubkey,
    pub kind: u8,
    pub bump: u8,
    /// Deploy slot the statement refers to; it goes stale when the hook is redeployed.
    pub deploy_slot: u64,
    pub content_hash: [u8; 32],
    pub created_at: i64,
    #[max_len(MAX_URI_LEN)]
    pub uri: String,
}

#[account]
#[derive(InitSpace)]
pub struct Pool {
    pub bump: u8,
    pub callback_bump: u8,
    pub flags: u8,
    pub _pad: u8,
    pub permissions: u32,
    pub fee_ppm: u32,
    pub max_fee_ppm: u32,
    pub salt: u64,
    pub mint_0: Pubkey,
    pub mint_1: Pubkey,
    pub vault_0: Pubkey,
    pub vault_1: Pubkey,
    pub token_program_0: Pubkey,
    pub token_program_1: Pubkey,
    pub lp_mint: Pubkey,
    /// `Pubkey::default()` for a plain pool.
    pub hook_program: Pubkey,
    pub creator: Pubkey,
    pub reserve_0: u64,
    pub reserve_1: u64,
    pub credit_0: u64,
    pub credit_1: u64,
    pub protocol_fee_0: u64,
    pub protocol_fee_1: u64,
    /// Includes the permanently locked minimum liquidity, which has no token.
    pub lp_supply: u64,
    pub sequence: u64,
    pub pinned_deploy_slot: u64,
    pub created_at: i64,
}

impl Pool {
    pub fn has_hook(&self) -> bool {
        self.hook_program != Pubkey::default()
    }

    pub fn signer_seeds(&self) -> [&[u8]; 6] {
        [
            POOL_SEED,
            self.mint_0.as_ref(),
            self.mint_1.as_ref(),
            self.hook_program.as_ref(),
            bytemuck::bytes_of(&self.salt),
            core::slice::from_ref(&self.bump),
        ]
    }

    pub fn book(&self) -> crate::engine::Book {
        crate::engine::Book {
            reserve: [self.reserve_0, self.reserve_1],
            credit: [self.credit_0, self.credit_1],
            protocol_fee: [self.protocol_fee_0, self.protocol_fee_1],
            lp_supply: self.lp_supply,
            fee_ppm: self.fee_ppm,
            max_fee_ppm: self.max_fee_ppm,
            permissions: self.permissions,
        }
    }

    pub fn store(&mut self, b: &crate::engine::Book) {
        [self.reserve_0, self.reserve_1] = b.reserve;
        [self.credit_0, self.credit_1] = b.credit;
        [self.protocol_fee_0, self.protocol_fee_1] = b.protocol_fee;
        self.lp_supply = b.lp_supply;
    }
}
