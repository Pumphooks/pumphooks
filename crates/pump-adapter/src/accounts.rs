//! Account order of the 17-account trade sets. The router receives these as a contiguous block of
//! remaining accounts and forwards them unchanged to Pump.

/// `pump_amm` `buy_v2` / `buy_exact_quote_in_v2` / `sell_v2`.
pub mod amm_v2 {
    pub const POOL: usize = 0;
    pub const USER: usize = 1;
    pub const GLOBAL_CONFIG: usize = 2;
    pub const BASE_MINT: usize = 3;
    pub const QUOTE_MINT: usize = 4;
    pub const USER_BASE_TOKEN_ACCOUNT: usize = 5;
    pub const USER_QUOTE_TOKEN_ACCOUNT: usize = 6;
    pub const POOL_BASE_TOKEN_ACCOUNT: usize = 7;
    pub const POOL_QUOTE_TOKEN_ACCOUNT: usize = 8;
    pub const BASE_TOKEN_PROGRAM: usize = 9;
    pub const QUOTE_TOKEN_PROGRAM: usize = 10;
    pub const SYSTEM_PROGRAM: usize = 11;
    pub const USER_VOLUME_ACCUMULATOR: usize = 12;
    pub const FEE_CONFIG: usize = 13;
    pub const BUYBACK_FEE_RECIPIENT: usize = 14;
    pub const EVENT_AUTHORITY: usize = 15;
    pub const PROGRAM: usize = 16;
    pub const LEN: usize = 17;
    /// `(is_signer, is_writable)` per index, from the IDL.
    pub const META: [(bool, bool); LEN] = [
        (false, true),
        (true, true),
        (false, false),
        (false, false),
        (false, false),
        (false, true),
        (false, true),
        (false, true),
        (false, true),
        (false, false),
        (false, false),
        (false, false),
        (false, true),
        (false, false),
        (false, true),
        (false, false),
        (false, false),
    ];
}

/// `pump` `buy_v3` / `buy_exact_quote_in_v3` / `sell_v3`.
pub mod curve_v3 {
    pub const GLOBAL: usize = 0;
    pub const BASE_MINT: usize = 1;
    pub const QUOTE_MINT: usize = 2;
    pub const BASE_TOKEN_PROGRAM: usize = 3;
    pub const QUOTE_TOKEN_PROGRAM: usize = 4;
    pub const BONDING_CURVE: usize = 5;
    pub const ASSOCIATED_BASE_BONDING_CURVE: usize = 6;
    pub const ASSOCIATED_QUOTE_BONDING_CURVE: usize = 7;
    pub const USER: usize = 8;
    pub const ASSOCIATED_BASE_USER: usize = 9;
    pub const ASSOCIATED_QUOTE_USER: usize = 10;
    pub const USER_VOLUME_ACCUMULATOR: usize = 11;
    pub const FEE_CONFIG: usize = 12;
    pub const BUYBACK_FEE_RECIPIENT: usize = 13;
    pub const SYSTEM_PROGRAM: usize = 14;
    pub const EVENT_AUTHORITY: usize = 15;
    pub const PROGRAM: usize = 16;
    pub const LEN: usize = 17;
    pub const META: [(bool, bool); LEN] = [
        (false, false),
        (false, false),
        (false, false),
        (false, false),
        (false, false),
        (false, true),
        (false, true),
        (false, true),
        (true, true),
        (false, true),
        (false, true),
        (false, true),
        (false, false),
        (false, true),
        (false, false),
        (false, false),
        (false, false),
    ];
}
