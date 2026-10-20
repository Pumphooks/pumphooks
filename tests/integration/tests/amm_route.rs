//! Routed PumpSwap trades on an existing devnet Pump token (cloned state, real `pump_amm` binary).

mod common;

use common::*;
use hook_runtime::modules::{holder_xp, sentient, twap, volume_guard};
use hook_types::{HookCallback as Cb, ModuleId, Regime, SignalAction, StrategySignal};
use pump_hooks::events::{HookDeniedEvent, HookExecutionEvent, PolicyExecutionEvent, RoutedSwapEvent, StrategySignalApplied};
use solana_keypair::Keypair;

fn vg_config(max_buy: u64) -> [u8; 48] {
    volume_guard::Config { max_buy_quote: max_buy, max_sell_quote: max_buy, window_secs: 3_600, window_max_quote: 0, wallet_window_max_quote: 0 }.encode()
}
fn xp_config() -> [u8; 48] {
    holder_xp::Config { per_trade_cap_quote: 10 * SOL, xp_per_sol: 100, vest_secs: 3_600, daily_cap: 10_000, hold_xp_per_day: 0, hold_cap_base: 0, min_trade_quote: SOL / 100 }.encode()
}
fn twap_config() -> [u8; 48] {
    twap::Config { max_total_quote: 10 * SOL, max_slice_quote: SOL, min_interval_secs: 60, max_duration_secs: 86_400, max_slippage_bps: 300 }.encode()
}
fn sentient_config(signer: &Keypair) -> [u8; 48] {
    sentient::Config {
        signer: kp_pk(signer).to_bytes(),
        defensive_max_quote: SOL / 4,
        max_signal_ttl: 3_600,
        flags: sentient::FLAG_APPLY_BUYS | sentient::FLAG_APPLY_SELLS,
    }
    .encode()
}

struct Demo {
    env: Env,
    owner: Keypair,
    market: Pk,
    engine: Keypair,
}

/// The ultimate demo stack on a token that existed before Pump Hooks:
/// Volume Guard + TWAP + Holder XP + SENTIENT.
fn demo() -> Demo {
    let mut env = Env::new();
    env.bootstrap_protocol();
    let owner = Keypair::new();
    env.svm.airdrop(&solana_signer::Signer::pubkey(&owner), 10 * SOL).unwrap();
    let market = env.create_market(&owner);
    let engine = Keypair::new();
    env.install(&owner, &market, ModuleId::VolumeGuard as u16, vg_config(5 * SOL));
    env.install(&owner, &market, ModuleId::Twap as u16, twap_config());
    env.install(&owner, &market, ModuleId::HolderXp as u16, xp_config());
    env.install(&owner, &market, ModuleId::Sentient as u16, sentient_config(&engine));
    Demo { env, owner, market, engine }
}

#[test]
fn market_detects_existing_pump_token() {
    let d = demo();
    let m = d.env.read_market(&d.market);
    assert_eq!(m.mint, d.env.fx.mint);
    assert_eq!(m.pool, d.env.fx.pool);
    assert_eq!(m.venue, hook_types::Venue::PumpSwap as u8);
    assert_eq!(m.quote_mint, pump_adapter::NATIVE_MINT);
    assert_eq!(m.base_token_program, d.env.fx.base_token_program);
    assert!(!m.has_flag(pump_hooks::state::market_flags::CREATOR_VERIFIED), "owner is not the coin creator");
    let slots = m.read_slots();
    let ids: Vec<u16> = slots.iter().filter(|s| !s.is_empty()).map(|s| s.module_id).collect();
    assert_eq!(ids, vec![2, 4, 6, 8], "sorted by priority: risk, accounting, strategies");
}

