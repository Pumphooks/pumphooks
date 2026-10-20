//! Cross-engine shared state: one `hook-state` record, written by an issuer, verified by two
//! engines (ACCOUNT: smart-account withdrawal; ROUTER: PumpSwap route) in the same transaction.
//! Shows that the transaction is atomic across engines (a revocation between the two steps undoes
//! both) and that consumers authenticate the record by owner and issuer, not by its contents.

mod common;

use anchor_lang::{solana_program::instruction::AccountMeta as Meta, system_program};
use bytemuck::Zeroable;
use common::{pump::*, registry, *};
use hook_kernel::builtin::{self, AllowlistConfig};
use hook_standard::{state, Environment, HookStack, StackEntry, IMPL_BUILTIN};
use solana_instruction::{AccountMeta as M, Instruction};
use solana_keypair::Keypair;

const ROUTER: Pk = hook_router::ID;
const ACCOUNTS: Pk = smart_accounts::ID;
const STATE: Pk = hook_state::ID;
const SYS: Pk = system_program::ID;
const ROUTER_BEFORE_SWAP: u16 = 1 << 2;
const ACCOUNT_BEFORE_WITHDRAW: u16 = 1 << 2;
/// `hook_kernel::reason::NOT_ALLOWLISTED` as the engines log it.
const DENIED: &str = "entry 0 reason 2 (";

fn ns() -> [u8; 32] {
    state::namespace(builtin::ALLOWLIST_NAMESPACE_TAG)
}

fn publish(issuer: &Pk, subject: &Pk) -> Instruction {
    let rec = hook_state::record_pda(issuer, &ns(), subject).0;
    Instruction {
        program_id: addr(&STATE),
        accounts: vec![M::new(addr(issuer), true), M::new(addr(&rec), false), M::new_readonly(addr(&SYS), false)],
        data: hook_state::publish_data(&ns(), builtin::ALLOWLIST_SCHEMA, subject, &[]),
    }
}
fn revoke(issuer: &Pk, subject: &Pk) -> Instruction {
    let rec = hook_state::record_pda(issuer, &ns(), subject).0;
    Instruction { program_id: addr(&STATE), accounts: vec![M::new_readonly(addr(issuer), true), M::new(addr(&rec), false)], data: hook_state::revoke_data(&ns(), subject) }
}

fn allowlist(version: &Pk, target: u8, issuer: &Pk, callbacks: u16) -> StackEntry {
    let c = AllowlistConfig { target, source: builtin::SOURCE_RECORD, _pad: [0; 6], max_age_slots: 0, key_a: STATE.to_bytes(), key_b: issuer.to_bytes(), _rest: [0; 16] };
    let mut e = StackEntry::zeroed();
    e.hook_version = version.to_bytes();
    e.impl_kind = IMPL_BUILTIN;
    e.builtin_id = builtin::ALLOWLIST;
    e.callbacks = callbacks;
    e.granted = builtin::required_caps(builtin::ALLOWLIST);
    e.mandatory = 1;
    e.config[..core::mem::size_of::<AllowlistConfig>()].copy_from_slice(bytemuck::bytes_of(&c));
    e
}
fn compact(env: Environment, e: StackEntry) -> Vec<u8> {
    let mut s = HookStack::empty(env);
    s.len = 1;
    s.entries[0] = e;
    s.compact().to_vec()
}

struct World {
    s: Svm,
    venue: PumpSwap,
    owner: Keypair,
    issuer: Keypair,
    account: Pk,
    vault: Pk,
    policy: Pk,
    nonce: u64,
}

