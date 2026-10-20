use super::*;
use bytemuck::Zeroable;

const SOL: u64 = 1_000_000_000;

fn ctx(cb: HookCallback, quote: u64, now: i64) -> HookContext {
    let mut c = HookContext::zeroed();
    c.version = 1;
    c.callback = cb as u8;
    c.quote_amount = quote;
    c.timestamp = now;
    c.market_activated_at = 1_000;
    c.sequence = 7;
    c
}

fn spec(trigger_: u8) -> PolicySpec {
    PolicySpec { trigger: trigger_, ..Default::default() }
}

fn cond(f: u8, o: u8, v: u64) -> Condition {
    Condition { field: f, op: o, value: v, ..Default::default() }
}

fn act(kind: u8, value: u64) -> Action {
    Action { kind, value, ..Default::default() }
}

fn install(s: &PolicySpec) -> PolicyData {
    s.validate(&ValidationEnv { has_volatility_guard: true }).unwrap();
    let mut p = PolicyData::zeroed();
    p.enabled = 1;
    s.write_to(&mut p);
    p
}

fn run(p: &mut PolicyData, cb: HookCallback, quote: u64, now: i64) -> PolicyOutcome {
    let c = ctx(cb, quote, now);
    let m = MarketStats::default();
    let u = UserState::default();
    evaluate(p, cb, &EvalInput { ctx: &c, market: &m, user: &u, volatility_bps: None })
}

/// "WHEN Before Buy IF Trade Size > 5 SOL THEN Reject Routed Trade"
fn whale_rule() -> PolicySpec {
    let mut s = spec(trigger::BEFORE_BUY);
    s.conditions[0] = cond(field::TRADE_SIZE, op::GT, 5 * SOL);
    s.n_conditions = 1;
    s.actions[0] = act(action::DENY, 0);
    s.n_actions = 1;
    s.deny_reason = 1;
    s
}

#[test]
fn brief_example_denies_above_threshold_only() {
    let mut p = install(&whale_rule());
    let out = run(&mut p, HookCallback::BeforeBuy, 5 * SOL + 1, 2_000);
    assert_eq!(out.deny, Some(9001));
    assert!(!out.decision().is_allowed());
    let out = run(&mut p, HookCallback::BeforeBuy, 5 * SOL, 2_000);
    assert_eq!(out.deny, None);
    assert!(out.matched == false && out.evaluated);
}

#[test]
fn trigger_mismatch_is_skipped() {
    let mut p = install(&whale_rule());
    let out = run(&mut p, HookCallback::BeforeSell, 100 * SOL, 2_000);
    assert!(!out.evaluated);
    assert_eq!(out.deny, None);
}

#[test]
fn disabled_policy_is_skipped() {
    let mut p = install(&whale_rule());
    p.enabled = 0;
    assert!(!run(&mut p, HookCallback::BeforeBuy, 100 * SOL, 2_000).evaluated);
}

#[test]
fn every_operator() {
    let cases = [
        (op::EQ, 5, 5, true),
        (op::EQ, 5, 6, false),
        (op::NEQ, 5, 6, true),
        (op::GT, 6, 5, true),
        (op::GT, 5, 5, false),
        (op::GTE, 5, 5, true),
        (op::LT, 4, 5, true),
        (op::LT, 5, 5, false),
        (op::LTE, 5, 5, true),
        (op::LTE, 6, 5, false),
    ];
    for (o, lhs, rhs, want) in cases {
        assert_eq!(compare(lhs, o, rhs), want, "op {o} {lhs} {rhs}");
    }
    assert!(!compare(1, 99, 1));
}