#[test]
fn routed_buy_runs_before_and_after_hooks_around_real_pumpswap_cpi() {
    let mut d = demo();
    let user = d.env.new_wallet(5 * SOL, 3 * SOL);
    let u = kp_pk(&user);
    let (base_ata, quote_ata) = d.env.user_atas(&u);
    let (b0, q0) = (d.env.token_balance(&base_ata), d.env.token_balance(&quote_ata));

    let ix = d.env.route_amm_ix(&d.market, &u, 1, SOL / 2, 1, &[]);
    let meta = d.env.send_ok(&[ix], &[&user]);
    let (b1, q1) = (d.env.token_balance(&base_ata), d.env.token_balance(&quote_ata));
    assert!(b1 > b0, "received base");
    assert_eq!(q0 - q1, SOL / 2, "spent exactly the quote in");

    // Execution trace: BeforeSwap hooks, Pump CPI, AfterBuy/AfterSwap hooks.
    let trace = &events::<HookExecutionEvent>(&meta.logs)[0];
    let seq: Vec<(u8, u16)> = trace.entries.iter().map(|e| (e.callback, e.module_id)).collect();
    assert_eq!(
        seq,
        vec![
            (Cb::BeforeSwap as u8, 2),
            (Cb::BeforeSwap as u8, 8),
            (Cb::AfterBuy as u8, 4),
            (Cb::AfterSwap as u8, 2),
            (Cb::AfterSwap as u8, 8),
        ]
    );
    assert!(trace.entries.iter().all(|e| e.allowed));
    let swap = &events::<RoutedSwapEvent>(&meta.logs)[0];
    assert_eq!(swap.quote_amount, SOL / 2);
    assert_eq!(swap.base_amount, b1 - b0);
    assert_eq!(swap.pending_xp, 50);
    assert!(swap.post_quote_reserves > swap.pre_quote_reserves && swap.post_base_reserves < swap.pre_base_reserves);
    // The Pump CPI happened inside our instruction.
    let pos_pump = meta.logs.iter().position(|l| l.contains("Program pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA invoke [2]")).expect("pump amm CPI");
    let pos_ours = meta.logs.iter().position(|l| l.contains(&format!("Program {} invoke [1]", pump_hooks::ID))).unwrap();
    assert!(pos_ours < pos_pump);

    let m = d.env.read_market(&d.market);
    assert_eq!((m.trade_count, m.routed_buy_quote, m.sequence), (1, SOL / 2, 1));
    let us = d.env.read_user(&d.market, &u);
    assert_eq!((us.pending_xp, us.trade_count, us.routed_buy_quote), (50, 1, SOL / 2));
    println!("routed buy (VG+XP+SENTIENT, TWAP config-only): {} CU", meta.compute_units_consumed);
}

#[test]
fn exact_base_out_buy_v2() {
    let mut d = demo();
    let user = d.env.new_wallet(5 * SOL, 3 * SOL);
    let u = kp_pk(&user);
    let (base_ata, _) = d.env.user_atas(&u);
    let want = 1_000_000_000; // 1000 tokens at 6 decimals
    let ix = d.env.route_amm_ix(&d.market, &u, 0, want, 2 * SOL, &[]);
    let meta = d.env.send_ok(&[ix], &[&user]);
    assert_eq!(d.env.token_balance(&base_ata), want);
    let swap = &events::<RoutedSwapEvent>(&meta.logs)[0];
    assert_eq!(swap.base_amount, want);
    assert!(swap.quote_amount > 0 && swap.quote_amount <= 2 * SOL);
}

#[test]
fn volume_guard_denial_reverts_everything() {
    let mut d = demo();
    let user = d.env.new_wallet(5 * SOL, 8 * SOL);
    let u = kp_pk(&user);
    let (base_ata, quote_ata) = d.env.user_atas(&u);
    let q0 = d.env.token_balance(&quote_ata);
    let ix = d.env.route_amm_ix(&d.market, &u, 1, 6 * SOL, 1, &[]);
    let err = d.env.send(&[ix], &[&user]).unwrap_err();
    assert!(custom_error_name(&err).contains("HookDenied"), "{}", custom_error_name(&err));
    let denied = &events::<HookDeniedEvent>(&err.meta.logs)[0];
    assert_eq!((denied.module_id, denied.reason_code), (2, hook_types::reason::volume_guard::MAX_BUY_EXCEEDED));
    assert!(!err.meta.logs.iter().any(|l| l.contains("Program pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA invoke")), "Pump never called");
    assert_eq!(d.env.token_balance(&quote_ata), q0);
    assert_eq!(d.env.token_balance(&base_ata), 0);
    assert!(d.env.svm.get_account(&addr(&d.env.user_state(&d.market, &u))).map(|a| a.lamports == 0).unwrap_or(true));
}

