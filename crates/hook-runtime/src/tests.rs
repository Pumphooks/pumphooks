use super::*;
use crate::modules::{holder_xp, launch_shield, sentient, treasury_router, volatility_guard as vg, volume_guard};
use bytemuck::Zeroable;
use hook_types::{reason, Regime, SignalAction, StrategySignal, TradeMode};
use policy_vm::{action, field, op, trigger, Action, Condition, PolicySpec, ValidationEnv};

const SOL: u64 = 1_000_000_000;
const T0: i64 = 1_800_000_000;

fn caps(m: ModuleId) -> u32 {
    m.info().capabilities
}

fn prio(m: ModuleId) -> u16 {
    m.info().default_priority
}

fn ctx(side: Side, quote: u64, now: i64, slot: u64) -> HookContext {
    let mut c = HookContext::zeroed();
    c.version = 1;
    c.side = side as u8;
    c.mode = if side == Side::Buy { TradeMode::ExactQuoteIn as u8 } else { TradeMode::ExactBaseIn as u8 };
    c.quote_amount = quote;
    c.base_amount = quote * 1_000;
    c.pre_base_reserves = 1_000_000_000_000_000;
    c.pre_quote_reserves = 100 * SOL;
    c.post_base_reserves = c.pre_base_reserves;
    c.post_quote_reserves = c.pre_quote_reserves;
    c.timestamp = now;
    c.slot = slot;
    c.market_activated_at = T0;
    c
}

struct Harness {
    slots: [HookSlot; MAX_HOOK_SLOTS],
    user: UserState,
    market: MarketStats,
    disabled: u64,
}

impl Harness {
    fn new() -> Self {
        Self { slots: [HookSlot::zeroed(); MAX_HOOK_SLOTS], user: UserState::default(), market: MarketStats::default(), disabled: 0 }
    }
    fn install(&mut self, m: ModuleId, config: [u8; 48]) -> usize {
        install_slot(&mut self.slots, m as u16, prio(m), 0, caps(m), config).unwrap()
    }
    /// Full routed trade: before phase, then (if allowed) after phase with observed amounts.
    fn trade(&mut self, c: HookContext, policies: &mut [Option<&mut PolicyData>; MAX_POLICIES]) -> Result<(Option<TraceEntry>, Trace), HookError> {
        let side = Side::from_u8(c.side).unwrap();
        let mut c = c;
        let mut trace = Trace::default();
        let before = run_phase(side, true, &mut self.slots, &mut c, &self.market, &mut self.user, policies, self.disabled, &mut trace)?;
        if before.denied.is_some() {
            return Ok((before.denied, trace));
        }
        c.flags |= hook_types::context_flags::AMOUNTS_OBSERVED;
        if side == Side::Buy {
            c.user_base_balance = c.user_base_balance.saturating_add(c.base_amount);
        } else {
            c.user_base_balance = c.user_base_balance.saturating_sub(c.base_amount);
        }
        run_phase(side, false, &mut self.slots, &mut c, &self.market, &mut self.user, policies, self.disabled, &mut trace)?;
        // Router bookkeeping (mirrors programs/pump-hooks commit step).
        self.user.last_trade_ts = c.timestamp;
        self.user.last_trade_slot = c.slot;
        self.user.last_side = c.side;
        self.user.trade_count += 1;
        if side == Side::Buy && self.user.first_buy_ts == 0 {
            self.user.first_buy_ts = c.timestamp;
        }
        Ok((None, trace))
    }
    fn go(&mut self, c: HookContext) -> Result<(Option<TraceEntry>, Trace), HookError> {
        self.trade(c, &mut [None, None, None, None])
    }
}

fn shield(cfg: launch_shield::Config) -> [u8; 48] {
    cfg.encode()
}