#[test]
fn multiple_conditions_all_and_any() {
    let mut s = spec(trigger::BEFORE_SWAP);
    s.conditions[0] = cond(field::TRADE_SIZE, op::GT, 1 * SOL);
    s.conditions[1] = cond(field::TIME_SINCE_LAUNCH, op::LT, 600);
    s.n_conditions = 2;
    s.actions[0] = act(action::DENY, 0);
    s.n_actions = 1;

    let mut p = install(&s);
    // activated at 1000; now 1500 => 500s since launch
    assert!(run(&mut p, HookCallback::BeforeSwap, 2 * SOL, 1_500).deny.is_some());
    assert!(run(&mut p, HookCallback::BeforeSwap, 2 * SOL, 1_700).deny.is_none());
    assert!(run(&mut p, HookCallback::BeforeSwap, SOL / 2, 1_500).deny.is_none());

    s.combinator = combinator::ANY;
    let mut p = install(&s);
    assert!(run(&mut p, HookCallback::BeforeSwap, SOL / 2, 1_500).deny.is_some());
    assert!(run(&mut p, HookCallback::BeforeSwap, 2 * SOL, 1_700).deny.is_some());
    assert!(run(&mut p, HookCallback::BeforeSwap, SOL / 2, 1_700).deny.is_none());
}

#[test]
fn multiple_actions_and_state() {
    let mut s = spec(trigger::AFTER_BUY);
    s.conditions[0] = cond(field::TRADE_SIZE, op::GTE, SOL);
    s.n_conditions = 1;
    s.actions[0] = act(action::AWARD_POINTS, 50);
    s.actions[1] = Action { kind: action::UPDATE_STATE, arg: 2, arg16: state_op::ADD, value: 1, ..Default::default() };
    s.actions[2] = act(action::RECORD, 77);
    s.n_actions = 3;
    let mut p = install(&s);

    let out = run(&mut p, HookCallback::AfterBuy, 2 * SOL, 2_000);
    assert_eq!(out.points, 50);
    assert_eq!(out.record_tag, Some(77));
    assert_eq!(p.registers[2], 1);
    assert_eq!(p.exec_count, 1);
    assert_eq!(p.last_exec_ts, 2_000);
    run(&mut p, HookCallback::AfterBuy, 2 * SOL, 2_001);
    assert_eq!(p.registers[2], 2);
}

#[test]
fn hook_state_condition_reads_registers() {
    // Deny buys after the register counter reaches 3 (e.g. "max 3 routed large buys").
    let mut s = spec(trigger::BEFORE_BUY);
    s.conditions[0] = Condition { field: field::HOOK_STATE, op: op::GTE, arg: 0, value: 3, ..Default::default() };
    s.n_conditions = 1;
    s.actions[0] = act(action::DENY, 0);
    s.n_actions = 1;
    let mut p = install(&s);
    assert!(run(&mut p, HookCallback::BeforeBuy, 1, 2_000).deny.is_none());
    p.registers[0] = 3;
    assert!(run(&mut p, HookCallback::BeforeBuy, 1, 2_000).deny.is_some());
}

#[test]
fn allow_stops_further_actions() {
    let mut s = spec(trigger::AFTER_SWAP);
    s.actions[0] = act(action::ALLOW, 0);
    s.actions[1] = act(action::AWARD_POINTS, 10);
    s.n_actions = 2;
    let mut p = install(&s);
    let out = run(&mut p, HookCallback::AfterSwap, 1, 2_000);
    assert!(out.matched);
    assert_eq!(out.points, 0);
    assert_eq!(p.exec_count, 0);
}

#[test]
fn cooldown_skips_effects_but_not_deny() {
    let mut s = spec(trigger::AFTER_BUY);
    s.cooldown_secs = 60;
    s.actions[0] = act(action::AWARD_POINTS, 10);
    s.n_actions = 1;
    let mut p = install(&s);
    assert_eq!(run(&mut p, HookCallback::AfterBuy, 1, 2_000).points, 10);
    let out = run(&mut p, HookCallback::AfterBuy, 1, 2_030);
    assert!(out.in_cooldown);
    assert_eq!(out.points, 0);
    assert_eq!(run(&mut p, HookCallback::AfterBuy, 1, 2_061).points, 10);

    let mut s = whale_rule();
    s.cooldown_secs = 1_000;
    let mut p = install(&s);
    p.last_exec_ts = 1_999;
    assert!(run(&mut p, HookCallback::BeforeBuy, 6 * SOL, 2_000).deny.is_some());
}

