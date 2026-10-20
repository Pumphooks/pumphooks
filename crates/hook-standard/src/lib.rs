//! PUMP HOOKS ABI v1 (`docs/STANDARD.md`). Ids, bits and layouts are frozen; later versions only
//! append.

pub mod loader;
pub mod state;

use bytemuck::{Pod, Zeroable};
use solana_program::{account_info::AccountInfo, program_error::ProgramError, pubkey::Pubkey};

pub const STANDARD: &str = "pump-hooks/v1";
pub const ABI_VERSION: u8 = 1;
pub const MAX_STACK: usize = 8;
pub const MAX_PROGRAM_HOOKS_PER_CALLBACK: usize = 2;
pub const CONFIG_LEN: usize = 96;
pub const ENTRY_STATE_LEN: usize = 32;
pub const CALLBACK_SEED: &[u8] = b"callback";

// ---------------------------------------------------------------- environments

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Environment {
    Token = 0,
    Router = 1,
    Pool = 2,
    Account = 3,
}

impl Environment {
    pub const ALL: [Environment; 4] = [
        Environment::Token,
        Environment::Router,
        Environment::Pool,
        Environment::Account,
    ];

    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.get(v as usize).copied()
    }
    pub const fn bit(self) -> u8 {
        1 << (self as u8)
    }
    pub const fn name(self) -> &'static str {
        match self {
            Environment::Token => "token",
            Environment::Router => "router",
            Environment::Pool => "pool",
            Environment::Account => "account",
        }
    }
    /// Callback names in id order.
    pub const fn callbacks(self) -> &'static [&'static str] {
        match self {
            Environment::Token => &[
                "before_transfer",
                "after_transfer",
                "before_wrap",
                "before_unwrap",
            ],
            Environment::Router => &[
                "before_execute",
                "after_execute",
                "before_swap",
                "after_swap",
            ],
            Environment::Pool => &[
                "before_initialize",
                "after_initialize",
                "before_add_liquidity",
                "after_add_liquidity",
                "before_remove_liquidity",
                "after_remove_liquidity",
                "before_swap",
                "after_swap",
                "before_donate",
                "after_donate",
                "quote",
            ],
            Environment::Account => &[
                "before_deposit",
                "after_deposit",
                "before_withdraw",
                "after_withdraw",
                "before_execute",
                "after_execute",
                "before_authority_change",
                "after_authority_change",
            ],
        }
    }
    /// Mask of every valid callback bit for this environment.
    pub const fn callback_mask(self) -> u16 {
        ((1u32 << self.callbacks().len()) - 1) as u16
    }
    pub const fn enforcement(self, callback: u8) -> Enforcement {
        match (self, callback) {
            (Environment::Token, 0) => Enforcement::Native,
            (Environment::Token, 1) => Enforcement::Unavailable,
            (Environment::Token, _) => Enforcement::Wrapper,
            (Environment::Router, _) => Enforcement::Router,
            (Environment::Pool, _) | (Environment::Account, _) => Enforcement::Program,
        }
    }
}

/// How (and whether) a callback is enforced. Off-chain observation is deliberately absent.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enforcement {
    /// Invoked by another protocol's runtime (Token-2022 `Execute`); not bypassable for the asset.
    Native = 0,
    /// Invoked by our program, which owns the state; not bypassable for that state.
    Program = 1,
    /// Invoked by our router around a third-party venue; bypassable by calling the venue directly.
    Router = 2,
    /// Invoked by the token wrapper around wrap/unwrap.
    Wrapper = 3,
    /// No enforceable point exists.
    Unavailable = 4,
}

// ---------------------------------------------------------------- capabilities

pub mod cap {
    pub const READ_ACCOUNT_STATE: u32 = 1 << 0;
    pub const READ_TOKEN_BALANCE: u32 = 1 << 1;
    pub const VALIDATE_OPERATION: u32 = 1 << 2;
    pub const VETO_OPERATION: u32 = 1 << 3;
    pub const UPDATE_OWN_STATE: u32 = 1 << 4;
    pub const REQUEST_AUTHORIZED_TRANSFER: u32 = 1 << 5;
    pub const REQUEST_AUTHORIZED_SWAP: u32 = 1 << 6;
    pub const REQUEST_AUTHORIZED_FEE_ALLOCATION: u32 = 1 << 7;
    pub const CUSTOM_PRICING: u32 = 1 << 8;
    pub const PRODUCE_STATE: u32 = 1 << 9;
    pub const CONSUME_STATE: u32 = 1 << 10;
    pub const ALL: u32 = (1 << 11) - 1;

