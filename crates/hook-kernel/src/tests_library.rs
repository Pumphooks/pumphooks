//! Hook library builtins (ids 9–14).

use super::builtin::*;
use super::*;
use bytemuck::Zeroable;
use hook_standard::{output, Environment, HookStack, StackEntry, IMPL_BUILTIN};
use proptest::prelude::*;

fn entry<T: bytemuck::Pod>(id: u8, callbacks: u16, cfg: T) -> StackEntry {
    let mut e = StackEntry::zeroed();
    e.impl_kind = IMPL_BUILTIN;
    e.builtin_id = id;
    e.callbacks = callbacks;
    e.granted = required_caps(id);
    e.config.copy_from_slice(bytemuck::bytes_of(&cfg));
    e
}

fn stack(env: Environment, entries: &[StackEntry]) -> HookStack {
    let mut s = HookStack::empty(env);
    s.len = entries.len() as u8;
    s.entries[..entries.len()].copy_from_slice(entries);
    s
}

const POOL_BEFORE_SWAP: u8 = 6;
const POOL_AFTER_SWAP: u8 = 7;
const POOL_BEFORE_ADD: u8 = 2;
const POOL_BEFORE_REMOVE: u8 = 4;
const ROUTER_BEFORE_SWAP: u8 = 2;
const ROUTER_AFTER_SWAP: u8 = 3;
const TOKEN_BEFORE_TRANSFER: u8 = 0;

fn run(s: &mut HookStack, cb: u8, op: &Op) -> Result<Outputs, Rejection> {
    crate::evaluate(s, cb, op, &mut NoProgramHooks)
}

fn denylist(target: u8, a: Pubkey) -> DenylistConfig {
    DenylistConfig { target, _pad: [0; 7], key_a: a.to_bytes(), key_b: [0; 32], _rest: [0; 24] }
}

#[test]
fn denylist_rejects_listed_keys_in_every_environment() {
    let bad = Pubkey::new_unique();
    let good = Pubkey::new_unique();
    for (env, cb) in [
        (Environment::Token, TOKEN_BEFORE_TRANSFER),
        (Environment::Router, ROUTER_BEFORE_SWAP),
        (Environment::Pool, POOL_BEFORE_SWAP),
        (Environment::Account, 2),
    ] {
        let mut s = stack(env, &[entry(DENYLIST, 1 << cb, denylist(TARGET_ACTOR, bad))]);
        assert_eq!(validate_stack(&s, 0), Ok(()), "{env:?}");
        let r = run(&mut s, cb, &Op { actor: bad, ..Op::default() }).unwrap_err();
        assert_eq!(r.reason, reason::DENYLISTED);
        assert!(run(&mut s, cb, &Op { actor: good, ..Op::default() }).is_ok());
    }
    // Counterparty target, e.g. a token transfer's destination owner.
    let mut s = stack(Environment::Token, &[entry(DENYLIST, 1, denylist(TARGET_COUNTERPARTY, bad))]);
    assert!(run(&mut s, 0, &Op { actor: bad, counterparty: good, ..Op::default() }).is_ok());
    assert!(run(&mut s, 0, &Op { actor: good, counterparty: bad, ..Op::default() }).is_err());
    // Empty list is a config error, not an open gate.
    let empty = DenylistConfig { target: 0, _pad: [0; 7], key_a: [0; 32], key_b: [0; 32], _rest: [0; 24] };
    assert!(validate_stack(&stack(Environment::Token, &[entry(DENYLIST, 1, empty)]), 0).is_err());
}

fn size_fee(base: u16, max: u16, pivot: u64) -> DynamicFeeConfig {
    DynamicFeeConfig { mode: FEE_MODE_SIZE, _pad: [0; 3], base_bps: base, max_bps: max, pivot, alpha_bps: 0, _pad2: 0, sensitivity_bps: 0, _rest: [0; 72] }
}

fn fee_of(o: &Outputs) -> u64 {
    o.iter().flatten().find(|x| x.kind == output::FEE_BPS).unwrap().value
}

