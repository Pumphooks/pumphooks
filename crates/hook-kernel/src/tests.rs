use super::builtin::{
    required_caps, AccountingState, AllowlistConfig, Condition, ConditionalConfig, MaxSizeConfig,
    MultisigConfig, SpendingLimitConfig, TimeWindowConfig, TimelockConfig, ACCOUNTING, ALLOWLIST,
    ALLOWLIST_NAMESPACE_TAG, ALLOWLIST_SCHEMA, CONDITIONAL, FIELD_AMOUNT, FIELD_DEP_OUTPUT,
    MAX_SIZE, MULTISIG, OP_GE, OP_GT, ROLE_PREMISE, ROLE_REQUIREMENT, SOURCE_INLINE, SOURCE_RECORD,
    SPENDING_LIMIT, TARGET_COUNTERPARTY, TIMELOCK, TIME_WINDOW,
};
use super::*;
use bytemuck::Zeroable;
use hook_standard::{
    output, state, Environment, HookStack, PolicyResult, StackEntry, IMPL_BUILTIN, IMPL_PROGRAM,
};
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

fn program_entry(callbacks: u16) -> StackEntry {
    let mut e = StackEntry::zeroed();
    e.impl_kind = IMPL_PROGRAM;
    e.program = Pubkey::new_unique().to_bytes();
    e.callbacks = callbacks;
    e.granted = cap::VALIDATE_OPERATION | cap::VETO_OPERATION;
    e
}

fn stack(env: Environment, entries: &[StackEntry]) -> HookStack {
    let mut s = HookStack::empty(env);
    s.len = entries.len() as u8;
    s.entries[..entries.len()].copy_from_slice(entries);
    s
}

const WITHDRAW: u8 = 2; // account before_withdraw
const EXECUTE: u8 = 4; // account before_execute
const W: u16 = 1 << WITHDRAW;

fn limit(asset: Pubkey, limit: u64, period: u32) -> StackEntry {
    entry(
        SPENDING_LIMIT,
        W,
        SpendingLimitConfig {
            asset: asset.to_bytes(),
            limit,
            period_secs: period,
            _pad: 0,
            _rest: [0; 48],
        },
    )
}
fn timelock(delay: u32) -> StackEntry {
    entry(
        TIMELOCK,
        W,
        TimelockConfig {
            delay_secs: delay,
            _pad: 0,
            not_before: 0,
            _rest: [0; 80],
        },
    )
}
fn multisig(threshold: u8, mask: u8) -> StackEntry {
    entry(
        MULTISIG,
        W,
        MultisigConfig {
            threshold,
            member_mask: mask,
            _rest: [0; 94],
        },
    )
}
fn max_size(max: u64) -> StackEntry {
    entry(
        MAX_SIZE,
        W,
        MaxSizeConfig {
            asset: [0; 32],
            max_amount: max,
            _rest: [0; 56],
        },
    )
}

fn run(s: &mut HookStack, cb: u8, op: &Op) -> Result<Outputs, Rejection> {
    evaluate(s, cb, op, &mut NoProgramHooks)
}

// ---------------------------------------------------------------- builtins

#[test]
fn spending_limit_windows() {
    let mut s = stack(Environment::Account, &[limit(Pubkey::default(), 100, 3600)]);
    validate_stack(&s, 1).unwrap();
    let op = |amount, now| Op {
        amount,
        now,
        ..Op::default()
    };
    let out = run(&mut s, WITHDRAW, &op(60, 10_000)).unwrap();
    assert_eq!(out[0].unwrap().kind, output::SPEND_REMAINING);
    assert_eq!(out[0].unwrap().value, 40);
    assert_eq!(
        run(&mut s.clone(), WITHDRAW, &op(41, 10_001))
            .unwrap_err()
            .reason,
        reason::SPENDING_LIMIT
    );
    run(&mut s, WITHDRAW, &op(40, 13_599)).unwrap();
    assert!(run(&mut s.clone(), WITHDRAW, &op(1, 13_599)).is_err());
    // New window.
    run(&mut s, WITHDRAW, &op(100, 13_600)).unwrap();
    // Other assets are not limited by this entry; u64 overflow is a rejection, not a wrap.
    run(
        &mut s.clone(),
        WITHDRAW,
        &Op {
            asset: Pubkey::new_unique(),
            amount: u64::MAX,
            now: 13_600,
            ..Op::default()
        },
    )
    .unwrap();
    assert!(run(&mut s, WITHDRAW, &op(u64::MAX, 13_600)).is_err());
}

