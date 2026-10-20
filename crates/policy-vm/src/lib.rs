//! Pump Hooks Policy VM.
//!
//! A policy is `WHEN trigger IF conditions THEN actions`, stored as a fixed 352-byte record.
//! Evaluation is O(conditions + actions) with both bounded, has no loops over external data, and
//! cannot call other programs. See `docs/policy-vm.md`.

#![no_std]

use bytemuck::{Pod, Zeroable};
use hook_types::{
    decision_flags, reason, HookCallback, HookContext, HookDecision, MarketStats, UserState,
};

pub const POLICY_VERSION: u8 = 1;
pub const MAX_CONDITIONS: usize = 8;
pub const MAX_ACTIONS: usize = 4;
pub const MAX_REGISTERS: usize = 4;
pub const MAX_DENY_REASON: u16 = 999;
pub const MAX_POINTS_PER_ACTION: u64 = 10_000;

pub mod trigger {
    pub const BEFORE_BUY: u8 = 0;
    pub const AFTER_BUY: u8 = 1;
    pub const BEFORE_SELL: u8 = 2;
    pub const AFTER_SELL: u8 = 3;
    pub const BEFORE_SWAP: u8 = 4;
    pub const AFTER_SWAP: u8 = 5;
    // Reserved (crank-driven) triggers.
    pub const VOLUME_THRESHOLD: u8 = 16;
    pub const VOLATILITY_THRESHOLD: u8 = 17;
    pub const TIME_INTERVAL: u8 = 18;
    pub const TREASURY_THRESHOLD: u8 = 19;
    pub const GRADUATION: u8 = 20;
    pub const FEE_EVENT: u8 = 21;
    pub const STRATEGY_SIGNAL: u8 = 22;

    pub const fn is_v1(t: u8) -> bool {
        t <= AFTER_SWAP
    }
    pub const fn is_before(t: u8) -> bool {
        matches!(t, BEFORE_BUY | BEFORE_SELL | BEFORE_SWAP)
    }
}

pub mod field {
    pub const TRADE_SIZE: u8 = 1;
    pub const TRADE_SIZE_BASE: u8 = 2;
    pub const ROUTED_VOLUME: u8 = 3;
    pub const VOLATILITY: u8 = 4;
    pub const TREASURY_BALANCE: u8 = 5;
    pub const TIME_SINCE_LAUNCH: u8 = 6;
    pub const TIME_SINCE_LAST_ACTION: u8 = 7;
    pub const BUY_SELL_RATIO: u8 = 8;
    pub const HOLDER_DURATION: u8 = 9;
    pub const HOOK_STATE: u8 = 10;
    pub const USER_ROUTED_VOLUME: u8 = 11;
    pub const USER_XP: u8 = 12;
    pub const USER_BASE_BALANCE: u8 = 13;
    pub const MAX: u8 = 13;
}

pub mod op {
    pub const EQ: u8 = 0;
    pub const NEQ: u8 = 1;
    pub const GT: u8 = 2;
    pub const GTE: u8 = 3;
    pub const LT: u8 = 4;
    pub const LTE: u8 = 5;
    pub const MAX: u8 = 5;
}

pub mod action {
    pub const NONE: u8 = 0;
    pub const ALLOW: u8 = 1;
    pub const DENY: u8 = 2;
    pub const RECORD: u8 = 3;
    pub const AWARD_POINTS: u8 = 4;
    pub const UPDATE_STATE: u8 = 5;
    pub const QUEUE_TWAP: u8 = 6;
    pub const ALLOCATE_AUTHORIZED_TREASURY: u8 = 7;
    pub const PAUSE_STRATEGY: u8 = 8;
    pub const RESUME_STRATEGY: u8 = 9;
    pub const EMIT_SIGNAL: u8 = 10;
    pub const EXECUTE_APPROVED_STRATEGY: u8 = 11;
    pub const MAX: u8 = 11;

    pub const fn is_intent(k: u8) -> bool {
        matches!(
            k,
            QUEUE_TWAP
                | ALLOCATE_AUTHORIZED_TREASURY
                | PAUSE_STRATEGY
                | RESUME_STRATEGY
                | EXECUTE_APPROVED_STRATEGY
        )
    }
}

