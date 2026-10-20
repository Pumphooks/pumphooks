//! The adapter must produce byte-identical instructions to the official `pump-rust-client` and
//! parse its serialized accounts identically.

use anchor_lang::AccountSerialize;
use pump_adapter::{
    ix::{self, AmmKeys, CurveKeys, Mode},
    layout::{CurveView, PoolView},
    pda, NATIVE_MINT, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID,
};
use pump_rust_client::{sdk::PumpSdk, state::pump_amm::Pool};
use solana_program::pubkey::Pubkey;

type APubkey = anchor_lang::prelude::Pubkey;

fn a(k: &Pubkey) -> APubkey {
    APubkey::new_from_array(k.to_bytes())
}
fn k(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

fn same(ours: &solana_program::instruction::Instruction, theirs: &anchor_lang::solana_program::instruction::Instruction) {
    assert_eq!(ours.program_id.to_bytes(), theirs.program_id.to_bytes());
    assert_eq!(ours.data, theirs.data, "instruction data");
    assert_eq!(ours.accounts.len(), theirs.accounts.len());
    for (i, (x, y)) in ours.accounts.iter().zip(&theirs.accounts).enumerate() {
        assert_eq!(x.pubkey.to_bytes(), y.pubkey.to_bytes(), "account {i}");
        assert_eq!((x.is_signer, x.is_writable), (y.is_signer, y.is_writable), "flags {i}");
    }
}

fn sample_pool(base_mint: Pubkey) -> Pool {
    let mut p = Pool::default();
    p.pool_bump = 254;
    p.index = 0;
    p.creator = a(&pda::pump::pool_authority(&base_mint));
    p.base_mint = a(&base_mint);
    p.quote_mint = a(&NATIVE_MINT);
    p.lp_mint = a(&k(3));
    p.pool_base_token_account = a(&k(4));
    p.pool_quote_token_account = a(&k(5));
    p.lp_supply = 77;
    p.coin_creator = a(&k(6));
    p.is_mayhem_mode = false;
    p.is_cashback_coin = true;
    p.virtual_quote_reserves = -123_456_789;
    p.creator_fee_bps = 30;
    p.is_holder_reward = true;
    p.protocol_fees = 11;
    p.creator_fees = 22;
    p
}

#[test]
fn pool_layout_matches_idl_serialization() {
    let mint = k(9);
    let pool = sample_pool(mint);
    let mut buf = Vec::new();
    pool.try_serialize(&mut buf).unwrap();
    let v = PoolView::parse(&buf).unwrap();
    assert_eq!(v.base_mint, mint);
    assert_eq!(v.quote_mint, NATIVE_MINT);
    assert_eq!(v.pool_base_token_account, k(4));
    assert_eq!(v.pool_quote_token_account, k(5));
    assert_eq!(v.coin_creator, k(6));
    assert!(v.is_cashback && !v.is_mayhem && v.is_holder_reward);
    assert_eq!(v.virtual_quote_reserves, -123_456_789);
    assert_eq!((v.creator_fee_bps, v.protocol_fees, v.creator_fees), (30, 11, 22));
    assert_eq!(v.effective_quote_reserve(1_000_000_000), Some(1_000_000_000 - 123_456_789));
    assert_eq!(v.effective_quote_reserve(1), None);
    // Legacy short account (pre-fee-bucket fields) still parses with zeros.
    let short = PoolView::parse(&buf[..245]).unwrap();
    assert_eq!((short.virtual_quote_reserves, short.protocol_fees), (0, 0));
}

#[test]
fn curve_layout_matches_idl_serialization() {
    let mut c = pump_rust_client::state::BondingCurve::default();
    c.virtual_token_reserves = 1_073_000_000_000_000;
    c.virtual_quote_reserves = 30_000_000_000;
    c.real_token_reserves = 793_100_000_000_000;
    c.real_quote_reserves = 5;
    c.token_total_supply = 1_000_000_000_000_000;
    c.complete = true;
    c.creator = a(&k(7));
    c.is_mayhem_mode = true;
    c.quote_mint = APubkey::default();
    c.is_holder_reward = true;
    let mut buf = Vec::new();
    c.try_serialize(&mut buf).unwrap();
    let v = CurveView::parse(&buf).unwrap();
    assert_eq!(v.virtual_token_reserves, 1_073_000_000_000_000);
    assert_eq!(v.virtual_quote_reserves, 30_000_000_000);
    assert_eq!(v.real_token_reserves, 793_100_000_000_000);
    assert!(v.complete && v.is_mayhem && !v.is_cashback && v.is_holder_reward && v.is_sol_quoted());
    assert_eq!(v.creator, k(7));
}

#[test]
fn amm_v2_instructions_match_official_client() {
    let mint = k(9);
    let pool = sample_pool(mint);
    let user = k(1);
    let buyback = k(2);
    let pool_key = pda::amm::canonical_pool(&mint, &NATIVE_MINT);
    let keys = AmmKeys {
        pool: pool_key,
        base_mint: mint,
        quote_mint: NATIVE_MINT,
        pool_base_token_account: k(4),
        pool_quote_token_account: k(5),
        base_token_program: TOKEN_2022_PROGRAM_ID,
        quote_token_program: TOKEN_PROGRAM_ID,
    };
    let sdk = PumpSdk;
    let (bt, qt) = (a(&TOKEN_2022_PROGRAM_ID), a(&TOKEN_PROGRAM_ID));
    same(
        &ix::amm_v2_instruction(&keys, &user, &buyback, Mode::ExactBaseOut, 10, 20),
        &sdk.buy_amm_v2_instruction(&pool, bt, qt, a(&user), a(&buyback), 10, 20),
    );
    same(
        &ix::amm_v2_instruction(&keys, &user, &buyback, Mode::ExactQuoteIn, 30, 40),
        &sdk.buy_exact_quote_in_amm_v2_instruction(&pool, bt, qt, a(&user), a(&buyback), 30, 40),
    );
    same(
        &ix::amm_v2_instruction(&keys, &user, &buyback, Mode::ExactBaseIn, 50, 60),
        &sdk.sell_amm_v2_instruction(&pool, bt, qt, a(&user), a(&buyback), 50, 60),
    );
}

#[test]
fn curve_v3_instructions_match_official_client() {
    let sdk = PumpSdk;
    let user = k(1);
    let buyback = k(2);
    for (quote, qprog) in [(NATIVE_MINT, TOKEN_PROGRAM_ID), (k(8), TOKEN_2022_PROGRAM_ID)] {
        let keys = CurveKeys { base_mint: k(9), quote_mint: quote, base_token_program: TOKEN_2022_PROGRAM_ID, quote_token_program: qprog };
        let args = (a(&k(9)), a(&quote), a(&TOKEN_2022_PROGRAM_ID), a(&qprog), a(&user), a(&buyback));
        same(
            &ix::curve_v3_instruction(&keys, &user, &buyback, Mode::ExactBaseOut, 1, 2),
            &sdk.buy_v3_instruction(args.0, args.1, args.2, args.3, args.4, args.5, 1, 2),
        );
        same(
            &ix::curve_v3_instruction(&keys, &user, &buyback, Mode::ExactQuoteIn, 3, 4),
            &sdk.buy_exact_quote_in_v3_instruction(args.0, args.1, args.2, args.3, args.4, args.5, 3, 4),
        );
        same(
            &ix::curve_v3_instruction(&keys, &user, &buyback, Mode::ExactBaseIn, 5, 6),
            &sdk.sell_v3_instruction(args.0, args.1, args.2, args.3, args.4, args.5, 5, 6),
        );
    }
}

#[test]
fn pdas_match_official_client() {
    use pump_rust_client::pda as p;
    let mint = k(9);
    let user = k(1);
    assert_eq!(pda::pump::bonding_curve(&mint).0.to_bytes(), p::pump::bonding_curve(&a(&mint)).0.to_bytes());
    assert_eq!(pda::amm::canonical_pool(&mint, &NATIVE_MINT).to_bytes(), p::pump_amm::canonical_pool(&a(&mint), &a(&NATIVE_MINT)).0.to_bytes());
    assert_eq!(pda::amm::fee_config().to_bytes(), p::pump_amm::fee_config().0.to_bytes());
    assert_eq!(pda::pump::fee_config().to_bytes(), p::pump::fee_config().0.to_bytes());
    assert_eq!(pda::amm::user_volume_accumulator(&user).to_bytes(), p::pump_amm::user_volume_accumulator(&a(&user)).0.to_bytes());
    assert_eq!(pda::pump::user_volume_accumulator(&user).to_bytes(), p::pump::user_volume_accumulator(&a(&user)).0.to_bytes());
}
