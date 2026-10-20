//! TOKEN engine: `token-hooks` (Token-2022 transfer hook on the shared kernel) and
//! `token-wrapper` (1:1 backed wrapped mints), against LiteSVM's bundled Token-2022 program.

mod common;

use anchor_lang::{solana_program::instruction::AccountMeta as Meta, system_program};
use anchor_spl::token_2022::spl_token_2022::{
    self,
    extension::{transfer_fee, transfer_hook, ExtensionType},
    state::{Account as T22Account, Mint as T22Mint},
};
use bytemuck::Zeroable;
use common::{registry, tokens::*, *};
use hook_kernel::builtin::{
    self, AllowlistConfig, CredentialConfig, MaxSizeConfig, SpendingLimitConfig, GATEWAY_PROGRAM,
    SOURCE_INLINE, TARGET_COUNTERPARTY,
};
use hook_standard::{Environment, HookStack, StackEntry, IMPL_BUILTIN};
use solana_keypair::Keypair;

const HOOKS: Pk = token_hooks::ID;
const WRAPPER: Pk = token_wrapper::ID;
const T22: Pk = spl_token_2022::ID;
const SYS: Pk = system_program::ID;
const T0: i64 = 1_700_000_000;
const DAY: i64 = 86_400;
const TRANSFER: u16 = 1 << 0;
const WRAP: u16 = 1 << 2;
const UNWRAP: u16 = 1 << 3;

fn t22(ix: anchor_lang::solana_program::instruction::Instruction) -> solana_instruction::Instruction {
    convert(ix.program_id, ix.accounts, ix.data)
}

struct Versions {
    max_size: Pk,
    spending: Pk,
    allowlist: Pk,
    credential: Pk,
}

struct T {
    s: Svm,
    admin: Keypair,
    v: Versions,
}

