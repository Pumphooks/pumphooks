//! Instruction data and account-key builders for routed trades.

use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use crate::{accounts, pda, PUMP_AMM_PROGRAM_ID, PUMP_PROGRAM_ID, SYSTEM_PROGRAM_ID};

pub mod disc {
    pub const AMM_BUY_V2: [u8; 8] = [184, 23, 238, 97, 103, 197, 211, 61];
    pub const AMM_BUY_EXACT_QUOTE_IN_V2: [u8; 8] = [194, 171, 28, 70, 104, 77, 91, 47];
    pub const AMM_SELL_V2: [u8; 8] = [93, 246, 130, 60, 231, 233, 64, 178];
    pub const CURVE_BUY_V3: [u8; 8] = [7, 5, 29, 196, 245, 23, 101, 80];
    pub const CURVE_BUY_EXACT_QUOTE_IN_V3: [u8; 8] = [225, 247, 80, 30, 213, 179, 132, 136];
    pub const CURVE_SELL_V3: [u8; 8] = [28, 146, 222, 119, 38, 196, 105, 213];
}

/// Routed trade modes, shared with `hook_types::TradeMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Mode {
    /// Buy exactly `amount` base for at most `limit` quote.
    ExactBaseOut = 0,
    /// Spend exactly `amount` quote for at least `limit` base.
    ExactQuoteIn = 1,
    /// Sell exactly `amount` base for at least `limit` quote.
    ExactBaseIn = 2,
}

impl Mode {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::ExactBaseOut),
            1 => Some(Self::ExactQuoteIn),
            2 => Some(Self::ExactBaseIn),
            _ => None,
        }
    }
    pub fn is_buy(self) -> bool {
        self != Self::ExactBaseIn
    }
}

fn data2(d: [u8; 8], a: u64, b: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(24);
    v.extend_from_slice(&d);
    v.extend_from_slice(&a.to_le_bytes());
    v.extend_from_slice(&b.to_le_bytes());
    v
}

/// AMM v2 data. Args are `(amount, limit)` in the order the IDL declares them.
pub fn amm_v2_data(mode: Mode, amount: u64, limit: u64) -> Vec<u8> {
    match mode {
        Mode::ExactBaseOut => data2(disc::AMM_BUY_V2, amount, limit),
        Mode::ExactQuoteIn => data2(disc::AMM_BUY_EXACT_QUOTE_IN_V2, amount, limit),
        Mode::ExactBaseIn => data2(disc::AMM_SELL_V2, amount, limit),
    }
}

/// Curve v3 data. Buys request a partial fill (`OptionBool(true)`), like `pump-rust-client`.
pub fn curve_v3_data(mode: Mode, amount: u64, limit: u64) -> Vec<u8> {
    match mode {
        Mode::ExactBaseOut => {
            let mut v = data2(disc::CURVE_BUY_V3, amount, limit);
            v.push(1);
            v
        }
        Mode::ExactQuoteIn => {
            let mut v = data2(disc::CURVE_BUY_EXACT_QUOTE_IN_V3, amount, limit);
            v.push(1);
            v
        }
        Mode::ExactBaseIn => data2(disc::CURVE_SELL_V3, amount, limit),
    }
}

/// Keys that identify a routed market's trade accounts.
#[derive(Clone, Copy, Debug)]
pub struct AmmKeys {
    pub pool: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub pool_base_token_account: Pubkey,
    pub pool_quote_token_account: Pubkey,
    pub base_token_program: Pubkey,
    pub quote_token_program: Pubkey,
}