#[test]
fn timelock_requires_aged_proposal() {
    let mut s = stack(Environment::Account, &[timelock(86_400)]);
    let direct = Op {
        now: 1_000_000,
        ..Op::default()
    };
    assert_eq!(
        run(&mut s, WITHDRAW, &direct).unwrap_err().reason,
        reason::PROPOSAL_REQUIRED
    );
    let young = Op {
        proposal_created_at: Some(1_000_000 - 86_399),
        ..direct
    };
    assert_eq!(
        run(&mut s, WITHDRAW, &young).unwrap_err().reason,
        reason::TIMELOCK_PENDING
    );
    let old = Op {
        proposal_created_at: Some(1_000_000 - 86_400),
        ..direct
    };
    assert_eq!(
        run(&mut s, WITHDRAW, &old).unwrap()[0].unwrap().kind,
        output::TIMELOCK_SATISFIED
    );
}

#[test]
fn multisig_counts_only_members_in_mask() {
    let mut s = stack(Environment::Account, &[multisig(2, 0b0111)]);
    validate_stack(&s, 3).unwrap();
    assert!(
        validate_stack(&s, 2).is_err(),
        "mask names a member that does not exist"
    );
    assert!(run(
        &mut s,
        WITHDRAW,
        &Op {
            approvals: 0b1001,
            ..Op::default()
        }
    )
    .is_err());
    assert!(run(
        &mut s,
        WITHDRAW,
        &Op {
            approvals: 0b0101,
            ..Op::default()
        }
    )
    .is_ok());
    assert!(validate_stack(&stack(Environment::Account, &[multisig(4, 0b0111)]), 3).is_err());
}

#[test]
fn allowlist_inline_and_records() {
    let alice = Pubkey::new_unique();
    let bob = Pubkey::new_unique();
    let inline = entry(
        ALLOWLIST,
        W,
        AllowlistConfig {
            target: TARGET_COUNTERPARTY,
            source: SOURCE_INLINE,
            _pad: [0; 6],
            max_age_slots: 0,
            key_a: alice.to_bytes(),
            key_b: [0; 32],
            _rest: [0; 16],
        },
    );
    let mut s = stack(Environment::Account, &[inline]);
    validate_stack(&s, 1).unwrap();
    assert!(run(
        &mut s,
        WITHDRAW,
        &Op {
            counterparty: alice,
            ..Op::default()
        }
    )
    .is_ok());
    assert_eq!(
        run(
            &mut s,
            WITHDRAW,
            &Op {
                counterparty: bob,
                ..Op::default()
            }
        )
        .unwrap_err()
        .reason,
        reason::NOT_ALLOWLISTED
    );
    // An all-zero second key must not allowlist the zero address.
    assert!(run(&mut s, WITHDRAW, &Op::default()).is_err());

    let producer = Pubkey::new_unique();
    let list_authority = Pubkey::new_unique();
    let mk = |subject: &Pubkey, issuer: &Pubkey, slot: u64| {
        let mut h = state::StateHeader::zeroed();
        h.magic = state::MAGIC;
        h.version = 1;
        h.schema = ALLOWLIST_SCHEMA;
        h.namespace = state::namespace(ALLOWLIST_NAMESPACE_TAG);
        h.producer = producer.to_bytes();
        h.subject = subject.to_bytes();
        h.issuer = issuer.to_bytes();
        h.slot = slot;
        bytemuck::bytes_of(&h).to_vec()
    };
    let by_record = entry(
        ALLOWLIST,
        W,
        AllowlistConfig {
            target: TARGET_COUNTERPARTY,
            source: SOURCE_RECORD,
            _pad: [0; 6],
            max_age_slots: 100,
            key_a: producer.to_bytes(),
            key_b: list_authority.to_bytes(),
            _rest: [0; 16],
        },
    );
    let mut s = stack(Environment::Account, &[by_record]);
    validate_stack(&s, 1).unwrap();
    let good = mk(&alice, &list_authority, 50);
    let rogue_issuer = mk(&alice, &Pubkey::new_unique(), 50);
    let ok = [Record {
        owner: producer,
        data: &good,
    }];
    assert!(run(
        &mut s,
        WITHDRAW,
        &Op {
            counterparty: alice,
            slot: 120,
            records: &ok,
            ..Op::default()
        }
    )
    .is_ok());
    // Record for someone else, stale record, forged owner, wrong issuer.
    assert!(run(
        &mut s,
        WITHDRAW,
        &Op {
            counterparty: bob,
            slot: 120,
            records: &ok,
            ..Op::default()
        }
    )
    .is_err());
    assert!(run(
        &mut s,
        WITHDRAW,
        &Op {
            counterparty: alice,
            slot: 151,
            records: &ok,
            ..Op::default()
        }
    )
    .is_err());
    let forged = [Record {
        owner: Pubkey::new_unique(),
        data: &good,
    }];
    assert!(run(
        &mut s,
        WITHDRAW,
        &Op {
            counterparty: alice,
            slot: 120,
            records: &forged,
            ..Op::default()
        }
    )
    .is_err());
    let rogue = [Record {
        owner: producer,
        data: &rogue_issuer,
    }];
    assert!(run(
        &mut s,
        WITHDRAW,
        &Op {
            counterparty: alice,
            slot: 120,
            records: &rogue,
            ..Op::default()
        }
    )
    .is_err());
}

