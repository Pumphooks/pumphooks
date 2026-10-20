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
pub const DENYLIST: u8 = 9;
pub const DYNAMIC_FEE: u8 = 10;
pub const PRICE_IMPACT: u8 = 11;
pub const PRICE_LIMIT: u8 = 12;
pub const CREDENTIAL: u8 = 13;
pub const SLOT_LIMIT: u8 = 14;

pub const IDS: [u8; 14] = [
    ALLOWLIST,
    SPENDING_LIMIT,
    TIMELOCK,
    MULTISIG,
    CONDITIONAL,
    MAX_SIZE,
    TIME_WINDOW,
    ACCOUNTING,
    DENYLIST,
    DYNAMIC_FEE,
    PRICE_IMPACT,
    PRICE_LIMIT,
    CREDENTIAL,
    SLOT_LIMIT,
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
        DENYLIST => "denylist",
        DYNAMIC_FEE => "dynamic-fee",
        PRICE_IMPACT => "price-impact",
        PRICE_LIMIT => "price-limit",
        CREDENTIAL => "credential",
        SLOT_LIMIT => "slot-limit",
        _ => return None,
    })
}

/// Callbacks (bitmask) a builtin may be installed on, per environment. Zero = unsupported.
pub fn supported_callbacks(id: u8, env: Environment) -> u16 {
    use Environment::*;
    // Account: 0 before_deposit 1 after_deposit 2 before_withdraw 3 after_withdraw
    //          4 before_execute 5 after_execute 6 before_authority_change 7 after_authority_change
    // Router:  0 before_execute 1 after_execute 2 before_swap 3 after_swap
    // Token:   0 before_transfer (token-hooks, called by Token-2022) 2 before_wrap
    //          3 before_unwrap (token-wrapper); 1 after_transfer has no enforcement point.
    // Pool:    2 before_add_liquidity 6 before_swap 8 before_donate gate entry; no builtin may
    //          run on 4 before_remove_liquidity, so a pool policy can never trap LP funds.
    let before_account = 0b0101_0101;
    let token_gates = 0b1101;
    match (id, env) {
        (ALLOWLIST | CONDITIONAL | TIME_WINDOW | DENYLIST | CREDENTIAL, Pool) => POOL_ENTRY,
        (SPENDING_LIMIT, Pool) => 1 << 6,
        (MAX_SIZE, Pool) => (1 << 2) | (1 << 6),
        (ACCOUNTING, Pool) => POOL_AFTER,
        (ALLOWLIST | DENYLIST | CREDENTIAL, Account) => before_account,
        (ALLOWLIST | DENYLIST | CREDENTIAL, Router) => 0b0101,
        (ALLOWLIST | DENYLIST | CREDENTIAL, Token) => token_gates,
        (SPENDING_LIMIT, Account) => (1 << 2) | (1 << 5),
        // Stateful: one callback per swap so a route is counted once.
        (SPENDING_LIMIT, Router) => 1 << 2,
        // A per-mint window (all holders together): the only limit a mint-level state can enforce.
        (SPENDING_LIMIT, Token) => 1 << 0,
        (TIMELOCK, Account) => (1 << 2) | (1 << 4) | (1 << 6),
        (TIMELOCK, Token) => 0b1001,
        (MULTISIG, Account) => (1 << 2) | (1 << 4) | (1 << 6),
        (CONDITIONAL, Account) => before_account | (1 << 5),
        (CONDITIONAL, Router) => 0b0101,
        (CONDITIONAL, Token) => token_gates,
        (MAX_SIZE, Account) => (1 << 2) | (1 << 5),
        (MAX_SIZE, Router) => 0b0101,
        (MAX_SIZE, Token) => token_gates,
        (TIME_WINDOW, Account | Router | Token) => env.callback_mask() & !odd_after(env),
        (ACCOUNTING, Account | Router) => env.callback_mask(),
        (ACCOUNTING, Token) => token_gates,
        (DYNAMIC_FEE | PRICE_IMPACT, Pool) => 1 << 6,
        (PRICE_LIMIT, Pool) => 1 << 7,
        (PRICE_LIMIT, Router) => 1 << 3,
        // Once per swap / add, so an operation is counted once.
        (SLOT_LIMIT, Pool) => (1 << 2) | (1 << 6),
        (SLOT_LIMIT, Router) => 1 << 2,
        _ => 0,
    }
}