#[test]
fn routed_sell_forfeits_unvested_xp() {
    let mut d = demo();
    let user = d.env.new_wallet(5 * SOL, 3 * SOL);
    let u = kp_pk(&user);
    let (base_ata, quote_ata) = d.env.user_atas(&u);
    let ix = d.env.route_amm_ix(&d.market, &u, 1, SOL, 1, &[]);
    d.env.send_ok(&[ix], &[&user]);
    assert_eq!(d.env.read_user(&d.market, &u).pending_xp, 100);

    d.env.advance(60, 150);
    let bal = d.env.token_balance(&base_ata);
    let q0 = d.env.token_balance(&quote_ata);
    let ix = d.env.route_amm_ix(&d.market, &u, 2, bal / 2, 1, &[]);
    let meta = d.env.send_ok(&[ix], &[&user]);
    let swap = &events::<RoutedSwapEvent>(&meta.logs)[0];
    assert_eq!(swap.side, 1);
    assert_eq!(swap.base_amount, bal / 2);
    assert_eq!(swap.quote_amount, d.env.token_balance(&quote_ata) - q0);
    let us = d.env.read_user(&d.market, &u);
    assert_eq!((us.xp, us.pending_xp, us.forfeited_xp), (0, 0, 100), "wash buy→sell earns nothing");
}

#[test]
fn kill_switch_fails_closed() {
    let mut d = demo();
    let user = d.env.new_wallet(5 * SOL, 3 * SOL);
    let u = kp_pk(&user);
    let admin = d.env.admin.insecure_clone();
    let ix = hooks_ix(
        pump_hooks::accounts::GlobalAdmin { authority: kp_pk(&admin), global: d.env.global() },
        vec![],
        pump_hooks::instruction::SetKillSwitch { disabled_modules: ModuleId::HolderXp.kill_bit(), paused: false },
    );
    d.env.send_ok(&[ix], &[&admin]);
    let ix = d.env.route_amm_ix(&d.market, &u, 1, SOL / 10, 1, &[]);
    let err = d.env.send(&[ix], &[&user]).unwrap_err();
    assert!(custom_error_name(&err).contains("ModuleDisabled"), "{}", custom_error_name(&err));

    // Non-admin cannot flip it.
    let ix = hooks_ix(
        pump_hooks::accounts::GlobalAdmin { authority: u, global: d.env.global() },
        vec![],
        pump_hooks::instruction::SetKillSwitch { disabled_modules: 0, paused: false },
    );
    assert!(d.env.send(&[ix], &[&user]).is_err());
}

#[test]
fn direct_pump_trades_bypass_the_router() {
    // Hooks only govern routed execution. A direct PumpSwap buy above the Volume Guard cap
    // succeeds; this test pins that documented limitation.
    let mut d = demo();
    let user = d.env.new_wallet(5 * SOL, 8 * SOL);
    let u = kp_pk(&user);
    let ix = d.env.direct_amm_ix(&u, pump_adapter::ix::Mode::ExactQuoteIn, 6 * SOL, 1);
    let meta = d.env.send_ok(&[ix], &[&user]);
    assert!(events::<HookExecutionEvent>(&meta.logs).is_empty());
    assert_eq!(d.env.read_market(&d.market).trade_count, 0);
}

#[test]
fn rejects_substituted_accounts() {
    let mut d = demo();
    let victim = d.env.new_wallet(2 * SOL, SOL);
    let attacker = d.env.new_wallet(5 * SOL, 3 * SOL);
    let a = kp_pk(&attacker);
    let (victim_base, _) = d.env.user_atas(&kp_pk(&victim));

    // Wrong base token account (someone else's) → rejected before Pump.
    let mut ix = d.env.route_amm_ix(&d.market, &a, 1, SOL / 10, 1, &[]);
    ix.accounts[5 + pump_adapter::accounts::amm_v2::USER_BASE_TOKEN_ACCOUNT].pubkey = addr(&victim_base);
    let err = d.env.send(&[ix], &[&attacker]).unwrap_err();
    assert!(custom_error_name(&err).contains("InvalidUserTokenAccount"), "{}", custom_error_name(&err));

    // Wrong pool → rejected.
    let mut ix = d.env.route_amm_ix(&d.market, &a, 1, SOL / 10, 1, &[]);
    ix.accounts[5 + pump_adapter::accounts::amm_v2::POOL].pubkey = addr(&d.env.fx.accounts["bonding_curve"]);
    let err = d.env.send(&[ix], &[&attacker]).unwrap_err();
    assert!(custom_error_name(&err).contains("PumpAccountMismatch"), "{}", custom_error_name(&err));

    // Missing Pump accounts → rejected.
    let mut ix = d.env.route_amm_ix(&d.market, &a, 1, SOL / 10, 1, &[]);
    ix.accounts.truncate(10);
    let err = d.env.send(&[ix], &[&attacker]).unwrap_err();
    assert!(custom_error_name(&err).contains("PumpAccountsMissing"), "{}", custom_error_name(&err));

    // A non-authority cannot install hooks on someone else's market.
    let ix = d.env.install_ix(&a, &d.market, ModuleId::LaunchShield as u16, 100, [0; 48]);
    assert!(d.env.send(&[ix], &[&attacker]).is_err());
}