#[test]
fn conditional_uses_declared_dependency_outputs_only() {
    // Entry 0: multisig 1-of-2 (emits APPROVALS). Entry 1: IF amount > 1000 THEN approvals-output >= 2.
    let mut cond = ConditionalConfig::zeroed();
    cond.conditions[0] = Condition {
        field: FIELD_AMOUNT,
        op: OP_GT,
        role: ROLE_PREMISE,
        value: 1000,
        ..Default::default()
    };
    cond.conditions[1] = Condition {
        field: FIELD_DEP_OUTPUT,
        op: OP_GE,
        role: ROLE_REQUIREMENT,
        dep_index: 0,
        output_kind: output::APPROVALS,
        value: 2,
        ..Default::default()
    };
    let mut c = entry(CONDITIONAL, W, cond);
    c.depends_on = 0b1;
    let mut s = stack(Environment::Account, &[multisig(1, 0b11), c]);
    validate_stack(&s, 2).unwrap();
    assert!(run(
        &mut s,
        WITHDRAW,
        &Op {
            amount: 1000,
            approvals: 0b01,
            ..Op::default()
        }
    )
    .is_ok());
    assert_eq!(
        run(
            &mut s,
            WITHDRAW,
            &Op {
                amount: 1001,
                approvals: 0b01,
                ..Op::default()
            }
        )
        .unwrap_err(),
        Rejection {
            index: 1,
            reason: reason::CONDITION_FAILED,
            hook_reason: 0
        }
    );
    assert!(run(
        &mut s,
        WITHDRAW,
        &Op {
            amount: 1001,
            approvals: 0b11,
            ..Op::default()
        }
    )
    .is_ok());

    // Undeclared dependency: the output is invisible, so the requirement fails closed.
    s.entries[1].depends_on = 0;
    assert!(run(
        &mut s,
        WITHDRAW,
        &Op {
            amount: 1001,
            approvals: 0b11,
            ..Op::default()
        }
    )
    .is_err());
    // Depending on itself or a later entry is invalid.
    s.entries[1].depends_on = 0b10;
    assert_eq!(validate_stack(&s, 2), Err(StackError::Dependency(1)));
}

#[test]
fn time_window_daily_wraparound() {
    let tw = |s, e| {
        entry(
            TIME_WINDOW,
            W,
            TimeWindowConfig {
                not_before: 0,
                not_after: 0,
                daily_start: s,
                daily_end: e,
                _rest: [0; 72],
            },
        )
    };
    let mut s = stack(Environment::Account, &[tw(22 * 3600, 2 * 3600)]);
    validate_stack(&s, 1).unwrap();
    let at = |h: i64| Op {
        now: 10 * 86_400 + h * 3600,
        ..Op::default()
    };
    assert!(run(&mut s, WITHDRAW, &at(23)).is_ok());
    assert!(run(&mut s, WITHDRAW, &at(1)).is_ok());
    assert_eq!(
        run(&mut s, WITHDRAW, &at(12)).unwrap_err().reason,
        reason::OUTSIDE_WINDOW
    );
    assert!(
        validate_stack(&stack(Environment::Account, &[tw(0, 0)]), 1).is_err(),
        "no constraint at all"
    );
    assert!(validate_stack(&stack(Environment::Account, &[tw(5, 5)]), 1).is_err());
}