const POOL_ENTRY: u16 = (1 << 2) | (1 << 6) | (1 << 8);
const POOL_AFTER: u16 = (1 << 3) | (1 << 5) | (1 << 7) | (1 << 9);

/// "after_*" callbacks of the router/account environments (odd ids).
fn odd_after(env: Environment) -> u16 {
    match env {
        Environment::Account => 0b1010_1010,
        Environment::Router => 0b1010,
        Environment::Token => 0b0010,
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
        DENYLIST => veto,
        DYNAMIC_FEE => cap::CUSTOM_PRICING | cap::UPDATE_OWN_STATE,
        PRICE_IMPACT | PRICE_LIMIT => veto | cap::READ_ACCOUNT_STATE,
        CREDENTIAL => veto | cap::READ_ACCOUNT_STATE | cap::CONSUME_STATE,
        SLOT_LIMIT => veto | cap::UPDATE_OWN_STATE,
        _ => u32::MAX,
    }
}

/// Static compute estimate (CU) per evaluation, measured on SBF via `tests/kernel`.
pub fn compute_estimate(id: u8) -> u32 {
    match id {
        ALLOWLIST => 2_500,
        CONDITIONAL => 800,
        CREDENTIAL => 1_500,
        DYNAMIC_FEE | PRICE_IMPACT | PRICE_LIMIT => 600,
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

/// Rejects when the target is one of the listed keys (zero = unused; at least one set). Inline
/// only: a record-based denylist could be bypassed by not supplying the record.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct DenylistConfig {
    pub target: u8,
    pub _pad: [u8; 7],
    pub key_a: [u8; 32],
    pub key_b: [u8; 32],
    pub _rest: [u8; 24],
}

pub const FEE_MODE_SIZE: u8 = 0;
pub const FEE_MODE_VOLATILITY: u8 = 1;

/// Swap fee in basis points, output as `FEE_BPS` for the pool to apply (clamped to its cap).
///
/// - Size mode: `base + (max − base)·min(amount, pivot)/pivot` (the size ramp studied in Patcha,
///   MIT).
/// - Volatility mode: `base + deviation_bps·sensitivity_bps/10_000`, capped at `max`, where
///   `deviation_bps` is the distance of the pre-swap pool price from an EMA kept in entry state,
///   updated by `alpha_bps` per swap.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct DynamicFeeConfig {
    pub mode: u8,
    pub _pad: [u8; 3],
    pub base_bps: u16,
    pub max_bps: u16,
    pub pivot: u64,
    pub alpha_bps: u16,
    pub _pad2: u16,
    pub sensitivity_bps: u32,
    pub _rest: [u8; 72],
}

/// Price of token 1 in token 0, Q64.64, and the slot it was last updated.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct DynamicFeeState {
    pub ema: u128,
    pub last_slot: u64,
    pub _rest: [u8; 8],
}

/// Rejects swaps whose constant-product price impact `amount / (reserve_in + amount)` exceeds
/// `max_bps`. Needs live reserves from the environment (pools); fails closed without them.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PriceImpactConfig {
    pub max_bps: u16,
    pub _pad: [u8; 6],
    pub _rest: [u8; 88],
}

pub const LIMIT_REALIZED: u8 = 1;
pub const LIMIT_POOL_PRICE: u8 = 2;

/// Price bounds as fractions (`num/den`, `max_num = 0` = no upper bound).
///
/// - `LIMIT_REALIZED`: `amount_b / amount` (output per input, measured after the swap) within
///   bounds. Only when the input asset is `asset` (zero = any).
/// - `LIMIT_POOL_PRICE`: post-swap pool price `reserve_1 / reserve_0` within bounds (range orders).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PriceLimitConfig {
    pub min_num: u64,
    pub min_den: u64,
    pub max_num: u64,
    pub max_den: u64,
    pub asset: [u8; 32],
    pub mode: u8,
    pub _pad: [u8; 7],
    pub _rest: [u8; 24],
}