#[test]
fn dynamic_fee_size_mode_ramps_and_saturates() {
    let mut s = stack(Environment::Pool, &[entry(DYNAMIC_FEE, 1 << POOL_BEFORE_SWAP, size_fee(30, 100, 1_000))]);
    assert_eq!(validate_stack(&s, 0), Ok(()));
    let fee = |s: &mut HookStack, amount| fee_of(&run(s, POOL_BEFORE_SWAP, &Op { amount, ..Op::default() }).unwrap());
    assert_eq!(fee(&mut s, 0), 30);
    assert_eq!(fee(&mut s, 500), 65);
    assert_eq!(fee(&mut s, 1_000), 100);
    assert_eq!(fee(&mut s, u64::MAX), 100);
    // Only pools can apply a fee; the builtin is refused elsewhere and on other callbacks.
    assert!(validate_stack(&stack(Environment::Router, &[entry(DYNAMIC_FEE, 1 << 2, size_fee(30, 100, 1))]), 0).is_err());
    assert!(validate_stack(&stack(Environment::Pool, &[entry(DYNAMIC_FEE, 1 << POOL_AFTER_SWAP, size_fee(30, 100, 1))]), 0).is_err());
    assert!(validate_stack(&stack(Environment::Pool, &[entry(DYNAMIC_FEE, 1 << 6, size_fee(101, 100, 1))]), 0).is_err());
}

#[test]
fn dynamic_fee_volatility_mode_tracks_an_ema() {
    let cfg = DynamicFeeConfig { mode: FEE_MODE_VOLATILITY, _pad: [0; 3], base_bps: 25, max_bps: 300, pivot: 0, alpha_bps: 5_000, _pad2: 0, sensitivity_bps: 10_000, _rest: [0; 72] };
    let mut s = stack(Environment::Pool, &[entry(DYNAMIC_FEE, 1 << POOL_BEFORE_SWAP, cfg)]);
    assert_eq!(validate_stack(&s, 0), Ok(()));
    let mut fee = |r: [u64; 2]| fee_of(&run(&mut s, POOL_BEFORE_SWAP, &Op { reserves: r, ..Op::default() }).unwrap());
    // First observation seeds the EMA at the base fee.
    assert_eq!(fee([1_000_000, 1_000_000]), 25);
    assert_eq!(fee([1_000_000, 1_000_000]), 25);
    // Price +1% vs EMA: ~100 bps deviation (99 after Q64.64 flooring) · 1.0 sensitivity.
    assert_eq!(fee([1_000_000, 1_010_000]), 124);
    // EMA moved halfway (alpha 50%); a return to 1.0 is ~0.5% away.
    let back = fee([1_000_000, 1_000_000]);
    assert!((70..=80).contains(&back), "{back}");
    // A 50% shock saturates at the max.
    assert_eq!(fee([1_000_000, 1_500_000]), 300);
    // Without reserves (no pool price) the base fee applies.
    assert_eq!(fee([0, 0]), 25);
}

#[test]
fn price_impact_uses_live_reserves_and_fails_closed() {
    let cfg = PriceImpactConfig { max_bps: 100, _pad: [0; 6], _rest: [0; 88] };
    let mut s = stack(Environment::Pool, &[entry(PRICE_IMPACT, 1 << POOL_BEFORE_SWAP, cfg)]);
    assert_eq!(validate_stack(&s, 0), Ok(()));
    let op = |amount, zero_for_one| Op { amount, reserves: [1_000_000, 50_000], zero_for_one, ..Op::default() };
    // 1% of input reserve after the trade: 10_101 / 1_010_101 → 99 bps.
    let o = run(&mut s, POOL_BEFORE_SWAP, &op(10_101, true)).unwrap();
    assert_eq!(o[0].unwrap().kind, output::PRICE_IMPACT_BPS);
    assert_eq!(o[0].unwrap().value, 99);
    // 10_200 is exactly 100 bps (allowed); 10_300 is 101.
    assert!(run(&mut s, POOL_BEFORE_SWAP, &op(10_200, true)).is_ok());
    assert_eq!(run(&mut s, POOL_BEFORE_SWAP, &op(10_300, true)).unwrap_err().reason, reason::PRICE_IMPACT);
    // Direction picks the input reserve: 600 into a 50_000 reserve is 118 bps.
    assert!(run(&mut s, POOL_BEFORE_SWAP, &op(600, false)).is_err());
    assert!(run(&mut s, POOL_BEFORE_SWAP, &op(400, false)).is_ok());
    // No reserves → reject (never silently allow).
    assert!(run(&mut s, POOL_BEFORE_SWAP, &Op { amount: 1, ..Op::default() }).is_err());
}

