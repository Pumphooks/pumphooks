//! Token movement, vault creation and the mint policy (`docs/engines/pool-manager.md` §7).

use anchor_lang::prelude::*;
use anchor_lang::solana_program::{program::invoke_signed, system_instruction};
use anchor_spl::token_2022::spl_token_2022::{
    self,
    extension::{
        transfer_fee::TransferFeeConfig, BaseStateWithExtensions, ExtensionType, StateWithExtensions,
    },
    state::{Account as TokenAccount2022, Mint as Mint2022},
};
use anchor_spl::token_interface::{self, TransferChecked};

use crate::errors::CoreError;

pub fn is_token_program(key: &Pubkey) -> bool {
    *key == anchor_spl::token::ID || *key == spl_token_2022::ID
}

/// Raw token amount (offset 64 in both SPL Token and Token-2022 accounts).
pub fn amount(account: &AccountInfo) -> Result<u64> {
    require!(is_token_program(account.owner), CoreError::PoolAccountMismatch);
    let d = account.try_borrow_data()?;
    require!(d.len() >= 72, CoreError::PoolAccountMismatch);
    Ok(u64::from_le_bytes(d[64..72].try_into().unwrap()))
}

pub fn decimals(mint: &AccountInfo) -> Result<u8> {
    require!(is_token_program(mint.owner), CoreError::PoolAccountMismatch);
    let d = mint.try_borrow_data()?;
    require!(d.len() >= 82, CoreError::PoolAccountMismatch);
    Ok(d[44])
}

/// Accepts SPL Token mints and Token-2022 mints whose every extension is on the allowlist.
/// Unknown extensions fail to parse and are rejected.
pub fn check_mint(mint: &AccountInfo) -> Result<()> {
    if *mint.owner == anchor_spl::token::ID {
        return Ok(());
    }
    require_keys_eq!(*mint.owner, spl_token_2022::ID, CoreError::PoolAccountMismatch);
    let d = mint.try_borrow_data()?;
    let state = StateWithExtensions::<Mint2022>::unpack(&d)
        .map_err(|_| error!(CoreError::UnsupportedMintExtension))?;
    let types = state.get_extension_types().map_err(|_| error!(CoreError::UnsupportedMintExtension))?;
    for t in types {
        let ok = matches!(
            t,
            ExtensionType::TransferFeeConfig
                | ExtensionType::MintCloseAuthority
                | ExtensionType::InterestBearingConfig
                | ExtensionType::MetadataPointer
                | ExtensionType::TokenMetadata
                | ExtensionType::GroupPointer
                | ExtensionType::TokenGroup
                | ExtensionType::GroupMemberPointer
                | ExtensionType::TokenGroupMember
        );
        if !ok {
            msg!("unsupported mint extension {:?}", t);
            return err!(CoreError::UnsupportedMintExtension);
        }
    }
    Ok(())
}

/// Whether transfers of `mint` currently charge a fee.
pub fn transfer_fee_active(mint: &AccountInfo) -> Result<bool> {
    if *mint.owner != spl_token_2022::ID {
        return Ok(false);
    }
    let d = mint.try_borrow_data()?;
    let state = StateWithExtensions::<Mint2022>::unpack(&d)
        .map_err(|_| error!(CoreError::UnsupportedMintExtension))?;
    Ok(match state.get_extension::<TransferFeeConfig>() {
        Ok(cfg) => {
            let fee = cfg.get_epoch_fee(Clock::get()?.epoch);
            u16::from(fee.transfer_fee_basis_points) > 0
        }
        Err(_) => false,
    })
}

/// Creates the PDA token account `["vault", pool, mint]` owned by `pool`, sized for the mint's
/// required account extensions.
#[allow(clippy::too_many_arguments)]
pub fn create_vault<'info>(
    payer: &AccountInfo<'info>,
    vault: &AccountInfo<'info>,
    mint: &AccountInfo<'info>,
    pool: &Pubkey,
    token_program: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    vault_seeds: &[&[u8]],
) -> Result<()> {
    let space = if *token_program.key == spl_token_2022::ID {
        let d = mint.try_borrow_data()?;
        let state = StateWithExtensions::<Mint2022>::unpack(&d)?;
        let required = ExtensionType::get_required_init_account_extensions(&state.get_extension_types()?);
        ExtensionType::try_calculate_account_len::<TokenAccount2022>(&required)?
    } else {
        anchor_spl::token::TokenAccount::LEN
    };
    let lamports = Rent::get()?.minimum_balance(space);
    invoke_signed(
        &system_instruction::create_account(payer.key, vault.key, lamports, space as u64, token_program.key),
        &[payer.clone(), vault.clone(), system_program.clone()],
        &[vault_seeds],
    )?;
    let ix = spl_token_2022::instruction::initialize_account3(token_program.key, vault.key, mint.key, pool)?;
    anchor_lang::solana_program::program::invoke(&ix, &[vault.clone(), mint.clone()])?;
    Ok(())
}

/// `transfer_checked`, signed by `seeds` when the authority is a PDA.
pub fn transfer<'info>(
    token_program: &AccountInfo<'info>,
    from: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    authority: &AccountInfo<'info>,
    mint: &AccountInfo<'info>,
    amount: u64,
    seeds: Option<&[&[u8]]>,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    let accts = TransferChecked {
        from: from.clone(),
        mint: mint.clone(),
        to: to.clone(),
        authority: authority.clone(),
    };
    let decimals = decimals(mint)?;
    match seeds {
        Some(s) => token_interface::transfer_checked(
            CpiContext::new_with_signer(token_program.clone(), accts, &[s]),
            amount,
            decimals,
        ),
        None => token_interface::transfer_checked(CpiContext::new(token_program.clone(), accts), amount, decimals),
    }
}

/// Transfers in and returns what the vault actually received (net of any transfer fee).
pub fn transfer_in<'info>(
    token_program: &AccountInfo<'info>,
    from: &AccountInfo<'info>,
    vault: &AccountInfo<'info>,
    authority: &AccountInfo<'info>,
    mint: &AccountInfo<'info>,
    amount: u64,
) -> Result<u64> {
    let before = self::amount(vault)?;
    transfer(token_program, from, vault, authority, mint, amount, None)?;
    Ok(self::amount(vault)? - before)
}
