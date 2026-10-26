use hook_types::{HookCallback, HookContext, HookDecision, MarketStats, UserState};

use crate::executor::HookError;

pub mod holder_xp;
pub mod launch_shield;
pub mod max_wallet;
pub mod sell_limit;
pub mod sentient;
pub mod trading_hours;
pub mod treasury_router;
pub mod twap;
pub mod volatility_guard;
pub mod volume_guard;

/// What a module sees for one callback. `state` and `user` are scratch copies: the executor
/// commits them only if the module holds the matching capability.
pub struct HookEnv<'a> {
    pub callback: HookCallback,
    pub ctx: &'a HookContext,
    pub config: &'a [u8; 48],
    pub state: &'a mut [u8; 48],
    pub user: &'a mut UserState,
    pub market: &'a MarketStats,
}

/// First-party module interface. External hooks implement the same contract over CPI
/// (`docs/hooks-spec-pump-router.md` §9).
pub trait PumpHook {
    const MODULE_ID: u16;
    fn validate_config(config: &[u8; 48]) -> Result<(), HookError>;
    /// Called once when the module is installed, to initialise its slot state.
    fn init_state(_config: &[u8; 48], _state: &mut [u8; 48], _now: i64) {}
    fn on_callback(env: &mut HookEnv<'_>) -> Result<HookDecision, HookError>;
}

pub(crate) fn bad_config(module: u16) -> HookError {
    HookError::BadConfig(module)
}
