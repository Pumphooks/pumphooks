//! Compute-unit measurements for hook stacks.
//!
//! - `unit`: `token_hooks::execute` run directly in Mollusk with one stack per case (the source
//!   account carries Token-2022's `transferring` flag, as during a real transfer). The marginal
//!   cost of a builtin is its case minus the empty stack.
//! - `transaction`: whole `transfer_checked` transactions in LiteSVM against the bundled
//!   Token-2022 program: no hook, the hook with an empty policy, and the hook with seven builtins.
//!
//! Every number is measured by the runtime; nothing is estimated. Policy stacks for the
//! non-empty cases are written into the policy account directly (the registry install path is
//! covered by `tests/hooks/tests/tokens.rs`); evaluation cost does not depend on how the stack
//! was installed.

#![allow(deprecated)]

use std::path::{Path, PathBuf};

use anchor_lang::{
    prelude::Pubkey as APubkey, solana_program::program_option::COption, Discriminator, InstructionData,
    ToAccountMetas,
};
use anchor_spl::token_2022::spl_token_2022::{
    self,
    extension::{
        transfer_hook::{TransferHook, TransferHookAccount},
        BaseStateWithExtensionsMut, ExtensionType, StateWithExtensionsMut,
    },
    state::{Account as T22Account, AccountState, Mint as T22Mint},
};
use bytemuck::Zeroable;
use hook_kernel::builtin::{self, *};
use hook_standard::{Environment, HookStack, StackEntry, IMPL_BUILTIN};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use solana_account::Account;
use solana_pubkey::Pubkey;