fn default_shield() -> launch_shield::Config {
    launch_shield::Config {
        start_ts: 0,
        duration_secs: 600,
        cooldown_secs: 30,
        max_buy_quote: 2 * SOL,
        max_sell_quote: 0,
        max_buys_per_slot: 2,
        flags: launch_shield::FLAG_DENY_SAME_SLOT_ROUND_TRIP,
    }
}

fn default_volume() -> volume_guard::Config {
    volume_guard::Config { max_buy_quote: 5 * SOL, max_sell_quote: 5 * SOL, window_secs: 3_600, window_max_quote: 20 * SOL, wallet_window_max_quote: 8 * SOL }
}

fn default_xp() -> holder_xp::Config {
    holder_xp::Config { per_trade_cap_quote: 10 * SOL, xp_per_sol: 100, vest_secs: 3_600, daily_cap: 1_000, hold_xp_per_day: 0, hold_cap_base: 0, min_trade_quote: SOL / 100 }
}

// ---------------------------------------------------------------- install / ordering

#[test]
fn install_orders_by_priority_regardless_of_insertion() {
    let mut h = Harness::new();
    h.install(ModuleId::HolderXp, default_xp().encode());
    h.install(ModuleId::VolumeGuard, default_volume().encode());
    h.install(ModuleId::LaunchShield, shield(default_shield()));
    let order: [u16; 3] = [h.slots[0].module_id, h.slots[1].module_id, h.slots[2].module_id];
    assert_eq!(order, [1, 2, 4]);

    let (denied, trace) = h.go(ctx(Side::Buy, SOL, T0 + 10, 100)).unwrap();
    assert!(denied.is_none());
    let seq: [(u8, u16); 4] = core::array::from_fn(|i| (trace.entries[i].callback, trace.entries[i].module_id));
    assert_eq!(
        seq,
        [
            (HookCallback::BeforeBuy as u8, 1),
            (HookCallback::BeforeSwap as u8, 2),
            (HookCallback::AfterBuy as u8, 4),
            (HookCallback::AfterSwap as u8, 2),
        ]
    );
    assert_eq!(trace.len, 4);
}

#[test]
fn rejects_more_than_max_runtime_hooks() {
    let mut h = Harness::new();
    h.install(ModuleId::LaunchShield, shield(default_shield()));
    h.install(ModuleId::VolumeGuard, default_volume().encode());
    h.install(ModuleId::HolderXp, default_xp().encode());
    h.install(ModuleId::Policy, [0; 48]);
    let vcfg = vg::Config { max_deviation_bps: 2_000, ema_alpha_bps: 2_000, min_observations: 3, check_min_quote: 0, flags: vg::FLAG_CHECK_BUYS };
    assert_eq!(
        install_slot(&mut h.slots, 3, prio(ModuleId::VolatilityGuard), 0, caps(ModuleId::VolatilityGuard), vcfg.encode()),
        Err(HookError::TooManyRuntimeHooks)
    );
    // Config-only modules still fit.
    let t = treasury_router::Config { reserve_bps: 5_000, strategy_bps: 3_000, rewards_bps: 1_000, operations_bps: 1_000 };
    assert!(install_slot(&mut h.slots, 5, prio(ModuleId::TreasuryRouter), 0, caps(ModuleId::TreasuryRouter), t.encode()).is_ok());
}

