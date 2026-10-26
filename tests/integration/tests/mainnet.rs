//! Routed trades against pump.fun's current mainnet binaries and live mainnet state (a coin on
//! its bonding curve and a graduated coin's PumpSwap pool, snapshotted by
//! `scripts/fetch_curve_fixtures.sh` / `scripts/fetch_fixtures.sh` with a mainnet RPC).

mod common;

use common::*;
use hook_runtime::modules::volume_guard;
use hook_types::{ModuleId, Venue};
use pump_hooks::events::{HookDeniedEvent, RoutedSwapEvent};
use solana_keypair::Keypair;

fn guarded(mut env: Env) -> (Env, Pk) {
    env.bootstrap_protocol();
    let owner = Keypair::new();
    env.svm.airdrop(&addr(&kp_pk(&owner)), 10 * SOL).unwrap();
    let market = env.create_market(&owner);
    let vg = volume_guard::Config { max_buy_quote: SOL, max_sell_quote: 0, window_secs: 0, window_max_quote: 0, wallet_window_max_quote: 0 };
    env.install(&owner, &market, ModuleId::VolumeGuard as u16, vg.encode());
    (env, market)
}

#[test]
fn mainnet_curve_routed_buy_sell_and_guard() {
    let (mut env, market) = guarded(Env::curve(MAINNET_CURVE));
    let m = env.read_market(&market);
    assert_eq!(m.venue, Venue::BondingCurve as u8);
    assert_eq!(m.bonding_curve, env.fx.accounts["bonding_curve"]);

    let user = env.new_wallet(5 * SOL, 0);
    let u = kp_pk(&user);
    let (base_ata, _) = env.user_atas(&u);

    let curve = env.fx.accounts["bonding_curve"];
    let fee_recipient = env.fx.buyback_recipient;
    let (c0, f0) = (env.lamports(&curve), env.lamports(&fee_recipient));
    let meta = env.send_ok(&[env.route_curve_ix(&market, &u, 1, SOL / 10, 1)], &[&user]);
    let swap = &events::<RoutedSwapEvent>(&meta.logs)[0];
    let got = env.token_balance(&base_ata);
    assert!(got > 0);
    assert_eq!(swap.base_amount, got);
    // Mainnet charges the buyback fee on top of the quote sent to the curve; the router reports
    // the wallet's real spend (net of rent for accounts Pump creates).
    let paid = (env.lamports(&curve) - c0) + (env.lamports(&fee_recipient) - f0);
    assert_eq!(swap.quote_amount, paid);
    assert!(swap.quote_amount >= SOL / 10 && swap.quote_amount < SOL / 10 * 101 / 100);

    let err = env.send(&[env.route_curve_ix(&market, &u, 1, 2 * SOL, 1)], &[&user]).unwrap_err();
    assert_eq!(events::<HookDeniedEvent>(&err.meta.logs)[0].reason_code, hook_types::reason::volume_guard::MAX_BUY_EXCEEDED);
    assert_eq!(env.token_balance(&base_ata), got, "denied buy changed nothing");

    env.advance(30, 75);
    let l1 = env.lamports(&u);
    let meta = env.send_ok(&[env.route_curve_ix(&market, &u, 2, got, 1)], &[&user]);
    let swap = &events::<RoutedSwapEvent>(&meta.logs)[0];
    assert_eq!(env.token_balance(&base_ata), 0);
    assert!(swap.quote_amount > SOL / 10 * 95 / 100 && swap.quote_amount < SOL / 10);
    assert_eq!(env.lamports(&u) + 5_000, l1 + swap.quote_amount, "proceeds = lamport delta + tx fee");
}

#[test]
fn mainnet_pool_routed_buy_sell_and_guard() {
    let (mut env, market) = guarded(Env::pool(MAINNET_POOL));
    let m = env.read_market(&market);
    assert_eq!(m.venue, Venue::PumpSwap as u8);
    assert_eq!(m.pool, env.fx.pool);

    let user = env.new_wallet(2 * SOL, 3 * SOL);
    let u = kp_pk(&user);
    let (base_ata, quote_ata) = env.user_atas(&u);

    let q0 = env.token_balance(&quote_ata);
    let meta = env.send_ok(&[env.route_amm_ix(&market, &u, 1, SOL / 10, 1, &[])], &[&user]);
    let swap = &events::<RoutedSwapEvent>(&meta.logs)[0];
    let got = env.token_balance(&base_ata);
    assert!(got > 0);
    assert_eq!(swap.base_amount, got);
    assert_eq!(q0 - env.token_balance(&quote_ata), swap.quote_amount);

    let err = env.send(&[env.route_amm_ix(&market, &u, 1, 2 * SOL, 1, &[])], &[&user]).unwrap_err();
    assert_eq!(events::<HookDeniedEvent>(&err.meta.logs)[0].reason_code, hook_types::reason::volume_guard::MAX_BUY_EXCEEDED);

    env.advance(30, 75);
    let q1 = env.token_balance(&quote_ata);
    let meta = env.send_ok(&[env.route_amm_ix(&market, &u, 2, got, 1, &[])], &[&user]);
    let swap = &events::<RoutedSwapEvent>(&meta.logs)[0];
    assert_eq!(env.token_balance(&base_ata), 0);
    assert_eq!(env.token_balance(&quote_ata) - q1, swap.quote_amount);
    assert!(swap.quote_amount > SOL / 10 * 95 / 100);
}

