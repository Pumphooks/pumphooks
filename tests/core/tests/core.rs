mod common;

use anchor_lang::solana_program::instruction::AccountMeta as Meta2;
use common::*;
use hooks_core::{accounts as A, instruction as I, instructions::ProtocolUpdate, state::*};
use hooks_interface::{callback_authority, hook_authority, math, perm::*, Callback, HookResult};
use solana_signer::Signer;

const SEED: u64 = 1_000_000_000;

fn signer(u: &User) -> solana_keypair::Keypair {
    u.kp.insecure_clone()
}

// ---------------------------------------------------------------- protocol

#[test]
fn protocol_init_requires_core_upgrade_authority() {
    let mut env = Env::uninitialized();
    let intruder = solana_keypair::Keypair::new();
    env.svm.airdrop(&intruder.pubkey(), SOL).unwrap();
    let ix = env.init_protocol_ix(&kp(&intruder));
    assert!(env.send_err(&[ix], &[&intruder]).contains("NotUpgradeAuthority"));
    let admin = env.admin.insecure_clone();
    let ix = env.init_protocol_ix(&kp(&admin));
    env.send_ok(&[ix], &[&admin]);
    assert_eq!(env.protocol_state().admin, kp(&admin));
    assert_eq!(env.protocol_state().protocol_fee_bps, 0);
}

#[test]
fn protocol_fee_share_is_capped_and_paid_out() {
    let mut env = Env::new();
    let ix = core_ix(
        A::UpdateProtocol { admin: kp(&env.admin), protocol: env.protocol },
        vec![],
        I::UpdateProtocol { update: ProtocolUpdate { protocol_fee_bps: Some(2_501), ..Default::default() } },
    );
    let admin = env.admin.insecure_clone();
    assert!(env.send_err(&[ix], &[&admin]).contains("ProtocolFeeTooHigh"));

    env.update_protocol(ProtocolUpdate { protocol_fee_bps: Some(2_000), ..Default::default() });
    let (p, lp) = env.pool_with(Default::default(), None, SEED);
    let ix = env.swap_ix(&p, &lp, true, true, 1_000_000, 0, vec![]);
    env.send_ok(&[ix], &[&signer(&lp)]);
    let s = env.pool(&p.pool);
    assert_eq!(s.protocol_fee_0, 600); // 20% of the 3000-unit fee
    env.assert_solvent(&p);

    let dest = [
        env.token_account(&p.mint[0], &p.token_program[0], &kp(&admin)),
        env.token_account(&p.mint[1], &p.token_program[1], &kp(&admin)),
    ];
    let ix = core_ix(
        A::WithdrawProtocolFees {
            fee_authority: kp(&admin),
            protocol: env.protocol,
            io: env.pool_io(&p),
            destination_0: dest[0],
            destination_1: dest[1],
        },
        vec![],
        I::WithdrawProtocolFees {},
    );
    env.send_ok(&[ix], &[&admin]);
    assert_eq!(env.balance(&dest[0]), 600);
    assert_eq!(env.pool(&p.pool).protocol_fee_0, 0);
    env.assert_solvent(&p);
}

// ---------------------------------------------------------------- plain pools

