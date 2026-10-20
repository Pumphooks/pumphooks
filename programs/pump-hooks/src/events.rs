use anchor_lang::prelude::*;

#[event]
pub struct MarketCreated {
    pub market: Pubkey,
    pub mint: Pubkey,
    pub authority: Pubkey,
    pub venue: u8,
    pub creator_verified: bool,
    pub bonding_curve: Pubkey,
    pub pool: Pubkey,
    pub quote_mint: Pubkey,
}

#[event]
pub struct MarketSynced {
    pub market: Pubkey,
    pub venue: u8,
    pub pool: Pubkey,
}

#[event]
pub struct MarketPausedSet {
    pub market: Pubkey,
    pub paused: bool,
}

#[event]
pub struct HookInstalled {
    pub market: Pubkey,
    pub module_id: u16,
    pub version: u16,
    pub slot_index: u8,
    pub priority: u16,
    pub callbacks: u16,
    pub capabilities: u32,
    pub config: [u8; 48],
}

#[event]
pub struct HookRemoved {
    pub market: Pubkey,
    pub module_id: u16,
}

#[event]
pub struct HookConfigured {
    pub market: Pubkey,
    pub module_id: u16,
    pub enabled: bool,
    pub config: [u8; 48],
}

#[event]
pub struct PolicyUpdated {
    pub market: Pubkey,
    pub index: u8,
    pub trigger: u8,
    pub n_conditions: u8,
    pub n_actions: u8,
    pub enabled: bool,
    pub closed: bool,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug)]
pub struct TraceItem {
    pub callback: u8,
    pub slot_index: u8,
    pub module_id: u16,
    pub allowed: bool,
    pub reason_code: u16,
    pub flags: u32,
}

/// One entry per hook callback that ran, in execution order.
#[event]
pub struct HookExecutionEvent {
    pub market: Pubkey,
    pub user: Pubkey,
    pub sequence: u64,
    pub entries: Vec<TraceItem>,
}

/// Emitted when a before-callback denies; the transaction then fails, so this event appears only
/// in the failed transaction's logs.
#[event]
pub struct HookDeniedEvent {
    pub market: Pubkey,
    pub user: Pubkey,
    pub sequence: u64,
    pub module_id: u16,
    pub callback: u8,
    pub slot_index: u8,
    pub reason_code: u16,
}

#[event]
pub struct RoutedSwapEvent {
    pub market: Pubkey,
    pub user: Pubkey,
    pub mint: Pubkey,
    pub venue: u8,
    pub side: u8,
    pub mode: u8,
    pub amount: u64,
    pub limit: u64,
    /// Observed from the user's balances.
    pub quote_amount: u64,
    pub base_amount: u64,
    pub pre_base_reserves: u64,
    pub pre_quote_reserves: u64,
    pub post_base_reserves: u64,
    pub post_quote_reserves: u64,
    pub sequence: u64,
    pub slot: u64,
    pub timestamp: i64,
    pub hooks_run: u8,
    pub xp: u64,
    pub pending_xp: u64,
    pub policy_points: u64,
}

#[event]
pub struct PolicyExecutionEvent {
    pub market: Pubkey,
    pub sequence: u64,
    pub policy_index: u8,
    pub trigger: u8,
    pub matched: bool,
    /// 0 allow, 1 deny, 2 record, 3 signal/intent.
    pub result: u8,
    pub reason_code: u16,
    pub tag: u64,
}

#[event]
pub struct StrategySignalApplied {
    pub market: Pubkey,
    pub signer: Pubkey,
    pub nonce: u64,
    pub action: u8,
    pub param: u8,
    pub valid_until: i64,
    pub confidence_bps: u16,
}

#[event]
pub struct KillSwitchSet {
    pub disabled_modules: u64,
    pub paused: bool,
}
