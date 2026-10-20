//! Venue verification: `hook-router` against the real Raydium CPMM and Meteora DAMM v2 binaries,
//! both the devnet and the mainnet builds (`scripts/dump_programs.sh`).
//!
//! - CPMM: pools are created with the venue's own `initialize`, using the network's real AmmConfig
//!   and create-pool fee receiver (`scripts/fetch_venue_fixtures.py`).
//! - DAMM v2: a funded pool cloned from each network (pool, vaults, mints).
//!
//! Each test routes swaps in both directions, checks the receipt against measured balances, and
//! checks that the router's own limits (min out, max in, SOL taken) bind independently of the
//! venue's.

mod common;

use std::{path::PathBuf, str::FromStr};

use anchor_lang::solana_program::{instruction::AccountMeta as Meta, sysvar};
use base64::Engine;
use common::{tokens::SPL, *};
use hook_router::{accounts as A, instruction as I, venues, Receipt, RouteParams};
use solana_account::Account;
use solana_address::Address;
use solana_instruction::Instruction;
use solana_keypair::Keypair;

const ID: Pk = hook_router::ID;
const SYS: Pk = anchor_lang::system_program::ID;
const ATA: Pk = pump_adapter::ATA_PROGRAM_ID;
const TOKEN_2022: Pk = anchor_spl::token_2022::ID;

#[derive(Clone, Copy, PartialEq)]
enum Net {
    Devnet,
    Mainnet,
}

impl Net {
    fn name(self) -> &'static str {
        match self {
            Net::Devnet => "devnet",
            Net::Mainnet => "mainnet",
        }
    }
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

fn load_program(s: &mut Svm, id: &Pk, file: &str) {
    let path = fixtures().join("programs").join(file);
    s.svm
        .add_program_from_file(addr(id), &path)
        .unwrap_or_else(|_| panic!("missing {} (scripts/dump_programs.sh)", path.display()));
}

fn manifest(net: Net, venue: &str) -> (PathBuf, serde_json::Value) {
    let dir = fixtures().join("venues").join(net.name()).join(venue);
    let m = std::fs::read_to_string(dir.join("manifest.json"))
        .unwrap_or_else(|_| panic!("missing {} (scripts/fetch_venue_fixtures.py)", dir.display()));
    (dir, serde_json::from_str(&m).unwrap())
}

fn clone_account(s: &mut Svm, dir: &PathBuf, key: &str) -> Pk {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(format!("{key}.json"))).unwrap()).unwrap();
    let a = &v["account"];
    let data = base64::engine::general_purpose::STANDARD.decode(a["data"][0].as_str().unwrap()).unwrap();
    let k = Pk::from_str(key).unwrap();
    let acct = Account {
        lamports: a["lamports"].as_u64().unwrap(),
        data,
        owner: Address::from_str(a["owner"].as_str().unwrap()).unwrap(),
        executable: false,
        rent_epoch: 0,
    };
    s.svm.set_account(addr(&k), acct).unwrap();
    k
}

fn key(m: &serde_json::Value, field: &str) -> Pk {
    Pk::from_str(m[field].as_str().unwrap()).unwrap()
}

fn disc(name: &str) -> [u8; 8] {
    let h = anchor_lang::solana_program::hash::hash(format!("global:{name}").as_bytes());
    h.to_bytes()[..8].try_into().unwrap()
}

/// Router deployed, initialized, `venue` enabled; clock at the current wall time.
fn router(venue: u8) -> (Svm, Keypair) {
    let mut s = Svm::new();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let mut clock: solana_clock::Clock = s.svm.get_sysvar();
    clock.unix_timestamp = now;
    clock.slot = 400_000_000;
    s.svm.set_sysvar(&clock);
    let admin = s.funded(1_000);
    s.add_program(&ID, "hook_router.so");
    s.set_upgrade_authority(&ID, Some(kp(&admin)));
    let config = Pk::find_program_address(&[hook_router::CONFIG_SEED], &ID).0;
    let init = ix(ID, A::Initialize { admin: kp(&admin), config, router_program: ID, router_program_data: programdata(&ID), system_program: SYS }, vec![], I::Initialize {});
    let enable = ix(ID, A::AdminOnly { admin: kp(&admin), config }, vec![], I::SetVenue { venue, enabled: true });
    s.send_ok(&[init, enable], &[&admin]);
    (s, admin)
}

