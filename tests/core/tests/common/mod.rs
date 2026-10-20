//! LiteSVM harness for the core: the locally built core and test hook, SPL Token / Token-2022
//! mints created with real instructions, and builders for every core instruction.

#![allow(dead_code, deprecated)]

use std::{path::PathBuf, str::FromStr};

use anchor_lang::{
    solana_program::{instruction::AccountMeta as Meta2, system_instruction},
    AccountDeserialize, InstructionData, ToAccountMetas,
};
use anchor_spl::token_2022::spl_token_2022::{
    self,
    extension::{transfer_fee, ExtensionType},
    state::{Account as Account2022, Mint as Mint2022},
};
use hooks_core::{
    accounts as A, instruction as I,
    instructions::{InitPoolParams, ProtocolUpdate},
    state::*,
};
use hooks_interface::{callback_authority, hook_authority, Callback, HookResult};
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
pub const SPL: Pk = anchor_spl::token::ID;
pub const T22: Pk = spl_token_2022::ID;
pub const CORE: Pk = hooks_core::ID;

pub fn addr(p: &Pk) -> Address {
    Address::new_from_array(p.to_bytes())
}
pub fn pk(a: &Address) -> Pk {
    Pk::new_from_array(a.to_bytes())
}
pub fn kp(k: &Keypair) -> Pk {
    pk(&k.pubkey())
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn convert(program: Pk, metas: Vec<Meta2>, data: Vec<u8>) -> Instruction {
    Instruction {
        program_id: addr(&program),
        accounts: metas
            .into_iter()
            .map(|m| AccountMeta { pubkey: addr(&m.pubkey), is_signer: m.is_signer, is_writable: m.is_writable })
            .collect(),
        data,
    }
}

fn spl_ix(ix: anchor_lang::solana_program::instruction::Instruction) -> Instruction {
    convert(ix.program_id, ix.accounts, ix.data)
}

pub fn core_ix(accounts: impl ToAccountMetas, extra: Vec<Meta2>, data: impl InstructionData) -> Instruction {
    let mut metas = accounts.to_account_metas(None);
    metas.extend(extra);
    convert(CORE, metas, data.data())
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

/// Test-hook script: return `r` for callback `cb`.
pub fn script(entries: &[(Callback, HookResult)]) -> Vec<u8> {
    let mut v = vec![];
    for (cb, r) in entries {
        v.push(*cb as u8);
        v.extend_from_slice(bytemuck::bytes_of(r));
    }
    v
}

#[derive(Clone, Copy, Default)]
pub enum MintKind {
    #[default]
    Spl,
    T22Plain,
    T22TransferFee(u16),
    T22PermanentDelegate,
}

#[derive(Clone, Copy, Debug)]
pub struct PoolKeys {
    pub pool: Pk,
    pub mint: [Pk; 2],
    pub token_program: [Pk; 2],
    pub vault: [Pk; 2],
    pub lp_mint: Pk,
    pub hook: Option<Pk>,
}

pub struct User {
    pub kp: Keypair,
    pub token: [Pk; 2],
    pub lp: Pk,
}

impl User {
    pub fn key(&self) -> Pk {
        kp(&self.kp)
    }
}

pub struct Env {
    pub svm: LiteSVM,
    pub admin: Keypair,
    pub hook_dev: Keypair,
    pub hook: Pk,
    pub protocol: Pk,
}

pub fn programdata(program: &Pk) -> Pk {
    Pk::find_program_address(&[program.as_ref()], &anchor_lang::solana_program::bpf_loader_upgradeable::ID).0
}

impl Env {
    pub fn new() -> Self {
        let mut env = Self::uninitialized();
        let ix = env.init_protocol_ix(&kp(&env.admin));
        let admin = env.admin.insecure_clone();
        env.send_ok(&[ix], &[&admin]);
        env
    }

    /// Programs deployed, protocol not yet initialized.
    pub fn uninitialized() -> Self {
        let mut svm = LiteSVM::new();
        let deploy = root().join("target/deploy");
        svm.add_program_from_file(addr(&CORE), deploy.join("hooks_core.so"))
            .expect("run scripts/build_core.sh programs/test-hook first");
        let hook = Pk::new_unique();
        svm.add_program_from_file(addr(&hook), deploy.join("test_hook.so")).unwrap();
        let admin = Keypair::new();
        let hook_dev = Keypair::new();
        svm.airdrop(&admin.pubkey(), 1_000 * SOL).unwrap();
        svm.airdrop(&hook_dev.pubkey(), 100 * SOL).unwrap();
        let protocol = Pk::find_program_address(&[PROTOCOL_SEED], &CORE).0;
        let mut env = Env { svm, admin, hook_dev, hook, protocol };
        env.set_upgrade_authority(&CORE, Some(kp(&env.admin)));
        env.set_upgrade_authority(&hook, Some(kp(&env.hook_dev)));
        env
    }

    pub fn init_protocol_ix(&self, admin: &Pk) -> Instruction {
        core_ix(
            A::InitializeProtocol {
                admin: *admin,
                protocol: self.protocol,
                core_program: CORE,
                core_program_data: programdata(&CORE),
                system_program: anchor_lang::system_program::ID,
            },
            vec![],
            I::InitializeProtocol { fee_authority: *admin },
        )
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

    /// Simulates a redeploy by moving the ProgramData deploy slot.
    pub fn set_deploy_slot(&mut self, program: &Pk, slot: u64) {
        let key = addr(&programdata(program));
        let mut a = self.svm.get_account(&key).unwrap();
        a.data[4..12].copy_from_slice(&slot.to_le_bytes());
        self.svm.set_account(key, a).unwrap();
    }

    pub fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> Result<TransactionMetadata, FailedTransactionMetadata> {
        let mut all = vec![compute_limit(1_400_000)];
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

    /// Sends a transaction that must fail and returns its logs joined, for matching the reason.
    pub fn send_err(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> String {
        match self.send(ixs, signers) {
            Ok(m) => panic!("tx unexpectedly succeeded:\n{}", m.logs.join("\n")),
            Err(e) => format!("{:?}\n{}", e.err, e.meta.logs.join("\n")),
        }
    }

    pub fn balance(&self, account: &Pk) -> u64 {
        self.svm
            .get_account(&addr(account))
            .map(|a| u64::from_le_bytes(a.data[64..72].try_into().unwrap()))
            .unwrap_or(0)
    }

    pub fn pool(&self, key: &Pk) -> Pool {
        let a = self.svm.get_account(&addr(key)).unwrap();
        Pool::try_deserialize(&mut &a.data[..]).unwrap()
    }

    pub fn protocol_state(&self) -> Protocol {
        let a = self.svm.get_account(&addr(&self.protocol)).unwrap();
        Protocol::try_deserialize(&mut &a.data[..]).unwrap()
    }

    pub fn account_exists(&self, key: &Pk) -> bool {
        self.svm.get_account(&addr(key)).is_some_and(|a| a.lamports > 0)
    }

    // ------------------------------------------------------------ tokens

    pub fn create_mint(&mut self, kind: MintKind) -> (Pk, Pk) {
        let mint = Keypair::new();
        let m = kp(&mint);
        let a = kp(&self.admin);
        let (program, exts) = match kind {
            MintKind::Spl => (SPL, vec![]),
            MintKind::T22Plain => (T22, vec![]),
            MintKind::T22TransferFee(_) => (T22, vec![ExtensionType::TransferFeeConfig]),
            MintKind::T22PermanentDelegate => (T22, vec![ExtensionType::PermanentDelegate]),
        };
        let space = if program == SPL { 82 } else { ExtensionType::try_calculate_account_len::<Mint2022>(&exts).unwrap() };
        let lamports = self.svm.minimum_balance_for_rent_exemption(space);
        let mut ixs = vec![spl_ix(system_instruction::create_account(&a, &m, lamports, space as u64, &program))];
        match kind {
            MintKind::T22TransferFee(bps) => ixs.push(spl_ix(
                transfer_fee::instruction::initialize_transfer_fee_config(&program, &m, Some(&a), Some(&a), bps, u64::MAX)
                    .unwrap(),
            )),
            MintKind::T22PermanentDelegate => {
                ixs.push(spl_ix(spl_token_2022::instruction::initialize_permanent_delegate(&program, &m, &a).unwrap()))
            }
            _ => {}
        }
        ixs.push(spl_ix(spl_token_2022::instruction::initialize_mint2(&program, &m, &a, None, 6).unwrap()));
        let admin = self.admin.insecure_clone();
        self.send_ok(&ixs, &[&admin, &mint]);
        (m, program)
    }

    /// Two mints sorted so the first is `mint_0`.
    pub fn mint_pair(&mut self, k0: MintKind, k1: MintKind) -> ([Pk; 2], [Pk; 2]) {
        let a = self.create_mint(k0);
        let b = self.create_mint(k1);
        if a.0 < b.0 { ([a.0, b.0], [a.1, b.1]) } else { ([b.0, a.0], [b.1, a.1]) }
    }

    pub fn token_account(&mut self, mint: &Pk, program: &Pk, owner: &Pk) -> Pk {
        let acct = Keypair::new();
        let k = kp(&acct);
        let a = kp(&self.admin);
        let space = if *program == SPL {
            165
        } else {
            let data = self.svm.get_account(&addr(mint)).unwrap().data;
            let state = spl_token_2022::extension::StateWithExtensions::<Mint2022>::unpack(&data).unwrap();
            use spl_token_2022::extension::BaseStateWithExtensions;
            let req = ExtensionType::get_required_init_account_extensions(&state.get_extension_types().unwrap());
            ExtensionType::try_calculate_account_len::<Account2022>(&req).unwrap()
        };
        let lamports = self.svm.minimum_balance_for_rent_exemption(space);
        let ixs = [
            spl_ix(system_instruction::create_account(&a, &k, lamports, space as u64, program)),
            spl_ix(spl_token_2022::instruction::initialize_account3(program, &k, mint, owner).unwrap()),
        ];
        let admin = self.admin.insecure_clone();
        self.send_ok(&ixs, &[&admin, &acct]);
        k
    }

    pub fn mint_to(&mut self, mint: &Pk, program: &Pk, to: &Pk, amount: u64) {
        let a = kp(&self.admin);
        let ix = spl_ix(spl_token_2022::instruction::mint_to(program, mint, to, &a, &[], amount).unwrap());
        let admin = self.admin.insecure_clone();
        self.send_ok(&[ix], &[&admin]);
    }

    /// A funded wallet with token accounts for both pool mints and the LP mint.
    pub fn user(&mut self, p: &PoolKeys, amount: u64) -> User {
        let k = Keypair::new();
        self.svm.airdrop(&k.pubkey(), 10 * SOL).unwrap();
        let owner = kp(&k);
        let t0 = self.token_account(&p.mint[0], &p.token_program[0], &owner);
        let t1 = self.token_account(&p.mint[1], &p.token_program[1], &owner);
        let lp = self.token_account(&p.lp_mint, &SPL, &owner);
        self.mint_to(&p.mint[0], &p.token_program[0], &t0, amount);
        self.mint_to(&p.mint[1], &p.token_program[1], &t1, amount);
        User { kp: k, token: [t0, t1], lp }
    }

    // ------------------------------------------------------------ registry and pools

    pub fn registration(&self, hook: &Pk) -> Pk {
        Pk::find_program_address(&[HOOK_SEED, hook.as_ref()], &CORE).0
    }

    pub fn register_hook_ix(&self, registrant: &Pk, hook: &Pk, permissions: u32) -> Instruction {
        core_ix(
            A::RegisterHook {
                registrant: *registrant,
                protocol: self.protocol,
                hook_program: *hook,
                hook_program_data: Some(programdata(hook)),
                registration: self.registration(hook),
                system_program: anchor_lang::system_program::ID,
            },
            vec![],
            I::RegisterHook { permissions, interface_version: 1, metadata_uri: "ipfs://test-hook".into() },
        )
    }

    pub fn register_hook(&mut self, permissions: u32) {
        let ix = self.register_hook_ix(&kp(&self.hook_dev), &self.hook.clone(), permissions);
        let dev = self.hook_dev.insecure_clone();
        self.send_ok(&[ix], &[&dev]);
    }

    pub fn pool_keys(&self, mint: [Pk; 2], token_program: [Pk; 2], hook: Option<Pk>, salt: u64) -> PoolKeys {
        let h = hook.unwrap_or_default();
        let pool = Pk::find_program_address(
            &[POOL_SEED, mint[0].as_ref(), mint[1].as_ref(), h.as_ref(), &salt.to_le_bytes()],
            &CORE,
        )
        .0;
        let vault = mint.map(|m| Pk::find_program_address(&[VAULT_SEED, pool.as_ref(), m.as_ref()], &CORE).0);
        let lp_mint = Pk::find_program_address(&[LP_SEED, pool.as_ref()], &CORE).0;
        PoolKeys { pool, mint, token_program, vault, lp_mint, hook }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_pool_ix(
        &self,
        creator: &Pk,
        p: &PoolKeys,
        permissions: u32,
        fee_ppm: u32,
        max_fee_ppm: u32,
        salt: u64,
        pin: bool,
        hook_data: Vec<u8>,
    ) -> Instruction {
        let h = p.hook;
        core_ix(
            A::InitializePool {
                creator: *creator,
                protocol: self.protocol,
                mint_0: p.mint[0],
                mint_1: p.mint[1],
                pool: p.pool,
                vault_0: p.vault[0],
                vault_1: p.vault[1],
                lp_mint: p.lp_mint,
                hook_registration: h.map(|h| self.registration(&h)),
                hook_program: h,
                callback_authority: h.map(|h| callback_authority(&h).0),
                hook_program_data: h.map(|h| programdata(&h)),
                token_program_0: p.token_program[0],
                token_program_1: p.token_program[1],
                lp_token_program: SPL,
                system_program: anchor_lang::system_program::ID,
            },
            vec![],
            I::InitializePool {
                params: InitPoolParams {
                    hook_program: h.unwrap_or_default(),
                    permissions,
                    fee_ppm,
                    max_fee_ppm,
                    salt,
                    pin_hook: pin,
                },
                hook_data,
            },
        )
    }

    /// Creates a pool (3000 ppm fee, 100_000 max) and seeds it with `seed` of each token.
    pub fn pool_with(&mut self, kinds: (MintKind, MintKind), hook: Option<u32>, seed: u64) -> (PoolKeys, User) {
        let (mint, tp) = self.mint_pair(kinds.0, kinds.1);
        let hook_key = hook.map(|_| self.hook);
        if let Some(perms) = hook {
            if !self.account_exists(&self.registration(&self.hook.clone())) {
                self.register_hook(perms);
            }
        }
        let p = self.pool_keys(mint, tp, hook_key, 0);
        let creator = self.admin.insecure_clone();
        let ix = self.create_pool_ix(&kp(&creator), &p, hook.unwrap_or(0), 3_000, 100_000, 0, false, vec![]);
        self.send_ok(&[ix], &[&creator]);
        let lp = self.user(&p, seed * 10);
        if seed > 0 {
            let ix = self.add_liquidity_ix(&p, &lp, seed, seed, 0, vec![]);
            let k = lp.kp.insecure_clone();
            self.send_ok(&[ix], &[&k]);
        }
        (p, lp)
    }

    pub fn pool_io(&self, p: &PoolKeys) -> A::PoolIo {
        A::PoolIo {
            pool: p.pool,
            vault_0: p.vault[0],
            vault_1: p.vault[1],
            mint_0: p.mint[0],
            mint_1: p.mint[1],
            token_program_0: p.token_program[0],
            token_program_1: p.token_program[1],
        }
    }

    pub fn hook_io(&self, p: &PoolKeys) -> A::HookIo {
        A::HookIo {
            hook_program: p.hook,
            callback_authority: p.hook.map(|h| callback_authority(&h).0),
            hook_program_data: p.hook.map(|h| programdata(&h)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn swap_ix_extra(
        &self,
        p: &PoolKeys,
        u: &User,
        zero_for_one: bool,
        exact_in: bool,
        amount: u64,
        limit: u64,
        hook_data: Vec<u8>,
        extra: Vec<Meta2>,
    ) -> Instruction {
        core_ix(
            A::Swap {
                trader: u.key(),
                protocol: self.protocol,
                io: self.pool_io(p),
                trader_token_0: u.token[0],
                trader_token_1: u.token[1],
                hooks: self.hook_io(p),
            },
            extra,
            I::Swap { zero_for_one, exact_in, amount, limit, hook_data },
        )
    }

    pub fn swap_ix(&self, p: &PoolKeys, u: &User, zfo: bool, exact_in: bool, amount: u64, limit: u64, hook_data: Vec<u8>) -> Instruction {
        self.swap_ix_extra(p, u, zfo, exact_in, amount, limit, hook_data, vec![])
    }

    fn liquidity(&self, p: &PoolKeys, u: &User) -> A::Liquidity {
        A::Liquidity {
            owner: u.key(),
            protocol: self.protocol,
            io: self.pool_io(p),
            lp_mint: p.lp_mint,
            owner_token_0: u.token[0],
            owner_token_1: u.token[1],
            owner_lp: u.lp,
            lp_token_program: SPL,
            hooks: self.hook_io(p),
        }
    }

    pub fn add_liquidity_ix(&self, p: &PoolKeys, u: &User, max_0: u64, max_1: u64, min_shares: u64, hook_data: Vec<u8>) -> Instruction {
        core_ix(self.liquidity(p, u), vec![], I::AddLiquidity { max_0, max_1, min_shares, hook_data })
    }

    pub fn remove_liquidity_ix(&self, p: &PoolKeys, u: &User, shares: u64, min_0: u64, min_1: u64, hook_data: Vec<u8>) -> Instruction {
        core_ix(self.liquidity(p, u), vec![], I::RemoveLiquidity { shares, min_0, min_1, hook_data })
    }

    pub fn donate_ix(&self, p: &PoolKeys, u: &User, amount_0: u64, amount_1: u64) -> Instruction {
        core_ix(
            A::Donate {
                donor: u.key(),
                protocol: self.protocol,
                io: self.pool_io(p),
                donor_token_0: u.token[0],
                donor_token_1: u.token[1],
                hooks: self.hook_io(p),
            },
            vec![],
            I::Donate { amount_0, amount_1, hook_data: vec![] },
        )
    }

    pub fn credit_deposit_ix(&self, p: &PoolKeys, u: &User, amount_0: u64, amount_1: u64) -> Instruction {
        core_ix(
            A::CreditDeposit {
                depositor: u.key(),
                io: self.pool_io(p),
                depositor_token_0: u.token[0],
                depositor_token_1: u.token[1],
            },
            vec![],
            I::CreditDeposit { amount_0, amount_1 },
        )
    }

    pub fn hook_op_accounts(&self, p: &PoolKeys, authority: Pk, dest: Option<[Pk; 2]>) -> A::HookOp {
        A::HookOp {
            hook_authority: authority,
            protocol: self.protocol,
            io: self.pool_io(p),
            destination_0: dest.map(|d| d[0]),
            destination_1: dest.map(|d| d[1]),
        }
    }

    /// Wraps a core instruction so the test hook signs it with its hook-authority PDA.
    pub fn via_hook(&self, core: Instruction) -> Instruction {
        let auth = addr(&hook_authority(&self.hook).0);
        let mut accounts = vec![AccountMeta::new_readonly(addr(&CORE), false)];
        accounts.extend(core.accounts.into_iter().map(|mut m| {
            if m.pubkey == auth {
                m.is_signer = false;
            }
            m
        }));
        let mut data = vec![test_hook::PROXY];
        data.extend(core.data);
        Instruction { program_id: addr(&self.hook), accounts, data }
    }

    pub fn update_protocol(&mut self, u: ProtocolUpdate) {
        let ix = core_ix(
            A::UpdateProtocol { admin: kp(&self.admin), protocol: self.protocol },
            vec![],
            I::UpdateProtocol { update: u },
        );
        let admin = self.admin.insecure_clone();
        self.send_ok(&[ix], &[&admin]);
    }

    /// Asserts each vault holds at least what the pool owes on that side.
    pub fn assert_solvent(&self, p: &PoolKeys) {
        let s = self.pool(&p.pool);
        let owed = [
            s.reserve_0 + s.credit_0 + s.protocol_fee_0,
            s.reserve_1 + s.credit_1 + s.protocol_fee_1,
        ];
        for i in 0..2 {
            assert!(self.balance(&p.vault[i]) >= owed[i], "vault {i} insolvent");
        }
    }
}
