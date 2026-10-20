//! ACCOUNT engine: smart accounts with builtin hook stacks, proposals, sessions, recovery, lock,
//! and measured-outflow spending limits on arbitrary CPIs.

mod common;

use anchor_lang::{solana_program::instruction::AccountMeta as Meta, system_program};
use bytemuck::Zeroable;
use common::{registry, tokens::*, *};
use hook_kernel::builtin::{self, MultisigConfig, SpendingLimitConfig, TimelockConfig};
use hook_standard::{Environment, HookStack, StackEntry, IMPL_BUILTIN};
use smart_accounts::{
    accounts as A, action_hash, instruction as I, state::*, Action, CpiMeta, CreateParams,
    Governance, SessionParams,
};
use solana_keypair::Keypair;

const ID: Pk = smart_accounts::ID;
const SYS: Pk = system_program::ID;
const T0: i64 = 1_700_000_000;
const DAY: i64 = 86_400;
const WITHDRAW: u16 = 1 << 2;
const EXECUTE_AFTER: u16 = 1 << 5;
const EXECUTE_BEFORE: u16 = 1 << 4;
const GOVERN: u16 = 1 << 6;

struct Versions {
    spending: Pk,
    timelock: Pk,
    multisig: Pk,
    max_size: Pk,
}

struct T {
    s: Svm,
    _admin: Keypair,
    owner: Keypair,
    v: Versions,
}

fn setup() -> T {
    let mut s = Svm::new();
    s.warp_time(T0);
    let admin = s.funded(1000);
    let owner = s.funded(100);
    registry::setup(&mut s, &admin);
    s.add_program(&ID, "smart_accounts.so");
    let acct = Environment::Account;
    let v = Versions {
        spending: registry::publish_builtin(
            &mut s,
            &admin,
            "spending-limit",
            builtin::SPENDING_LIMIT,
            acct,
            WITHDRAW | EXECUTE_AFTER,
        ),
        timelock: registry::publish_builtin(
            &mut s,
            &admin,
            "timelock",
            builtin::TIMELOCK,
            acct,
            WITHDRAW | EXECUTE_BEFORE | GOVERN,
        ),
        multisig: registry::publish_builtin(
            &mut s,
            &admin,
            "multisig",
            builtin::MULTISIG,
            acct,
            WITHDRAW | EXECUTE_BEFORE | GOVERN,
        ),
        max_size: registry::publish_builtin(
            &mut s,
            &admin,
            "max-size",
            builtin::MAX_SIZE,
            acct,
            WITHDRAW | EXECUTE_AFTER,
        ),
    };
    T {
        s,
        _admin: admin,
        owner,
        v,
    }
}

fn entry<C: bytemuck::Pod>(
    version: &Pk,
    id: u8,
    callbacks: u16,
    cfg: C,
    mandatory: bool,
) -> StackEntry {
    let mut e = StackEntry::zeroed();
    e.hook_version = version.to_bytes();
    e.impl_kind = IMPL_BUILTIN;
    e.builtin_id = id;
    e.callbacks = callbacks;
    e.granted = builtin::required_caps(id);
    e.mandatory = mandatory as u8;
    e.config.copy_from_slice(bytemuck::bytes_of(&cfg));
    e
}

fn sol_limit(t: &T, limit: u64, callbacks: u16, mandatory: bool) -> StackEntry {
    entry(
        &t.v.spending,
        builtin::SPENDING_LIMIT,
        callbacks,
        SpendingLimitConfig {
            asset: [0; 32],
            limit,
            period_secs: DAY as u32,
            _pad: 0,
            _rest: [0; 48],
        },
        mandatory,
    )
}
fn token_limit(t: &T, mint: &Pk, limit: u64) -> StackEntry {
    entry(
        &t.v.spending,
        builtin::SPENDING_LIMIT,
        EXECUTE_AFTER,
        SpendingLimitConfig {
            asset: mint.to_bytes(),
            limit,
            period_secs: DAY as u32,
            _pad: 0,
            _rest: [0; 48],
        },
        true,
    )
}
fn timelock(t: &T, secs: u32, callbacks: u16) -> StackEntry {
    entry(
        &t.v.timelock,
        builtin::TIMELOCK,
        callbacks,
        TimelockConfig {
            delay_secs: secs,
            _pad: 0,
            not_before: 0,
            _rest: [0; 80],
        },
        true,
    )
}
fn multisig(t: &T, threshold: u8, mask: u8, callbacks: u16) -> StackEntry {
    entry(
        &t.v.multisig,
        builtin::MULTISIG,
        callbacks,
        MultisigConfig {
            threshold,
            member_mask: mask,
            _rest: [0; 94],
        },
        true,
    )
}