fn limit_cfg(mode: u8, min: (u64, u64), max: (u64, u64)) -> PriceLimitConfig {
    PriceLimitConfig { min_num: min.0, min_den: min.1, max_num: max.0, max_den: max.1, asset: [0; 32], mode, _pad: [0; 7], _rest: [0; 24] }
}

#[test]
fn price_limit_bounds_realized_and_pool_price() {
    // Router: at least 2 out per 1 in, measured after the venue returns.
    let mut s = stack(Environment::Router, &[entry(PRICE_LIMIT, 1 << ROUTER_AFTER_SWAP, limit_cfg(LIMIT_REALIZED, (2, 1), (0, 0)))]);
    assert_eq!(validate_stack(&s, 0), Ok(()));
    assert!(run(&mut s, ROUTER_AFTER_SWAP, &Op { amount: 100, amount_b: 200, ..Op::default() }).is_ok());
    assert_eq!(run(&mut s, ROUTER_AFTER_SWAP, &Op { amount: 100, amount_b: 199, ..Op::default() }).unwrap_err().reason, reason::PRICE_LIMIT);
    assert!(run(&mut s, ROUTER_AFTER_SWAP, &Op { amount: 0, amount_b: 0, ..Op::default() }).is_err());
    // Asset filter: other input assets pass.
    let mint = Pubkey::new_unique();
    let mut c = limit_cfg(LIMIT_REALIZED, (2, 1), (0, 0));
    c.asset = mint.to_bytes();
    let mut s = stack(Environment::Router, &[entry(PRICE_LIMIT, 1 << ROUTER_AFTER_SWAP, c)]);
    assert!(run(&mut s, ROUTER_AFTER_SWAP, &Op { amount: 100, amount_b: 1, ..Op::default() }).is_ok());
    assert!(run(&mut s, ROUTER_AFTER_SWAP, &Op { asset: mint, amount: 100, amount_b: 1, ..Op::default() }).is_err());

    // Pool range: price (reserve_1/reserve_0) must end within [0.9, 1.1].
    let mut s = stack(Environment::Pool, &[entry(PRICE_LIMIT, 1 << POOL_AFTER_SWAP, limit_cfg(LIMIT_POOL_PRICE, (9, 10), (11, 10)))]);
    assert_eq!(validate_stack(&s, 0), Ok(()));
    assert!(run(&mut s, POOL_AFTER_SWAP, &Op { reserves: [1_000, 1_050], ..Op::default() }).is_ok());
    assert!(run(&mut s, POOL_AFTER_SWAP, &Op { reserves: [1_000, 1_101], ..Op::default() }).is_err());
    assert!(run(&mut s, POOL_AFTER_SWAP, &Op { reserves: [1_000, 899], ..Op::default() }).is_err());
    // Inverted bounds and empty modes are config errors.
    for bad in [limit_cfg(LIMIT_REALIZED, (3, 1), (2, 1)), limit_cfg(0, (1, 1), (0, 0)), limit_cfg(LIMIT_REALIZED, (0, 0), (0, 0))] {
        assert!(validate_stack(&stack(Environment::Pool, &[entry(PRICE_LIMIT, 1 << POOL_AFTER_SWAP, bad)]), 0).is_err());
    }
}

