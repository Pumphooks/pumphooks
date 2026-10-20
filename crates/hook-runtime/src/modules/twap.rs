//! TWAP: configuration-only ceilings for scheduled execution of **vault** funds. The
//! strategy-vault program refuses TWAP orders that exceed these market-set bounds. Slices route
//! through the same hook stack as any other routed trade.

use hook_types::slot::le;
use hook_types::{HookDecision, ModuleId};

use super::{bad_config, HookEnv, PumpHook};
use crate::executor::HookError;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub max_total_quote: u64,
    pub max_slice_quote: u64,
    pub min_interval_secs: u32,
    pub max_duration_secs: u32,
    pub max_slippage_bps: u16,
}

impl Config {
    pub fn decode(b: &[u8; 48]) -> Self {
        Self {
            max_total_quote: le::u64_at(b, 0),
            max_slice_quote: le::u64_at(b, 8),
            min_interval_secs: le::u32_at(b, 16),
            max_duration_secs: le::u32_at(b, 20),
            max_slippage_bps: le::u16_at(b, 24),
        }
    }
    pub fn encode(&self) -> [u8; 48] {
        let mut b = [0u8; 48];
        le::put_u64(&mut b, 0, self.max_total_quote);
        le::put_u64(&mut b, 8, self.max_slice_quote);
        le::put_u32(&mut b, 16, self.min_interval_secs);
        le::put_u32(&mut b, 20, self.max_duration_secs);
        le::put_u16(&mut b, 24, self.max_slippage_bps);
        b
    }
}

pub struct Twap;

impl PumpHook for Twap {
    const MODULE_ID: u16 = ModuleId::Twap as u16;

    fn validate_config(config: &[u8; 48]) -> Result<(), HookError> {
        let c = Config::decode(config);
        if c.max_total_quote == 0
            || c.max_slice_quote == 0
            || c.max_slice_quote > c.max_total_quote
            || c.min_interval_secs < 10
            || c.max_duration_secs == 0
            || c.max_slippage_bps == 0
            || c.max_slippage_bps > 2_000
            || config[26..].iter().any(|&x| x != 0)
        {
            return Err(bad_config(Self::MODULE_ID));
        }
        Ok(())
    }

    fn on_callback(_env: &mut HookEnv<'_>) -> Result<HookDecision, HookError> {
        Ok(HookDecision::allow())
    }
}