#[test]
fn accounting_and_max_size() {
    let mut acct = StackEntry::zeroed();
    acct.impl_kind = IMPL_BUILTIN;
    acct.builtin_id = ACCOUNTING;
    acct.callbacks = W;
    acct.granted = required_caps(ACCOUNTING);
    let mut s = stack(Environment::Account, &[max_size(10), acct]);
    validate_stack(&s, 1).unwrap();
    run(
        &mut s,
        WITHDRAW,
        &Op {
            amount: 7,
            ..Op::default()
        },
    )
    .unwrap();
    let out = run(
        &mut s,
        WITHDRAW,
        &Op {
            amount: 3,
            ..Op::default()
        },
    )
    .unwrap();
    assert_eq!(out[1].unwrap().value, 2);
    let st: AccountingState = bytemuck::pod_read_unaligned(&s.entries[1].state);
    assert_eq!((st.count, st.volume), (2, 10));
    assert_eq!(
        run(
            &mut s,
            WITHDRAW,
            &Op {
                amount: 11,
                ..Op::default()
            }
        )
        .unwrap_err()
        .reason,
        reason::SIZE_CAP
    );
}

// ---------------------------------------------------------------- validation

#[test]
fn validation_rejects_bad_stacks() {
    let ok = max_size(10);
    let check = |e: StackEntry| validate_stack(&stack(Environment::Account, &[e]), 1);
    assert!(check(ok).is_ok());
    assert_eq!(
        check(StackEntry { callbacks: 0, ..ok }),
        Err(StackError::Callbacks(0))
    );
    assert_eq!(
        check(StackEntry {
            callbacks: 1 << 8,
            ..ok
        }),
        Err(StackError::Callbacks(0))
    );
    // max-size is not offered on before_deposit.
    assert_eq!(
        check(StackEntry { callbacks: 1, ..ok }),
        Err(StackError::Callbacks(0))
    );
    assert_eq!(
        check(StackEntry {
            granted: ok.granted | cap::REQUEST_AUTHORIZED_TRANSFER,
            ..ok
        }),
        Err(StackError::Capabilities(0))
    );
    assert_eq!(
        check(StackEntry {
            builtin_id: 99,
            ..ok
        }),
        Err(StackError::UnknownImpl(0))
    );
    assert_eq!(
        check(StackEntry {
            program: [1; 32],
            ..ok
        }),
        Err(StackError::Program(0))
    );
    assert_eq!(
        check(StackEntry { mandatory: 2, ..ok }),
        Err(StackError::Config(0))
    );
    assert_eq!(check(max_size(0)), Err(StackError::Config(0)));

    // Program hooks: no asset-moving or pricing capabilities in policy environments.
    let p = program_entry(W);
    assert!(check(p).is_ok());
    for bad in [
        cap::REQUEST_AUTHORIZED_TRANSFER,
        cap::REQUEST_AUTHORIZED_SWAP,
        cap::REQUEST_AUTHORIZED_FEE_ALLOCATION,
        cap::CUSTOM_PRICING,
    ] {
        assert_eq!(
            check(StackEntry {
                granted: p.granted | bad,
                ..p
            }),
            Err(StackError::Capabilities(0))
        );
    }
    assert_eq!(
        check(StackEntry {
            program: [0; 32],
            ..p
        }),
        Err(StackError::Program(0))
    );
    let three = [program_entry(W), program_entry(W), program_entry(W)];
    assert_eq!(
        validate_stack(&stack(Environment::Account, &three), 1),
        Err(StackError::TooManyProgramHooks(WITHDRAW))
    );

    let mut dup = ok;
    dup.hook_version = [7; 32];
    assert_eq!(
        validate_stack(&stack(Environment::Account, &[dup, dup]), 1),
        Err(StackError::Duplicate(1))
    );
    let mut dirty = stack(Environment::Account, &[ok]);
    dirty.entries[3].config[0] = 1;
    assert_eq!(validate_stack(&dirty, 1), Err(StackError::DirtySlot(3)));
    let mut long = stack(Environment::Account, &[ok]);
    long.len = 9;
    assert_eq!(validate_stack(&long, 1), Err(StackError::TooLong));
    let pool_program = StackEntry {
        callbacks: 1 << 6,
        ..program_entry(W)
    };
    assert_eq!(
        validate_stack(&stack(Environment::Pool, &[pool_program]), 1),
        Err(StackError::UnknownImpl(0))
    );
}

