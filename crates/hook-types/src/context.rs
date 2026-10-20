use bytemuck::{Pod, Zeroable};

use crate::callback::{HookCallback, Side, TradeMode, Venue};

pub mod context_flags {
    pub const QUOTE_NATIVE_SOL: u8 = 1 << 0;
    pub const BASE_TOKEN_2022: u8 = 1 << 1;
    pub const HOLDER_REWARD_COIN: u8 = 1 << 2;
    pub const MAYHEM: u8 = 1 << 3;
    pub const CREATOR_VERIFIED_MARKET: u8 = 1 << 4;
    pub const STRATEGY_EXECUTION: u8 = 1 << 5;
    pub const AMOUNTS_OBSERVED: u8 = 1 << 6;
}

/// Everything a hook may know about a routed trade. Built by the router from account data only.
/// 272 bytes, 8-byte aligned. Offsets are frozen (see `docs/hooks-spec-pump-router.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
#[repr(C)]
pub struct HookContext {
    pub version: u8,
    pub callback: u8,
    pub venue: u8,
    pub side: u8,
    pub mode: u8,
    pub flags: u8,
    pub slot_index: u8,
    pub _reserved: u8,
    pub market: [u8; 32],
    pub user: [u8; 32],
    pub base_mint: [u8; 32],
    pub quote_mint: [u8; 32],
    pub pool: [u8; 32],
    pub amount: u64,
    pub limit: u64,
    pub quote_amount: u64,
    pub base_amount: u64,
    pub pre_base_reserves: u64,
    pub pre_quote_reserves: u64,
    pub post_base_reserves: u64,
    pub post_quote_reserves: u64,
    pub timestamp: i64,
    pub slot: u64,
    pub sequence: u64,
    pub market_activated_at: i64,
    pub user_base_balance: u64,
}

const _: () = assert!(core::mem::size_of::<HookContext>() == 272);

impl HookContext {
    pub fn callback(&self) -> Option<HookCallback> {
        HookCallback::from_u8(self.callback)
    }
    pub fn side(&self) -> Option<Side> {
        Side::from_u8(self.side)
    }
    pub fn mode(&self) -> Option<TradeMode> {
        TradeMode::from_u8(self.mode)
    }
    pub fn venue(&self) -> Option<Venue> {
        Venue::from_u8(self.venue)
    }
    pub fn has_flag(&self, f: u8) -> bool {
        self.flags & f != 0
    }
    pub fn is_buy(&self) -> bool {
        self.side == Side::Buy as u8
    }
    pub fn seconds_since_activation(&self) -> u64 {
        self.timestamp.saturating_sub(self.market_activated_at).max(0) as u64
    }
    /// Spot price as Q64.64 quote-per-base from the given reserves. `None` if base is 0.
    pub fn spot_q64(base_reserves: u64, quote_reserves: u64) -> Option<u128> {
        if base_reserves == 0 {
            return None;
        }
        Some(((quote_reserves as u128) << 64) / base_reserves as u128)
    }
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
}

/// Market-level routed statistics exposed read-only to modules and policies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MarketStats {
    pub created_at: i64,
    pub routed_buy_quote: u64,
    pub routed_sell_quote: u64,
    pub trade_count: u64,
}

impl MarketStats {
    pub fn routed_volume(&self) -> u64 {
        self.routed_buy_quote.saturating_add(self.routed_sell_quote)
    }
    /// `buy * 10_000 / max(sell, 1)`, saturating at u64::MAX.
    pub fn buy_sell_ratio_bps(&self) -> u64 {
        let r = (self.routed_buy_quote as u128) * 10_000 / (self.routed_sell_quote.max(1) as u128);
        r.min(u64::MAX as u128) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::offset_of;

    #[test]
    fn frozen_offsets() {
        assert_eq!(offset_of!(HookContext, market), 8);
        assert_eq!(offset_of!(HookContext, user), 40);
        assert_eq!(offset_of!(HookContext, base_mint), 72);
        assert_eq!(offset_of!(HookContext, quote_mint), 104);
        assert_eq!(offset_of!(HookContext, pool), 136);
        assert_eq!(offset_of!(HookContext, amount), 168);
        assert_eq!(offset_of!(HookContext, limit), 176);
        assert_eq!(offset_of!(HookContext, quote_amount), 184);
        assert_eq!(offset_of!(HookContext, base_amount), 192);
        assert_eq!(offset_of!(HookContext, pre_base_reserves), 200);
        assert_eq!(offset_of!(HookContext, pre_quote_reserves), 208);
        assert_eq!(offset_of!(HookContext, post_base_reserves), 216);
        assert_eq!(offset_of!(HookContext, post_quote_reserves), 224);
        assert_eq!(offset_of!(HookContext, timestamp), 232);
        assert_eq!(offset_of!(HookContext, slot), 240);
        assert_eq!(offset_of!(HookContext, sequence), 248);
        assert_eq!(offset_of!(HookContext, market_activated_at), 256);
        assert_eq!(offset_of!(HookContext, user_base_balance), 264);
    }

    #[test]
    fn spot_price_handles_extremes() {
        assert_eq!(HookContext::spot_q64(0, 5), None);
        assert_eq!(HookContext::spot_q64(1, 1), Some(1u128 << 64));
        assert!(HookContext::spot_q64(1, u64::MAX).is_some());
    }

    #[test]
    fn ratio_never_divides_by_zero() {
        let s = MarketStats { routed_buy_quote: u64::MAX, routed_sell_quote: 0, ..Default::default() };
        assert_eq!(s.buy_sell_ratio_bps(), u64::MAX);
        let s = MarketStats { routed_buy_quote: 50, routed_sell_quote: 100, ..Default::default() };
        assert_eq!(s.buy_sell_ratio_bps(), 5_000);
    }
}
