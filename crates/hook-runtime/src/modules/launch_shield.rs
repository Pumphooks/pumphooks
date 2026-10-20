//! Launch Shield: limits on routed execution during an activation window.
//!
//! Applies to routed trades only. A transaction that trades on Pump directly is not affected.

use hook_types::reason::launch_shield as r;
use hook_types::slot::le;
use hook_types::{decision_flags, HookCallback, HookDecision, ModuleId, Side};

use super::{bad_config, HookEnv, PumpHook};
use crate::executor::HookError;

pub const FLAG_DENY_SAME_SLOT_ROUND_TRIP: u16 = 1 << 0;
pub const FLAG_COOLDOWN_SELLS: u16 = 1 << 1;
/// Longest window a shield may run: 30 days.
pub const MAX_DURATION_SECS: u32 = 30 * 86_400;
pub const MAX_COOLDOWN_SECS: u32 = 86_400;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// 0 = start at market activation.
    pub start_ts: i64,
    pub duration_secs: u32,
    pub cooldown_secs: u32,
    /// 0 = no limit.
    pub max_buy_quote: u64,
    pub max_sell_quote: u64,
    /// 0 = no limit.
    pub max_buys_per_slot: u16,
    pub flags: u16,
}

impl Config {
    pub fn decode(b: &[u8; 48]) -> Self {
        Self {
            start_ts: le::i64_at(b, 0),
            duration_secs: le::u32_at(b, 8),
            cooldown_secs: le::u32_at(b, 12),
            max_buy_quote: le::u64_at(b, 16),
            max_sell_quote: le::u64_at(b, 24),
            max_buys_per_slot: le::u16_at(b, 32),
            flags: le::u16_at(b, 34),
        }
    }
    pub fn encode(&self) -> [u8; 48] {
        let mut b = [0u8; 48];
        le::put_i64(&mut b, 0, self.start_ts);
        le::put_u32(&mut b, 8, self.duration_secs);
        le::put_u32(&mut b, 12, self.cooldown_secs);
        le::put_u64(&mut b, 16, self.max_buy_quote);
        le::put_u64(&mut b, 24, self.max_sell_quote);
        le::put_u16(&mut b, 32, self.max_buys_per_slot);
        le::put_u16(&mut b, 34, self.flags);
        b
    }
}

// state: [0..8] tracked slot, [8..12] buys in tracked slot
pub struct LaunchShield;

impl PumpHook for LaunchShield {
    const MODULE_ID: u16 = ModuleId::LaunchShield as u16;

    fn validate_config(config: &[u8; 48]) -> Result<(), HookError> {
        let c = Config::decode(config);
        let known = FLAG_DENY_SAME_SLOT_ROUND_TRIP | FLAG_COOLDOWN_SELLS;
        if c.duration_secs == 0
            || c.duration_secs > MAX_DURATION_SECS
            || c.cooldown_secs > MAX_COOLDOWN_SECS
            || c.start_ts < 0
            || c.flags & !known != 0
            || config[36..].iter().any(|&x| x != 0)
        {
            return Err(bad_config(Self::MODULE_ID));
        }
        Ok(())
    }

    fn on_callback(env: &mut HookEnv<'_>) -> Result<HookDecision, HookError> {
        let c = Config::decode(env.config);
        let ctx = env.ctx;
        let start = if c.start_ts == 0 { ctx.market_activated_at } else { c.start_ts };
        let now = ctx.timestamp;
        if now < start || now >= start.saturating_add(c.duration_secs as i64) {
            return Ok(HookDecision::allow().with_flags(decision_flags::INACTIVE));
        }
        let side = if ctx.is_buy() { Side::Buy } else { Side::Sell };
        let is_before_side = matches!(env.callback, HookCallback::BeforeBuy | HookCallback::BeforeSell | HookCallback::BeforeSwap);
        if !is_before_side {
            return Ok(HookDecision::allow());
        }

        match side {
            Side::Buy if c.max_buy_quote > 0 && ctx.quote_amount > c.max_buy_quote => {
                return Ok(HookDecision::deny(r::MAX_BUY_EXCEEDED))
            }
            Side::Sell if c.max_sell_quote > 0 && ctx.quote_amount > c.max_sell_quote => {
                return Ok(HookDecision::deny(r::MAX_SELL_EXCEEDED))
            }
            _ => {}
        }

        let u = &*env.user;
        let cooldown_applies = side == Side::Buy || c.flags & FLAG_COOLDOWN_SELLS != 0;
        if cooldown_applies
            && c.cooldown_secs > 0
            && u.last_trade_ts != 0
            && now.saturating_sub(u.last_trade_ts) < c.cooldown_secs as i64
        {
            return Ok(HookDecision::deny(r::COOLDOWN));
        }

        if c.flags & FLAG_DENY_SAME_SLOT_ROUND_TRIP != 0
            && u.trade_count > 0
            && u.last_trade_slot == ctx.slot
            && u.last_side != side as u8
        {
            return Ok(HookDecision::deny(r::SAME_SLOT_ROUND_TRIP));
        }

        if side == Side::Buy && c.max_buys_per_slot > 0 {
            let tracked = le::u64_at(env.state, 0);
            let count = if tracked == ctx.slot { le::u32_at(env.state, 8) } else { 0 };
            if count >= c.max_buys_per_slot as u32 {
                return Ok(HookDecision::deny(r::SLOT_BUY_CAP));
            }
            le::put_u64(env.state, 0, ctx.slot);
            le::put_u32(env.state, 8, count + 1);
            return Ok(HookDecision::allow().with_flags(decision_flags::STATE_WRITTEN));
        }
        Ok(HookDecision::allow())
    }
}