struct Swap {
    program: Pk,
    metas: Vec<Meta>,
    data: Vec<u8>,
    input: Pk,
    output: Pk,
}

fn route_ix(s: &Svm, user: &Pk, sw: &Swap, max_in: u64, min_out: u64, max_lamports: u64, nonce: u64) -> (Instruction, Pk) {
    let config = Pk::find_program_address(&[hook_router::CONFIG_SEED], &ID).0;
    let receipt = Pk::find_program_address(&[hook_router::RECEIPT_SEED, user.as_ref(), &nonce.to_le_bytes()], &ID).0;
    let mut extra = sw.metas.clone();
    extra.push(Meta::new_readonly(sw.program, false));
    let _ = s;
    let i = ix(
        ID,
        A::Route { user: *user, config, policy: Some(ID), user_input: sw.input, user_output: sw.output, receipt, system_program: SYS },
        extra,
        I::Route {
            p: RouteParams { venue_accounts: sw.metas.len() as u8, max_in, min_out, max_lamports, deadline: i64::MAX, nonce, data: sw.data.clone() },
        },
    );
    (i, receipt)
}

/// Routes `sw` and checks the receipt against measured balances; returns (spent, received).
fn route_and_check(s: &mut Svm, user: &Keypair, sw: &Swap, venue: u8, nonce: u64) -> (u64, u64) {
    let (in0, out0, sol0) = (s.token_balance(&sw.input), s.token_balance(&sw.output), s.lamports(&kp(user)));
    let (i, receipt) = route_ix(s, &kp(user), sw, u64::MAX, 1, 0, nonce);
    s.send_ok(&[i], &[user]);
    let r: Receipt = s.get(&receipt);
    let (in1, out1) = (s.token_balance(&sw.input), s.token_balance(&sw.output));
    assert_eq!(r.venue, venue);
    assert!(r.venue_verified, "only venues exercised here may be marked verified");
    assert_eq!(r.spent, in0 - in1, "receipt spent = measured input delta");
    assert_eq!(r.received, out1 - out0, "receipt received = measured output delta");
    assert!(r.spent > 0 && r.received > 0);
    assert_eq!(r.lamports_spent, 0, "the venue took no SOL from the wallet");
    // Only the receipt rent and the fee left the wallet.
    assert!(sol0 - s.lamports(&kp(user)) < SOL / 100);
    (r.spent, r.received)
}

fn router_limits_bind(s: &mut Svm, user: &Keypair, sw: &Swap, nonce: u64) {
    let (i, _) = route_ix(s, &kp(user), sw, u64::MAX, u64::MAX / 2, 0, nonce);
    assert!(s.send_err(&[i], &[user]).contains("SlippageExceeded"));
    let (i, _) = route_ix(s, &kp(user), sw, 1, 1, 0, nonce);
    assert!(s.send_err(&[i], &[user]).contains("MaxInExceeded"));
}

const NATIVE_MINT: Pk = pump_adapter::NATIVE_MINT;

/// A token account with `amount`, written directly (the cloned mints' authorities are not ours).
/// Wrapped-SOL accounts are written as native accounts (lamports = rent reserve + amount).
fn forge_token_account(s: &mut Svm, mint: &Pk, owner: &Pk, amount: u64) -> Pk {
    let k = Pk::new_unique();
    let rent = s.svm.minimum_balance_for_rent_exemption(165);
    let mut d = vec![0u8; 165];
    d[..32].copy_from_slice(mint.as_ref());
    d[32..64].copy_from_slice(owner.as_ref());
    d[64..72].copy_from_slice(&amount.to_le_bytes());
    d[108] = 1; // initialized
    let mut lamports = rent;
    if *mint == NATIVE_MINT {
        d[109] = 1; // is_native: Some(rent reserve)
        d[113..121].copy_from_slice(&rent.to_le_bytes());
        lamports += amount;
    }
    s.svm.set_account(addr(&k), Account { lamports, data: d, owner: addr(&SPL), executable: false, rent_epoch: 0 }).unwrap();
    k
}