#[test]
fn intent_actions_only_record_intents() {
    let mut s = spec(trigger::AFTER_SELL);
    s.actions[0] = act(action::QUEUE_TWAP, 123);
    s.n_actions = 1;
    let mut p = install(&s);
    let out = run(&mut p, HookCallback::AfterSell, 1, 2_000);
    assert!(out.intent);
    assert_eq!(p.intent.kind, action::QUEUE_TWAP);
    assert_eq!(p.intent.value, 123);
    assert_eq!(p.intent.sequence, 7);
}

#[test]
fn missing_volatility_data_never_fires() {
    let mut s = spec(trigger::BEFORE_BUY);
    s.conditions[0] = cond(field::VOLATILITY, op::GT, 100);
    s.n_conditions = 1;
    s.actions[0] = act(action::DENY, 0);
    s.n_actions = 1;
    let mut p = install(&s);
    assert!(run(&mut p, HookCallback::BeforeBuy, 1, 2_000).deny.is_none());

    let c = ctx(HookCallback::BeforeBuy, 1, 2_000);
    let m = MarketStats::default();
    let u = UserState::default();
    let out = evaluate(&mut p, HookCallback::BeforeBuy, &EvalInput { ctx: &c, market: &m, user: &u, volatility_bps: Some(101) });
    assert!(out.deny.is_some());
}

#[test]
fn time_since_last_action_starts_infinite() {
    let mut s = spec(trigger::AFTER_BUY);
    s.conditions[0] = cond(field::TIME_SINCE_LAST_ACTION, op::GTE, 3_600);
    s.n_conditions = 1;
    s.actions[0] = act(action::EMIT_SIGNAL, 0);
    s.n_actions = 1;
    let mut p = install(&s);
    assert!(run(&mut p, HookCallback::AfterBuy, 1, 2_000).signal);
    assert!(!run(&mut p, HookCallback::AfterBuy, 1, 3_000).signal);
    assert!(run(&mut p, HookCallback::AfterBuy, 1, 5_600).signal);
}

#[test]
fn u64_boundaries() {
    let mut s = spec(trigger::BEFORE_BUY);
    s.conditions[0] = cond(field::TRADE_SIZE, op::GTE, u64::MAX);
    s.n_conditions = 1;
    s.actions[0] = act(action::DENY, 0);
    s.n_actions = 1;
    let mut p = install(&s);
    assert!(run(&mut p, HookCallback::BeforeBuy, u64::MAX, 2_000).deny.is_some());
    assert!(run(&mut p, HookCallback::BeforeBuy, u64::MAX - 1, 2_000).deny.is_none());

    let mut s = spec(trigger::AFTER_BUY);
    s.actions[0] = Action { kind: action::UPDATE_STATE, arg: 0, arg16: state_op::ADD, value: u64::MAX, ..Default::default() };
    s.actions[1] = Action { kind: action::UPDATE_STATE, arg: 1, arg16: state_op::SUB, value: 1, ..Default::default() };
    s.n_actions = 2;
    let mut p = install(&s);
    run(&mut p, HookCallback::AfterBuy, 0, 2_000);
    run(&mut p, HookCallback::AfterBuy, 0, 2_001);
    assert_eq!(p.registers[0], u64::MAX, "ADD saturates (no overflow)");
    assert_eq!(p.registers[1], 0, "SUB saturates (no underflow)");
}

// ---- validation ----

fn v(s: &PolicySpec) -> Result<(), PolicyError> {
    s.validate(&ValidationEnv::default())
}

#[test]
fn rejects_complexity_beyond_limits() {
    let mut s = whale_rule();
    s.n_conditions = 9;
    assert_eq!(v(&s), Err(PolicyError::TooManyConditions));
    let mut s = whale_rule();
    s.n_actions = 5;
    assert_eq!(v(&s), Err(PolicyError::TooManyActions));
    let mut s = whale_rule();
    s.n_actions = 0;
    assert_eq!(v(&s), Err(PolicyError::NoActions));
}

#[test]
fn rejects_invalid_operator_field_action() {
    let mut s = whale_rule();
    s.conditions[0].op = 6;
    assert_eq!(v(&s), Err(PolicyError::UnknownOperator));
    let mut s = whale_rule();
    s.conditions[0].field = 0;
    assert_eq!(v(&s), Err(PolicyError::UnknownField));
    let mut s = whale_rule();
    s.conditions[0].field = field::MAX + 1;
    assert_eq!(v(&s), Err(PolicyError::UnknownField));
    let mut s = whale_rule();
    s.actions[0].kind = action::MAX + 1;
    assert_eq!(v(&s), Err(PolicyError::UnknownAction));
    let mut s = whale_rule();
    s.actions[0].kind = action::NONE;
    assert_eq!(v(&s), Err(PolicyError::UnknownAction));
}

