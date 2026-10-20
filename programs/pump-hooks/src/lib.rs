//! Pump Hooks: a programmable execution layer for existing Pump markets.
//!
//! One program holds the hook registry, market configs, the router and the policy engine.
//! Hooks run at the router boundary: they govern trades routed through `route_amm` /
//! `route_curve`. Pump and PumpSwap do not call back into this program, and trades sent to Pump
//! directly are not affected by any hook installed here.

#![allow(unexpected_cfgs)]

use anchor_lang::prelude::*;

pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;

use instructions::*;

declare_id!("91kRNewiEDrKVtUXpahsDXYiURuKQKwhCt2hnaKNfkgD");

#[program]
pub mod pump_hooks {
    use super::*;
    use hook_types::Venue;

    // ---- protocol admin
    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        admin::initialize(ctx)
    }
    pub fn set_kill_switch(ctx: Context<GlobalAdmin>, disabled_modules: u64, paused: bool) -> Result<()> {
        admin::set_kill_switch(ctx, disabled_modules, paused)
    }
    pub fn propose_global_authority(ctx: Context<GlobalAdmin>, new_authority: Pubkey) -> Result<()> {
        admin::propose_global_authority(ctx, new_authority)
    }
    pub fn accept_global_authority(ctx: Context<AcceptGlobalAuthority>) -> Result<()> {
        admin::accept_global_authority(ctx)
    }
    pub fn register_hook(ctx: Context<RegisterHook>, module_id: u16, source_hash: [u8; 32]) -> Result<()> {
        admin::register_hook(ctx, module_id, source_hash)
    }
    pub fn set_hook_definition(ctx: Context<SetHookDefinition>, enabled: bool, source_hash: [u8; 32]) -> Result<()> {
        admin::set_hook_definition(ctx, enabled, source_hash)
    }

    // ---- markets (existing Pump tokens)
    pub fn create_market(ctx: Context<CreateMarket>) -> Result<()> {
        market::create_market(ctx)
    }
    pub fn sync_market(ctx: Context<SyncMarket>) -> Result<()> {
        market::sync_market(ctx)
    }
    pub fn set_market_paused(ctx: Context<MarketAdmin>, paused: bool) -> Result<()> {
        market::set_market_paused(ctx, paused)
    }
    pub fn propose_market_authority(ctx: Context<MarketAdmin>, new_authority: Pubkey) -> Result<()> {
        market::propose_market_authority(ctx, new_authority)
    }
    pub fn accept_market_authority(ctx: Context<MarketAdmin>) -> Result<()> {
        market::accept_market_authority(ctx)
    }

    // ---- hook stack
    pub fn install_hook(
        ctx: Context<InstallHook>,
        module_id: u16,
        priority: u16,
        callbacks: u16,
        capabilities: u32,
        config: [u8; 48],
    ) -> Result<()> {
        hooks::install_hook(ctx, module_id, priority, callbacks, capabilities, config)
    }
    pub fn remove_hook(ctx: Context<RemoveHook>, module_id: u16) -> Result<()> {
        hooks::remove_hook(ctx, module_id)
    }
    pub fn configure_hook(ctx: Context<ConfigureHook>, module_id: u16, enabled: bool, config: Option<[u8; 48]>) -> Result<()> {
        hooks::configure_hook(ctx, module_id, enabled, config)
    }

    // ---- composer policies
    pub fn create_policy(ctx: Context<CreatePolicy>, index: u8, args: PolicyArgs) -> Result<()> {
        policy::create_policy(ctx, index, args)
    }
    pub fn update_policy(ctx: Context<UpdatePolicy>, index: u8, enabled: bool, args: Option<PolicyArgs>) -> Result<()> {
        policy::update_policy(ctx, index, enabled, args)
    }
    pub fn close_policy(ctx: Context<ClosePolicy>, index: u8) -> Result<()> {
        policy::close_policy(ctx, index)
    }

    // ---- routed execution
    /// PumpSwap `buy_v2` (mode 0), `buy_exact_quote_in_v2` (1) or `sell_v2` (2).
    pub fn route_amm<'info>(ctx: Context<'_, '_, 'info, 'info, Route<'info>>, mode: u8, amount: u64, limit: u64) -> Result<()> {
        route::route(ctx, Venue::PumpSwap, mode, amount, limit)
    }
    /// Pump bonding curve `buy_v3` (0), `buy_exact_quote_in_v3` (1) or `sell_v3` (2).
    pub fn route_curve<'info>(ctx: Context<'_, '_, 'info, 'info, Route<'info>>, mode: u8, amount: u64, limit: u64) -> Result<()> {
        route::route(ctx, Venue::BondingCurve, mode, amount, limit)
    }

    // ---- SENTIENT
    pub fn submit_signal(ctx: Context<SubmitSignal>, signal: [u8; 96]) -> Result<()> {
        signal::submit_signal(ctx, signal)
    }
}
