//! Max Wallet: a routed buy may not take the wallet's balance of the coin above a cap.
//!
//! Checked before the buy on the estimate and again after it on the observed balance, so a
//! buy that overshoots the estimate reverts. The cap is per token account and applies to routed
//! buys only: transfers, other wallets and direct Pump trades are not affected.

use hook_types::reason::max_wallet as r;
use hook_types::slot::le;
use hook_types::{decision_flags, HookDecision, ModuleId};

use super::{bad_config, HookEnv, PumpHook};
use crate::executor::HookError;

/// Longest limited window: 365 days (0 = no end).
pub const MAX_DURATION_SECS: u32 = 365 * 86_400;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// Largest balance (base units) a routed buy may leave the wallet with.
    pub max_base: u64,
    /// 0 = from market activation.
    pub start_ts: i64,
    /// 0 = no end.
    pub duration_secs: u32,
}

impl Config {
    pub fn decode(b: &[u8; 48]) -> Self {
        Self { max_base: le::u64_at(b, 0), start_ts: le::i64_at(b, 8), duration_secs: le::u32_at(b, 16) }
    }
    pub fn encode(&self) -> [u8; 48] {
        let mut b = [0u8; 48];
        le::put_u64(&mut b, 0, self.max_base);
        le::put_i64(&mut b, 8, self.start_ts);
        le::put_u32(&mut b, 16, self.duration_secs);
        b
    }
}

/// Whether a `[start, start + duration)` window (duration 0 = open-ended) covers `now`.
pub(crate) fn window_active(start_ts: i64, duration_secs: u32, activated_at: i64, now: i64) -> bool {
    let start = if start_ts == 0 { activated_at } else { start_ts };
    now >= start && (duration_secs == 0 || now < start.saturating_add(duration_secs as i64))
}

pub struct MaxWallet;

impl PumpHook for MaxWallet {
    const MODULE_ID: u16 = ModuleId::MaxWallet as u16;

    fn validate_config(config: &[u8; 48]) -> Result<(), HookError> {
        let c = Config::decode(config);
        if c.max_base == 0 || c.start_ts < 0 || c.duration_secs > MAX_DURATION_SECS || config[20..].iter().any(|&x| x != 0) {
            return Err(bad_config(Self::MODULE_ID));
        }
        Ok(())
    }

    fn on_callback(env: &mut HookEnv<'_>) -> Result<HookDecision, HookError> {
        let c = Config::decode(env.config);
        let ctx = env.ctx;
        if !ctx.is_buy() || !window_active(c.start_ts, c.duration_secs, ctx.market_activated_at, ctx.timestamp) {
            return Ok(HookDecision::allow().with_flags(decision_flags::INACTIVE));
        }
        if env.callback.is_before() {
            if ctx.user_base_balance.saturating_add(ctx.base_amount) > c.max_base {
                return Ok(HookDecision::deny(r::BALANCE_CAP));
            }
        } else if ctx.user_base_balance > c.max_base {
            // After the buy, `user_base_balance` is the observed post-trade balance.
            return Err(HookError::PostCondition { module: Self::MODULE_ID, reason: r::BALANCE_CAP });
        }
        Ok(HookDecision::allow())
    }
}