const MEMO: Pk = anchor_lang::solana_program::pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");

/// Clones every account saved for a mainnet concentrated-liquidity venue.
fn clone_dir(s: &mut Svm, dir: &PathBuf) {
    for f in std::fs::read_dir(dir).unwrap() {
        let name = f.unwrap().file_name().into_string().unwrap();
        if let Some(key) = name.strip_suffix(".json").filter(|k| *k != "manifest") {
            clone_account(s, dir, key);
        }
    }
}

fn arrays(m: &serde_json::Value, field: &str) -> Vec<(i64, Pk)> {
    m[field].as_array().unwrap().iter().map(|a| (a[0].as_i64().unwrap(), Pk::from_str(a[1].as_str().unwrap()).unwrap())).collect()
}

/// Funded user accounts for (mint_a, mint_b), 1% of each vault.
fn concentrated_user(s: &mut Svm, mints: [Pk; 2], vaults: [Pk; 2]) -> (Keypair, [Pk; 2]) {
    let user = s.funded(10);
    let u = kp(&user);
    let ua = [0, 1].map(|i| {
        let v = s.token_balance(&vaults[i]);
        forge_token_account(s, &mints[i], &u, v / 100)
    });
    (user, ua)
}

// ---------------------------------------------------------------- Raydium CLMM (mainnet)

#[test]
fn raydium_clmm_mainnet_binary_routes_both_ways() {
    let (mut s, _) = router(venues::RAYDIUM_CLMM);
    let (dir, m) = manifest(Net::Mainnet, "raydium-clmm");
    let program = key(&m, "program");
    load_program(&mut s, &program, "raydium_clmm_mainnet.so");
    load_program(&mut s, &MEMO, "spl_memo.so");
    clone_dir(&mut s, &dir);
    let (pool, config, obs, ext) = (key(&m, "pool"), key(&m, "amm_config"), key(&m, "observation"), key(&m, "bitmap_extension"));
    let mint = [key(&m, "mint_0"), key(&m, "mint_1")];
    let vault = [key(&m, "vault_0"), key(&m, "vault_1")];
    let ticks = arrays(&m, "tick_arrays");
    let tick = m["tick_current"].as_i64().unwrap();
    let size = 60 * m["tick_spacing"].as_i64().unwrap();
    let current = tick.div_euclid(size) * size;
    let (user, ua) = concentrated_user(&mut s, mint, vault);
    let u = kp(&user);

    let swap = |from: usize, amount: u64| -> Swap {
        let to = 1 - from;
        let mut metas = vec![
            Meta::new_readonly(u, true),
            Meta::new_readonly(config, false),
            Meta::new(pool, false),
            Meta::new(ua[from], false),
            Meta::new(ua[to], false),
            Meta::new(vault[from], false),
            Meta::new(vault[to], false),
            Meta::new(obs, false),
            Meta::new_readonly(SPL, false),
            Meta::new_readonly(TOKEN_2022, false),
            Meta::new_readonly(MEMO, false),
            Meta::new_readonly(mint[from], false),
            Meta::new_readonly(mint[to], false),
            Meta::new_readonly(ext, false),
        ];
        // Tick arrays from the current one in the price direction (0 → 1 moves the tick down).
        let mut path: Vec<&(i64, Pk)> = ticks.iter().filter(|(st, _)| if from == 0 { *st <= current } else { *st >= current }).collect();
        path.sort_by_key(|(st, _)| if from == 0 { -st } else { *st });
        metas.extend(path.iter().take(3).map(|(_, k)| Meta::new(*k, false)));
        let mut data = disc("swap_v2").to_vec();
        data.extend_from_slice(&amount.to_le_bytes());
        data.extend_from_slice(&1u64.to_le_bytes());
        data.extend_from_slice(&0u128.to_le_bytes());
        data.push(1); // is_base_input
        Swap { program, metas, data, input: ua[from], output: ua[to] }
    };
    let (_, usdc) = route_and_check(&mut s, &user, &swap(0, 1_000_000_000), venues::RAYDIUM_CLMM, 1);
    route_and_check(&mut s, &user, &swap(1, usdc / 2), venues::RAYDIUM_CLMM, 2);
    router_limits_bind(&mut s, &user, &swap(0, 100_000_000), 3);
}

