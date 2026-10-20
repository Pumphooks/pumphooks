//! Builtin policies: restricted, CPI-free hooks evaluated inside the environment program.
//!
//! Every config is a fixed little-endian layout inside `StackEntry::config` (96 bytes); every
//! piece of mutable state lives in `StackEntry::state` (32 bytes).

use bytemuck::{Pod, Zeroable};
use hook_standard::{cap, output, Environment, Output, CONFIG_LEN, ENTRY_STATE_LEN};
use solana_program::pubkey::Pubkey;

use crate::{reason, Op, Verdict};

pub const ALLOWLIST: u8 = 1;
pub const SPENDING_LIMIT: u8 = 2;
pub const TIMELOCK: u8 = 3;
pub const MULTISIG: u8 = 4;
pub const CONDITIONAL: u8 = 5;
pub const MAX_SIZE: u8 = 6;
pub const TIME_WINDOW: u8 = 7;
pub const ACCOUNTING: u8 = 8;

pub const IDS: [u8; 8] = [
    ALLOWLIST,
    SPENDING_LIMIT,
    TIMELOCK,
    MULTISIG,
    CONDITIONAL,
    MAX_SIZE,
    TIME_WINDOW,
    ACCOUNTING,
];

pub fn name(id: u8) -> Option<&'static str> {
    Some(match id {
        ALLOWLIST => "allowlist",
        SPENDING_LIMIT => "spending-limit",
        TIMELOCK => "timelock",
        MULTISIG => "multisig",
        CONDITIONAL => "conditional",
        MAX_SIZE => "max-size",
        TIME_WINDOW => "time-window",
        ACCOUNTING => "accounting",
        _ => return None,
    })
}

/// Callbacks (bitmask) a builtin may be installed on, per environment. Zero = unsupported.
pub fn supported_callbacks(id: u8, env: Environment) -> u16 {
    use Environment::*;
    // Account: 0 before_deposit 1 after_deposit 2 before_withdraw 3 after_withdraw
    //          4 before_execute 5 after_execute 6 before_authority_change 7 after_authority_change
    // Router:  0 before_execute 1 after_execute 2 before_swap 3 after_swap
    // Token:   2 before_wrap 3 before_unwrap (0 before_transfer is the Token-2022 hook program)
    let before_account = 0b0101_0101;
    match (id, env) {
        (ALLOWLIST, Account) => before_account,
        (ALLOWLIST, Router) => 0b0101,
        (ALLOWLIST, Token) => 0b1100,
        (SPENDING_LIMIT, Account) => (1 << 2) | (1 << 5),
        (SPENDING_LIMIT, Router) => 0b0101,
        (TIMELOCK, Account) => (1 << 2) | (1 << 4) | (1 << 6),
        (TIMELOCK, Token) => 0b1000,
        (MULTISIG, Account) => (1 << 2) | (1 << 4) | (1 << 6),
        (CONDITIONAL, Account) => before_account | (1 << 5),
        (CONDITIONAL, Router) => 0b0101,
        (MAX_SIZE, Account) => (1 << 2) | (1 << 5),
        (MAX_SIZE, Router) => 0b0101,
        (MAX_SIZE, Token) => 0b1100,
        (TIME_WINDOW, Account | Router | Token) => env.callback_mask() & !odd_after(env),
        (ACCOUNTING, Account | Router) => env.callback_mask(),
        (ACCOUNTING, Token) => 0b1100,
        _ => 0,
    }
}

/// "after_*" callbacks of the router/account environments (odd ids).
fn odd_after(env: Environment) -> u16 {
    match env {
        Environment::Account => 0b1010_1010,
        Environment::Router => 0b1010,
        _ => 0,
    }
}

/// Capabilities a builtin needs; installs must grant exactly these.
pub fn required_caps(id: u8) -> u32 {
    let veto = cap::VALIDATE_OPERATION | cap::VETO_OPERATION;
    match id {
        ALLOWLIST => veto | cap::READ_ACCOUNT_STATE | cap::CONSUME_STATE,
        SPENDING_LIMIT => veto | cap::UPDATE_OWN_STATE,
        TIMELOCK | MULTISIG | MAX_SIZE | TIME_WINDOW => veto,
        CONDITIONAL => veto | cap::READ_ACCOUNT_STATE,
        ACCOUNTING => cap::UPDATE_OWN_STATE,
        _ => u32::MAX,
    }
}

/// Static compute estimate (CU) per evaluation, measured on SBF via `tests/kernel`.
pub fn compute_estimate(id: u8) -> u32 {
    match id {
        ALLOWLIST => 2_500,
        CONDITIONAL => 800,
        _ => 300,
    }
}

// ---------------------------------------------------------------- configs

