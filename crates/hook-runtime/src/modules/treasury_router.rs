//! Treasury Router: configuration-only module describing how a creator wants explicitly deposited
//! funds split. The strategy-vault program enforces the split; nothing here touches Pump fees.

use hook_types::slot::le;
use hook_types::{HookDecision, ModuleId};

use super::{bad_config, HookEnv, PumpHook};
use crate::executor::HookError;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub reserve_bps: u16,
    pub strategy_bps: u16,
    pub rewards_bps: u16,
    pub operations_bps: u16,
}

impl Config {
    pub fn decode(b: &[u8; 48]) -> Self {
        Self {
            reserve_bps: le::u16_at(b, 0),
            strategy_bps: le::u16_at(b, 2),
            rewards_bps: le::u16_at(b, 4),
            operations_bps: le::u16_at(b, 6),
        }
    }
    pub fn encode(&self) -> [u8; 48] {
        let mut b = [0u8; 48];
        le::put_u16(&mut b, 0, self.reserve_bps);
        le::put_u16(&mut b, 2, self.strategy_bps);
        le::put_u16(&mut b, 4, self.rewards_bps);
        le::put_u16(&mut b, 6, self.operations_bps);
        b
    }
}

pub struct TreasuryRouter;

impl PumpHook for TreasuryRouter {
    const MODULE_ID: u16 = ModuleId::TreasuryRouter as u16;

    fn validate_config(config: &[u8; 48]) -> Result<(), HookError> {
        let c = Config::decode(config);
        let sum = c.reserve_bps as u32 + c.strategy_bps as u32 + c.rewards_bps as u32 + c.operations_bps as u32;
        if sum != 10_000 || config[8..].iter().any(|&x| x != 0) {
            return Err(bad_config(Self::MODULE_ID));
        }
        Ok(())
    }

    fn on_callback(_env: &mut HookEnv<'_>) -> Result<HookDecision, HookError> {
        Ok(HookDecision::allow())
    }
}
