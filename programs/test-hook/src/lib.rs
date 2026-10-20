//! TEST ONLY: a scriptable hook for the core's integration tests. Never deploy.
//!
//! Callback `hook_data` is a list of 41-byte entries `callback_id ‖ HookResult`; the hook returns
//! the entry for the current callback, or `HookResult::ok()`. A leading attack byte selects
//! hostile behavior instead:
//!
//! - `0xA1` steal: transfer from forwarded account 2 (trader's token account) to account 3.
//! - `0xA2` re-enter: CPI the core with the remaining accounts.
//! - `0xA3` silent: return no result.
//!
//! Instruction `0xFF ‖ core_ix_data` (non-callback) proxies a core instruction signed by this
//! hook's hook-authority PDA. Accounts: `[core_program, ...core instruction accounts]`.

#![allow(unexpected_cfgs)]

use hooks_interface::{decode_call, hook_authority, verify_core_call, HookResult, CORE_PROGRAM_ID, HOOK_AUTHORITY_SEED, RESULT_LEN};
use solana_program::{
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    pubkey::Pubkey,
};

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(process);

pub const STEAL: u8 = 0xA1;
pub const REENTER: u8 = 0xA2;
pub const SILENT: u8 = 0xA3;
pub const PROXY: u8 = 0xFF;
const ENTRY: usize = 1 + RESULT_LEN;

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    if let Some(call) = decode_call(data) {
        verify_core_call(program_id, accounts, &call.ctx)?;
        let script = call.hook_data;
        match script.first() {
            Some(&STEAL) => {
                let amount = 1;
                let ix = Instruction {
                    program_id: *accounts[4].key,
                    accounts: vec![
                        AccountMeta::new(*accounts[2].key, false),
                        AccountMeta::new(*accounts[3].key, false),
                        AccountMeta::new_readonly(*accounts[5].key, true),
                    ],
                    // SPL Token `Transfer` (3) of `amount`.
                    data: [&[3u8][..], &(amount as u64).to_le_bytes()].concat(),
                };
                invoke(&ix, accounts)?;
            }
            Some(&REENTER) => {
                let ix = Instruction {
                    program_id: CORE_PROGRAM_ID,
                    accounts: accounts[2..].iter().map(|a| AccountMeta::new_readonly(*a.key, false)).collect(),
                    data: vec![0; 8],
                };
                invoke(&ix, accounts)?;
            }
            Some(&SILENT) => return Ok(()),
            _ => {}
        }
        let r = script
            .chunks_exact(ENTRY)
            .find(|e| e[0] == call.callback as u8)
            .map(|e| bytemuck::pod_read_unaligned::<HookResult>(&e[1..]))
            .unwrap_or(HookResult::ok());
        solana_program::program::set_return_data(bytemuck::bytes_of(&r));
        return Ok(());
    }
    match data.split_first() {
        Some((&PROXY, core_data)) => {
            let (authority, bump) = hook_authority(program_id);
            let [_core, rest @ ..] = accounts else {
                return Err(ProgramError::NotEnoughAccountKeys);
            };
            let metas = rest
                .iter()
                .map(|a| AccountMeta {
                    pubkey: *a.key,
                    is_signer: *a.key == authority || a.is_signer,
                    is_writable: a.is_writable,
                })
                .collect();
            let ix = Instruction { program_id: CORE_PROGRAM_ID, accounts: metas, data: core_data.to_vec() };
            invoke_signed(&ix, accounts, &[&[HOOK_AUTHORITY_SEED, &[bump]]])
        }
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
