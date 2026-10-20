//! Hook registry: publication rules, append-only versions, status, named reviewers.

mod common;

use common::{registry::*, *};
use hook_kernel::builtin;
use hook_registry::{
    accounts as A, instruction as I, Hook, HookVersion, Review, STATUS_DEPRECATED, STATUS_REVOKED,
};
use hook_standard::{cap, Environment, StackEntry, IMPL_BUILTIN, IMPL_PROGRAM};
use solana_keypair::Keypair;

const POLICY: u32 = cap::VALIDATE_OPERATION | cap::VETO_OPERATION;

struct T {
    s: Svm,
    admin: Keypair,
    dev: Keypair,
    hook_program: Pk,
}

fn setup() -> T {
    let mut s = Svm::new();
    let admin = s.funded(100);
    let dev = s.funded(100);
    registry::setup(&mut s, &admin);
    let hook_program = Pk::new_unique();
    s.add_program(&hook_program, "test_hook.so");
    s.set_upgrade_authority(&hook_program, Some(kp(&dev)));
    T {
        s,
        admin,
        dev,
        hook_program,
    }
}

fn status_ix(
    authority: &Pk,
    hook: &Pk,
    version: &Pk,
    status: u8,
) -> solana_instruction::Instruction {
    ix(
        ID,
        A::SetVersionStatus {
            authority: *authority,
            hook: *hook,
            version: *version,
        },
        vec![],
        I::SetVersionStatus { status },
    )
}

fn review_ix(
    reviewer: &Pk,
    reviewer_account: Option<Pk>,
    version: &Pk,
    kind: u8,
) -> solana_instruction::Instruction {
    ix(
        ID,
        A::PostReview {
            reviewer: *reviewer,
            reviewer_account: Some(reviewer_account.unwrap_or(ID)),
            version: *version,
            review: review(version, reviewer),
            system_program: SYS,
        },
        vec![],
        I::PostReview {
            kind,
            evidence_hash: [9; 32],
            evidence_uri: "https://example.invalid/report.pdf".into(),
        },
    )
}

#[test]
fn initialization_cannot_be_front_run() {
    let mut s = Svm::new();
    let admin = s.funded(10);
    let attacker = s.funded(10);
    s.add_program(&ID, "hook_registry.so");
    s.set_upgrade_authority(&ID, Some(kp(&admin)));
    let e = s.send_err(&[initialize(&kp(&attacker))], &[&attacker]);
    assert!(e.contains("Unauthorized"), "{e}");
    s.send_ok(&[initialize(&kp(&admin))], &[&admin]);
}

#[test]
fn slugs_are_validated_and_unique() {
    let mut t = setup();
    let dev = kp(&t.dev);
    for bad in ["ab", "Has-Caps", "-lead", "trail-", "under_score"] {
        assert!(
            t.s.send_err(&[create_hook(&dev, bad)], &[&t.dev])
                .contains("InvalidSlug"),
            "{bad}"
        );
    }
    t.s.send_ok(&[create_hook(&dev, "my-hook")], &[&t.dev]);
    let other = t.s.funded(1);
    t.s.send_err(&[create_hook(&kp(&other), "my-hook")], &[&other]);
}

#[test]
fn builtins_only_from_admin_with_exact_claims() {
    let mut t = setup();
    let dev = kp(&t.dev);
    let admin = kp(&t.admin);
    let account_withdraw = 1 << 2;

    // A developer cannot publish a "builtin": builtins are code in our environment programs.
    t.s.send_ok(&[create_hook(&dev, "fake-builtin")], &[&t.dev]);
    let h = hook("fake-builtin");
    let p = builtin_params(
        builtin::SPENDING_LIMIT,
        Environment::Account,
        account_withdraw,
        [1, 0, 0],
    );
    assert!(t
        .s
        .send_err(&[publish(&dev, &h, 0, None, p)], &[&t.dev])
        .contains("Unauthorized"));

    t.s.send_ok(&[create_hook(&admin, "spending-limit")], &[&t.admin]);
    let h = hook("spending-limit");
    // Over-claimed capabilities, unsupported callbacks, unknown builtin.
    let mut over = builtin_params(
        builtin::SPENDING_LIMIT,
        Environment::Account,
        account_withdraw,
        [1, 0, 0],
    );
    over.capabilities |= cap::REQUEST_AUTHORIZED_TRANSFER;
    let mut wrong_cb = builtin_params(builtin::SPENDING_LIMIT, Environment::Account, 1, [1, 0, 0]);
    wrong_cb.callbacks[Environment::Account as usize] = 1; // before_deposit
    let mut unknown = builtin_params(
        builtin::SPENDING_LIMIT,
        Environment::Account,
        account_withdraw,
        [1, 0, 0],
    );
    unknown.builtin_id = 99;
    for p in [over, wrong_cb] {
        assert!(t
            .s
            .send_err(&[publish(&admin, &h, 0, None, p)], &[&t.admin])
            .contains("InvalidClaims"));
    }
    t.s.send_err(&[publish(&admin, &h, 0, None, unknown)], &[&t.admin]);

    let ok = builtin_params(
        builtin::SPENDING_LIMIT,
        Environment::Account,
        account_withdraw,
        [1, 0, 0],
    );
    t.s.send_ok(&[publish(&admin, &h, 0, None, ok)], &[&t.admin]);
    let v: HookVersion = t.s.get(&version(&h, 0));
    assert_eq!(
        (v.impl_kind, v.builtin_id, v.upgrade_policy, v.program),
        (
            IMPL_BUILTIN,
            builtin::SPENDING_LIMIT,
            hook_registry::UPGRADE_BUILTIN,
            Pk::default()
        )
    );
}

