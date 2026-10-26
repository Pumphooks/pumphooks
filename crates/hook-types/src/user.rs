use bytemuck::{Pod, Zeroable};

/// Per-wallet, per-market state shared by first-party modules. Written only by the router during
/// a successful routed trade. 160 bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
#[repr(C)]
pub struct UserState {
    /// Vested Holder XP.
    pub xp: u64,
    /// Buy XP waiting for its hold period; forfeited by a routed sell before `pending_vest_at`.
    pub pending_xp: u64,
    pub pending_vest_at: i64,
    /// `unix_timestamp / 86_400` of the last XP award, for the daily cap.
    pub xp_day: u32,
    pub xp_today: u32,
    /// Points from Composer `AWARD_POINTS` actions (separate ledger from XP).
    pub policy_points: u64,

    pub routed_buy_quote: u64,
    pub routed_sell_quote: u64,
    pub trade_count: u32,
    pub last_side: u8,
    pub _pad0: [u8; 3],

    pub last_trade_ts: i64,
    pub last_trade_slot: u64,
    /// First routed buy of the current holding period (reset when the wallet sells to zero).
    pub first_buy_ts: i64,

    /// Holding-XP checkpoint: accrual uses `min(balance_then, balance_now) × elapsed`.
    pub hold_checkpoint_ts: i64,
    pub hold_checkpoint_balance: u64,

    /// Volume Guard per-wallet rolling window.
    pub vg_window_start: i64,
    pub vg_window_volume: u64,

    pub forfeited_xp: u64,
    /// Last routed sell (0 = never). Set by the router; read by Sell Limit's cooldown.
    pub last_sell_ts: i64,
    pub _reserved: [u8; 24],
}

const _: () = assert!(core::mem::size_of::<UserState>() == 160);

impl UserState {
    pub fn routed_volume(&self) -> u64 {
        self.routed_buy_quote.saturating_add(self.routed_sell_quote)
    }
    pub fn holder_duration(&self, now: i64) -> u64 {
        if self.first_buy_ts == 0 {
            0
        } else {
            now.saturating_sub(self.first_buy_ts).max(0) as u64
        }
    }
}