pub const TARGET_ACTOR: u8 = 0;
pub const TARGET_COUNTERPARTY: u8 = 1;
pub const SOURCE_INLINE: u8 = 0;
pub const SOURCE_RECORD: u8 = 1;
pub const ALLOWLIST_SCHEMA: u16 = 1;
pub const ALLOWLIST_NAMESPACE_TAG: &[u8] = b"allowlist:v1";

/// Inline: `key_a`/`key_b` are the allowed keys (zero = unused).
/// Record: `key_a` = producer program, `key_b` = required issuer (list authority); an allowlist
/// state record for the target must exist (schema 1, namespace `allowlist:v1`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct AllowlistConfig {
    pub target: u8,
    pub source: u8,
    pub _pad: [u8; 6],
    pub max_age_slots: u64,
    pub key_a: [u8; 32],
    pub key_b: [u8; 32],
    pub _rest: [u8; 16],
}

/// Limit spend of `asset` (zero = native SOL) to `limit` per `period_secs` window.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SpendingLimitConfig {
    pub asset: [u8; 32],
    pub limit: u64,
    pub period_secs: u32,
    pub _pad: u32,
    pub _rest: [u8; 48],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SpendingLimitState {
    pub window_start: i64,
    pub spent: u64,
    pub _rest: [u8; 16],
}

/// The operation must come from a proposal at least `delay_secs` old, and not before `not_before`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TimelockConfig {
    pub delay_secs: u32,
    pub _pad: u32,
    pub not_before: i64,
    pub _rest: [u8; 80],
}

/// At least `threshold` of the environment's members selected by `member_mask` approved.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MultisigConfig {
    pub threshold: u8,
    pub member_mask: u8,
    pub _rest: [u8; 94],
}

pub const FIELD_NONE: u8 = 0;
pub const FIELD_AMOUNT: u8 = 1;
pub const FIELD_AMOUNT_B: u8 = 2;
pub const FIELD_TIME: u8 = 3;
pub const FIELD_SLOT: u8 = 4;
pub const FIELD_APPROVALS: u8 = 5;
/// Output of an earlier entry (`dep_index`, `output_kind`); absent output = condition false.
pub const FIELD_DEP_OUTPUT: u8 = 6;
/// Counterparty equals `key` (op: EQ/NE only).
pub const FIELD_COUNTERPARTY: u8 = 7;
/// Asset equals `key` (op: EQ/NE only).
pub const FIELD_ASSET: u8 = 8;

pub const OP_LT: u8 = 0;
pub const OP_LE: u8 = 1;
pub const OP_GT: u8 = 2;
pub const OP_GE: u8 = 3;
pub const OP_EQ: u8 = 4;
pub const OP_NE: u8 = 5;

pub const ROLE_PREMISE: u8 = 0;
pub const ROLE_REQUIREMENT: u8 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, Default)]
pub struct Condition {
    pub field: u8,
    pub op: u8,
    pub role: u8,
    pub dep_index: u8,
    pub output_kind: u16,
    pub _pad: u16,
    pub value: u64,
}

/// IF every premise holds THEN every requirement must hold. Unused slots have `field = 0`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ConditionalConfig {
    pub conditions: [Condition; 4],
    pub key: [u8; 32],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MaxSizeConfig {
    pub asset: [u8; 32],
    pub max_amount: u64,
    pub _rest: [u8; 56],
}

/// Absolute bounds (0 = unbounded) and an optional UTC daily window `[start, end)` in seconds.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TimeWindowConfig {
    pub not_before: i64,
    pub not_after: i64,
    pub daily_start: u32,
    pub daily_end: u32,
    pub _rest: [u8; 72],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct AccountingState {
    pub volume: u128,
    pub count: u64,
    pub _rest: [u8; 8],
}

const _: () = {
    assert!(core::mem::size_of::<AllowlistConfig>() == CONFIG_LEN);
    assert!(core::mem::size_of::<SpendingLimitConfig>() == CONFIG_LEN);
    assert!(core::mem::size_of::<TimelockConfig>() == CONFIG_LEN);
    assert!(core::mem::size_of::<MultisigConfig>() == CONFIG_LEN);
    assert!(core::mem::size_of::<ConditionalConfig>() == CONFIG_LEN);
    assert!(core::mem::size_of::<MaxSizeConfig>() == CONFIG_LEN);
    assert!(core::mem::size_of::<TimeWindowConfig>() == CONFIG_LEN);
    assert!(core::mem::size_of::<SpendingLimitState>() == ENTRY_STATE_LEN);
    assert!(core::mem::size_of::<AccountingState>() == ENTRY_STATE_LEN);
};

fn read<T: Pod>(b: &[u8]) -> T {
    bytemuck::pod_read_unaligned(&b[..core::mem::size_of::<T>()])
}

