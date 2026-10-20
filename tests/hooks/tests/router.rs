//! ROUTER engine against the real PumpSwap binary and a cloned devnet pool: adapter checks,
//! deadline, measured max-in / min-out, receipts, and router hook stacks (max size, time window,
//! allowlist, accounting). Also shows the venue stays directly callable (router hooks are
//! bypassable by design).

mod common;

use anchor_lang::solana_program::instruction::AccountMeta as Meta;
use bytemuck::Zeroable;
use common::{pump::*, registry, *};
use hook_kernel::builtin::{self, AccountingState, AllowlistConfig, MaxSizeConfig, TimeWindowConfig};
use hook_router::{accounts as A, instruction as I, venues, Receipt, RouteParams, RoutePolicy};
use hook_standard::{Environment, HookStack, StackEntry, IMPL_BUILTIN};
use solana_keypair::Keypair;

const ID: Pk = hook_router::ID;
const SYS: Pk = anchor_lang::system_program::ID;
const BEFORE_EXECUTE: u16 = 1 << 0;
const BEFORE_SWAP: u16 = 1 << 2;
const AFTER_SWAP: u16 = 1 << 3;
const DAY: i64 = 86_400;

struct Versions {
    max_size: Pk,
    window: Pk,
    allowlist: Pk,
    accounting: Pk,
}

struct T {
    s: Svm,
    admin: Keypair,
    venue: PumpSwap,
    v: Versions,
    nonce: u64,
}

fn config_pda() -> Pk {
    Pk::find_program_address(&[hook_router::CONFIG_SEED], &ID).0
}
fn policy_pda(authority: &Pk, seed: u64) -> Pk {
    Pk::find_program_address(&[hook_router::POLICY_SEED, authority.as_ref(), &seed.to_le_bytes()], &ID).0
}
fn receipt_pda(user: &Pk, nonce: u64) -> Pk {
    Pk::find_program_address(&[hook_router::RECEIPT_SEED, user.as_ref(), &nonce.to_le_bytes()], &ID).0
}

fn initialize(admin: &Pk) -> solana_instruction::Instruction {
    ix(
        ID,
        A::Initialize { admin: *admin, config: config_pda(), router_program: ID, router_program_data: programdata(&ID), system_program: SYS },
        vec![],
        I::Initialize {},
    )
}
fn set_venue(admin: &Pk, venue: u8, enabled: bool) -> solana_instruction::Instruction {
    ix(ID, A::AdminOnly { admin: *admin, config: config_pda() }, vec![], I::SetVenue { venue, enabled })
}

fn setup() -> T {
    let mut s = Svm::new();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let mut clock: solana_clock::Clock = s.svm.get_sysvar();
    clock.unix_timestamp = now;
    clock.slot = 400_000_000;
    s.svm.set_sysvar(&clock);

    let admin = s.funded(1000);
    registry::setup(&mut s, &admin);
    s.add_program(&ID, "hook_router.so");
    s.set_upgrade_authority(&ID, Some(kp(&admin)));
    let venue = PumpSwap::load(&mut s);
    s.send_ok(&[initialize(&kp(&admin)), set_venue(&kp(&admin), venues::PUMPSWAP, true)], &[&admin]);

    let r = Environment::Router;
    let v = Versions {
        max_size: registry::publish_builtin(&mut s, &admin, "max-size", builtin::MAX_SIZE, r, BEFORE_EXECUTE | BEFORE_SWAP),
        window: registry::publish_builtin(&mut s, &admin, "time-window", builtin::TIME_WINDOW, r, BEFORE_EXECUTE | BEFORE_SWAP),
        allowlist: registry::publish_builtin(&mut s, &admin, "allowlist", builtin::ALLOWLIST, r, BEFORE_EXECUTE | BEFORE_SWAP),
        accounting: registry::publish_builtin(&mut s, &admin, "accounting", builtin::ACCOUNTING, r, AFTER_SWAP),
    };
    T { s, admin, venue, v, nonce: 0 }
}