#[test]
fn install_validation() {
    let mut s = [HookSlot::zeroed(); MAX_HOOK_SLOTS];
    let cfg = shield(default_shield());
    assert_eq!(install_slot(&mut s, 99, 0, 0, 0, cfg), Err(HookError::UnknownModule(99)));
    assert_eq!(install_slot(&mut s, 7, 3200, 0, 0, [0; 48]), Err(HookError::ModuleUnavailable(7)));
    assert_eq!(install_slot(&mut s, 1, 1500, 0, caps(ModuleId::LaunchShield), cfg), Err(HookError::BadPriority(1)));
    assert_eq!(
        install_slot(&mut s, 1, 100, HookCallback::AfterBuy.mask(), caps(ModuleId::LaunchShield), cfg),
        Err(HookError::BadCallbacks(1))
    );
    assert_eq!(
        install_slot(&mut s, 1, 100, 0, caps(ModuleId::LaunchShield) | C::EXTERNAL_CPI, cfg),
        Err(HookError::CapabilityNotAllowed(1))
    );
    assert_eq!(install_slot(&mut s, 1, 100, 0, C::AWARD_POINTS, cfg), Err(HookError::CapabilityNotAllowed(1)));
    let mut bad = default_shield();
    bad.duration_secs = 0;
    assert_eq!(install_slot(&mut s, 1, 100, 0, caps(ModuleId::LaunchShield), bad.encode()), Err(HookError::BadConfig(1)));
    install_slot(&mut s, 1, 100, 0, caps(ModuleId::LaunchShield), cfg).unwrap();
    assert_eq!(install_slot(&mut s, 1, 100, 0, caps(ModuleId::LaunchShield), cfg), Err(HookError::DuplicateModule(1)));
}

#[test]
fn remove_keeps_order() {
    let mut h = Harness::new();
    h.install(ModuleId::LaunchShield, shield(default_shield()));
    h.install(ModuleId::VolumeGuard, default_volume().encode());
    h.install(ModuleId::HolderXp, default_xp().encode());
    remove_slot(&mut h.slots, 2).unwrap();
    assert_eq!((h.slots[0].module_id, h.slots[1].module_id, h.slots[2].module_id), (1, 4, 0));
    assert_eq!(remove_slot(&mut h.slots, 2), Err(HookError::SlotNotFound));
}

// ---------------------------------------------------------------- allow / deny / fail closed

#[test]
fn before_hook_denies_and_short_circuits() {
    let mut h = Harness::new();
    h.install(ModuleId::LaunchShield, shield(default_shield()));
    h.install(ModuleId::VolumeGuard, default_volume().encode());
    let (denied, trace) = h.go(ctx(Side::Buy, 3 * SOL, T0 + 10, 100)).unwrap();
    let d = denied.unwrap();
    assert_eq!((d.module_id, d.reason_code), (1, reason::launch_shield::MAX_BUY_EXCEEDED));
    assert_eq!(trace.len, 1, "volume guard never ran");
    assert_eq!(h.user.trade_count, 0);
}

#[test]
fn disabled_module_fails_closed_and_disabled_slot_is_skipped() {
    let mut h = Harness::new();
    h.install(ModuleId::VolumeGuard, default_volume().encode());
    h.disabled = ModuleId::VolumeGuard.kill_bit();
    assert_eq!(h.go(ctx(Side::Buy, SOL, T0, 1)).unwrap_err(), HookError::ModuleDisabled(2));

    h.disabled = 0;
    h.slots[0].flags &= !hook_types::slot_flags::ENABLED;
    let (denied, trace) = h.go(ctx(Side::Buy, 50 * SOL, T0, 1)).unwrap();
    assert!(denied.is_none());
    assert_eq!(trace.len, 0);
}

#[test]
fn capability_violation_is_an_error_not_a_denial() {
    let mut h = Harness::new();
    // Grant Launch Shield no veto capability: its denial must become a protocol error.
    install_slot(&mut h.slots, 1, 100, 0, C::READ_MARKET | C::WRITE_HOOK_STATE, shield(default_shield())).unwrap();
    assert_eq!(h.go(ctx(Side::Buy, 3 * SOL, T0, 1)).unwrap_err(), HookError::CapabilityViolation(1));

    // Holder XP without AWARD_POINTS cannot award XP.
    let mut h = Harness::new();
    let mut xp = default_xp();
    xp.vest_secs = 0;
    install_slot(&mut h.slots, 4, 2100, 0, C::READ_MARKET | C::WRITE_HOOK_STATE | C::WRITE_USER_STATE, xp.encode()).unwrap();
    assert_eq!(h.go(ctx(Side::Buy, SOL, T0, 1)).unwrap_err(), HookError::CapabilityViolation(4));
}