#[test]
fn rejects_phase_violations() {
    let mut s = whale_rule();
    s.trigger = trigger::AFTER_BUY;
    assert_eq!(v(&s), Err(PolicyError::DenyNotAllowedForTrigger));
    let mut s = spec(trigger::BEFORE_BUY);
    s.actions[0] = act(action::AWARD_POINTS, 1);
    s.n_actions = 1;
    assert_eq!(v(&s), Err(PolicyError::PointsNotAllowedForTrigger));
    let mut s = whale_rule();
    s.actions[1] = act(action::DENY, 0);
    s.n_actions = 2;
    assert_eq!(v(&s), Err(PolicyError::MultipleDeny));
}

#[test]
fn rejects_reserved_triggers_and_unavailable_fields() {
    let mut s = whale_rule();
    s.trigger = trigger::TIME_INTERVAL;
    assert_eq!(v(&s), Err(PolicyError::UnsupportedTrigger));
    let mut s = whale_rule();
    s.conditions[0].field = field::TREASURY_BALANCE;
    assert_eq!(v(&s), Err(PolicyError::FieldNotAvailable));
    let mut s = whale_rule();
    s.conditions[0].field = field::VOLATILITY;
    assert_eq!(v(&s), Err(PolicyError::RequiresVolatilityGuard));
}

#[test]
fn rejects_out_of_range_params() {
    let mut s = whale_rule();
    s.deny_reason = 1_000;
    assert_eq!(v(&s), Err(PolicyError::DenyReasonTooLarge));
    let mut s = spec(trigger::AFTER_BUY);
    s.actions[0] = act(action::AWARD_POINTS, MAX_POINTS_PER_ACTION + 1);
    s.n_actions = 1;
    assert_eq!(v(&s), Err(PolicyError::PointsTooLarge));
    let mut s = spec(trigger::AFTER_BUY);
    s.actions[0] = Action { kind: action::UPDATE_STATE, arg: 4, ..Default::default() };
    s.n_actions = 1;
    assert_eq!(v(&s), Err(PolicyError::BadRegister));
    let mut s = spec(trigger::AFTER_BUY);
    s.actions[0] = Action { kind: action::UPDATE_STATE, arg16: 3, ..Default::default() };
    s.n_actions = 1;
    assert_eq!(v(&s), Err(PolicyError::BadStateOp));
    let mut s = whale_rule();
    s.combinator = 2;
    assert_eq!(v(&s), Err(PolicyError::BadCombinator));
}

#[test]
fn rejects_garbage_in_unused_entries() {
    let mut s = whale_rule();
    s.conditions[5].value = 1;
    assert_eq!(v(&s), Err(PolicyError::UnusedEntryNotZero));
    let mut s = whale_rule();
    s.actions[3].kind = action::RECORD;
    assert_eq!(v(&s), Err(PolicyError::UnusedEntryNotZero));
}

#[test]
fn max_size_policy_validates_and_runs() {
    let mut s = spec(trigger::BEFORE_SWAP);
    for i in 0..MAX_CONDITIONS {
        s.conditions[i] = cond(field::TRADE_SIZE, op::GTE, i as u64);
    }
    s.n_conditions = MAX_CONDITIONS as u8;
    s.actions[0] = act(action::RECORD, 1);
    s.actions[1] = act(action::EMIT_SIGNAL, 0);
    s.actions[2] = Action { kind: action::UPDATE_STATE, arg: 3, arg16: state_op::SET, value: 9, ..Default::default() };
    s.actions[3] = act(action::DENY, 0);
    s.n_actions = MAX_ACTIONS as u8;
    let mut p = install(&s);
    let out = run(&mut p, HookCallback::BeforeSwap, 100, 2_000);
    assert!(out.deny.is_some());
    assert_eq!(p.registers[3], 9);
}