#[test]
fn composer_policy_denies_large_routed_sells_and_awards_points() {
    use policy_vm::{action, field, op, trigger};
    use pump_hooks::instructions::{ActionArg, ConditionArg, PolicyArgs};

    let mut env = Env::new();
    env.bootstrap_protocol();
    let owner = Keypair::new();
    env.svm.airdrop(&solana_signer::Signer::pubkey(&owner), 10 * SOL).unwrap();
    let o = kp_pk(&owner);
    let market = env.create_market(&owner);
    env.install(&owner, &market, ModuleId::Policy as u16, [0; 48]);

    let mk = |index: u8, args: PolicyArgs| {
        hooks_ix(
            pump_hooks::accounts::CreatePolicy {
                authority: o,
                market,
                policy: Pk::find_program_address(&[b"policy", market.as_ref(), &[index]], &pump_hooks::ID).0,
                system_program: pump_adapter::SYSTEM_PROGRAM_ID,
            },
            vec![],
            pump_hooks::instruction::CreatePolicy { index, args },
        )
    };
    // WHEN BeforeSell IF trade size > 0.2 SOL THEN deny(42)
    let deny = PolicyArgs {
        trigger: trigger::BEFORE_SELL,
        combinator: 0,
        cooldown_secs: 0,
        deny_reason: 42,
        conditions: vec![ConditionArg { field: field::TRADE_SIZE, op: op::GT, arg: 0, value: SOL / 5 }],
        actions: vec![ActionArg { kind: action::DENY, arg: 0, arg16: 0, value: 0 }],
        label_hash: [1; 32],
    };
    // WHEN AfterBuy THEN award 25 points
    let award = PolicyArgs {
        trigger: trigger::AFTER_BUY,
        combinator: 0,
        cooldown_secs: 0,
        deny_reason: 0,
        conditions: vec![],
        actions: vec![ActionArg { kind: action::AWARD_POINTS, arg: 0, arg16: 0, value: 25 }],
        label_hash: [2; 32],
    };
    env.send_ok(&[mk(0, deny), mk(1, award)], &[&owner]);
    let pols = [env.policy(&market, 0), env.policy(&market, 1)];

    let user = env.new_wallet(5 * SOL, 3 * SOL);
    let u = kp_pk(&user);
    let meta = env.send_ok(&[env.route_amm_ix(&market, &u, 1, SOL, 1, &pols)], &[&user]);
    assert_eq!(env.read_user(&market, &u).policy_points, 25);
    let pe = events::<PolicyExecutionEvent>(&meta.logs);
    assert!(pe.iter().any(|e| e.policy_index == 1 && e.matched));

    let (base_ata, _) = env.user_atas(&u);
    let bal = env.token_balance(&base_ata);
    env.advance(5, 10);
    let err = env.send(&[env.route_amm_ix(&market, &u, 2, bal, 1, &pols)], &[&user]).unwrap_err();
    let denied = &events::<HookDeniedEvent>(&err.meta.logs)[0];
    assert_eq!((denied.module_id, denied.reason_code), (9, 9042));

    // Missing policy accounts fail closed.
    let err = env.send(&[env.route_amm_ix(&market, &u, 1, SOL / 10, 1, &[])], &[&user]).unwrap_err();
    assert!(custom_error_name(&err).contains("PolicyAccountMissing"), "{}", custom_error_name(&err));
}

