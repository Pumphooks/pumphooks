//! Producer of cross-environment state records (`hook_standard::state`, docs/STANDARD.md §7).
//!
//! An **issuer** (any signer: a compliance key, a DAO, a list authority) publishes a record about
//! a **subject** under a **namespace** and **schema**. The record lives at the PDA
//! `[b"record", issuer, namespace, subject]`, owned by this program, so a consumer that pins
//! `producer = this program` and `issuer = <key>` in its own configuration knows exactly who
//! wrote it. Nobody but the issuer can change or revoke it.
//!
//! Instructions:
//! - `0 publish { namespace [32], schema u16, subject [32], body_len u16, body }`:
//!   `[issuer (signer, writable), record (writable), system]`. Creates or updates the record;
//!   the sequence increases by one on every publish. The body length is fixed at creation.
//! - `1 revoke { namespace [32], subject [32] }`: `[issuer (signer), record (writable)]`.
//!   Clears the magic so `verify_record` fails, and keeps the account so sequence numbers never
//!   repeat after a later re-publish.
//!
//! Consumers read records only through `hook_standard::state::verify_record`; a record whose
//! owner is not the configured producer is rejected regardless of its contents.

#![allow(unexpected_cfgs, deprecated)]

use bytemuck::Zeroable;
use hook_standard::state::{StateHeader, HEADER_LEN, MAGIC};
use solana_program::{
    account_info::AccountInfo, clock::Clock, entrypoint::ProgramResult, program::invoke, program::invoke_signed, program_error::ProgramError, pubkey::Pubkey,
    rent::Rent, system_instruction, sysvar::Sysvar,
};

solana_program::declare_id!("AZ3DwmvABEwrLJF25r94yyA92W4zvHgAwuhxBi8QJHRW");

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(process_instruction);

pub const RECORD_SEED: &[u8] = b"record";
pub const MAX_BODY: usize = 512;

pub fn record_pda(issuer: &Pubkey, namespace: &[u8; 32], subject: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[RECORD_SEED, issuer.as_ref(), namespace, subject.as_ref()], &ID)
}

pub fn publish_data(namespace: &[u8; 32], schema: u16, subject: &Pubkey, body: &[u8]) -> Vec<u8> {
    let mut d = vec![0u8];
    d.extend_from_slice(namespace);
    d.extend_from_slice(&schema.to_le_bytes());
    d.extend_from_slice(subject.as_ref());
    d.extend_from_slice(&(body.len() as u16).to_le_bytes());
    d.extend_from_slice(body);
    d
}

pub fn revoke_data(namespace: &[u8; 32], subject: &Pubkey) -> Vec<u8> {
    let mut d = vec![1u8];
    d.extend_from_slice(namespace);
    d.extend_from_slice(subject.as_ref());
    d
}

pub fn process_instruction(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    if program_id != &ID {
        return Err(ProgramError::IncorrectProgramId);
    }
    match data.split_first() {
        Some((0, rest)) if rest.len() >= 32 + 2 + 32 + 2 => {
            let namespace: [u8; 32] = rest[..32].try_into().unwrap();
            let schema = u16::from_le_bytes(rest[32..34].try_into().unwrap());
            let subject = Pubkey::new_from_array(rest[34..66].try_into().unwrap());
            let len = u16::from_le_bytes(rest[66..68].try_into().unwrap()) as usize;
            let body = &rest[68..];
            if body.len() != len || len > MAX_BODY {
                return Err(ProgramError::InvalidInstructionData);
            }
            publish(accounts, &namespace, schema, &subject, body)
        }
        Some((1, rest)) if rest.len() == 64 => {
            let namespace: [u8; 32] = rest[..32].try_into().unwrap();
            let subject = Pubkey::new_from_array(rest[32..64].try_into().unwrap());
            revoke(accounts, &namespace, &subject)
        }
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

fn publish(accounts: &[AccountInfo], namespace: &[u8; 32], schema: u16, subject: &Pubkey, body: &[u8]) -> ProgramResult {
    let [issuer, record, system, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !issuer.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let (expected, bump) = record_pda(issuer.key, namespace, subject);
    if *record.key != expected {
        return Err(ProgramError::InvalidSeeds);
    }
    let len = HEADER_LEN + body.len();
    let mut sequence = 1u64;
    if record.owner == &ID {
        let d = record.try_borrow_data()?;
        if d.len() != len {
            return Err(ProgramError::InvalidAccountData);
        }
        let h: StateHeader = bytemuck::pod_read_unaligned(&d[..HEADER_LEN]);
        sequence = h.sequence.checked_add(1).ok_or(ProgramError::ArithmeticOverflow)?;
    } else {
        create(issuer, record, system, len, &[RECORD_SEED, issuer.key.as_ref(), namespace, subject.as_ref(), &[bump]])?;
    }
    let clock = Clock::get()?;
    let mut h = StateHeader::zeroed();
    h.magic = MAGIC;
    h.version = hook_standard::ABI_VERSION;
    h.schema = schema;
    h.namespace = *namespace;
    h.producer = ID.to_bytes();
    h.subject = subject.to_bytes();
    h.issuer = issuer.key.to_bytes();
    h.slot = clock.slot;
    h.timestamp = clock.unix_timestamp;
    h.sequence = sequence;
    let mut d = record.try_borrow_mut_data()?;
    d[..HEADER_LEN].copy_from_slice(bytemuck::bytes_of(&h));
    d[HEADER_LEN..].copy_from_slice(body);
    Ok(())
}

fn revoke(accounts: &[AccountInfo], namespace: &[u8; 32], subject: &Pubkey) -> ProgramResult {
    let [issuer, record, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !issuer.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *record.key != record_pda(issuer.key, namespace, subject).0 || record.owner != &ID {
        return Err(ProgramError::InvalidSeeds);
    }
    record.try_borrow_mut_data()?[..4].fill(0);
    Ok(())
}

/// Creates the PDA, also when its address was pre-funded (which makes `create_account` fail).
fn create<'info>(payer: &AccountInfo<'info>, target: &AccountInfo<'info>, system: &AccountInfo<'info>, len: usize, seeds: &[&[u8]]) -> ProgramResult {
    let rent = Rent::get()?.minimum_balance(len);
    let infos = [payer.clone(), target.clone(), system.clone()];
    if target.lamports() == 0 {
        return invoke_signed(&system_instruction::create_account(payer.key, target.key, rent, len as u64, &ID), &infos, &[seeds]);
    }
    if rent > target.lamports() {
        invoke(&system_instruction::transfer(payer.key, target.key, rent - target.lamports()), &infos)?;
    }
    invoke_signed(&system_instruction::allocate(target.key, len as u64), &infos, &[seeds])?;
    invoke_signed(&system_instruction::assign(target.key, &ID), &infos, &[seeds])
}