// ---------------------------------------------------------------- launch shield

#[test]
fn shield_cooldown_slot_cap_round_trip_and_expiry() {
    let mut h = Harness::new();
    h.install(ModuleId::LaunchShield, shield(default_shield()));

    assert!(h.go(ctx(Side::Buy, SOL, T0 + 1, 10)).unwrap().0.is_none());
    // Same wallet within 30s cooldown.
    let d = h.go(ctx(Side::Buy, SOL, T0 + 5, 11)).unwrap().0.unwrap();
    assert_eq!(d.reason_code, reason::launch_shield::COOLDOWN);
    // Sell in the same slot as the buy: round trip denied.
    let d = h.go(ctx(Side::Sell, SOL / 2, T0 + 1, 10)).unwrap().0.unwrap();
    assert_eq!(d.reason_code, reason::launch_shield::SAME_SLOT_ROUND_TRIP);

    // Per-slot buy cap across different wallets (fresh user state each).
    let mut h = Harness::new();
    h.install(ModuleId::LaunchShield, shield(default_shield()));
    for _ in 0..2 {
        h.user = UserState::default();
        assert!(h.go(ctx(Side::Buy, SOL, T0 + 1, 50)).unwrap().0.is_none());
    }
    h.user = UserState::default();
    assert_eq!(h.go(ctx(Side::Buy, SOL, T0 + 1, 50)).unwrap().0.unwrap().reason_code, reason::launch_shield::SLOT_BUY_CAP);
    h.user = UserState::default();
    assert!(h.go(ctx(Side::Buy, SOL, T0 + 1, 51)).unwrap().0.is_none(), "new slot resets the cap");

    // After the window the shield is inactive.
    h.user = UserState::default();
    let (denied, trace) = h.go(ctx(Side::Buy, 100 * SOL, T0 + 600, 60)).unwrap();
    assert!(denied.is_none());
    assert!(trace.entries[0].flags & hook_types::decision_flags::INACTIVE != 0);
}

// ---------------------------------------------------------------- volume guard

#[test]
fn volume_guard_per_trade_and_windows() {
    let mut h = Harness::new();
    h.install(ModuleId::VolumeGuard, default_volume().encode());
    assert_eq!(h.go(ctx(Side::Buy, 5 * SOL + 1, T0, 1)).unwrap().0.unwrap().reason_code, reason::volume_guard::MAX_BUY_EXCEEDED);
    assert_eq!(h.go(ctx(Side::Sell, 6 * SOL, T0, 1)).unwrap().0.unwrap().reason_code, reason::volume_guard::MAX_SELL_EXCEEDED);
    // Wallet window: 8 SOL per hour.
    assert!(h.go(ctx(Side::Buy, 5 * SOL, T0, 1)).unwrap().0.is_none());
    assert_eq!(h.go(ctx(Side::Buy, 4 * SOL, T0 + 10, 2)).unwrap().0.unwrap().reason_code, reason::volume_guard::WALLET_WINDOW_EXCEEDED);
    assert!(h.go(ctx(Side::Buy, 3 * SOL, T0 + 10, 2)).unwrap().0.is_none());
    // Market window: 20 SOL per hour across wallets.
    for i in 0..3 {
        h.user = UserState::default();
        assert!(h.go(ctx(Side::Buy, 4 * SOL, T0 + 20 + i, 3)).unwrap().0.is_none());
    }
    h.user = UserState::default();
    assert_eq!(h.go(ctx(Side::Buy, 1 * SOL, T0 + 30, 4)).unwrap().0.unwrap().reason_code, reason::volume_guard::MARKET_WINDOW_EXCEEDED);
    // Window rolls over.
    h.user = UserState::default();
    assert!(h.go(ctx(Side::Buy, 5 * SOL, T0 + 3_601, 5)).unwrap().0.is_none());
}