pub mod state_op {
    pub const SET: u16 = 0;
    pub const ADD: u16 = 1;
    pub const SUB: u16 = 2;
}

pub mod combinator {
    pub const ALL: u8 = 0;
    pub const ANY: u8 = 1;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
#[repr(C)]
pub struct Condition {
    pub field: u8,
    pub op: u8,
    /// Field argument (register index for `HOOK_STATE`).
    pub arg: u8,
    pub _pad: [u8; 5],
    pub value: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
#[repr(C)]
pub struct Action {
    pub kind: u8,
    pub arg: u8,
    pub arg16: u16,
    pub _pad: u32,
    pub value: u64,
}

/// Pending treasury/strategy intent. Only the strategy-vault program acts on it, within its own
/// bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
#[repr(C)]
pub struct Intent {
    pub kind: u8,
    pub _pad: [u8; 7],
    pub value: u64,
    pub created_at: i64,
    pub sequence: u64,
}

/// On-chain policy body (the `Policy` account minus its discriminator). 352 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
#[repr(C)]
pub struct PolicyData {
    pub market: [u8; 32],
    pub index: u8,
    pub version: u8,
    pub trigger: u8,
    pub combinator: u8,
    pub n_conditions: u8,
    pub n_actions: u8,
    pub enabled: u8,
    pub bump: u8,
    pub cooldown_secs: u32,
    pub deny_reason: u16,
    pub _pad: u16,
    pub conditions: [Condition; MAX_CONDITIONS],
    pub actions: [Action; MAX_ACTIONS],
    pub registers: [u64; MAX_REGISTERS],
    pub exec_count: u64,
    pub last_exec_ts: i64,
    pub intent: Intent,
    pub label_hash: [u8; 32],
}

const _: () = assert!(core::mem::size_of::<PolicyData>() == 352);

/// The user-authored part of a policy (what `create_policy` / `update_policy` accept).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PolicySpec {
    pub trigger: u8,
    pub combinator: u8,
    pub cooldown_secs: u32,
    pub deny_reason: u16,
    pub conditions: [Condition; MAX_CONDITIONS],
    pub n_conditions: u8,
    pub actions: [Action; MAX_ACTIONS],
    pub n_actions: u8,
    pub label_hash: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyError {
    BadVersion,
    BadIndex,
    UnsupportedTrigger,
    BadCombinator,
    TooManyConditions,
    TooManyActions,
    NoActions,
    UnknownField,
    UnknownOperator,
    UnknownAction,
    BadRegister,
    BadStateOp,
    DenyNotAllowedForTrigger,
    PointsNotAllowedForTrigger,
    MultipleDeny,
    DenyReasonTooLarge,
    PointsTooLarge,
    RequiresVolatilityGuard,
    FieldNotAvailable,
    UnusedEntryNotZero,
}

/// Environment facts validation depends on.
#[derive(Clone, Copy, Debug, Default)]
pub struct ValidationEnv {
    pub has_volatility_guard: bool,
}

impl PolicySpec {
    pub fn validate(&self, env: &ValidationEnv) -> Result<(), PolicyError> {
        if !trigger::is_v1(self.trigger) {
            return Err(PolicyError::UnsupportedTrigger);
        }
        if self.combinator > combinator::ANY {
            return Err(PolicyError::BadCombinator);
        }
        let nc = self.n_conditions as usize;
        let na = self.n_actions as usize;
        if nc > MAX_CONDITIONS {
            return Err(PolicyError::TooManyConditions);
        }
        if na > MAX_ACTIONS {
            return Err(PolicyError::TooManyActions);
        }
        if na == 0 {
            return Err(PolicyError::NoActions);
        }
        if self.deny_reason > MAX_DENY_REASON {
            return Err(PolicyError::DenyReasonTooLarge);
        }
        for c in &self.conditions[..nc] {
            if c.field == 0 || c.field > field::MAX {
                return Err(PolicyError::UnknownField);
            }
            if c.op > op::MAX {
                return Err(PolicyError::UnknownOperator);
            }
            match c.field {
                field::HOOK_STATE if c.arg as usize >= MAX_REGISTERS => {
                    return Err(PolicyError::BadRegister)
                }
                field::VOLATILITY if !env.has_volatility_guard => {
                    return Err(PolicyError::RequiresVolatilityGuard)
                }
                field::TREASURY_BALANCE => return Err(PolicyError::FieldNotAvailable),
                _ => {}
            }
        }
        for c in &self.conditions[nc..] {
            if *c != Condition::default() {
                return Err(PolicyError::UnusedEntryNotZero);
            }
        }
        let before = trigger::is_before(self.trigger);
        let mut denies = 0;
        for a in &self.actions[..na] {
            match a.kind {
                action::NONE => return Err(PolicyError::UnknownAction),
                k if k > action::MAX => return Err(PolicyError::UnknownAction),
                action::DENY => {
                    if !before {
                        return Err(PolicyError::DenyNotAllowedForTrigger);
                    }
                    denies += 1;
                }
                action::AWARD_POINTS => {
                    if before {
                        return Err(PolicyError::PointsNotAllowedForTrigger);
                    }
                    if a.value > MAX_POINTS_PER_ACTION {
                        return Err(PolicyError::PointsTooLarge);
                    }
                }
                action::UPDATE_STATE => {
                    if a.arg as usize >= MAX_REGISTERS {
                        return Err(PolicyError::BadRegister);
                    }
                    if a.arg16 > state_op::SUB {
                        return Err(PolicyError::BadStateOp);
                    }
                }
                _ => {}
            }
        }
        if denies > 1 {
            return Err(PolicyError::MultipleDeny);
        }
        for a in &self.actions[na..] {
            if *a != Action::default() {
                return Err(PolicyError::UnusedEntryNotZero);
            }
        }
        Ok(())
    }