#[test]
fn pool_policies_gate_entry_and_never_withdrawals() {
    use hook_standard::Environment::Pool;
    const BEFORE_REMOVE: u16 = 1 << 4;
    for id in builtin::IDS {
        assert_eq!(builtin::supported_callbacks(id, Pool) & BEFORE_REMOVE, 0, "builtin {id}");
    }
    let s = stack(Pool, &[StackEntry { callbacks: BEFORE_REMOVE, ..max_size(10) }]);
    assert_eq!(validate_stack(&s, 1), Err(StackError::Callbacks(0)));
    let s = stack(Pool, &[StackEntry { callbacks: 1 << 6, ..max_size(10) }]);
    assert_eq!(validate_stack(&s, 1), Ok(()));
}

// ---------------------------------------------------------------- program hooks

struct Scripted(Vec<Result<PolicyResult, ()>>, Vec<PolicyContext>);
impl ProgramHooks for Scripted {
    fn call(&mut self, _: usize, _: &StackEntry, ctx: PolicyContext) -> Result<PolicyResult, ()> {
        self.1.push(ctx);
        self.0.remove(0)
    }
}

#[test]
fn program_hooks_fail_closed_and_receive_dependency_outputs() {
    let mut p = program_entry(W);
    p.depends_on = 0b1;
    let mut s = stack(
        Environment::Account,
        &[limit(Pubkey::default(), 100, 60), p],
    );
    validate_stack(&s, 1).unwrap();
    let op = Op {
        amount: 30,
        now: 10,
        ..Op::default()
    };

    let mut h = Scripted(
        vec![Ok(PolicyResult::allow().with_output(0x8001, 5))],
        vec![],
    );
    let out = evaluate(&mut s.clone(), WITHDRAW, &op, &mut h).unwrap();
    assert_eq!(out[1].unwrap().value, 5);
    let ctx = h.1[0];
    assert_eq!(
        (ctx.inputs[0].kind, ctx.inputs[0].value),
        (output::SPEND_REMAINING, 70)
    );
    assert_eq!(
        (ctx.environment, ctx.callback, ctx.entry_index, ctx.amount),
        (Environment::Account as u8, WITHDRAW, 1, 30)
    );

    let mut h = Scripted(vec![Ok(PolicyResult::reject(42))], vec![]);
    assert_eq!(
        evaluate(&mut s.clone(), WITHDRAW, &op, &mut h).unwrap_err(),
        Rejection {
            index: 1,
            reason: reason::HOOK_REJECTED,
            hook_reason: 42
        }
    );
    let mut h = Scripted(vec![Err(())], vec![]);
    assert_eq!(
        evaluate(&mut s.clone(), WITHDRAW, &op, &mut h)
            .unwrap_err()
            .reason,
        reason::HOOK_FAILED
    );
    assert_eq!(
        evaluate(&mut s, WITHDRAW, &op, &mut NoProgramHooks)
            .unwrap_err()
            .reason,
        reason::HOOK_FAILED
    );
    // Entries not installed on a callback are not called.
    assert!(evaluate(&mut s, EXECUTE, &op, &mut NoProgramHooks).is_ok());
}

// ---------------------------------------------------------------- lifecycle