#[test]
fn sentient_signal_moves_market_into_defensive_regime() {
    let mut d = demo();
    let user = d.env.new_wallet(5 * SOL, 3 * SOL);
    let u = kp_pk(&user);
    let now = d.env.now();
    let signal = StrategySignal {
        version: 1,
        action: SignalAction::ChangeStrategyState as u8,
        param: Regime::Defensive as u8,
        _pad: 0,
        confidence_bps: 8_200,
        amount_bps: 0,
        market: d.market.to_bytes(),
        nonce: 1,
        issued_at: now,
        valid_until: now + 600,
        _reserved: [0; 32],
    };
    let submit = |signal: &StrategySignal| {
        hooks_ix(
            pump_hooks::accounts::SubmitSignal {
                market: d.market,
                instructions: anchor_lang::solana_program::sysvar::instructions::ID,
            },
            vec![],
            pump_hooks::instruction::SubmitSignal { signal: bytemuck::bytes_of(signal).try_into().unwrap() },
        )
    };

    // Unsigned (no ed25519 instruction) → rejected.
    let err = d.env.send(&[submit(&signal)], &[&user]).unwrap_err();
    assert!(custom_error_name(&err).contains("SignalNotVerified"), "{}", custom_error_name(&err));
    // Signed by the wrong key → rejected.
    let impostor = Keypair::new();
    let err = d.env.send(&[ed25519_ix(&impostor, &signal.message()), submit(&signal)], &[&user]).unwrap_err();
    assert!(custom_error_name(&err).contains("SignalNotVerified"));

    // Relayed by anyone, signed by the engine → applied.
    let meta = d.env.send_ok(&[ed25519_ix(&d.engine, &signal.message()), submit(&signal)], &[&user]);
    let applied = &events::<StrategySignalApplied>(&meta.logs)[0];
    assert_eq!((applied.nonce, applied.param), (1, Regime::Defensive as u8));

    // Replay → rejected.
    let err = d.env.send(&[ed25519_ix(&d.engine, &signal.message()), submit(&signal)], &[&user]).unwrap_err();
    assert!(custom_error_name(&err).contains("SignalRejected"));

    // Defensive: routed buys above 0.25 SOL denied, smaller ones pass.
    let err = d.env.send(&[d.env.route_amm_ix(&d.market, &u, 1, SOL / 2, 1, &[])], &[&user]).unwrap_err();
    assert_eq!(events::<HookDeniedEvent>(&err.meta.logs)[0].reason_code, hook_types::reason::sentient::DEFENSIVE_SIZE_CAP);
    d.env.send_ok(&[d.env.route_amm_ix(&d.market, &u, 1, SOL / 5, 1, &[])], &[&user]);

    // Regime expires.
    d.env.advance(601, 1500);
    d.env.send_ok(&[d.env.route_amm_ix(&d.market, &u, 1, SOL / 2, 1, &[])], &[&user]);
    let _ = &d.owner;
}

#[test]
fn compute_benchmarks() {
    // direct vs routed with 0 / 1 / 2 / 4 runtime hooks. Written to target/benchmarks.json.
    let mut env = Env::new();
    env.bootstrap_protocol();
    let mut rows = vec![];

    let user = env.new_wallet(20 * SOL, 15 * SOL);
    let u = kp_pk(&user);
    let ix = env.direct_amm_ix(&u, pump_adapter::ix::Mode::ExactQuoteIn, SOL / 10, 1);
    let meta = env.send_ok(&[ix.clone()], &[&user]);
    // Second direct trade (accumulator already exists) is the steady-state number.
    env.advance(1, 2);
    let meta2 = env.send_ok(&[ix.clone()], &[&user]);
    rows.push(("direct PumpSwap buy_exact_quote_in_v2".to_string(), meta.compute_units_consumed, meta2.compute_units_consumed, ix.accounts.len(), ix.data.len()));

    let configs: Vec<(&str, Vec<(u16, [u8; 48])>)> = vec![
        ("routed, 0 hooks", vec![]),
        ("routed, 1 hook (Volume Guard)", vec![(2, vg_config(5 * SOL))]),
        ("routed, 2 hooks (+ Holder XP)", vec![(2, vg_config(5 * SOL)), (4, xp_config())]),
        (
            "routed, 4 hooks (+ SENTIENT, Policy)",
            vec![(2, vg_config(5 * SOL)), (4, xp_config()), (8, sentient_config(&Keypair::new())), (9, [0; 48])],
        ),
    ];
    for (label, hooks) in configs {
        let owner = Keypair::new();
        env.svm.airdrop(&solana_signer::Signer::pubkey(&owner), 5 * SOL).unwrap();
        let market = env.create_market(&owner);
        for (m, c) in hooks {
            env.install(&owner, &market, m, c);
        }
        let ix = env.route_amm_ix(&market, &u, 1, SOL / 10, 1, &[]);
        let first = env.send_ok(&[ix.clone()], &[&user]);
        env.advance(31, 80);
        let steady = env.send_ok(&[ix.clone()], &[&user]);
        rows.push((label.to_string(), first.compute_units_consumed, steady.compute_units_consumed, ix.accounts.len(), ix.data.len()));
    }
    for r in &rows {
        println!("{:<45} first={:>7} CU  steady={:>7} CU  accounts={:>2}  data={} B", r.0, r.1, r.2, r.3, r.4);
    }
    let json: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| serde_json::json!({"case": r.0, "cu_first": r.1, "cu_steady": r.2, "accounts": r.3, "ix_data_bytes": r.4}))
        .collect();
    let out = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/benchmarks.json");
    std::fs::write(out, serde_json::to_string_pretty(&json).unwrap()).unwrap();
}
