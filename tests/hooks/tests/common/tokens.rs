//! SPL Token helpers built from real token-program instructions.

use anchor_spl::token::spl_token;
use solana_keypair::Keypair;

use super::*;

pub const SPL: Pk = anchor_spl::token::ID;

fn spl_ix(ix: anchor_lang::solana_program::instruction::Instruction) -> Instruction {
    convert(ix.program_id, ix.accounts, ix.data)
}

fn create_account_ix(payer: &Pk, new: &Pk, lamports: u64, space: u64, owner: &Pk) -> Instruction {
    spl_ix(
        anchor_lang::solana_program::system_instruction::create_account(
            payer, new, lamports, space, owner,
        ),
    )
}

impl Svm {
    pub fn create_mint(&mut self, payer: &Keypair, decimals: u8) -> Pk {
        let mint = Keypair::new();
        let m = kp(&mint);
        let rent = self.svm.minimum_balance_for_rent_exemption(82);
        let ixs = [
            create_account_ix(&kp(payer), &m, rent, 82, &SPL),
            spl_ix(
                spl_token::instruction::initialize_mint2(&SPL, &m, &kp(payer), None, decimals)
                    .unwrap(),
            ),
        ];
        self.send_ok(&ixs, &[payer, &mint]);
        m
    }

    pub fn create_token_account(&mut self, payer: &Keypair, mint: &Pk, owner: &Pk) -> Pk {
        let acct = Keypair::new();
        let a = kp(&acct);
        let rent = self.svm.minimum_balance_for_rent_exemption(165);
        let ixs = [
            create_account_ix(&kp(payer), &a, rent, 165, &SPL),
            spl_ix(spl_token::instruction::initialize_account3(&SPL, &a, mint, owner).unwrap()),
        ];
        self.send_ok(&ixs, &[payer, &acct]);
        a
    }

    /// `authority` must be the mint authority.
    pub fn mint_to(&mut self, authority: &Keypair, mint: &Pk, to: &Pk, amount: u64) {
        let ix = spl_ix(
            spl_token::instruction::mint_to(&SPL, mint, to, &kp(authority), &[], amount).unwrap(),
        );
        self.send_ok(&[ix], &[authority]);
    }
}

pub fn transfer_ix(
    from: &Pk,
    to: &Pk,
    authority: &Pk,
    amount: u64,
) -> anchor_lang::solana_program::instruction::Instruction {
    spl_token::instruction::transfer(&SPL, from, to, authority, &[], amount).unwrap()
}

pub fn approve_ix(
    account: &Pk,
    delegate: &Pk,
    owner: &Pk,
    amount: u64,
) -> anchor_lang::solana_program::instruction::Instruction {
    spl_token::instruction::approve(&SPL, account, delegate, owner, &[], amount).unwrap()
}

pub fn close_ix(
    account: &Pk,
    dest: &Pk,
    owner: &Pk,
) -> anchor_lang::solana_program::instruction::Instruction {
    spl_token::instruction::close_account(&SPL, account, dest, owner, &[]).unwrap()
}