fn stack(entries: &[StackEntry]) -> Vec<u8> {
    let mut s = HookStack::empty(Environment::Account);
    s.len = entries.len() as u8;
    s.entries[..entries.len()].copy_from_slice(entries);
    bytemuck::bytes_of(&s).to_vec()
}

fn versions_of(entries: &[StackEntry]) -> Vec<Meta> {
    let mut v: Vec<Pk> = entries
        .iter()
        .map(|e| Pk::new_from_array(e.hook_version))
        .collect();
    v.dedup();
    v.into_iter()
        .map(|k| Meta::new_readonly(k, false))
        .collect()
}

fn account_pda(payer: &Pk, seed: u64) -> Pk {
    Pk::find_program_address(&[ACCOUNT_SEED, payer.as_ref(), &seed.to_le_bytes()], &ID).0
}
fn vault_pda(account: &Pk) -> Pk {
    Pk::find_program_address(&[VAULT_SEED, account.as_ref()], &ID).0
}
fn proposal_pda(account: &Pk, index: u64) -> Pk {
    Pk::find_program_address(
        &[PROPOSAL_SEED, account.as_ref(), &index.to_le_bytes()],
        &ID,
    )
    .0
}
fn session_pda(account: &Pk, key: &Pk) -> Pk {
    Pk::find_program_address(&[SESSION_SEED, account.as_ref(), key.as_ref()], &ID).0
}

struct Acct {
    key: Pk,
    vault: Pk,
}

fn create(
    t: &mut T,
    recovery: Option<Pk>,
    members: Vec<Pk>,
    entries: &[StackEntry],
    policy_delay: i64,
) -> Acct {
    let owner = kp(&t.owner);
    let key = account_pda(&owner, 1);
    let vault = vault_pda(&key);
    let p = CreateParams {
        seed: 1,
        owner,
        recovery: recovery.unwrap_or_default(),
        recovery_delay: if recovery.is_some() { 2 * DAY } else { 0 },
        policy_delay,
        members,
        stack: stack(entries),
    };
    let ixn = ix(
        ID,
        A::CreateAccount {
            payer: owner,
            account: key,
            vault,
            system_program: SYS,
        },
        versions_of(entries),
        I::CreateAccount { p },
    );
    t.s.send_ok(&[ixn], &[&t.owner]);
    Acct { key, vault }
}

fn deposit_sol(t: &mut T, a: &Acct, amount: u64) {
    let d = t.s.funded(amount / SOL + 2);
    let ixn = ix(
        ID,
        A::Deposit {
            depositor: kp(&d),
            account: a.key,
            vault: a.vault,
            mint: None,
            depositor_token: None,
            vault_token: None,
            token_program: None,
            system_program: SYS,
        },
        vec![],
        I::Deposit { amount },
    );
    t.s.send_ok(&[ixn], &[&d]);
}

fn withdraw(
    authority: &Pk,
    a: &Acct,
    dest: &Pk,
    amount: u64,
    session: Option<Pk>,
    proposal: Option<Pk>,
    extra: Vec<Meta>,
) -> solana_instruction::Instruction {
    ix(
        ID,
        A::Withdraw {
            authority: *authority,
            account: a.key,
            vault: a.vault,
            destination: *dest,
            session: Some(session.unwrap_or(ID)),
            proposal: Some(proposal.unwrap_or(ID)),
            mint: None,
            vault_token: None,
            token_program: None,
            system_program: SYS,
        },
        extra,
        I::Withdraw { amount },
    )
}

fn execute(
    authority: &Pk,
    a: &Acct,
    cpi: &anchor_lang::solana_program::instruction::Instruction,
    proposal: Option<Pk>,
    session: Option<Pk>,
) -> solana_instruction::Instruction {
    let metas: Vec<CpiMeta> = cpi
        .accounts
        .iter()
        .map(|m| CpiMeta {
            pubkey: m.pubkey,
            is_signer: m.is_signer,
            is_writable: m.is_writable,
        })
        .collect();
    let mut extra: Vec<Meta> = cpi
        .accounts
        .iter()
        .map(|m| Meta {
            pubkey: m.pubkey,
            is_signer: false,
            is_writable: m.is_writable,
        })
        .collect();
    extra.push(Meta::new_readonly(cpi.program_id, false));
    ix(
        ID,
        A::Execute {
            authority: *authority,
            account: a.key,
            vault: a.vault,
            session: Some(session.unwrap_or(ID)),
            proposal: Some(proposal.unwrap_or(ID)),
        },
        extra,
        I::Execute {
            program: cpi.program_id,
            accounts: metas,
            data: cpi.data.clone(),
        },
    )
}

