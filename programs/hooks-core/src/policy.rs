//! The pool environment of the shared hook standard: installing a pool's program hook from a
//! `hook-registry` version, and evaluating a pool's builtin policy stack with `hook-kernel`.

use anchor_lang::prelude::*;
use hook_kernel::{reason, validate_stack, NoProgramHooks, Op, Record};
use hook_registry::{HookVersion, STATUS_ACTIVE};
use hook_standard::{
    cap, Environment, HookStack, IMPL_BUILTIN, IMPL_PROGRAM,
};
use hooks_interface::perm;

use crate::{errors::CoreError, events::PolicyRejected, state::PoolPolicy};

const CALLBACK_BITS: u32 = 0x7ff;
const PRICING: u32 = perm::BEFORE_SWAP_RETURNS_DELTA
    | perm::AFTER_SWAP_RETURNS_DELTA
    | perm::AFTER_ADD_LIQUIDITY_RETURNS_DELTA
    | perm::AFTER_REMOVE_LIQUIDITY_RETURNS_DELTA
    | perm::CUSTOM_CURVE;

/// Capabilities a hook version must declare for the powers a pool grants it. Every pool callback
/// may reject; deltas and custom curves price the trade; a dynamic fee allocates LP fees.
pub fn required_caps(permissions: u32) -> u32 {
    let mut c = cap::VALIDATE_OPERATION | cap::VETO_OPERATION;
    if permissions & PRICING != 0 {
        c |= cap::CUSTOM_PRICING;
    }
    if permissions & perm::DYNAMIC_FEE != 0 {
        c |= cap::REQUEST_AUTHORIZED_FEE_ALLOCATION;
    }
    c
}

/// A pool may install `hook_program` with `permissions` only from an active Pool-environment
/// program version of that program that declares every callback and capability used.
pub fn check_hook_version(version: &HookVersion, hook_program: &Pubkey, permissions: u32) -> Result<()> {
    require!(
        version.status == STATUS_ACTIVE
            && version.impl_kind == IMPL_PROGRAM
            && version.environments & Environment::Pool.bit() != 0
            && version.program == *hook_program,
        CoreError::HookVersionMismatch
    );
    let callbacks = (permissions & CALLBACK_BITS) as u16;
    require!(
        callbacks & !version.callbacks[Environment::Pool as usize] == 0
            && required_caps(permissions) & !version.capabilities == 0,
        CoreError::PermissionsNotDeclared
    );
    Ok(())
}

pub fn parse_stack(bytes: &[u8]) -> Result<Box<HookStack>> {
    let mut s: Box<HookStack> = bytemuck::zeroed_box();
    require!(HookStack::read_compact(bytes, &mut s), CoreError::InvalidPolicy);
    require!(s.environment == Environment::Pool as u8, CoreError::InvalidPolicy);
    Ok(s)
}

/// Validates a pool stack (builtins only) and checks each entry against its registry version.
pub fn check_install<'info>(stack: &mut HookStack, remaining: &'info [AccountInfo<'info>]) -> Result<()> {
    if let Err(e) = validate_stack(stack, 0) {
        msg!("invalid pool policy: {:?}", e);
        return err!(CoreError::InvalidPolicy);
    }
    for e in stack.active() {
        require!(e.impl_kind == IMPL_BUILTIN, CoreError::InvalidPolicy);
        let key = Pubkey::new_from_array(e.hook_version);
        let info = remaining.iter().find(|i| *i.key == key).ok_or(error!(CoreError::MissingVersion))?;
        let version: Account<HookVersion> = Account::try_from(info)?;
        hook_registry::verify_install(&key, &version, e, Environment::Pool)?;
    }
    let len = stack.len as usize;
    for e in &mut stack.entries[..len] {
        e.state = Default::default();
    }
    Ok(())
}

/// The operation as the pool's builtins see it.
pub struct PoolOp {
    pub actor: Pubkey,
    pub asset: Pubkey,
    pub amount: u64,
    pub amount_b: u64,
    pub sequence: u64,
    /// Reserves in mint order: before the operation for `before_*`, after it for `after_*`.
    pub reserves: [u64; 2],
    pub zero_for_one: bool,
}

/// Evaluates the pool's policy (if any) on `callback`. Returns a policy swap fee in ppm, if any
/// entry produced one. `policy` must be the pool's policy account
/// when the pool has one; it is ignored otherwise.
pub fn run<'info>(
    pool_key: Pubkey,
    pool_policy: Pubkey,
    policy: Option<&AccountLoader<'info, PoolPolicy>>,
    callback: u8,
    op: &PoolOp,
    remaining: &[AccountInfo<'info>],
) -> Result<Option<u32>> {
    if pool_policy == Pubkey::default() {
        return Ok(None);
    }
    let loader = policy.ok_or(error!(CoreError::PolicyMismatch))?;
    require_keys_eq!(loader.key(), pool_policy, CoreError::PolicyMismatch);
    let mut p = loader.load_mut()?;
    let datas = hook_kernel::collect_records(remaining);
    let records: Vec<Record> = datas.iter().map(|(o, d)| Record { owner: *o, data: d }).collect();
    let clock = Clock::get()?;
    let kernel_op = Op {
        subject: pool_key,
        actor: op.actor,
        asset: op.asset,
        amount: op.amount,
        amount_b: op.amount_b,
        slot: clock.slot,
        now: clock.unix_timestamp,
        sequence: op.sequence,
        records: &records,
        reserves: op.reserves,
        zero_for_one: op.zero_for_one,
        ..Op::default()
    };
    let outputs = hook_kernel::evaluate(&mut p.stack, callback, &kernel_op, &mut NoProgramHooks).map_err(|r| {
        msg!(
            "pool policy rejected: callback {} entry {} reason {} ({})",
            callback,
            r.index,
            r.reason,
            reason::describe(r.reason)
        );
        emit!(PolicyRejected { pool: pool_key, callback, entry: r.index, reason: r.reason });
        error!(CoreError::PolicyRejected)
    })?;
    // The last fee output wins; the engine clamps it to the pool's fee cap.
    Ok(outputs.iter().flatten().filter(|o| o.kind == hook_standard::output::FEE_BPS).last().map(|o| o.value.min(10_000) as u32 * 100))
}