#[test]
fn mandatory_entries_need_the_guard() {
    let mut m = timelock(3600);
    m.mandatory = 1;
    let o = max_size(10);
    let old = stack(Environment::Account, &[m, o]);
    assert!(
        !requires_guard(&old, &stack(Environment::Account, &[m])),
        "removing optional"
    );
    assert!(
        !requires_guard(
            &old,
            &stack(
                Environment::Account,
                &[m, o, limit(Pubkey::default(), 1, 1)]
            )
        ),
        "adding"
    );
    assert!(
        !requires_guard(&old, &stack(Environment::Account, &[o, m])),
        "mandatory entry kept"
    );
    assert!(
        requires_guard(&old, &stack(Environment::Account, &[o])),
        "removing mandatory"
    );
    assert!(
        requires_guard(
            &old,
            &stack(Environment::Account, &[StackEntry { mandatory: 0, ..m }, o])
        ),
        "demoting"
    );
    assert!(
        requires_guard(&old, &stack(Environment::Account, &[timelock(1), o])),
        "loosening config"
    );
    let mut m2 = m;
    m2.state = [9; 32];
    assert!(
        !requires_guard(&old, &stack(Environment::Account, &[m2, o])),
        "state is not policy"
    );
}

#[test]
fn resubmitting_a_stack_keeps_spending_state() {
    let mut old = stack(Environment::Account, &[limit(Pubkey::default(), 100, 3600)]);
    run(
        &mut old,
        WITHDRAW,
        &Op {
            amount: 90,
            now: 5,
            ..Op::default()
        },
    )
    .unwrap();
    let mut new = stack(
        Environment::Account,
        &[max_size(1000), limit(Pubkey::default(), 100, 3600)],
    );
    new.entries[1].state = [0; 32];
    carry_state(&old, &mut new);
    assert_eq!(
        run(
            &mut new,
            WITHDRAW,
            &Op {
                amount: 11,
                now: 6,
                ..Op::default()
            }
        )
        .unwrap_err()
        .reason,
        reason::SPENDING_LIMIT
    );
}

// ---------------------------------------------------------------- properties

fn arb_entry() -> impl Strategy<Value = StackEntry> {
    prop_oneof![
        (1u64..500, 1u32..100).prop_map(|(l, p)| limit(Pubkey::default(), l, p)),
        (1u32..100).prop_map(timelock),
        (1u8..3).prop_map(|t| multisig(t, 0b11)),
        (1u64..500).prop_map(max_size),
    ]
    .prop_map(|mut e| {
        e.mandatory = 0;
        e
    })
}

fn arb_op() -> impl Strategy<Value = (u64, i64, u8, Option<i64>)> {
    (0u64..600, 0i64..300, 0u8..4, prop::option::of(0i64..300))
}

proptest! {
    /// Adding an entry can only restrict: if S rejects, S+E rejects; if S+E allows, S allows.
    #[test]
    fn composition_is_monotone(base in prop::collection::vec(arb_entry(), 0..6), extra in arb_entry(), pos in 0usize..7, ops in prop::collection::vec(arb_op(), 1..8)) {
        let mut s = stack(Environment::Account, &base);
        let mut with = base.clone();
        with.insert(pos.min(base.len()), extra);
        let mut t = stack(Environment::Account, &with);
        for (amount, now, approvals, created) in ops {
            let op = Op { amount, now, approvals, proposal_created_at: created, ..Op::default() };
            // Evaluate on copies so a rejection does not commit state (as on-chain).
            let mut s2 = s;
            let mut t2 = t;
            let rs = run(&mut s2, WITHDRAW, &op);
            let rt = run(&mut t2, WITHDRAW, &op);
            if rs.is_err() { prop_assert!(rt.is_err()); }
            if rt.is_ok() { prop_assert!(rs.is_ok()); t = t2; s = s2; }
            else if rs.is_ok() {
                // S allowed but S+E did not: keep both in the pre-op state.
            }
        }
    }

    /// Program hooks: any failure or rejection anywhere fails the whole operation.
    #[test]
    fn fail_closed(results in prop::collection::vec(0u8..4, 1..3)) {
        let entries: Vec<StackEntry> = results.iter().map(|_| program_entry(W)).collect();
        let mut s = stack(Environment::Account, &entries);
        let script: Vec<Result<PolicyResult, ()>> = results.iter().map(|r| match r {
            0 => Ok(PolicyResult::allow()),
            1 => Ok(PolicyResult::reject(1)),
            _ => Err(()),
        }).collect();
        let all_allow = results.iter().all(|r| *r == 0);
        let r = evaluate(&mut s, WITHDRAW, &Op::default(), &mut Scripted(script, vec![]));
        prop_assert_eq!(r.is_ok(), all_allow);
    }
}