/// Structural validation of a builtin's config at install time. `members` is the number of
/// members the environment has (for multisig).
pub fn validate_config(id: u8, config: &[u8; CONFIG_LEN], members: u8) -> bool {
    match id {
        ALLOWLIST => {
            let c: AllowlistConfig = read(config);
            c.target <= TARGET_COUNTERPARTY
                && match c.source {
                    SOURCE_INLINE => c.key_a != [0; 32] && c.max_age_slots == 0,
                    SOURCE_RECORD => c.key_a != [0; 32] && c.key_b != [0; 32],
                    _ => false,
                }
        }
        SPENDING_LIMIT => {
            let c: SpendingLimitConfig = read(config);
            c.period_secs > 0
        }
        TIMELOCK => {
            let c: TimelockConfig = read(config);
            c.delay_secs > 0 || c.not_before > 0
        }
        MULTISIG => {
            let c: MultisigConfig = read(config);
            let members_mask = if members >= 8 {
                0xff
            } else {
                (1u8 << members) - 1
            };
            c.threshold > 0
                && c.member_mask & !members_mask == 0
                && c.threshold as u32 <= c.member_mask.count_ones()
        }
        CONDITIONAL => {
            let c: ConditionalConfig = read(config);
            let used: Vec<&Condition> = c
                .conditions
                .iter()
                .filter(|x| x.field != FIELD_NONE)
                .collect();
            used.iter().any(|x| x.role == ROLE_REQUIREMENT)
                && used.iter().all(|x| {
                    x.role <= ROLE_REQUIREMENT
                        && x.op <= OP_NE
                        && match x.field {
                            FIELD_COUNTERPARTY | FIELD_ASSET => x.op >= OP_EQ,
                            f => f <= FIELD_ASSET,
                        }
                })
        }
        MAX_SIZE => read::<MaxSizeConfig>(config).max_amount > 0,
        TIME_WINDOW => {
            let c: TimeWindowConfig = read(config);
            let daily = (c.daily_start, c.daily_end) != (0, 0);
            let daily_ok = !daily
                || (c.daily_start < 86_400
                    && c.daily_end <= 86_400
                    && c.daily_start != c.daily_end);
            (c.not_after == 0 || c.not_after > c.not_before)
                && daily_ok
                && (daily || c.not_before != 0 || c.not_after != 0)
        }
        ACCOUNTING => config.iter().all(|b| *b == 0),
        _ => false,
    }
}

/// Dependency outputs visible to an entry: only from the indices it declared.
pub struct Deps<'a> {
    pub outputs: &'a [Option<Output>],
    pub depends_on: u8,
}

impl Deps<'_> {
    fn get(&self, index: u8, kind: u16) -> Option<u64> {
        if index >= 8 || self.depends_on & (1 << index) == 0 {
            return None;
        }
        self.outputs
            .get(index as usize)?
            .filter(|o| o.kind == kind)
            .map(|o| o.value)
    }
}

fn cmp(op: u8, a: u64, b: u64) -> bool {
    match op {
        OP_LT => a < b,
        OP_LE => a <= b,
        OP_GT => a > b,
        OP_GE => a >= b,
        OP_EQ => a == b,
        OP_NE => a != b,
        _ => false,
    }
}

fn condition_holds(c: &Condition, key: &[u8; 32], op: &Op, deps: &Deps) -> bool {
    let num = |v: Option<u64>| v.is_some_and(|v| cmp(c.op, v, c.value));
    let key_eq = |k: &Pubkey| match c.op {
        OP_EQ => k.to_bytes() == *key,
        OP_NE => k.to_bytes() != *key,
        _ => false,
    };
    match c.field {
        FIELD_AMOUNT => num(Some(op.amount)),
        FIELD_AMOUNT_B => num(Some(op.amount_b)),
        FIELD_TIME => num(u64::try_from(op.now).ok()),
        FIELD_SLOT => num(Some(op.slot)),
        FIELD_APPROVALS => num(Some(op.approvals.count_ones() as u64)),
        FIELD_DEP_OUTPUT => num(deps.get(c.dep_index, c.output_kind)),
        FIELD_COUNTERPARTY => key_eq(&op.counterparty),
        FIELD_ASSET => key_eq(&op.asset),
        _ => false,
    }
}

