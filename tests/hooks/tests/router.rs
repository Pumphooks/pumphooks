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
    s.compact().to_vec()
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

/// Borsh `GatewayToken` (solana-gateway 0.5.0): version, parent None, owner, identity None,
/// network, issuing gatekeeper, state, expiry.
fn gateway_token(s: &mut Svm, owner: &Pk, network: &Pk, state: u8, expiry: i64) -> Pk {
    let mut d = vec![0u8, 0];
    d.extend_from_slice(owner.as_ref());
    d.push(0);
    d.extend_from_slice(network.as_ref());
    d.extend_from_slice(Pk::new_unique().as_ref());
    d.push(state);
    d.push(1);
    d.extend_from_slice(&expiry.to_le_bytes());
    let k = Pk::new_unique();
    let lamports = s.svm.minimum_balance_for_rent_exemption(d.len());
    s.svm
        .set_account(addr(&k), solana_account::Account { lamports, data: d, owner: addr(&builtin::GATEWAY_PROGRAM), executable: false, rent_epoch: 0 })
        .unwrap();
    k
}

#[test]
fn credential_and_realized_price_limit_gate_routes() {
    use hook_kernel::builtin::{CredentialConfig, PriceLimitConfig, LIMIT_REALIZED};
    let mut t = setup();
    let admin = t.admin.insecure_clone();
    let r = Environment::Router;
    let cred_v = registry::publish_builtin(&mut t.s, &admin, "credential", builtin::CREDENTIAL, r, BEFORE_SWAP);
    let limit_v = registry::publish_builtin(&mut t.s, &admin, "price-limit", builtin::PRICE_LIMIT, r, AFTER_SWAP);
    let user = t.venue.wallet(&mut t.s, 5 * SOL, 2 * SOL);
    let u = kp(&user);
    let network = Pk::new_unique();
    let now: solana_clock::Clock = t.s.svm.get_sysvar();

    // Reference price from an unrestricted buy.
    let r0 = route_ok(&mut t, &user, &Route::buy(SOL / 10));
    let cred = CredentialConfig { target: 0, flags: 0, _pad: [0; 6], gatekeeper_network: network.to_bytes(), _reserved: [0; 32], _rest: [0; 24] };
    // Require ≥ half the reference price (tokens per lamport) on WSOL-in routes.
    let floor = PriceLimitConfig { min_num: r0.received / 2, min_den: r0.spent, max_num: 0, max_den: 0, asset: pump_adapter::NATIVE_MINT.to_bytes(), mode: LIMIT_REALIZED, _pad: [0; 7], _rest: [0; 24] };
    let entries = [entry(&cred_v, builtin::CREDENTIAL, BEFORE_SWAP, cred, false), entry(&limit_v, builtin::PRICE_LIMIT, AFTER_SWAP, floor, false)];
    let policy = create_policy_with(&mut t, &user, 1, 0, false, &entries);

    let route_with = |t: &mut T, extra: Vec<Meta>, route: &Route| {
        let (base, quote) = t.venue.atas(&u);
        let metas = t.venue.metas(&u);
        let (mut i, _) = route_ix_with(t, &u, route, metas, quote, base);
        i.accounts.extend(extra.into_iter().map(|m| solana_instruction::AccountMeta { pubkey: addr(&m.pubkey), is_signer: false, is_writable: false }));
        i
    };
    let buy = Route { policy: Some(policy), ..Route::buy(SOL / 10) };

    // No credential supplied → refused before the venue runs.
    let i = route_with(&mut t, vec![], &buy);
    assert!(t.s.send_err(&[i], &[&user]).contains("reason 16"));
    // Expired, revoked, wrong network or wrong wallet → refused.
    for (owner, net, state, exp) in [(u, network, 0, now.unix_timestamp - 1), (u, network, 2, i64::MAX), (u, Pk::new_unique(), 0, i64::MAX), (Pk::new_unique(), network, 0, i64::MAX)] {
        let g = gateway_token(&mut t.s, &owner, &net, state, exp);
        let i = route_with(&mut t, vec![Meta::new_readonly(g, false)], &buy);
        assert!(t.s.send_err(&[i], &[&user]).contains("reason 16"));
    }
    // A valid pass: the buy goes through at a price above the floor.
    let pass = gateway_token(&mut t.s, &u, &network, 0, i64::MAX);
    let i = route_with(&mut t, vec![Meta::new_readonly(pass, false)], &buy);
    t.s.send_ok(&[i], &[&user]);

    // Raise the floor above what the venue pays: the route reverts after the venue CPI.
    let high = PriceLimitConfig { min_num: r0.received * 2, ..floor };
    let entries = [entry(&cred_v, builtin::CREDENTIAL, BEFORE_SWAP, cred, false), entry(&limit_v, builtin::PRICE_LIMIT, AFTER_SWAP, high, false)];
    let strict = create_policy_with(&mut t, &user, 2, 0, false, &entries);
    let (base, _) = t.venue.atas(&u);
    let before = t.s.token_balance(&base);
    let i = route_with(&mut t, vec![Meta::new_readonly(pass, false)], &Route { policy: Some(strict), ..Route::buy(SOL / 10) });
    assert!(t.s.send_err(&[i], &[&user]).contains("reason 15"));
    assert_eq!(t.s.token_balance(&base), before);
}