fn govern(
    authority: &Pk,
    a: &Acct,
    g: Governance,
    proposal: Option<Pk>,
    extra: Vec<Meta>,
) -> solana_instruction::Instruction {
    ix(
        ID,
        A::Govern {
            authority: *authority,
            account: a.key,
            proposal: Some(proposal.unwrap_or(ID)),
        },
        extra,
        I::Govern { g },
    )
}

fn propose(t: &mut T, proposer: &Keypair, a: &Acct, action: &Action) -> Pk {
    let count = proposal_count(t, a);
    let p = proposal_pda(&a.key, count);
    let ixn = ix(
        ID,
        A::Propose {
            proposer: kp(proposer),
            account: a.key,
            proposal: p,
            system_program: SYS,
        },
        vec![],
        I::Propose {
            action_hash: action_hash(action),
        },
    );
    t.s.send_ok(&[ixn], &[proposer]);
    p
}

fn proposal_count(t: &T, a: &Acct) -> u64 {
    state(t, a).proposal_count
}

fn state(t: &T, a: &Acct) -> SmartAccount {
    let acc = t.s.svm.get_account(&addr(&a.key)).unwrap();
    *bytemuck::from_bytes::<SmartAccount>(&acc.data[8..8 + core::mem::size_of::<SmartAccount>()])
}

fn approve(member: &Pk, a: &Acct, proposal: &Pk) -> solana_instruction::Instruction {
    ix(
        ID,
        A::Approve {
            member: *member,
            account: a.key,
            proposal: *proposal,
        },
        vec![],
        I::Approve {},
    )
}

fn lock(authority: &Pk, a: &Acct) -> solana_instruction::Instruction {
    ix(
        ID,
        A::Lock {
            authority: *authority,
            account: a.key,
        },
        vec![],
        I::Lock {},
    )
}

fn signer_meta(k: &Pk) -> Meta {
    Meta::new_readonly(*k, true)
}

// ---------------------------------------------------------------- withdraw + spending limit

#[test]
fn spending_limit_on_withdraw() {
    let mut t = setup();
    let e = sol_limit(&t, 2 * SOL, WITHDRAW, true);
    let a = create(&mut t, None, vec![], &[e], DAY);
    deposit_sol(&mut t, &a, 10 * SOL);
    let owner = kp(&t.owner);
    let dest = Pk::new_unique();

    t.s.send_ok(
        &[withdraw(
            &owner,
            &a,
            &dest,
            SOL + SOL / 2,
            None,
            None,
            vec![],
        )],
        &[&t.owner],
    );
    let e = t.s.send_err(
        &[withdraw(&owner, &a, &dest, SOL, None, None, vec![])],
        &[&t.owner],
    );
    assert!(
        e.contains("PolicyRejected") && e.contains("spending limit"),
        "{e}"
    );
    assert_eq!(t.s.lamports(&dest), SOL + SOL / 2);

    // A stranger cannot withdraw at all.
    let mallory = t.s.funded(1);
    let e = t.s.send_err(
        &[withdraw(
            &kp(&mallory),
            &a,
            &kp(&mallory),
            1,
            None,
            None,
            vec![],
        )],
        &[&mallory],
    );
    assert!(e.contains("Unauthorized"), "{e}");

    t.s.warp_time(T0 + DAY);
    t.s.send_ok(
        &[withdraw(&owner, &a, &dest, 2 * SOL, None, None, vec![])],
        &[&t.owner],
    );
}

// ---------------------------------------------------------------- execute: measured outflows

