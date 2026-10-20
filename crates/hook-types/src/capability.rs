/// Capability bitmask. `HookDefinition.capabilities` is the ceiling; a slot stores the granted
/// subset and the runtime enforces it on every callback.
pub struct Capabilities;

impl Capabilities {
    pub const READ_MARKET: u32 = 1 << 0;
    pub const VETO_ROUTED_SWAP: u32 = 1 << 1;
    pub const WRITE_HOOK_STATE: u32 = 1 << 2;
    pub const WRITE_USER_STATE: u32 = 1 << 3;
    pub const AWARD_POINTS: u32 = 1 << 4;
    pub const TREASURY_SIGNAL: u32 = 1 << 5;
    pub const STRATEGY_EXECUTION: u32 = 1 << 6;
    pub const ORACLE_READ: u32 = 1 << 7;
    pub const EXTERNAL_CPI: u32 = 1 << 8;
    pub const EMIT_SIGNAL: u32 = 1 << 9;

    pub const ALL_KNOWN: u32 = (1 << 10) - 1;
    /// Rejected in v1: no external program receives control during a routed trade.
    pub const FORBIDDEN_V1: u32 = Self::EXTERNAL_CPI;

    pub const NAMES: [(u32, &'static str); 10] = [
        (Self::READ_MARKET, "READ_MARKET"),
        (Self::VETO_ROUTED_SWAP, "VETO_ROUTED_SWAP"),
        (Self::WRITE_HOOK_STATE, "WRITE_HOOK_STATE"),
        (Self::WRITE_USER_STATE, "WRITE_USER_STATE"),
        (Self::AWARD_POINTS, "AWARD_POINTS"),
        (Self::TREASURY_SIGNAL, "TREASURY_SIGNAL"),
        (Self::STRATEGY_EXECUTION, "STRATEGY_EXECUTION"),
        (Self::ORACLE_READ, "ORACLE_READ"),
        (Self::EXTERNAL_CPI, "EXTERNAL_CPI"),
        (Self::EMIT_SIGNAL, "EMIT_SIGNAL"),
    ];

    pub const fn is_subset(granted: u32, ceiling: u32) -> bool {
        granted & !ceiling == 0
    }
}