fn entry<C: bytemuck::Pod>(version: &Pk, id: u8, callbacks: u16, cfg: C, mandatory: bool) -> StackEntry {
    let mut e = StackEntry::zeroed();
    e.hook_version = version.to_bytes();
    e.impl_kind = IMPL_BUILTIN;
    e.builtin_id = id;
    e.callbacks = callbacks;
    e.granted = builtin::required_caps(id);
    e.mandatory = mandatory as u8;
    e.config[..core::mem::size_of::<C>()].copy_from_slice(bytemuck::bytes_of(&cfg));
    e
}
fn max_size(t: &T, max: u64) -> StackEntry {
    let c = MaxSizeConfig { asset: pump_adapter::NATIVE_MINT.to_bytes(), max_amount: max, _rest: [0; 56] };
    entry(&t.v.max_size, builtin::MAX_SIZE, BEFORE_SWAP, c, true)
}
fn window(t: &T, not_before: i64, not_after: i64) -> StackEntry {
    let c = TimeWindowConfig { not_before, not_after, daily_start: 0, daily_end: 0, _rest: [0; 72] };
    entry(&t.v.window, builtin::TIME_WINDOW, BEFORE_EXECUTE, c, false)
}
fn allowlist(t: &T, allowed: &Pk) -> StackEntry {
    let c = AllowlistConfig { target: 0, source: builtin::SOURCE_INLINE, _pad: [0; 6], max_age_slots: 0, key_a: allowed.to_bytes(), key_b: [0; 32], _rest: [0; 16] };
    entry(&t.v.allowlist, builtin::ALLOWLIST, BEFORE_SWAP, c, false)
}
fn accounting(t: &T) -> StackEntry {
    entry(&t.v.accounting, builtin::ACCOUNTING, AFTER_SWAP, [0u8; 96], false)
}

fn stack(entries: &[StackEntry]) -> Vec<u8> {
    let mut s = HookStack::empty(Environment::Router);
    s.len = entries.len() as u8;
    s.entries[..entries.len()].copy_from_slice(entries);
    bytemuck::bytes_of(&s).to_vec()
}
fn versions_of(entries: &[StackEntry]) -> Vec<Meta> {
    let mut v: Vec<Pk> = entries.iter().map(|e| Pk::new_from_array(e.hook_version)).collect();
    v.sort();
    v.dedup();
    v.into_iter().map(|k| Meta::new_readonly(k, false)).collect()
}

fn create_policy(t: &mut T, authority: &Keypair, seed: u64, delay: i64, entries: &[StackEntry]) -> Pk {
    create_policy_with(t, authority, seed, delay, true, entries)
}

fn create_policy_with(t: &mut T, authority: &Keypair, seed: u64, delay: i64, shared: bool, entries: &[StackEntry]) -> Pk {
    let p = policy_pda(&kp(authority), seed);
    let i = ix(
        ID,
        A::CreatePolicy { authority: kp(authority), policy: p, system_program: SYS },
        versions_of(entries),
        I::CreatePolicy { seed, delay, shared, stack: stack(entries) },
    );
    t.s.send_ok(&[i], &[authority]);
    p
}
fn update_policy(authority: &Pk, policy: &Pk, entries: &[StackEntry]) -> solana_instruction::Instruction {
    ix(ID, A::UpdatePolicy { authority: *authority, policy: *policy }, versions_of(entries), I::UpdatePolicy { stack: stack(entries) })
}
fn read_policy(t: &T, p: &Pk) -> RoutePolicy {
    let a = t.s.svm.get_account(&addr(p)).unwrap();
    *bytemuck::from_bytes(&a.data[8..8 + core::mem::size_of::<RoutePolicy>()])
}

struct Route {
    data: Vec<u8>,
    max_in: u64,
    min_out: u64,
    max_lamports: u64,
    deadline: i64,
    policy: Option<Pk>,
    buy: bool,
}