#[test]
fn execute_counts_measured_outflows_against_limits() {
    let mut t = setup();
    let e = sol_limit(&t, SOL, EXECUTE_AFTER, true);
    let a = create(&mut t, None, vec![], &[e], DAY);
    deposit_sol(&mut t, &a, 5 * SOL);
    let owner = kp(&t.owner);
    let dest = Pk::new_unique();

    // The vault signs an arbitrary system transfer; the after_execute spending limit sees it.
    let small = anchor_lang::solana_program::system_instruction::transfer(&a.vault, &dest, SOL / 2);
    t.s.send_ok(&[execute(&owner, &a, &small, None, None)], &[&t.owner]);
    let big =
        anchor_lang::solana_program::system_instruction::transfer(&a.vault, &dest, SOL / 2 + 1);
    let e =
        t.s.send_err(&[execute(&owner, &a, &big, None, None)], &[&t.owner]);
    assert!(e.contains("spending limit"), "{e}");

    // Reassigning the vault to another program is a side channel.
    let assign =
        anchor_lang::solana_program::system_instruction::assign(&a.vault, &Pk::new_unique());
    let e =
        t.s.send_err(&[execute(&owner, &a, &assign, None, None)], &[&t.owner]);
    assert!(e.contains("SideChannel"), "{e}");
}

#[test]
fn execute_blocks_token_side_channels_and_limits_tokens() {
    let mut t = setup();
    let owner_kp = t.owner.insecure_clone();
    let mint = t.s.create_mint(&owner_kp, 6);
    let e = token_limit(&t, &mint, 1_000);
    let a = create(&mut t, None, vec![], &[e], DAY);
    let vault_token = t.s.create_token_account(&owner_kp, &mint, &a.vault);
    let other =
        t.s.create_token_account(&owner_kp, &mint, &Pk::new_unique());
    t.s.mint_to(&owner_kp, &mint, &vault_token, 10_000);
    let owner = kp(&t.owner);

    t.s.send_ok(
        &[execute(
            &owner,
            &a,
            &transfer_ix(&vault_token, &other, &a.vault, 600),
            None,
            None,
        )],
        &[&t.owner],
    );
    let e = t.s.send_err(
        &[execute(
            &owner,
            &a,
            &transfer_ix(&vault_token, &other, &a.vault, 401),
            None,
            None,
        )],
        &[&t.owner],
    );
    assert!(e.contains("spending limit"), "{e}");

    // Approving a delegate moves nothing now but would allow a later drain: rejected.
    let e = t.s.send_err(
        &[execute(
            &owner,
            &a,
            &approve_ix(&vault_token, &Pk::new_unique(), &a.vault, u64::MAX),
            None,
            None,
        )],
        &[&t.owner],
    );
    assert!(e.contains("SideChannel"), "{e}");
    assert_eq!(t.s.token_balance(&vault_token), 9_400);
}

#[test]
fn execute_rejects_protected_accounts_self_cpi_and_substitution() {
    let mut t = setup();
    let a = create(&mut t, None, vec![], &[], DAY);
    deposit_sol(&mut t, &a, SOL);
    let owner = kp(&t.owner);

    // Passing the smart account's own state to the executed program.
    let cpi = anchor_lang::solana_program::instruction::Instruction {
        program_id: SYS,
        accounts: vec![Meta::new(a.key, false)],
        data: vec![],
    };
    assert!(t
        .s
        .send_err(&[execute(&owner, &a, &cpi, None, None)], &[&t.owner])
        .contains("ProtectedAccount"));

    // Calling back into smart-accounts.
    let cpi = anchor_lang::solana_program::instruction::Instruction {
        program_id: ID,
        accounts: vec![],
        data: vec![],
    };
    assert!(t
        .s
        .send_err(&[execute(&owner, &a, &cpi, None, None)], &[&t.owner])
        .contains("ProtectedAccount"));

    // Declared metas that differ from the accounts actually supplied.
    let cpi =
        anchor_lang::solana_program::system_instruction::transfer(&a.vault, &Pk::new_unique(), 1);
    let mut ixn = execute(&owner, &a, &cpi, None, None);
    let n = ixn.accounts.len();
    ixn.accounts[n - 2].pubkey = addr(&Pk::new_unique());
    assert!(t
        .s
        .send_err(&[ixn], &[&t.owner])
        .contains("AccountMismatch"));
}

// ---------------------------------------------------------------- timelock via proposals