#[test]
fn plain_pool_lifecycle() {
    let mut env = Env::new();
    let (p, lp) = env.pool_with(Default::default(), None, SEED);
    let s = env.pool(&p.pool);
    assert_eq!((s.reserve_0, s.reserve_1, s.lp_supply), (SEED, SEED, SEED));
    assert_eq!(env.balance(&lp.lp), SEED - math::MIN_LIQUIDITY);

    // exact-in
    let trader = env.user(&p, 100_000_000);
    let ix = env.swap_ix(&p, &trader, true, true, 10_000_000, 0, vec![]);
    env.send_ok(&[ix], &[&signer(&trader)]);
    let expect = math::cp_amount_out(10_000_000 - 30_000, SEED, SEED).unwrap();
    assert_eq!(env.balance(&trader.token[1]), 100_000_000 + expect);
    env.assert_solvent(&p);

    // exact-out with a tight max-in that is exactly met
    let s = env.pool(&p.pool);
    let net = math::cp_amount_in(5_000_000, s.reserve_1, s.reserve_0).unwrap();
    let gross = math::gross_up(net, 3_000).unwrap();
    let before = env.balance(&trader.token[1]);
    let ix = env.swap_ix(&p, &trader, false, false, 5_000_000, gross - 1, vec![]);
    assert!(env.send_err(&[ix], &[&signer(&trader)]).contains("Slippage"));
    let ix = env.swap_ix(&p, &trader, false, false, 5_000_000, gross, vec![]);
    env.send_ok(&[ix], &[&signer(&trader)]);
    assert_eq!(env.balance(&trader.token[0]), 100_000_000 - 5_000_000);
    assert_eq!(env.balance(&trader.token[1]), before - gross);

    // min-out enforced
    let ix = env.swap_ix(&p, &trader, true, true, 1_000, 1_000, vec![]);
    assert!(env.send_err(&[ix], &[&signer(&trader)]).contains("Slippage"));

    // second LP gets proportional shares; full exit returns at most what was put in
    let lp2 = env.user(&p, 1_000_000_000);
    let ix = env.add_liquidity_ix(&p, &lp2, 50_000_000, 50_000_000, 1, vec![]);
    env.send_ok(&[ix], &[&signer(&lp2)]);
    let shares = env.balance(&lp2.lp);
    let paid = [1_000_000_000 - env.balance(&lp2.token[0]), 1_000_000_000 - env.balance(&lp2.token[1])];
    assert!(paid[0] <= 50_000_000 && paid[1] <= 50_000_000 && shares > 0);
    let ix = env.remove_liquidity_ix(&p, &lp2, shares, 0, 0, vec![]);
    env.send_ok(&[ix], &[&signer(&lp2)]);
    assert!(1_000_000_000 - env.balance(&lp2.token[0]) <= paid[0]);
    assert!(env.balance(&lp2.token[0]) + env.balance(&lp2.token[1]) >= 2_000_000_000 - paid[0] - paid[1] - 2);
    assert_eq!(env.balance(&lp2.lp), 0);
    env.assert_solvent(&p);

    // donate raises reserves without shares
    let r0 = env.pool(&p.pool).reserve_0;
    let ix = env.donate_ix(&p, &trader, 1_000, 0);
    env.send_ok(&[ix], &[&signer(&trader)]);
    assert_eq!(env.pool(&p.pool).reserve_0, r0 + 1_000);
    assert!(env.pool(&p.pool).sequence >= 6);
}

#[test]
fn pool_creation_validates_inputs() {
    let mut env = Env::new();
    let (mint, tp) = env.mint_pair(MintKind::Spl, MintKind::Spl);
    let admin = env.admin.insecure_clone();
    // unsorted mints
    let mut bad = env.pool_keys([mint[1], mint[0]], [tp[1], tp[0]], None, 0);
    bad.vault = [bad.vault[0], bad.vault[1]];
    let ix = env.create_pool_ix(&kp(&admin), &bad, 0, 3_000, 10_000, 0, false, vec![]);
    assert!(env.send_err(&[ix], &[&admin]).contains("MintOrder"));
    // fee above max / max above cap
    let p = env.pool_keys(mint, tp, None, 0);
    let ix = env.create_pool_ix(&kp(&admin), &p, 0, 20_000, 10_000, 0, false, vec![]);
    assert!(env.send_err(&[ix], &[&admin]).contains("FeeTooHigh"));
    let ix = env.create_pool_ix(&kp(&admin), &p, 0, 0, 500_001, 0, false, vec![]);
    assert!(env.send_err(&[ix], &[&admin]).contains("FeeTooHigh"));
    // permissions without a hook
    let ix = env.create_pool_ix(&kp(&admin), &p, BEFORE_SWAP, 3_000, 10_000, 0, false, vec![]);
    assert!(env.send_err(&[ix], &[&admin]).contains("InvalidPermissions"));
    // unregistered hook
    let hp = env.pool_keys(mint, tp, Some(env.hook), 0);
    let ix = env.create_pool_ix(&kp(&admin), &hp, BEFORE_SWAP, 3_000, 10_000, 0, false, vec![]);
    assert!(env.send_err(&[ix], &[&admin]).contains("AccountNotInitialized"));
    // same mints, different salt → separate pools
    for salt in [0u64, 1] {
        let p = env.pool_keys(mint, tp, None, salt);
        let ix = env.create_pool_ix(&kp(&admin), &p, 0, 3_000, 10_000, salt, false, vec![]);
        env.send_ok(&[ix], &[&admin]);
    }
}