    pub const NAMES: [&str; 11] = [
        "READ_ACCOUNT_STATE",
        "READ_TOKEN_BALANCE",
        "VALIDATE_OPERATION",
        "VETO_OPERATION",
        "UPDATE_OWN_STATE",
        "REQUEST_AUTHORIZED_TRANSFER",
        "REQUEST_AUTHORIZED_SWAP",
        "REQUEST_AUTHORIZED_FEE_ALLOCATION",
        "CUSTOM_PRICING",
        "PRODUCE_STATE",
        "CONSUME_STATE",
    ];
}

// ---------------------------------------------------------------- typed outputs

pub mod output {
    pub const NONE: u16 = 0;
    /// The actor/destination passed an access check (value: 1).
    pub const PERMISSION_GRANTED: u16 = 1;
    /// Remaining spend in the current window after this operation.
    pub const SPEND_REMAINING: u16 = 2;
    /// Timelock satisfied (value: unlock time).
    pub const TIMELOCK_SATISFIED: u16 = 3;
    /// Number of approvals counted.
    pub const APPROVALS: u16 = 4;
    /// Operation count recorded by an accounting hook.
    pub const RECORDED_COUNT: u16 = 5;
    /// Condition evaluated true.
    pub const CONDITION_MET: u16 = 6;
    /// Amount within a size cap (value: the cap).
    pub const WITHIN_SIZE_CAP: u16 = 7;
    /// Hook-defined outputs start here.
    pub const CUSTOM_BASE: u16 = 0x8000;
}

// ---------------------------------------------------------------- hook stack

pub const IMPL_BUILTIN: u8 = 0;
pub const IMPL_PROGRAM: u8 = 1;

/// One installed hook. 216 bytes, frozen.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq, Eq)]
pub struct StackEntry {
    /// Registry `HookVersion` account this entry pins.
    pub hook_version: [u8; 32],
    /// Program id for program hooks; zero for builtins.
    pub program: [u8; 32],
    pub impl_kind: u8,
    pub builtin_id: u8,
    pub mandatory: u8,
    /// Bitmask of earlier entry indices whose outputs this entry consumes.
    pub depends_on: u8,
    /// Callbacks (environment-specific bits) this entry runs on.
    pub callbacks: u16,
    pub _pad: u16,
    /// Capabilities granted to this entry.
    pub granted: u32,
    pub _reserved: u32,
    /// Program hooks: the environment refuses to call the program if its ProgramData slot differs
    /// (0 = not pinned).
    pub pinned_slot: u64,
    pub config: [u8; CONFIG_LEN],
    /// Builtin-owned mutable state (e.g. spent-in-window).
    pub state: [u8; ENTRY_STATE_LEN],
}
const _: () = assert!(core::mem::size_of::<StackEntry>() == 216);

impl StackEntry {
    pub fn is_builtin(&self) -> bool {
        self.impl_kind == IMPL_BUILTIN
    }
    pub fn is_mandatory(&self) -> bool {
        self.mandatory != 0
    }
    pub fn program(&self) -> Pubkey {
        Pubkey::new_from_array(self.program)
    }
    pub fn runs_on(&self, callback: u8) -> bool {
        self.callbacks & (1 << callback) != 0
    }
}

/// A complete stack. 8 + 8·216 = 1736 bytes, frozen.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq, Eq)]
pub struct HookStack {
    pub version: u8,
    pub environment: u8,
    pub len: u8,
    pub _pad: [u8; 5],
    pub entries: [StackEntry; MAX_STACK],
}
pub const STACK_LEN: usize = core::mem::size_of::<HookStack>();
const _: () = assert!(STACK_LEN == 1736);