#[test]
fn timelocked_withdrawals_need_aged_proposals_and_cannot_be_replayed() {
    let mut t = setup();
    let e = timelock(&t, DAY as u32, WITHDRAW);
    let a = create(&mut t, None, vec![], &[e], DAY);
    deposit_sol(&mut t, &a, 5 * SOL);
    let owner = kp(&t.owner);
    let owner_kp = t.owner.insecure_clone();
    let dest = Pk::new_unique();

    let e = t.s.send_err(
        &[withdraw(&owner, &a, &dest, SOL, None, None, vec![])],
        &[&t.owner],
    );
    assert!(e.contains("timelocked proposal"), "{e}");

    let action = Action::Withdraw {
        asset: Pk::default(),
        amount: SOL,
        destination: dest,
    };
    let p = propose(&mut t, &owner_kp, &a, &action);
    t.s.warp_time(T0 + DAY - 1);
    assert!(t
        .s
        .send_err(
            &[withdraw(&owner, &a, &dest, SOL, None, Some(p), vec![])],
            &[&t.owner]
        )
        .contains("timelock not elapsed"));
    t.s.warp_time(T0 + DAY);
    // Substituted destination or amount: the action hash no longer matches.
    let other = Pk::new_unique();
    assert!(t
        .s
        .send_err(
            &[withdraw(&owner, &a, &other, SOL, None, Some(p), vec![])],
            &[&t.owner]
        )
        .contains("ActionMismatch"));
    assert!(t
        .s
        .send_err(
            &[withdraw(&owner, &a, &dest, 2 * SOL, None, Some(p), vec![])],
            &[&t.owner]
        )
        .contains("ActionMismatch"));
    t.s.send_ok(
        &[withdraw(&owner, &a, &dest, SOL, None, Some(p), vec![])],
        &[&t.owner],
    );
    assert_eq!(t.s.lamports(&dest), SOL);
    // Replay.
    assert!(t
        .s
        .send_err(
            &[withdraw(&owner, &a, &dest, SOL, None, Some(p), vec![])],
            &[&t.owner]
        )
        .contains("StaleProposal"));
}

// ---------------------------------------------------------------- multisig

#[test]
fn multisig_counts_signers_and_proposal_approvals() {
    let mut t = setup();
    let m1 = t.s.funded(1);
    let m2 = t.s.funded(1);
    let m3 = t.s.funded(1);
    let e = multisig(&t, 2, 0b111, WITHDRAW);
    let a = create(&mut t, None, vec![kp(&m1), kp(&m2), kp(&m3)], &[e], DAY);
    deposit_sol(&mut t, &a, 5 * SOL);
    let owner = kp(&t.owner);
    let dest = Pk::new_unique();

    let e = t.s.send_err(
        &[withdraw(
            &owner,
            &a,
            &dest,
            SOL,
            None,
            None,
            vec![signer_meta(&kp(&m1))],
        )],
        &[&t.owner, &m1],
    );
    assert!(e.contains("not enough approvals"), "{e}");
    // A non-signing member listed as an account does not count.
    let e = t.s.send_err(
        &[withdraw(
            &owner,
            &a,
            &dest,
            SOL,
            None,
            None,
            vec![signer_meta(&kp(&m1)), Meta::new_readonly(kp(&m2), false)],
        )],
        &[&t.owner, &m1],
    );
    assert!(e.contains("not enough approvals"), "{e}");
    t.s.send_ok(
        &[withdraw(
            &owner,
            &a,
            &dest,
            SOL,
            None,
            None,
            vec![signer_meta(&kp(&m1)), signer_meta(&kp(&m3))],
        )],
        &[&t.owner, &m1, &m3],
    );

    // Asynchronous approvals on a proposal.
    let action = Action::Withdraw {
        asset: Pk::default(),
        amount: SOL,
        destination: dest,
    };
    let p = propose(&mut t, &m2, &a, &action);
    let e = t.s.send_err(
        &[withdraw(&owner, &a, &dest, SOL, None, Some(p), vec![])],
        &[&t.owner],
    );
    assert!(e.contains("not enough approvals"), "{e}");
    let outsider = t.s.funded(1);
    assert!(t
        .s
        .send_err(&[approve(&kp(&outsider), &a, &p)], &[&outsider])
        .contains("Unauthorized"));
    t.s.send_ok(&[approve(&kp(&m1), &a, &p)], &[&m1]);
    t.s.send_ok(
        &[withdraw(&owner, &a, &dest, SOL, None, Some(p), vec![])],
        &[&t.owner],
    );
    assert_eq!(t.s.lamports(&dest), 2 * SOL);
}

// ---------------------------------------------------------------- sessions

