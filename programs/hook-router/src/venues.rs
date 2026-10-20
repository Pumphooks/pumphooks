//! Venue adapters: which programs and instructions the router will forward a user's signature
//! to, and where the user's signer and token accounts sit in each instruction.
//!
//! Sources: `docs/research/amm-venues.md` (program ids, discriminators, account orders).
//! `verified` = every swap instruction listed for the venue is exercised through the router
//! against the venue's real binary (`tests/hooks/tests/router.rs`, `tests/hooks/tests/venues.rs`).
//! Every venue, verified or not, is refused until the admin enables it.

use anchor_lang::prelude::*;

#[derive(Clone, Copy, Debug)]
pub struct SwapIx {
    pub discriminator: [u8; 8],
    /// Index of the user's input / output token account in the venue instruction.
    pub input: usize,
    pub output: usize,
    /// Byte offset of a direction flag in the instruction data: when it is 0, `input` and
    /// `output` swap roles (Whirlpools' `a_to_b`).
    pub direction_flag: Option<usize>,
}

impl SwapIx {
    const fn new(discriminator: [u8; 8], input: usize, output: usize) -> Self {
        Self { discriminator, input, output, direction_flag: None }
    }

    /// `(input, output)` account indices for this instruction data.
    pub fn io(&self, data: &[u8]) -> Option<(usize, usize)> {
        match self.direction_flag {
            None => Some((self.input, self.output)),
            Some(at) => match *data.get(at)? {
                1 => Some((self.input, self.output)),
                0 => Some((self.output, self.input)),
                _ => None,
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Venue {
    pub id: u8,
    pub name: &'static str,
    pub program: Pubkey,
    /// Index of the transfer-authority signer (must be the router's user).
    pub signer: usize,
    /// Index of the pool / market account, which must be owned by `program`.
    pub pool: usize,
    pub min_accounts: usize,
    pub max_accounts: usize,
    pub swaps: &'static [SwapIx],
    pub verified: bool,
}

pub const PUMPSWAP: u8 = 0;
pub const RAYDIUM_CPMM: u8 = 1;
pub const RAYDIUM_CPMM_DEVNET: u8 = 2;
pub const METEORA_DAMM_V2: u8 = 3;
pub const RAYDIUM_CLMM: u8 = 4;
pub const RAYDIUM_CLMM_DEVNET: u8 = 5;
pub const ORCA_WHIRLPOOL: u8 = 6;
pub const METEORA_DLMM: u8 = 7;

const PUMPSWAP_SWAPS: &[SwapIx] = &[
    // buy_v2(base_amount_out, max_quote_in): quote ATA → base ATA
    SwapIx::new([184, 23, 238, 97, 103, 197, 211, 61], 6, 5),
    // buy_exact_quote_in_v2(quote_in, min_base_out)
    SwapIx::new([194, 171, 28, 70, 104, 77, 91, 47], 6, 5),
    // sell_v2(base_in, min_quote_out): base ATA → quote ATA
    SwapIx::new([93, 246, 130, 60, 231, 233, 64, 178], 5, 6),
];

const CPMM_SWAPS: &[SwapIx] = &[
    SwapIx::new([143, 190, 90, 218, 196, 30, 51, 222], 4, 5),
    SwapIx::new([55, 217, 98, 86, 163, 74, 180, 173], 4, 5),
];

const DAMM_V2_SWAPS: &[SwapIx] = &[
    SwapIx::new([65, 75, 63, 76, 235, 91, 91, 136], 2, 3),
    SwapIx::new([248, 198, 158, 145, 225, 117, 135, 200], 2, 3),
];

const CLMM_SWAPS: &[SwapIx] = &[
    // swap_v2(amount, other_amount_threshold, sqrt_price_limit_x64, is_base_input)
    SwapIx::new([43, 4, 237, 11, 26, 201, 30, 98], 3, 4),
];

const WHIRLPOOL_SWAPS: &[SwapIx] = &[
    // swap_v2(amount, other_amount_threshold, sqrt_price_limit, amount_specified_is_input, a_to_b,
    // remaining_accounts_info): owner account A is 7, B is 9; a_to_b sits at byte 41.
    SwapIx { discriminator: [43, 4, 237, 11, 26, 201, 30, 98], input: 7, output: 9, direction_flag: Some(41) },
];

const DLMM_SWAPS: &[SwapIx] = &[
    // swap2(amount_in, min_amount_out, remaining_accounts_info)
    SwapIx::new([65, 75, 63, 76, 235, 91, 91, 136], 4, 5),
];

pub const VENUES: &[Venue] = &[
    Venue {
        id: PUMPSWAP,
        name: "PumpSwap",
        program: pubkey!("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA"),
        signer: 1,
        pool: 0,
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
        pool: 3,
        min_accounts: 13,
        max_accounts: 13,
        swaps: CPMM_SWAPS,
        verified: true,
    },
    Venue {
        id: RAYDIUM_CPMM_DEVNET,
        name: "Raydium CPMM (devnet)",
        program: pubkey!("DRaycpLY18LhpbydsBWbVJtxpNv9oXPgjRSfpF2bWpYb"),
        signer: 0,
        pool: 3,
        min_accounts: 13,
        max_accounts: 13,
        swaps: CPMM_SWAPS,
        verified: true,
    },
    Venue {
        id: METEORA_DAMM_V2,
        name: "Meteora DAMM v2",
        program: pubkey!("cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG"),
        signer: 8,
        pool: 1,
        min_accounts: 14,
        max_accounts: 15,
        swaps: DAMM_V2_SWAPS,
        verified: true,
    },
    Venue {
        id: RAYDIUM_CLMM,
        name: "Raydium CLMM",
        program: pubkey!("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK"),
        signer: 0,
        pool: 2,
        // 13 fixed + optional bitmap extension + tick arrays.
        min_accounts: 14,
        max_accounts: 24,
        swaps: CLMM_SWAPS,
        verified: true,
    },
    Venue {
        id: RAYDIUM_CLMM_DEVNET,
        name: "Raydium CLMM (devnet)",
        program: pubkey!("DRayAUgENGQBKVaX8owNhgzkEDyoHTGVEGHVJT1E9pfH"),
        signer: 0,
        pool: 2,
        min_accounts: 14,
        max_accounts: 24,
        swaps: CLMM_SWAPS,
        verified: false,
    },
    Venue {
        id: ORCA_WHIRLPOOL,
        name: "Orca Whirlpools",
        program: pubkey!("whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc"),
        signer: 3,
        pool: 4,
        // 15 fixed + transfer-hook and supplemental tick-array slices.
        min_accounts: 15,
        max_accounts: 24,
        swaps: WHIRLPOOL_SWAPS,
        verified: true,
    },
    Venue {
        id: METEORA_DLMM,
        name: "Meteora DLMM",
        program: pubkey!("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo"),
        signer: 10,
        pool: 0,
        // 16 fixed + transfer-hook slices + bin arrays.
        min_accounts: 17,
        max_accounts: 28,
        swaps: DLMM_SWAPS,
        verified: true,
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
