use anchor_lang::prelude::*;
use hook_standard::loader;
use hook_types::ModuleId;

use crate::{errors::PumpHooksError as E, events::KillSwitchSet, state::*};

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(init, payer = authority, space = 8 + GlobalConfig::INIT_SPACE, seeds = [GLOBAL_SEED], bump)]
    pub global: Account<'info, GlobalConfig>,
    pub system_program: Program<'info, System>,
    /// CHECK: this program; its ProgramData proves who may initialize.
    #[account(address = crate::ID)]
    pub program: UncheckedAccount<'info>,
    /// CHECK: validated by `loader::read_program`.
    pub program_data: UncheckedAccount<'info>,
}

/// Only the upgrade authority may initialize, so deployment can't be front-run.
pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
    let info = loader::read_program(&ctx.accounts.program, Some(&ctx.accounts.program_data))?;
    require!(info.upgrade_authority == Some(ctx.accounts.authority.key()), E::Unauthorized);
    let g = &mut ctx.accounts.global;
    g.authority = ctx.accounts.authority.key();
    g.pending_authority = Pubkey::default();
    g.disabled_modules = 0;
    g.market_count = 0;
    g.paused = false;
    g.bump = ctx.bumps.global;
    Ok(())
}

#[derive(Accounts)]
pub struct GlobalAdmin<'info> {
    pub authority: Signer<'info>,
    #[account(mut, seeds = [GLOBAL_SEED], bump = global.bump, has_one = authority @ E::Unauthorized)]
    pub global: Account<'info, GlobalConfig>,
}

pub fn set_kill_switch(ctx: Context<GlobalAdmin>, disabled_modules: u64, paused: bool) -> Result<()> {
    let g = &mut ctx.accounts.global;
    g.disabled_modules = disabled_modules;
    g.paused = paused;
    emit!(KillSwitchSet { disabled_modules, paused });
    Ok(())
}

pub fn propose_global_authority(ctx: Context<GlobalAdmin>, new_authority: Pubkey) -> Result<()> {
    ctx.accounts.global.pending_authority = new_authority;
    Ok(())
}

#[derive(Accounts)]
pub struct AcceptGlobalAuthority<'info> {
    pub new_authority: Signer<'info>,
    #[account(mut, seeds = [GLOBAL_SEED], bump = global.bump)]
    pub global: Account<'info, GlobalConfig>,
}

pub fn accept_global_authority(ctx: Context<AcceptGlobalAuthority>) -> Result<()> {
    let g = &mut ctx.accounts.global;
    require_keys_eq!(g.pending_authority, ctx.accounts.new_authority.key(), E::NoPendingAuthority);
    g.authority = g.pending_authority;
    g.pending_authority = Pubkey::default();
    Ok(())
}

#[derive(Accounts)]
#[instruction(module_id: u16)]
pub struct RegisterHook<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(seeds = [GLOBAL_SEED], bump = global.bump, has_one = authority @ E::Unauthorized)]
    pub global: Account<'info, GlobalConfig>,
    #[account(init, payer = authority, space = 8 + HookDefinition::INIT_SPACE, seeds = [HOOK_SEED, &module_id.to_le_bytes()], bump)]
    pub definition: Account<'info, HookDefinition>,
    pub system_program: Program<'info, System>,
}

/// Register a first-party module from the static module table.
pub fn register_hook(ctx: Context<RegisterHook>, module_id: u16, source_hash: [u8; 32]) -> Result<()> {
    let m = ModuleId::from_u16(module_id).ok_or(E::UnknownModule)?;
    let info = m.info();
    require!(info.available, E::ModuleUnavailable);
    let d = &mut ctx.accounts.definition;
    d.module_id = module_id;
    d.version = info.version;
    d.capabilities = info.capabilities;
    d.callbacks = info.callbacks;
    d.priority_min = info.priority_min;
    d.priority_max = info.priority_max;
    d.runtime = info.runtime;
    d.enabled = true;
    d.installs = 0;
    d.source_hash = source_hash;
    d.audit_status = 0;
    d.bump = ctx.bumps.definition;
    Ok(())
}

#[derive(Accounts)]
pub struct SetHookDefinition<'info> {
    pub authority: Signer<'info>,
    #[account(seeds = [GLOBAL_SEED], bump = global.bump, has_one = authority @ E::Unauthorized)]
    pub global: Account<'info, GlobalConfig>,
    #[account(mut, seeds = [HOOK_SEED, &definition.module_id.to_le_bytes()], bump = definition.bump)]
    pub definition: Account<'info, HookDefinition>,
}

/// Disabling a definition blocks new installs; use the kill switch to stop existing installs.
pub fn set_hook_definition(ctx: Context<SetHookDefinition>, enabled: bool, source_hash: [u8; 32]) -> Result<()> {
    let d = &mut ctx.accounts.definition;
    d.enabled = enabled;
    d.source_hash = source_hash;
    Ok(())
}