fn setup() -> World {
    let mut s = Svm::new();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let mut clock: solana_clock::Clock = s.svm.get_sysvar();
    clock.unix_timestamp = now;
    clock.slot = 400_000_000;
    s.svm.set_sysvar(&clock);
    let admin = s.funded(1000);
    let owner = s.funded(100);
    let issuer = s.funded(10);
    registry::setup(&mut s, &admin);
    s.add_program(&ROUTER, "hook_router.so");
    s.set_upgrade_authority(&ROUTER, Some(kp(&admin)));
    s.add_program(&ACCOUNTS, "smart_accounts.so");
    s.add_program(&STATE, "hook_state.so");
    let venue = PumpSwap::load(&mut s);

    let config = Pk::find_program_address(&[hook_router::CONFIG_SEED], &ROUTER).0;
    let a = kp(&admin);
    s.send_ok(
        &[
            ix(ROUTER, hook_router::accounts::Initialize { admin: a, config, router_program: ROUTER, router_program_data: programdata(&ROUTER), system_program: SYS }, vec![], hook_router::instruction::Initialize {}),
            ix(ROUTER, hook_router::accounts::AdminOnly { admin: a, config }, vec![], hook_router::instruction::SetVenue { venue: hook_router::venues::PUMPSWAP, enabled: true }),
        ],
        &[&admin],
    );

    let router_v = registry::publish_builtin(&mut s, &admin, "allowlist-router", builtin::ALLOWLIST, Environment::Router, ROUTER_BEFORE_SWAP);
    let account_v = registry::publish_builtin(&mut s, &admin, "allowlist-account", builtin::ALLOWLIST, Environment::Account, ACCOUNT_BEFORE_WITHDRAW);
    let i = kp(&issuer);

    // Router policy: the trader (actor) must hold a record from `issuer`.
    let o = kp(&owner);
    let policy = Pk::find_program_address(&[hook_router::POLICY_SEED, o.as_ref(), &1u64.to_le_bytes()], &ROUTER).0;
    let stack = compact(Environment::Router, allowlist(&router_v, builtin::TARGET_ACTOR, &i, ROUTER_BEFORE_SWAP));
    s.send_ok(
        &[ix(
            ROUTER,
            hook_router::accounts::CreatePolicy { authority: o, policy, system_program: SYS },
            vec![Meta::new_readonly(router_v, false)],
            hook_router::instruction::CreatePolicy { seed: 1, delay: 0, shared: true, stack },
        )],
        &[&owner],
    );

    // Smart account: withdrawals only to destinations holding a record from the same issuer.
    let account = Pk::find_program_address(&[smart_accounts::state::ACCOUNT_SEED, o.as_ref(), &1u64.to_le_bytes()], &ACCOUNTS).0;
    let vault = Pk::find_program_address(&[smart_accounts::state::VAULT_SEED, account.as_ref()], &ACCOUNTS).0;
    let p = smart_accounts::CreateParams {
        seed: 1,
        owner: o,
        recovery: Pk::default(),
        recovery_delay: 0,
        policy_delay: 0,
        members: vec![],
        stack: compact(Environment::Account, allowlist(&account_v, builtin::TARGET_COUNTERPARTY, &i, ACCOUNT_BEFORE_WITHDRAW)),
    };
    s.send_ok(
        &[ix(ACCOUNTS, smart_accounts::accounts::CreateAccount { payer: o, account, vault, system_program: SYS }, vec![Meta::new_readonly(account_v, false)], smart_accounts::instruction::CreateAccount { p })],
        &[&owner],
    );
    let funder = s.funded(20);
    s.send_ok(
        &[ix(
            ACCOUNTS,
            smart_accounts::accounts::Deposit { depositor: kp(&funder), account, vault, mint: None, depositor_token: None, vault_token: None, token_program: None, system_program: SYS },
            vec![],
            smart_accounts::instruction::Deposit { amount: 10 * SOL },
        )],
        &[&funder],
    );
    World { s, venue, owner, issuer, account, vault, policy, nonce: 0 }
}

impl World {
    fn withdraw(&self, to: &Pk, amount: u64, record: &Pk) -> Instruction {
        ix(
            ACCOUNTS,
            smart_accounts::accounts::Withdraw {
                authority: kp(&self.owner),
                account: self.account,
                vault: self.vault,
                destination: *to,
                session: Some(ACCOUNTS),
                proposal: Some(ACCOUNTS),
                mint: None,
                vault_token: None,
                token_program: None,
                system_program: SYS,
            },
            vec![Meta::new_readonly(*record, false)],
            smart_accounts::instruction::Withdraw { amount },
        )
    }

    fn buy(&mut self, user: &Pk, quote_in: u64, record: &Pk) -> Instruction {
        self.nonce += 1;
        let receipt = Pk::find_program_address(&[hook_router::RECEIPT_SEED, user.as_ref(), &self.nonce.to_le_bytes()], &ROUTER).0;
        let (base, quote) = self.venue.atas(user);
        let mut extra = self.venue.metas(user);
        let n = extra.len() as u8;
        extra.push(Meta::new_readonly(pump_adapter::PUMP_AMM_PROGRAM_ID, false));
        extra.push(Meta::new_readonly(*record, false));
        ix(
            ROUTER,
            hook_router::accounts::Route {
                user: *user,
                config: Pk::find_program_address(&[hook_router::CONFIG_SEED], &ROUTER).0,
                policy: Some(self.policy),
                user_input: quote,
                user_output: base,
                receipt,
                system_program: SYS,
            },
            extra,
            hook_router::instruction::Route {
                p: hook_router::RouteParams {
                    venue_accounts: n,
                    max_in: quote_in,
                    min_out: 1,
                    max_lamports: SOL / 100,
                    deadline: i64::MAX,
                    nonce: self.nonce,
                    data: PumpSwap::data(Mode::ExactQuoteIn, quote_in, 1),
                },
            },
        )
    }
}

