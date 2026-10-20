//! PUMP HOOKS composition kernel (`docs/STANDARD.md` §5).
//!
//! One implementation of hook-stack semantics shared by every policy environment (token wrapper,
//! router, smart accounts) and by the off-chain simulator:
//!
//! - [`validate_stack`]: structure, capabilities, dependencies, per-callback CPI bound.
//! - [`evaluate`]: monotone, fail-closed evaluation. Any reject or failure rejects the operation;
//!   no entry can override another's rejection.
//! - [`requires_guard`]: whether a stack change touches a mandatory entry (and so must go through
//!   the environment's guarded, timelocked path).

pub mod builtin;
pub mod cpi;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_library;

use hook_standard::{
    cap, Environment, HookStack, Output, PolicyContext, PolicyResult, StackEntry, ABI_VERSION,
    DECISION_ALLOW, IMPL_BUILTIN, IMPL_PROGRAM, MAX_PROGRAM_HOOKS_PER_CALLBACK, MAX_STACK,
};
use solana_program::pubkey::Pubkey;

pub mod reason {
    pub const MALFORMED: u16 = 1;
    pub const NOT_ALLOWLISTED: u16 = 2;
    pub const SPENDING_LIMIT: u16 = 3;
    pub const TIMELOCK_PENDING: u16 = 4;
    pub const PROPOSAL_REQUIRED: u16 = 5;
    pub const NOT_ENOUGH_APPROVALS: u16 = 6;
    pub const CONDITION_FAILED: u16 = 7;
    pub const SIZE_CAP: u16 = 8;
    pub const OUTSIDE_WINDOW: u16 = 9;
    /// A program hook returned REJECT (its own reason is in [`super::Rejection::hook_reason`]).
    pub const HOOK_REJECTED: u16 = 10;
    /// A program hook failed, returned nothing, or returned a malformed result.
    pub const HOOK_FAILED: u16 = 11;
    pub const UNSUPPORTED_CALLBACK: u16 = 12;
    pub const DENYLISTED: u16 = 13;
    pub const PRICE_IMPACT: u16 = 14;
    pub const PRICE_LIMIT: u16 = 15;
    pub const CREDENTIAL: u16 = 16;
    pub const SLOT_LIMIT: u16 = 17;

    pub fn describe(code: u16) -> &'static str {
        match code {
            MALFORMED => "malformed stack",
            NOT_ALLOWLISTED => "not allowlisted",
            SPENDING_LIMIT => "spending limit exceeded",
            TIMELOCK_PENDING => "timelock not elapsed",
            PROPOSAL_REQUIRED => "operation must go through a timelocked proposal",
            NOT_ENOUGH_APPROVALS => "not enough approvals",
            CONDITION_FAILED => "conditional requirement failed",
            SIZE_CAP => "amount exceeds size cap",
            OUTSIDE_WINDOW => "outside allowed time window",
            HOOK_REJECTED => "hook rejected",
            HOOK_FAILED => "hook failed",
            UNSUPPORTED_CALLBACK => "unsupported callback",
            DENYLISTED => "denylisted",
            PRICE_IMPACT => "price impact above limit",
            PRICE_LIMIT => "price outside limit",
            CREDENTIAL => "no valid credential",
            SLOT_LIMIT => "slot operation limit reached",
            _ => "unknown",
        }
    }
}

/// Capabilities a program hook may be granted in a policy environment. Asset-moving requests and
/// custom pricing exist only in the pool environment, where the core bounds them.
pub const POLICY_PROGRAM_CAPS: u32 = cap::READ_ACCOUNT_STATE
    | cap::READ_TOKEN_BALANCE
    | cap::VALIDATE_OPERATION
    | cap::VETO_OPERATION
    | cap::UPDATE_OWN_STATE
    | cap::PRODUCE_STATE
    | cap::CONSUME_STATE;

/// A state record supplied with the operation. Unverified: builtins verify owner and header.
#[derive(Clone, Copy, Debug)]
pub struct Record<'a> {
    pub owner: Pubkey,
    pub data: &'a [u8],
}