#[test]
fn volume_guard_after_check_reverts_on_observed_overshoot() {
    let mut h = Harness::new();
    h.install(ModuleId::VolumeGuard, default_volume().encode());
    let side = Side::Buy;
    let mut c = ctx(side, 4 * SOL, T0, 1);
    let mut t = Trace::default();
    let pol = &mut [None, None, None, None];
    assert!(run_phase(side, true, &mut h.slots, &mut c, &h.market, &mut h.user, pol, 0, &mut t).unwrap().denied.is_none());
    c.quote_amount = 6 * SOL; // Pump executed more than the estimate.
    assert_eq!(
        run_phase(side, false, &mut h.slots, &mut c, &h.market, &mut h.user, pol, 0, &mut t).unwrap_err(),
        HookError::PostCondition { module: 2, reason: reason::volume_guard::MAX_BUY_EXCEEDED }
    );
}

#[test]
fn volume_guard_config_validation() {
    assert!(volume_guard::VolumeGuard::validate_config(&[0; 48]).is_err());
    let mut c = default_volume();
    c.window_secs = 0;
    assert!(volume_guard::VolumeGuard::validate_config(&c.encode()).is_err());
    c.window_max_quote = 0;
    c.wallet_window_max_quote = 0;
    assert!(volume_guard::VolumeGuard::validate_config(&c.encode()).is_ok());
}

// ---------------------------------------------------------------- volatility guard

#[test]
fn volatility_guard_warms_up_then_denies_on_deviation() {
    let mut h = Harness::new();
    let cfg = vg::Config { max_deviation_bps: 1_000, ema_alpha_bps: 5_000, min_observations: 2, check_min_quote: 0, flags: vg::FLAG_CHECK_BUYS | vg::FLAG_CHECK_SELLS };
    h.install(ModuleId::VolatilityGuard, cfg.encode());
    // Two routed observations at a steady price.
    for i in 0..2 {
        h.user = UserState::default();
        let (d, t) = h.go(ctx(Side::Buy, SOL, T0 + i, 1 + i as u64)).unwrap();
        assert!(d.is_none());
        if i == 0 {
            assert!(t.entries[0].flags & hook_types::decision_flags::INACTIVE != 0);
        }
    }
    let s = vg::State::decode(&h.slots[0].state);
    assert_eq!(s.obs_count, 2);
    // Someone pumps the pool directly: spot +50%.
    let mut c = ctx(Side::Buy, SOL, T0 + 10, 10);
    c.pre_quote_reserves = 150 * SOL;
    let dev = volatility_bps(&h.slots, &c).unwrap();
    assert!((4_999..=5_000).contains(&dev), "Q64 floor rounding: {dev}");
    assert_eq!(h.go(c).unwrap().0.unwrap().reason_code, reason::volatility_guard::BUY_DEVIATION);
    // Within threshold.
    let mut c = ctx(Side::Sell, SOL, T0 + 10, 10);
    c.pre_quote_reserves = 105 * SOL;
    assert!(h.go(c).unwrap().0.is_none());
}

#[test]
fn volatility_math_extremes() {
    assert_eq!(vg::deviation_bps(0, 0), 0);
    assert_eq!(vg::deviation_bps(u128::MAX, 1), u64::MAX);
    assert_eq!(vg::deviation_bps(110, 100), 1_000);
    assert_eq!(vg::deviation_bps(90, 100), 1_000);
}

// ---------------------------------------------------------------- holder xp