#[test]
fn sessions_are_scoped_expire_and_die_with_the_epoch() {
    let mut t = setup();
    let e = sol_limit(&t, 3 * SOL, WITHDRAW, true);
    let a = create(&mut t, None, vec![], &[e], DAY);
    deposit_sol(&mut t, &a, 10 * SOL);
    let owner = kp(&t.owner);
    let key = t.s.funded(1);
    let session = session_pda(&a.key, &kp(&key));
    let p = SessionParams {
        key: kp(&key),
        expires_at: T0 + 3600,
        ops: SESSION_WITHDRAW,
        asset: Pk::default(),
        max_amount: SOL,
        target_program: Pk::default(),
    };
    let create_s = ix(
        ID,
        A::CreateSession {
            owner,
            account: a.key,
            session,
            system_program: SYS,
        },
        vec![],
        I::CreateSession { p },
    );
    t.s.send_ok(&[create_s], &[&t.owner]);
    let dest = Pk::new_unique();
    let sk = kp(&key);

    t.s.send_ok(
        &[withdraw(&sk, &a, &dest, SOL, Some(session), None, vec![])],
        &[&key],
    );
    assert!(t
        .s
        .send_err(
            &[withdraw(
                &sk,
                &a,
                &dest,
                SOL + 1,
                Some(session),
                None,
                vec![]
            )],
            &[&key]
        )
        .contains("SessionScope"));
    // The stack still applies to sessions.
    t.s.send_ok(
        &[withdraw(&sk, &a, &dest, SOL, Some(session), None, vec![])],
        &[&key],
    );
    t.s.send_ok(
        &[withdraw(&sk, &a, &dest, SOL, Some(session), None, vec![])],
        &[&key],
    );
    assert!(t
        .s
        .send_err(
            &[withdraw(&sk, &a, &dest, 1, Some(session), None, vec![])],
            &[&key]
        )
        .contains("spending limit"));
    // Execute is outside a withdraw-only session.
    let cpi = anchor_lang::solana_program::system_instruction::transfer(&a.vault, &dest, 1);
    assert!(t
        .s
        .send_err(&[execute(&sk, &a, &cpi, None, Some(session))], &[&key])
        .contains("SessionScope"));

    t.s.warp_time(T0 + DAY);
    assert!(t
        .s
        .send_err(
            &[withdraw(&sk, &a, &dest, 1, Some(session), None, vec![])],
            &[&key]
        )
        .contains("SessionExpired"));

    // A fresh session is voided by an emergency lock (epoch bump), even after unlocking.
    let key2 = t.s.funded(1);
    let session2 = session_pda(&a.key, &kp(&key2));
    let p2 = SessionParams {
        key: kp(&key2),
        expires_at: T0 + DAY + 3600,
        ops: SESSION_WITHDRAW,
        asset: Pk::default(),
        max_amount: SOL,
        target_program: Pk::default(),
    };
    t.s.send_ok(
        &[ix(
            ID,
            A::CreateSession {
                owner,
                account: a.key,
                session: session2,
                system_program: SYS,
            },
            vec![],
            I::CreateSession { p: p2 },
        )],
        &[&t.owner],
    );
    t.s.send_ok(&[lock(&owner, &a)], &[&t.owner]);
    assert!(t
        .s
        .send_err(
            &[withdraw(
                &kp(&key2),
                &a,
                &dest,
                1,
                Some(session2),
                None,
                vec![]
            )],
            &[&key2]
        )
        .contains("Locked"));
}

// ---------------------------------------------------------------- governance

