use anchor_lang::prelude::*;
use hook_runtime::{install_slot, remove_slot, validate_module_config};
use hook_types::{slot_flags, ModuleId};

use super::market::check_market_authority;
use crate::{
    errors::{hook_err, PumpHooksError as E},
    events::{HookConfigured, HookInstalled, HookRemoved},
    state::*,
};

#[derive(Accounts)]
#[instruction(module_id: u16)]
pub struct InstallHook<'info> {
    pub authority: Signer<'info>,
    #[account(seeds = [GLOBAL_SEED], bump = global.bump)]
    pub global: Account<'info, GlobalConfig>,
    #[account(mut, seeds = [HOOK_SEED, &module_id.to_le_bytes()], bump = definition.bump)]
    pub definition: Account<'info, HookDefinition>,
    #[account(mut)]
    pub market: AccountLoader<'info, MarketConfig>,
}

pub fn install_hook(
    ctx: Context<InstallHook>,
    module_id: u16,
    priority: u16,
    callbacks: u16,
    capabilities: u32,
    config: [u8; 48],
) -> Result<()> {
    let def = &ctx.accounts.definition;
    require!(def.enabled, E::HookDefinitionDisabled);
    let module = ModuleId::from_u16(module_id).ok_or(E::UnknownModule)?;
    require!(ctx.accounts.global.disabled_modules & module.kill_bit() == 0, E::ModuleDisabled);
    if module == ModuleId::Policy {
        require!(config == [0u8; 48], E::PolicyConfigManaged);
    }

    let mut m = ctx.accounts.market.load_mut()?;
    check_market_authority(&m, &ctx.accounts.authority.key())?;
    let mut slots = m.read_slots();
    let idx = install_slot(&mut slots, module_id, priority, callbacks, capabilities, config).map_err(hook_err)?;
    let s = slots[idx];
    m.write_slots(&slots);
    drop(m);

    let def = &mut ctx.accounts.definition;
    def.installs = def.installs.saturating_add(1);
    emit!(HookInstalled {
        market: ctx.accounts.market.key(),
        module_id,
        version: s.version,
        slot_index: idx as u8,
        priority: s.priority,
        callbacks: s.callbacks,
        capabilities: s.capabilities,
        config,
    });
    Ok(())
}

#[derive(Accounts)]
#[instruction(module_id: u16)]
pub struct RemoveHook<'info> {
    pub authority: Signer<'info>,
    #[account(mut, seeds = [HOOK_SEED, &module_id.to_le_bytes()], bump = definition.bump)]
    pub definition: Account<'info, HookDefinition>,
    #[account(mut)]
    pub market: AccountLoader<'info, MarketConfig>,
}

pub fn remove_hook(ctx: Context<RemoveHook>, module_id: u16) -> Result<()> {
    let mut m = ctx.accounts.market.load_mut()?;
    check_market_authority(&m, &ctx.accounts.authority.key())?;
    let mut slots = m.read_slots();
    remove_slot(&mut slots, module_id).map_err(hook_err)?;
    m.write_slots(&slots);
    drop(m);
    let def = &mut ctx.accounts.definition;
    def.installs = def.installs.saturating_sub(1);
    emit!(HookRemoved { market: ctx.accounts.market.key(), module_id });
    Ok(())
}

#[derive(Accounts)]
pub struct ConfigureHook<'info> {
    pub authority: Signer<'info>,
    #[account(mut)]
    pub market: AccountLoader<'info, MarketConfig>,
}

/// Replace a module's config (state is kept) and/or toggle it.
pub fn configure_hook(ctx: Context<ConfigureHook>, module_id: u16, enabled: bool, config: Option<[u8; 48]>) -> Result<()> {
    let module = ModuleId::from_u16(module_id).ok_or(E::UnknownModule)?;
    let mut m = ctx.accounts.market.load_mut()?;
    check_market_authority(&m, &ctx.accounts.authority.key())?;
    let mut slots = m.read_slots();
    let s = slots.iter_mut().find(|s| s.module_id == module_id && !s.is_empty()).ok_or(E::SlotNotFound)?;
    if let Some(c) = config {
        require!(module != ModuleId::Policy, E::PolicyConfigManaged);
        validate_module_config(module, &c).map_err(hook_err)?;
        s.config = c;
    }
    if enabled {
        s.flags |= slot_flags::ENABLED;
    } else {
        s.flags &= !slot_flags::ENABLED;
    }
    let (cfg, en) = (s.config, s.is_enabled());
    m.write_slots(&slots);
    emit!(HookConfigured { market: ctx.accounts.market.key(), module_id, enabled: en, config: cfg });
    Ok(())
}