impl Route {
    fn buy(quote_in: u64) -> Self {
        Route { data: PumpSwap::data(Mode::ExactQuoteIn, quote_in, 1), max_in: quote_in, min_out: 1, max_lamports: SOL / 100, deadline: i64::MAX, policy: None, buy: true }
    }
    fn sell(base_in: u64) -> Self {
        Route { data: PumpSwap::data(Mode::ExactBaseIn, base_in, 1), max_in: base_in, min_out: 1, max_lamports: SOL / 100, deadline: i64::MAX, policy: None, buy: false }
    }
}

fn route_ix_with(t: &mut T, user: &Pk, r: &Route, metas: Vec<Meta>, input: Pk, output: Pk) -> (solana_instruction::Instruction, Pk) {
    t.nonce += 1;
    let receipt = receipt_pda(user, t.nonce);
    let n = metas.len() as u8;
    let mut extra = metas;
    extra.push(Meta::new_readonly(pump_adapter::PUMP_AMM_PROGRAM_ID, false));
    let i = ix(
        ID,
        A::Route {
            user: *user,
            config: config_pda(),
            policy: Some(r.policy.unwrap_or(ID)),
            user_input: input,
            user_output: output,
            receipt,
            system_program: SYS,
        },
        extra,
        I::Route { p: RouteParams { venue_accounts: n, max_in: r.max_in, min_out: r.min_out, max_lamports: r.max_lamports, deadline: r.deadline, nonce: t.nonce, data: r.data.clone() } },
    );
    (i, receipt)
}

fn route_ix(t: &mut T, user: &Pk, r: &Route) -> (solana_instruction::Instruction, Pk) {
    let (base, quote) = t.venue.atas(user);
    let (input, output) = if r.buy { (quote, base) } else { (base, quote) };
    let metas = t.venue.metas(user);
    route_ix_with(t, user, r, metas, input, output)
}

fn route_ok(t: &mut T, user: &Keypair, r: &Route) -> Receipt {
    let (i, receipt) = route_ix(t, &kp(user), r);
    t.s.send_ok(&[i], &[user]);
    t.s.get(&receipt)
}
fn route_err(t: &mut T, user: &Keypair, r: &Route) -> String {
    let (i, _) = route_ix(t, &kp(user), r);
    t.s.send_err(&[i], &[user])
}

#[test]
fn buy_and_sell_through_pumpswap_write_measured_receipts() {
    let mut t = setup();
    let user = t.venue.wallet(&mut t.s, 5 * SOL, 2 * SOL);
    let u = kp(&user);
    let (base, quote) = t.venue.atas(&u);

    let (b0, q0) = (t.s.token_balance(&base), t.s.token_balance(&quote));
    let r = route_ok(&mut t, &user, &Route::buy(SOL / 2));
    let (b1, q1) = (t.s.token_balance(&base), t.s.token_balance(&quote));
    assert_eq!(r.user, u);
    assert_eq!(r.venue, venues::PUMPSWAP);
    assert!(r.venue_verified);
    assert_eq!(r.input_mint, pump_adapter::NATIVE_MINT);
    assert_eq!(r.output_mint, t.venue.mint);
    assert_eq!(r.spent, q0 - q1);
    assert_eq!(r.spent, SOL / 2);
    assert_eq!(r.received, b1 - b0);
    assert!(r.received > 0);
    assert_eq!(r.policy, Pk::default());

    let r2 = route_ok(&mut t, &user, &Route::sell(b1 / 2));
    assert_eq!(r2.input_mint, t.venue.mint);
    assert_eq!(r2.spent, b1 / 2);
    assert_eq!(r2.received, t.s.token_balance(&quote) - q1);
    assert!(r2.received > 0);
}

#[test]
fn measured_min_out_and_max_in_are_enforced_by_the_router() {
    let mut t = setup();
    let user = t.venue.wallet(&mut t.s, 5 * SOL, 2 * SOL);
    // The venue's own limit is 1; the router's measured min_out is unreachable.
    let mut r = Route::buy(SOL / 10);
    r.min_out = u64::MAX / 2;
    assert!(route_err(&mut t, &user, &r).contains("SlippageExceeded"));

    // buy_v2: exact base out with a generous venue limit; the router caps measured quote spent.
    let mut r = Route::buy(0);
    r.data = PumpSwap::data(Mode::ExactBaseOut, 1_000_000, SOL);
    r.max_in = 1;
    assert!(route_err(&mut t, &user, &r).contains("MaxInExceeded"));
    r.max_in = SOL;
    route_ok(&mut t, &user, &r);
}

