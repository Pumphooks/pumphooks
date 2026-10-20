//! Rust SDK for PUMP HOOKS program hooks.
//!
//! A program hook is a Solana program that an environment (the router, or a smart account) calls
//! with a `PolicyContext` before or after an operation. It returns allow or reject (with an
//! optional output for later entries). Implement [`PolicyHook`] and call [`process`] from the
//! program's entrypoint:
//!
//! ```ignore
//! struct MaxIn;
//! impl hook_sdk::PolicyHook for MaxIn {
//!     const ENVIRONMENT: hook_sdk::Environment = hook_sdk::Environment::Router;
//!     const ENVIRONMENT_PROGRAM: Pubkey = hook_sdk::ROUTER_PROGRAM_ID;
//!     fn evaluate(_: &Pubkey, call: &hook_sdk::Call, _: &[AccountInfo]) -> Result<hook_sdk::Decision, ProgramError> {
//!         Ok(if call.amount() > 1_000 { hook_sdk::Decision::reject(1) } else { hook_sdk::Decision::allow() })
//!     }
//! }
//! ```
//!
//! What the SDK guarantees before `evaluate` runs: the instruction is a policy call for
//! `ENVIRONMENT` (namespaced discriminator, ABI version), account 0 is that environment's
//! callback-authority PDA for this hook and signed, and account 1 is the subject owned by the
//! environment program. Anything else fails closed: the environment treats an error or a missing
//! result as a rejection.
//!
//! What a hook cannot do: it never receives the user's signature, so it cannot move user funds;
//! it only sees accounts the environment forwards (read-only, except accounts the hook program
//! itself owns when granted `UPDATE_OWN_STATE`).

use solana_program::{account_info::AccountInfo, entrypoint::ProgramResult, program_error::ProgramError, pubkey, pubkey::Pubkey};

pub use hook_standard::{
    cap, decode_policy_call, output, policy_discriminator, state, verify_policy_call, Environment, Output, PolicyContext, PolicyResult,
    ABI_VERSION, CALLBACK_SEED, POLICY_CONTEXT_LEN,
};

/// `hook-router`, the router environment.
pub const ROUTER_PROGRAM_ID: Pubkey = pubkey!("2a3wWdS8z9g7vBcYWSjdETyEzo7DEjV3iYRZFsM52q4U");
/// `smart-accounts`, the account environment.
pub const SMART_ACCOUNTS_PROGRAM_ID: Pubkey = pubkey!("6AEiwvifk4TVJ8g3UoVXqhaK8AqYy43A4DzWCgMmrtQw");

/// Hook-specific rejection reasons should start here; lower values are the kernel's.
pub const CUSTOM_REASON_BASE: u16 = 1000;

/// A verified policy call.
pub struct Call<'a> {
    pub ctx: PolicyContext,
    pub op_data: &'a [u8],
}

impl Call<'_> {
    pub fn callback(&self) -> u8 {
        self.ctx.callback
    }
    pub fn callback_name(&self, env: Environment) -> &'static str {
        env.callbacks().get(self.ctx.callback as usize).copied().unwrap_or("unknown")
    }
    pub fn subject(&self) -> Pubkey {
        Pubkey::new_from_array(self.ctx.subject)
    }
    pub fn actor(&self) -> Pubkey {
        Pubkey::new_from_array(self.ctx.actor)
    }
    pub fn asset(&self) -> Pubkey {
        Pubkey::new_from_array(self.ctx.asset)
    }
    pub fn counterparty(&self) -> Pubkey {
        Pubkey::new_from_array(self.ctx.counterparty)
    }
    pub fn amount(&self) -> u64 {
        self.ctx.amount
    }
    pub fn amount_b(&self) -> u64 {
        self.ctx.amount_b
    }
    pub fn slot(&self) -> u64 {
        self.ctx.slot
    }
    pub fn unix_timestamp(&self) -> i64 {
        self.ctx.unix_timestamp
    }
    /// Outputs of earlier entries this entry was installed to depend on.
    pub fn inputs(&self) -> impl Iterator<Item = &Output> {
        self.ctx.inputs.iter().filter(|o| o.kind != 0)
    }
}