// ---------------------------------------------------------------- registry

#[test]
fn registration_rules() {
    let mut env = Env::new();
    let hook = env.hook;
    // Not the upgrade authority.
    let admin = env.admin.insecure_clone();
    let ix = env.register_hook_ix(&kp(&admin), &hook, BEFORE_SWAP);
    assert!(env.send_err(&[ix], &[&admin]).contains("NotUpgradeAuthority"));
    // Invalid mask (returns-delta without its callback).
    let dev = env.hook_dev.insecure_clone();
    let ix = env.register_hook_ix(&kp(&dev), &hook, BEFORE_SWAP_RETURNS_DELTA);
    assert!(env.send_err(&[ix], &[&dev]).contains("InvalidPermissions"));
    // Immutable programs can be registered by anyone.
    env.set_upgrade_authority(&hook, None);
    let ix = env.register_hook_ix(&kp(&admin), &hook, BEFORE_SWAP | AFTER_SWAP);
    env.send_ok(&[ix], &[&admin]);
    assert_eq!(env.protocol_state().hook_count, 1);

    // Pools may only use declared permissions.
    let (mint, tp) = env.mint_pair(MintKind::Spl, MintKind::Spl);
    let p = env.pool_keys(mint, tp, Some(hook), 0);
    let ix = env.create_pool_ix(&kp(&admin), &p, BEFORE_SWAP | DYNAMIC_FEE, 3_000, 10_000, 0, false, vec![]);
    assert!(env.send_err(&[ix], &[&admin]).contains("PermissionsNotDeclared"));
    let ix = env.create_pool_ix(&kp(&admin), &p, BEFORE_SWAP, 3_000, 10_000, 0, false, vec![]);
    env.send_ok(&[ix], &[&admin]);

    // Attestations are signed statements; revocable by the attester only.
    let auditor = solana_keypair::Keypair::new();
    env.svm.airdrop(&auditor.pubkey(), SOL).unwrap();
    let att = Pk::find_program_address(&[ATTEST_SEED, hook.as_ref(), kp(&auditor).as_ref()], &CORE).0;
    let ix = core_ix(
        A::Attest {
            attester: kp(&auditor),
            registration: env.registration(&hook),
            attestation: att,
            system_program: anchor_lang::system_program::ID,
        },
        vec![],
        I::Attest { kind: ATTEST_REVIEW, content_hash: [7; 32], uri: "https://example.com/review".into() },
    );
    env.send_ok(&[ix], &[&auditor]);
    let ix = core_ix(A::RevokeAttestation { attester: kp(&admin), attestation: att }, vec![], I::RevokeAttestation {});
    assert!(env.send_err(&[ix], &[&admin]).contains("Unauthorized"));
    let ix = core_ix(A::RevokeAttestation { attester: kp(&auditor), attestation: att }, vec![], I::RevokeAttestation {});
    env.send_ok(&[ix], &[&auditor]);
    assert!(!env.account_exists(&att));
}

// ---------------------------------------------------------------- hook powers

const ALL_SWAP: u32 = BEFORE_SWAP | AFTER_SWAP | BEFORE_SWAP_RETURNS_DELTA | AFTER_SWAP_RETURNS_DELTA | DYNAMIC_FEE;