#[test]
fn xp_vests_and_is_forfeited_by_early_sell() {
    let mut h = Harness::new();
    h.install(ModuleId::HolderXp, default_xp().encode());
    let mut c = ctx(Side::Buy, 2 * SOL, T0, 1);
    c.user_base_balance = 0;
    h.go(c).unwrap();
    assert_eq!((h.user.xp, h.user.pending_xp), (0, 200));

    // Sell 10 minutes later: unvested XP forfeited.
    let mut c = ctx(Side::Sell, SOL, T0 + 600, 2);
    c.user_base_balance = 2 * SOL * 1_000;
    h.go(c).unwrap();
    assert_eq!((h.user.xp, h.user.pending_xp, h.user.forfeited_xp), (0, 0, 200));

    // Buy and hold past vesting: XP settles on the next routed interaction.
    let mut c = ctx(Side::Buy, SOL, T0 + 1_000, 3);
    c.user_base_balance = SOL * 1_000;
    h.go(c).unwrap();
    assert_eq!(h.user.pending_xp, 100);
    let mut c = ctx(Side::Sell, SOL / 10, T0 + 1_000 + 3_600, 4);
    c.user_base_balance = 2 * SOL * 1_000;
    h.go(c).unwrap();
    assert_eq!(h.user.xp, 100);
}

#[test]
fn xp_wash_loop_earns_nothing() {
    let mut h = Harness::new();
    h.install(ModuleId::HolderXp, default_xp().encode());
    let mut bal = 0;
    for i in 0..20 {
        let mut b = ctx(Side::Buy, 5 * SOL, T0 + i * 120, 10 + i as u64 * 2);
        b.user_base_balance = bal;
        h.go(b).unwrap();
        bal += b.base_amount;
        let mut s = ctx(Side::Sell, 5 * SOL, T0 + i * 120 + 60, 11 + i as u64 * 2);
        s.user_base_balance = bal;
        h.go(s).unwrap();
        bal -= s.base_amount;
    }
    assert_eq!(h.user.xp, 0);
    assert!(h.user.forfeited_xp > 0);
}

#[test]
fn xp_daily_and_per_trade_caps() {
    let mut h = Harness::new();
    let mut cfg = default_xp();
    cfg.vest_secs = 0;
    cfg.daily_cap = 250;
    cfg.per_trade_cap_quote = 2 * SOL;
    h.install(ModuleId::HolderXp, cfg.encode());
    h.go(ctx(Side::Buy, 100 * SOL, T0, 1)).unwrap();
    assert_eq!(h.user.xp, 200, "per-trade cap");
    h.go(ctx(Side::Buy, 100 * SOL, T0 + 60, 2)).unwrap();
    assert_eq!(h.user.xp, 250, "daily cap");
    h.go(ctx(Side::Buy, 100 * SOL, T0 + 86_400, 3)).unwrap();
    assert_eq!(h.user.xp, 450, "cap resets next day");
    // Dust buys earn nothing.
    let before = h.user.xp;
    h.go(ctx(Side::Buy, SOL / 1_000, T0 + 86_500, 4)).unwrap();
    assert_eq!(h.user.xp, before);
}

#[test]
fn holding_xp_uses_min_balance() {
    let mut h = Harness::new();
    let mut cfg = default_xp();
    cfg.xp_per_sol = 1;
    cfg.hold_xp_per_day = 100;
    cfg.hold_cap_base = 1_000_000;
    h.install(ModuleId::HolderXp, cfg.encode());
    let mut c = ctx(Side::Buy, SOL, T0, 1);
    c.base_amount = 1_000_000;
    c.user_base_balance = 0;
    h.go(c).unwrap(); // checkpoint 1_000_000
    // One day later the wallet holds only half (moved tokens out directly).
    let mut s = ctx(Side::Sell, SOL / 100, T0 + 86_400, 2);
    s.base_amount = 0;
    s.user_base_balance = 500_000;
    h.go(s).unwrap();
    // 1 vested trade XP from the buy + 50 holding XP for min(1_000_000, 500_000) over one day.
    assert_eq!(h.user.xp, 51);
}

// ---------------------------------------------------------------- sentient

fn sentient_cfg() -> sentient::Config {
    sentient::Config { signer: [9; 32], defensive_max_quote: SOL, max_signal_ttl: 3_600, flags: sentient::FLAG_APPLY_BUYS | sentient::FLAG_APPLY_SELLS }
}