impl HookStack {
    pub fn empty(env: Environment) -> Self {
        let mut s = Self::zeroed();
        s.version = ABI_VERSION;
        s.environment = env as u8;
        s
    }
    pub fn active(&self) -> &[StackEntry] {
        &self.entries[..(self.len as usize).min(MAX_STACK)]
    }
    pub fn active_mut(&mut self) -> &mut [StackEntry] {
        let n = (self.len as usize).min(MAX_STACK);
        &mut self.entries[..n]
    }
}

// ---------------------------------------------------------------- policy calls

pub const POLICY_CONTEXT_LEN: usize = 208;

/// Context for program hooks in the token-wrapper, router and account environments. 208 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq, Eq)]
pub struct PolicyContext {
    pub version: u8,
    pub environment: u8,
    pub callback: u8,
    /// Canonical bump of the environment's callback-authority PDA for this hook.
    pub authority_bump: u8,
    pub entry_index: u8,
    pub _pad: [u8; 3],
    /// Pool / smart account / wrapper / route market.
    pub subject: [u8; 32],
    /// The signer that initiated the operation.
    pub actor: [u8; 32],
    /// Asset involved (mint), or zero for native SOL.
    pub asset: [u8; 32],
    /// Destination / counterparty / target program.
    pub counterparty: [u8; 32],
    pub amount: u64,
    pub amount_b: u64,
    pub slot: u64,
    pub unix_timestamp: i64,
    pub sequence: u64,
    /// Outputs of earlier entries this entry depends on: (type, value) pairs.
    pub inputs: [Output; 2],
}
const _: () = assert!(core::mem::size_of::<PolicyContext>() == POLICY_CONTEXT_LEN);

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct Output {
    pub kind: u16,
    pub _pad: [u8; 6],
    pub value: u64,
}

pub const POLICY_RESULT_LEN: usize = 40;
pub const DECISION_ALLOW: u8 = 0;
pub const DECISION_REJECT: u8 = 1;

/// Result of a program hook in a policy environment. 40 bytes via `set_return_data`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq, Eq)]
pub struct PolicyResult {
    pub version: u8,
    pub decision: u8,
    pub reason: u16,
    pub output_kind: u16,
    pub _pad: u16,
    pub output_value: u64,
    pub _reserved: [u8; 24],
}
const _: () = assert!(core::mem::size_of::<PolicyResult>() == POLICY_RESULT_LEN);

impl PolicyResult {
    pub const fn allow() -> Self {
        Self {
            version: ABI_VERSION,
            decision: DECISION_ALLOW,
            reason: 0,
            output_kind: 0,
            _pad: 0,
            output_value: 0,
            _reserved: [0; 24],
        }
    }
    pub const fn reject(reason: u16) -> Self {
        let mut r = Self::allow();
        r.decision = DECISION_REJECT;
        r.reason = reason;
        r
    }
    pub const fn with_output(mut self, kind: u16, value: u64) -> Self {
        self.output_kind = kind;
        self.output_value = value;
        self
    }
    pub fn set(&self) {
        solana_program::program::set_return_data(bytemuck::bytes_of(self));
    }
    /// Strict decode: exact length, version, known decision, zero padding.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != POLICY_RESULT_LEN {
            return None;
        }
        let r: PolicyResult = bytemuck::pod_read_unaligned(bytes);
        let ok = r.version == ABI_VERSION
            && r.decision <= DECISION_REJECT
            && r._pad == 0
            && r._reserved == [0; 24];
        ok.then_some(r)
    }
}

/// `sha256("pump-hooks-v1:<environment>:<callback>")[..8]`.
pub fn policy_discriminator(env: Environment, callback: u8) -> Option<[u8; 8]> {
    let name = env.callbacks().get(callback as usize)?;
    let h = solana_program::hash::hashv(&[
        b"pump-hooks-v1:",
        env.name().as_bytes(),
        b":",
        name.as_bytes(),
    ]);
    let mut d = [0u8; 8];
    d.copy_from_slice(&h.to_bytes()[..8]);
    Some(d)
}

/// A decoded policy call, for hook programs.
pub struct PolicyCall<'a> {
    pub ctx: PolicyContext,
    pub op_data: &'a [u8],
}