fn setup() -> T {
    let mut s = Svm::new();
    s.warp_time(T0);
    let admin = s.funded(1000);
    registry::setup(&mut s, &admin);
    s.add_program(&HOOKS, "token_hooks.so");
    s.add_program(&WRAPPER, "token_wrapper.so");
    let tok = Environment::Token;
    let mut publish = |slug: &str, id: u8| {
        registry::publish_builtin(&mut s, &admin, slug, id, tok, builtin::supported_callbacks(id, tok))
    };
    let v = Versions {
        max_size: publish("max-size", builtin::MAX_SIZE),
        spending: publish("spending-limit", builtin::SPENDING_LIMIT),
        allowlist: publish("allowlist", builtin::ALLOWLIST),
        credential: publish("credential", builtin::CREDENTIAL),
    };
    T { s, admin, v }
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
fn max_size(t: &T, mint: &Pk, max: u64, callbacks: u16) -> StackEntry {
    let c = MaxSizeConfig { asset: mint.to_bytes(), max_amount: max, _rest: [0; 56] };
    entry(&t.v.max_size, builtin::MAX_SIZE, callbacks, c, true)
}
fn spending(t: &T, mint: &Pk, limit: u64) -> StackEntry {
    let c = SpendingLimitConfig { asset: mint.to_bytes(), limit, period_secs: DAY as u32, _pad: 0, _rest: [0; 48] };
    entry(&t.v.spending, builtin::SPENDING_LIMIT, TRANSFER, c, false)
}
fn recipients(t: &T, a: &Pk, b: &Pk) -> StackEntry {
    let c = AllowlistConfig { target: TARGET_COUNTERPARTY, source: SOURCE_INLINE, _pad: [0; 6], max_age_slots: 0, key_a: a.to_bytes(), key_b: b.to_bytes(), _rest: [0; 16] };
    entry(&t.v.allowlist, builtin::ALLOWLIST, TRANSFER, c, false)
}
fn credential(t: &T, network: &Pk) -> StackEntry {
    let mut c = CredentialConfig::zeroed();
    c.target = TARGET_COUNTERPARTY;
    c.gatekeeper_network = network.to_bytes();
    entry(&t.v.credential, builtin::CREDENTIAL, TRANSFER, c, false)
}

fn stack(entries: &[StackEntry]) -> Vec<u8> {
    let mut s = HookStack::empty(Environment::Token);
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

fn policy(mint: &Pk) -> Pk {
    Pk::find_program_address(&[token_hooks::POLICY_SEED, mint.as_ref()], &HOOKS).0
}
fn metas(mint: &Pk) -> Pk {
    Pk::find_program_address(&[token_hooks::EXTRA_METAS_SEED, mint.as_ref()], &HOOKS).0
}
fn gateway_pda(wallet: &Pk, network: &Pk) -> Pk {
    Pk::find_program_address(&[wallet.as_ref(), b"gateway", &[0; 8], network.as_ref()], &GATEWAY_PROGRAM).0
}

fn create_policy_ix(payer: &Pk, authority: &Pk, mint: &Pk, delay: i64, entries: &[StackEntry]) -> solana_instruction::Instruction {
    ix(
        HOOKS,
        token_hooks::accounts::CreatePolicy { payer: *payer, authority: *authority, mint: *mint, policy: policy(mint), extra_account_metas: metas(mint), system_program: SYS },
        versions_of(entries),
        token_hooks::instruction::CreatePolicy { delay, stack: stack(entries) },
    )
}
fn update_policy_ix(authority: &Pk, mint: &Pk, entries: &[StackEntry]) -> solana_instruction::Instruction {
    ix(
        HOOKS,
        token_hooks::accounts::UpdatePolicy { authority: *authority, mint: *mint, policy: policy(mint), extra_account_metas: metas(mint), system_program: SYS },
        versions_of(entries),
        token_hooks::instruction::UpdatePolicy { stack: stack(entries) },
    )
}

impl Svm {
    /// A Token-2022 mint with `exts` initialized by `init` before `initialize_mint2`.
    fn mint22(
        &mut self,
        payer: &Keypair,
        authority: &Pk,
        decimals: u8,
        exts: &[ExtensionType],
        init: impl Fn(&Pk) -> Vec<anchor_lang::solana_program::instruction::Instruction>,
    ) -> Pk {
        let mint = Keypair::new();
        let m = kp(&mint);
        let len = ExtensionType::try_calculate_account_len::<T22Mint>(exts).unwrap();
        let rent = self.svm.minimum_balance_for_rent_exemption(len);
        let mut ixs = vec![t22(anchor_lang::solana_program::system_instruction::create_account(&kp(payer), &m, rent, len as u64, &T22))];
        ixs.extend(init(&m).into_iter().map(t22));
        ixs.push(t22(spl_token_2022::instruction::initialize_mint2(&T22, &m, authority, None, decimals).unwrap()));
        self.send_ok(&ixs, &[payer, &mint]);
        m
    }

    fn hooked_mint(&mut self, payer: &Keypair, hook_program: &Pk) -> Pk {
        let a = kp(payer);
        let program = *hook_program;
        self.mint22(payer, &a, 6, &[ExtensionType::TransferHook], move |m| {
            vec![transfer_hook::instruction::initialize(&T22, m, Some(a), Some(program)).unwrap()]
        })
    }

    fn account22(&mut self, payer: &Keypair, mint: &Pk, owner: &Pk, mint_exts: &[ExtensionType]) -> Pk {
        let acct = Keypair::new();
        let a = kp(&acct);
        let exts = ExtensionType::get_required_init_account_extensions(mint_exts);
        let len = ExtensionType::try_calculate_account_len::<T22Account>(&exts).unwrap();
        let rent = self.svm.minimum_balance_for_rent_exemption(len);
        let ixs = [
            t22(anchor_lang::solana_program::system_instruction::create_account(&kp(payer), &a, rent, len as u64, &T22)),
            t22(spl_token_2022::instruction::initialize_account3(&T22, &a, mint, owner).unwrap()),
        ];
        self.send_ok(&ixs, &[payer, &acct]);
        a
    }

    fn mint22_to(&mut self, authority: &Keypair, mint: &Pk, to: &Pk, amount: u64) {
        let i = t22(spl_token_2022::instruction::mint_to(&T22, mint, to, &kp(authority), &[], amount).unwrap());
        self.send_ok(&[i], &[authority]);
    }
}

/// `transfer_checked` plus the accounts Token-2022 resolves from the mint's metas list.
fn transfer(mint: &Pk, from: &Pk, to: &Pk, owner: &Pk, amount: u64, extra: &[Meta]) -> solana_instruction::Instruction {
    let mut i = spl_token_2022::instruction::transfer_checked(&T22, from, mint, to, owner, &[], amount, 6).unwrap();
    i.accounts.push(Meta::new(policy(mint), false));
    i.accounts.extend_from_slice(extra);
    i.accounts.push(Meta::new_readonly(HOOKS, false));
    i.accounts.push(Meta::new_readonly(metas(mint), false));
    t22(i)
}

struct Holders {
    issuer: Keypair,
    alice: Keypair,
    bob: Keypair,
    mint: Pk,
    a: Pk,
    b: Pk,
}

fn hooked(t: &mut T) -> Holders {
    let issuer = t.s.funded(10);
    let alice = t.s.funded(10);
    let bob = t.s.funded(10);
    let mint = t.s.hooked_mint(&issuer, &HOOKS);
    let ext = [ExtensionType::TransferHook];
    let a = t.s.account22(&issuer, &mint, &kp(&alice), &ext);
    let b = t.s.account22(&issuer, &mint, &kp(&bob), &ext);
    t.s.mint22_to(&issuer, &mint, &a, 1_000);
    Holders { issuer, alice, bob, mint, a, b }
}

#[test]
fn transfer_hook_runs_the_token_stack_on_every_transfer() {
    let mut t = setup();
    let h = hooked(&mut t);
    let (iss, mint) = (kp(&h.issuer), h.mint);

    // No policy yet: Token-2022 cannot resolve the hook's accounts, so the mint is frozen in place.
    let send = |t: &mut T, amount: u64| t.s.send(&[transfer(&mint, &h.a, &h.b, &kp(&h.alice), amount, &[])], &[&h.alice]);
    let err = format!("{:?}", send(&mut t, 10).unwrap_err().err);
    assert!(err.contains("NotEnoughAccountKeys") || err.contains("MissingAccount") || err.contains("InvalidAccountData") || err.contains("Custom"), "{err}");

    // Only the transfer-hook authority can install, and only on a mint naming this program.
    let stranger = t.s.funded(1);
    let e = [max_size(&t, &mint, 100, TRANSFER)];
    let err = t.s.send_err(&[create_policy_ix(&kp(&stranger), &kp(&stranger), &mint, 0, &e)], &[&stranger]);
    assert!(err.contains("Unauthorized"), "{err}");
    let other = t.s.hooked_mint(&h.issuer, &Pk::new_unique());
    let err = t.s.send_err(&[create_policy_ix(&iss, &iss, &other, 0, &e)], &[&h.issuer]);
    assert!(err.contains("NotHookedMint"), "{err}");

    t.s.send_ok(&[create_policy_ix(&iss, &iss, &mint, 0, &e)], &[&h.issuer]);
    send(&mut t, 60).unwrap();
    assert_eq!(t.s.token_balance(&h.b), 60);
    let err = format!("{:?}", send(&mut t, 101).unwrap_err().meta.logs);
    assert!(err.contains("reason") && err.contains("PolicyRejected"), "{err}");

    // Calling Execute directly (not mid-transfer) is refused, so it cannot consume limits.
    let direct = ix(
        HOOKS,
        token_hooks::accounts::Execute { source: h.a, mint, destination: h.b, owner: kp(&h.alice), extra_account_metas: metas(&mint), policy: policy(&mint) },
        vec![],
        token_hooks::instruction::Execute { amount: 1 },
    );
    assert!(t.s.send_err(&[direct], &[&h.alice]).contains("NotTransferring"));

    // Add a daily spending limit and a recipient allowlist; the mandatory max-size entry stays.
    let carol = t.s.funded(1);
    let c = t.s.account22(&h.issuer, &mint, &kp(&carol), &[ExtensionType::TransferHook]);
    let e2 = [e[0], spending(&t, &mint, 120), recipients(&t, &kp(&h.bob), &kp(&h.issuer))];
    t.s.send_ok(&[update_policy_ix(&iss, &mint, &e2)], &[&h.issuer]);
    send(&mut t, 100).unwrap();
    let err = format!("{:?}", send(&mut t, 30).unwrap_err().meta.logs);
    assert!(err.contains("PolicyRejected") && err.contains("entry 1"), "spending limit: 100 + 30 > 120 per day: {err}");
    let to_carol = transfer(&mint, &h.a, &c, &kp(&h.alice), 1, &[]);
    assert!(t.s.send_err(&[to_carol.clone()], &[&h.alice]).contains("PolicyRejected"));
    t.s.warp_time(T0 + DAY + 1);
    send(&mut t, 30).unwrap();
    assert_eq!(t.s.token_balance(&h.b), 190);

    // The token engine refuses stacks Token-2022 could not feed (record-sourced allowlists).
    let mut rec = recipients(&t, &kp(&h.bob), &iss);
    rec.config[1] = builtin::SOURCE_RECORD;
    let err = t.s.send_err(&[update_policy_ix(&iss, &mint, &[e[0], rec])], &[&h.issuer]);
    assert!(err.contains("InvalidStack"), "{err}");
}

/// Gateway token (solana-gateway 0.5 Borsh layout) at the PDA Token-2022 derives from the metas.
fn gateway_token(s: &mut Svm, owner: &Pk, network: &Pk, expiry: i64) -> Pk {
    let mut d = vec![0u8, 0];
    d.extend_from_slice(owner.as_ref());
    d.push(0);
    d.extend_from_slice(network.as_ref());
    d.extend_from_slice(Pk::new_unique().as_ref());
    d.push(0); // active
    d.push(1);
    d.extend_from_slice(&expiry.to_le_bytes());
    let k = gateway_pda(owner, network);
    let lamports = s.svm.minimum_balance_for_rent_exemption(d.len());
    s.svm.set_account(addr(&k), solana_account::Account { lamports, data: d, owner: addr(&GATEWAY_PROGRAM), executable: false, rent_epoch: 0 }).unwrap();
    k
}

#[test]
fn credential_policy_resolves_gateway_tokens_from_extra_metas() {
    let mut t = setup();
    let h = hooked(&mut t);
    let (iss, mint) = (kp(&h.issuer), h.mint);
    let network = Pk::new_unique();
    let e = [credential(&t, &network)];
    t.s.send_ok(&[create_policy_ix(&iss, &iss, &mint, 0, &e)], &[&h.issuer]);

    let extra = [Meta::new_readonly(GATEWAY_PROGRAM, false), Meta::new_readonly(network, false), Meta::new_readonly(gateway_pda(&kp(&h.bob), &network), false)];
    let send = |t: &mut T, n: u64| t.s.send(&[transfer(&mint, &h.a, &h.b, &kp(&h.alice), n, &extra)], &[&h.alice]);
    let err = format!("{:?}", send(&mut t, 5).unwrap_err().meta.logs);
    assert!(err.contains("reason 16"), "no pass: {err}");
    gateway_token(&mut t.s, &kp(&h.bob), &network, T0 - 1);
    let err = format!("{:?}", send(&mut t, 6).unwrap_err().meta.logs);
    assert!(err.contains("reason 16"), "expired pass: {err}");
    gateway_token(&mut t.s, &kp(&h.bob), &network, T0 + DAY);
    send(&mut t, 7).unwrap();
    assert_eq!(t.s.token_balance(&h.b), 7);
    // A pass for the wrong wallet is a different PDA: Token-2022 refuses the substituted account.
    let forged = [extra[0].clone(), extra[1].clone(), Meta::new_readonly(gateway_token(&mut t.s, &kp(&h.alice), &network, T0 + DAY), false)];
    let err = format!("{:?}", t.s.send(&[transfer(&mint, &h.a, &h.b, &kp(&h.alice), 1, &forged)], &[&h.alice]).unwrap_err().meta.logs);
    assert!(!err.contains("PolicyRejected"), "refused by Token-2022's account resolution, before the hook: {err}");
}

// ---------------------------------------------------------------- wrapper

fn wrapper_keys(underlying: &Pk) -> (Pk, Pk, Pk) {
    let w = |seed: &[u8]| Pk::find_program_address(&[seed, underlying.as_ref()], &WRAPPER).0;
    (w(token_wrapper::WRAPPER_SEED), w(token_wrapper::MINT_SEED), w(token_wrapper::VAULT_SEED))
}

fn create_wrapper_ix(payer: &Pk, underlying: &Pk, program: &Pk, hook_authority: &Pk) -> solana_instruction::Instruction {
    let (wrapper, mint, vault) = wrapper_keys(underlying);
    ix(
        WRAPPER,
        token_wrapper::accounts::CreateWrapper {
            payer: *payer,
            underlying_mint: *underlying,
            wrapper,
            wrapped_mint: mint,
            vault,
            underlying_program: *program,
            token_2022_program: T22,
            system_program: SYS,
        },
        vec![],
        token_wrapper::instruction::CreateWrapper { hook_authority: *hook_authority },
    )
}

struct Wrapped {
    underlying: Pk,
    program: Pk,
    mint: Pk,
    vault: Pk,
    wrapper: Pk,
}

fn operate(w: &Wrapped, user: &Pk, user_underlying: &Pk, user_wrapped: &Pk, amount: u64, min_out: u64, wrap: bool) -> solana_instruction::Instruction {
    let accounts = token_wrapper::accounts::Operate {
        user: *user,
        wrapper: w.wrapper,
        underlying_mint: w.underlying,
        wrapped_mint: w.mint,
        vault: w.vault,
        user_underlying: *user_underlying,
        user_wrapped: *user_wrapped,
        policy: policy(&w.mint),
        underlying_program: w.program,
        token_2022_program: T22,
        token_hooks_program: HOOKS,
    };
    if wrap {
        ix(WRAPPER, accounts, vec![], token_wrapper::instruction::Wrap { amount, min_out })
    } else {
        ix(WRAPPER, accounts, vec![], token_wrapper::instruction::Unwrap { amount, min_out })
    }
}

fn verify_ix(w: &Wrapped) -> solana_instruction::Instruction {
    ix(WRAPPER, token_wrapper::accounts::VerifyReserves { wrapper: w.wrapper, wrapped_mint: w.mint, vault: w.vault }, vec![], token_wrapper::instruction::VerifyReserves {})
}

fn wrap_asset(t: &mut T, underlying: &Pk, program: &Pk, entries: &[StackEntry]) -> Wrapped {
    let admin = t.admin.insecure_clone();
    let a = kp(&admin);
    let (wrapper, mint, vault) = wrapper_keys(underlying);
    // The wrapper and its token policy are created atomically.
    t.s.send_ok(&[create_wrapper_ix(&a, underlying, program, &a), create_policy_ix(&a, &a, &mint, 0, entries)], &[&admin]);
    Wrapped { underlying: *underlying, program: *program, mint, vault, wrapper }
}

#[test]
fn wrapper_backs_every_wrapped_token_and_runs_the_hooks() {
    let mut t = setup();
    let issuer = t.s.funded(10);
    let alice = t.s.funded(10);
    let bob = t.s.funded(10);
    let bonk = t.s.create_mint(&issuer, 5);
    let alice_bonk = t.s.create_token_account(&issuer, &bonk, &kp(&alice));
    let bob_bonk = t.s.create_token_account(&issuer, &bonk, &kp(&bob));
    t.s.mint_to(&issuer, &bonk, &alice_bonk, 1_000_000);

    let (_, ph_mint, _) = wrapper_keys(&bonk);
    let e = [max_size(&t, &ph_mint, 500_000, WRAP | UNWRAP | TRANSFER)];
    let w = wrap_asset(&mut t, &bonk, &SPL, &e);
    let ext = [ExtensionType::TransferHook];
    let alice_ph = t.s.account22(&issuer, &w.mint, &kp(&alice), &ext);
    let bob_ph = t.s.account22(&issuer, &w.mint, &kp(&bob), &ext);
    let wr: token_wrapper::Wrapper = t.s.get(&w.wrapper);
    assert_eq!((wr.decimals, wr.wrapped_mint, wr.vault), (5, w.mint, w.vault));

    // Wrap: the vault's measured increase is minted, and the hook stack runs first.
    let i = operate(&w, &kp(&alice), &alice_bonk, &alice_ph, 600_000, 0, true);
    assert!(t.s.send_err(&[i], &[&alice]).contains("PolicyRejected"), "max-size on wrap");
    t.s.send_ok(&[operate(&w, &kp(&alice), &alice_bonk, &alice_ph, 400_000, 400_000, true)], &[&alice]);
    assert_eq!((t.s.token_balance(&alice_ph), t.s.token_balance(&w.vault)), (400_000, 400_000));
    t.s.send_ok(&[verify_ix(&w)], &[&alice]);

    // Wrapped tokens move with the transfer hook; the original token is untouched by it.
    let i = spl_token_2022::instruction::transfer_checked(&T22, &alice_ph, &w.mint, &bob_ph, &kp(&alice), &[], 150_000, 5).unwrap();
    let mut i = t22(i);
    for m in [Meta::new(policy(&w.mint), false), Meta::new_readonly(HOOKS, false), Meta::new_readonly(metas(&w.mint), false)] {
        i.accounts.push(solana_instruction::AccountMeta { pubkey: addr(&m.pubkey), is_signer: false, is_writable: m.is_writable });
    }
    t.s.send_ok(&[i], &[&alice]);

    // Unwrap burns first, then releases the same amount.
    t.s.send_ok(&[operate(&w, &kp(&bob), &bob_bonk, &bob_ph, 150_000, 150_000, false)], &[&bob]);
    assert_eq!((t.s.token_balance(&bob_bonk), t.s.token_balance(&bob_ph)), (150_000, 0));
    assert_eq!(t.s.token_balance(&w.vault), 250_000);
    let i = operate(&w, &kp(&bob), &bob_bonk, &bob_ph, 1, 0, false);
    assert!(t.s.send_err(&[i], &[&bob]).contains("insufficient funds"), "cannot unwrap without wrapped balance");
    // Burning someone else's wrapped tokens needs their signature.
    let i = operate(&w, &kp(&bob), &bob_bonk, &alice_ph, 1, 0, false);
    assert!(t.s.send_err(&[i], &[&bob]).contains("owner does not match"));

    // Nobody but the wrapper PDA can mint wrapped tokens or drive the wrap callbacks.
    let i = t22(spl_token_2022::instruction::mint_to(&T22, &w.mint, &alice_ph, &kp(&issuer), &[], 1).unwrap());
    assert!(t.s.send_err(&[i], &[&issuer]).contains("owner does not match"));
    let i = ix(
        HOOKS,
        token_hooks::accounts::CheckOperation { mint_authority: kp(&issuer), mint: w.mint, policy: policy(&w.mint) },
        vec![],
        token_hooks::instruction::CheckOperation { callback: token_hooks::callback::BEFORE_WRAP, actor: kp(&issuer), counterparty: kp(&issuer), amount: 1 },
    );
    assert!(t.s.send_err(&[i], &[&issuer]).contains("Unauthorized"));

    // A deposit sent straight to the vault is a surplus; reserves still cover supply.
    let i = t22(spl_token_2022::instruction::transfer_checked(&SPL, &alice_bonk, &bonk, &w.vault, &kp(&alice), &[], 1, 5).unwrap());
    t.s.send_ok(&[i], &[&alice]);
    t.s.send_ok(&[verify_ix(&w)], &[&alice]);
    t.s.send_ok(&[operate(&w, &kp(&alice), &alice_bonk, &alice_ph, 250_000, 250_000, false)], &[&alice]);
    assert_eq!(t.s.token_balance(&w.vault), 1);
}

#[test]
fn wrapper_refuses_unsafe_underlyings_and_measures_transfer_fees() {
    let mut t = setup();
    let issuer = t.s.funded(10);
    let alice = t.s.funded(10);
    let i = kp(&issuer);

    // A permanent delegate could drain the vault: refused.
    let risky = t.s.mint22(&issuer, &i, 6, &[ExtensionType::PermanentDelegate], move |m| {
        vec![spl_token_2022::instruction::initialize_permanent_delegate(&T22, m, &i).unwrap()]
    });
    let admin = t.admin.insecure_clone();
    let err = t.s.send_err(&[create_wrapper_ix(&kp(&admin), &risky, &T22, &kp(&admin))], &[&admin]);
    assert!(err.contains("UnsupportedExtension"), "{err}");
    // So is an underlying with its own transfer hook.
    let hooked = t.s.hooked_mint(&issuer, &Pk::new_unique());
    let err = t.s.send_err(&[create_wrapper_ix(&kp(&admin), &hooked, &T22, &kp(&admin))], &[&admin]);
    assert!(err.contains("UnsupportedExtension"), "{err}");
    // Wrong token program for the mint.
    let spl = t.s.create_mint(&issuer, 6);
    let err = t.s.send_err(&[create_wrapper_ix(&kp(&admin), &spl, &T22, &kp(&admin))], &[&admin]);
    assert!(err.contains("WrongTokenProgram"), "{err}");

    // 1% transfer fee: only what the vault measurably received is minted.
    let fee_exts = [ExtensionType::TransferFeeConfig];
    let feed = t.s.mint22(&issuer, &i, 6, &fee_exts, move |m| {
        vec![transfer_fee::instruction::initialize_transfer_fee_config(&T22, m, Some(&i), Some(&i), 100, u64::MAX).unwrap()]
    });
    let alice_u = t.s.account22(&issuer, &feed, &kp(&alice), &fee_exts);
    t.s.mint22_to(&issuer, &feed, &alice_u, 1_000_000);
    let w = wrap_asset(&mut t, &feed, &T22, &[]);
    let alice_w = t.s.account22(&issuer, &w.mint, &kp(&alice), &[ExtensionType::TransferHook]);
    let i = operate(&w, &kp(&alice), &alice_u, &alice_w, 100_000, 99_001, true);
    assert!(t.s.send_err(&[i], &[&alice]).contains("SlippageExceeded"));
    t.s.send_ok(&[operate(&w, &kp(&alice), &alice_u, &alice_w, 100_000, 99_000, true)], &[&alice]);
    assert_eq!((t.s.token_balance(&alice_w), t.s.token_balance(&w.vault)), (99_000, 99_000));
    t.s.send_ok(&[operate(&w, &kp(&alice), &alice_u, &alice_w, 99_000, 98_010, false)], &[&alice]);
    assert_eq!((t.s.token_balance(&alice_w), t.s.token_balance(&w.vault)), (0, 0));
    t.s.send_ok(&[verify_ix(&w)], &[&alice]);
}
