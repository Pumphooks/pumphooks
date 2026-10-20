//! On-chain program-hook invocation for policy environments.
//!
//! Capability enforcement is done here, by what is forwarded:
//! - account 0: the environment's callback-authority PDA for the hook (the only signer);
//! - account 1: the subject, read-only;
//! - extra accounts only with a READ_* / CONSUME_STATE capability, always without signatures,
//!   writable only with UPDATE_OWN_STATE *and* when owned by the hook program.
//!
//! The result must come from the hook itself (return-data program id) and decode strictly.

use hook_standard::{
    cap, loader, policy_discriminator, Environment, PolicyContext, PolicyResult, StackEntry,
    CALLBACK_SEED, POLICY_CONTEXT_LEN,
};
use solana_program::{
    account_info::AccountInfo,
    instruction::{AccountMeta, Instruction},
    program::{get_return_data, invoke_signed},
    pubkey::Pubkey,
};

use crate::ProgramHooks;

const READ_CAPS: u32 = cap::READ_ACCOUNT_STATE | cap::READ_TOKEN_BALANCE | cap::CONSUME_STATE;

pub struct CpiPolicyHooks<'a, 'info> {
    pub environment: Environment,
    /// The calling environment program (signs as `["callback", hook]`).
    pub environment_program: &'a Pubkey,
    pub subject: &'a AccountInfo<'info>,
    /// Remaining accounts: hook programs, their callback authorities, ProgramData accounts for
    /// pinned hooks, and any accounts to forward.
    pub accounts: &'a [AccountInfo<'info>],
    /// Opaque per-operation data appended after the context.
    pub op_data: &'a [u8],
}

pub fn callback_authority(environment_program: &Pubkey, hook: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[CALLBACK_SEED, hook.as_ref()], environment_program)
}

impl<'a, 'info> CpiPolicyHooks<'a, 'info> {
    fn find(&self, key: &Pubkey) -> Option<&'a AccountInfo<'info>> {
        self.accounts.iter().find(|a| a.key == key)
    }
}

impl<'a, 'info> ProgramHooks for CpiPolicyHooks<'a, 'info> {
    fn call(
        &mut self,
        _index: usize,
        entry: &StackEntry,
        mut ctx: PolicyContext,
    ) -> Result<PolicyResult, ()> {
        let hook = entry.program();
        let program = self.find(&hook).ok_or(())?.clone();
        if !program.executable {
            return Err(());
        }
        if entry.pinned_slot != 0 {
            let pd_key = loader::program_data_address(&program).ok_or(())?;
            let pd = self.find(&pd_key).ok_or(())?;
            let info = loader::read_program(&program, Some(pd)).map_err(|_| ())?;
            if info.deploy_slot != entry.pinned_slot {
                return Err(());
            }
        }
        let (authority_key, bump) = callback_authority(self.environment_program, &hook);
        let authority = self.find(&authority_key).ok_or(())?.clone();
        ctx.authority_bump = bump;

        let mut data = Vec::with_capacity(8 + POLICY_CONTEXT_LEN + self.op_data.len());
        data.extend_from_slice(&policy_discriminator(self.environment, ctx.callback).ok_or(())?);
        data.extend_from_slice(bytemuck::bytes_of(&ctx));
        data.extend_from_slice(self.op_data);

        let mut metas = vec![
            AccountMeta::new_readonly(authority_key, true),
            AccountMeta::new_readonly(*self.subject.key, false),
        ];
        let mut infos: Vec<AccountInfo<'info>> = vec![authority, self.subject.clone()];
        if entry.granted & READ_CAPS != 0 {
            let may_write = entry.granted & cap::UPDATE_OWN_STATE != 0;
            for a in self.accounts {
                let skip = a.key == &hook
                    || a.key == &authority_key
                    || a.key == self.subject.key
                    || a.executable;
                if skip || metas.iter().any(|m| m.pubkey == *a.key) {
                    continue;
                }
                let writable = may_write && a.is_writable && a.owner == &hook;
                metas.push(AccountMeta {
                    pubkey: *a.key,
                    is_signer: false,
                    is_writable: writable,
                });
                infos.push(a.clone());
            }
        } else if entry.granted & cap::UPDATE_OWN_STATE != 0 {
            for a in self
                .accounts
                .iter()
                .filter(|a| a.owner == &hook && !a.executable)
            {
                metas.push(AccountMeta {
                    pubkey: *a.key,
                    is_signer: false,
                    is_writable: a.is_writable,
                });
                infos.push(a.clone());
            }
        }
        infos.push(program);

        let ix = Instruction {
            program_id: hook,
            accounts: metas,
            data,
        };
        invoke_signed(&ix, &infos, &[&[CALLBACK_SEED, hook.as_ref(), &[bump]]]).map_err(|_| ())?;
        let (from, bytes) = get_return_data().ok_or(())?;
        if from != hook {
            return Err(());
        }
        PolicyResult::decode(&bytes).ok_or(())
    }
}
