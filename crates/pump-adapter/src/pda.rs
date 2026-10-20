use solana_program::pubkey::Pubkey;

use crate::{ATA_PROGRAM_ID, PUMP_AMM_PROGRAM_ID, PUMP_FEES_PROGRAM_ID, PUMP_PROGRAM_ID};

pub fn associated_token(owner: &Pubkey, token_program: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[owner.as_ref(), token_program.as_ref(), mint.as_ref()], &ATA_PROGRAM_ID).0
}

pub mod pump {
    use super::*;

    pub fn global() -> Pubkey {
        Pubkey::find_program_address(&[b"global"], &PUMP_PROGRAM_ID).0
    }
    pub fn bonding_curve(mint: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[b"bonding-curve", mint.as_ref()], &PUMP_PROGRAM_ID)
    }
    pub fn pool_authority(mint: &Pubkey) -> Pubkey {
        Pubkey::find_program_address(&[b"pool-authority", mint.as_ref()], &PUMP_PROGRAM_ID).0
    }
    pub fn event_authority() -> Pubkey {
        Pubkey::find_program_address(&[b"__event_authority"], &PUMP_PROGRAM_ID).0
    }
    pub fn user_volume_accumulator(user: &Pubkey) -> Pubkey {
        Pubkey::find_program_address(&[b"user_volume_accumulator", user.as_ref()], &PUMP_PROGRAM_ID).0
    }
    pub fn fee_config() -> Pubkey {
        Pubkey::find_program_address(&[b"fee_config", PUMP_PROGRAM_ID.as_ref()], &PUMP_FEES_PROGRAM_ID).0
    }
}

pub mod amm {
    use super::*;

    pub fn global_config() -> Pubkey {
        Pubkey::find_program_address(&[b"global_config"], &PUMP_AMM_PROGRAM_ID).0
    }
    pub fn pool(index: u16, creator: &Pubkey, base_mint: &Pubkey, quote_mint: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[b"pool", &index.to_le_bytes(), creator.as_ref(), base_mint.as_ref(), quote_mint.as_ref()],
            &PUMP_AMM_PROGRAM_ID,
        )
    }
    /// The pool Pump creates on migration: index 0, created by the curve's `pool-authority`.
    pub fn canonical_pool(base_mint: &Pubkey, quote_mint: &Pubkey) -> Pubkey {
        pool(0, &super::pump::pool_authority(base_mint), base_mint, quote_mint).0
    }
    pub fn event_authority() -> Pubkey {
        Pubkey::find_program_address(&[b"__event_authority"], &PUMP_AMM_PROGRAM_ID).0
    }
    pub fn user_volume_accumulator(user: &Pubkey) -> Pubkey {
        Pubkey::find_program_address(&[b"user_volume_accumulator", user.as_ref()], &PUMP_AMM_PROGRAM_ID).0
    }
    pub fn fee_config() -> Pubkey {
        Pubkey::find_program_address(&[b"fee_config", PUMP_AMM_PROGRAM_ID.as_ref()], &PUMP_FEES_PROGRAM_ID).0
    }
}
