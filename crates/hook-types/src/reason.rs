//! Reason codes. `0` = OK; `module_id * 1000 + n` for first-party modules; `9000 + n` for
//! Composer policies; `40000+` for external hooks.

pub const OK: u16 = 0;

pub mod launch_shield {
    pub const MAX_BUY_EXCEEDED: u16 = 1001;
    pub const COOLDOWN: u16 = 1002;
    pub const SLOT_BUY_CAP: u16 = 1003;
    pub const SAME_SLOT_ROUND_TRIP: u16 = 1004;
    pub const MAX_SELL_EXCEEDED: u16 = 1005;
}

pub mod volume_guard {
    pub const MAX_BUY_EXCEEDED: u16 = 2001;
    pub const MAX_SELL_EXCEEDED: u16 = 2002;
    pub const MARKET_WINDOW_EXCEEDED: u16 = 2003;
    pub const WALLET_WINDOW_EXCEEDED: u16 = 2004;
}

pub mod volatility_guard {
    pub const BUY_DEVIATION: u16 = 3001;
    pub const SELL_DEVIATION: u16 = 3002;
}

pub mod sentient {
    pub const DEFENSIVE_SIZE_CAP: u16 = 8001;
    pub const CIRCUIT_BREAKER: u16 = 8002;
}

pub mod max_wallet {
    pub const BALANCE_CAP: u16 = 10001;
}

pub mod sell_limit {
    pub const SELL_TOO_LARGE: u16 = 11001;
    pub const SELL_COOLDOWN: u16 = 11002;
}

pub mod trading_hours {
    pub const MARKET_CLOSED: u16 = 12001;
}

pub const POLICY_BASE: u16 = 9000;
pub const POLICY_MAX: u16 = 9999;
pub const EXTERNAL_BASE: u16 = 40000;

/// Human-readable label for a reason code (SDK, explorer, CLI).
pub fn describe(code: u16) -> &'static str {
    match code {
        OK => "ok",
        launch_shield::MAX_BUY_EXCEEDED => "Launch Shield: routed buy above the launch-phase limit",
        launch_shield::COOLDOWN => "Launch Shield: wallet cooldown active",
        launch_shield::SLOT_BUY_CAP => "Launch Shield: routed buys per slot exhausted",
        launch_shield::SAME_SLOT_ROUND_TRIP => "Launch Shield: buy and sell by one wallet in one slot",
        launch_shield::MAX_SELL_EXCEEDED => "Launch Shield: routed sell above the launch-phase limit",
        volume_guard::MAX_BUY_EXCEEDED => "Volume Guard: routed buy above max trade size",
        volume_guard::MAX_SELL_EXCEEDED => "Volume Guard: routed sell above max trade size",
        volume_guard::MARKET_WINDOW_EXCEEDED => "Volume Guard: rolling market routed volume limit",
        volume_guard::WALLET_WINDOW_EXCEEDED => "Volume Guard: rolling wallet routed volume limit",
        volatility_guard::BUY_DEVIATION => "Volatility Guard: price deviates from routed EMA (buy)",
        volatility_guard::SELL_DEVIATION => "Volatility Guard: price deviates from routed EMA (sell)",
        sentient::DEFENSIVE_SIZE_CAP => "SENTIENT: defensive regime size cap",
        sentient::CIRCUIT_BREAKER => "SENTIENT: circuit breaker regime",
        max_wallet::BALANCE_CAP => "Max Wallet: buy would take the wallet above the max balance",
        sell_limit::SELL_TOO_LARGE => "Sell Limit: sell is above the allowed share of the wallet's balance",
        sell_limit::SELL_COOLDOWN => "Sell Limit: wallet sold too recently",
        trading_hours::MARKET_CLOSED => "Trading Hours: outside trading hours",
        POLICY_BASE..=POLICY_MAX => "Custom policy denied the routed trade",
        c if c >= EXTERNAL_BASE => "External hook denied the routed trade",
        _ => "unknown",
    }
}