pub const RUNTIME: &str = "mollusk-svm 0.15.1 (unit), litesvm 0.16.0 with bundled Token-2022 11.0 (transaction)";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Measurement {
    pub name: String,
    pub compute_units: u64,
    pub ok: bool,
    /// Kernel's static estimate for the builtin(s) in this case, when there is one.
    pub kernel_estimate: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Results {
    pub runtime: String,
    pub programs: Vec<(String, String)>,
    pub unit: Vec<Measurement>,
    pub transaction: Vec<Measurement>,
}

impl Results {
    pub fn get(&self, name: &str) -> Option<&Measurement> {
        self.unit.iter().chain(&self.transaction).find(|m| m.name == name)
    }
}

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn program_path(name: &str) -> PathBuf {
    root().join("target/deploy").join(format!("{name}.so"))
}

fn sha256_file(p: &Path) -> String {
    let bytes = std::fs::read(p).unwrap_or_else(|_| panic!("missing {} (run scripts/build_programs.sh)", p.display()));
    Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn pk(a: &APubkey) -> Pubkey {
    Pubkey::new_from_array(a.to_bytes())
}

// ---------------------------------------------------------------- shared fixtures

pub struct Parties {
    pub mint: APubkey,
    pub alice: APubkey,
    pub bob: APubkey,
    pub network: APubkey,
}

impl Default for Parties {
    fn default() -> Self {
        Self { mint: APubkey::new_from_array([1; 32]), alice: APubkey::new_from_array([2; 32]), bob: APubkey::new_from_array([3; 32]), network: APubkey::new_from_array([4; 32]) }
    }
}

fn entry<C: bytemuck::Pod>(id: u8, cfg: C) -> StackEntry {
    let mut e = StackEntry::zeroed();
    e.hook_version = [id; 32];
    e.impl_kind = IMPL_BUILTIN;
    e.builtin_id = id;
    e.callbacks = 1; // before_transfer
    e.granted = builtin::required_caps(id);
    e.config[..core::mem::size_of::<C>()].copy_from_slice(bytemuck::bytes_of(&cfg));
    e
}

/// One passing entry per token builtin, configured so a 100-unit transfer from alice to bob is allowed.
pub fn builtin_cases(p: &Parties) -> Vec<(&'static str, StackEntry)> {
    let mint = p.mint.to_bytes();
    let mut cred = CredentialConfig::zeroed();
    cred.target = TARGET_COUNTERPARTY;
    cred.gatekeeper_network = p.network.to_bytes();
    vec![
        ("allowlist", entry(ALLOWLIST, AllowlistConfig { target: TARGET_COUNTERPARTY, source: SOURCE_INLINE, _pad: [0; 6], max_age_slots: 0, key_a: p.bob.to_bytes(), key_b: [0; 32], _rest: [0; 16] })),
        ("denylist", entry(DENYLIST, DenylistConfig { target: TARGET_COUNTERPARTY, _pad: [0; 7], key_a: [9; 32], key_b: [0; 32], _rest: [0; 24] })),
        ("max-size", entry(MAX_SIZE, MaxSizeConfig { asset: mint, max_amount: 1_000_000, _rest: [0; 56] })),
        ("spending-limit", entry(SPENDING_LIMIT, SpendingLimitConfig { asset: mint, limit: 1_000_000, period_secs: 86_400, _pad: 0, _rest: [0; 48] })),
        ("time-window", entry(TIME_WINDOW, TimeWindowConfig { not_before: 0, not_after: i64::MAX, daily_start: 0, daily_end: 0, _rest: [0; 72] })),
        ("timelock", entry(TIMELOCK, TimelockConfig { delay_secs: 0, _pad: 0, not_before: 1, _rest: [0; 80] })),
        ("accounting", entry(ACCOUNTING, [0u8; 96])),
        ("credential", entry(CREDENTIAL, cred)),
    ]
}

pub fn stack_of(entries: &[StackEntry]) -> HookStack {
    let mut s = HookStack::empty(Environment::Token);
    s.len = entries.len() as u8;
    s.entries[..entries.len()].copy_from_slice(entries);
    s
}

fn policy_pda(mint: &APubkey) -> (APubkey, u8) {
    APubkey::find_program_address(&[token_hooks::POLICY_SEED, mint.as_ref()], &token_hooks::ID)
}
fn metas_pda(mint: &APubkey) -> APubkey {
    APubkey::find_program_address(&[token_hooks::EXTRA_METAS_SEED, mint.as_ref()], &token_hooks::ID).0
}

pub fn policy_data(mint: &APubkey, stack: &HookStack) -> Vec<u8> {
    let (_, bump) = policy_pda(mint);
    let metas_bump = APubkey::find_program_address(&[token_hooks::EXTRA_METAS_SEED, mint.as_ref()], &token_hooks::ID).1;
    let mut d = token_hooks::TokenPolicy::DISCRIMINATOR.to_vec();
    d.extend_from_slice(mint.as_ref());
    d.extend_from_slice(&[0; 32]); // authority
    d.extend_from_slice(&[0; 32]); // pending_hash
    d.extend_from_slice(&[0; 24]); // sequence, delay, pending_at
    d.push(bump);
    d.push(metas_bump);
    d.extend_from_slice(&[0; 6]);
    d.extend_from_slice(bytemuck::bytes_of(stack));
    d
}

fn hooked_mint_data(authority: &APubkey) -> Vec<u8> {
    let len = ExtensionType::try_calculate_account_len::<T22Mint>(&[ExtensionType::TransferHook]).unwrap();
    let mut d = vec![0u8; len];
    let mut s = StateWithExtensionsMut::<T22Mint>::unpack_uninitialized(&mut d).unwrap();
    let h = s.init_extension::<TransferHook>(true).unwrap();
    h.program_id = Some(token_hooks::ID).try_into().unwrap();
    s.base = T22Mint { mint_authority: COption::Some(*authority), supply: 1_000_000, decimals: 6, is_initialized: true, freeze_authority: COption::None };
    s.pack_base();
    s.init_account_type().unwrap();
    d
}

fn hooked_account_data(mint: &APubkey, owner: &APubkey, amount: u64, transferring: bool) -> Vec<u8> {
    let len = ExtensionType::try_calculate_account_len::<T22Account>(&[ExtensionType::TransferHookAccount]).unwrap();
    let mut d = vec![0u8; len];
    let mut s = StateWithExtensionsMut::<T22Account>::unpack_uninitialized(&mut d).unwrap();
    let e = s.init_extension::<TransferHookAccount>(true).unwrap();
    e.transferring = transferring.into();
    s.base = T22Account { mint: *mint, owner: *owner, amount, state: AccountState::Initialized, ..Default::default() };
    s.pack_base();
    s.init_account_type().unwrap();
    d
}

/// Gateway token (solana-gateway 0.5 layout) for `owner` on `network`, active, far expiry.
pub fn gateway_token_data(owner: &APubkey, network: &APubkey) -> Vec<u8> {
    let mut d = vec![0u8, 0];
    d.extend_from_slice(owner.as_ref());
    d.push(0);
    d.extend_from_slice(network.as_ref());
    d.extend_from_slice(&[7; 32]);
    d.push(0);
    d.push(1);
    d.extend_from_slice(&i64::MAX.to_le_bytes());
    d
}

fn acct(owner: &APubkey, data: Vec<u8>) -> Account {
    Account { lamports: 10_000_000, data, owner: pk(owner), executable: false, rent_epoch: 0 }
}

// ---------------------------------------------------------------- unit: Mollusk

pub fn mollusk() -> mollusk_svm::Mollusk {
    let elf = std::fs::read(program_path("token_hooks")).expect("run scripts/build_programs.sh first");
    let mut m = mollusk_svm::Mollusk::default();
    m.add_program_with_loader_and_elf(&pk(&token_hooks::ID), &mollusk_svm::program::loader_keys::LOADER_V3, &elf);
    m.sysvars.clock.unix_timestamp = NOW;
    m
}

/// Clock used by both runtimes, so time-based builtins see the same instant.
pub const NOW: i64 = 1_800_000_000;

/// Runs `token_hooks::execute` for a 100-unit alice -> bob transfer under `entries`.
pub fn execute_case(m: &mollusk_svm::Mollusk, p: &Parties, name: &str, entries: &[StackEntry], estimate: Option<u64>) -> Measurement {
    let t22 = spl_token_2022::ID;
    let (policy, _) = policy_pda(&p.mint);
    let source = APubkey::new_from_array([11; 32]);
    let dest = APubkey::new_from_array([12; 32]);
    let gateway = APubkey::find_program_address(&[p.bob.as_ref(), b"gateway", &[0; 8], p.network.as_ref()], &GATEWAY_PROGRAM).0;
    let stack = stack_of(entries);
    let accounts = token_hooks::accounts::Execute { source, mint: p.mint, destination: dest, owner: p.alice, extra_account_metas: metas_pda(&p.mint), policy };
    let mut metas = accounts.to_account_metas(None);
    let creds = entries.iter().any(|e| e.builtin_id == CREDENTIAL);
    if creds {
        metas.push(anchor_lang::solana_program::instruction::AccountMeta::new_readonly(GATEWAY_PROGRAM, false));
        metas.push(anchor_lang::solana_program::instruction::AccountMeta::new_readonly(p.network, false));
        metas.push(anchor_lang::solana_program::instruction::AccountMeta::new_readonly(gateway, false));
    }
    let ix = solana_instruction::Instruction {
        program_id: pk(&token_hooks::ID),
        accounts: metas.iter().map(|a| solana_instruction::AccountMeta { pubkey: pk(&a.pubkey), is_signer: a.is_signer, is_writable: a.is_writable }).collect(),
        data: token_hooks::instruction::Execute { amount: 100 }.data(),
    };
    let mut accts = vec![
        (pk(&source), acct(&t22, hooked_account_data(&p.mint, &p.alice, 1_000, true))),
        (pk(&p.mint), acct(&t22, hooked_mint_data(&p.alice))),
        (pk(&dest), acct(&t22, hooked_account_data(&p.mint, &p.bob, 0, false))),
        (pk(&p.alice), acct(&anchor_lang::system_program::ID, vec![])),
        (pk(&metas_pda(&p.mint)), acct(&token_hooks::ID, vec![0; 8])),
        (pk(&policy), acct(&token_hooks::ID, policy_data(&p.mint, &stack))),
    ];
    if creds {
        accts.push((pk(&GATEWAY_PROGRAM), Account { lamports: 1, data: vec![], owner: pk(&anchor_lang::solana_program::bpf_loader_upgradeable::ID), executable: true, rent_epoch: 0 }));
        accts.push((pk(&p.network), acct(&anchor_lang::system_program::ID, vec![])));
        accts.push((pk(&gateway), acct(&GATEWAY_PROGRAM, gateway_token_data(&p.bob, &p.network))));
    }
    let r = m.process_instruction(&ix, &accts);
    Measurement { name: name.into(), compute_units: r.compute_units_consumed, ok: r.program_result.is_ok(), kernel_estimate: estimate }
}

pub fn unit() -> Vec<Measurement> {
    let m = mollusk();
    let p = Parties::default();
    let run = |name: &str, entries: &[StackEntry], estimate: Option<u64>| execute_case(&m, &p, name, entries, estimate);

    let mut out = vec![run("execute: empty stack", &[], None)];
    let cases = builtin_cases(&p);
    for (name, e) in &cases {
        out.push(run(&format!("execute: {name}"), &[*e], Some(builtin::compute_estimate(e.builtin_id) as u64)));
    }
    let all: Vec<StackEntry> = cases.iter().map(|(_, e)| *e).collect();
    let est = all.iter().map(|e| builtin::compute_estimate(e.builtin_id) as u64).sum();
    out.push(run("execute: all 8 token builtins", &all, Some(est)));
    out
}

// ---------------------------------------------------------------- transaction: LiteSVM

pub fn transaction() -> Vec<Measurement> {
    use litesvm::LiteSVM;
    use solana_address::Address;
    use solana_keypair::Keypair;
    use solana_signer::Signer;
    use solana_transaction::Transaction;

    let addr = |p: &APubkey| Address::new_from_array(p.to_bytes());
    let conv = |i: anchor_lang::solana_program::instruction::Instruction| solana_instruction::Instruction {
        program_id: addr(&i.program_id),
        accounts: i.accounts.iter().map(|a| solana_instruction::AccountMeta { pubkey: addr(&a.pubkey), is_signer: a.is_signer, is_writable: a.is_writable }).collect(),
        data: i.data,
    };
    let mut svm = LiteSVM::new();
    svm.add_program_from_file(addr(&token_hooks::ID), program_path("token_hooks")).unwrap();
    let mut clock = svm.get_sysvar::<solana_clock::Clock>();
    clock.unix_timestamp = NOW;
    svm.set_sysvar(&clock);
    let payer = Keypair::new_from_array([20; 32]);
    svm.airdrop(&payer.pubkey(), 100_000_000_000).unwrap();
    let pay = APubkey::new_from_array(payer.pubkey().to_bytes());
    let t22 = spl_token_2022::ID;

    let send = |svm: &mut LiteSVM, ixs: Vec<solana_instruction::Instruction>, extra: &[&Keypair]| -> (u64, bool) {
        let mut signers: Vec<&Keypair> = vec![&payer];
        signers.extend_from_slice(extra);
        let tx = Transaction::new_signed_with_payer(&ixs, Some(&payer.pubkey()), &signers, svm.latest_blockhash());
        let r = svm.send_transaction(tx);
        svm.expire_blockhash();
        match r {
            Ok(m) => (m.compute_units_consumed, true),
            Err(e) => (e.meta.compute_units_consumed, false),
        }
    };

    let mint_with = |svm: &mut LiteSVM, hooked: bool| -> (APubkey, APubkey, APubkey) {
        let seed = if hooked { 30 } else { 40 };
        let mint = Keypair::new_from_array([seed; 32]);
        let m = APubkey::new_from_array(mint.pubkey().to_bytes());
        let exts: &[ExtensionType] = if hooked { &[ExtensionType::TransferHook] } else { &[] };
        let len = ExtensionType::try_calculate_account_len::<T22Mint>(exts).unwrap();
        let mut ixs = vec![conv(anchor_lang::solana_program::system_instruction::create_account(&pay, &m, svm.minimum_balance_for_rent_exemption(len), len as u64, &t22))];
        if hooked {
            ixs.push(conv(spl_token_2022::extension::transfer_hook::instruction::initialize(&t22, &m, Some(pay), Some(token_hooks::ID)).unwrap()));
        }
        ixs.push(conv(spl_token_2022::instruction::initialize_mint2(&t22, &m, &pay, None, 6).unwrap()));
        let acct_exts = ExtensionType::get_required_init_account_extensions(exts);
        let alen = ExtensionType::try_calculate_account_len::<T22Account>(&acct_exts).unwrap();
        let (a, b) = (Keypair::new_from_array([seed + 1; 32]), Keypair::new_from_array([seed + 2; 32]));
        let (ak, bk) = (APubkey::new_from_array(a.pubkey().to_bytes()), APubkey::new_from_array(b.pubkey().to_bytes()));
        for k in [&ak, &bk] {
            ixs.push(conv(anchor_lang::solana_program::system_instruction::create_account(&pay, k, svm.minimum_balance_for_rent_exemption(alen), alen as u64, &t22)));
            ixs.push(conv(spl_token_2022::instruction::initialize_account3(&t22, k, &m, &pay).unwrap()));
        }
        ixs.push(conv(spl_token_2022::instruction::mint_to(&t22, &m, &ak, &pay, &[], 1_000_000).unwrap()));
        assert!(send(svm, ixs, &[&mint, &a, &b]).1, "setup");
        (m, ak, bk)
    };

    let transfer = |m: &APubkey, a: &APubkey, b: &APubkey, hooked: bool| {
        let mut i = spl_token_2022::instruction::transfer_checked(&t22, a, m, b, &pay, &[], 100, 6).unwrap();
        if hooked {
            i.accounts.push(anchor_lang::solana_program::instruction::AccountMeta::new(policy_pda(m).0, false));
            i.accounts.push(anchor_lang::solana_program::instruction::AccountMeta::new_readonly(token_hooks::ID, false));
            i.accounts.push(anchor_lang::solana_program::instruction::AccountMeta::new_readonly(metas_pda(m), false));
        }
        conv(i)
    };

    let mut out = vec![];
    let (m, a, b) = mint_with(&mut svm, false);
    let (cu, ok) = send(&mut svm, vec![transfer(&m, &a, &b, false)], &[]);
    out.push(Measurement { name: "transfer_checked: Token-2022, no hook".into(), compute_units: cu, ok, kernel_estimate: None });

    let (m, a, b) = mint_with(&mut svm, true);
    let empty = stack_of(&[]);
    let create = token_hooks::instruction::CreatePolicy { delay: 0, stack: empty.compact().to_vec() };
    let accounts = token_hooks::accounts::CreatePolicy { payer: pay, authority: pay, mint: m, policy: policy_pda(&m).0, extra_account_metas: metas_pda(&m), system_program: anchor_lang::system_program::ID };
    let ix = anchor_lang::solana_program::instruction::Instruction { program_id: token_hooks::ID, accounts: accounts.to_account_metas(None), data: create.data() };
    assert!(send(&mut svm, vec![conv(ix)], &[]).1, "create_policy");
    let (cu, ok) = send(&mut svm, vec![transfer(&m, &a, &b, true)], &[]);
    out.push(Measurement { name: "transfer_checked: token-hooks, empty policy".into(), compute_units: cu, ok, kernel_estimate: None });

    // Seven builtins: every passing token builtin except credential (its accounts come from the metas list).
    let p = Parties { mint: m, alice: pay, bob: pay, network: APubkey::new_from_array([4; 32]) };
    let seven: Vec<StackEntry> = builtin_cases(&p).into_iter().filter(|(n, _)| *n != "credential").map(|(_, e)| e).collect();
    let key = addr(&policy_pda(&m).0);
    let mut acc = svm.get_account(&key).unwrap();
    acc.data = policy_data(&m, &stack_of(&seven));
    svm.set_account(key, acc).unwrap();
    let est = seven.iter().map(|e| builtin::compute_estimate(e.builtin_id) as u64).sum();
    let (cu, ok) = send(&mut svm, vec![transfer(&m, &a, &b, true)], &[]);
    out.push(Measurement { name: "transfer_checked: token-hooks, 7 builtins".into(), compute_units: cu, ok, kernel_estimate: Some(est) });
    out
}

pub fn measure() -> Results {
    Results {
        runtime: RUNTIME.into(),
        programs: vec![("token_hooks".into(), sha256_file(&program_path("token_hooks")))],
        unit: unit(),
        transaction: transaction(),
    }
}

/// Markdown report. Marginal costs are relative to the empty stack.
pub fn report(r: &Results) -> String {
    let mut s = String::new();
    s.push_str("# Hook compute benchmarks\n\n");
    s.push_str("Generated by `cargo run -p hook-bench --release` (`bench/`). Every number below was measured by the runtime;\n");
    s.push_str("the kernel's static estimates (`hook_kernel::builtin::compute_estimate`) are shown next to them for comparison.\n\n");
    s.push_str(&format!("Runtime: {}.\n\n", r.runtime));
    for (name, h) in &r.programs {
        s.push_str(&format!("- `{name}.so` sha256 `{h}`\n"));
    }
    let base = r.get("execute: empty stack").map(|m| m.compute_units).unwrap_or(0);
    s.push_str("\n## token-hooks `Execute` (Mollusk)\n\n");
    s.push_str("One stack per row, evaluated on `before_transfer` for a 100-unit transfer that every entry allows.\n\n");
    s.push_str("| Case | CU | Marginal vs empty | Kernel estimate | Result |\n| --- | ---: | ---: | ---: | --- |\n");
    for m in &r.unit {
        let marginal = if m.name == "execute: empty stack" { "—".to_string() } else { format!("{}", m.compute_units as i64 - base as i64) };
        let est = m.kernel_estimate.map(|e| e.to_string()).unwrap_or_else(|| "—".into());
        s.push_str(&format!("| {} | {} | {} | {} | {} |\n", m.name.trim_start_matches("execute: "), m.compute_units, marginal, est, if m.ok { "ok" } else { "**failed**" }));
    }
    s.push_str("\n## Token-2022 transfers (LiteSVM)\n\n");
    s.push_str("Whole single-instruction transactions: Token-2022 `transfer_checked` plus, when hooked, its CPI into token-hooks.\n\n");
    s.push_str("| Transaction | CU | Result |\n| --- | ---: | --- |\n");
    for m in &r.transaction {
        s.push_str(&format!("| {} | {} | {} |\n", m.name.trim_start_matches("transfer_checked: "), m.compute_units, if m.ok { "ok" } else { "**failed**" }));
    }
    s.push_str("\n## Notes\n\n");
    s.push_str("- Non-empty stacks are written into the policy account directly; installing through the registry is tested in `tests/hooks/tests/tokens.rs` and does not change evaluation cost.\n");
    s.push_str("- In the transaction rows, the hooked total minus the plain transfer minus `execute: empty stack` is Token-2022's own cost of resolving the extra-account-metas list and invoking the hook.\n");
    s.push_str("- The credential row includes reading one gateway-token account. The transaction rows omit credential, because its accounts come from the extra-account-metas list.\n");
    s.push_str("- Regression gate: `cargo test -p hook-bench` fails if any case exceeds `bench/baseline.json` by more than 5%.\n");
    s
}