/// Identity.com gateway program (Civic Pass): `gatem74V238djXdzWnJf94Wo1DcnuGkfijbf3AuBhfs`.
pub const GATEWAY_PROGRAM: Pubkey = solana_program::pubkey!("gatem74V238djXdzWnJf94Wo1DcnuGkfijbf3AuBhfs");
pub const CREDENTIAL_ALLOW_NO_EXPIRY: u8 = 1;

/// The target must hold an active gateway token of `gatekeeper_network`, owned by
/// [`GATEWAY_PROGRAM`] and not expired. Tokens without an expiry are
/// accepted only with `CREDENTIAL_ALLOW_NO_EXPIRY`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CredentialConfig {
    pub target: u8,
    pub flags: u8,
    pub _pad: [u8; 6],
    pub gatekeeper_network: [u8; 32],
    /// Reserved (zero).
    pub _reserved: [u8; 32],
    pub _rest: [u8; 24],
}

pub const SLOT_LIMIT_PER_ACTOR: u8 = 1;

/// At most `max_per_slot` operations on the subject per slot; with `SLOT_LIMIT_PER_ACTOR`, an actor
/// that already operated in this slot (one of the first two actors seen) is rejected. Only sees
/// operations through this subject.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SlotLimitConfig {
    pub max_per_slot: u32,
    pub flags: u8,
    pub _pad: [u8; 3],
    pub _rest: [u8; 88],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SlotLimitState {
    pub slot: u64,
    pub count: u32,
    pub _pad: u32,
    /// First 8 bytes of the first two actors seen this slot.
    pub actors: [[u8; 8]; 2],
}

const _: () = {
    assert!(core::mem::size_of::<DenylistConfig>() == CONFIG_LEN);
    assert!(core::mem::size_of::<DynamicFeeConfig>() == CONFIG_LEN);
    assert!(core::mem::size_of::<DynamicFeeState>() == ENTRY_STATE_LEN);
    assert!(core::mem::size_of::<PriceImpactConfig>() == CONFIG_LEN);
    assert!(core::mem::size_of::<PriceLimitConfig>() == CONFIG_LEN);
    assert!(core::mem::size_of::<CredentialConfig>() == CONFIG_LEN);
    assert!(core::mem::size_of::<SlotLimitConfig>() == CONFIG_LEN);
    assert!(core::mem::size_of::<SlotLimitState>() == ENTRY_STATE_LEN);
};

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
        DENYLIST => {
            let c: DenylistConfig = read(config);
            c.target <= TARGET_COUNTERPARTY && (c.key_a != [0; 32] || c.key_b != [0; 32])
        }
        DYNAMIC_FEE => {
            let c: DynamicFeeConfig = read(config);
            c.base_bps <= c.max_bps
                && c.max_bps <= 10_000
                && match c.mode {
                    FEE_MODE_SIZE => c.pivot > 0,
                    FEE_MODE_VOLATILITY => {
                        c.alpha_bps > 0 && c.alpha_bps <= 10_000 && c.sensitivity_bps > 0
                    }
                    _ => false,
                }
        }
        PRICE_IMPACT => {
            let c: PriceImpactConfig = read(config);
            c.max_bps > 0 && c.max_bps < 10_000
        }
        PRICE_LIMIT => {
            let c: PriceLimitConfig = read(config);
            let lower = c.min_den > 0;
            let upper = c.max_num > 0 && c.max_den > 0;
            let ordered = !(lower && upper)
                || (c.min_num as u128) * (c.max_den as u128)
                    <= (c.max_num as u128) * (c.min_den as u128);
            c.mode & !(LIMIT_REALIZED | LIMIT_POOL_PRICE) == 0
                && c.mode != 0
                && (lower || upper)
                && (c.max_num == 0) == (c.max_den == 0)
                && ordered
        }
        CREDENTIAL => {
            let c: CredentialConfig = read(config);
            c.target <= TARGET_COUNTERPARTY
                && c.flags & !CREDENTIAL_ALLOW_NO_EXPIRY == 0
                && c.gatekeeper_network != [0; 32]
                && c._reserved == [0; 32]
        }
        SLOT_LIMIT => {
            let c: SlotLimitConfig = read(config);
            c.max_per_slot > 0 && c.flags & !SLOT_LIMIT_PER_ACTOR == 0
        }
        _ => false,
    }
}