/// What the hook returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision(pub PolicyResult);

impl Decision {
    pub const fn allow() -> Self {
        Self(PolicyResult::allow())
    }
    pub const fn reject(reason: u16) -> Self {
        Self(PolicyResult::reject(reason))
    }
    /// Publishes `(kind, value)` to later entries that depend on this one.
    pub const fn with_output(self, kind: u16, value: u64) -> Self {
        Self(self.0.with_output(kind, value))
    }
    pub fn is_allow(&self) -> bool {
        self.0.decision == hook_standard::DECISION_ALLOW
    }
}

pub trait PolicyHook {
    const ENVIRONMENT: Environment;
    const ENVIRONMENT_PROGRAM: Pubkey;
    /// `accounts[0]` is the callback authority, `accounts[1]` the subject; the rest are what the
    /// environment forwarded for this entry's capabilities.
    fn evaluate(program_id: &Pubkey, call: &Call, accounts: &[AccountInfo]) -> Result<Decision, ProgramError>;
}

/// Entry point for policy calls. Returns `Ok(false)` when `data` is not a policy call for
/// `H::ENVIRONMENT`, so the program can handle its own instructions.
pub fn process<H: PolicyHook>(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> Result<bool, ProgramError> {
    let Some(call) = decode_policy_call(H::ENVIRONMENT, data) else {
        return Ok(false);
    };
    verify_policy_call(program_id, &H::ENVIRONMENT_PROGRAM, accounts, &call.ctx)?;
    let call = Call { ctx: call.ctx, op_data: call.op_data };
    let d = H::evaluate(program_id, &call, accounts)?;
    d.0.set();
    Ok(true)
}

/// Like [`process`], for programs that only handle policy calls.
pub fn process_only<H: PolicyHook>(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    if process::<H>(program_id, accounts, data)? {
        Ok(())
    } else {
        Err(ProgramError::InvalidInstructionData)
    }
}

/// The forwarded account at `address` that this program owns, if any.
pub fn owned<'a, 'info>(program_id: &Pubkey, accounts: &'a [AccountInfo<'info>], address: &Pubkey) -> Option<&'a AccountInfo<'info>> {
    accounts.iter().skip(2).find(|a| a.key == address && a.owner == program_id)
}

/// Builders for tests and off-chain simulation.
pub mod testing {
    use super::*;
    use bytemuck::Zeroable;
    use solana_program::instruction::{AccountMeta, Instruction};

    pub fn context(env: Environment, callback: u8, subject: Pubkey, actor: Pubkey) -> PolicyContext {
        let mut c = PolicyContext::zeroed();
        c.version = ABI_VERSION;
        c.environment = env as u8;
        c.callback = callback;
        c.subject = subject.to_bytes();
        c.actor = actor.to_bytes();
        c
    }

    /// The instruction an environment sends: `[authority (signer), subject, extra...]`.
    pub fn call(hook: &Pubkey, environment_program: &Pubkey, mut ctx: PolicyContext, extra: &[AccountMeta], op_data: &[u8]) -> Instruction {
        let env = match ctx.environment {
            0 => Environment::Token,
            1 => Environment::Router,
            2 => Environment::Pool,
            _ => Environment::Account,
        };
        let (authority, bump) = Pubkey::find_program_address(&[CALLBACK_SEED, hook.as_ref()], environment_program);
        ctx.authority_bump = bump;
        let mut data = policy_discriminator(env, ctx.callback).expect("callback").to_vec();
        data.extend_from_slice(bytemuck::bytes_of(&ctx));
        data.extend_from_slice(op_data);
        let mut accounts = vec![AccountMeta::new_readonly(authority, true), AccountMeta::new_readonly(Pubkey::new_from_array(ctx.subject), false)];
        accounts.extend_from_slice(extra);
        Instruction { program_id: *hook, accounts, data }
    }

