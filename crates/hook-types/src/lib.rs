//! Pump Hooks ABI.
//!
//! Everything in this crate is part of the stable wire format between the router, first-party
//! modules, future external hooks, the SDK and the indexer. Discriminants and layouts are
//! append-only. See `docs/hooks-spec-pump-router.md`.

#![no_std]

pub mod callback;
pub mod capability;
pub mod context;
pub mod decision;
pub mod module;
pub mod reason;
pub mod signal;
pub mod slot;
pub mod user;

pub use callback::{HookCallback, Side, TradeMode, Venue};
pub use capability::Capabilities;
pub use context::{context_flags, HookContext, MarketStats};
pub use decision::{decision_flags, HookDecision};
pub use module::{ModuleId, ModuleInfo, MODULES};
pub use signal::{Regime, SignalAction, StrategySignal, SIGNAL_DOMAIN};
pub use slot::{slot_flags, HookSlot, SLOT_CONFIG_LEN, SLOT_STATE_LEN};
pub use user::UserState;

/// ABI version carried in `HookContext::version`.
pub const ABI_VERSION: u8 = 1;
/// Inline hook slots per market (runtime + config-only modules).
pub const MAX_HOOK_SLOTS: usize = 6;
/// Hooks with at least one subscribed callback. Revisit with `docs/benchmarks.md`.
pub const MAX_RUNTIME_HOOKS: usize = 6;
/// Composer policies per market.
pub const MAX_POLICIES: usize = 4;
/// Trace entries recorded per routed trade (4 runtime hooks × 2 callbacks per phase × 2 phases).
pub const MAX_TRACE: usize = 16;