// ---------------------------------------------------------------- Orca Whirlpools (mainnet)

#[test]
fn orca_whirlpool_mainnet_binary_routes_both_ways() {
    let (mut s, _) = router(venues::ORCA_WHIRLPOOL);
    let (dir, m) = manifest(Net::Mainnet, "whirlpool");
    let program = key(&m, "program");
    load_program(&mut s, &program, "orca_whirlpool_mainnet.so");
    load_program(&mut s, &MEMO, "spl_memo.so");
    clone_dir(&mut s, &dir);
    let pool = key(&m, "pool");
    let mint = [key(&m, "mint_a"), key(&m, "mint_b")];
    let vault = [key(&m, "vault_a"), key(&m, "vault_b")];
    let oracle = key(&m, "oracle");
    let tick = m["tick_current"].as_i64().unwrap();
    let size = 88 * m["tick_spacing"].as_i64().unwrap();
    let current = tick.div_euclid(size) * size;
    let pda = |st: i64| Pk::find_program_address(&[b"tick_array", pool.as_ref(), st.to_string().as_bytes()], &program).0;
    let (user, ua) = concentrated_user(&mut s, mint, vault);
    let u = kp(&user);

    let swap = |a_to_b: bool, amount: u64| -> Swap {
        let step = if a_to_b { -size } else { size };
        let mut metas = vec![
            Meta::new_readonly(SPL, false),
            Meta::new_readonly(SPL, false),
            Meta::new_readonly(MEMO, false),
            Meta::new_readonly(u, true),
            Meta::new(pool, false),
            Meta::new_readonly(mint[0], false),
            Meta::new_readonly(mint[1], false),
            Meta::new(ua[0], false),
            Meta::new(vault[0], false),
            Meta::new(ua[1], false),
            Meta::new(vault[1], false),
        ];
        metas.extend((0..3).map(|i| Meta::new(pda(current + i * step), false)));
        metas.push(Meta::new(oracle, false));
        let mut data = disc("swap_v2").to_vec();
        data.extend_from_slice(&amount.to_le_bytes());
        data.extend_from_slice(&1u64.to_le_bytes());
        data.extend_from_slice(&0u128.to_le_bytes());
        data.push(1); // amount_specified_is_input
        data.push(a_to_b as u8);
        data.push(0); // remaining_accounts_info: None
        let (input, output) = if a_to_b { (ua[0], ua[1]) } else { (ua[1], ua[0]) };
        Swap { program, metas, data, input, output }
    };
    let (_, got) = route_and_check(&mut s, &user, &swap(true, 1_000_000_000), venues::ORCA_WHIRLPOOL, 1);
    route_and_check(&mut s, &user, &swap(false, got / 2), venues::ORCA_WHIRLPOOL, 2);
    router_limits_bind(&mut s, &user, &swap(true, 100_000_000), 3);
    // The direction flag decides which account is the input: claiming the other one is refused.
    let mut wrong = swap(true, 100_000_000);
    std::mem::swap(&mut wrong.input, &mut wrong.output);
    let (i, _) = route_ix(&s, &u, &wrong, u64::MAX, 1, 0, 9);
    assert!(s.send_err(&[i], &[&user]).contains("AccountMismatch"));
}

// ---------------------------------------------------------------- Meteora DLMM (mainnet)

