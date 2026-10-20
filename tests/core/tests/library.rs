//! Hook library builtins (dynamic fee, price impact, price limit, slot limit, denylist) running
//! inside the pool engine on SBF, installed from registry versions.

mod common;

use common::*;
use hook_kernel::builtin::{self, *};
use hook_standard::{Environment, HookStack, StackEntry, IMPL_BUILTIN};
use hooks_interface::{math, Callback};

const SEED: u64 = 1_000_000_000;
const CB_ADD: u16 = 1 << Callback::BeforeAddLiquidity as u8;
const CB_SWAP: u16 = 1 << Callback::BeforeSwap as u8;
const CB_AFTER_SWAP: u16 = 1 << Callback::AfterSwap as u8;

fn signer(u: &User) -> solana_keypair::Keypair {
    u.kp.insecure_clone()
}

fn entry<C: bytemuck::Pod>(version: &Pk, id: u8, callbacks: u16, cfg: C) -> StackEntry {
    let mut e: StackEntry = bytemuck::Zeroable::zeroed();
    e.hook_version = version.to_bytes();
    e.impl_kind = IMPL_BUILTIN;
    e.builtin_id = id;
    e.callbacks = callbacks;
    e.granted = builtin::required_caps(id);
    e.config.copy_from_slice(bytemuck::bytes_of(&cfg));
    e
}

/// A pool with `entries` attached before any liquidity, then seeded with `SEED` of each side.
fn pool_with_policy(env: &mut Env, entries: &[(u8, u16, Box<dyn Fn(&PoolKeys) -> [u8; 96]>)]) -> (PoolKeys, User) {
    let versions: Vec<Pk> = entries.iter().map(|(id, cb, _)| env.publish_builtin(*id, *cb)).collect();
    let (p, _) = env.pool_with(Default::default(), None, 0);
    let mut s = HookStack::empty(Environment::Pool);
    s.len = entries.len() as u8;
    for (i, (id, cb, cfg)) in entries.iter().enumerate() {
        s.entries[i] = entry(&versions[i], *id, *cb, cfg(&p));
    }
    let admin = env.admin.insecure_clone();
    let ix = env.attach_policy_ix(&kp(&admin), &p, &s, &versions);
    env.send_ok(&[ix], &[&admin]);
    let lp = env.user(&p, SEED * 10);
    let ix = env.add_liquidity_ix(&p, &lp, SEED, SEED, 0, vec![]);
    env.send_ok(&[ix], &[&signer(&lp)]);
    (p, lp)
}

fn bytes<T: bytemuck::Pod>(v: T) -> [u8; 96] {
    bytemuck::bytes_of(&v).try_into().unwrap()
}

#[test]
fn dynamic_fee_policy_sets_the_swap_fee() {
    let mut env = Env::new();
    let cfg = DynamicFeeConfig { mode: FEE_MODE_SIZE, _pad: [0; 3], base_bps: 30, max_bps: 100, pivot: 100_000_000, alpha_bps: 0, _pad2: 0, sensitivity_bps: 0, _rest: [0; 72] };
    let (p, _) = pool_with_policy(&mut env, &[(DYNAMIC_FEE, CB_SWAP, Box::new(move |_| bytes(cfg)))]);
    let trader = env.user(&p, SEED);

    // Small swap: ~base fee. 1_000_000 in → 30 + 70·0.01 = 30.7 → 30 bps (floor) = 3_000 ppm.
    let before = env.balance(&trader.token[1]);
    env.send_ok(&[env.swap_ix(&p, &trader, true, true, 1_000_000, 0, vec![])], &[&signer(&trader)]);
    let fee = math::fee_on_input(1_000_000, 3_000).unwrap();
    let expect = math::cp_amount_out(1_000_000 - fee, SEED, SEED).unwrap();
    assert_eq!(env.balance(&trader.token[1]) - before, expect);

    // At the pivot the fee reaches 100 bps (10_000 ppm), the pool's cap allowing.
    let s = env.pool(&p.pool);
    let before = env.balance(&trader.token[1]);
    env.send_ok(&[env.swap_ix(&p, &trader, true, true, 100_000_000, 0, vec![])], &[&signer(&trader)]);
    let fee = math::fee_on_input(100_000_000, 10_000).unwrap();
    let expect = math::cp_amount_out(100_000_000 - fee, s.reserve_0, s.reserve_1).unwrap();
    assert_eq!(env.balance(&trader.token[1]) - before, expect);
    // The pool's stored fee is untouched: the policy fee applies per swap.
    assert_eq!(env.pool(&p.pool).fee_ppm, 3_000);
    env.assert_solvent(&p);
}

#[test]
fn price_impact_policy_reads_live_reserves() {
    let mut env = Env::new();
    let cfg = PriceImpactConfig { max_bps: 50, _pad: [0; 6], _rest: [0; 88] };
    let (p, _) = pool_with_policy(&mut env, &[(PRICE_IMPACT, CB_SWAP, Box::new(move |_| bytes(cfg)))]);
    let trader = env.user(&p, SEED);
    // 5_000_000 into 1e9: 49 bps. 5_100_000: 50.7 → 50 (allowed). 5_200_000: 51 → rejected.
    env.send_ok(&[env.swap_ix(&p, &trader, true, true, 5_000_000, 0, vec![])], &[&signer(&trader)]);
    let err = env.send_err(&[env.swap_ix(&p, &trader, true, true, 6_000_000, 0, vec![])], &[&signer(&trader)]);
    assert!(err.contains("PolicyRejected") && err.contains("reason 14"), "{err}");
    env.send_ok(&[env.swap_ix(&p, &trader, false, true, 5_000_000, 0, vec![])], &[&signer(&trader)]);
    env.assert_solvent(&p);
}

