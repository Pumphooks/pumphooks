use anchor_lang::prelude::*;

#[event]
pub struct ProtocolUpdated {
    pub admin: Pubkey,
    pub pending_admin: Pubkey,
    pub fee_authority: Pubkey,
    pub protocol_fee_bps: u16,
    pub paused: u8,
}

#[event]
pub struct HookRegistered {
    pub program: Pubkey,
    pub registrant: Pubkey,
    pub permissions: u32,
    pub upgrade_authority: Option<Pubkey>,
    pub deploy_slot: u64,
    pub metadata_uri: String,
}

#[event]
pub struct Attested {
    pub hook_program: Pubkey,
    pub attester: Pubkey,
    pub kind: u8,
    pub deploy_slot: u64,
    pub content_hash: [u8; 32],
    pub uri: String,
    pub revoked: bool,
}

#[event]
pub struct PoolInitialized {
    pub pool: Pubkey,
    pub creator: Pubkey,
    pub mint_0: Pubkey,
    pub mint_1: Pubkey,
    pub hook_program: Pubkey,
    pub permissions: u32,
    pub fee_ppm: u32,
    pub max_fee_ppm: u32,
    pub salt: u64,
    pub pinned: bool,
}

#[event]
pub struct HookCalled {
    pub pool: Pubkey,
    pub callback: u8,
    pub decision: u8,
    pub reason: u16,
    pub fee_override: Option<u32>,
    pub delta_0: i64,
    pub delta_1: i64,
    pub quote: u64,
}

#[event]
pub struct Swapped {
    pub pool: Pubkey,
    pub trader: Pubkey,
    pub zero_for_one: bool,
    pub exact_in: bool,
    /// True for hook-initiated swaps against the hook's credit (no callbacks ran).
    pub from_credit: bool,
    pub amount_in: u64,
    pub amount_out: u64,
    pub curve_in: u64,
    pub curve_out: u64,
    pub fee: u64,
    pub protocol_fee: u64,
    pub fee_ppm: u32,
    pub delta_specified: i64,
    pub delta_unspecified: i64,
    pub reserve_0: u64,
    pub reserve_1: u64,
    pub sequence: u64,
}

#[event]
pub struct LiquidityChanged {
    pub pool: Pubkey,
    pub owner: Pubkey,
    pub added: bool,
    pub amount_0: u64,
    pub amount_1: u64,
    pub shares: u64,
    pub hook_delta_0: i64,
    pub hook_delta_1: i64,
    pub reserve_0: u64,
    pub reserve_1: u64,
    pub lp_supply: u64,
    pub sequence: u64,
}

#[event]
pub struct Donated {
    pub pool: Pubkey,
    pub donor: Pubkey,
    pub from_credit: bool,
    pub amount_0: u64,
    pub amount_1: u64,
    pub sequence: u64,
}

#[event]
pub struct CreditMoved {
    pub pool: Pubkey,
    pub actor: Pubkey,
    pub deposit: bool,
    pub amount_0: u64,
    pub amount_1: u64,
    pub credit_0: u64,
    pub credit_1: u64,
}

#[event]
pub struct FeeChanged {
    pub pool: Pubkey,
    pub fee_ppm: u32,
}

#[event]
pub struct ProtocolFeesWithdrawn {
    pub pool: Pubkey,
    pub amount_0: u64,
    pub amount_1: u64,
}