/// The `examples/anti-jit-hook` program, built with `crates/hook-sdk`, installed from the registry
/// as a program entry and called by the router through the real PumpSwap route.
#[test]
fn example_program_hook_refuses_round_trips_within_the_window() {
    use hook_standard::{cap, IMPL_PROGRAM};
    use solana_instruction::{AccountMeta as M, Instruction};
    let mut t = setup();
    let dev = t.s.funded(100);
    let hook = Pk::new_unique();
    t.s.add_program(&hook, "anti_jit_hook.so");
    t.s.set_upgrade_authority(&hook, Some(kp(&dev)));
    let caps = cap::VALIDATE_OPERATION | cap::VETO_OPERATION | cap::UPDATE_OWN_STATE;
    let h = registry::hook("anti-jit");
    t.s.send_ok(
        &[
            registry::create_hook(&kp(&dev), "anti-jit"),
            registry::publish(&kp(&dev), &h, 0, Some(hook), registry::program_params(Environment::Router, BEFORE_SWAP, caps, [1, 0, 0])),
        ],
        &[&dev],
    );
    let mut e = StackEntry::zeroed();
    e.hook_version = registry::version(&h, 0).to_bytes();
    e.program = hook.to_bytes();
    e.impl_kind = IMPL_PROGRAM;
    e.callbacks = BEFORE_SWAP;
    e.granted = caps;
    e.mandatory = 1;
    let owner = t.s.funded(10);
    let policy = create_policy(&mut t, &owner, 7, 0, &[e]);

    let cfg = anti_jit_hook::config_pda(&hook, &policy).0;
    let configure = |by: &Pk, slots: u64| Instruction {
        program_id: addr(&hook),
        accounts: vec![M::new(addr(by), true), M::new_readonly(addr(&policy), false), M::new(addr(&cfg), false), M::new_readonly(addr(&SYS), false)],
        data: [&[0u8][..], &slots.to_le_bytes()].concat(),
    };
    let mallory = t.s.funded(10);
    assert!(t.s.send_err(&[configure(&kp(&mallory), 0)], &[&mallory]).contains("IllegalOwner"), "only the policy authority configures");
    t.s.send_ok(&[configure(&kp(&owner), 10)], &[&owner]);

    let user = t.venue.wallet(&mut t.s, 5 * SOL, 2 * SOL);
    let u = kp(&user);
    let state = anti_jit_hook::actor_pda(&hook, &policy, &u).0;
    let authority = hook_kernel::cpi::callback_authority(&ID, &hook).0;
    let route = |t: &mut T, r: Route| {
        let (mut i, _) = route_ix(t, &u, &Route { policy: Some(policy), ..r });
        for (k, w) in [(hook, false), (authority, false), (cfg, false), (state, true)] {
            i.accounts.push(solana_instruction::AccountMeta { pubkey: addr(&k), is_signer: false, is_writable: w });
        }
        i
    };

    // Not registered: refused (fail closed), with the hook's own reason.
    let i = route(&mut t, Route::buy(SOL / 10));
    assert!(t.s.send_err(&[i], &[&user]).contains("hook_reason 1001"));

    // Register in the same transaction as the first buy.
    let register = Instruction {
        program_id: addr(&hook),
        accounts: vec![M::new(addr(&u), true), M::new_readonly(addr(&policy), false), M::new_readonly(addr(&u), false), M::new(addr(&state), false), M::new_readonly(addr(&SYS), false)],
        data: vec![1],
    };
    let i = route(&mut t, Route::buy(SOL / 10));
    t.s.send_ok(&[register.clone(), i], &[&user]);
    let (base, _) = t.venue.atas(&u);
    let tokens = t.s.token_balance(&base);
    assert!(tokens > 0);

    // Selling back within 10 slots reverses the buy: refused, nothing moves.
    let i = route(&mut t, Route::sell(tokens / 2));
    assert!(t.s.send_err(&[i], &[&user]).contains("hook_reason 1000"));
    assert_eq!(t.s.token_balance(&base), tokens);
    // Buying again (same direction) is fine.
    let i = route(&mut t, Route::buy(SOL / 20));
    t.s.send_ok(&[i], &[&user]);
    // After the window, the sell goes through.
    let slot = t.s.svm.get_sysvar::<solana_clock::Clock>().slot;
    t.s.warp_slot(slot + 11);
    let tokens = t.s.token_balance(&base);
    let i = route(&mut t, Route::sell(tokens / 2));
    t.s.send_ok(&[i], &[&user]);

    // A policy call not signed by the router's callback authority is refused by the SDK.
    let mut ctx = hook_sdk::testing::context(Environment::Router, 2, policy, u);
    ctx.slot = 1;
    let mut forged = hook_sdk::testing::call(&hook, &ID, ctx, &[], &[]);
    forged.accounts[0].is_signer = false;
    let forged = Instruction {
        program_id: addr(&forged.program_id),
        accounts: forged.accounts.iter().map(|a| M { pubkey: addr(&a.pubkey), is_signer: a.is_signer, is_writable: a.is_writable }).collect(),
        data: forged.data,
    };
    assert!(t.s.send_err(&[forged], &[&user]).contains("missing required signature"));
}
