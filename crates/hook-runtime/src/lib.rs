//! Pump Hooks runtime.
//!
//! Executes a market's hook stack for one phase of a routed trade. The same code runs inside the
//! `pump-hooks` program and in the off-chain simulator, so a backtest and an on-chain execution
//! make the same decisions.

#![no_std]

pub mod executor;
pub mod modules;

pub use executor::{
    commit_trade, install_slot, remove_slot, run_phase, validate_module_config, volatility_bps, HookError, PhaseOutcome, PolicyEvent, Trace,
    TraceEntry,
};
pub use modules::PumpHook;