/// Borsh `GatewayToken` (solana-gateway 0.5.0).
pub(crate) fn gateway_token(owner: &Pubkey, network: &Pubkey, state: u8, expiry: Option<i64>) -> Vec<u8> {
    let mut d = vec![0u8, 0];
    d.extend_from_slice(owner.as_ref());
    d.push(0);
    d.extend_from_slice(network.as_ref());
    d.extend_from_slice(Pubkey::new_unique().as_ref());
    d.push(state);
    match expiry {
        Some(e) => {
            d.push(1);
            d.extend_from_slice(&e.to_le_bytes());
        }
        None => d.push(0),
    }
    d
}

#[test]
fn credential_requires_an_active_unexpired_gateway_token() {
    let user = Pubkey::new_unique();
    let network = Pubkey::new_unique();
    let cfg = CredentialConfig { target: TARGET_ACTOR, flags: 0, _pad: [0; 6], gatekeeper_network: network.to_bytes(), _reserved: [0; 32], _rest: [0; 24] };
    let mut s = stack(Environment::Router, &[entry(CREDENTIAL, 1 << ROUTER_BEFORE_SWAP, cfg)]);
    assert_eq!(validate_stack(&s, 0), Ok(()));
    let check = |s: &mut HookStack, owner: Pubkey, data: Vec<u8>| {
        let recs = [Record { owner, data: &data }];
        run(s, ROUTER_BEFORE_SWAP, &Op { actor: user, now: 1_000, records: &recs, ..Op::default() }).map(|o| o[0].unwrap().value)
    };
    assert_eq!(check(&mut s, GATEWAY_PROGRAM, gateway_token(&user, &network, 0, Some(2_000))), Ok(2_000));
    let fail = |r: Result<u64, Rejection>| r.unwrap_err().reason;
    assert_eq!(fail(check(&mut s, GATEWAY_PROGRAM, gateway_token(&user, &network, 0, Some(1_000)))), reason::CREDENTIAL);
    assert_eq!(fail(check(&mut s, GATEWAY_PROGRAM, gateway_token(&user, &network, 1, Some(2_000)))), reason::CREDENTIAL);
    assert_eq!(fail(check(&mut s, GATEWAY_PROGRAM, gateway_token(&user, &network, 2, Some(2_000)))), reason::CREDENTIAL);
    assert_eq!(fail(check(&mut s, GATEWAY_PROGRAM, gateway_token(&user, &Pubkey::new_unique(), 0, Some(2_000)))), reason::CREDENTIAL);
    assert_eq!(fail(check(&mut s, GATEWAY_PROGRAM, gateway_token(&Pubkey::new_unique(), &network, 0, Some(2_000)))), reason::CREDENTIAL);
    // Forged: correct bytes, wrong owner program.
    assert_eq!(fail(check(&mut s, Pubkey::new_unique(), gateway_token(&user, &network, 0, Some(2_000)))), reason::CREDENTIAL);
    // No expiry: refused unless the config accepts it.
    assert_eq!(fail(check(&mut s, GATEWAY_PROGRAM, gateway_token(&user, &network, 0, None))), reason::CREDENTIAL);
    let mut open = cfg;
    open.flags = CREDENTIAL_ALLOW_NO_EXPIRY;
    let mut s2 = stack(Environment::Router, &[entry(CREDENTIAL, 1 << ROUTER_BEFORE_SWAP, open)]);
    assert_eq!(check(&mut s2, GATEWAY_PROGRAM, gateway_token(&user, &network, 0, None)), Ok(i64::MAX as u64));
}

#[test]
fn gateway_token_parser_rejects_malformed_bytes() {
    let (u, n) = (Pubkey::new_unique(), Pubkey::new_unique());
    let good = gateway_token(&u, &n, 0, Some(5));
    assert_eq!(parse_gateway_token(&good).unwrap().owner_wallet, u);
    for cut in 0..good.len() {
        assert!(parse_gateway_token(&good[..cut]).is_none(), "{cut}");
    }
    let mut bad = good.clone();
    bad[1] = 7; // invalid Option tag
    assert!(parse_gateway_token(&bad).is_none());
    let mut bad = good;
    bad[2 + 32 + 1 + 64] = 3; // invalid state
    assert!(parse_gateway_token(&bad).is_none());
}