#[test]
fn deadline_and_receipt_nonce_replay() {
    let mut t = setup();
    let user = t.venue.wallet(&mut t.s, 5 * SOL, 2 * SOL);
    let mut r = Route::buy(SOL / 10);
    r.deadline = t.s.now() - 1;
    assert!(route_err(&mut t, &user, &r).contains("DeadlineExceeded"));

    let r = Route::buy(SOL / 10);
    let (i, receipt) = route_ix(&mut t, &kp(&user), &r);
    t.s.send_ok(std::slice::from_ref(&i), &[&user]);
    let before = t.s.token_balance(&t.venue.atas(&kp(&user)).1);
    let err = t.s.send_err(&[i], &[&user]);
    assert!(err.contains("already in use"), "{err}");
    assert_eq!(t.s.token_balance(&t.venue.atas(&kp(&user)).1), before, "replay moved nothing");

    // Closing returns rent to the user only.
    let other = t.s.funded(1);
    let close = |who: &Pk| ix(ID, A::CloseReceipt { user: *who, receipt }, vec![], I::CloseReceipt {});
    assert!(t.s.send_err(&[close(&kp(&other))], &[&other]).contains("Unauthorized"));
    t.s.send_ok(&[close(&kp(&user))], &[&user]);
    assert!(!t.s.exists(&receipt));
}

#[test]
fn adapter_rejects_disabled_venues_unknown_instructions_and_substituted_accounts() {
    let mut t = setup();
    let user = t.venue.wallet(&mut t.s, 5 * SOL, 2 * SOL);
    let u = kp(&user);
    let (base, quote) = t.venue.atas(&u);

    // Not an allowlisted swap discriminator (PumpSwap `deposit`-style data).
    let mut r = Route::buy(SOL / 10);
    r.data = vec![242, 35, 198, 137, 82, 225, 242, 182, 0, 0, 0, 0, 0, 0, 0, 0];
    assert!(route_err(&mut t, &user, &r).contains("UnknownInstruction"));

    // Measured accounts must be the venue's input/output slots.
    let other = t.venue.wallet(&mut t.s, SOL, SOL);
    let (_, other_quote) = t.venue.atas(&kp(&other));
    let metas = t.venue.metas(&u);
    let (i, _) = route_ix_with(&mut t, &u, &Route::buy(SOL / 10), metas.clone(), other_quote, base);
    assert!(t.s.send_err(&[i], &[&user]).contains("AccountMismatch"));
    let (i, _) = route_ix_with(&mut t, &u, &Route::buy(SOL / 10), metas.clone(), base, quote);
    assert!(t.s.send_err(&[i], &[&user]).contains("AccountMismatch"), "swapped input/output");

    // The venue's signer slot must be the router's user.
    let mut other_metas = t.venue.metas(&kp(&other));
    other_metas.iter_mut().for_each(|m| m.is_signer = false);
    let (i, _) = route_ix_with(&mut t, &u, &Route::buy(SOL / 10), other_metas, quote, base);
    assert!(t.s.send_err(&[i], &[&user]).contains("AccountMismatch"));

    // Too few accounts for the venue.
    let (i, _) = route_ix_with(&mut t, &u, &Route::buy(SOL / 10), metas[..16].to_vec(), quote, base);
    assert!(t.s.send_err(&[i], &[&user]).contains("AccountMismatch"));

    // Admin-only venue switch; disabled venue refused.
    let admin = kp(&t.admin);
    assert!(t.s.send_err(&[set_venue(&u, venues::PUMPSWAP, false)], &[&user]).contains("Unauthorized"));
    let a = t.admin.insecure_clone();
    t.s.send_ok(&[set_venue(&admin, venues::PUMPSWAP, false)], &[&a]);
    assert!(route_err(&mut t, &user, &Route::buy(SOL / 10)).contains("VenueDisabled"));
}