#[test]
fn meteora_dlmm_mainnet_binary_routes_both_ways() {
    let (mut s, _) = router(venues::METEORA_DLMM);
    let (dir, m) = manifest(Net::Mainnet, "dlmm");
    let program = key(&m, "program");
    load_program(&mut s, &program, "meteora_dlmm_mainnet.so");
    load_program(&mut s, &MEMO, "spl_memo.so");
    clone_dir(&mut s, &dir);
    let pool = key(&m, "pool");
    let mint = [key(&m, "mint_x"), key(&m, "mint_y")];
    let vault = [key(&m, "reserve_x"), key(&m, "reserve_y")];
    let (oracle, ext, events) = (key(&m, "oracle"), key(&m, "bitmap_extension"), key(&m, "event_authority"));
    let bins = arrays(&m, "bin_arrays");
    let current = m["active_id"].as_i64().unwrap().div_euclid(70);
    let (user, ua) = concentrated_user(&mut s, mint, vault);
    let u = kp(&user);

    let swap = |from: usize, amount: u64| -> Swap {
        let to = 1 - from;
        let mut metas = vec![
            Meta::new(pool, false),
            Meta::new(ext, false),
            Meta::new(vault[0], false),
            Meta::new(vault[1], false),
            Meta::new(ua[from], false),
            Meta::new(ua[to], false),
            Meta::new_readonly(mint[0], false),
            Meta::new_readonly(mint[1], false),
            Meta::new(oracle, false),
            Meta::new_readonly(program, false), // host_fee_in: none
            Meta::new_readonly(u, true),
            Meta::new_readonly(SPL, false),
            Meta::new_readonly(SPL, false),
            Meta::new_readonly(MEMO, false),
            Meta::new_readonly(events, false),
            Meta::new_readonly(program, false),
        ];
        // Selling X moves the active bin down.
        let mut path: Vec<&(i64, Pk)> = bins.iter().filter(|(i, _)| if from == 0 { *i <= current } else { *i >= current }).collect();
        path.sort_by_key(|(i, _)| if from == 0 { -i } else { *i });
        metas.extend(path.iter().take(3).map(|(_, k)| Meta::new(*k, false)));
        let mut data = disc("swap2").to_vec();
        data.extend_from_slice(&amount.to_le_bytes());
        data.extend_from_slice(&1u64.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes()); // remaining_accounts_info: no slices
        Swap { program, metas, data, input: ua[from], output: ua[to] }
    };
    let (_, got) = route_and_check(&mut s, &user, &swap(0, 1_000_000_000), venues::METEORA_DLMM, 1);
    route_and_check(&mut s, &user, &swap(1, got / 2), venues::METEORA_DLMM, 2);
    router_limits_bind(&mut s, &user, &swap(0, 100_000_000), 3);
}

// ---------------------------------------------------------------- Raydium CPMM

struct Cpmm {
    program: Pk,
    config: Pk,
    pool: Pk,
    authority: Pk,
    mint: [Pk; 2],
    vault: [Pk; 2],
    observation: Pk,
}

impl Cpmm {
    fn swap(&self, user: &Pk, from: usize, accounts: [Pk; 2], exact_in: bool, a: u64, b: u64) -> Swap {
        let to = 1 - from;
        let metas = vec![
            Meta::new_readonly(*user, true),
            Meta::new_readonly(self.authority, false),
            Meta::new_readonly(self.config, false),
            Meta::new(self.pool, false),
            Meta::new(accounts[from], false),
            Meta::new(accounts[to], false),
            Meta::new(self.vault[from], false),
            Meta::new(self.vault[to], false),
            Meta::new_readonly(SPL, false),
            Meta::new_readonly(SPL, false),
            Meta::new_readonly(self.mint[from], false),
            Meta::new_readonly(self.mint[to], false),
            Meta::new(self.observation, false),
        ];
        let mut data = disc(if exact_in { "swap_base_input" } else { "swap_base_output" }).to_vec();
        data.extend_from_slice(&a.to_le_bytes());
        data.extend_from_slice(&b.to_le_bytes());
        Swap { program: self.program, metas, data, input: accounts[from], output: accounts[to] }
    }
}

