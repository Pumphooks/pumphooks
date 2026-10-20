//! Holder XP: a loyalty ledger earned only through routed activity.
//!
//! Anti-abuse rules, enforced on-chain:
//! - XP is earned on routed **buys** only, never on sells;
//! - buy XP vests after `vest_secs`; any routed sell before vesting forfeits the unvested XP, so
//!   buy/sell loops earn nothing;
//! - per-trade quote cap, minimum trade size and a daily cap per wallet;
//! - holding XP accrues on `min(balance at last checkpoint, balance now) × elapsed`, so tokens
//!   moved out between checkpoints earn nothing for that interval.
//!
//! Residual risk (documented in `docs/threat-model.md` T20): two wallets that each hold at both of
//! their own checkpoints can overlap holding intervals. Caps bound the effect.

use hook_types::slot::le;
use hook_types::{decision_flags, HookCallback, HookDecision, ModuleId, UserState};

use super::{bad_config, HookEnv, PumpHook};
use crate::executor::HookError;

pub const LAMPORTS_PER_SOL: u128 = 1_000_000_000;
pub const SECS_PER_DAY: i64 = 86_400;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// Quote above this per trade earns nothing extra.
    pub per_trade_cap_quote: u64,
    /// XP per 1 SOL (1e9 quote units) of routed buy.
    pub xp_per_sol: u32,
    pub vest_secs: u32,
    pub daily_cap: u32,
    /// XP per day for holding at least `hold_cap_base` (pro-rata below).
    pub hold_xp_per_day: u32,
    pub hold_cap_base: u64,
    pub min_trade_quote: u64,
}

impl Config {
    pub fn decode(b: &[u8; 48]) -> Self {
        Self {
            per_trade_cap_quote: le::u64_at(b, 0),
            xp_per_sol: le::u32_at(b, 8),
            vest_secs: le::u32_at(b, 12),
            daily_cap: le::u32_at(b, 16),
            hold_xp_per_day: le::u32_at(b, 20),
            hold_cap_base: le::u64_at(b, 24),
            min_trade_quote: le::u64_at(b, 32),
        }
    }
    pub fn encode(&self) -> [u8; 48] {
        let mut b = [0u8; 48];
        le::put_u64(&mut b, 0, self.per_trade_cap_quote);
        le::put_u32(&mut b, 8, self.xp_per_sol);
        le::put_u32(&mut b, 12, self.vest_secs);
        le::put_u32(&mut b, 16, self.daily_cap);
        le::put_u32(&mut b, 20, self.hold_xp_per_day);
        le::put_u64(&mut b, 24, self.hold_cap_base);
        le::put_u64(&mut b, 32, self.min_trade_quote);
        b
    }
}

// state: [0..8] total vested XP awarded in this market, [8..16] total forfeited
pub struct HolderXp;

fn settle_vested(u: &mut UserState, now: i64) -> u64 {
    if u.pending_xp > 0 && now >= u.pending_vest_at {
        let v = u.pending_xp;
        u.xp = u.xp.saturating_add(v);
        u.pending_xp = 0;
        v
    } else {
        0
    }
}

fn accrue_holding(c: &Config, u: &mut UserState, balance_now: u64, now: i64) -> u64 {
    if c.hold_xp_per_day == 0 || c.hold_cap_base == 0 || u.hold_checkpoint_ts == 0 {
        return 0;
    }
    let elapsed = now.saturating_sub(u.hold_checkpoint_ts).max(0) as u128;
    let held = u.hold_checkpoint_balance.min(balance_now).min(c.hold_cap_base) as u128;
    let xp = held * c.hold_xp_per_day as u128 * elapsed
        / (c.hold_cap_base as u128 * SECS_PER_DAY as u128);
    let xp = xp.min(u64::MAX as u128) as u64;
    u.xp = u.xp.saturating_add(xp);
    xp
}

impl PumpHook for HolderXp {
    const MODULE_ID: u16 = ModuleId::HolderXp as u16;

    fn validate_config(config: &[u8; 48]) -> Result<(), HookError> {
        let c = Config::decode(config);
        if c.xp_per_sol == 0
            || c.per_trade_cap_quote == 0
            || c.daily_cap == 0
            || c.vest_secs > 30 * 86_400
            || (c.hold_xp_per_day > 0 && c.hold_cap_base == 0)
            || config[40..].iter().any(|&x| x != 0)
        {
            return Err(bad_config(Self::MODULE_ID));
        }
        Ok(())
    }

    fn on_callback(env: &mut HookEnv<'_>) -> Result<HookDecision, HookError> {
        let c = Config::decode(env.config);
        let ctx = env.ctx;
        let now = ctx.timestamp;
        let u = &mut *env.user;
        let mut vested = settle_vested(u, now);

        match env.callback {
            HookCallback::AfterBuy => {
                let post = ctx.user_base_balance;
                let pre = post.saturating_sub(ctx.base_amount);
                vested = vested.saturating_add(accrue_holding(&c, u, pre, now));
                u.hold_checkpoint_ts = now;
                u.hold_checkpoint_balance = post;

                if ctx.quote_amount >= c.min_trade_quote.max(1) {
                    let day = (now / SECS_PER_DAY).max(0) as u32;
                    if u.xp_day != day {
                        u.xp_day = day;
                        u.xp_today = 0;
                    }
                    let q = ctx.quote_amount.min(c.per_trade_cap_quote) as u128;
                    let raw = (q * c.xp_per_sol as u128 / LAMPORTS_PER_SOL).min(u32::MAX as u128) as u32;
                    let room = c.daily_cap.saturating_sub(u.xp_today);
                    let award = raw.min(room);
                    if award > 0 {
                        u.xp_today += award;
                        u.pending_xp = u.pending_xp.saturating_add(award as u64);
                        u.pending_vest_at = now.saturating_add(c.vest_secs as i64);
                        if c.vest_secs == 0 {
                            vested = vested.saturating_add(settle_vested(u, now));
                        }
                    }
                }
            }
            HookCallback::AfterSell => {
                let post = ctx.user_base_balance;
                vested = vested.saturating_add(accrue_holding(&c, u, post, now));
                if u.pending_xp > 0 {
                    let f = u.pending_xp;
                    u.forfeited_xp = u.forfeited_xp.saturating_add(f);
                    u.pending_xp = 0;
                    let tf = le::u64_at(env.state, 8).saturating_add(f);
                    le::put_u64(env.state, 8, tf);
                }
                u.hold_checkpoint_ts = now;
                u.hold_checkpoint_balance = post;
            }
            _ => return Ok(HookDecision::allow()),
        }

        if vested > 0 {
            let t = le::u64_at(env.state, 0).saturating_add(vested);
            le::put_u64(env.state, 0, t);
        }
        Ok(HookDecision::allow().with_flags(decision_flags::STATE_WRITTEN | decision_flags::POINTS_AWARDED))
    }
}