#[test]
fn program_versions_record_loader_facts() {
    let mut t = setup();
    let dev = kp(&t.dev);
    t.s.send_ok(&[create_hook(&dev, "my-policy")], &[&t.dev]);
    let h = hook("my-policy");
    let p = || program_params(Environment::Account, 0b100, POLICY, [1, 0, 0]);

    // Someone other than the upgrade authority cannot publish an upgradeable program.
    let mallory = t.s.funded(10);
    t.s.send_ok(&[create_hook(&kp(&mallory), "squat")], &[&mallory]);
    let e = t.s.send_err(
        &[publish(
            &kp(&mallory),
            &hook("squat"),
            0,
            Some(t.hook_program),
            p(),
        )],
        &[&mallory],
    );
    assert!(e.contains("Unauthorized"), "{e}");

    // Asset-moving capabilities are not available to program hooks in policy environments.
    for bad in [cap::REQUEST_AUTHORIZED_TRANSFER, cap::CUSTOM_PRICING] {
        let mut q = p();
        q.capabilities |= bad;
        assert!(t
            .s
            .send_err(&[publish(&dev, &h, 0, Some(t.hook_program), q)], &[&t.dev])
            .contains("InvalidClaims"));
    }
    // Callbacks listed for an environment not in the bitmask.
    let mut q = p();
    q.callbacks[Environment::Router as usize] = 1;
    assert!(t
        .s
        .send_err(&[publish(&dev, &h, 0, Some(t.hook_program), q)], &[&t.dev])
        .contains("InvalidClaims"));

    t.s.set_deploy_slot(&t.hook_program, 77);
    t.s.send_ok(
        &[publish(&dev, &h, 0, Some(t.hook_program), p())],
        &[&t.dev],
    );
    let v: HookVersion = t.s.get(&version(&h, 0));
    assert_eq!(v.program, t.hook_program);
    assert_eq!(v.observed_upgrade_authority, Some(dev));
    assert_eq!(v.observed_deploy_slot, 77);
    assert_eq!(v.upgrade_policy, hook_registry::UPGRADE_UPGRADEABLE);

    // Semver must increase; indices are sequential.
    let e = t.s.send_err(
        &[publish(&dev, &h, 1, Some(t.hook_program), p())],
        &[&t.dev],
    );
    assert!(e.contains("VersionNotIncreasing"), "{e}");
    t.s.set_upgrade_authority(&t.hook_program, None);
    t.s.send_ok(
        &[publish(
            &dev,
            &h,
            1,
            Some(t.hook_program),
            program_params(Environment::Account, 0b100, POLICY, [1, 1, 0]),
        )],
        &[&t.dev],
    );
    let v1: HookVersion = t.s.get(&version(&h, 1));
    assert_eq!(
        (v1.index, v1.upgrade_policy, v1.observed_upgrade_authority),
        (1, hook_registry::UPGRADE_IMMUTABLE, None)
    );
    let hk: Hook = t.s.get(&h);
    assert_eq!((hk.version_count, hk.latest), (2, Some([1, 1, 0])));

    // Pool hooks may request pricing (the pool core bounds it).
    t.s.send_ok(&[create_hook(&dev, "my-curve")], &[&t.dev]);
    let pool = program_params(
        Environment::Pool,
        1 << 10,
        cap::CUSTOM_PRICING | cap::VALIDATE_OPERATION,
        [0, 1, 0],
    );
    t.s.send_ok(
        &[publish(
            &dev,
            &hook("my-curve"),
            0,
            Some(t.hook_program),
            pool,
        )],
        &[&t.dev],
    );
}

#[test]
fn status_only_moves_forward_and_only_by_authority() {
    let mut t = setup();
    let v = publish_builtin(
        &mut t.s,
        &t.admin,
        "max-size",
        builtin::MAX_SIZE,
        Environment::Router,
        0b1,
    );
    let h = hook("max-size");
    let admin = kp(&t.admin);
    t.s.send_err(
        &[status_ix(&kp(&t.dev), &h, &v, STATUS_DEPRECATED)],
        &[&t.dev],
    );
    t.s.send_ok(&[status_ix(&admin, &h, &v, STATUS_DEPRECATED)], &[&t.admin]);
    assert!(t
        .s
        .send_err(&[status_ix(&admin, &h, &v, 0)], &[&t.admin])
        .contains("InvalidStatus"));
    t.s.send_ok(&[status_ix(&admin, &h, &v, STATUS_REVOKED)], &[&t.admin]);
    assert_eq!(t.s.get::<HookVersion>(&v).status, STATUS_REVOKED);
}

