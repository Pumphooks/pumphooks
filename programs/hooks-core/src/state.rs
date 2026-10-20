use anchor_lang::prelude::*;

pub const PROTOCOL_SEED: &[u8] = b"protocol";
pub const POOL_SEED: &[u8] = b"pool";
pub const VAULT_SEED: &[u8] = b"vault";
pub const LP_SEED: &[u8] = b"lp";
pub const POLICY_SEED: &[u8] = b"pool-policy";

pub const MAX_PROTOCOL_FEE_BPS: u16 = 2_500;
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
    /// `hook-registry` HookVersion the hook was installed from; default for a plain pool.
    pub hook_version: Pubkey,
    /// `PoolPolicy` account, or default when the pool has no builtin policy stack.
    pub policy: Pubkey,
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

/// Builtin-only ABI v1 hook stack evaluated by `hook-kernel` around this pool's operations.
/// Attached before the first deposit and immutable afterwards.
#[account(zero_copy)]
#[repr(C)]
pub struct PoolPolicy {
    pub pool: Pubkey,
    pub bump: u8,
    pub _pad: [u8; 7],
    pub stack: hook_standard::HookStack,
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
