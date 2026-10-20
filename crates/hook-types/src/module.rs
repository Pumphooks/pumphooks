use crate::callback::HookCallback as Cb;
use crate::capability::Capabilities as C;

/// First-party module identifiers. Frozen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ModuleId {
    LaunchShield = 1,
    VolumeGuard = 2,
    VolatilityGuard = 3,
    HolderXp = 4,
    TreasuryRouter = 5,
    Twap = 6,
    ConditionalOrders = 7,
    Sentient = 8,
    Policy = 9,
}

impl ModuleId {
    pub const fn from_u16(v: u16) -> Option<Self> {
        match v {
            1 => Some(ModuleId::LaunchShield),
            2 => Some(ModuleId::VolumeGuard),
            3 => Some(ModuleId::VolatilityGuard),
            4 => Some(ModuleId::HolderXp),
            5 => Some(ModuleId::TreasuryRouter),
            6 => Some(ModuleId::Twap),
            7 => Some(ModuleId::ConditionalOrders),
            8 => Some(ModuleId::Sentient),
            9 => Some(ModuleId::Policy),
            _ => None,
        }
    }

    pub const fn info(self) -> &'static ModuleInfo {
        &MODULES[self as usize - 1]
    }

    /// Bit in `GlobalConfig.disabled_modules`.
    pub const fn kill_bit(self) -> u64 {
        1u64 << (self as u16)
    }
}

/// Static description of a first-party module. Mirrored in `hooks/*/hook.json`.
#[derive(Debug)]
pub struct ModuleInfo {
    pub id: u16,
    pub slug: &'static str,
    pub name: &'static str,
    pub version: u16,
    /// Callbacks the module can subscribe to.
    pub callbacks: u16,
    /// Callbacks subscribed by default when installed.
    pub default_callbacks: u16,
    pub capabilities: u32,
    pub priority_min: u16,
    pub priority_max: u16,
    pub default_priority: u16,
    /// Runtime modules consume one of `MAX_RUNTIME_HOOKS`; config-only modules do not.
    pub runtime: bool,
    /// Whether the module is implemented and installable in this release.
    pub available: bool,
}

const fn m(cbs: &[Cb]) -> u16 {
    let mut out = 0u16;
    let mut i = 0;
    while i < cbs.len() {
        out |= cbs[i].mask();
        i += 1;
    }
    out
}

const TRADE_BEFORE: u16 = m(&[Cb::BeforeBuy, Cb::BeforeSell, Cb::BeforeSwap]);
const TRADE_ALL: u16 = Cb::V1_MASK;

