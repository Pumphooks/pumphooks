//! The single place Pump Hooks touches Pump internals.
//!
//! Layouts, seeds and discriminators come from the official `pump` / `pump_amm` IDLs shipped with
//! `pump-rust-client` 0.4.1 and are cross-checked against that crate in `tests/`. Only the
//! instructions the router routes are covered: curve `buy_v3` / `buy_exact_quote_in_v3` /
//! `sell_v3` and AMM `buy_v2` / `buy_exact_quote_in_v2` / `sell_v2` (17 accounts each).

pub mod accounts;
pub mod ix;
pub mod layout;
pub mod math;
pub mod pda;

use solana_program::{pubkey, pubkey::Pubkey};

pub const PUMP_PROGRAM_ID: Pubkey = pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");
pub const PUMP_AMM_PROGRAM_ID: Pubkey = pubkey!("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA");
pub const PUMP_FEES_PROGRAM_ID: Pubkey = pubkey!("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");
pub const MAYHEM_PROGRAM_ID: Pubkey = pubkey!("MAyhSmzXzV1pTf7LsNkrNwkWKTo4ougAJ1PPg47MD4e");

pub const TOKEN_PROGRAM_ID: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const TOKEN_2022_PROGRAM_ID: Pubkey = pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
pub const ATA_PROGRAM_ID: Pubkey = pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
pub const NATIVE_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");
pub const SYSTEM_PROGRAM_ID: Pubkey = pubkey!("11111111111111111111111111111111");

/// Address lookup tables Pump publishes for its static accounts.
pub const MAINNET_ALT: Pubkey = pubkey!("Hyif6eWb8x88RVrvjPfabsgRYnwkVnyByEXTVTXbUcyP");
pub const DEVNET_ALT: Pubkey = pubkey!("7Je3GuLHSXZT6oC96EuAKTH8LsMhvC32z72cqVxUnvR7");

pub fn is_token_program(key: &Pubkey) -> bool {
    *key == TOKEN_PROGRAM_ID || *key == TOKEN_2022_PROGRAM_ID
}

/// `Pubkey::default()` and wSOL both mean a SOL-quoted curve.
pub fn resolve_quote_mint(q: &Pubkey) -> Pubkey {
    if *q == Pubkey::default() {
        NATIVE_MINT
    } else {
        *q
    }
}
