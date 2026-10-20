//! Routed bonding-curve trades on existing, un-migrated devnet Pump coins (cloned state, real
//! `pump` binary).

mod common;

use common::*;
use hook_runtime::modules::{holder_xp, volume_guard};
use hook_types::{HookCallback as Cb, ModuleId, Venue};
use pump_hooks::events::{HookDeniedEvent, HookExecutionEvent, RoutedSwapEvent};
use solana_keypair::Keypair;

fn setup(curve: &str) -> (Env, Pk) {
    let mut env = Env::curve(curve);
    env.bootstrap_protocol();
    let owner = Keypair::new();
    env.svm.airdrop(&addr(&kp_pk(&owner)), 10 * SOL).unwrap();
    let market = env.create_market(&owner);
    let vg = volume_guard::Config { max_buy_quote: SOL, max_sell_quote: SOL, window_secs: 3_600, window_max_quote: 0, wallet_window_max_quote: 0 };
    env.install(&owner, &market, ModuleId::VolumeGuard as u16, vg.encode());
    let xp = holder_xp::Config { per_trade_cap_quote: 10 * SOL, xp_per_sol: 100, vest_secs: 3_600, daily_cap: 10_000, hold_xp_per_day: 0, hold_cap_base: 0, min_trade_quote: SOL / 100 };
    env.install(&owner, &market, ModuleId::HolderXp as u16, xp.encode());
    (env, market)
}

fn curve_len(env: &Env) -> usize {
    env.svm.get_account(&addr(&env.fx.accounts["bonding_curve"])).unwrap().data.len()
}

#[test]
fn curve_market_detected() {
    let (env, market) = setup(DEEP_CURVE);
    let m = env.read_market(&market);
    assert_eq!(m.venue, Venue::BondingCurve as u8);
    assert_eq!(m.pool, Pk::default());
    assert_eq!(m.bonding_curve, env.fx.accounts["bonding_curve"]);
    assert_eq!(m.quote_mint, pump_adapter::NATIVE_MINT);
    assert_eq!(m.base_token_program, pump_adapter::TOKEN_PROGRAM_ID, "legacy create coin");
}

#[test]
fn routed_curve_buy_and_sell_with_sol_quote() {
    let (mut env, market) = setup(DEEP_CURVE);
    let user = env.new_wallet(3 * SOL, 0);
    let u = kp_pk(&user);
    let (base_ata, _) = env.user_atas(&u);
    assert_eq!(curve_len(&env), 82);

    let l0 = env.lamports(&u);
    let meta = env.send_ok(&[env.route_curve_ix(&market, &u, 1, SOL / 5, 1)], &[&user]);
    let swap = &events::<RoutedSwapEvent>(&meta.logs)[0];
    let got = env.token_balance(&base_ata);
    assert!(got > 0);
    assert_eq!(swap.base_amount, got);
    // Pump resizes the legacy curve and creates the volume accumulator at the wallet's expense;
    // the router reports the trade net of that rent.
    assert!(curve_len(&env) > 82);
    assert_eq!(swap.quote_amount, SOL / 5, "spendable quote fully used");
    assert!(l0 - env.lamports(&u) > swap.quote_amount);
    assert!(swap.post_quote_reserves > swap.pre_quote_reserves);
    let trace = &events::<HookExecutionEvent>(&meta.logs)[0];
    let seq: Vec<(u8, u16)> = trace.entries.iter().map(|e| (e.callback, e.module_id)).collect();
    assert_eq!(seq, vec![(Cb::BeforeSwap as u8, 2), (Cb::AfterBuy as u8, 4), (Cb::AfterSwap as u8, 2)]);
    assert_eq!(env.read_user(&market, &u).pending_xp, 20);

    env.advance(30, 75);
    let l1 = env.lamports(&u);
    let meta = env.send_ok(&[env.route_curve_ix(&market, &u, 2, got, 1)], &[&user]);
    let swap = &events::<RoutedSwapEvent>(&meta.logs)[0];
    assert_eq!(env.token_balance(&base_ata), 0);
    assert_eq!(swap.base_amount, got);
    assert!(swap.quote_amount > SOL / 5 * 95 / 100 && swap.quote_amount < SOL / 5);
    assert_eq!(env.lamports(&u) + 5_000, l1 + swap.quote_amount, "proceeds = lamport delta + tx fee");
    let us = env.read_user(&market, &u);
    assert!(us.forfeited_xp > 0 && us.xp == 0);
}

#[test]
fn routed_buy_that_completes_the_curve_then_routes_fail_until_migration() {
    let (mut env, market) = setup(NEAR_COMPLETE_CURVE);
    let user = env.new_wallet(3 * SOL, 0);
    let u = kp_pk(&user);
    let meta = env.send_ok(&[env.route_curve_ix(&market, &u, 1, SOL / 5, 1)], &[&user]);
    let swap = &events::<RoutedSwapEvent>(&meta.logs)[0];
    let curve = pump_adapter::layout::CurveView::parse(&env.svm.get_account(&addr(&env.fx.accounts["bonding_curve"])).unwrap().data).unwrap();
    // The router reports what the wallet observed, whatever fill Pump chose for a completing buy.
    assert_eq!(swap.quote_amount, SOL / 5);
    assert_eq!(swap.base_amount, env.token_balance(&env.user_atas(&u).0));
    assert!(curve.complete);
    assert_eq!(curve.real_token_reserves, 0);

    let err = env.send(&[env.route_curve_ix(&market, &u, 1, SOL / 10, 1)], &[&user]).unwrap_err();
    assert!(custom_error_name(&err).contains("MarketMigrated"), "{}", custom_error_name(&err));
}

#[test]
fn curve_guard_denies_and_venue_is_enforced() {
    let (mut env, market) = setup(DEEP_CURVE);
    let user = env.new_wallet(5 * SOL, 0);
    let u = kp_pk(&user);
    let err = env.send(&[env.route_curve_ix(&market, &u, 1, 2 * SOL, 1)], &[&user]).unwrap_err();
    assert_eq!(events::<HookDeniedEvent>(&err.meta.logs)[0].reason_code, hook_types::reason::volume_guard::MAX_BUY_EXCEEDED);

    // A curve market cannot be routed as PumpSwap.
    let mut ix = env.route_curve_ix(&market, &u, 1, SOL / 10, 1);
    ix.data[..8].copy_from_slice(&<pump_hooks::instruction::RouteAmm as anchor_lang::Discriminator>::DISCRIMINATOR[..8]);
    let err = env.send(&[ix], &[&user]).unwrap_err();
    assert!(custom_error_name(&err).contains("MarketNotMigrated"), "{}", custom_error_name(&err));
}
