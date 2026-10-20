//! Read-only views over Pump account data. Short (legacy) accounts read as zero past their end,
//! matching `pump-rust-client`'s `AccountWrapper` padding.

use solana_program::pubkey::Pubkey;

pub const POOL_DISCRIMINATOR: [u8; 8] = [241, 154, 109, 4, 17, 177, 109, 188];
pub const BONDING_CURVE_DISCRIMINATOR: [u8; 8] = [23, 183, 248, 55, 96, 216, 172, 96];
pub const AMM_GLOBAL_CONFIG_DISCRIMINATOR: [u8; 8] = [149, 8, 156, 202, 160, 252, 176, 217];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    WrongDiscriminator,
    TooShort,
}

fn bytes<const N: usize>(d: &[u8], off: usize) -> [u8; N] {
    let mut out = [0u8; N];
    if off < d.len() {
        let end = (off + N).min(d.len());
        out[..end - off].copy_from_slice(&d[off..end]);
    }
    out
}
fn u64_at(d: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(bytes(d, off))
}
fn i128_at(d: &[u8], off: usize) -> i128 {
    i128::from_le_bytes(bytes(d, off))
}
fn key_at(d: &[u8], off: usize) -> Pubkey {
    Pubkey::new_from_array(bytes(d, off))
}
fn bool_at(d: &[u8], off: usize) -> bool {
    bytes::<1>(d, off)[0] != 0
}

/// `pump_amm::Pool`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolView {
    pub index: u16,
    pub creator: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub lp_mint: Pubkey,
    pub pool_base_token_account: Pubkey,
    pub pool_quote_token_account: Pubkey,
    pub coin_creator: Pubkey,
    pub is_mayhem: bool,
    pub is_cashback: bool,
    pub virtual_quote_reserves: i128,
    pub creator_fee_bps: u64,
    pub is_holder_reward: bool,
    pub protocol_fees: u64,
    pub creator_fees: u64,
}

impl PoolView {
    pub fn parse(d: &[u8]) -> Result<Self, LayoutError> {
        if d.len() < 8 + 203 {
            return Err(LayoutError::TooShort);
        }
        if d[..8] != POOL_DISCRIMINATOR {
            return Err(LayoutError::WrongDiscriminator);
        }
        Ok(Self {
            index: u16::from_le_bytes(bytes(d, 9)),
            creator: key_at(d, 11),
            base_mint: key_at(d, 43),
            quote_mint: key_at(d, 75),
            lp_mint: key_at(d, 107),
            pool_base_token_account: key_at(d, 139),
            pool_quote_token_account: key_at(d, 171),
            coin_creator: key_at(d, 211),
            is_mayhem: bool_at(d, 243),
            is_cashback: bool_at(d, 244),
            virtual_quote_reserves: i128_at(d, 245),
            creator_fee_bps: u64_at(d, 261),
            is_holder_reward: bool_at(d, 270),
            protocol_fees: u64_at(d, 271),
            creator_fees: u64_at(d, 279),
        })
    }

    /// Quote reserve the pool prices against: vault balance + `virtual_quote_reserves` (signed).
    pub fn effective_quote_reserve(&self, quote_vault_balance: u64) -> Option<u64> {
        let v = (quote_vault_balance as i128).checked_add(self.virtual_quote_reserves)?;
        u64::try_from(v).ok()
    }
}

/// `pump::BondingCurve`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CurveView {
    pub virtual_token_reserves: u64,
    pub virtual_quote_reserves: u64,
    pub real_token_reserves: u64,
    pub real_quote_reserves: u64,
    pub token_total_supply: u64,
    pub complete: bool,
    pub creator: Pubkey,
    pub is_mayhem: bool,
    pub is_cashback: bool,
    /// Raw field; `Pubkey::default()` means SOL (see [`crate::resolve_quote_mint`]).
    pub quote_mint: Pubkey,
    pub is_holder_reward: bool,
}

impl CurveView {
    pub fn parse(d: &[u8]) -> Result<Self, LayoutError> {
        if d.len() < 8 + 41 {
            return Err(LayoutError::TooShort);
        }
        if d[..8] != BONDING_CURVE_DISCRIMINATOR {
            return Err(LayoutError::WrongDiscriminator);
        }
        Ok(Self {
            virtual_token_reserves: u64_at(d, 8),
            virtual_quote_reserves: u64_at(d, 16),
            real_token_reserves: u64_at(d, 24),
            real_quote_reserves: u64_at(d, 32),
            token_total_supply: u64_at(d, 40),
            complete: bool_at(d, 48),
            creator: key_at(d, 49),
            is_mayhem: bool_at(d, 81),
            is_cashback: bool_at(d, 82),
            quote_mint: key_at(d, 83),
            is_holder_reward: bool_at(d, 124),
        })
    }
    pub fn is_sol_quoted(&self) -> bool {
        self.quote_mint == Pubkey::default() || self.quote_mint == crate::NATIVE_MINT
    }
}

/// `pump_amm::GlobalConfig` fields the router cares about.
pub fn amm_buyback_fee_recipients(d: &[u8]) -> Result<[Pubkey; 8], LayoutError> {
    if d.len() < 899 {
        return Err(LayoutError::TooShort);
    }
    if d[..8] != AMM_GLOBAL_CONFIG_DISCRIMINATOR {
        return Err(LayoutError::WrongDiscriminator);
    }
    Ok(core::array::from_fn(|i| key_at(d, 643 + 32 * i)))
}

/// SPL Token / Token-2022 account base layout (identical for both programs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenAccountView {
    pub mint: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
}

impl TokenAccountView {
    pub fn parse(d: &[u8]) -> Result<Self, LayoutError> {
        if d.len() < 165 {
            return Err(LayoutError::TooShort);
        }
        Ok(Self { mint: key_at(d, 0), owner: key_at(d, 32), amount: u64_at(d, 64) })
    }
}