/// Creates a pool through CPMM's own `initialize` on the network's real AmmConfig.
fn cpmm_pool(s: &mut Svm, net: Net) -> (Cpmm, Keypair) {
    let (dir, m) = manifest(net, "raydium-cpmm");
    let program = key(&m, "program");
    load_program(s, &program, &format!("raydium_cpmm_{}.so", net.name()));
    let config = clone_account(s, &dir, m["amm_config"].as_str().unwrap());
    let fee = clone_account(s, &dir, m["create_pool_fee"].as_str().unwrap());

    let creator = s.funded(100);
    let (a, b) = (s.create_mint(&creator, 6), s.create_mint(&creator, 9));
    let mint = if a < b { [a, b] } else { [b, a] };
    let ct = mint.map(|mt| s.create_token_account(&creator, &mt, &kp(&creator)));
    for i in 0..2 {
        s.mint_to(&creator, &mint[i], &ct[i], 10_000_000_000_000);
    }
    let pda = |seeds: &[&[u8]]| Pk::find_program_address(seeds, &program).0;
    let authority = pda(&[b"vault_and_lp_mint_auth_seed"]);
    let pool = pda(&[b"pool", config.as_ref(), mint[0].as_ref(), mint[1].as_ref()]);
    let lp = pda(&[b"pool_lp_mint", pool.as_ref()]);
    let vault = mint.map(|mt| pda(&[b"pool_vault", pool.as_ref(), mt.as_ref()]));
    let observation = pda(&[b"observation", pool.as_ref()]);
    let creator_lp = pump_adapter::pda::associated_token(&kp(&creator), &SPL, &lp);
    let c = kp(&creator);
    let metas = vec![
        Meta::new(c, true),
        Meta::new_readonly(config, false),
        Meta::new_readonly(authority, false),
        Meta::new(pool, false),
        Meta::new_readonly(mint[0], false),
        Meta::new_readonly(mint[1], false),
        Meta::new(lp, false),
        Meta::new(ct[0], false),
        Meta::new(ct[1], false),
        Meta::new(creator_lp, false),
        Meta::new(vault[0], false),
        Meta::new(vault[1], false),
        Meta::new(fee, false),
        Meta::new(observation, false),
        Meta::new_readonly(SPL, false),
        Meta::new_readonly(SPL, false),
        Meta::new_readonly(SPL, false),
        Meta::new_readonly(ATA, false),
        Meta::new_readonly(SYS, false),
        Meta::new_readonly(sysvar::rent::ID, false),
    ];
    let mut data = disc("initialize").to_vec();
    for v in [1_000_000_000_000u64, 2_000_000_000_000, 0] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    s.send_ok(&[convert(program, metas, data)], &[&creator]);
    // CPMM opens a pool for swaps no earlier than one second after creation.
    let mut clock: solana_clock::Clock = s.svm.get_sysvar();
    clock.unix_timestamp += 2;
    s.svm.set_sysvar(&clock);
    (Cpmm { program, config, pool, authority, mint, vault, observation }, creator)
}

fn verify_cpmm(net: Net, venue: u8) {
    let (mut s, _) = router(venue);
    let (cp, creator) = cpmm_pool(&mut s, net);
    let user = s.funded(10);
    let ua = cp.mint.map(|m| s.create_token_account(&user, &m, &kp(&user)));
    s.mint_to(&creator, &cp.mint[0], &ua[0], 50_000_000);
    let u = kp(&user);

    // swap_base_input 0 -> 1, then 1 -> 0 with what came back.
    let (_, got) = route_and_check(&mut s, &user, &cp.swap(&u, 0, ua, true, 10_000_000, 1), venue, 1);
    route_and_check(&mut s, &user, &cp.swap(&u, 1, ua, true, got / 2, 1), venue, 2);
    // swap_base_output: exact amount out, venue max-in generous.
    let (_, out) = route_and_check(&mut s, &user, &cp.swap(&u, 0, ua, false, 10_000_000, 1_000_000), venue, 3);
    assert_eq!(out, 1_000_000);

    router_limits_bind(&mut s, &user, &cp.swap(&u, 0, ua, true, 1_000_000, 1), 4);

    // The venue stays directly callable: router hooks are opt-in.
    let direct = cp.swap(&u, 0, ua, true, 1_000_000, 1);
    s.send_ok(&[convert(direct.program, direct.metas, direct.data)], &[&user]);
}

#[test]
fn raydium_cpmm_devnet_binary_routes_both_ways() {
    verify_cpmm(Net::Devnet, venues::RAYDIUM_CPMM_DEVNET);
}

#[test]
fn raydium_cpmm_mainnet_binary_routes_both_ways() {
    verify_cpmm(Net::Mainnet, venues::RAYDIUM_CPMM);
}

// ---------------------------------------------------------------- Meteora DAMM v2

struct Damm {
    program: Pk,
    pool: Pk,
    mint: [Pk; 2],
    vault: [Pk; 2],
}