#[test]
fn initialize_requires_the_upgrade_authority() {
    let mut s = Svm::new();
    let admin = s.funded(10);
    let mallory = s.funded(10);
    s.add_program(&ID, "hook_router.so");
    s.set_upgrade_authority(&ID, Some(kp(&admin)));
    assert!(s.send_err(&[initialize(&kp(&mallory))], &[&mallory]).contains("Unauthorized"));
    s.send_ok(&[initialize(&kp(&admin))], &[&admin]);
}

#[test]
fn max_size_caps_buys_and_venue_stays_directly_callable() {
    let mut t = setup();
    let owner = t.s.funded(10);
    let e = max_size(&t, SOL / 4);
    let p = create_policy(&mut t, &owner, 1, DAY, &[e]);
    let user = t.venue.wallet(&mut t.s, 5 * SOL, 2 * SOL);

    let mut r = Route::buy(SOL / 2);
    r.policy = Some(p);
    let err = route_err(&mut t, &user, &r);
    assert!(err.contains("PolicyRejected"), "{err}");
    let mut r = Route::buy(SOL / 5);
    r.policy = Some(p);
    let rec = route_ok(&mut t, &user, &r);
    assert_eq!(rec.policy, p);

    // Router enforcement is opt-in: the same oversize buy straight to PumpSwap succeeds.
    let direct = t.venue.direct(&kp(&user), Mode::ExactQuoteIn, SOL / 2, 1);
    t.s.send_ok(&[direct], &[&user]);
}

#[test]
fn time_window_allowlist_and_accounting_compose() {
    let mut t = setup();
    let owner = t.s.funded(10);
    let alice = t.venue.wallet(&mut t.s, 5 * SOL, 2 * SOL);
    let bob = t.venue.wallet(&mut t.s, 5 * SOL, 2 * SOL);
    let now = t.s.now();
    let entries = [window(&t, now - 10, now + 3_600), allowlist(&t, &kp(&alice)), accounting(&t)];
    let p = create_policy(&mut t, &owner, 7, 0, &entries);

    let mut r = Route::buy(SOL / 10);
    r.policy = Some(p);
    assert!(route_err(&mut t, &bob, &r).contains("PolicyRejected"), "bob is not allowlisted");
    let a = route_ok(&mut t, &alice, &r);
    let b = route_ok(&mut t, &alice, &r);

    let st = read_policy(&t, &p);
    let acc: AccountingState = bytemuck::pod_read_unaligned(&st.stack.entries[2].state[..32]);
    assert_eq!(acc.count, 2);
    assert_eq!(acc.volume, (a.spent + b.spent) as u128, "accounting sees measured amounts");
    assert_eq!(st.sequence, 2);

    let mut c: solana_clock::Clock = t.s.svm.get_sysvar();
    c.unix_timestamp = now + 3_601;
    t.s.svm.set_sysvar(&c);
    assert!(route_err(&mut t, &alice, &r).contains("PolicyRejected"), "outside the window");
}

#[test]
fn mandatory_policy_entries_change_only_after_the_delay() {
    let mut t = setup();
    let owner = t.s.funded(10);
    let o = kp(&owner);
    let strict = max_size(&t, SOL / 4);
    let p = create_policy(&mut t, &owner, 3, DAY, &[strict]);

    let loose = max_size(&t, 10 * SOL);
    let mallory = t.s.funded(1);
    assert!(t.s.send_err(&[update_policy(&kp(&mallory), &p, &[loose])], &[&mallory]).contains("Unauthorized"));

    t.s.send_ok(&[update_policy(&o, &p, &[loose])], &[&owner]);
    assert_eq!(read_policy(&t, &p).stack.entries[0].config[32..40], (SOL / 4).to_le_bytes(), "only staged");
    assert!(t.s.send_err(&[update_policy(&o, &p, &[loose])], &[&owner]).contains("TimelockPending"));

    let mut c: solana_clock::Clock = t.s.svm.get_sysvar();
    c.unix_timestamp += DAY;
    t.s.svm.set_sysvar(&c);
    t.s.send_ok(&[update_policy(&o, &p, &[loose])], &[&owner]);
    assert_eq!(read_policy(&t, &p).stack.entries[0].config[32..40], (10 * SOL).to_le_bytes());

    // Adding an entry leaves the mandatory one untouched, so it applies immediately.
    let now = t.s.now();
    let extra = window(&t, now, now + DAY);
    t.s.send_ok(&[update_policy(&o, &p, &[loose, extra])], &[&owner]);
    assert_eq!(read_policy(&t, &p).stack.len, 2);
}

