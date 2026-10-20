use anchor_lang::prelude::*;
use hook_types::{HookSlot, MarketStats, UserState, MAX_HOOK_SLOTS};
use policy_vm::PolicyData;

pub const GLOBAL_SEED: &[u8] = b"global";
pub const HOOK_SEED: &[u8] = b"hook";
pub const MARKET_SEED: &[u8] = b"market";
pub const USER_SEED: &[u8] = b"user";
pub const POLICY_SEED: &[u8] = b"policy";

pub const SLOTS_LEN: usize = core::mem::size_of::<HookSlot>() * MAX_HOOK_SLOTS;
pub const USER_STATE_LEN: usize = core::mem::size_of::<UserState>();
pub const POLICY_DATA_LEN: usize = core::mem::size_of::<PolicyData>();

#[account]
#[derive(InitSpace)]
pub struct GlobalConfig {
    pub authority: Pubkey,
    pub pending_authority: Pubkey,
    /// Kill switch: bit `1 << module_id` set means routed trades through markets with that module
    /// installed fail closed.
    pub disabled_modules: u64,
    pub market_count: u64,
    pub paused: bool,
    pub bump: u8,
}

/// Registry entry for a hook module. V1 registers first-party modules only.
#[account]
#[derive(InitSpace)]
pub struct HookDefinition {
    pub module_id: u16,
    pub version: u16,
    pub capabilities: u32,
    pub callbacks: u16,
    pub priority_min: u16,
    pub priority_max: u16,
    pub runtime: bool,
    pub enabled: bool,
    pub installs: u64,
    /// sha256 of the module source at the registered version.
    pub source_hash: [u8; 32],
    /// 0 = unaudited. Nothing in this repository sets a non-zero value.
    pub audit_status: u8,
    pub bump: u8,
}

pub mod market_flags {
    pub const CREATOR_VERIFIED: u32 = 1 << 0;
    pub const PAUSED: u32 = 1 << 1;
    /// Curve reported `complete` after a routed trade; `sync_market` switches the venue.
    pub const MIGRATION_PENDING: u32 = 1 << 2;
    pub const BASE_TOKEN_2022: u32 = 1 << 3;
    pub const HOLDER_REWARD_COIN: u32 = 1 << 4;
    pub const MAYHEM: u32 = 1 << 5;
}

/// A programmable Pump market: one existing Pump token plus the hook stack installed on it under
/// one namespace (`["market", mint, namespace]`).
#[account(zero_copy)]
#[repr(C)]
pub struct MarketConfig {
    /// Seed authority. Fixed at creation.
    pub namespace: Pubkey,
    pub authority: Pubkey,
    pub pending_authority: Pubkey,
    pub mint: Pubkey,
    pub bonding_curve: Pubkey,
    /// Canonical PumpSwap pool; default until the coin has migrated.
    pub pool: Pubkey,
    /// Resolved quote mint (wSOL for SOL markets).
    pub quote_mint: Pubkey,
    pub base_token_program: Pubkey,
    pub quote_token_program: Pubkey,
    pub strategy_vault: Pubkey,
    pub created_at: i64,
    pub activated_at: i64,
    pub routed_buy_quote: u64,
    pub routed_sell_quote: u64,
    pub trade_count: u64,
    pub sequence: u64,
    pub flags: u32,
    /// `hook_types::Venue`.
    pub venue: u8,
    pub bump: u8,
    /// Bit `i` set when `Policy[i]` exists.
    pub policy_mask: u8,
    pub _pad0: u8,
    /// `[HookSlot; MAX_HOOK_SLOTS]`, sorted by priority. Byte array so the account layout does
    /// not depend on Anchor's view of foreign types.
    pub slots: [u8; SLOTS_LEN],
    pub _reserved: [u8; 128],
}

const _: () = assert!(core::mem::size_of::<MarketConfig>() == 1176);

impl MarketConfig {
    pub const SIZE: usize = core::mem::size_of::<MarketConfig>();

    pub fn read_slots(&self) -> [HookSlot; MAX_HOOK_SLOTS] {
        bytemuck::pod_read_unaligned(&self.slots)
    }
    pub fn write_slots(&mut self, s: &[HookSlot; MAX_HOOK_SLOTS]) {
        self.slots.copy_from_slice(bytemuck::bytes_of(s));
    }
    pub fn stats(&self) -> MarketStats {
        MarketStats {
            created_at: self.created_at,
            routed_buy_quote: self.routed_buy_quote,
            routed_sell_quote: self.routed_sell_quote,
            trade_count: self.trade_count,
        }
    }
    pub fn set_stats(&mut self, s: &MarketStats) {
        self.routed_buy_quote = s.routed_buy_quote;
        self.routed_sell_quote = s.routed_sell_quote;
        self.trade_count = s.trade_count;
    }
    pub fn has_flag(&self, f: u32) -> bool {
        self.flags & f != 0
    }
}

/// Per (market, wallet) routed history. Created by the wallet's first routed trade.
#[account(zero_copy)]
#[repr(C)]
pub struct UserMarketState {
    pub market: Pubkey,
    pub user: Pubkey,
    pub created_at: i64,
    pub bump: u8,
    pub _pad: [u8; 7],
    pub state: [u8; USER_STATE_LEN],
}

impl UserMarketState {
    pub const SIZE: usize = core::mem::size_of::<UserMarketState>();
    pub fn read(&self) -> UserState {
        bytemuck::pod_read_unaligned(&self.state)
    }
    pub fn write(&mut self, u: &UserState) {
        self.state.copy_from_slice(bytemuck::bytes_of(u));
    }
}

/// A compiled Composer policy (`["policy", market, index]`).
#[account(zero_copy)]
#[repr(C)]
pub struct Policy {
    pub data: [u8; POLICY_DATA_LEN],
}

impl Policy {
    pub const SIZE: usize = POLICY_DATA_LEN;
    pub fn read(&self) -> PolicyData {
        bytemuck::pod_read_unaligned(&self.data)
    }
    pub fn write(&mut self, p: &PolicyData) {
        self.data.copy_from_slice(bytemuck::bytes_of(p));
    }
}