#[test]
fn price_range_policy_reverts_swaps_that_leave_the_range() {
    let mut env = Env::new();
    // Range order: the pool price (reserve_1 / reserve_0) must stay within [0.98, 1.02].
    let cfg = PriceLimitConfig { min_num: 98, min_den: 100, max_num: 102, max_den: 100, asset: [0; 32], mode: LIMIT_POOL_PRICE, _pad: [0; 7], _rest: [0; 24] };
    let (p, _) = pool_with_policy(&mut env, &[(PRICE_LIMIT, CB_AFTER_SWAP, Box::new(move |_| bytes(cfg)))]);
    let trader = env.user(&p, SEED);
    env.send_ok(&[env.swap_ix(&p, &trader, true, true, 5_000_000, 0, vec![])], &[&signer(&trader)]);
    let before = env.pool(&p.pool);
    let err = env.send_err(&[env.swap_ix(&p, &trader, true, true, 20_000_000, 0, vec![])], &[&signer(&trader)]);
    assert!(err.contains("PolicyRejected") && err.contains("reason 15"), "{err}");
    // The rejected swap left no trace.
    let after = env.pool(&p.pool);
    assert_eq!((before.reserve_0, before.reserve_1), (after.reserve_0, after.reserve_1));
    env.assert_solvent(&p);
}

#[test]
fn realized_price_limit_guards_a_limit_order() {
    let mut env = Env::new();
    // Sell mint_0 only at ≥ 0.99 mint_1 each (net of fees and impact).
    let cfg = move |p: &PoolKeys| bytes(PriceLimitConfig { min_num: 99, min_den: 100, max_num: 0, max_den: 0, asset: p.mint[0].to_bytes(), mode: LIMIT_REALIZED, _pad: [0; 7], _rest: [0; 24] });
    let (p, _) = pool_with_policy(&mut env, &[(PRICE_LIMIT, CB_AFTER_SWAP, Box::new(cfg))]);
    let trader = env.user(&p, SEED);
    // 1_000_000: 30 bps fee + ~10 bps impact → 0.996: fills.
    env.send_ok(&[env.swap_ix(&p, &trader, true, true, 1_000_000, 0, vec![])], &[&signer(&trader)]);
    // 10_000_000: ~1.3% worse → refused, even though the trader's own min_out (0) would allow it.
    let err = env.send_err(&[env.swap_ix(&p, &trader, true, true, 10_000_000, 0, vec![])], &[&signer(&trader)]);
    assert!(err.contains("reason 15"), "{err}");
    // The other direction is not limited.
    env.send_ok(&[env.swap_ix(&p, &trader, false, true, 10_000_000, 0, vec![])], &[&signer(&trader)]);
}

#[test]
fn slot_limit_refuses_same_slot_round_trips() {
    let mut env = Env::new();
    let cfg = SlotLimitConfig { max_per_slot: 4, flags: SLOT_LIMIT_PER_ACTOR, _pad: [0; 3], _rest: [0; 88] };
    let (p, _) = pool_with_policy(&mut env, &[(SLOT_LIMIT, CB_SWAP | CB_ADD, Box::new(move |_| bytes(cfg)))]);
    let attacker = env.user(&p, SEED);
    let victim = env.user(&p, SEED);
    // The seeding add happened in this slot already (count 1, first actor = LP).
    env.send_ok(&[env.swap_ix(&p, &attacker, true, true, 10_000_000, 0, vec![])], &[&signer(&attacker)]);
    env.send_ok(&[env.swap_ix(&p, &victim, true, true, 10_000_000, 0, vec![])], &[&signer(&victim)]);
    let err = env.send_err(&[env.swap_ix(&p, &attacker, false, true, 10_000_000, 0, vec![])], &[&signer(&attacker)]);
    assert!(err.contains("reason 17"), "{err}");
    // Next slot: the attacker may trade again.
    let mut clock: solana_clock::Clock = env.svm.get_sysvar();
    clock.slot += 1;
    env.svm.set_sysvar(&clock);
    env.send_ok(&[env.swap_ix(&p, &attacker, false, true, 10_000_000, 0, vec![])], &[&signer(&attacker)]);
}

#[test]
fn denylist_policy_blocks_listed_traders_only() {
    let mut env = Env::new();
    let blocked = solana_keypair::Keypair::new();
    let bad = kp(&blocked);
    let cfg = DenylistConfig { target: TARGET_ACTOR, _pad: [0; 7], key_a: bad.to_bytes(), key_b: [0; 32], _rest: [0; 24] };
    let (p, _) = pool_with_policy(&mut env, &[(DENYLIST, CB_SWAP | CB_ADD, Box::new(move |_| bytes(cfg)))]);
    let ok = env.user(&p, SEED);
    env.send_ok(&[env.swap_ix(&p, &ok, true, true, 1_000_000, 0, vec![])], &[&signer(&ok)]);
    env.svm.airdrop(&addr(&bad), SOL).unwrap();
    let token = [0, 1].map(|i| env.token_account(&p.mint[i], &p.token_program[i], &bad));
    env.mint_to(&p.mint[0], &p.token_program[0], &token[0], 10_000_000);
    let listed = User { kp: blocked, token, lp: token[0] };
    let err = env.send_err(&[env.swap_ix(&p, &listed, true, true, 1_000_000, 0, vec![])], &[&signer(&listed)]);
    assert!(err.contains("reason 13"), "{err}");
}
