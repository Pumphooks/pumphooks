//! Shared LiteSVM harness for the engine tests: locally built programs, upgradeable-loader
//! metadata manipulation, transaction helpers.

#![allow(dead_code, deprecated)]

use std::{path::PathBuf, str::FromStr};

use anchor_lang::{
    solana_program::instruction::AccountMeta as Meta2, AccountDeserialize, InstructionData,
    ToAccountMetas,
};
use litesvm::{
    types::{FailedTransactionMetadata, TransactionMetadata},
    LiteSVM,
};
use solana_address::Address;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction::Transaction;

pub type Pk = anchor_lang::prelude::Pubkey;
pub const SOL: u64 = 1_000_000_000;

pub fn addr(p: &Pk) -> Address {
    Address::new_from_array(p.to_bytes())
}
pub fn pk(a: &Address) -> Pk {
    Pk::new_from_array(a.to_bytes())
}
pub fn kp(k: &Keypair) -> Pk {
    pk(&k.pubkey())
}

pub fn deploy_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/deploy")
}

pub fn convert(program: Pk, metas: Vec<Meta2>, data: Vec<u8>) -> Instruction {
    Instruction {
        program_id: addr(&program),
        accounts: metas
            .into_iter()
            .map(|m| AccountMeta {
                pubkey: addr(&m.pubkey),
                is_signer: m.is_signer,
                is_writable: m.is_writable,
            })
            .collect(),
        data,
    }
}