pub const MODULES: [ModuleInfo; 9] = [
    ModuleInfo {
        id: 1,
        slug: "launch-shield",
        name: "Launch Shield",
        version: 1,
        callbacks: TRADE_BEFORE,
        default_callbacks: m(&[Cb::BeforeBuy, Cb::BeforeSell]),
        capabilities: C::READ_MARKET | C::VETO_ROUTED_SWAP | C::WRITE_HOOK_STATE,
        priority_min: 0,
        priority_max: 999,
        default_priority: 100,
        runtime: true,
        available: true,
    },
    ModuleInfo {
        id: 2,
        slug: "volume-guard",
        name: "Volume Guard",
        version: 1,
        callbacks: TRADE_ALL,
        default_callbacks: m(&[Cb::BeforeSwap, Cb::AfterSwap]),
        capabilities: C::READ_MARKET | C::VETO_ROUTED_SWAP | C::WRITE_HOOK_STATE | C::WRITE_USER_STATE,
        priority_min: 1000,
        priority_max: 1999,
        default_priority: 1100,
        runtime: true,
        available: true,
    },
    ModuleInfo {
        id: 3,
        slug: "volatility-guard",
        name: "Volatility Guard",
        version: 1,
        callbacks: TRADE_ALL,
        default_callbacks: m(&[Cb::BeforeSwap, Cb::AfterSwap]),
        capabilities: C::READ_MARKET | C::VETO_ROUTED_SWAP | C::WRITE_HOOK_STATE,
        priority_min: 1000,
        priority_max: 1999,
        default_priority: 1200,
        runtime: true,
        available: true,
    },
    ModuleInfo {
        id: 4,
        slug: "holder-xp",
        name: "Holder XP",
        version: 1,
        callbacks: m(&[Cb::AfterBuy, Cb::AfterSell]),
        default_callbacks: m(&[Cb::AfterBuy, Cb::AfterSell]),
        capabilities: C::READ_MARKET | C::WRITE_HOOK_STATE | C::WRITE_USER_STATE | C::AWARD_POINTS,
        priority_min: 2000,
        priority_max: 2999,
        default_priority: 2100,
        runtime: true,
        available: true,
    },
    ModuleInfo {
        id: 5,
        slug: "treasury-router",
        name: "Treasury Router",
        version: 1,
        callbacks: 0,
        default_callbacks: 0,
        capabilities: C::READ_MARKET | C::TREASURY_SIGNAL,
        priority_min: 2000,
        priority_max: 2999,
        default_priority: 2500,
        runtime: false,
        available: true,
    },
    ModuleInfo {
        id: 6,
        slug: "twap",
        name: "TWAP",
        version: 1,
        callbacks: 0,
        default_callbacks: 0,
        capabilities: C::READ_MARKET | C::STRATEGY_EXECUTION,
        priority_min: 3000,
        priority_max: 3999,
        default_priority: 3100,
        runtime: false,
        available: true,
    },
    ModuleInfo {
        id: 7,
        slug: "conditional-orders",
        name: "Conditional Orders",
        version: 1,
        callbacks: 0,
        default_callbacks: 0,
        capabilities: C::READ_MARKET | C::STRATEGY_EXECUTION,
        priority_min: 3000,
        priority_max: 3999,
        default_priority: 3200,
        runtime: false,
        available: false,
    },
    ModuleInfo {
        id: 8,
        slug: "sentient",
        name: "SENTIENT",
        version: 1,
        callbacks: TRADE_ALL,
        default_callbacks: m(&[Cb::BeforeSwap, Cb::AfterSwap]),
        capabilities: C::READ_MARKET
            | C::VETO_ROUTED_SWAP
            | C::WRITE_HOOK_STATE
            | C::STRATEGY_EXECUTION
            | C::EMIT_SIGNAL,
        priority_min: 3000,
        priority_max: 3999,
        default_priority: 3500,
        runtime: true,
        available: true,
    },
    ModuleInfo {
        id: 9,
        slug: "policy",
        name: "Custom Policy",
        version: 1,
        callbacks: TRADE_ALL,
        default_callbacks: 0,
        capabilities: C::READ_MARKET
            | C::VETO_ROUTED_SWAP
            | C::WRITE_HOOK_STATE
            | C::WRITE_USER_STATE
            | C::AWARD_POINTS
            | C::TREASURY_SIGNAL
            | C::EMIT_SIGNAL,
        priority_min: 0,
        priority_max: 4999,
        default_priority: 4000,
        runtime: true,
        available: true,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_indexed_by_id() {
        for (i, info) in MODULES.iter().enumerate() {
            assert_eq!(info.id as usize, i + 1);
            let id = ModuleId::from_u16(info.id).unwrap();
            assert_eq!(id.info().slug, info.slug);
            assert!(info.default_callbacks & !info.callbacks == 0);
            assert!(info.priority_min <= info.default_priority && info.default_priority <= info.priority_max);
            assert!(info.capabilities & C::EXTERNAL_CPI == 0, "no first-party module needs CPI");
            assert_eq!(info.runtime, info.callbacks != 0);
        }
        assert!(ModuleId::from_u16(0).is_none());
        assert!(ModuleId::from_u16(10).is_none());
    }
}
