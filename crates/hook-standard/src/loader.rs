//! What the BPF loaders say about a program: upgrade authority and last-deploy slot.
#![allow(deprecated)]

use solana_program::{
    account_info::AccountInfo, bpf_loader, bpf_loader_deprecated, bpf_loader_upgradeable,
    program_error::ProgramError, pubkey::Pubkey,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgramInfo {
    /// `None` = immutable.
    pub upgrade_authority: Option<Pubkey>,
    /// 0 for the non-upgradeable loaders.
    pub deploy_slot: u64,
}

const PROGRAMDATA_METADATA_LEN: usize = 45;

/// The ProgramData address an upgradeable program account points to.
pub fn program_data_address(program: &AccountInfo) -> Option<Pubkey> {
    if *program.owner != bpf_loader_upgradeable::ID {
        return None;
    }
    let d = program.try_borrow_data().ok()?;
    (d.len() >= 36 && d[..4] == 2u32.to_le_bytes()).then(|| Pubkey::try_from(&d[4..36]).unwrap())
}

/// Upgradeable programs need their ProgramData account.
pub fn read_program(
    program: &AccountInfo,
    program_data: Option<&AccountInfo>,
) -> Result<ProgramInfo, ProgramError> {
    if !program.executable {
        return Err(ProgramError::InvalidAccountData);
    }
    if *program.owner == bpf_loader::ID || *program.owner == bpf_loader_deprecated::ID {
        return Ok(ProgramInfo {
            upgrade_authority: None,
            deploy_slot: 0,
        });
    }
    let expected = program_data_address(program).ok_or(ProgramError::IncorrectProgramId)?;
    let pd = program_data.ok_or(ProgramError::NotEnoughAccountKeys)?;
    if *pd.key != expected || *pd.owner != bpf_loader_upgradeable::ID {
        return Err(ProgramError::InvalidAccountData);
    }
    let d = pd.try_borrow_data()?;
    if d.len() < PROGRAMDATA_METADATA_LEN || d[..4] != 3u32.to_le_bytes() {
        return Err(ProgramError::InvalidAccountData);
    }
    let deploy_slot = u64::from_le_bytes(d[4..12].try_into().unwrap());
    let upgrade_authority = match d[12] {
        0 => None,
        1 => Some(Pubkey::try_from(&d[13..45]).unwrap()),
        _ => return Err(ProgramError::InvalidAccountData),
    };
    Ok(ProgramInfo {
        upgrade_authority,
        deploy_slot,
    })
}
