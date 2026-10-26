//! Shared LiteSVM harness: real Pump binaries dumped from devnet, accounts of an existing devnet
//! PumpSwap pool cloned into the VM, and the locally built pump-hooks program.

#![allow(dead_code)]

use std::{collections::HashMap, path::PathBuf, str::FromStr};

use anchor_lang::{AnchorDeserialize, Discriminator, InstructionData, ToAccountMetas};
use base64::Engine;
use litesvm::{
    types::{FailedTransactionMetadata, TransactionMetadata},
    LiteSVM,
};
use solana_account::Account;
use solana_address::Address;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction::Transaction;

pub type Pk = anchor_lang::prelude::Pubkey;

pub const POOL: &str = "9Nhddo1PcCBms54wcLYbxoRcAJbpmSvHWMcbdMf9tdmZ";
pub const SOL: u64 = 1_000_000_000;

pub fn addr(p: &Pk) -> Address {
    Address::new_from_array(p.to_bytes())
}
pub fn pk(a: &Address) -> Pk {
    Pk::new_from_array(a.to_bytes())
}
pub fn kp_pk(k: &Keypair) -> Pk {
    pk(&k.pubkey())
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn convert_ix(program: Pk, metas: Vec<anchor_lang::solana_program::instruction::AccountMeta>, data: Vec<u8>) -> Instruction {
    Instruction {
        program_id: addr(&program),
        accounts: metas
            .into_iter()
            .map(|m| AccountMeta { pubkey: addr(&m.pubkey), is_signer: m.is_signer, is_writable: m.is_writable })
            .collect(),
        data,
    }
}

pub fn hooks_ix(accounts: impl ToAccountMetas, extra: Vec<anchor_lang::solana_program::instruction::AccountMeta>, data: impl InstructionData) -> Instruction {
    let mut metas = accounts.to_account_metas(None);
    metas.extend(extra);
    convert_ix(pump_hooks::ID, metas, data.data())
}

pub struct Fixture {
    pub pool: Pk,
    pub mint: Pk,
    pub base_token_program: Pk,
    pub buyback_recipient: Pk,
    pub accounts: HashMap<String, Pk>,
}

pub struct Env {
    pub svm: LiteSVM,
    pub fx: Fixture,
    pub admin: Keypair,
}

fn load_fixture_account(path: &PathBuf) -> (Pk, Account) {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let key = Pk::from_str(v["pubkey"].as_str().unwrap()).unwrap();
    let a = &v["account"];
    let data = base64::engine::general_purpose::STANDARD.decode(a["data"][0].as_str().unwrap()).unwrap();
    (
        key,
        Account {
            lamports: a["lamports"].as_u64().unwrap(),
            data,
            owner: Address::from_str(a["owner"].as_str().unwrap()).unwrap(),
            executable: a["executable"].as_bool().unwrap(),
            rent_epoch: 0,
        },
    )
}

/// Pump binaries are dumped per cluster (`pump.so` from devnet, `pump_mainnet.so` from mainnet);
/// a fixture runs against the binaries of the cluster it was snapshotted from.
fn program_path(name: &str, mainnet: bool) -> PathBuf {
    let file = if mainnet { format!("{name}_mainnet") } else { name.to_string() };
    let path = root().join(format!("tests/fixtures/programs/{file}.so"));
    if path.exists() {
        return path;
    }
    panic!("missing {} — run scripts/dump_programs.sh", path.display());
}

/// Legacy 82-byte curve with deep remaining reserves.
pub const DEEP_CURVE: &str = "AwHTvGo8UcR4CEyqmTWs1xEbeajBNwC5ELFdMdPyB33w";
/// 115-byte curve with ~37k tokens left: a 0.2 SOL buy completes it.
pub const NEAR_COMPLETE_CURVE: &str = "6MyxotriXqB78HkY5Yc4Lc1jVwkKw82NJmYDPktRpjak";
/// Live mainnet coin still on its bonding curve.
pub const MAINNET_CURVE: &str = "Ykfw76rQnNKoBVtTTgVXgCzYHVZopGppb1Loj67u36H";
/// Live mainnet PumpSwap pool of a graduated coin.
pub const MAINNET_POOL: &str = "6DNxwecbtdNNstpB5tdnxZsfGe3HmsNgEUZhdue4b1PH";

impl Env {
    /// Existing migrated coin: PumpSwap pool fixture.
    pub fn new() -> Self {
        Self::load(POOL)
    }

    /// Existing migrated coin from any pool fixture.
    pub fn pool(pool: &str) -> Self {
        Self::load(pool)
    }

    /// Existing un-migrated coin: bonding-curve fixture.
    pub fn curve(curve: &str) -> Self {
        Self::load(&format!("curve-{curve}"))
    }

    fn load(fixture: &str) -> Self {
        let mut svm = LiteSVM::new().with_precompiles().with_log_bytes_limit(None);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let mut clock: solana_clock::Clock = svm.get_sysvar();
        clock.unix_timestamp = now;
        clock.slot = 400_000_000;
        svm.set_sysvar(&clock);

        let dir = root().join(format!("tests/fixtures/{fixture}"));
        let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
        let mainnet = manifest["source_rpc"].as_str().unwrap_or("").contains("mainnet");

        svm.add_program_from_file(addr(&pump_adapter::PUMP_AMM_PROGRAM_ID), program_path("pump_amm", mainnet)).unwrap();
        svm.add_program_from_file(addr(&pump_adapter::PUMP_PROGRAM_ID), program_path("pump", mainnet)).unwrap();
        svm.add_program_from_file(addr(&pump_adapter::PUMP_FEES_PROGRAM_ID), program_path("pump_fees", mainnet)).unwrap();
        let so = root().join("target/deploy/pump_hooks.so");
        assert!(so.exists(), "build the program first: cargo build-sbf --manifest-path programs/pump-hooks/Cargo.toml");
        svm.add_program_from_file(addr(&pump_hooks::ID), so).unwrap();

        let mut accounts = HashMap::new();
        for (label, key) in manifest["accounts"].as_object().unwrap() {
            let (k, acct) = load_fixture_account(&dir.join(format!("{}.json", key.as_str().unwrap())));
            if label == "quote_mint" && svm.get_account(&addr(&k)).is_some() {
                // keep the VM's native mint
            } else {
                svm.set_account(addr(&k), acct).unwrap();
            }
            accounts.insert(label.clone(), k);
        }
        let mint = Pk::from_str(manifest["mint"].as_str().unwrap()).unwrap();
        let base_token_program = pk(&svm.get_account(&addr(&mint)).unwrap().owner);
        let fx = Fixture {
            pool: manifest["pool"].as_str().map(|p| Pk::from_str(p).unwrap()).unwrap_or_default(),
            mint,
            base_token_program,
            buyback_recipient: Pk::from_str(manifest["buyback_fee_recipient"].as_str().unwrap()).unwrap(),
            accounts,
        };
        let admin = Keypair::new();
        svm.airdrop(&admin.pubkey(), 100 * SOL).unwrap();
        Self { svm, fx, admin }
    }

    pub fn now(&self) -> i64 {
        let c: solana_clock::Clock = self.svm.get_sysvar();
        c.unix_timestamp
    }

    pub fn advance(&mut self, secs: i64, slots: u64) {
        let mut c: solana_clock::Clock = self.svm.get_sysvar();
        c.unix_timestamp += secs;
        c.slot += slots;
        self.svm.set_sysvar(&c);
        self.svm.expire_blockhash();
    }

    pub fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> Result<TransactionMetadata, FailedTransactionMetadata> {
        let mut all = vec![compute_limit(1_000_000)];
        all.extend_from_slice(ixs);
        let tx = Transaction::new_signed_with_payer(&all, Some(&signers[0].pubkey()), signers, self.svm.latest_blockhash());
        let r = self.svm.send_transaction(tx);
        self.svm.expire_blockhash();
        r
    }

    pub fn send_ok(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> TransactionMetadata {
        match self.send(ixs, signers) {
            Ok(m) => m,
            Err(e) => panic!("tx failed: {:?}\n{}", e.err, e.meta.logs.join("\n")),
        }
    }

    pub fn token_balance(&self, account: &Pk) -> u64 {
        self.svm
            .get_account(&addr(account))
            .map(|a| u64::from_le_bytes(a.data[64..72].try_into().unwrap()))
            .unwrap_or(0)
    }

    pub fn new_wallet(&mut self, sol: u64, wsol: u64) -> Keypair {
        let k = Keypair::new();
        self.svm.airdrop(&k.pubkey(), sol + wsol).unwrap();
        let owner = kp_pk(&k);
        let mut ixs = vec![
            create_ata_idempotent(&owner, &owner, &pump_adapter::NATIVE_MINT, &pump_adapter::TOKEN_PROGRAM_ID),
            create_ata_idempotent(&owner, &owner, &self.fx.mint, &self.fx.base_token_program),
        ];
        if wsol > 0 {
            let ata = pump_adapter::pda::associated_token(&owner, &pump_adapter::TOKEN_PROGRAM_ID, &pump_adapter::NATIVE_MINT);
            ixs.push(system_transfer(&owner, &ata, wsol));
            ixs.push(sync_native(&ata));
        }
        self.send_ok(&ixs, &[&k]);
        k
    }

    pub fn user_atas(&self, user: &Pk) -> (Pk, Pk) {
        (
            pump_adapter::pda::associated_token(user, &self.fx.base_token_program, &self.fx.mint),
            pump_adapter::pda::associated_token(user, &pump_adapter::TOKEN_PROGRAM_ID, &pump_adapter::NATIVE_MINT),
        )
    }

    pub fn amm_keys(&self) -> pump_adapter::ix::AmmKeys {
        pump_adapter::ix::AmmKeys {
            pool: self.fx.pool,
            base_mint: self.fx.mint,
            quote_mint: pump_adapter::NATIVE_MINT,
            pool_base_token_account: self.fx.accounts["pool_base_token_account"],
            pool_quote_token_account: self.fx.accounts["pool_quote_token_account"],
            base_token_program: self.fx.base_token_program,
            quote_token_program: pump_adapter::TOKEN_PROGRAM_ID,
        }
    }

    // ------------------------------------------------------------------ pump-hooks helpers

    pub fn global(&self) -> Pk {
        Pk::find_program_address(&[b"global"], &pump_hooks::ID).0
    }
    pub fn definition(&self, module: u16) -> Pk {
        Pk::find_program_address(&[b"hook", &module.to_le_bytes()], &pump_hooks::ID).0
    }
    pub fn market(&self, namespace: &Pk) -> Pk {
        Pk::find_program_address(&[b"market", self.fx.mint.as_ref(), namespace.as_ref()], &pump_hooks::ID).0
    }
    pub fn user_state(&self, market: &Pk, user: &Pk) -> Pk {
        Pk::find_program_address(&[b"user", market.as_ref(), user.as_ref()], &pump_hooks::ID).0
    }
    pub fn policy(&self, market: &Pk, index: u8) -> Pk {
        Pk::find_program_address(&[b"policy", market.as_ref(), &[index]], &pump_hooks::ID).0
    }

    /// initialize + register every available first-party module.
    pub fn bootstrap_protocol(&mut self) {
        let admin = kp_pk(&self.admin);
        let ix = hooks_ix(
            pump_hooks::accounts::Initialize { authority: admin, global: self.global(), system_program: pump_adapter::SYSTEM_PROGRAM_ID },
            vec![],
            pump_hooks::instruction::Initialize {},
        );
        let a = self.admin.insecure_clone();
        self.send_ok(&[ix], &[&a]);
        for m in [1u16, 2, 3, 4, 5, 6, 8, 9, 10, 11, 12] {
            let ix = hooks_ix(
                pump_hooks::accounts::RegisterHook {
                    authority: admin,
                    global: self.global(),
                    definition: self.definition(m),
                    system_program: pump_adapter::SYSTEM_PROGRAM_ID,
                },
                vec![],
                pump_hooks::instruction::RegisterHook { module_id: m, source_hash: [m as u8; 32] },
            );
            self.send_ok(&[ix], &[&a]);
        }
    }

    pub fn create_market(&mut self, authority: &Keypair) -> Pk {
        let auth = kp_pk(authority);
        let market = self.market(&auth);
        let ix = hooks_ix(
            pump_hooks::accounts::CreateMarket {
                authority: auth,
                global: self.global(),
                mint: self.fx.mint,
                bonding_curve: self.fx.accounts["bonding_curve"],
                quote_mint: pump_adapter::NATIVE_MINT,
                pool: self.fx.pool,
                market,
                system_program: pump_adapter::SYSTEM_PROGRAM_ID,
            },
            vec![],
            pump_hooks::instruction::CreateMarket {},
        );
        self.send_ok(&[ix], &[authority]);
        market
    }

    pub fn install_ix(&self, authority: &Pk, market: &Pk, module: u16, priority: u16, config: [u8; 48]) -> Instruction {
        let caps = hook_types::ModuleId::from_u16(module).unwrap().info().capabilities;
        hooks_ix(
            pump_hooks::accounts::InstallHook { authority: *authority, global: self.global(), definition: self.definition(module), market: *market },
            vec![],
            pump_hooks::instruction::InstallHook { module_id: module, priority, callbacks: 0, capabilities: caps, config },
        )
    }

    pub fn install(&mut self, authority: &Keypair, market: &Pk, module: u16, config: [u8; 48]) {
        let prio = hook_types::ModuleId::from_u16(module).unwrap().info().default_priority;
        let ix = self.install_ix(&kp_pk(authority), market, module, prio, config);
        self.send_ok(&[ix], &[authority]);
    }

    pub fn route_amm_ix(&self, market: &Pk, user: &Pk, mode: u8, amount: u64, limit: u64, policies: &[Pk]) -> Instruction {
        let mut extra = pump_adapter::ix::amm_v2_metas(&self.amm_keys(), user, &self.fx.buyback_recipient);
        for p in policies {
            extra.push(anchor_lang::solana_program::instruction::AccountMeta::new(*p, false));
        }
        hooks_ix(
            pump_hooks::accounts::Route {
                user: *user,
                global: self.global(),
                market: *market,
                user_state: self.user_state(market, user),
                system_program: pump_adapter::SYSTEM_PROGRAM_ID,
            },
            extra,
            pump_hooks::instruction::RouteAmm { mode, amount, limit },
        )
    }

    pub fn curve_keys(&self) -> pump_adapter::ix::CurveKeys {
        pump_adapter::ix::CurveKeys {
            base_mint: self.fx.mint,
            quote_mint: pump_adapter::NATIVE_MINT,
            base_token_program: self.fx.base_token_program,
            quote_token_program: pump_adapter::TOKEN_PROGRAM_ID,
        }
    }

    pub fn route_curve_ix(&self, market: &Pk, user: &Pk, mode: u8, amount: u64, limit: u64) -> Instruction {
        let extra = pump_adapter::ix::curve_v3_metas(&self.curve_keys(), user, &self.fx.buyback_recipient);
        hooks_ix(
            pump_hooks::accounts::Route {
                user: *user,
                global: self.global(),
                market: *market,
                user_state: self.user_state(market, user),
                system_program: pump_adapter::SYSTEM_PROGRAM_ID,
            },
            extra,
            pump_hooks::instruction::RouteCurve { mode, amount, limit },
        )
    }

    pub fn lamports(&self, k: &Pk) -> u64 {
        self.svm.get_account(&addr(k)).map(|a| a.lamports).unwrap_or(0)
    }

    pub fn direct_amm_ix(&self, user: &Pk, mode: pump_adapter::ix::Mode, amount: u64, limit: u64) -> Instruction {
        let i = pump_adapter::ix::amm_v2_instruction(&self.amm_keys(), user, &self.fx.buyback_recipient, mode, amount, limit);
        convert_ix(i.program_id, i.accounts, i.data)
    }

    pub fn read_market(&self, market: &Pk) -> pump_hooks::state::MarketConfig {
        let a = self.svm.get_account(&addr(market)).unwrap();
        *bytemuck::from_bytes(&a.data[8..8 + pump_hooks::state::MarketConfig::SIZE])
    }
    pub fn read_user(&self, market: &Pk, user: &Pk) -> hook_types::UserState {
        let a = self.svm.get_account(&addr(&self.user_state(market, user))).unwrap();
        let us: pump_hooks::state::UserMarketState = *bytemuck::from_bytes(&a.data[8..8 + pump_hooks::state::UserMarketState::SIZE]);
        us.read()
    }
}

pub fn compute_limit(units: u32) -> Instruction {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    Instruction {
        program_id: Address::from_str("ComputeBudget111111111111111111111111111111").unwrap(),
        accounts: vec![],
        data,
    }
}

pub fn create_ata_idempotent(payer: &Pk, owner: &Pk, mint: &Pk, token_program: &Pk) -> Instruction {
    let ata = pump_adapter::pda::associated_token(owner, token_program, mint);
    Instruction {
        program_id: addr(&pump_adapter::ATA_PROGRAM_ID),
        accounts: vec![
            AccountMeta::new(addr(payer), true),
            AccountMeta::new(addr(&ata), false),
            AccountMeta::new_readonly(addr(owner), false),
            AccountMeta::new_readonly(addr(mint), false),
            AccountMeta::new_readonly(addr(&pump_adapter::SYSTEM_PROGRAM_ID), false),
            AccountMeta::new_readonly(addr(token_program), false),
        ],
        data: vec![1],
    }
}

pub fn system_transfer(from: &Pk, to: &Pk, lamports: u64) -> Instruction {
    let mut data = 2u32.to_le_bytes().to_vec();
    data.extend_from_slice(&lamports.to_le_bytes());
    Instruction {
        program_id: addr(&pump_adapter::SYSTEM_PROGRAM_ID),
        accounts: vec![AccountMeta::new(addr(from), true), AccountMeta::new(addr(to), false)],
        data,
    }
}

pub fn sync_native(account: &Pk) -> Instruction {
    Instruction { program_id: addr(&pump_adapter::TOKEN_PROGRAM_ID), accounts: vec![AccountMeta::new(addr(account), false)], data: vec![17] }
}

/// Ed25519 precompile instruction verifying one signature over `msg`, offsets self-contained.
pub fn ed25519_ix(signer: &Keypair, msg: &[u8]) -> Instruction {
    let sig = signer.sign_message(msg);
    let pk = signer.pubkey().to_bytes();
    let (pk_off, sig_off, msg_off) = (16u16, 48u16, 112u16);
    let mut d = vec![1u8, 0];
    for v in [sig_off, u16::MAX, pk_off, u16::MAX, msg_off, msg.len() as u16, u16::MAX] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&pk);
    d.extend_from_slice(sig.as_ref());
    d.extend_from_slice(msg);
    Instruction { program_id: Address::from_str("Ed25519SigVerify111111111111111111111111111").unwrap(), accounts: vec![], data: d }
}

/// Decode every `T` event emitted (anchor `emit!` → "Program data:" log lines).
pub fn events<T: AnchorDeserialize + Discriminator>(logs: &[String]) -> Vec<T> {
    logs.iter()
        .filter_map(|l| l.strip_prefix("Program data: "))
        .filter_map(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
        .filter(|d| d.len() >= 8 && d[..8] == *T::DISCRIMINATOR)
        .filter_map(|d| T::deserialize(&mut &d[8..]).ok())
        .collect()
}

pub fn custom_error_name(e: &FailedTransactionMetadata) -> String {
    e.meta.logs.iter().find(|l| l.contains("Error Code:")).cloned().unwrap_or_else(|| format!("{:?}", e.err))
}