#[test]
fn before_swap_can_reject() {
    let mut env = Env::new();
    let (p, _) = env.pool_with(Default::default(), Some(ALL_SWAP), SEED);
    let t = env.user(&p, 10_000_000);
    let ix = env.swap_ix(&p, &t, true, true, 1_000, 0, script(&[(Callback::BeforeSwap, HookResult::reject(42))]));
    let err = env.send_err(&[ix], &[&signer(&t)]);
    assert!(err.contains("hook rejected before_swap: reason 42") && err.contains("HookRejected"), "{err}");
}

#[test]
fn dynamic_fee_override() {
    let mut env = Env::new();
    let (p, _) = env.pool_with(Default::default(), Some(ALL_SWAP), SEED);
    let t = env.user(&p, 10_000_000);
    let ix = env.swap_ix(&p, &t, true, true, 1_000_000, 0, script(&[(Callback::BeforeSwap, HookResult::ok().with_fee(50_000))]));
    env.send_ok(&[ix], &[&signer(&t)]);
    let out = env.balance(&t.token[1]) - 10_000_000;
    assert_eq!(out, math::cp_amount_out(950_000, SEED, SEED).unwrap());
    // above the pool's max_fee_ppm (100_000)
    let ix = env.swap_ix(&p, &t, true, true, 1_000, 0, script(&[(Callback::BeforeSwap, HookResult::ok().with_fee(100_001))]));
    assert!(env.send_err(&[ix], &[&signer(&t)]).contains("HookResultNotPermitted"));
}

#[test]
fn noop_takeover_settles_against_hook_credit() {
    let mut env = Env::new();
    let (p, lp) = env.pool_with(Default::default(), Some(ALL_SWAP), SEED);
    // Fund the hook's credit in token 1, then let it fill a 1000 → 990 swap entirely itself.
    let ix = env.credit_deposit_ix(&p, &lp, 0, 5_000);
    env.send_ok(&[ix], &[&signer(&lp)]);
    let t = env.user(&p, 10_000);
    let r0 = env.pool(&p.pool).reserve_0;
    let ix = env.swap_ix(&p, &t, true, true, 1_000, 990, script(&[(Callback::BeforeSwap, HookResult::ok().with_deltas(1_000, -990))]));
    env.send_ok(&[ix], &[&signer(&t)]);
    assert_eq!(env.balance(&t.token[1]), 10_990);
    let s = env.pool(&p.pool);
    assert_eq!((s.credit_0, s.credit_1, s.reserve_0), (1_000, 4_010, r0));
    env.assert_solvent(&p);

    // The hook cannot pay out more than its credit.
    let ix = env.swap_ix(&p, &t, true, true, 1_000, 0, script(&[(Callback::BeforeSwap, HookResult::ok().with_deltas(1_000, -5_000))]));
    assert!(env.send_err(&[ix], &[&signer(&t)]).contains("InsufficientCredit"));
    // ...nor flip exact-in into something else.
    let ix = env.swap_ix(&p, &t, true, true, 1_000, 0, script(&[(Callback::BeforeSwap, HookResult::ok().with_deltas(1_001, 0))]));
    assert!(env.send_err(&[ix], &[&signer(&t)]).contains("DeltaExceedsAmount"));
}

#[test]
fn after_swap_fee_is_bounded_by_trader_limit() {
    let mut env = Env::new();
    let (p, _) = env.pool_with(Default::default(), Some(ALL_SWAP), SEED);
    let t = env.user(&p, 10_000_000);
    let curve = math::cp_amount_out(997_000, SEED, SEED).unwrap();
    let take = script(&[(Callback::AfterSwap, HookResult::ok().with_deltas(0, 10_000))]);
    let ix = env.swap_ix(&p, &t, true, true, 1_000_000, curve - 9_999, take.clone());
    assert!(env.send_err(&[ix], &[&signer(&t)]).contains("Slippage"));
    let ix = env.swap_ix(&p, &t, true, true, 1_000_000, curve - 10_000, take);
    env.send_ok(&[ix], &[&signer(&t)]);
    assert_eq!(env.pool(&p.pool).credit_1, 10_000);
    // A non-zero specified delta from afterSwap is never allowed.
    let ix = env.swap_ix(&p, &t, true, true, 1_000, 0, script(&[(Callback::AfterSwap, HookResult::ok().with_deltas(1, 0))]));
    assert!(env.send_err(&[ix], &[&signer(&t)]).contains("HookResultNotPermitted"));
}