#[test]
fn stacks_must_be_router_stacks_backed_by_registry_versions() {
    let mut t = setup();
    let owner = t.s.funded(10);
    let e = max_size(&t, SOL);
    let mut bad = HookStack::empty(Environment::Account);
    bad.len = 1;
    bad.entries[0] = e;
    let p = policy_pda(&kp(&owner), 9);
    let i = ix(
        ID,
        A::CreatePolicy { authority: kp(&owner), policy: p, system_program: SYS },
        versions_of(&[e]),
        I::CreatePolicy { seed: 9, delay: 0, shared: false, stack: bytemuck::bytes_of(&bad).to_vec() },
    );
    assert!(t.s.send_err(&[i], &[&owner]).contains("InvalidStack"));

    let i = ix(
        ID,
        A::CreatePolicy { authority: kp(&owner), policy: p, system_program: SYS },
        vec![],
        I::CreatePolicy { seed: 9, delay: 0, shared: false, stack: stack(&[e]) },
    );
    assert!(t.s.send_err(&[i], &[&owner]).contains("MissingVersion"));

    // A version published for another builtin cannot back this entry.
    let mut forged = e;
    forged.hook_version = t.v.accounting.to_bytes();
    let i = ix(
        ID,
        A::CreatePolicy { authority: kp(&owner), policy: p, system_program: SYS },
        versions_of(&[forged]),
        I::CreatePolicy { seed: 9, delay: 0, shared: false, stack: stack(&[forged]) },
    );
    t.s.send_err(&[i], &[&owner]);
}

#[test]
fn private_policies_only_apply_to_their_authority() {
    let mut t = setup();
    let owner = t.venue.wallet(&mut t.s, 5 * SOL, 2 * SOL);
    let stranger = t.venue.wallet(&mut t.s, 5 * SOL, 2 * SOL);
    let entries = [accounting(&t)];
    let p = create_policy_with(&mut t, &owner, 11, 0, false, &entries);

    let mut r = Route::buy(SOL / 10);
    r.policy = Some(p);
    assert!(route_err(&mut t, &stranger, &r).contains("PolicyNotShared"), "cannot spend or pollute another user's policy state");
    route_ok(&mut t, &owner, &r);
    let acc: AccountingState = bytemuck::pod_read_unaligned(&read_policy(&t, &p).stack.entries[0].state[..32]);
    assert_eq!(acc.count, 1);
}

#[test]
fn sol_taken_from_the_wallet_is_measured_and_capped() {
    let mut t = setup();
    let user = t.venue.wallet(&mut t.s, 5 * SOL, 2 * SOL);
    let u = kp(&user);

    // A wallet's first PumpSwap trade pays rent for its volume accumulator out of the wallet.
    let mut r = Route::buy(SOL / 10);
    r.max_lamports = 0;
    assert!(route_err(&mut t, &user, &r).contains("LamportsExceeded"));

    let before = t.s.lamports(&u);
    let (i, receipt) = route_ix(&mut t, &u, &Route::buy(SOL / 10));
    t.s.send_ok(&[i], &[&user]);
    let rec: Receipt = t.s.get(&receipt);
    let fee = 5_000;
    assert!(rec.lamports_spent > 0);
    assert_eq!(before - t.s.lamports(&u), rec.lamports_spent + t.s.lamports(&receipt) + fee, "receipt records exactly what the venue took");

    // Afterwards the venue takes no SOL, so a zero cap passes.
    let rec = route_ok(&mut t, &user, &r);
    assert_eq!(rec.lamports_spent, 0);
}