    /// Write this spec into a policy account body. Registers, intent and cooldown reset so state
    /// from a previous rule never leaks into the new one; `exec_count` is kept for analytics.
    pub fn write_to(&self, p: &mut PolicyData) {
        p.version = POLICY_VERSION;
        p.trigger = self.trigger;
        p.combinator = self.combinator;
        p.n_conditions = self.n_conditions;
        p.n_actions = self.n_actions;
        p.cooldown_secs = self.cooldown_secs;
        p.deny_reason = self.deny_reason;
        p.conditions = self.conditions;
        p.actions = self.actions;
        p.label_hash = self.label_hash;
        p.registers = [0; MAX_REGISTERS];
        p.intent = Intent::default();
        p.last_exec_ts = 0;
    }
}

/// Inputs a policy can read. All derived from accounts in the routed transaction.
pub struct EvalInput<'a> {
    pub ctx: &'a HookContext,
    pub market: &'a MarketStats,
    pub user: &'a UserState,
    /// Deviation of spot from the Volatility Guard EMA in bps, if the guard is installed and warm.
    pub volatility_bps: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PolicyOutcome {
    pub evaluated: bool,
    pub matched: bool,
    /// `Some(reason)` when the policy denies the routed trade.
    pub deny: Option<u16>,
    pub points: u64,
    pub record_tag: Option<u64>,
    pub signal: bool,
    pub intent: bool,
    pub in_cooldown: bool,
    pub flags: u32,
}

impl PolicyOutcome {
    pub fn decision(&self) -> HookDecision {
        match self.deny {
            Some(r) => HookDecision::deny(r),
            None => HookDecision::allow().with_flags(self.flags),
        }
    }
}

fn read_field(c: &Condition, p: &PolicyData, input: &EvalInput<'_>) -> Option<u64> {
    let now = input.ctx.timestamp;
    Some(match c.field {
        field::TRADE_SIZE => input.ctx.quote_amount,
        field::TRADE_SIZE_BASE => input.ctx.base_amount,
        field::ROUTED_VOLUME => input.market.routed_volume(),
        field::VOLATILITY => input.volatility_bps?,
        field::TIME_SINCE_LAUNCH => input.ctx.seconds_since_activation(),
        field::TIME_SINCE_LAST_ACTION => {
            if p.last_exec_ts == 0 {
                u64::MAX
            } else {
                now.saturating_sub(p.last_exec_ts).max(0) as u64
            }
        }
        field::BUY_SELL_RATIO => input.market.buy_sell_ratio_bps(),
        field::HOLDER_DURATION => input.user.holder_duration(now),
        field::HOOK_STATE => p.registers[c.arg as usize % MAX_REGISTERS],
        field::USER_ROUTED_VOLUME => input.user.routed_volume(),
        field::USER_XP => input.user.xp,
        field::USER_BASE_BALANCE => input.ctx.user_base_balance,
        _ => return None,
    })
}

fn compare(lhs: u64, o: u8, rhs: u64) -> bool {
    match o {
        op::EQ => lhs == rhs,
        op::NEQ => lhs != rhs,
        op::GT => lhs > rhs,
        op::GTE => lhs >= rhs,
        op::LT => lhs < rhs,
        op::LTE => lhs <= rhs,
        _ => false,
    }
}

/// Evaluate one policy for one callback. Mutates only the policy's own registers, counters and
/// intent; returns effects (points, records) for the router to apply.
///
/// A condition whose data is unavailable (e.g. volatility before the guard is warm) evaluates to
/// `false`, so a DENY rule on missing data does not fire and an ALLOW-gated rule does not unlock.
pub fn evaluate(p: &mut PolicyData, callback: HookCallback, input: &EvalInput<'_>) -> PolicyOutcome {
    let mut out = PolicyOutcome::default();
    if p.enabled == 0 || p.trigger != callback as u8 {
        return out;
    }
    out.evaluated = true;
    let nc = (p.n_conditions as usize).min(MAX_CONDITIONS);
    let na = (p.n_actions as usize).min(MAX_ACTIONS);

    out.matched = if nc == 0 {
        true
    } else if p.combinator == combinator::ANY {
        p.conditions[..nc]
            .iter()
            .any(|c| read_field(c, p, input).is_some_and(|v| compare(v, c.op, c.value)))
    } else {
        p.conditions[..nc]
            .iter()
            .all(|c| read_field(c, p, input).is_some_and(|v| compare(v, c.op, c.value)))
    };
    if !out.matched {
        return out;
    }

    let now = input.ctx.timestamp;
    out.in_cooldown = p.cooldown_secs > 0
        && p.last_exec_ts != 0
        && now.saturating_sub(p.last_exec_ts) < p.cooldown_secs as i64;

    let mut applied = false;
    for i in 0..na {
        let a = p.actions[i];
        match a.kind {
            action::DENY => {
                out.deny = Some(reason::POLICY_BASE + p.deny_reason.min(MAX_DENY_REASON));
                return out;
            }
            action::ALLOW => break,
            _ if out.in_cooldown => {}
            action::RECORD => {
                out.record_tag = Some(a.value);
                applied = true;
            }
            action::AWARD_POINTS => {
                out.points = out.points.saturating_add(a.value.min(MAX_POINTS_PER_ACTION));
                out.flags |= decision_flags::POINTS_AWARDED;
                applied = true;
            }
            action::UPDATE_STATE => {
                let r = &mut p.registers[a.arg as usize % MAX_REGISTERS];
                *r = match a.arg16 {
                    state_op::SET => a.value,
                    state_op::ADD => r.saturating_add(a.value),
                    _ => r.saturating_sub(a.value),
                };
                out.flags |= decision_flags::STATE_WRITTEN;
                applied = true;
            }
            action::EMIT_SIGNAL => {
                out.signal = true;
                out.flags |= decision_flags::SIGNAL_EMITTED;
                applied = true;
            }
            k if action::is_intent(k) => {
                p.intent = Intent {
                    kind: k,
                    _pad: [0; 7],
                    value: a.value,
                    created_at: now,
                    sequence: input.ctx.sequence,
                };
                out.intent = true;
                out.flags |= decision_flags::SIGNAL_EMITTED;
                applied = true;
            }
            _ => {}
        }
    }
    if applied {
        p.exec_count = p.exec_count.saturating_add(1);
        p.last_exec_ts = now;
    }
    out
}

#[cfg(test)]
mod tests;
