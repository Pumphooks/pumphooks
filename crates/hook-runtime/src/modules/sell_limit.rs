//! Sell Limit (anti-dump): a routed sell may move at most a share of the wallet's balance, and a
//! wallet may sell at most once per cooldown. Balances at or below `exempt_below` may sell in
//! full, so small holders can always exit.
//!
//! Applies to routed sells only; a direct Pump sell is not affected.

use hook_types::reason::sell_limit as r;
use hook_types::slot::le;
use hook_types::{decision_flags, HookDecision, ModuleId};

use super::max_wallet::{window_active, MAX_DURATION_SECS};
use super::{bad_config, HookEnv, PumpHook};
use crate::executor::HookError;

pub const MAX_COOLDOWN_SECS: u32 = 7 * 86_400;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// Largest share of the wallet's balance one sell may move, in bps (0 = no share limit).
    pub max_sell_bps: u16,
    /// Minimum seconds between two routed sells of one wallet (0 = none).
    pub cooldown_secs: u32,
    /// Balances at or below this (base units) are not share-limited.
    pub exempt_below: u64,
    /// 0 = from market activation.
    pub start_ts: i64,
    /// 0 = no end.
    pub duration_secs: u32,
}

impl Config {
    pub fn decode(b: &[u8; 48]) -> Self {
        Self {
            max_sell_bps: le::u16_at(b, 0),
            cooldown_secs: le::u32_at(b, 4),
            exempt_below: le::u64_at(b, 8),
            start_ts: le::i64_at(b, 16),
            duration_secs: le::u32_at(b, 24),
        }
    }
    pub fn encode(&self) -> [u8; 48] {
        let mut b = [0u8; 48];
        le::put_u16(&mut b, 0, self.max_sell_bps);
        le::put_u32(&mut b, 4, self.cooldown_secs);
        le::put_u64(&mut b, 8, self.exempt_below);
        le::put_i64(&mut b, 16, self.start_ts);
        le::put_u32(&mut b, 24, self.duration_secs);
        b
    }
}

/// Largest sell (base units) allowed from `balance`; never below 1 so a holder can always sell.
pub fn allowed_sell(balance: u64, max_sell_bps: u16) -> u64 {
    ((balance as u128 * max_sell_bps as u128 / 10_000) as u64).max(1)
}

pub struct SellLimit;

impl PumpHook for SellLimit {
    const MODULE_ID: u16 = ModuleId::SellLimit as u16;

    fn validate_config(config: &[u8; 48]) -> Result<(), HookError> {
        let c = Config::decode(config);
        if (c.max_sell_bps == 0 && c.cooldown_secs == 0)
            || c.max_sell_bps >= 10_000
            || c.cooldown_secs > MAX_COOLDOWN_SECS
            || c.start_ts < 0
            || c.duration_secs > MAX_DURATION_SECS
            || config[2..4].iter().any(|&x| x != 0)
            || config[28..].iter().any(|&x| x != 0)
        {
            return Err(bad_config(Self::MODULE_ID));
        }
        Ok(())
    }

    fn on_callback(env: &mut HookEnv<'_>) -> Result<HookDecision, HookError> {
        let c = Config::decode(env.config);
        let ctx = env.ctx;
        if ctx.is_buy() || !env.callback.is_before() || !window_active(c.start_ts, c.duration_secs, ctx.market_activated_at, ctx.timestamp) {
            return Ok(HookDecision::allow().with_flags(decision_flags::INACTIVE));
        }
        let balance = ctx.user_base_balance;
        if c.max_sell_bps > 0 && balance > c.exempt_below && ctx.base_amount > allowed_sell(balance, c.max_sell_bps) {
            return Ok(HookDecision::deny(r::SELL_TOO_LARGE));
        }
        let last = env.user.last_sell_ts;
        if c.cooldown_secs > 0 && last != 0 && ctx.timestamp.saturating_sub(last) < c.cooldown_secs as i64 {
            return Ok(HookDecision::deny(r::SELL_COOLDOWN));
        }
        Ok(HookDecision::allow())
    }
}
