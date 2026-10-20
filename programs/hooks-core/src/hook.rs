//! Calling hook programs: building the context, the CPI, and authenticating the result.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    bpf_loader, bpf_loader_deprecated, bpf_loader_upgradeable,
    instruction::{AccountMeta, Instruction},
    program::{get_return_data, invoke_signed},
};
use hooks_interface::{
    encode_call, Callback, HookContext, HookResult, CALLBACK_SEED, CONTEXT_LEN, HOOK_AUTHORITY_SEED,
    INTERFACE_VERSION, MAX_HOOK_DATA,
};

use crate::{
    engine::{Book, Hooks},
    errors::CoreError,
    events::HookCalled,
    state::{Pool, POOL_HOOK_PINNED},
};

/// What the loader says about a program.
pub struct ProgramInfo {
    pub upgrade_authority: Option<Pubkey>,
    pub deploy_slot: u64,
}

const PROGRAMDATA_METADATA_LEN: usize = 45;

/// Reads upgrade authority and last-deploy slot. Upgradeable programs need their ProgramData
/// account; programs on the immutable loaders report no authority and slot 0.
pub fn read_program(program: &AccountInfo, program_data: Option<&AccountInfo>) -> Result<ProgramInfo> {
    require!(program.executable, CoreError::UnsupportedLoader);
    if *program.owner == bpf_loader::ID || *program.owner == bpf_loader_deprecated::ID {
        return Ok(ProgramInfo { upgrade_authority: None, deploy_slot: 0 });
    }
    require_keys_eq!(*program.owner, bpf_loader_upgradeable::ID, CoreError::UnsupportedLoader);
    let pd = program_data.ok_or(error!(CoreError::InvalidProgramData))?;
    let expected = {
        let d = program.try_borrow_data()?;
        require!(d.len() >= 36 && d[..4] == 2u32.to_le_bytes(), CoreError::InvalidProgramData);
        Pubkey::try_from(&d[4..36]).unwrap()
    };
    require_keys_eq!(*pd.key, expected, CoreError::InvalidProgramData);
    require_keys_eq!(*pd.owner, bpf_loader_upgradeable::ID, CoreError::InvalidProgramData);
    let d = pd.try_borrow_data()?;
    require!(
        d.len() >= PROGRAMDATA_METADATA_LEN && d[..4] == 3u32.to_le_bytes(),
        CoreError::InvalidProgramData
    );
    let deploy_slot = u64::from_le_bytes(d[4..12].try_into().unwrap());
    let upgrade_authority = match d[12] {
        0 => None,
        1 => Some(Pubkey::try_from(&d[13..45]).unwrap()),
        _ => return err!(CoreError::InvalidProgramData),
    };
    Ok(ProgramInfo { upgrade_authority, deploy_slot })
}

/// The hook-side accounts every pool operation may carry.
pub struct HookAccounts<'a, 'info> {
    pub hook_program: Option<&'a AccountInfo<'info>>,
    pub callback_authority: Option<&'a AccountInfo<'info>>,
    pub hook_program_data: Option<&'a AccountInfo<'info>>,
}

/// Hooks implementation that CPIs the pool's hook program.
pub struct CpiHooks<'a, 'info> {
    hook_program: &'a AccountInfo<'info>,
    callback_authority: &'a AccountInfo<'info>,
    callback_bump: u8,
    pool_info: &'a AccountInfo<'info>,
    extra: &'a [AccountInfo<'info>],
    hook_data: &'a [u8],
    base: HookContext,
}