/// An operation, normalized by the environment.
#[derive(Clone, Copy, Debug, Default)]
pub struct Op<'a> {
    pub subject: Pubkey,
    pub actor: Pubkey,
    pub asset: Pubkey,
    pub counterparty: Pubkey,
    pub amount: u64,
    pub amount_b: u64,
    pub slot: u64,
    pub now: i64,
    pub sequence: u64,
    /// Bitmask over the environment's members that approved (signed now or on the proposal).
    pub approvals: u8,
    /// Creation time of the proposal this operation executes, if any.
    pub proposal_created_at: Option<i64>,
    pub records: &'a [Record<'a>],
    /// Pool reserves in mint order (before the operation for `before_*`, after it for
    /// `after_*`); zero when the environment has none.
    pub reserves: [u64; 2],
    /// Swap direction when `reserves` is set: input is mint 0.
    pub zero_for_one: bool,
}

/// Accounts an environment hands to builtins as records: typed state records (header only) and
/// gateway tokens (full data, bounded). Executable accounts are skipped.
pub fn collect_records(accounts: &[solana_program::account_info::AccountInfo]) -> Vec<(Pubkey, Vec<u8>)> {
    use hook_standard::state::{HEADER_LEN, MAGIC};
    accounts
        .iter()
        .filter(|i| !i.executable)
        .filter_map(|i| {
            let d = i.try_borrow_data().ok()?;
            if d.len() >= HEADER_LEN && d[..4] == MAGIC {
                Some((*i.owner, d[..HEADER_LEN].to_vec()))
            } else if *i.owner == builtin::GATEWAY_PROGRAM {
                Some((*i.owner, d[..d.len().min(256)].to_vec()))
            } else {
                None
            }
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow(Option<Output>),
    Reject(u16),
}

impl Verdict {
    pub fn allow(kind: u16, value: u64) -> Self {
        Verdict::Allow(Some(Output {
            kind,
            _pad: [0; 6],
            value,
        }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rejection {
    pub index: u8,
    pub reason: u16,
    pub hook_reason: u16,
}

/// Invokes program hooks. Implemented by environments (CPI) and the simulator.
#[allow(clippy::result_unit_err)]
pub trait ProgramHooks {
    /// Calls the hook and returns its authenticated, strictly-decoded result, or `Err` on any
    /// failure (which the kernel treats as a rejection).
    fn call(
        &mut self,
        index: usize,
        entry: &StackEntry,
        ctx: PolicyContext,
    ) -> Result<PolicyResult, ()>;
}

/// For contexts without program hooks: every program entry fails closed.
pub struct NoProgramHooks;
impl ProgramHooks for NoProgramHooks {
    fn call(&mut self, _: usize, _: &StackEntry, _: PolicyContext) -> Result<PolicyResult, ()> {
        Err(())
    }
}

pub type Outputs = [Option<Output>; MAX_STACK];

/// Runs every entry installed on `callback`, in order. Builtin state is updated in place; callers
/// must not persist the stack if this returns `Err` (on-chain, the transaction aborts).
pub fn evaluate(
    stack: &mut HookStack,
    callback: u8,
    op: &Op,
    hooks: &mut impl ProgramHooks,
) -> Result<Outputs, Rejection> {
    let reject = |index: usize, reason: u16| Rejection {
        index: index as u8,
        reason,
        hook_reason: 0,
    };
    let env = Environment::from_u8(stack.environment).ok_or(reject(0, reason::MALFORMED))?;
    if stack.version != ABI_VERSION || stack.len as usize > MAX_STACK {
        return Err(reject(0, reason::MALFORMED));
    }
    if env.callback_mask() & (1u16 << callback.min(15)) == 0 {
        return Err(reject(0, reason::UNSUPPORTED_CALLBACK));
    }
    let mut outputs: Outputs = [None; MAX_STACK];
    let len = stack.len as usize;
    for i in 0..len {
        let entry = stack.entries[i];
        if !entry.runs_on(callback) {
            continue;
        }
        let output = match entry.impl_kind {
            IMPL_BUILTIN => {
                let deps = builtin::Deps {
                    outputs: &outputs[..i],
                    depends_on: entry.depends_on,
                };
                let e = &mut stack.entries[i];
                match builtin::evaluate(entry.builtin_id, &e.config, &mut e.state, op, &deps) {
                    Verdict::Allow(o) => o,
                    Verdict::Reject(r) => return Err(reject(i, r)),
                }
            }
            IMPL_PROGRAM => {
                let ctx = policy_context(env, callback, i, &entry, op, &outputs[..i]);
                let r = hooks
                    .call(i, &entry, ctx)
                    .map_err(|_| reject(i, reason::HOOK_FAILED))?;
                if r.decision != DECISION_ALLOW {
                    return Err(Rejection {
                        index: i as u8,
                        reason: reason::HOOK_REJECTED,
                        hook_reason: r.reason,
                    });
                }
                (r.output_kind != 0).then_some(Output {
                    kind: r.output_kind,
                    _pad: [0; 6],
                    value: r.output_value,
                })
            }
            _ => return Err(reject(i, reason::MALFORMED)),
        };
        outputs[i] = output;
    }
    Ok(outputs)
}

/// The context passed to program hooks. `inputs` carries outputs of (at most two) declared
/// dependencies, lowest index first; `authority_bump` is filled in by the environment.
pub fn policy_context(
    env: Environment,
    callback: u8,
    index: usize,
    entry: &StackEntry,
    op: &Op,
    earlier: &[Option<Output>],
) -> PolicyContext {
    let mut inputs = [Output::default(); 2];
    let deps = earlier
        .iter()
        .enumerate()
        .filter(|(j, _)| entry.depends_on & (1 << j) != 0)
        .filter_map(|(_, o)| *o);
    for (slot, o) in inputs.iter_mut().zip(deps) {
        *slot = o;
    }
    PolicyContext {
        version: ABI_VERSION,
        environment: env as u8,
        callback,
        authority_bump: 0,
        entry_index: index as u8,
        _pad: [0; 3],
        subject: op.subject.to_bytes(),
        actor: op.actor.to_bytes(),
        asset: op.asset.to_bytes(),
        counterparty: op.counterparty.to_bytes(),
        amount: op.amount,
        amount_b: op.amount_b,
        slot: op.slot,
        unix_timestamp: op.now,
        sequence: op.sequence,
        inputs,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackError {
    Version,
    Environment,
    TooLong,
    /// Entry `i` has unknown impl kind or builtin id.
    UnknownImpl(u8),
    /// Entry `i` runs on no callback, or one not valid for the environment / builtin.
    Callbacks(u8),
    /// Entry `i` was granted capabilities outside what its kind allows.
    Capabilities(u8),
    /// Entry `i` depends on itself or a later entry.
    Dependency(u8),
    /// Entry `i` has an invalid config.
    Config(u8),
    /// Entry `i`: program hook with a zero program id, or builtin with a non-zero one.
    Program(u8),
    /// More than `MAX_PROGRAM_HOOKS_PER_CALLBACK` program hooks on callback `c`.
    TooManyProgramHooks(u8),
    /// Entry `i` repeats an earlier entry's hook version.
    Duplicate(u8),
    /// Unused slot `i` is not zeroed.
    DirtySlot(u8),
}

/// Validates an installation. `members` is the environment's member count (for multisig).
/// Builtin state is not checked here; environments zero it on install.
pub fn validate_stack(stack: &HookStack, members: u8) -> Result<(), StackError> {
    if stack.version != ABI_VERSION {
        return Err(StackError::Version);
    }
    let env = Environment::from_u8(stack.environment).ok_or(StackError::Environment)?;
    let len = stack.len as usize;
    if len > MAX_STACK {
        return Err(StackError::TooLong);
    }
    let mask = env.callback_mask();
    for (i, e) in stack.active().iter().enumerate() {
        let ix = i as u8;
        if e.callbacks == 0 || e.callbacks & !mask != 0 {
            return Err(StackError::Callbacks(ix));
        }
        if e.depends_on >> i != 0 {
            return Err(StackError::Dependency(ix));
        }
        if e.hook_version != [0; 32]
            && stack.active()[..i]
                .iter()
                .any(|p| p.hook_version == e.hook_version)
        {
            return Err(StackError::Duplicate(ix));
        }
        if e._pad != 0 || e._reserved != 0 || e.mandatory > 1 {
            return Err(StackError::Config(ix));
        }
        match e.impl_kind {
            IMPL_BUILTIN => {
                if builtin::name(e.builtin_id).is_none() {
                    return Err(StackError::UnknownImpl(ix));
                }
                if e.program != [0; 32] || e.pinned_slot != 0 {
                    return Err(StackError::Program(ix));
                }
                if e.callbacks & !builtin::supported_callbacks(e.builtin_id, env) != 0 {
                    return Err(StackError::Callbacks(ix));
                }
                if e.granted != builtin::required_caps(e.builtin_id) {
                    return Err(StackError::Capabilities(ix));
                }
                if !builtin::validate_config(e.builtin_id, &e.config, members) {
                    return Err(StackError::Config(ix));
                }
            }
            IMPL_PROGRAM => {
                // A pool's program hook is its single pool-interface hook (fees, deltas, quotes),
                // not a stack entry; pool stacks hold builtins only.
                if e.builtin_id != 0 || env == Environment::Pool {
                    return Err(StackError::UnknownImpl(ix));
                }
                if e.program == [0; 32] {
                    return Err(StackError::Program(ix));
                }
                if e.granted & !POLICY_PROGRAM_CAPS != 0
                    || e.granted & (cap::VALIDATE_OPERATION | cap::VETO_OPERATION) == 0
                {
                    return Err(StackError::Capabilities(ix));
                }
            }
            _ => return Err(StackError::UnknownImpl(ix)),
        }
    }
    for c in 0..16u8 {
        let n = stack
            .active()
            .iter()
            .filter(|e| e.impl_kind == IMPL_PROGRAM && e.runs_on(c))
            .count();
        if n > MAX_PROGRAM_HOOKS_PER_CALLBACK {
            return Err(StackError::TooManyProgramHooks(c));
        }
    }
    for (i, e) in stack.entries.iter().enumerate().skip(len) {
        if *e != bytemuck::Zeroable::zeroed() {
            return Err(StackError::DirtySlot(i as u8));
        }
    }
    Ok(())
}

/// Prepares `new` for installation over `old`: each entry identical in policy to a not-yet-matched
/// `old` entry keeps that entry's builtin state (so re-submitting a stack cannot reset a spending
/// window); every other entry starts with zero state.
pub fn carry_state(old: &HookStack, new: &mut HookStack) {
    let mut taken = [false; MAX_STACK];
    let len = (new.len as usize).min(MAX_STACK);
    for n in new.entries[..len].iter_mut() {
        n.state = [0; 32];
        if let Some(j) =
            (0..old.active().len()).find(|&j| !taken[j] && same_policy(&old.entries[j], n))
        {
            taken[j] = true;
            n.state = old.entries[j].state;
        }
    }
}

fn same_policy(a: &StackEntry, b: &StackEntry) -> bool {
    let mut a = *a;
    let mut b = *b;
    a.state = [0; 32];
    b.state = [0; 32];
    a == b
}

/// True when replacing `old` with `new` removes, reorders, or changes any mandatory entry of
/// `old`. Such changes must go through the environment's guarded path. Adding entries, removing
/// optional ones and promoting optional to mandatory never require the guard.
pub fn requires_guard(old: &HookStack, new: &HookStack) -> bool {
    if old.environment != new.environment {
        return true;
    }
    let mut rest = new.active().iter();
    old.active()
        .iter()
        .filter(|e| e.is_mandatory())
        .any(|m| !rest.any(|n| same_policy(m, n)))
}

/// Sum of builtin compute estimates for `callback`, plus `program_hook_cu` per program hook.
pub fn compute_estimate(stack: &HookStack, callback: u8, program_hook_cu: u32) -> u32 {
    stack
        .active()
        .iter()
        .filter(|e| e.runs_on(callback))
        .map(|e| {
            if e.is_builtin() {
                builtin::compute_estimate(e.builtin_id)
            } else {
                program_hook_cu
            }
        })
        .sum()
}

/// `sha256(stack bytes)`: the installation plan hash.
pub fn plan_hash(stack: &HookStack) -> [u8; 32] {
    solana_program::hash::hash(bytemuck::bytes_of(stack)).to_bytes()
}