#[test]
fn slot_limit_caps_operations_and_same_slot_round_trips() {
    let cfg = SlotLimitConfig { max_per_slot: 3, flags: SLOT_LIMIT_PER_ACTOR, _pad: [0; 3], _rest: [0; 88] };
    let mut s = stack(Environment::Pool, &[entry(SLOT_LIMIT, (1 << POOL_BEFORE_SWAP) | (1 << POOL_BEFORE_ADD), cfg)]);
    assert_eq!(validate_stack(&s, 0), Ok(()));
    let (a, v, x) = (Pubkey::new_unique(), Pubkey::new_unique(), Pubkey::new_unique());
    let op = |actor, slot| Op { actor, slot, ..Op::default() };
    // Sandwich shape: attacker, victim, attacker in one slot → the back-run is refused.
    assert!(run(&mut s, POOL_BEFORE_SWAP, &op(a, 10)).is_ok());
    assert!(run(&mut s, POOL_BEFORE_SWAP, &op(v, 10)).is_ok());
    assert_eq!(run(&mut s, POOL_BEFORE_SWAP, &op(a, 10)).unwrap_err().reason, reason::SLOT_LIMIT);
    // Third distinct actor fits; a fourth op exceeds the per-slot cap.
    assert!(run(&mut s, POOL_BEFORE_ADD, &op(x, 10)).is_ok());
    assert!(run(&mut s, POOL_BEFORE_SWAP, &op(Pubkey::new_unique(), 10)).is_err());
    // A new slot resets the window.
    assert!(run(&mut s, POOL_BEFORE_SWAP, &op(a, 11)).is_ok());
    assert!(run(&mut s, POOL_BEFORE_SWAP, &op(a, 12)).is_ok());
}

#[test]
fn library_never_gates_pool_withdrawals_or_after_transfer() {
    for id in IDS {
        assert_eq!(supported_callbacks(id, Environment::Pool) & (1 << POOL_BEFORE_REMOVE), 0, "{id}");
        assert_eq!(supported_callbacks(id, Environment::Token) & 0b10, 0, "{id}");
        assert!(name(id).is_some());
        assert_ne!(required_caps(id), u32::MAX);
    }
}

proptest! {
    #[test]
    fn size_fee_is_monotone_and_bounded(base in 0u16..=10_000, span in 0u16..=10_000, pivot in 1u64..u64::MAX, a in any::<u64>(), b in any::<u64>()) {
        let max = base.saturating_add(span).min(10_000);
        let mut s = stack(Environment::Pool, &[entry(DYNAMIC_FEE, 1 << 6, size_fee(base, max, pivot))]);
        let (lo, hi) = (a.min(b), a.max(b));
        let f = |s: &mut HookStack, x| fee_of(&run(s, 6, &Op { amount: x, ..Op::default() }).unwrap());
        let (fl, fh) = (f(&mut s, lo), f(&mut s, hi));
        prop_assert!(fl <= fh);
        prop_assert!(fl >= base as u64 && fh <= max as u64);
    }

    #[test]
    fn volatility_fee_stays_in_bounds(prices in proptest::collection::vec((1u64..u64::MAX, 1u64..u64::MAX), 1..20)) {
        let cfg = DynamicFeeConfig { mode: FEE_MODE_VOLATILITY, _pad: [0; 3], base_bps: 10, max_bps: 500, pivot: 0, alpha_bps: 1_000, _pad2: 0, sensitivity_bps: 50_000, _rest: [0; 72] };
        let mut s = stack(Environment::Pool, &[entry(DYNAMIC_FEE, 1 << 6, cfg)]);
        for r in prices {
            let f = fee_of(&run(&mut s, 6, &Op { reserves: [r.0, r.1], ..Op::default() }).unwrap());
            prop_assert!((10..=500).contains(&f));
        }
    }

    #[test]
    fn price_impact_matches_constant_product(reserve in 1u64..u64::MAX / 2, amount in 0u64..u64::MAX / 2, max in 1u16..10_000) {
        let mut s = stack(Environment::Pool, &[entry(PRICE_IMPACT, 1 << 6, PriceImpactConfig { max_bps: max, _pad: [0; 6], _rest: [0; 88] })]);
        let impact = amount as u128 * 10_000 / (reserve as u128 + amount as u128);
        let r = run(&mut s, 6, &Op { amount, reserves: [reserve, 1], zero_for_one: true, ..Op::default() });
        prop_assert_eq!(r.is_ok(), impact <= max as u128);
    }
}