#[test]
fn mandatory_policy_cannot_be_removed_without_an_aged_proposal() {
    let mut t = setup();
    let limit = sol_limit(&t, SOL, WITHDRAW, true);
    let a = create(&mut t, None, vec![], &[limit], 3 * DAY);
    let owner = kp(&t.owner);
    let owner_kp = t.owner.insecure_clone();

    // Removing the mandatory spending limit directly.
    let g = Governance::SetPolicy { stack: stack(&[]) };
    let e =
        t.s.send_err(&[govern(&owner, &a, g.clone(), None, vec![])], &[&t.owner]);
    assert!(e.contains("ProposalRequired"), "{e}");
    // Loosening its config is also guarded.
    let looser = sol_limit(&t, 100 * SOL, WITHDRAW, true);
    let e = t.s.send_err(
        &[govern(
            &owner,
            &a,
            Governance::SetPolicy {
                stack: stack(&[looser]),
            },
            None,
            versions_of(&[looser]),
        )],
        &[&t.owner],
    );
    assert!(e.contains("ProposalRequired"), "{e}");

    // Adding a restriction is not.
    let max = entry(
        &t.v.max_size,
        builtin::MAX_SIZE,
        WITHDRAW,
        builtin::MaxSizeConfig {
            asset: [0; 32],
            max_amount: SOL / 2,
            _rest: [0; 56],
        },
        false,
    );
    t.s.send_ok(
        &[govern(
            &owner,
            &a,
            Governance::SetPolicy {
                stack: stack(&[limit, max]),
            },
            None,
            versions_of(&[limit, max]),
        )],
        &[&t.owner],
    );
    assert_eq!(state(&t, &a).stack.len, 2);

    // An unregistered version is refused.
    let mut rogue = max;
    rogue.hook_version = Pk::new_unique().to_bytes();
    let e = t.s.send_err(
        &[govern(
            &owner,
            &a,
            Governance::SetPolicy {
                stack: stack(&[limit, rogue]),
            },
            None,
            vec![],
        )],
        &[&t.owner],
    );
    assert!(e.contains("MissingVersion"), "{e}");

    // Through a proposal, after the policy delay.
    let p = propose(
        &mut t,
        &owner_kp,
        &a,
        &smart_accounts::Action::Govern(g.clone()),
    );
    t.s.warp_time(T0 + 3 * DAY - 1);
    assert!(t
        .s
        .send_err(
            &[govern(&owner, &a, g.clone(), Some(p), vec![])],
            &[&t.owner]
        )
        .contains("TimelockPending"));
    t.s.warp_time(T0 + 3 * DAY);
    t.s.send_ok(&[govern(&owner, &a, g, Some(p), vec![])], &[&t.owner]);
    assert_eq!(state(&t, &a).stack.len, 0);
}

#[test]
fn resubmitting_the_stack_does_not_reset_spending() {
    let mut t = setup();
    let limit = sol_limit(&t, 2 * SOL, WITHDRAW, true);
    let a = create(&mut t, None, vec![], &[limit], DAY);
    deposit_sol(&mut t, &a, 10 * SOL);
    let owner = kp(&t.owner);
    let dest = Pk::new_unique();
    t.s.send_ok(
        &[withdraw(&owner, &a, &dest, 2 * SOL, None, None, vec![])],
        &[&t.owner],
    );
    t.s.send_ok(
        &[govern(
            &owner,
            &a,
            Governance::SetPolicy {
                stack: stack(&[limit]),
            },
            None,
            versions_of(&[limit]),
        )],
        &[&t.owner],
    );
    assert!(t
        .s
        .send_err(
            &[withdraw(&owner, &a, &dest, 1, None, None, vec![])],
            &[&t.owner]
        )
        .contains("spending limit"));
}

#[test]
fn governance_hooks_apply_to_owner_changes() {
    let mut t = setup();
    let m1 = t.s.funded(1);
    let m2 = t.s.funded(1);
    let ms = multisig(&t, 2, 0b11, GOVERN);
    let a = create(&mut t, None, vec![kp(&m1), kp(&m2)], &[ms], 0);
    let owner = kp(&t.owner);
    let new_owner = Pk::new_unique();
    let g = Governance::SetOwner { owner: new_owner };
    let owner_kp = t.owner.insecure_clone();
    let p = propose(&mut t, &owner_kp, &a, &Action::Govern(g.clone()));
    let e = t.s.send_err(
        &[govern(
            &owner,
            &a,
            g.clone(),
            Some(p),
            vec![signer_meta(&kp(&m1))],
        )],
        &[&t.owner, &m1],
    );
    assert!(e.contains("not enough approvals"), "{e}");
    t.s.send_ok(
        &[govern(
            &owner,
            &a,
            g,
            Some(p),
            vec![signer_meta(&kp(&m1)), signer_meta(&kp(&m2))],
        )],
        &[&t.owner, &m1, &m2],
    );
    assert_eq!(state(&t, &a).owner, new_owner);
}

// ---------------------------------------------------------------- recovery and lock

