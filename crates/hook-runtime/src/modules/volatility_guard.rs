//! Volatility Guard: compares the venue's current on-chain spot price to an EMA of prices observed
//! after routed trades.
//!
//! Data source: on-chain reserves only (deterministic). Observations are sparse — they happen on
//! routed trades — so the EMA describes routed-observed history, not every Pump trade.

use hook_types::reason::volatility_guard as r;
use hook_types::slot::le;
use hook_types::{decision_flags, HookContext, HookDecision, ModuleId};

use super::{bad_config, HookEnv, PumpHook};
use crate::executor::HookError;

pub const FLAG_CHECK_BUYS: u32 = 1 << 0;
pub const FLAG_CHECK_SELLS: u32 = 1 << 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// Deny when |spot − ema| / ema exceeds this (bps).
    pub max_deviation_bps: u32,
    /// Weight of a new observation in the EMA (bps of 10_000).
    pub ema_alpha_bps: u16,
    /// Observations required before the guard enforces anything.
    pub min_observations: u16,
    /// Trades below this quote size are never checked (0 = check all).
    pub check_min_quote: u64,
    pub flags: u32,
}

impl Config {
    pub fn decode(b: &[u8; 48]) -> Self {
        Self {
            max_deviation_bps: le::u32_at(b, 0),
            ema_alpha_bps: le::u16_at(b, 4),
            min_observations: le::u16_at(b, 6),
            check_min_quote: le::u64_at(b, 8),
            flags: le::u32_at(b, 16),
        }
    }
    pub fn encode(&self) -> [u8; 48] {
        let mut b = [0u8; 48];
        le::put_u32(&mut b, 0, self.max_deviation_bps);
        le::put_u16(&mut b, 4, self.ema_alpha_bps);
        le::put_u16(&mut b, 6, self.min_observations);
        le::put_u64(&mut b, 8, self.check_min_quote);
        le::put_u32(&mut b, 16, self.flags);
        b
    }
}

/// Slot state layout: `[0..16] ema_q64`, `[16..24] last_obs_ts`, `[24..28] obs_count`.
pub struct State {
    pub ema_q64: u128,
    pub last_obs_ts: i64,
    pub obs_count: u32,
}

impl State {
    pub fn decode(b: &[u8; 48]) -> Self {
        Self { ema_q64: le::u128_at(b, 0), last_obs_ts: le::i64_at(b, 16), obs_count: le::u32_at(b, 24) }
    }
    fn write(&self, b: &mut [u8; 48]) {
        le::put_u128(b, 0, self.ema_q64);
        le::put_i64(b, 16, self.last_obs_ts);
        le::put_u32(b, 24, self.obs_count);
    }
}

/// Deviation of `spot` from `ema` in bps, saturating.
pub fn deviation_bps(spot: u128, ema: u128) -> u64 {
    if ema == 0 {
        return 0;
    }
    let diff = spot.abs_diff(ema);
    // diff * 10_000 can overflow u128 only for absurd prices; saturate.
    let num = diff.saturating_mul(10_000);
    (num / ema).min(u64::MAX as u128) as u64
}

/// Current deviation if the guard is warm. Used by policies (`VOLATILITY` field).
pub fn current_deviation(config: &[u8; 48], state: &[u8; 48], ctx: &HookContext) -> Option<u64> {
    let c = Config::decode(config);
    let s = State::decode(state);
    if s.obs_count < c.min_observations.max(1) as u32 {
        return None;
    }
    let spot = HookContext::spot_q64(ctx.pre_base_reserves, ctx.pre_quote_reserves)?;
    Some(deviation_bps(spot, s.ema_q64))
}

pub struct VolatilityGuard;

impl PumpHook for VolatilityGuard {
    const MODULE_ID: u16 = ModuleId::VolatilityGuard as u16;

    fn validate_config(config: &[u8; 48]) -> Result<(), HookError> {
        let c = Config::decode(config);
        if c.max_deviation_bps == 0
            || c.max_deviation_bps > 1_000_000
            || c.ema_alpha_bps == 0
            || c.ema_alpha_bps > 10_000
            || c.flags & !(FLAG_CHECK_BUYS | FLAG_CHECK_SELLS) != 0
            || c.flags == 0
            || config[20..].iter().any(|&x| x != 0)
        {
            return Err(bad_config(Self::MODULE_ID));
        }
        Ok(())
    }

    fn on_callback(env: &mut HookEnv<'_>) -> Result<HookDecision, HookError> {
        let c = Config::decode(env.config);
        let ctx = env.ctx;
        if env.callback.is_before() {
            let applies = if ctx.is_buy() { c.flags & FLAG_CHECK_BUYS != 0 } else { c.flags & FLAG_CHECK_SELLS != 0 };
            if !applies || ctx.quote_amount < c.check_min_quote {
                return Ok(HookDecision::allow());
            }
            return Ok(match current_deviation(env.config, env.state, ctx) {
                Some(d) if d > c.max_deviation_bps as u64 => {
                    HookDecision::deny(if ctx.is_buy() { r::BUY_DEVIATION } else { r::SELL_DEVIATION })
                }
                Some(_) => HookDecision::allow(),
                None => HookDecision::allow().with_flags(decision_flags::INACTIVE),
            });
        }

        // After: observe post-trade spot.
        let Some(spot) = HookContext::spot_q64(ctx.post_base_reserves, ctx.post_quote_reserves) else {
            return Ok(HookDecision::allow());
        };
        let mut s = State::decode(env.state);
        s.ema_q64 = if s.obs_count == 0 {
            spot
        } else {
            let a = c.ema_alpha_bps as u128;
            // ema + (spot - ema) * a / 10_000, computed without signed math.
            if spot >= s.ema_q64 {
                s.ema_q64 + (spot - s.ema_q64) / 10_000 * a + ((spot - s.ema_q64) % 10_000) * a / 10_000
            } else {
                s.ema_q64 - ((s.ema_q64 - spot) / 10_000 * a + ((s.ema_q64 - spot) % 10_000) * a / 10_000)
            }
        };
        s.last_obs_ts = ctx.timestamp;
        s.obs_count = s.obs_count.saturating_add(1);
        s.write(env.state);
        Ok(HookDecision::allow().with_flags(decision_flags::STATE_WRITTEN))
    }
}