#[test]
fn one_record_gates_two_engines_atomically() {
    let mut w = setup();
    let trader = w.venue.wallet(&mut w.s, 5 * SOL, 2 * SOL);
    let (t, i) = (kp(&trader), kp(&w.issuer));
    let record = hook_state::record_pda(&i, &ns(), &t).0;
    let (base, _) = w.venue.atas(&t);

    // Without a record both engines refuse.
    let wd = w.withdraw(&t, SOL, &record);
    assert!(w.s.send_err(&[wd], &[&w.owner]).contains(DENIED));
    let buy = w.buy(&t, SOL / 10, &record);
    assert!(w.s.send_err(&[buy], &[&trader]).contains(DENIED));

    // One transaction: the issuer publishes, the account pays the trader, the trader buys.
    let lamports0 = w.s.svm.get_balance(&addr(&t)).unwrap();
    let vault0 = w.s.svm.get_balance(&addr(&w.vault)).unwrap();
    let wd = w.withdraw(&t, SOL, &record);
    let buy = w.buy(&t, SOL / 10, &record);
    w.s.send_ok(&[publish(&i, &t), wd, buy], &[&trader, &w.owner, &w.issuer]);
    assert_eq!(w.s.svm.get_balance(&addr(&w.vault)).unwrap(), vault0 - SOL);
    assert!(w.s.svm.get_balance(&addr(&t)).unwrap() > lamports0);
    let tokens = w.s.token_balance(&base);
    assert!(tokens > 0);
    let h: state::StateHeader = bytemuck::pod_read_unaligned(&w.s.svm.get_account(&addr(&record)).unwrap().data[..state::HEADER_LEN]);
    assert_eq!((h.producer, h.issuer, h.subject, h.sequence), (STATE.to_bytes(), i.to_bytes(), t.to_bytes(), 1));

    // Revoked between the two engines in one transaction: the router refuses, and the
    // account's payout in the same transaction is undone with it.
    let (vault1, lamports1) = (w.s.svm.get_balance(&addr(&w.vault)).unwrap(), w.s.svm.get_balance(&addr(&t)).unwrap());
    let wd = w.withdraw(&t, SOL, &record);
    let buy = w.buy(&t, SOL / 10, &record);
    let failed = w.s.send(&[wd, revoke(&i, &t), buy], &[&trader, &w.owner, &w.issuer]).unwrap_err();
    assert!(failed.meta.logs.join("\n").contains(DENIED), "{:?}", failed.meta.logs);
    assert_eq!(w.s.svm.get_balance(&addr(&w.vault)).unwrap(), vault1);
    // The trader (fee payer) keeps everything but the fee of the failed transaction.
    assert_eq!(w.s.svm.get_balance(&addr(&t)).unwrap(), lamports1 - failed.meta.fee);
    assert_eq!(w.s.token_balance(&base), tokens);

    // A byte-identical copy owned by another program is not a record from the producer.
    let forged = Pk::new_unique();
    let mut acc = w.s.svm.get_account(&addr(&record)).unwrap();
    acc.owner = addr(&Pk::new_unique());
    w.s.svm.set_account(addr(&forged), acc).unwrap();
    w.s.send_ok(&[revoke(&i, &t)], &[&w.issuer]);
    let wd = w.withdraw(&t, SOL, &forged);
    assert!(w.s.send_err(&[wd], &[&w.owner]).contains(DENIED));
    let buy = w.buy(&t, SOL / 10, &forged);
    assert!(w.s.send_err(&[buy], &[&trader]).contains(DENIED));

    // A record from a different issuer does not satisfy a policy pinned to `issuer`.
    let mallory = w.s.funded(5);
    let m_record = hook_state::record_pda(&kp(&mallory), &ns(), &t).0;
    w.s.send_ok(&[publish(&kp(&mallory), &t)], &[&mallory]);
    let wd = w.withdraw(&t, SOL, &m_record);
    assert!(w.s.send_err(&[wd], &[&w.owner]).contains(DENIED));

    // Only the issuer can write its records; re-publishing continues the sequence.
    let hijack = Instruction { accounts: vec![M::new(addr(&kp(&mallory)), true), M::new(addr(&record), false), M::new_readonly(addr(&SYS), false)], ..publish(&i, &t) };
    w.s.send_err(&[hijack], &[&mallory]);
    let revoked = w.s.svm.get_account(&addr(&record)).unwrap().data;
    assert_eq!(revoked[..4], [0; 4], "still revoked; the hijack changed nothing");
    w.s.send_ok(&[publish(&i, &t)], &[&w.issuer]);
    let h: state::StateHeader = bytemuck::pod_read_unaligned(&w.s.svm.get_account(&addr(&record)).unwrap().data[..state::HEADER_LEN]);
    assert_eq!(h.sequence, 2);
    let wd = w.withdraw(&t, SOL, &record);
    w.s.send_ok(&[wd], &[&w.owner]);
}

#[test]
fn records_survive_pre_funded_addresses() {
    let mut w = setup();
    let subject = Pk::new_unique();
    let i = kp(&w.issuer);
    let record = hook_state::record_pda(&i, &ns(), &subject).0;
    // Anyone can send lamports to the record address first; publishing still works.
    let rent = w.s.svm.minimum_balance_for_rent_exemption(0);
    w.s.svm.airdrop(&addr(&record), rent).unwrap();
    w.s.send_ok(&[publish(&i, &subject)], &[&w.issuer]);
    assert_eq!(w.s.svm.get_account(&addr(&record)).unwrap().owner, addr(&STATE));
}