impl Damm {
    fn swap(&self, user: &Pk, from: usize, accounts: [Pk; 2], mode: u8, a: u64, b: u64) -> Swap {
        let pda = |seeds: &[&[u8]]| Pk::find_program_address(seeds, &self.program).0;
        let metas = vec![
            Meta::new_readonly(pda(&[b"pool_authority"]), false),
            Meta::new(self.pool, false),
            Meta::new(accounts[from], false),
            Meta::new(accounts[1 - from], false),
            Meta::new(self.vault[0], false),
            Meta::new(self.vault[1], false),
            Meta::new_readonly(self.mint[0], false),
            Meta::new_readonly(self.mint[1], false),
            Meta::new_readonly(*user, true),
            Meta::new_readonly(SPL, false),
            Meta::new_readonly(SPL, false),
            Meta::new_readonly(self.program, false),
            Meta::new_readonly(pda(&[b"__event_authority"]), false),
            Meta::new_readonly(self.program, false),
            Meta::new_readonly(sysvar::instructions::ID, false),
        ];
        let mut data = disc("swap2").to_vec();
        data.extend_from_slice(&a.to_le_bytes());
        data.extend_from_slice(&b.to_le_bytes());
        data.push(mode);
        Swap { program: self.program, metas, data, input: accounts[from], output: accounts[1 - from] }
    }
}

fn verify_damm(net: Net) {
    let (mut s, _) = router(venues::METEORA_DAMM_V2);
    let (dir, m) = manifest(net, "damm-v2");
    let program = key(&m, "program");
    load_program(&mut s, &program, &format!("meteora_damm_v2_{}.so", net.name()));
    for f in ["pool", "token_a_mint", "token_b_mint", "token_a_vault", "token_b_vault"] {
        clone_account(&mut s, &dir, m[f].as_str().unwrap());
    }
    let d = Damm {
        program,
        pool: key(&m, "pool"),
        mint: [key(&m, "token_a_mint"), key(&m, "token_b_mint")],
        vault: [key(&m, "token_a_vault"), key(&m, "token_b_vault")],
    };
    let bal: Vec<u64> = m["vault_balances"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    let user = s.funded(10);
    let u = kp(&user);
    let ua = [forge_token_account(&mut s, &d.mint[0], &u, bal[0] / 100), forge_token_account(&mut s, &d.mint[1], &u, bal[1] / 100)];

    const EXACT_IN: u8 = 0;
    const EXACT_OUT: u8 = 2;
    // A -> B and B -> A, exact in.
    route_and_check(&mut s, &user, &d.swap(&u, 0, ua, EXACT_IN, bal[0] / 1_000, 1), venues::METEORA_DAMM_V2, 1);
    route_and_check(&mut s, &user, &d.swap(&u, 1, ua, EXACT_IN, bal[1] / 1_000, 1), venues::METEORA_DAMM_V2, 2);
    // Exact out: amount_0 = amount out, amount_1 = max in.
    let want = bal[1] / 10_000;
    let (_, out) = route_and_check(&mut s, &user, &d.swap(&u, 0, ua, EXACT_OUT, want, bal[0] / 100), venues::METEORA_DAMM_V2, 3);
    assert_eq!(out, want);
    // The original `swap(amount_in, minimum_amount_out)` shares swap2's account order.
    let mut legacy = d.swap(&u, 1, ua, EXACT_IN, bal[1] / 1_000, 1);
    legacy.data = disc("swap").to_vec();
    legacy.data.extend_from_slice(&(bal[1] / 1_000).to_le_bytes());
    legacy.data.extend_from_slice(&1u64.to_le_bytes());
    route_and_check(&mut s, &user, &legacy, venues::METEORA_DAMM_V2, 5);

    router_limits_bind(&mut s, &user, &d.swap(&u, 0, ua, EXACT_IN, bal[0] / 10_000, 1), 4);
}

#[test]
fn meteora_damm_v2_devnet_binary_routes_both_ways() {
    verify_damm(Net::Devnet);
}

#[test]
fn meteora_damm_v2_mainnet_binary_routes_both_ways() {
    verify_damm(Net::Mainnet);
}