#[test]
fn audits_require_named_reviewers() {
    let mut t = setup();
    let v = publish_builtin(
        &mut t.s,
        &t.admin,
        "timelock",
        builtin::TIMELOCK,
        Environment::Account,
        1 << 2,
    );
    let auditor = t.s.funded(10);
    let anon = t.s.funded(10);

    // Not named: may only post a security warning, which is marked unverified.
    let e = t.s.send_err(
        &[review_ix(
            &kp(&anon),
            None,
            &v,
            hook_registry::REVIEW_INDEPENDENT_AUDIT,
        )],
        &[&anon],
    );
    assert!(e.contains("NotNamedReviewer"), "{e}");
    t.s.send_ok(
        &[review_ix(
            &kp(&anon),
            None,
            &v,
            hook_registry::REVIEW_SECURITY_WARNING,
        )],
        &[&anon],
    );
    assert!(!t.s.get::<Review>(&review(&v, &kp(&anon))).named_reviewer);

    let add = ix(
        ID,
        A::AddReviewer {
            admin: kp(&t.admin),
            config: config(),
            reviewer_account: reviewer(&kp(&auditor)),
            system_program: SYS,
        },
        vec![],
        I::AddReviewer {
            reviewer: kp(&auditor),
            name: "Example Auditors".into(),
            uri: "https://example.invalid".into(),
        },
    );
    // Only the admin names reviewers.
    let mut forged = add.clone();
    forged.accounts[0].pubkey = addr(&kp(&anon));
    t.s.send_err(&[forged], &[&anon]);
    t.s.send_ok(&[add], &[&t.admin]);

    // Presenting someone else's reviewer account does not make you named.
    let e = t.s.send_err(
        &[review_ix(
            &kp(&t.dev),
            Some(reviewer(&kp(&auditor))),
            &v,
            hook_registry::REVIEW_INDEPENDENT_AUDIT,
        )],
        &[&t.dev],
    );
    assert!(e.contains("Unauthorized"), "{e}");

    t.s.send_ok(
        &[review_ix(
            &kp(&auditor),
            Some(reviewer(&kp(&auditor))),
            &v,
            hook_registry::REVIEW_INDEPENDENT_AUDIT,
        )],
        &[&auditor],
    );
    let r: Review = t.s.get(&review(&v, &kp(&auditor)));
    assert!(r.named_reviewer && !r.retracted);

    let retract = ix(
        ID,
        A::RetractReview {
            reviewer: kp(&auditor),
            review: review(&v, &kp(&auditor)),
        },
        vec![],
        I::RetractReview {},
    );
    t.s.send_ok(&[retract], &[&auditor]);
    assert!(t.s.get::<Review>(&review(&v, &kp(&auditor))).retracted);
}

#[test]
fn installs_must_match_the_version() {
    let mut t = setup();
    let vkey = publish_builtin(
        &mut t.s,
        &t.admin,
        "spending-limit",
        builtin::SPENDING_LIMIT,
        Environment::Account,
        (1 << 2) | (1 << 5),
    );
    let v: HookVersion = t.s.get(&vkey);
    let mut e = StackEntry {
        hook_version: vkey.to_bytes(),
        impl_kind: IMPL_BUILTIN,
        builtin_id: builtin::SPENDING_LIMIT,
        callbacks: 1 << 2,
        granted: v.capabilities,
        ..bytemuck::Zeroable::zeroed()
    };
    hook_registry::verify_install(&vkey, &v, &e, Environment::Account).unwrap();
    assert!(
        hook_registry::verify_install(&vkey, &v, &e, Environment::Router).is_err(),
        "environment not published"
    );
    assert!(
        hook_registry::verify_install(&Pk::new_unique(), &v, &e, Environment::Account).is_err(),
        "different version account"
    );
    e.callbacks |= 1 << 4;
    assert!(
        hook_registry::verify_install(&vkey, &v, &e, Environment::Account).is_err(),
        "callback not published"
    );
    e.callbacks = 1 << 2;
    e.builtin_id = builtin::TIMELOCK;
    assert!(
        hook_registry::verify_install(&vkey, &v, &e, Environment::Account).is_err(),
        "different builtin"
    );
    e.builtin_id = builtin::SPENDING_LIMIT;
    e.impl_kind = IMPL_PROGRAM;
    assert!(
        hook_registry::verify_install(&vkey, &v, &e, Environment::Account).is_err(),
        "different impl kind"
    );
    let mut revoked = v.clone();
    revoked.status = STATUS_DEPRECATED;
    e.impl_kind = IMPL_BUILTIN;
    assert!(
        hook_registry::verify_install(&vkey, &revoked, &e, Environment::Account).is_err(),
        "deprecated versions take no new installs"
    );
}