pub fn amm_v2_account_keys(k: &AmmKeys, user: &Pubkey, buyback_fee_recipient_wallet: &Pubkey) -> [Pubkey; accounts::amm_v2::LEN] {
    [
        k.pool,
        *user,
        pda::amm::global_config(),
        k.base_mint,
        k.quote_mint,
        pda::associated_token(user, &k.base_token_program, &k.base_mint),
        pda::associated_token(user, &k.quote_token_program, &k.quote_mint),
        k.pool_base_token_account,
        k.pool_quote_token_account,
        k.base_token_program,
        k.quote_token_program,
        SYSTEM_PROGRAM_ID,
        pda::amm::user_volume_accumulator(user),
        pda::amm::fee_config(),
        pda::associated_token(buyback_fee_recipient_wallet, &k.quote_token_program, &k.quote_mint),
        pda::amm::event_authority(),
        PUMP_AMM_PROGRAM_ID,
    ]
}

#[derive(Clone, Copy, Debug)]
pub struct CurveKeys {
    pub base_mint: Pubkey,
    /// Resolved quote mint (wSOL for SOL curves).
    pub quote_mint: Pubkey,
    pub base_token_program: Pubkey,
    pub quote_token_program: Pubkey,
}

pub fn curve_v3_account_keys(k: &CurveKeys, user: &Pubkey, buyback_fee_recipient_wallet: &Pubkey) -> [Pubkey; accounts::curve_v3::LEN] {
    let bc = pda::pump::bonding_curve(&k.base_mint).0;
    let sol = k.quote_mint == crate::NATIVE_MINT;
    [
        pda::pump::global(),
        k.base_mint,
        k.quote_mint,
        k.base_token_program,
        k.quote_token_program,
        bc,
        pda::associated_token(&bc, &k.base_token_program, &k.base_mint),
        pda::associated_token(&bc, &k.quote_token_program, &k.quote_mint),
        *user,
        pda::associated_token(user, &k.base_token_program, &k.base_mint),
        pda::associated_token(user, &k.quote_token_program, &k.quote_mint),
        pda::pump::user_volume_accumulator(user),
        pda::pump::fee_config(),
        if sol {
            *buyback_fee_recipient_wallet
        } else {
            pda::associated_token(buyback_fee_recipient_wallet, &k.quote_token_program, &k.quote_mint)
        },
        SYSTEM_PROGRAM_ID,
        pda::pump::event_authority(),
        PUMP_PROGRAM_ID,
    ]
}

fn metas(keys: &[Pubkey], meta: &[(bool, bool)]) -> Vec<AccountMeta> {
    keys.iter()
        .zip(meta)
        .map(|(k, &(s, w))| if w { AccountMeta::new(*k, s) } else { AccountMeta::new_readonly(*k, s) })
        .collect()
}

/// Direct (un-routed) Pump AMM instruction; used for benchmarks and to verify equivalence.
pub fn amm_v2_instruction(k: &AmmKeys, user: &Pubkey, buyback: &Pubkey, mode: Mode, amount: u64, limit: u64) -> Instruction {
    Instruction {
        program_id: PUMP_AMM_PROGRAM_ID,
        accounts: metas(&amm_v2_account_keys(k, user, buyback), &accounts::amm_v2::META),
        data: amm_v2_data(mode, amount, limit),
    }
}

pub fn curve_v3_instruction(k: &CurveKeys, user: &Pubkey, buyback: &Pubkey, mode: Mode, amount: u64, limit: u64) -> Instruction {
    Instruction {
        program_id: PUMP_PROGRAM_ID,
        accounts: metas(&curve_v3_account_keys(k, user, buyback), &accounts::curve_v3::META),
        data: curve_v3_data(mode, amount, limit),
    }
}

/// The Pump trade metas as they must appear in the router's remaining accounts.
pub fn amm_v2_metas(k: &AmmKeys, user: &Pubkey, buyback: &Pubkey) -> Vec<AccountMeta> {
    metas(&amm_v2_account_keys(k, user, buyback), &accounts::amm_v2::META)
}

pub fn curve_v3_metas(k: &CurveKeys, user: &Pubkey, buyback: &Pubkey) -> Vec<AccountMeta> {
    metas(&curve_v3_account_keys(k, user, buyback), &accounts::curve_v3::META)
}