#[test]
fn mainnet_direct_pumpswap_trade_bypasses_the_router() {
    let (mut env, _market) = guarded(Env::pool(MAINNET_POOL));
    let user = env.new_wallet(2 * SOL, 3 * SOL);
    let u = kp_pk(&user);
    let (base_ata, _) = env.user_atas(&u);
    // 2 SOL is above the guard's cap, but sent straight to PumpSwap the guard never runs.
    env.send_ok(&[env.direct_amm_ix(&u, pump_adapter::ix::Mode::ExactQuoteIn, 2 * SOL, 1)], &[&user]);
    assert!(env.token_balance(&base_ata) > 0);
}

// ------------------------------------------------------------------ new pump.fun rules

use hook_runtime::modules::{holder_xp, launch_shield, max_wallet, sell_limit, trading_hours};

fn bare(mut env: Env) -> (Env, Pk, Keypair) {
    env.bootstrap_protocol();
    let owner = Keypair::new();
    env.svm.airdrop(&addr(&kp_pk(&owner)), 10 * SOL).unwrap();
    let market = env.create_market(&owner);
    (env, market, owner)
}

fn denial(err: &litesvm::types::FailedTransactionMetadata) -> u16 {
    events::<HookDeniedEvent>(&err.meta.logs).first().map(|e| e.reason_code).unwrap_or_else(|| panic!("{}", custom_error_name(err)))
}

#[test]
fn mainnet_max_wallet_caps_routed_buys() {
    let (mut env, market, owner) = bare(Env::curve(MAINNET_CURVE));
    // Size the cap from a probe buy on a copy of the same state.
    let probe = env.new_wallet(5 * SOL, 0);
    let p = kp_pk(&probe);
    env.send_ok(&[env.route_curve_ix(&market, &p, 1, SOL / 10, 1)], &[&probe]);
    let per_buy = env.token_balance(&env.user_atas(&p).0);

    let cap = per_buy * 3 / 2;
    env.install(&owner, &market, ModuleId::MaxWallet as u16, max_wallet::Config { max_base: cap, start_ts: 0, duration_secs: 0 }.encode());
    let user = env.new_wallet(5 * SOL, 0);
    let u = kp_pk(&user);
    let (base_ata, _) = env.user_atas(&u);
    env.send_ok(&[env.route_curve_ix(&market, &u, 1, SOL / 10, 1)], &[&user]);
    let held = env.token_balance(&base_ata);
    assert!(held > 0 && held <= cap);

    env.advance(5, 12);
    let err = env.send(&[env.route_curve_ix(&market, &u, 1, SOL / 10, 1)], &[&user]).unwrap_err();
    assert_eq!(denial(&err), hook_types::reason::max_wallet::BALANCE_CAP);
    assert_eq!(env.token_balance(&base_ata), held, "denied buy changed nothing");

    // Buying exactly the remaining room (exact base out) is allowed.
    let room = cap - held;
    env.send_ok(&[env.route_curve_ix(&market, &u, 0, room, SOL)], &[&user]);
    assert_eq!(env.token_balance(&base_ata), cap);
}

#[test]
fn mainnet_sell_limit_share_and_cooldown() {
    let (mut env, market, owner) = bare(Env::pool(MAINNET_POOL));
    let cfg = sell_limit::Config { max_sell_bps: 5_000, cooldown_secs: 60, exempt_below: 0, start_ts: 0, duration_secs: 0 };
    env.install(&owner, &market, ModuleId::SellLimit as u16, cfg.encode());
    let user = env.new_wallet(2 * SOL, 3 * SOL);
    let u = kp_pk(&user);
    let (base_ata, _) = env.user_atas(&u);
    env.send_ok(&[env.route_amm_ix(&market, &u, 1, SOL / 10, 1, &[])], &[&user]);
    let bag = env.token_balance(&base_ata);

    env.advance(5, 12);
    let err = env.send(&[env.route_amm_ix(&market, &u, 2, bag, 1, &[])], &[&user]).unwrap_err();
    assert_eq!(denial(&err), hook_types::reason::sell_limit::SELL_TOO_LARGE);
    env.send_ok(&[env.route_amm_ix(&market, &u, 2, bag / 2, 1, &[])], &[&user]);

    env.advance(30, 75);
    let err = env.send(&[env.route_amm_ix(&market, &u, 2, bag / 8, 1, &[])], &[&user]).unwrap_err();
    assert_eq!(denial(&err), hook_types::reason::sell_limit::SELL_COOLDOWN);
    env.advance(31, 78);
    env.send_ok(&[env.route_amm_ix(&market, &u, 2, bag / 8, 1, &[])], &[&user]);
}

