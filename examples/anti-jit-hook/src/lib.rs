//! EXAMPLE program hook: a round-trip (JIT) guard for routed swaps.
//!
//! Installed on a router policy's `before_swap` with `VALIDATE_OPERATION | VETO_OPERATION |
//! UPDATE_OWN_STATE`. For each actor it remembers the slot and input mint of the last routed swap
//! under that policy and refuses a swap with a different input mint (the reverse direction) within
//! `min_slots` of it.
//!
//! Instructions besides the policy call:
//! - `0 configure(min_slots: u64)`: `[authority (signer, writable), policy, config (writable), system]`.
//!   The signer must be the router policy's authority.
//! - `1 register`: `[payer (signer, writable), policy, actor, actor_state (writable), system]`.
//!   Creates the actor's state; anyone may pay for it. Swaps by unregistered actors are refused,
//!   so a client must include `register` before the actor's first routed swap.
//!
//! Limits, stated plainly: router hooks are bypassable (the pool stays directly callable), and an
//! actor can use several wallets. This guards routed flow under one policy, nothing more.

#![allow(unexpected_cfgs, deprecated)]

use hook_sdk::{Call, Decision, Environment, PolicyHook, CUSTOM_REASON_BASE, ROUTER_PROGRAM_ID};
use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, program::invoke_signed, program_error::ProgramError, pubkey::Pubkey, rent::Rent,
    system_instruction, sysvar::Sysvar,
};

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(process_instruction);

pub const CONFIG_SEED: &[u8] = b"config";
pub const ACTOR_SEED: &[u8] = b"actor";
pub const CONFIG_LEN: usize = 8 + 8;
pub const ACTOR_LEN: usize = 8 + 8 + 32;
const CONFIG_TAG: [u8; 8] = *b"ajitcfg1";
const ACTOR_TAG: [u8; 8] = *b"ajitact1";
/// `RoutePolicy.authority` offset in the router's policy account.
const POLICY_AUTHORITY: core::ops::Range<usize> = 8..40;

pub mod reason {
    use super::CUSTOM_REASON_BASE;
    pub const ROUND_TRIP: u16 = CUSTOM_REASON_BASE;
    pub const NOT_REGISTERED: u16 = CUSTOM_REASON_BASE + 1;
    pub const NOT_CONFIGURED: u16 = CUSTOM_REASON_BASE + 2;
}

pub fn config_pda(program: &Pubkey, policy: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[CONFIG_SEED, policy.as_ref()], program)
}
pub fn actor_pda(program: &Pubkey, policy: &Pubkey, actor: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[ACTOR_SEED, policy.as_ref(), actor.as_ref()], program)
}

pub struct AntiJit;

impl PolicyHook for AntiJit {
    const ENVIRONMENT: Environment = Environment::Router;
    const ENVIRONMENT_PROGRAM: Pubkey = ROUTER_PROGRAM_ID;

    fn evaluate(program_id: &Pubkey, call: &Call, accounts: &[AccountInfo]) -> Result<Decision, ProgramError> {
        let policy = call.subject();
        let Some(config) = hook_sdk::owned(program_id, accounts, &config_pda(program_id, &policy).0) else {
            return Ok(Decision::reject(reason::NOT_CONFIGURED));
        };
        let min_slots = {
            let d = config.try_borrow_data()?;
            if d.len() != CONFIG_LEN || d[..8] != CONFIG_TAG {
                return Err(ProgramError::InvalidAccountData);
            }
            u64::from_le_bytes(d[8..16].try_into().unwrap())
        };
        let Some(state) = hook_sdk::owned(program_id, accounts, &actor_pda(program_id, &policy, &call.actor()).0) else {
            return Ok(Decision::reject(reason::NOT_REGISTERED));
        };
        if !state.is_writable {
            return Err(ProgramError::InvalidAccountData);
        }
        let mut d = state.try_borrow_mut_data()?;
        if d.len() != ACTOR_LEN || d[..8] != ACTOR_TAG {
            return Err(ProgramError::InvalidAccountData);
        }
        let last_slot = u64::from_le_bytes(d[8..16].try_into().unwrap());
        let last_mint = &d[16..48];
        let reverses = last_slot != 0 && last_mint != call.asset().as_ref();
        if reverses && call.slot().saturating_sub(last_slot) < min_slots {
            return Ok(Decision::reject(reason::ROUND_TRIP));
        }
        d[8..16].copy_from_slice(&call.slot().to_le_bytes());
        d[16..48].copy_from_slice(call.asset().as_ref());
        Ok(Decision::allow())
    }
}

pub fn process_instruction(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    if hook_sdk::process::<AntiJit>(program_id, accounts, data)? {
        return Ok(());
    }
    match data.split_first() {
        Some((0, rest)) if rest.len() == 8 => configure(program_id, accounts, u64::from_le_bytes(rest.try_into().unwrap())),
        Some((1, [])) => register(program_id, accounts),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

/// Creates a PDA, also when someone pre-funded its address (which makes `create_account` fail).
fn create<'info>(program_id: &Pubkey, payer: &AccountInfo<'info>, target: &AccountInfo<'info>, system: &AccountInfo<'info>, len: usize, seeds: &[&[u8]]) -> ProgramResult {
    let rent = Rent::get()?.minimum_balance(len);
    let infos = [payer.clone(), target.clone(), system.clone()];
    if target.lamports() == 0 {
        return invoke_signed(&system_instruction::create_account(payer.key, target.key, rent, len as u64, program_id), &infos, &[seeds]);
    }
    if rent > target.lamports() {
        solana_program::program::invoke(&system_instruction::transfer(payer.key, target.key, rent - target.lamports()), &infos)?;
    }
    invoke_signed(&system_instruction::allocate(target.key, len as u64), &infos, &[seeds])?;
    invoke_signed(&system_instruction::assign(target.key, program_id), &infos, &[seeds])
}

fn configure(program_id: &Pubkey, accounts: &[AccountInfo], min_slots: u64) -> ProgramResult {
    let [authority, policy, config, system, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !authority.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if policy.owner != &ROUTER_PROGRAM_ID || policy.try_borrow_data()?.get(POLICY_AUTHORITY) != Some(authority.key.as_ref()) {
        return Err(ProgramError::IllegalOwner);
    }
    let (expected, bump) = config_pda(program_id, policy.key);
    if *config.key != expected {
        return Err(ProgramError::InvalidSeeds);
    }
    if config.data_is_empty() {
        create(program_id, authority, config, system, CONFIG_LEN, &[CONFIG_SEED, policy.key.as_ref(), &[bump]])?;
    } else if config.owner != program_id {
        return Err(ProgramError::IllegalOwner);
    }
    let mut d = config.try_borrow_mut_data()?;
    d[..8].copy_from_slice(&CONFIG_TAG);
    d[8..16].copy_from_slice(&min_slots.to_le_bytes());
    Ok(())
}

fn register(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let [payer, policy, actor, state, system, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !payer.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if policy.owner != &ROUTER_PROGRAM_ID {
        return Err(ProgramError::IllegalOwner);
    }
    let (expected, bump) = actor_pda(program_id, policy.key, actor.key);
    if *state.key != expected {
        return Err(ProgramError::InvalidSeeds);
    }
    if !state.data_is_empty() {
        return Ok(());
    }
    create(program_id, payer, state, system, ACTOR_LEN, &[ACTOR_SEED, policy.key.as_ref(), actor.key.as_ref(), &[bump]])?;
    state.try_borrow_mut_data()?[..8].copy_from_slice(&ACTOR_TAG);
    Ok(())
}