pub fn evaluate(
    id: u8,
    config: &[u8; CONFIG_LEN],
    state: &mut [u8; ENTRY_STATE_LEN],
    op: &Op,
    deps: &Deps,
) -> Verdict {
    match id {
        ALLOWLIST => {
            let c: AllowlistConfig = read(config);
            let target = if c.target == TARGET_ACTOR {
                op.actor
            } else {
                op.counterparty
            };
            let allowed = match c.source {
                SOURCE_INLINE => {
                    target.to_bytes() == c.key_a
                        || (c.key_b != [0; 32] && target.to_bytes() == c.key_b)
                }
                _ => {
                    let producer = Pubkey::new_from_array(c.key_a);
                    let ns = hook_standard::state::namespace(ALLOWLIST_NAMESPACE_TAG);
                    let expect = hook_standard::state::Expect {
                        producer: &producer,
                        schema: ALLOWLIST_SCHEMA,
                        namespace: &ns,
                        subject: Some(&target),
                        max_age_slots: c.max_age_slots,
                        current_slot: op.slot,
                        min_sequence_exclusive: None,
                    };
                    op.records.iter().any(|r| {
                        hook_standard::state::verify_bytes(&r.owner, r.data, &expect)
                            .is_ok_and(|h| h.issuer == c.key_b)
                    })
                }
            };
            if allowed {
                Verdict::allow(output::PERMISSION_GRANTED, 1)
            } else {
                Verdict::Reject(reason::NOT_ALLOWLISTED)
            }
        }
        SPENDING_LIMIT => {
            let c: SpendingLimitConfig = read(config);
            if op.asset.to_bytes() != c.asset || op.amount == 0 {
                return Verdict::Allow(None);
            }
            let mut s: SpendingLimitState = read(state);
            if op.now >= s.window_start.saturating_add(c.period_secs as i64) {
                s.window_start = op.now;
                s.spent = 0;
            }
            match s.spent.checked_add(op.amount).filter(|v| *v <= c.limit) {
                Some(spent) => {
                    s.spent = spent;
                    state.copy_from_slice(bytemuck::bytes_of(&s));
                    Verdict::allow(output::SPEND_REMAINING, c.limit - spent)
                }
                None => Verdict::Reject(reason::SPENDING_LIMIT),
            }
        }
        TIMELOCK => {
            let c: TimelockConfig = read(config);
            if op.now < c.not_before {
                return Verdict::Reject(reason::TIMELOCK_PENDING);
            }
            if c.delay_secs > 0 {
                let Some(created) = op.proposal_created_at else {
                    return Verdict::Reject(reason::PROPOSAL_REQUIRED);
                };
                let unlock = created.saturating_add(c.delay_secs as i64);
                if op.now < unlock {
                    return Verdict::Reject(reason::TIMELOCK_PENDING);
                }
                return Verdict::allow(output::TIMELOCK_SATISFIED, unlock as u64);
            }
            Verdict::allow(output::TIMELOCK_SATISFIED, c.not_before as u64)
        }
        MULTISIG => {
            let c: MultisigConfig = read(config);
            let n = (op.approvals & c.member_mask).count_ones();
            if n >= c.threshold as u32 {
                Verdict::allow(output::APPROVALS, n as u64)
            } else {
                Verdict::Reject(reason::NOT_ENOUGH_APPROVALS)
            }
        }
        CONDITIONAL => {
            let c: ConditionalConfig = read(config);
            let used = c.conditions.iter().filter(|x| x.field != FIELD_NONE);
            let premises = used
                .clone()
                .filter(|x| x.role == ROLE_PREMISE)
                .all(|x| condition_holds(x, &c.key, op, deps));
            if !premises {
                return Verdict::Allow(None);
            }
            if used
                .filter(|x| x.role == ROLE_REQUIREMENT)
                .all(|x| condition_holds(x, &c.key, op, deps))
            {
                Verdict::allow(output::CONDITION_MET, 1)
            } else {
                Verdict::Reject(reason::CONDITION_FAILED)
            }
        }
        MAX_SIZE => {
            let c: MaxSizeConfig = read(config);
            if op.asset.to_bytes() != c.asset {
                return Verdict::Allow(None);
            }
            if op.amount > c.max_amount {
                Verdict::Reject(reason::SIZE_CAP)
            } else {
                Verdict::allow(output::WITHIN_SIZE_CAP, c.max_amount)
            }
        }
        TIME_WINDOW => {
            let c: TimeWindowConfig = read(config);
            let in_abs = op.now >= c.not_before && (c.not_after == 0 || op.now < c.not_after);
            let in_daily = if (c.daily_start, c.daily_end) == (0, 0) {
                true
            } else {
                let t = op.now.rem_euclid(86_400) as u32;
                if c.daily_start <= c.daily_end {
                    t >= c.daily_start && t < c.daily_end
                } else {
                    t >= c.daily_start || t < c.daily_end
                }
            };
            if in_abs && in_daily {
                Verdict::Allow(None)
            } else {
                Verdict::Reject(reason::OUTSIDE_WINDOW)
            }
        }
        ACCOUNTING => {
            let mut s: AccountingState = read(state);
            s.count = s.count.saturating_add(1);
            s.volume = s.volume.saturating_add(op.amount as u128);
            state.copy_from_slice(bytemuck::bytes_of(&s));
            Verdict::allow(output::RECORDED_COUNT, s.count)
        }
        _ => Verdict::Reject(reason::MALFORMED),
    }
}