/// `hooks/library/builtins.json` (read by the SDK, analyzer and web) must match the kernel.
#[test]
fn library_manifest_matches_the_kernel() {
    use core::mem::offset_of;
    let raw = include_str!("../../../hooks/library/builtins.json");
    let doc: serde_json::Value = serde_json::from_str(raw).unwrap();
    let list = doc["builtins"].as_array().unwrap();
    assert_eq!(list.len(), IDS.len());
    let offsets = |id: u8| -> Vec<(&'static str, usize)> {
        macro_rules! o {
            ($t:ty, $($f:ident),*) => { vec![$((stringify!($f), offset_of!($t, $f))),*] };
        }
        match id {
            ALLOWLIST => o!(AllowlistConfig, target, source, max_age_slots, key_a, key_b),
            SPENDING_LIMIT => o!(SpendingLimitConfig, asset, limit, period_secs),
            TIMELOCK => o!(TimelockConfig, delay_secs, not_before),
            MULTISIG => o!(MultisigConfig, threshold, member_mask),
            CONDITIONAL => o!(ConditionalConfig, conditions, key),
            MAX_SIZE => o!(MaxSizeConfig, asset, max_amount),
            TIME_WINDOW => o!(TimeWindowConfig, not_before, not_after, daily_start, daily_end),
            ACCOUNTING => vec![],
            DENYLIST => o!(DenylistConfig, target, key_a, key_b),
            DYNAMIC_FEE => o!(DynamicFeeConfig, mode, base_bps, max_bps, pivot, alpha_bps, sensitivity_bps),
            PRICE_IMPACT => o!(PriceImpactConfig, max_bps),
            PRICE_LIMIT => o!(PriceLimitConfig, min_num, min_den, max_num, max_den, asset, mode),
            CREDENTIAL => o!(CredentialConfig, target, flags, gatekeeper_network),
            SLOT_LIMIT => o!(SlotLimitConfig, max_per_slot, flags),
            _ => unreachable!(),
        }
    };
    for (h, id) in list.iter().zip(IDS) {
        assert_eq!(h["id"].as_u64(), Some(id as u64));
        assert_eq!(h["slug"].as_str(), name(id));
        let caps: Vec<&str> = h["capabilities"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
        let want: Vec<&str> = (0..11).filter(|b| required_caps(id) & (1 << b) != 0).map(|b| hook_standard::cap::NAMES[b]).collect();
        let mut caps_sorted = caps.clone();
        caps_sorted.sort();
        let mut want_sorted = want.clone();
        want_sorted.sort();
        assert_eq!(caps_sorted, want_sorted, "{:?}", name(id));
        for env in Environment::ALL {
            let listed: Vec<&str> = h["environments"][env.name()].as_array().map(|a| a.iter().map(|c| c.as_str().unwrap()).collect()).unwrap_or_default();
            let mask = supported_callbacks(id, env);
            let want: Vec<&str> = env.callbacks().iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, n)| *n).collect();
            assert_eq!(listed, want, "{:?} {}", name(id), env.name());
        }
        let fields: Vec<(String, usize)> = h["config"].as_array().unwrap().iter().map(|f| (f["name"].as_str().unwrap().to_string(), f["offset"].as_u64().unwrap() as usize)).collect();
        let want: Vec<(String, usize)> = offsets(id).into_iter().map(|(n, o)| (n.to_string(), o)).collect();
        assert_eq!(fields, want, "{:?}", name(id));
        assert_eq!(h["compute"]["estimate_cu"].as_u64(), Some(crate::builtin::compute_estimate(id) as u64));
    }
}