/// Validates the hook accounts for `pool` and returns a caller, or `None` for plain pools.
pub fn cpi_hooks<'a, 'info>(
    pool: &Pool,
    pool_info: &'a AccountInfo<'info>,
    accts: HookAccounts<'a, 'info>,
    extra: &'a [AccountInfo<'info>],
    hook_data: &'a [u8],
    caller: Pubkey,
) -> Result<Option<CpiHooks<'a, 'info>>> {
    require!(hook_data.len() <= MAX_HOOK_DATA, CoreError::HookDataTooLong);
    if !pool.has_hook() {
        return Ok(None);
    }
    let hook_program = accts.hook_program.ok_or(error!(CoreError::HookAccountMismatch))?;
    require_keys_eq!(*hook_program.key, pool.hook_program, CoreError::HookAccountMismatch);
    let callback_authority = accts.callback_authority.ok_or(error!(CoreError::HookAccountMismatch))?;
    let expected = Pubkey::create_program_address(
        &[CALLBACK_SEED, pool.hook_program.as_ref(), &[pool.callback_bump]],
        &crate::ID,
    )
    .map_err(|_| error!(CoreError::HookAccountMismatch))?;
    require_keys_eq!(*callback_authority.key, expected, CoreError::HookAccountMismatch);
    if pool.flags & POOL_HOOK_PINNED != 0 {
        let info = read_program(hook_program, accts.hook_program_data)?;
        require!(info.deploy_slot == pool.pinned_deploy_slot, CoreError::HookRedeployed);
    }
    let clock = Clock::get()?;
    let base = HookContext {
        version: INTERFACE_VERSION,
        callback: 0,
        swap_flags: 0,
        _reserved: 0,
        permissions: pool.permissions,
        pool: pool_info.key.to_bytes(),
        caller: caller.to_bytes(),
        mint_0: pool.mint_0.to_bytes(),
        mint_1: pool.mint_1.to_bytes(),
        reserve_0: 0,
        reserve_1: 0,
        lp_supply: 0,
        sequence: pool.sequence,
        slot: clock.slot,
        unix_timestamp: clock.unix_timestamp,
        fee_ppm: 0,
        hook_data_len: hook_data.len() as u32,
        args: [0; 7],
    };
    Ok(Some(CpiHooks {
        hook_program,
        callback_authority,
        callback_bump: pool.callback_bump,
        pool_info,
        extra,
        hook_data,
        base,
    }))
}

impl Hooks for CpiHooks<'_, '_> {
    fn call(&mut self, cb: Callback, book: &Book, swap_flags: u8, args: [u64; 7]) -> Result<HookResult> {
        let mut ctx = self.base;
        ctx.callback = cb as u8;
        ctx.swap_flags = swap_flags;
        [ctx.reserve_0, ctx.reserve_1] = book.reserve;
        ctx.lp_supply = book.lp_supply;
        ctx.fee_ppm = book.fee_ppm;
        ctx.args = args;

        let mut data = vec![0u8; 8 + CONTEXT_LEN + self.hook_data.len()];
        encode_call(&ctx, self.hook_data, &mut data).ok_or(error!(CoreError::HookDataTooLong))?;

        let mut metas = Vec::with_capacity(2 + self.extra.len());
        metas.push(AccountMeta::new_readonly(*self.callback_authority.key, true));
        metas.push(AccountMeta::new_readonly(*self.pool_info.key, false));
        let mut infos = Vec::with_capacity(3 + self.extra.len());
        infos.push(self.callback_authority.clone());
        infos.push(self.pool_info.clone());
        for a in self.extra {
            // Never forward a signature: a hook must not act with the trader's authority.
            metas.push(AccountMeta { pubkey: *a.key, is_signer: false, is_writable: a.is_writable });
            infos.push(a.clone());
        }
        infos.push(self.hook_program.clone());

        let ix = Instruction { program_id: *self.hook_program.key, accounts: metas, data };
        let hook = self.hook_program.key;
        invoke_signed(&ix, &infos, &[&[CALLBACK_SEED, hook.as_ref(), &[self.callback_bump]]])?;

        let (from, bytes) = get_return_data().ok_or(error!(CoreError::HookResultInvalid))?;
        require_keys_eq!(from, *hook, CoreError::HookResultInvalid);
        let r = HookResult::decode(&bytes).ok_or(error!(CoreError::HookResultInvalid))?;
        emit!(HookCalled {
            pool: *self.pool_info.key,
            callback: cb as u8,
            decision: r.decision,
            reason: r.reason,
            fee_override: r.fee_override(),
            delta_0: r.delta_0,
            delta_1: r.delta_1,
            quote: r.quote,
        });
        Ok(r)
    }
}

impl Hooks for Option<CpiHooks<'_, '_>> {
    fn call(&mut self, cb: Callback, book: &Book, swap_flags: u8, args: [u64; 7]) -> Result<HookResult> {
        match self {
            Some(h) => h.call(cb, book, swap_flags, args),
            // Plain pools have no permissions, so the engine never asks.
            None => err!(CoreError::HookAccountMismatch),
        }
    }
}

/// Requires `signer` to be the pool hook's hook-authority PDA (`["hook-authority"]` under the hook).
pub fn require_hook_authority(pool: &Pool, signer: &Pubkey) -> Result<()> {
    require!(pool.has_hook(), CoreError::Unauthorized);
    let (expected, _) = Pubkey::find_program_address(&[HOOK_AUTHORITY_SEED], &pool.hook_program);
    require_keys_eq!(*signer, expected, CoreError::Unauthorized);
    Ok(())
}