pub fn ix(
    program: Pk,
    accounts: impl ToAccountMetas,
    extra: Vec<Meta2>,
    data: impl InstructionData,
) -> Instruction {
    let mut metas = accounts.to_account_metas(None);
    metas.extend(extra);
    convert(program, metas, data.data())
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

pub fn programdata(program: &Pk) -> Pk {
    Pk::find_program_address(
        &[program.as_ref()],
        &anchor_lang::solana_program::bpf_loader_upgradeable::ID,
    )
    .0
}

pub struct Svm {
    pub svm: LiteSVM,
}

impl Svm {
    pub fn new() -> Self {
        Svm {
            svm: LiteSVM::new(),
        }
    }

    pub fn add_program(&mut self, id: &Pk, so: &str) {
        self.svm
            .add_program_from_file(addr(id), deploy_dir().join(so))
            .unwrap_or_else(|_| panic!("build {so} first (scripts/build_programs.sh)"));
    }

    pub fn funded(&mut self, sol: u64) -> Keypair {
        let k = Keypair::new();
        self.svm.airdrop(&k.pubkey(), sol * SOL).unwrap();
        k
    }

    pub fn set_upgrade_authority(&mut self, program: &Pk, authority: Option<Pk>) {
        let key = addr(&programdata(program));
        let mut a = self.svm.get_account(&key).unwrap();
        match authority {
            Some(k) => {
                a.data[12] = 1;
                a.data[13..45].copy_from_slice(k.as_ref());
            }
            None => {
                a.data[12] = 0;
                a.data[13..45].fill(0);
            }
        }
        self.svm.set_account(key, a).unwrap();
    }

    pub fn set_deploy_slot(&mut self, program: &Pk, slot: u64) {
        let key = addr(&programdata(program));
        let mut a = self.svm.get_account(&key).unwrap();
        a.data[4..12].copy_from_slice(&slot.to_le_bytes());
        self.svm.set_account(key, a).unwrap();
    }

    pub fn deploy_slot(&self, program: &Pk) -> u64 {
        let a = self.svm.get_account(&addr(&programdata(program))).unwrap();
        u64::from_le_bytes(a.data[4..12].try_into().unwrap())
    }

    pub fn warp_time(&mut self, unix_timestamp: i64) {
        let mut c: solana_clock::Clock = self.svm.get_sysvar();
        c.unix_timestamp = unix_timestamp;
        self.svm.set_sysvar(&c);
    }

    pub fn now(&self) -> i64 {
        self.svm.get_sysvar::<solana_clock::Clock>().unix_timestamp
    }

    pub fn warp_slot(&mut self, slot: u64) {
        self.svm.warp_to_slot(slot);
    }

    pub fn send(
        &mut self,
        ixs: &[Instruction],
        signers: &[&Keypair],
    ) -> Result<TransactionMetadata, FailedTransactionMetadata> {
        let mut all = vec![compute_limit(1_400_000)];
        all.extend_from_slice(ixs);
        let tx = Transaction::new_signed_with_payer(
            &all,
            Some(&signers[0].pubkey()),
            signers,
            self.svm.latest_blockhash(),
        );
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

    pub fn send_err(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> String {
        match self.send(ixs, signers) {
            Ok(m) => panic!("tx unexpectedly succeeded:\n{}", m.logs.join("\n")),
            Err(e) => format!("{:?}\n{}", e.err, e.meta.logs.join("\n")),
        }
    }

    pub fn get<T: AccountDeserialize>(&self, key: &Pk) -> T {
        let a = self
            .svm
            .get_account(&addr(key))
            .unwrap_or_else(|| panic!("missing account {key}"));
        T::try_deserialize(&mut &a.data[..]).unwrap()
    }

    pub fn exists(&self, key: &Pk) -> bool {
        self.svm
            .get_account(&addr(key))
            .is_some_and(|a| a.lamports > 0)
    }

    pub fn lamports(&self, key: &Pk) -> u64 {
        self.svm
            .get_account(&addr(key))
            .map(|a| a.lamports)
            .unwrap_or(0)
    }

    pub fn token_balance(&self, account: &Pk) -> u64 {
        self.svm
            .get_account(&addr(account))
            .map(|a| u64::from_le_bytes(a.data[64..72].try_into().unwrap()))
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------- registry

pub mod registry {
    use super::*;
    use hook_registry::{
        accounts as A, instruction as I, PublishParams, CONFIG_SEED, HOOK_SEED, REVIEWER_SEED,
        REVIEW_SEED, VERSION_SEED,
    };

    pub const ID: Pk = hook_registry::ID;
    pub const SYS: Pk = anchor_lang::system_program::ID;

    pub fn config() -> Pk {
        Pk::find_program_address(&[CONFIG_SEED], &ID).0
    }
    pub fn hook(slug: &str) -> Pk {
        Pk::find_program_address(&[HOOK_SEED, slug.as_bytes()], &ID).0
    }
    pub fn version(hook: &Pk, index: u32) -> Pk {
        Pk::find_program_address(&[VERSION_SEED, hook.as_ref(), &index.to_le_bytes()], &ID).0
    }
    pub fn reviewer(key: &Pk) -> Pk {
        Pk::find_program_address(&[REVIEWER_SEED, key.as_ref()], &ID).0
    }
    pub fn review(version: &Pk, reviewer: &Pk) -> Pk {
        Pk::find_program_address(&[REVIEW_SEED, version.as_ref(), reviewer.as_ref()], &ID).0
    }

    /// Deploys the registry with `admin` as upgrade authority and initializes it.
    pub fn setup(s: &mut Svm, admin: &Keypair) {
        s.add_program(&ID, "hook_registry.so");
        s.set_upgrade_authority(&ID, Some(kp(admin)));
        s.send_ok(&[initialize(&kp(admin))], &[admin]);
    }

    pub fn initialize(admin: &Pk) -> Instruction {
        ix(
            ID,
            A::Initialize {
                admin: *admin,
                config: config(),
                registry_program: ID,
                registry_program_data: programdata(&ID),
                system_program: SYS,
            },
            vec![],
            I::Initialize {},
        )
    }

    pub fn create_hook(authority: &Pk, slug: &str) -> Instruction {
        ix(
            ID,
            A::CreateHook {
                authority: *authority,
                hook: hook(slug),
                system_program: SYS,
            },
            vec![],
            I::CreateHook { slug: slug.into() },
        )
    }

    pub fn publish(
        publisher: &Pk,
        hook_key: &Pk,
        index: u32,
        program: Option<Pk>,
        p: PublishParams,
    ) -> Instruction {
        ix(
            ID,
            A::PublishVersion {
                publisher: *publisher,
                hook: *hook_key,
                config: config(),
                version: version(hook_key, index),
                hook_program: Some(program.unwrap_or(ID)),
                hook_program_data: Some(program.map(|p| programdata(&p)).unwrap_or(ID)),
                system_program: SYS,
            },
            vec![],
            I::PublishVersion { p },
        )
    }

    pub fn builtin_params(
        id: u8,
        env: hook_standard::Environment,
        callbacks: u16,
        semver: [u16; 3],
    ) -> PublishParams {
        let mut cbs = [0u16; 4];
        cbs[env as usize] = callbacks;
        PublishParams {
            semver,
            impl_kind: hook_standard::IMPL_BUILTIN,
            builtin_id: id,
            environments: env.bit(),
            callbacks: cbs,
            capabilities: hook_kernel::builtin::required_caps(id),
            config_schema_hash: [1; 32],
            manifest_hash: [2; 32],
            manifest_uri: format!(
                "https://pumphooks.fun/manifests/{}.json",
                hook_kernel::builtin::name(id).unwrap()
            ),
            artifact_hash: [0; 32],
        }
    }

    pub fn program_params(
        env: hook_standard::Environment,
        callbacks: u16,
        caps: u32,
        semver: [u16; 3],
    ) -> PublishParams {
        let mut cbs = [0u16; 4];
        cbs[env as usize] = callbacks;
        PublishParams {
            semver,
            impl_kind: hook_standard::IMPL_PROGRAM,
            builtin_id: 0,
            environments: env.bit(),
            callbacks: cbs,
            capabilities: caps,
            config_schema_hash: [1; 32],
            manifest_hash: [2; 32],
            manifest_uri: "https://example.invalid/manifest.json".into(),
            artifact_hash: [3; 32],
        }
    }

    /// Creates `slug` (owned by `admin`) and publishes builtin `id` as version 0.
    pub fn publish_builtin(
        s: &mut Svm,
        admin: &Keypair,
        slug: &str,
        id: u8,
        env: hook_standard::Environment,
        callbacks: u16,
    ) -> Pk {
        let h = hook(slug);
        s.send_ok(
            &[
                create_hook(&kp(admin), slug),
                publish(
                    &kp(admin),
                    &h,
                    0,
                    None,
                    builtin_params(id, env, callbacks, [1, 0, 0]),
                ),
            ],
            &[admin],
        );
        version(&h, 0)
    }
}
