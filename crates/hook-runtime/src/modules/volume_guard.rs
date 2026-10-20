//! Volume Guard: bounds on routed trade size and rolling routed volume.
//!
//! Before-callbacks check the deterministic estimate; after-callbacks re-check the observed
//! amount and fail the transaction if Pump executed more than the guard allows.

use hook_types::reason::volume_guard as r;
use hook_types::slot::le;
use hook_types::{decision_flags, HookDecision, ModuleId};

use super::{bad_config, HookEnv, PumpHook};
use crate::executor::HookError;

pub const MAX_WINDOW_SECS: u32 = 7 * 86_400;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// 0 = no per-trade limit for that side.
    pub max_buy_quote: u64,
    pub max_sell_quote: u64,
    /// 0 = no rolling window.
    pub window_secs: u32,
    /// Market-wide routed volume per window (0 = none).
    pub window_max_quote: u64,
    /// Per-wallet routed volume per window (0 = none).
    pub wallet_window_max_quote: u64,
}

impl Config {
    pub fn decode(b: &[u8; 48]) -> Self {
        Self {
            max_buy_quote: le::u64_at(b, 0),
            max_sell_quote: le::u64_at(b, 8),
            window_secs: le::u32_at(b, 16),
            window_max_quote: le::u64_at(b, 24),
            wallet_window_max_quote: le::u64_at(b, 32),
        }
    }
    pub fn encode(&self) -> [u8; 48] {
        let mut b = [0u8; 48];
        le::put_u64(&mut b, 0, self.max_buy_quote);
        le::put_u64(&mut b, 8, self.max_sell_quote);
        le::put_u32(&mut b, 16, self.window_secs);
        le::put_u64(&mut b, 24, self.window_max_quote);
        le::put_u64(&mut b, 32, self.wallet_window_max_quote);
        b
    }
}

// state: [0..8] window_start, [8..16] window_volume
pub struct VolumeGuard;

fn window_volume(start: i64, vol: u64, now: i64, window: u32) -> u64 {
    if now.saturating_sub(start) >= window as i64 { 0 } else { vol }
}

impl PumpHook for VolumeGuard {
    const MODULE_ID: u16 = ModuleId::VolumeGuard as u16;

    fn validate_config(config: &[u8; 48]) -> Result<(), HookError> {
        let c = Config::decode(config);
        let windowed = c.window_max_quote > 0 || c.wallet_window_max_quote > 0;
        let any = c.max_buy_quote > 0 || c.max_sell_quote > 0 || windowed;
        if !any
            || (windowed && c.window_secs == 0)
            || c.window_secs > MAX_WINDOW_SECS
            || config[20..24].iter().any(|&x| x != 0)
            || config[40..].iter().any(|&x| x != 0)
        {
            return Err(bad_config(Self::MODULE_ID));
        }
        Ok(())
    }

    fn on_callback(env: &mut HookEnv<'_>) -> Result<HookDecision, HookError> {
        let c = Config::decode(env.config);
        let ctx = env.ctx;
        let quote = ctx.quote_amount;
        let before = env.callback.is_before();

        let per_trade = if ctx.is_buy() { c.max_buy_quote } else { c.max_sell_quote };
        if per_trade > 0 && quote > per_trade {
            let code = if ctx.is_buy() { r::MAX_BUY_EXCEEDED } else { r::MAX_SELL_EXCEEDED };
            return if before {
                Ok(HookDecision::deny(code))
            } else {
                Err(HookError::PostCondition { module: Self::MODULE_ID, reason: code })
            };
        }
        if c.window_secs == 0 {
            return Ok(HookDecision::allow());
        }

        let now = ctx.timestamp;
        let m_start = le::i64_at(env.state, 0);
        let m_vol = window_volume(m_start, le::u64_at(env.state, 8), now, c.window_secs);
        let w_vol = window_volume(env.user.vg_window_start, env.user.vg_window_volume, now, c.window_secs);

        if c.window_max_quote > 0 && m_vol.saturating_add(quote) > c.window_max_quote {
            return if before {
                Ok(HookDecision::deny(r::MARKET_WINDOW_EXCEEDED))
            } else {
                Err(HookError::PostCondition { module: Self::MODULE_ID, reason: r::MARKET_WINDOW_EXCEEDED })
            };
        }
        if c.wallet_window_max_quote > 0 && w_vol.saturating_add(quote) > c.wallet_window_max_quote {
            return if before {
                Ok(HookDecision::deny(r::WALLET_WINDOW_EXCEEDED))
            } else {
                Err(HookError::PostCondition { module: Self::MODULE_ID, reason: r::WALLET_WINDOW_EXCEEDED })
            };
        }
        if before {
            return Ok(HookDecision::allow());
        }

        // After: commit observed volume to both windows.
        if m_vol == 0 && now.saturating_sub(m_start) >= c.window_secs as i64 {
            le::put_i64(env.state, 0, now);
        }
        le::put_u64(env.state, 8, m_vol.saturating_add(quote));
        if now.saturating_sub(env.user.vg_window_start) >= c.window_secs as i64 {
            env.user.vg_window_start = now;
        }
        env.user.vg_window_volume = w_vol.saturating_add(quote);
        Ok(HookDecision::allow().with_flags(decision_flags::STATE_WRITTEN))
    }
}
