//! Venue adapters: which programs and instructions the router will forward a user's signature
//! to, and where the user's signer and token accounts sit in each instruction.
//!
//! Sources: `docs/research/amm-venues.md` (program ids, discriminators, account orders).
//! `verified` = exercised in `tests/hooks/tests/router.rs` against the venue's real binary.
//! Unverified venues exist in this table but are refused unless the admin enables them.

use anchor_lang::prelude::*;

#[derive(Clone, Copy, Debug)]
pub struct SwapIx {
    pub discriminator: [u8; 8],
    /// Index of the user's input / output token account in the venue instruction.
    pub input: usize,
    pub output: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Venue {
    pub id: u8,
    pub name: &'static str,
    pub program: Pubkey,
    /// Index of the transfer-authority signer (must be the router's user).
    pub signer: usize,
    pub min_accounts: usize,
    pub max_accounts: usize,
    pub swaps: &'static [SwapIx],
    pub verified: bool,
}

pub const PUMPSWAP: u8 = 0;
pub const RAYDIUM_CPMM: u8 = 1;
pub const RAYDIUM_CPMM_DEVNET: u8 = 2;
pub const METEORA_DAMM_V2: u8 = 3;

const PUMPSWAP_SWAPS: &[SwapIx] = &[
    // buy_v2(base_amount_out, max_quote_in): quote ATA → base ATA
    SwapIx { discriminator: [184, 23, 238, 97, 103, 197, 211, 61], input: 6, output: 5 },
    // buy_exact_quote_in_v2(quote_in, min_base_out)
    SwapIx { discriminator: [194, 171, 28, 70, 104, 77, 91, 47], input: 6, output: 5 },
    // sell_v2(base_in, min_quote_out): base ATA → quote ATA
    SwapIx { discriminator: [93, 246, 130, 60, 231, 233, 64, 178], input: 5, output: 6 },
];

const CPMM_SWAPS: &[SwapIx] = &[
    SwapIx { discriminator: [143, 190, 90, 218, 196, 30, 51, 222], input: 4, output: 5 },
    SwapIx { discriminator: [55, 217, 98, 86, 163, 74, 180, 173], input: 4, output: 5 },
];

const DAMM_V2_SWAPS: &[SwapIx] = &[
    SwapIx { discriminator: [65, 75, 63, 76, 235, 91, 91, 136], input: 2, output: 3 },
    SwapIx { discriminator: [248, 198, 158, 145, 225, 117, 135, 200], input: 2, output: 3 },
];

pub const VENUES: &[Venue] = &[
    Venue {
        id: PUMPSWAP,
        name: "PumpSwap",
        program: pubkey!("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA"),
        signer: 1,
        min_accounts: 17,
        max_accounts: 24,
        swaps: PUMPSWAP_SWAPS,
        verified: true,
    },
    Venue {
        id: RAYDIUM_CPMM,
        name: "Raydium CPMM",
        program: pubkey!("CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C"),
        signer: 0,
        min_accounts: 13,
        max_accounts: 13,
        swaps: CPMM_SWAPS,
        verified: false,
    },
    Venue {
        id: RAYDIUM_CPMM_DEVNET,
        name: "Raydium CPMM (devnet)",
        program: pubkey!("DRaycpLY18LhpbydsBWbVJtxpNv9oXPgjRSfpF2bWpYb"),
        signer: 0,
        min_accounts: 13,
        max_accounts: 13,
        swaps: CPMM_SWAPS,
        verified: false,
    },
    Venue {
        id: METEORA_DAMM_V2,
        name: "Meteora DAMM v2",
        program: pubkey!("cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG"),
        signer: 8,
        min_accounts: 14,
        max_accounts: 15,
        swaps: DAMM_V2_SWAPS,
        verified: false,
    },
];

pub fn by_program(program: &Pubkey) -> Option<&'static Venue> {
    VENUES.iter().find(|v| v.program == *program)
}

impl Venue {
    pub fn swap(&self, data: &[u8]) -> Option<&'static SwapIx> {
        let d: [u8; 8] = data.get(..8)?.try_into().ok()?;
        self.swaps.iter().find(|s| s.discriminator == d)
    }
}