#[test]
fn recovery_replaces_owner_only_after_its_delay_and_owner_can_cancel() {
    let mut t = setup();
    let recovery = t.s.funded(10);
    let ms = multisig(&t, 1, 0b1, GOVERN);
    let m = t.s.funded(1);
    let a = create(&mut t, Some(kp(&recovery)), vec![kp(&m)], &[ms], DAY);
    let owner = kp(&t.owner);
    let rk = kp(&recovery);
    let new_owner = Pk::new_unique();
    let g = Governance::SetOwner { owner: new_owner };

    // Without a proposal, or before the recovery delay.
    assert!(t
        .s
        .send_err(&[govern(&rk, &a, g.clone(), None, vec![])], &[&recovery])
        .contains("Unauthorized"));
    let p = propose(&mut t, &recovery, &a, &Action::Govern(g.clone()));
    t.s.warp_time(T0 + 2 * DAY - 1);
    assert!(t
        .s
        .send_err(&[govern(&rk, &a, g.clone(), Some(p), vec![])], &[&recovery])
        .contains("TimelockPending"));

    // The owner cancels (rent returns to the proposer).
    let cancel = ix(
        ID,
        A::Cancel {
            signer: owner,
            account: a.key,
            proposal: p,
            rent_payer: rk,
        },
        vec![],
        I::Cancel {},
    );
    t.s.send_ok(&[cancel], &[&t.owner]);
    assert!(!t.s.exists(&p));

    // Recovery may not change policy, even after its delay.
    let g2 = Governance::SetPolicy { stack: stack(&[]) };
    let p2 = propose(&mut t, &recovery, &a, &Action::Govern(g2.clone()));
    t.s.warp_time(T0 + 10 * DAY);
    assert!(t
        .s
        .send_err(&[govern(&rk, &a, g2, Some(p2), vec![])], &[&recovery])
        .contains("Unauthorized"));

    // A new recovery proposal, after its delay, succeeds without the members (bypasses the stack).
    let p3 = propose(&mut t, &recovery, &a, &Action::Govern(g.clone()));
    t.s.warp_time(T0 + 12 * DAY);
    t.s.send_ok(&[govern(&rk, &a, g, Some(p3), vec![])], &[&recovery]);
    assert_eq!(state(&t, &a).owner, new_owner);
}

#[test]
fn lock_blocks_outflows_and_unlock_needs_recovery_or_delay() {
    let mut t = setup();
    let recovery = t.s.funded(10);
    let a = create(&mut t, Some(kp(&recovery)), vec![], &[], 3 * DAY);
    deposit_sol(&mut t, &a, 2 * SOL);
    let owner = kp(&t.owner);
    let dest = Pk::new_unique();

    let mallory = t.s.funded(1);
    assert!(t
        .s
        .send_err(&[lock(&kp(&mallory), &a)], &[&mallory])
        .contains("Unauthorized"));
    t.s.send_ok(&[lock(&kp(&recovery), &a)], &[&recovery]);
    assert!(t
        .s
        .send_err(
            &[withdraw(&owner, &a, &dest, 1, None, None, vec![])],
            &[&t.owner]
        )
        .contains("Locked"));
    let cpi = anchor_lang::solana_program::system_instruction::transfer(&a.vault, &dest, 1);
    assert!(t
        .s
        .send_err(&[execute(&owner, &a, &cpi, None, None)], &[&t.owner])
        .contains("Locked"));
    deposit_sol(&mut t, &a, SOL);

    // The owner alone (e.g. a stolen owner key) cannot unlock immediately.
    assert!(t
        .s
        .send_err(
            &[govern(&owner, &a, Governance::Unlock, None, vec![])],
            &[&t.owner]
        )
        .contains("ProposalRequired"));
    t.s.send_ok(
        &[govern(
            &owner,
            &a,
            Governance::Unlock,
            None,
            vec![signer_meta(&kp(&recovery))],
        )],
        &[&t.owner, &recovery],
    );
    t.s.send_ok(
        &[withdraw(&owner, &a, &dest, SOL, None, None, vec![])],
        &[&t.owner],
    );
}

#[test]
fn proposals_go_stale_on_governance_changes() {
    let mut t = setup();
    let e = timelock(&t, 3600, WITHDRAW);
    let a = create(&mut t, None, vec![], &[e], 0);
    deposit_sol(&mut t, &a, 2 * SOL);
    let owner = kp(&t.owner);
    let owner_kp = t.owner.insecure_clone();
    let dest = Pk::new_unique();
    let p = propose(
        &mut t,
        &owner_kp,
        &a,
        &Action::Withdraw {
            asset: Pk::default(),
            amount: SOL,
            destination: dest,
        },
    );
    t.s.send_ok(
        &[govern(
            &owner,
            &a,
            Governance::SetPolicyDelay { delay: 10 },
            None,
            vec![],
        )],
        &[&t.owner],
    );
    t.s.warp_time(T0 + 7200);
    assert!(t
        .s
        .send_err(
            &[withdraw(&owner, &a, &dest, SOL, None, Some(p), vec![])],
            &[&t.owner]
        )
        .contains("StaleProposal"));
}