    /// Decodes the hook's return data.
    pub fn result(return_data: &[u8]) -> Option<PolicyResult> {
        PolicyResult::decode(return_data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::instruction::Instruction;

    struct Deny;
    impl PolicyHook for Deny {
        const ENVIRONMENT: Environment = Environment::Router;
        const ENVIRONMENT_PROGRAM: Pubkey = ROUTER_PROGRAM_ID;
        fn evaluate(_: &Pubkey, call: &Call, _: &[AccountInfo]) -> Result<Decision, ProgramError> {
            Ok(if call.amount() > 10 { Decision::reject(CUSTOM_REASON_BASE) } else { Decision::allow() })
        }
    }

    fn run(ix: &Instruction, subject_owner: Pubkey) -> Result<bool, ProgramError> {
        let mut lamports = vec![0u64; ix.accounts.len()];
        let mut data: Vec<Vec<u8>> = vec![vec![]; ix.accounts.len()];
        let owners: Vec<Pubkey> = (0..ix.accounts.len()).map(|i| if i == 1 { subject_owner } else { Pubkey::default() }).collect();
        let infos: Vec<AccountInfo> = ix
            .accounts
            .iter()
            .zip(lamports.iter_mut())
            .zip(data.iter_mut())
            .zip(owners.iter())
            .map(|(((m, l), d), o)| AccountInfo::new(&m.pubkey, m.is_signer, m.is_writable, l, d, o, false, 0))
            .collect();
        process::<Deny>(&ix.program_id, &infos, &ix.data)
    }

    fn router_call(hook: &Pubkey, amount: u64) -> Instruction {
        let mut ctx = testing::context(Environment::Router, 2, Pubkey::new_unique(), Pubkey::new_unique());
        ctx.amount = amount;
        testing::call(hook, &ROUTER_PROGRAM_ID, ctx, &[], &[])
    }

    #[test]
    fn non_policy_data_is_left_to_the_program() {
        let hook = Pubkey::new_unique();
        let mut ix = router_call(&hook, 1);
        assert_eq!(run(&ix, ROUTER_PROGRAM_ID), Ok(true));
        ix.data = vec![0, 1, 2];
        assert_eq!(run(&ix, ROUTER_PROGRAM_ID), Ok(false));
        // A policy call for another environment is not this hook's.
        let ctx = testing::context(Environment::Account, 0, Pubkey::new_unique(), Pubkey::new_unique());
        let other = testing::call(&hook, &SMART_ACCOUNTS_PROGRAM_ID, ctx, &[], &[]);
        assert_eq!(run(&other, SMART_ACCOUNTS_PROGRAM_ID), Ok(false));
    }

    #[test]
    fn forged_calls_fail_closed() {
        let hook = Pubkey::new_unique();
        let ix = router_call(&hook, 1);
        let mut unsigned = ix.clone();
        unsigned.accounts[0].is_signer = false;
        assert!(run(&unsigned, ROUTER_PROGRAM_ID).is_err());
        let mut wrong_authority = ix.clone();
        wrong_authority.accounts[0].pubkey = Pubkey::new_unique();
        assert!(run(&wrong_authority, ROUTER_PROGRAM_ID).is_err());
        // The subject must be owned by the environment program.
        assert!(run(&ix, Pubkey::new_unique()).is_err());
        // Calls built for another hook do not verify here.
        let mut other_hook = router_call(&Pubkey::new_unique(), 1);
        other_hook.program_id = hook;
        assert!(run(&other_hook, ROUTER_PROGRAM_ID).is_err());
    }

    #[test]
    fn decisions_round_trip_through_return_data_encoding() {
        let d = Decision::reject(CUSTOM_REASON_BASE + 3).with_output(output::NONE + 1, 42);
        let bytes = bytemuck::bytes_of(&d.0);
        let back = testing::result(bytes).unwrap();
        assert_eq!(back, d.0);
        assert!(!d.is_allow());
        assert!(Decision::allow().is_allow());
    }
}
