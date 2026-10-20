//! Real PumpSwap venue in LiteSVM: devnet-dumped `pump_amm` / `pump` / `pump_fees` binaries and
//! the cloned accounts of an existing devnet pool (`tests/fixtures/<pool>`).

use std::{collections::HashMap, path::PathBuf, str::FromStr};

use anchor_lang::solana_program::instruction::AccountMeta as Meta2;
use base64::Engine;
use solana_account::Account;
use solana_address::Address;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_signer::Signer;

use super::*;

pub const POOL: &str = "9Nhddo1PcCBms54wcLYbxoRcAJbpmSvHWMcbdMf9tdmZ";
pub use pump_adapter::ix::Mode;

pub struct PumpSwap {
    pub pool: Pk,
    pub mint: Pk,
    pub base_token_program: Pk,
    pub buyback: Pk,
    pub accounts: HashMap<String, Pk>,
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

fn load_account(path: &PathBuf) -> (Pk, Account) {
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

impl PumpSwap {
    pub fn load(s: &mut Svm) -> Self {
        for (id, name) in [
            (pump_adapter::PUMP_AMM_PROGRAM_ID, "pump_amm"),
            (pump_adapter::PUMP_PROGRAM_ID, "pump"),
            (pump_adapter::PUMP_FEES_PROGRAM_ID, "pump_fees"),
        ] {
            let path = fixtures().join(format!("programs/{name}.so"));
            s.svm.add_program_from_file(addr(&id), &path).unwrap_or_else(|_| panic!("missing {} (scripts/dump_programs.sh)", path.display()));
        }
        let dir = fixtures().join(POOL);
        let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
        let mut accounts = HashMap::new();
        for (label, key) in manifest["accounts"].as_object().unwrap() {
            let (k, acct) = load_account(&dir.join(format!("{}.json", key.as_str().unwrap())));
            if !(label == "quote_mint" && s.svm.get_account(&addr(&k)).is_some()) {
                s.svm.set_account(addr(&k), acct).unwrap();
            }
            accounts.insert(label.clone(), k);
        }
        let mint = Pk::from_str(manifest["mint"].as_str().unwrap()).unwrap();
        let base_token_program = pk(&s.svm.get_account(&addr(&mint)).unwrap().owner);
        PumpSwap {
            pool: Pk::from_str(manifest["pool"].as_str().unwrap()).unwrap(),
            mint,
            base_token_program,
            buyback: Pk::from_str(manifest["buyback_fee_recipient"].as_str().unwrap()).unwrap(),
            accounts,
        }
    }

    pub fn keys(&self) -> pump_adapter::ix::AmmKeys {
        pump_adapter::ix::AmmKeys {
            pool: self.pool,
            base_mint: self.mint,
            quote_mint: pump_adapter::NATIVE_MINT,
            pool_base_token_account: self.accounts["pool_base_token_account"],
            pool_quote_token_account: self.accounts["pool_quote_token_account"],
            base_token_program: self.base_token_program,
            quote_token_program: pump_adapter::TOKEN_PROGRAM_ID,
        }
    }

    /// `(base ATA, wSOL ATA)`.
    pub fn atas(&self, user: &Pk) -> (Pk, Pk) {
        (
            pump_adapter::pda::associated_token(user, &self.base_token_program, &self.mint),
            pump_adapter::pda::associated_token(user, &pump_adapter::TOKEN_PROGRAM_ID, &pump_adapter::NATIVE_MINT),
        )
    }

    pub fn metas(&self, user: &Pk) -> Vec<Meta2> {
        pump_adapter::ix::amm_v2_metas(&self.keys(), user, &self.buyback)
    }

    pub fn data(mode: Mode, amount: u64, limit: u64) -> Vec<u8> {
        pump_adapter::ix::amm_v2_data(mode, amount, limit)
    }

    pub fn direct(&self, user: &Pk, mode: Mode, amount: u64, limit: u64) -> Instruction {
        let i = pump_adapter::ix::amm_v2_instruction(&self.keys(), user, &self.buyback, mode, amount, limit);
        convert(i.program_id, i.accounts, i.data)
    }

    /// Wallet with both ATAs and `wsol` lamports wrapped.
    pub fn wallet(&self, s: &mut Svm, sol: u64, wsol: u64) -> Keypair {
        let k = Keypair::new();
        s.svm.airdrop(&k.pubkey(), sol + wsol).unwrap();
        let owner = kp(&k);
        let (_, quote) = self.atas(&owner);
        let mut ixs = vec![
            create_ata(&owner, &pump_adapter::NATIVE_MINT, &pump_adapter::TOKEN_PROGRAM_ID),
            create_ata(&owner, &self.mint, &self.base_token_program),
        ];
        if wsol > 0 {
            let mut d = 2u32.to_le_bytes().to_vec();
            d.extend_from_slice(&wsol.to_le_bytes());
            ixs.push(Instruction {
                program_id: addr(&pump_adapter::SYSTEM_PROGRAM_ID),
                accounts: vec![AccountMeta::new(addr(&owner), true), AccountMeta::new(addr(&quote), false)],
                data: d,
            });
            ixs.push(Instruction { program_id: addr(&pump_adapter::TOKEN_PROGRAM_ID), accounts: vec![AccountMeta::new(addr(&quote), false)], data: vec![17] });
        }
        s.send_ok(&ixs, &[&k]);
        k
    }
}

fn create_ata(owner: &Pk, mint: &Pk, token_program: &Pk) -> Instruction {
    let ata = pump_adapter::pda::associated_token(owner, token_program, mint);
    Instruction {
        program_id: addr(&pump_adapter::ATA_PROGRAM_ID),
        accounts: vec![
            AccountMeta::new(addr(owner), true),
            AccountMeta::new(addr(&ata), false),
            AccountMeta::new_readonly(addr(owner), false),
            AccountMeta::new_readonly(addr(mint), false),
            AccountMeta::new_readonly(addr(&pump_adapter::SYSTEM_PROGRAM_ID), false),
            AccountMeta::new_readonly(addr(token_program), false),
        ],
        data: vec![1],
    }
}