fn signal(nonce: u64, regime: Regime, issued: i64, until: i64) -> StrategySignal {
    StrategySignal {
        version: 1,
        action: SignalAction::ChangeStrategyState as u8,
        param: regime as u8,
        _pad: 0,
        confidence_bps: 9_000,
        amount_bps: 0,
        market: [3; 32],
        nonce,
        issued_at: issued,
        valid_until: until,
        _reserved: [0; 32],
    }
}

#[test]
fn sentient_regimes_restrict_routed_flow_and_expire() {
    let mut h = Harness::new();
    let idx = h.install(ModuleId::Sentient, sentient_cfg().encode());
    assert!(h.go(ctx(Side::Buy, 5 * SOL, T0, 1)).unwrap().0.is_none());

    let cfg = h.slots[idx].config;
    sentient::apply_signal([3; 32], &cfg, &mut h.slots[idx].state, &signal(1, Regime::Defensive, T0, T0 + 600), T0).unwrap();
    assert_eq!(h.go(ctx(Side::Buy, 2 * SOL, T0 + 1, 2)).unwrap().0.unwrap().reason_code, reason::sentient::DEFENSIVE_SIZE_CAP);
    assert!(h.go(ctx(Side::Buy, SOL, T0 + 1, 2)).unwrap().0.is_none());

    sentient::apply_signal([3; 32], &cfg, &mut h.slots[idx].state, &signal(2, Regime::CircuitBreaker, T0 + 2, T0 + 60), T0 + 2).unwrap();
    assert_eq!(h.go(ctx(Side::Sell, 1, T0 + 3, 3)).unwrap().0.unwrap().reason_code, reason::sentient::CIRCUIT_BREAKER);
    // Expired: back to normal.
    assert!(h.go(ctx(Side::Sell, 5 * SOL, T0 + 61, 4)).unwrap().0.is_none());
    let s = sentient::State::decode(&h.slots[idx].state);
    assert_eq!(s.sell_flow, 5 * SOL);
}

#[test]
fn sentient_signal_replay_expiry_future_and_ttl() {
    use hook_types::signal::SignalRejection as R;
    let cfg = sentient_cfg().encode();
    let mut state = [0u8; 48];
    let m = [3; 32];
    sentient::apply_signal(m, &cfg, &mut state, &signal(5, Regime::Cautious, T0, T0 + 10), T0).unwrap();
    assert_eq!(sentient::apply_signal(m, &cfg, &mut state, &signal(5, Regime::Normal, T0, T0 + 10), T0), Err(R::Replayed));
    assert_eq!(sentient::apply_signal(m, &cfg, &mut state, &signal(4, Regime::Normal, T0, T0 + 10), T0), Err(R::Replayed));
    assert_eq!(sentient::apply_signal(m, &cfg, &mut state, &signal(6, Regime::Normal, T0, T0 + 10), T0 + 11), Err(R::Expired));
    assert_eq!(sentient::apply_signal(m, &cfg, &mut state, &signal(6, Regime::Normal, T0 + 100, T0 + 200), T0), Err(R::FromFuture));
    assert_eq!(sentient::apply_signal(m, &cfg, &mut state, &signal(6, Regime::Normal, T0, T0 + 3_601), T0), Err(R::ValidityTooLong));
    assert_eq!(sentient::apply_signal([4; 32], &cfg, &mut state, &signal(6, Regime::Normal, T0, T0 + 10), T0), Err(R::WrongMarket));
    let mut bad = signal(6, Regime::Normal, T0, T0 + 10);
    bad.action = SignalAction::QueueBuyback as u8;
    assert_eq!(sentient::apply_signal(m, &cfg, &mut state, &bad, T0), Err(R::ActionNotPermitted));
    // Rejections never advance the nonce.
    assert_eq!(sentient::State::decode(&state).last_nonce, 5);
}