#[test]
fn custom_curve_quote_prices_the_swap() {
    let mut env = Env::new();
    let (p, _) = env.pool_with(Default::default(), Some(CUSTOM_CURVE | BEFORE_SWAP), SEED);
    let t = env.user(&p, 10_000_000);
    // A 1:2 fixed-price curve chosen by the hook for this swap.
    let ix = env.swap_ix(&p, &t, true, true, 1_000, 0, script(&[(Callback::Quote, HookResult::ok().with_quote(1_994))]));
    env.send_ok(&[ix], &[&signer(&t)]);
    assert_eq!(env.balance(&t.token[1]), 10_001_994);
    // Quotes that would drain the reserve are refused.
    let ix = env.swap_ix(&p, &t, true, true, 1_000, 0, script(&[(Callback::Quote, HookResult::ok().with_quote(SEED * 2))]));
    assert!(env.send_err(&[ix], &[&signer(&t)]).contains("QuoteOutOfRange"));
    env.assert_solvent(&p);
}

#[test]
fn liquidity_hooks_charge_and_withhold() {
    let mut env = Env::new();
    let perms = BEFORE_ADD_LIQUIDITY
        | AFTER_ADD_LIQUIDITY
        | AFTER_ADD_LIQUIDITY_RETURNS_DELTA
        | BEFORE_REMOVE_LIQUIDITY
        | AFTER_REMOVE_LIQUIDITY
        | AFTER_REMOVE_LIQUIDITY_RETURNS_DELTA;
    let (p, lp) = env.pool_with(Default::default(), Some(perms), SEED);
    // Withdrawal fee of 1000 token-0 into hook credit.
    let shares = 1_000_000;
    let before = env.balance(&lp.token[0]);
    let ix = env.remove_liquidity_ix(&p, &lp, shares, 0, 0, script(&[(Callback::AfterRemoveLiquidity, HookResult::ok().with_deltas(1_000, 0))]));
    env.send_ok(&[ix], &[&signer(&lp)]);
    assert_eq!(env.balance(&lp.token[0]) - before, 1_000_000 - 1_000);
    assert_eq!(env.pool(&p.pool).credit_0, 1_000);
    // Rebate on deposit paid from that credit.
    let before = env.balance(&lp.token[0]);
    let ix = env.add_liquidity_ix(&p, &lp, 1_000_000, 1_000_000, 0, script(&[(Callback::AfterAddLiquidity, HookResult::ok().with_deltas(-400, 0))]));
    env.send_ok(&[ix], &[&signer(&lp)]);
    assert_eq!(before - env.balance(&lp.token[0]), 1_000_000 - 400);
    assert_eq!(env.pool(&p.pool).credit_0, 600);
    // A hook with BEFORE_REMOVE_LIQUIDITY can block exits (disclosed power).
    let ix = env.remove_liquidity_ix(&p, &lp, 10, 0, 0, script(&[(Callback::BeforeRemoveLiquidity, HookResult::reject(9))]));
    assert!(env.send_err(&[ix], &[&signer(&lp)]).contains("HookRejected"));
    env.assert_solvent(&p);
}

// ---------------------------------------------------------------- hostile hooks

#[test]
fn hook_cannot_spend_trader_tokens() {
    let mut env = Env::new();
    let (p, _) = env.pool_with(Default::default(), Some(ALL_SWAP), SEED);
    let t = env.user(&p, 10_000_000);
    let thief = env.token_account(&p.mint[0], &p.token_program[0], &Pk::new_unique());
    let extra = vec![
        Meta2::new(t.token[0], false),
        Meta2::new(thief, false),
        Meta2::new_readonly(p.token_program[0], false),
        // The trader signed the outer transaction; the core must not forward that signature.
        Meta2::new_readonly(t.key(), false),
    ];
    let ix = env.swap_ix_extra(&p, &t, true, true, 1_000, 0, vec![test_hook::STEAL], extra);
    let err = env.send_err(&[ix], &[&signer(&t)]);
    assert!(err.contains("signer privilege escalated") || err.contains("PrivilegeEscalation"), "{err}");
    assert_eq!(env.balance(&thief), 0);
}