/// Decodes a policy call for `env`. Returns `None` when `data` is not one.
pub fn decode_policy_call(env: Environment, data: &[u8]) -> Option<PolicyCall<'_>> {
    let body = data.get(8..8 + POLICY_CONTEXT_LEN)?;
    let ctx: PolicyContext = bytemuck::pod_read_unaligned(body);
    if ctx.version != ABI_VERSION || ctx.environment != env as u8 {
        return None;
    }
    if policy_discriminator(env, ctx.callback)? != data[..8] {
        return None;
    }
    Some(PolicyCall {
        ctx,
        op_data: &data[8 + POLICY_CONTEXT_LEN..],
    })
}

/// What every policy hook must check first: account 0 is the environment program's
/// callback-authority PDA for this hook, signed; account 1 is the subject, owned by that program.
pub fn verify_policy_call(
    hook_program: &Pubkey,
    environment_program: &Pubkey,
    accounts: &[AccountInfo],
    ctx: &PolicyContext,
) -> Result<(), ProgramError> {
    let [authority, subject, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !authority.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let expected = Pubkey::create_program_address(
        &[CALLBACK_SEED, hook_program.as_ref(), &[ctx.authority_bump]],
        environment_program,
    )
    .map_err(|_| ProgramError::InvalidArgument)?;
    if *authority.key != expected {
        return Err(ProgramError::InvalidArgument);
    }
    if subject.key.to_bytes() != ctx.subject || subject.owner != environment_program {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn discriminators_are_namespaced_hashes() {
        for env in Environment::ALL {
            for (i, name) in env.callbacks().iter().enumerate() {
                let h = Sha256::digest(format!("pump-hooks-v1:{}:{}", env.name(), name).as_bytes());
                assert_eq!(&h[..8], &policy_discriminator(env, i as u8).unwrap());
            }
            assert!(policy_discriminator(env, env.callbacks().len() as u8).is_none());
        }
        // Pinned vectors shared with packages/hook-types.
        assert_eq!(
            policy_discriminator(Environment::Account, 2).unwrap(),
            [5, 96, 230, 49, 122, 240, 169, 152]
        );
        assert_eq!(
            policy_discriminator(Environment::Router, 2).unwrap(),
            [25, 176, 132, 91, 189, 161, 105, 119]
        );
    }

    #[test]
    fn enforcement_classes_are_honest() {
        assert_eq!(Environment::Token.enforcement(0), Enforcement::Native);
        assert_eq!(Environment::Token.enforcement(1), Enforcement::Unavailable);
        assert_eq!(Environment::Token.enforcement(2), Enforcement::Wrapper);
        assert_eq!(Environment::Router.enforcement(2), Enforcement::Router);
        assert_eq!(Environment::Pool.enforcement(6), Enforcement::Program);
        assert_eq!(Environment::Account.callback_mask(), 0xff);
        assert_eq!(Environment::Pool.callback_mask(), 0x7ff);
    }

    #[test]
    fn result_decode_is_strict() {
        let r = PolicyResult::reject(3).with_output(output::APPROVALS, 2);
        assert_eq!(PolicyResult::decode(bytemuck::bytes_of(&r)), Some(r));
        let mut b = bytemuck::bytes_of(&r).to_vec();
        b[20] = 1;
        assert!(PolicyResult::decode(&b).is_none());
        assert!(PolicyResult::decode(&b[..39]).is_none());
    }

    #[test]
    fn policy_call_roundtrip() {
        let mut ctx = PolicyContext::zeroed();
        ctx.version = 1;
        ctx.environment = Environment::Account as u8;
        ctx.callback = 4;
        let mut data = policy_discriminator(Environment::Account, 4)
            .unwrap()
            .to_vec();
        data.extend_from_slice(bytemuck::bytes_of(&ctx));
        data.extend_from_slice(&[9, 9]);
        let call = decode_policy_call(Environment::Account, &data).unwrap();
        assert_eq!(call.op_data, &[9, 9]);
        assert!(decode_policy_call(Environment::Router, &data).is_none());
        data[0] ^= 1;
        assert!(decode_policy_call(Environment::Account, &data).is_none());
    }
}