#[test]
fn mainnet_trading_hours_close_and_open_the_market() {
    let (mut env, market, owner) = bare(Env::curve(MAINNET_CURVE));
    let minute = (env.now().rem_euclid(86_400) / 60) as u16;
    // Window opens one hour from now and lasts one hour.
    let cfg = trading_hours::Config {
        weekdays: 0,
        dst: trading_hours::DST_NONE,
        tz_offset_min: 0,
        open_min: (minute + 60) % 1440,
        close_min: (minute + 120) % 1440,
        sides: 0,
    };
    env.install(&owner, &market, ModuleId::TradingHours as u16, cfg.encode());
    let user = env.new_wallet(5 * SOL, 0);
    let u = kp_pk(&user);
    let err = env.send(&[env.route_curve_ix(&market, &u, 1, SOL / 10, 1)], &[&user]).unwrap_err();
    assert_eq!(denial(&err), hook_types::reason::trading_hours::MARKET_CLOSED);
    env.advance(3_600 + 60, 9_000);
    env.send_ok(&[env.route_curve_ix(&market, &u, 1, SOL / 10, 1)], &[&user]);
}

#[test]
fn six_rules_fit_the_compute_budget() {
    let (mut env, market, owner) = bare(Env::curve(MAINNET_CURVE));
    let shield = launch_shield::Config { start_ts: 0, duration_secs: 3_600, cooldown_secs: 0, max_buy_quote: 2 * SOL, max_sell_quote: 0, max_buys_per_slot: 3, flags: launch_shield::FLAG_DENY_SAME_SLOT_ROUND_TRIP };
    env.install(&owner, &market, ModuleId::LaunchShield as u16, shield.encode());
    let vg = volume_guard::Config { max_buy_quote: SOL, max_sell_quote: SOL, window_secs: 3_600, window_max_quote: 0, wallet_window_max_quote: 5 * SOL };
    env.install(&owner, &market, ModuleId::VolumeGuard as u16, vg.encode());
    let xp = holder_xp::Config { per_trade_cap_quote: 10 * SOL, xp_per_sol: 100, vest_secs: 3_600, daily_cap: 10_000, hold_xp_per_day: 0, hold_cap_base: 0, min_trade_quote: SOL / 100 };
    env.install(&owner, &market, ModuleId::HolderXp as u16, xp.encode());
    env.install(&owner, &market, ModuleId::MaxWallet as u16, max_wallet::Config { max_base: u64::MAX / 2, start_ts: 0, duration_secs: 0 }.encode());
    let sl = sell_limit::Config { max_sell_bps: 9_999, cooldown_secs: 1, exempt_below: u64::MAX, start_ts: 0, duration_secs: 0 };
    env.install(&owner, &market, ModuleId::SellLimit as u16, sl.encode());
    let minute = (env.now().rem_euclid(86_400) / 60) as u16;
    let th = trading_hours::Config { weekdays: 0, dst: trading_hours::DST_NONE, tz_offset_min: 0, open_min: (minute + 1380) % 1440, close_min: (minute + 60) % 1440, sides: 0 };
    env.install(&owner, &market, ModuleId::TradingHours as u16, th.encode());

    let user = env.new_wallet(5 * SOL, 0);
    let u = kp_pk(&user);
    let buy = env.send_ok(&[env.route_curve_ix(&market, &u, 1, SOL / 10, 1)], &[&user]);
    let trace = &events::<pump_hooks::events::HookExecutionEvent>(&buy.logs)[0];
    let ran: std::collections::BTreeSet<u16> = trace.entries.iter().map(|e| e.module_id).collect();
    assert_eq!(ran.into_iter().collect::<Vec<_>>(), vec![1, 2, 4, 10, 12], "every buy-side rule ran");
    env.advance(5, 12);
    let bag = env.token_balance(&env.user_atas(&u).0);
    let sell = env.send_ok(&[env.route_curve_ix(&market, &u, 2, bag / 2, 1)], &[&user]);
    println!("six rules on mainnet pump: first buy {} CU, sell {} CU", buy.compute_units_consumed, sell.compute_units_consumed);
    assert!(buy.compute_units_consumed < 400_000 && sell.compute_units_consumed < 400_000);
}