#[test]
fn hook_cannot_reenter_core() {
    let mut env = Env::new();
    let (p, _) = env.pool_with(Default::default(), Some(ALL_SWAP), SEED);
    let t = env.user(&p, 10_000_000);
    let extra = vec![Meta2::new_readonly(CORE, false)];
    let ix = env.swap_ix_extra(&p, &t, true, true, 1_000, 0, vec![test_hook::REENTER], extra);
    let err = env.send_err(&[ix], &[&signer(&t)]);
    assert!(err.contains("ReentrancyNotAllowed") || err.contains("reentrancy"), "{err}");
}

#[test]
fn silent_hook_is_an_error_not_an_allow() {
    let mut env = Env::new();
    let (p, _) = env.pool_with(Default::default(), Some(ALL_SWAP), SEED);
    let t = env.user(&p, 10_000_000);
    let ix = env.swap_ix(&p, &t, true, true, 1_000, 0, vec![test_hook::SILENT]);
    assert!(env.send_err(&[ix], &[&signer(&t)]).contains("HookResultInvalid"));
}

#[test]
fn forged_callbacks_are_refused_by_the_hook() {
    let mut env = Env::new();
    let (p, _) = env.pool_with(Default::default(), Some(ALL_SWAP), SEED);
    let fake = solana_keypair::Keypair::new();
    env.svm.airdrop(&fake.pubkey(), SOL).unwrap();
    let mut ctx: hooks_interface::HookContext = bytemuck::Zeroable::zeroed();
    ctx.version = 1;
    ctx.callback = Callback::BeforeSwap as u8;
    ctx.pool = p.pool.to_bytes();
    let mut data = vec![0u8; 8 + hooks_interface::CONTEXT_LEN];
    hooks_interface::encode_call(&ctx, &[], &mut data).unwrap();
    // Signed by a random key instead of the callback authority.
    let ix = convert(env.hook, vec![Meta2::new_readonly(kp(&fake), true), Meta2::new_readonly(p.pool, false)], data);
    assert!(env.send_err(&[ix], &[&fake]).contains("invalid program argument"));
}

#[test]
fn wrong_hook_accounts_are_rejected() {
    let mut env = Env::new();
    let (p, _) = env.pool_with(Default::default(), Some(ALL_SWAP), SEED);
    let t = env.user(&p, 10_000_000);
    let mut ix = env.swap_ix(&p, &t, true, true, 1_000, 0, vec![]);
    // Swap the callback authority for another hook's PDA.
    let other = solana_address::Address::new_from_array(callback_authority(&Pk::new_unique()).0.to_bytes());
    let cb = solana_address::Address::new_from_array(callback_authority(&env.hook).0.to_bytes());
    for m in ix.accounts.iter_mut() {
        if m.pubkey == cb {
            m.pubkey = other;
        }
    }
    assert!(env.send_err(&[ix], &[&signer(&t)]).contains("HookAccountMismatch"));
}

// ---------------------------------------------------------------- hook-initiated operations