/// `reserve_1 / reserve_0` as Q64.64; `None` when a reserve is zero.
pub fn pool_price_q64(reserves: [u64; 2]) -> Option<u128> {
    if reserves[0] == 0 || reserves[1] == 0 {
        return None;
    }
    Some(((reserves[1] as u128) << 64) / reserves[0] as u128)
}

/// `num_a/den_a >= num_b/den_b` without division.
fn frac_ge(num_a: u64, den_a: u64, num_b: u64, den_b: u64) -> bool {
    (num_a as u128) * (den_b as u128) >= (num_b as u128) * (den_a as u128)
}

/// Decoded Identity.com gateway token (Borsh layout of `solana-gateway` 0.5.0, Apache-2.0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GatewayToken {
    pub owner_wallet: Pubkey,
    pub gatekeeper_network: Pubkey,
    /// 0 active, 1 frozen, 2 revoked.
    pub state: u8,
    pub expire_time: Option<i64>,
}

pub fn parse_gateway_token(d: &[u8]) -> Option<GatewayToken> {
    let mut at = 1usize; // version
    let skip_opt_key = |at: &mut usize| -> Option<()> {
        match *d.get(*at)? {
            0 => *at += 1,
            1 => *at += 33,
            _ => return None,
        }
        Some(())
    };
    let key = |at: &mut usize| -> Option<Pubkey> {
        let k = Pubkey::new_from_array(d.get(*at..*at + 32)?.try_into().ok()?);
        *at += 32;
        Some(k)
    };
    skip_opt_key(&mut at)?;
    let owner_wallet = key(&mut at)?;
    skip_opt_key(&mut at)?;
    let gatekeeper_network = key(&mut at)?;
    key(&mut at)?; // issuing gatekeeper
    let state = *d.get(at)?;
    if state > 2 {
        return None;
    }
    at += 1;
    let expire_time = match *d.get(at)? {
        0 => None,
        1 => Some(i64::from_le_bytes(d.get(at + 1..at + 9)?.try_into().ok()?)),
        _ => return None,
    };
    Some(GatewayToken { owner_wallet, gatekeeper_network, state, expire_time })
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
        DENYLIST => {
            let c: DenylistConfig = read(config);
            let target = if c.target == TARGET_ACTOR { op.actor } else { op.counterparty }.to_bytes();
            if (c.key_a != [0; 32] && target == c.key_a) || (c.key_b != [0; 32] && target == c.key_b) {
                Verdict::Reject(reason::DENYLISTED)
            } else {
                Verdict::Allow(None)
            }
        }
        DYNAMIC_FEE => {
            let c: DynamicFeeConfig = read(config);
            let span = (c.max_bps - c.base_bps) as u128;
            let fee = match c.mode {
                FEE_MODE_SIZE => {
                    let x = op.amount.min(c.pivot) as u128;
                    c.base_bps as u128 + span * x / c.pivot as u128
                }
                _ => {
                    let Some(price) = pool_price_q64(op.reserves) else {
                        return Verdict::allow(output::FEE_BPS, c.base_bps as u64);
                    };
                    let mut s: DynamicFeeState = read(state);
                    let fee = if s.ema == 0 {
                        c.base_bps as u128
                    } else {
                        let dev = price.abs_diff(s.ema).saturating_mul(10_000) / s.ema;
                        let extra = dev.saturating_mul(c.sensitivity_bps as u128) / 10_000;
                        (c.base_bps as u128).saturating_add(extra).min(c.max_bps as u128)
                    };
                    let step = |d: u128| (d / 10_000) * c.alpha_bps as u128 + (d % 10_000) * c.alpha_bps as u128 / 10_000;
                    s.ema = if s.ema == 0 {
                        price
                    } else if price >= s.ema {
                        s.ema + step(price - s.ema)
                    } else {
                        s.ema - step(s.ema - price)
                    };
                    s.last_slot = op.slot;
                    state.copy_from_slice(bytemuck::bytes_of(&s));
                    fee
                }
            };
            Verdict::allow(output::FEE_BPS, fee as u64)
        }
        PRICE_IMPACT => {
            let c: PriceImpactConfig = read(config);
            let reserve_in = op.reserves[if op.zero_for_one { 0 } else { 1 }];
            if reserve_in == 0 {
                return Verdict::Reject(reason::PRICE_IMPACT);
            }
            let impact = op.amount as u128 * 10_000 / (reserve_in as u128 + op.amount as u128);
            if impact > c.max_bps as u128 {
                Verdict::Reject(reason::PRICE_IMPACT)
            } else {
                Verdict::allow(output::PRICE_IMPACT_BPS, impact as u64)
            }
        }
        PRICE_LIMIT => {
            let c: PriceLimitConfig = read(config);
            let within = |num: u64, den: u64| {
                (c.min_den == 0 || frac_ge(num, den, c.min_num, c.min_den))
                    && (c.max_den == 0 || frac_ge(c.max_num, c.max_den, num, den))
            };
            if c.mode & LIMIT_REALIZED != 0 && (c.asset == [0; 32] || op.asset.to_bytes() == c.asset) {
                if op.amount == 0 || !within(op.amount_b, op.amount) {
                    return Verdict::Reject(reason::PRICE_LIMIT);
                }
            }
            if c.mode & LIMIT_POOL_PRICE != 0 {
                let [r0, r1] = op.reserves;
                if r0 == 0 || !within(r1, r0) {
                    return Verdict::Reject(reason::PRICE_LIMIT);
                }
            }
            Verdict::Allow(None)
        }
        CREDENTIAL => {
            let c: CredentialConfig = read(config);
            let target = if c.target == TARGET_ACTOR { op.actor } else { op.counterparty };
            let program = GATEWAY_PROGRAM;
            let network = Pubkey::new_from_array(c.gatekeeper_network);
            let valid = op.records.iter().filter(|r| r.owner == program).filter_map(|r| parse_gateway_token(r.data)).find(|t| {
                t.owner_wallet == target
                    && t.gatekeeper_network == network
                    && t.state == 0
                    && match t.expire_time {
                        Some(e) => op.now < e,
                        None => c.flags & CREDENTIAL_ALLOW_NO_EXPIRY != 0,
                    }
            });
            match valid {
                Some(t) => Verdict::allow(output::CREDENTIAL_EXPIRY, t.expire_time.unwrap_or(i64::MAX) as u64),
                None => Verdict::Reject(reason::CREDENTIAL),
            }
        }
        SLOT_LIMIT => {
            let c: SlotLimitConfig = read(config);
            let mut s: SlotLimitState = read(state);
            if s.slot != op.slot {
                s = SlotLimitState::zeroed();
                s.slot = op.slot;
            }
            let me: [u8; 8] = op.actor.to_bytes()[..8].try_into().unwrap();
            let seen = s.actors[..(s.count as usize).min(2)].contains(&me);
            if s.count >= c.max_per_slot || (c.flags & SLOT_LIMIT_PER_ACTOR != 0 && seen) {
                return Verdict::Reject(reason::SLOT_LIMIT);
            }
            if (s.count as usize) < 2 {
                s.actors[s.count as usize] = me;
            }
            s.count += 1;
            state.copy_from_slice(bytemuck::bytes_of(&s));
            Verdict::allow(output::SLOT_COUNT, s.count as u64)
        }
        _ => Verdict::Reject(reason::MALFORMED),
    }
}