// ---------------------------------------------------------------- policies in the stack

fn policy(spec: &PolicySpec, idx: u8) -> PolicyData {
    spec.validate(&ValidationEnv { has_volatility_guard: false }).unwrap();
    let mut p = PolicyData::zeroed();
    p.index = idx;
    p.enabled = 1;
    spec.write_to(&mut p);
    p
}

#[test]
fn policies_run_inside_stack_and_can_deny_and_award() {
    let mut h = Harness::new();
    let mut cfg = [0u8; 48];
    cfg[0] = 0b011;
    h.install(ModuleId::Policy, cfg);

    let mut deny = PolicySpec { trigger: trigger::BEFORE_SELL, deny_reason: 7, ..Default::default() };
    deny.conditions[0] = Condition { field: field::TRADE_SIZE, op: op::GT, value: 3 * SOL, ..Default::default() };
    deny.n_conditions = 1;
    deny.actions[0] = Action { kind: action::DENY, ..Default::default() };
    deny.n_actions = 1;
    let mut award = PolicySpec { trigger: trigger::AFTER_BUY, ..Default::default() };
    award.actions[0] = Action { kind: action::AWARD_POINTS, value: 25, ..Default::default() };
    award.n_actions = 1;

    let mut p0 = policy(&deny, 0);
    let mut p1 = policy(&award, 1);
    {
        let mut pol = [Some(&mut p0), Some(&mut p1), None, None];
        let (d, _) = h.trade(ctx(Side::Sell, 4 * SOL, T0, 1), &mut pol).unwrap();
        assert_eq!(d.unwrap().reason_code, 9007);
        let (d, _) = h.trade(ctx(Side::Buy, 4 * SOL, T0, 1), &mut pol).unwrap();
        assert!(d.is_none());
    }
    assert_eq!(h.user.policy_points, 25);
    assert_eq!(p1.exec_count, 1);

    // Missing policy account fails closed.
    let mut pol = [Some(&mut p0), None, None, None];
    assert_eq!(h.trade(ctx(Side::Buy, SOL, T0, 2), &mut pol).unwrap_err(), HookError::PolicyAccountMissing(1));
}

#[test]
fn policy_slot_config_validation() {
    let mut s = [HookSlot::zeroed(); MAX_HOOK_SLOTS];
    let mut cfg = [0u8; 48];
    cfg[0] = 0b1_0000;
    assert_eq!(install_slot(&mut s, 9, 4000, 0, caps(ModuleId::Policy), cfg), Err(HookError::BadConfig(9)));
}

// ---------------------------------------------------------------- the ultimate demo stack

#[test]
fn demo_stack_volume_guard_holder_xp_sentient_policy() {
    let mut h = Harness::new();
    h.install(ModuleId::VolumeGuard, default_volume().encode());
    h.install(ModuleId::HolderXp, default_xp().encode());
    h.install(ModuleId::Sentient, sentient_cfg().encode());
    let t = hook_runtime_twap();
    install_slot(&mut h.slots, 6, prio(ModuleId::Twap), 0, caps(ModuleId::Twap), t).unwrap();
    let (d, trace) = h.go(ctx(Side::Buy, SOL, T0, 1)).unwrap();
    assert!(d.is_none());
    let mods: [u16; 6] = core::array::from_fn(|i| trace.entries[i].module_id);
    // before: VolumeGuard(BeforeSwap), Sentient(BeforeSwap); after: XP(AfterBuy), VG(AfterSwap), Sentient(AfterSwap)
    assert_eq!(&mods[..5], &[2, 8, 4, 2, 8]);
    assert_eq!(trace.len, 5);
}

fn hook_runtime_twap() -> [u8; 48] {
    crate::modules::twap::Config { max_total_quote: 10 * SOL, max_slice_quote: SOL, min_interval_secs: 60, max_duration_secs: 86_400, max_slippage_bps: 300 }.encode()
}