#[test]
fn credit_operations_require_hook_authority() {
    let mut env = Env::new();
    let (p, lp) = env.pool_with(Default::default(), Some(ALL_SWAP), SEED);
    let ix = env.credit_deposit_ix(&p, &lp, 100_000, 100_000);
    env.send_ok(&[ix], &[&signer(&lp)]);
    let dest = [
        env.token_account(&p.mint[0], &p.token_program[0], &kp(&env.hook_dev)),
        env.token_account(&p.mint[1], &p.token_program[1], &kp(&env.hook_dev)),
    ];

    // Anyone else signing as "hook authority" is refused.
    let impostor = lp.kp.insecure_clone();
    let ix = core_ix(env.hook_op_accounts(&p, kp(&impostor), Some(dest)), vec![], I::CreditWithdraw { amount_0: 1, amount_1: 0 });
    assert!(env.send_err(&[ix], &[&impostor]).contains("Unauthorized"));

    let auth = hook_authority(&env.hook).0;
    let payer = env.hook_dev.insecure_clone();
    // credit_swap: 50_000 of token-0 credit into token-1 credit, no callbacks.
    let ix = env.via_hook(core_ix(
        env.hook_op_accounts(&p, auth, None),
        vec![],
        I::CreditSwap { zero_for_one: true, exact_in: true, amount: 50_000, limit: 1, quote: 0 },
    ));
    env.send_ok(&[ix], &[&payer]);
    let s = env.pool(&p.pool);
    let out = math::cp_amount_out(50_000 - 150, SEED, SEED).unwrap();
    assert_eq!((s.credit_0, s.credit_1), (50_000, 100_000 + out));
    // credit_withdraw
    let ix = env.via_hook(core_ix(env.hook_op_accounts(&p, auth, Some(dest)), vec![], I::CreditWithdraw { amount_0: 50_000, amount_1: 0 }));
    env.send_ok(&[ix], &[&payer]);
    assert_eq!(env.balance(&dest[0]), 50_000);
    let ix = env.via_hook(core_ix(env.hook_op_accounts(&p, auth, Some(dest)), vec![], I::CreditWithdraw { amount_0: 1, amount_1: 0 }));
    assert!(env.send_err(&[ix], &[&payer]).contains("InsufficientCredit"));
    // credit_donate pays LPs
    let r1 = env.pool(&p.pool).reserve_1;
    let ix = env.via_hook(core_ix(env.hook_op_accounts(&p, auth, None), vec![], I::CreditDonate { amount_0: 0, amount_1: 1_000 }));
    env.send_ok(&[ix], &[&payer]);
    assert_eq!(env.pool(&p.pool).reserve_1, r1 + 1_000);
    // set_fee within the pool's max
    let ix = env.via_hook(core_ix(A::SetFee { hook_authority: auth, pool: p.pool }, vec![], I::SetFee { fee_ppm: 100_001 }));
    assert!(env.send_err(&[ix], &[&payer]).contains("FeeTooHigh"));
    let ix = env.via_hook(core_ix(A::SetFee { hook_authority: auth, pool: p.pool }, vec![], I::SetFee { fee_ppm: 10_000 }));
    env.send_ok(&[ix], &[&payer]);
    assert_eq!(env.pool(&p.pool).fee_ppm, 10_000);
    env.assert_solvent(&p);
}

// ---------------------------------------------------------------- pause and pinning

#[test]
fn pause_never_blocks_withdrawals() {
    let mut env = Env::new();
    let (p, lp) = env.pool_with(Default::default(), None, SEED);
    env.update_protocol(ProtocolUpdate { paused: Some(PAUSE_SWAPS | PAUSE_ADDS | PAUSE_POOLS), ..Default::default() });
    let ix = env.swap_ix(&p, &lp, true, true, 1_000, 0, vec![]);
    assert!(env.send_err(&[ix], &[&signer(&lp)]).contains("Paused"));
    let ix = env.add_liquidity_ix(&p, &lp, 1_000, 1_000, 0, vec![]);
    assert!(env.send_err(&[ix], &[&signer(&lp)]).contains("Paused"));
    let shares = env.balance(&lp.lp);
    let ix = env.remove_liquidity_ix(&p, &lp, shares, 0, 0, vec![]);
    env.send_ok(&[ix], &[&signer(&lp)]);
    assert_eq!(env.balance(&lp.lp), 0);
    env.assert_solvent(&p);
}

