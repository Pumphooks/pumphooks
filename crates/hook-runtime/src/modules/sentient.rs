//! SENTIENT: on-chain half of the autonomous risk engine.
//!
//! The off-chain engine proposes a `Regime` through a signed `StrategySignal`; the program verifies
//! signer, nonce, freshness and bounds before writing it here (see `apply_signal`). This module
//! only *restricts* routed flow according to the active regime and records routed buy/sell flow
//! that the engine reads back. It never trades and never moves funds.

use hook_types::reason::sentient as r;
use hook_types::signal::{SignalBounds, SignalRejection};
use hook_types::slot::le;
use hook_types::{decision_flags, HookDecision, ModuleId, Regime, SignalAction, StrategySignal};

use super::{bad_config, HookEnv, PumpHook};
use crate::executor::HookError;

pub const FLAG_APPLY_BUYS: u32 = 1 << 0;
pub const FLAG_APPLY_SELLS: u32 = 1 << 1;
pub const FLOW_WINDOW_SECS: i64 = 3_600;
pub const MAX_TTL_SECS: u32 = 86_400;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// ed25519 key of the off-chain engine.
    pub signer: [u8; 32],
    /// Routed trade cap while `Defensive`.
    pub defensive_max_quote: u64,
    /// Longest regime a single signal may set.
    pub max_signal_ttl: u32,
    pub flags: u32,
}

impl Config {
    pub fn decode(b: &[u8; 48]) -> Self {
        let mut signer = [0u8; 32];
        signer.copy_from_slice(&b[..32]);
        Self {
            signer,
            defensive_max_quote: le::u64_at(b, 32),
            max_signal_ttl: le::u32_at(b, 40),
            flags: le::u32_at(b, 44),
        }
    }
    pub fn encode(&self) -> [u8; 48] {
        let mut b = [0u8; 48];
        b[..32].copy_from_slice(&self.signer);
        le::put_u64(&mut b, 32, self.defensive_max_quote);
        le::put_u32(&mut b, 40, self.max_signal_ttl);
        le::put_u32(&mut b, 44, self.flags);
        b
    }
}

/// Slot state: `[0] regime`, `[8..16] regime_until`, `[16..24] last_nonce`,
/// `[24..32] flow_window_start`, `[32..40] buy_flow`, `[40..48] sell_flow`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub regime: u8,
    pub regime_until: i64,
    pub last_nonce: u64,
    pub flow_window_start: i64,
    pub buy_flow: u64,
    pub sell_flow: u64,
}

impl State {
    pub fn decode(b: &[u8; 48]) -> Self {
        Self {
            regime: b[0],
            regime_until: le::i64_at(b, 8),
            last_nonce: le::u64_at(b, 16),
            flow_window_start: le::i64_at(b, 24),
            buy_flow: le::u64_at(b, 32),
            sell_flow: le::u64_at(b, 40),
        }
    }
    pub fn write(&self, b: &mut [u8; 48]) {
        b[0] = self.regime;
        b[1..8].fill(0);
        le::put_i64(b, 8, self.regime_until);
        le::put_u64(b, 16, self.last_nonce);
        le::put_i64(b, 24, self.flow_window_start);
        le::put_u64(b, 32, self.buy_flow);
        le::put_u64(b, 40, self.sell_flow);
    }
    /// Regime in force at `now`; expired regimes fall back to `Normal`.
    pub fn effective_regime(&self, now: i64) -> Regime {
        if now > self.regime_until {
            Regime::Normal
        } else {
            Regime::from_u8(self.regime).unwrap_or(Regime::Normal)
        }
    }
}

/// Validate a signal (signature already verified by the caller) and apply it to slot state.
/// Only `ChangeStrategyState` and `NoOp` act here; treasury actions belong to the vault.
pub fn apply_signal(
    market: [u8; 32],
    config: &[u8; 48],
    state: &mut [u8; 48],
    signal: &StrategySignal,
    now: i64,
) -> Result<SignalAction, SignalRejection> {
    let c = Config::decode(config);
    let mut s = State::decode(state);
    let bounds = SignalBounds {
        market,
        last_nonce: s.last_nonce,
        allowed_actions: SignalAction::NoOp.bit() | SignalAction::ChangeStrategyState.bit(),
        max_amount_bps: 0,
        max_validity_secs: c.max_signal_ttl as i64,
    };
    let action = signal.check(&bounds, now)?;
    s.last_nonce = signal.nonce;
    if action == SignalAction::ChangeStrategyState {
        s.regime = signal.param;
        s.regime_until = signal.valid_until;
    }
    s.write(state);
    Ok(action)
}

pub struct Sentient;

impl PumpHook for Sentient {
    const MODULE_ID: u16 = ModuleId::Sentient as u16;

    fn validate_config(config: &[u8; 48]) -> Result<(), HookError> {
        let c = Config::decode(config);
        if c.signer == [0u8; 32]
            || c.max_signal_ttl == 0
            || c.max_signal_ttl > MAX_TTL_SECS
            || c.flags & !(FLAG_APPLY_BUYS | FLAG_APPLY_SELLS) != 0
        {
            return Err(bad_config(Self::MODULE_ID));
        }
        Ok(())
    }

    fn on_callback(env: &mut HookEnv<'_>) -> Result<HookDecision, HookError> {
        let c = Config::decode(env.config);
        let ctx = env.ctx;
        let now = ctx.timestamp;
        let mut s = State::decode(env.state);

        if env.callback.is_before() {
            let applies = if ctx.is_buy() { c.flags & FLAG_APPLY_BUYS != 0 } else { c.flags & FLAG_APPLY_SELLS != 0 };
            if !applies {
                return Ok(HookDecision::allow());
            }
            return Ok(match s.effective_regime(now) {
                Regime::Defensive if c.defensive_max_quote > 0 && ctx.quote_amount > c.defensive_max_quote => {
                    HookDecision::deny(r::DEFENSIVE_SIZE_CAP)
                }
                Regime::CircuitBreaker => HookDecision::deny(r::CIRCUIT_BREAKER),
                _ => HookDecision::allow(),
            });
        }

        if now.saturating_sub(s.flow_window_start) >= FLOW_WINDOW_SECS {
            s.flow_window_start = now;
            s.buy_flow = 0;
            s.sell_flow = 0;
        }
        if ctx.is_buy() {
            s.buy_flow = s.buy_flow.saturating_add(ctx.quote_amount);
        } else {
            s.sell_flow = s.sell_flow.saturating_add(ctx.quote_amount);
        }
        s.write(env.state);
        Ok(HookDecision::allow().with_flags(decision_flags::STATE_WRITTEN))
    }
}