#[test]
fn pinned_hook_stops_on_redeploy() {
    let mut env = Env::new();
    env.register_hook(ALL_SWAP);
    let (mint, tp) = env.mint_pair(MintKind::Spl, MintKind::Spl);
    let p = env.pool_keys(mint, tp, Some(env.hook), 0);
    let admin = env.admin.insecure_clone();
    let ix = env.create_pool_ix(&kp(&admin), &p, BEFORE_SWAP, 3_000, 10_000, 0, true, vec![]);
    env.send_ok(&[ix], &[&admin]);
    assert_eq!(env.pool(&p.pool).flags & POOL_HOOK_PINNED, POOL_HOOK_PINNED);
    let lp = env.user(&p, SEED * 2);
    let ix = env.add_liquidity_ix(&p, &lp, SEED, SEED, 0, vec![]);
    env.send_ok(&[ix], &[&signer(&lp)]);
    let ix = env.swap_ix(&p, &lp, true, true, 1_000, 0, vec![]);
    env.send_ok(&[ix], &[&signer(&lp)]);
    let hook = env.hook;
    env.set_deploy_slot(&hook, 999);
    let ix = env.swap_ix(&p, &lp, true, true, 1_000, 0, vec![]);
    assert!(env.send_err(&[ix], &[&signer(&lp)]).contains("HookRedeployed"));
}

// ---------------------------------------------------------------- Token-2022

#[test]
fn token_2022_policy() {
    let mut env = Env::new();
    // Permanent delegate mints are refused.
    let (mint, tp) = env.mint_pair(MintKind::Spl, MintKind::T22PermanentDelegate);
    let p = env.pool_keys(mint, tp, None, 0);
    let admin = env.admin.insecure_clone();
    let ix = env.create_pool_ix(&kp(&admin), &p, 0, 3_000, 10_000, 0, false, vec![]);
    assert!(env.send_err(&[ix], &[&admin]).contains("UnsupportedMintExtension"));

    // Transfer-fee mints work; the pool credits what it actually receives.
    let (p, lp) = env.pool_with((MintKind::T22TransferFee(100), MintKind::T22TransferFee(100)), None, SEED);
    let s = env.pool(&p.pool);
    assert_eq!(s.reserve_0, SEED - SEED / 100);
    env.assert_solvent(&p);
    let t = env.user(&p, 10_000_000);
    let ix = env.swap_ix(&p, &t, true, true, 1_000_000, 0, vec![]);
    env.send_ok(&[ix], &[&signer(&t)]);
    env.assert_solvent(&p);
    // Exact-out with a fee-charging input is refused rather than mispriced.
    let ix = env.swap_ix(&p, &t, true, false, 1_000, u64::MAX, vec![]);
    assert!(env.send_err(&[ix], &[&signer(&t)]).contains("ExactOutTransferFee"));
    let _ = lp;

    // Plain Token-2022 on one side, SPL on the other.
    let (p, _) = env.pool_with((MintKind::T22Plain, MintKind::Spl), None, SEED);
    let t = env.user(&p, 10_000_000);
    let ix = env.swap_ix(&p, &t, false, false, 1_000, u64::MAX, vec![]);
    env.send_ok(&[ix], &[&signer(&t)]);
    env.assert_solvent(&p);
}

// ---------------------------------------------------------------- compute

#[test]
fn compute_budget_report() {
    let mut env = Env::new();
    let (p, _) = env.pool_with(Default::default(), None, SEED);
    let t = env.user(&p, 10_000_000);
    let ix = env.swap_ix(&p, &t, true, true, 1_000, 0, vec![]);
    let plain = env.send_ok(&[ix], &[&signer(&t)]).compute_units_consumed;

    let mut env = Env::new();
    let (p, _) = env.pool_with(Default::default(), Some(ALL_SWAP), SEED);
    let t = env.user(&p, 10_000_000);
    let ix = env.swap_ix(&p, &t, true, true, 1_000, 0, vec![]);
    let hooked = env.send_ok(&[ix], &[&signer(&t)]).compute_units_consumed;
    println!("swap CU: plain {plain}, with before+after hook {hooked}");
    assert!(plain < 30_000, "plain swap {plain} CU");
    assert!(hooked < 40_000, "hooked swap {hooked} CU");
}
