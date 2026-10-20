//! Hook interface v1 for the programmable-market core.
//!
//! The wire format is frozen and specified in `docs/engines/pool-hook-interface.md`. A hook program:
//!
//! 1. matches the 8-byte discriminator ([`Callback::from_discriminator`]);
//! 2. calls [`verify_core_call`] before anything else;
//! 3. computes a [`HookResult`] and returns it with [`HookResult::set`].
//!
//! Offsets only ever grow in later interface versions.

#![cfg_attr(not(test), no_std)]

pub mod math;

use bytemuck::{Pod, Zeroable};
use solana_program::{
    account_info::AccountInfo, program_error::ProgramError, pubkey, pubkey::Pubkey,
};

/// The core program. Hooks only accept callbacks signed for this id.
pub const CORE_PROGRAM_ID: Pubkey = pubkey!("8L8VFMpzP7S3ZfSyr9vu5vDVy93jgfWVfEzY4RTadFzw");

pub const INTERFACE_VERSION: u8 = 1;
pub const MAX_HOOK_DATA: usize = 512;
pub const PPM: u32 = 1_000_000;
/// Hard cap on any LP fee, including dynamic overrides.
pub const MAX_FEE_PPM: u32 = 500_000;

pub const CALLBACK_SEED: &[u8] = b"callback";
pub const HOOK_AUTHORITY_SEED: &[u8] = b"hook-authority";

/// Hook callbacks. Ids and discriminators are frozen.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Callback {
    BeforeInitialize = 0,
    AfterInitialize = 1,
    BeforeAddLiquidity = 2,
    AfterAddLiquidity = 3,
    BeforeRemoveLiquidity = 4,
    AfterRemoveLiquidity = 5,
    BeforeSwap = 6,
    AfterSwap = 7,
    BeforeDonate = 8,
    AfterDonate = 9,
    Quote = 10,
}

impl Callback {
    pub const ALL: [Callback; 11] = [
        Callback::BeforeInitialize,
        Callback::AfterInitialize,
        Callback::BeforeAddLiquidity,
        Callback::AfterAddLiquidity,
        Callback::BeforeRemoveLiquidity,
        Callback::AfterRemoveLiquidity,
        Callback::BeforeSwap,
        Callback::AfterSwap,
        Callback::BeforeDonate,
        Callback::AfterDonate,
        Callback::Quote,
    ];

    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.get(v as usize).copied()
    }

    /// The permission bit that enables this callback.
    pub const fn permission(self) -> u32 {
        1 << (self as u8)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Callback::BeforeInitialize => "before_initialize",
            Callback::AfterInitialize => "after_initialize",
            Callback::BeforeAddLiquidity => "before_add_liquidity",
            Callback::AfterAddLiquidity => "after_add_liquidity",
            Callback::BeforeRemoveLiquidity => "before_remove_liquidity",
            Callback::AfterRemoveLiquidity => "after_remove_liquidity",
            Callback::BeforeSwap => "before_swap",
            Callback::AfterSwap => "after_swap",
            Callback::BeforeDonate => "before_donate",
            Callback::AfterDonate => "after_donate",
            Callback::Quote => "quote",
        }
    }

    /// `sha256("market-hooks-v1:" + name)[..8]`, pinned by tests.
    pub const fn discriminator(self) -> [u8; 8] {
        match self {
            Callback::BeforeInitialize => [75, 31, 209, 185, 197, 220, 6, 239],
            Callback::AfterInitialize => [108, 148, 246, 230, 172, 86, 6, 98],
            Callback::BeforeAddLiquidity => [205, 244, 8, 25, 116, 183, 111, 56],
            Callback::AfterAddLiquidity => [249, 228, 8, 219, 181, 201, 237, 37],
            Callback::BeforeRemoveLiquidity => [198, 224, 83, 44, 4, 244, 59, 145],
            Callback::AfterRemoveLiquidity => [150, 139, 35, 167, 108, 8, 24, 194],
            Callback::BeforeSwap => [159, 201, 158, 220, 121, 11, 5, 235],
            Callback::AfterSwap => [135, 16, 39, 252, 78, 73, 15, 17],
            Callback::BeforeDonate => [25, 65, 136, 50, 183, 177, 235, 2],
            Callback::AfterDonate => [50, 128, 204, 214, 31, 49, 207, 165],
            Callback::Quote => [212, 245, 177, 194, 53, 255, 96, 178],
        }
    }

    pub fn from_discriminator(d: &[u8]) -> Option<Self> {
        let d: [u8; 8] = d.get(..8)?.try_into().ok()?;
        Self::ALL.iter().copied().find(|c| c.discriminator() == d)
    }

    /// Which result deltas this callback may return, given the pool's permissions.
    pub fn allowed_deltas(self, permissions: u32) -> (bool, bool) {
        let has = |bit: u32| permissions & bit != 0;
        match self {
            Callback::BeforeSwap if has(perm::BEFORE_SWAP_RETURNS_DELTA) => (true, true),
            Callback::AfterSwap if has(perm::AFTER_SWAP_RETURNS_DELTA) => (false, true),
            Callback::AfterAddLiquidity if has(perm::AFTER_ADD_LIQUIDITY_RETURNS_DELTA) => (true, true),
            Callback::AfterRemoveLiquidity if has(perm::AFTER_REMOVE_LIQUIDITY_RETURNS_DELTA) => {
                (true, true)
            }
            _ => (false, false),
        }
    }
}

/// Permission bits (`docs/engines/pool-hook-interface.md` §1).
pub mod perm {
    use super::Callback;

    pub const BEFORE_INITIALIZE: u32 = Callback::BeforeInitialize.permission();
    pub const AFTER_INITIALIZE: u32 = Callback::AfterInitialize.permission();
    pub const BEFORE_ADD_LIQUIDITY: u32 = Callback::BeforeAddLiquidity.permission();
    pub const AFTER_ADD_LIQUIDITY: u32 = Callback::AfterAddLiquidity.permission();
    pub const BEFORE_REMOVE_LIQUIDITY: u32 = Callback::BeforeRemoveLiquidity.permission();
    pub const AFTER_REMOVE_LIQUIDITY: u32 = Callback::AfterRemoveLiquidity.permission();
    pub const BEFORE_SWAP: u32 = Callback::BeforeSwap.permission();
    pub const AFTER_SWAP: u32 = Callback::AfterSwap.permission();
    pub const BEFORE_DONATE: u32 = Callback::BeforeDonate.permission();
    pub const AFTER_DONATE: u32 = Callback::AfterDonate.permission();
    pub const CUSTOM_CURVE: u32 = Callback::Quote.permission();

    pub const BEFORE_SWAP_RETURNS_DELTA: u32 = 1 << 16;
    pub const AFTER_SWAP_RETURNS_DELTA: u32 = 1 << 17;
    pub const AFTER_ADD_LIQUIDITY_RETURNS_DELTA: u32 = 1 << 18;
    pub const AFTER_REMOVE_LIQUIDITY_RETURNS_DELTA: u32 = 1 << 19;
    pub const DYNAMIC_FEE: u32 = 1 << 20;

    pub const ALL: u32 = 0x7ff | 0x1f << 16;

    /// Structural validity: no reserved bits, and every returns-delta power has its base callback.
    pub const fn is_valid(p: u32) -> bool {
        if p & !ALL != 0 {
            return false;
        }
        let needs = [
            (BEFORE_SWAP_RETURNS_DELTA, BEFORE_SWAP),
            (AFTER_SWAP_RETURNS_DELTA, AFTER_SWAP),
            (AFTER_ADD_LIQUIDITY_RETURNS_DELTA, AFTER_ADD_LIQUIDITY),
            (AFTER_REMOVE_LIQUIDITY_RETURNS_DELTA, AFTER_REMOVE_LIQUIDITY),
        ];
        let mut i = 0;
        while i < needs.len() {
            if p & needs[i].0 != 0 && p & needs[i].1 == 0 {
                return false;
            }
            i += 1;
        }
        true
    }
}

pub const SWAP_ZERO_FOR_ONE: u8 = 1 << 0;
pub const SWAP_EXACT_IN: u8 = 1 << 1;

/// Context passed to every callback, after the discriminator. 248 bytes, frozen.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq, Eq)]
pub struct HookContext {
    pub version: u8,
    pub callback: u8,
    pub swap_flags: u8,
    /// Canonical bump of the callback-authority PDA, so hooks verify with one
    /// `create_program_address` instead of a variable-cost `find_program_address`.
    pub authority_bump: u8,
    pub permissions: u32,
    pub pool: [u8; 32],
    pub caller: [u8; 32],
    pub mint_0: [u8; 32],
    pub mint_1: [u8; 32],
    pub reserve_0: u64,
    pub reserve_1: u64,
    pub lp_supply: u64,
    pub sequence: u64,
    pub slot: u64,
    pub unix_timestamp: i64,
    pub fee_ppm: u32,
    pub hook_data_len: u32,
    pub args: [u64; 7],
}

pub const CONTEXT_LEN: usize = core::mem::size_of::<HookContext>();
const _: () = assert!(CONTEXT_LEN == 248);

impl HookContext {
    pub fn callback(&self) -> Option<Callback> {
        Callback::from_u8(self.callback)
    }
    pub fn pool(&self) -> Pubkey {
        Pubkey::new_from_array(self.pool)
    }
    pub fn caller(&self) -> Pubkey {
        Pubkey::new_from_array(self.caller)
    }
    pub fn zero_for_one(&self) -> bool {
        self.swap_flags & SWAP_ZERO_FOR_ONE != 0
    }
    pub fn exact_in(&self) -> bool {
        self.swap_flags & SWAP_EXACT_IN != 0
    }
    /// `args[i]` reinterpreted as a signed delta (AfterSwap carries BeforeSwap's deltas).
    pub fn signed_arg(&self, i: usize) -> i64 {
        self.args[i] as i64
    }
}

pub const RESULT_LEN: usize = 40;
pub const RESULT_FEE_OVERRIDE: u32 = 1 << 0;
pub const DECISION_CONTINUE: u8 = 0;
pub const DECISION_REJECT: u8 = 1;

/// What a hook returns. 40 bytes via `set_return_data`, frozen.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct HookResult {
    pub version: u8,
    pub decision: u8,
    pub reason: u16,
    pub flags: u32,
    pub fee_ppm: u32,
    pub _reserved: u32,
    pub delta_0: i64,
    pub delta_1: i64,
    pub quote: u64,
}
const _: () = assert!(core::mem::size_of::<HookResult>() == RESULT_LEN);

impl HookResult {
    pub const fn ok() -> Self {
        Self {
            version: INTERFACE_VERSION,
            decision: DECISION_CONTINUE,
            reason: 0,
            flags: 0,
            fee_ppm: 0,
            _reserved: 0,
            delta_0: 0,
            delta_1: 0,
            quote: 0,
        }
    }
    pub const fn reject(reason: u16) -> Self {
        let mut r = Self::ok();
        r.decision = DECISION_REJECT;
        r.reason = reason;
        r
    }
    pub const fn with_fee(mut self, fee_ppm: u32) -> Self {
        self.flags |= RESULT_FEE_OVERRIDE;
        self.fee_ppm = fee_ppm;
        self
    }
    pub const fn with_deltas(mut self, delta_0: i64, delta_1: i64) -> Self {
        self.delta_0 = delta_0;
        self.delta_1 = delta_1;
        self
    }
    pub const fn with_quote(mut self, quote: u64) -> Self {
        self.quote = quote;
        self
    }
    pub fn fee_override(&self) -> Option<u32> {
        (self.flags & RESULT_FEE_OVERRIDE != 0).then_some(self.fee_ppm)
    }
    pub fn rejected(&self) -> bool {
        self.decision == DECISION_REJECT
    }

    /// Returns the result to the core.
    pub fn set(&self) {
        solana_program::program::set_return_data(bytemuck::bytes_of(self));
    }

    /// Strict decode used by the core: exact length, version, known decision and flags,
    /// zero reserved field.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != RESULT_LEN {
            return None;
        }
        let r: HookResult = bytemuck::pod_read_unaligned(bytes);
        let ok = r.version == INTERFACE_VERSION
            && r.decision <= DECISION_REJECT
            && r.flags & !RESULT_FEE_OVERRIDE == 0
            && r._reserved == 0;
        ok.then_some(r)
    }
}

/// Instruction data for a callback: discriminator ‖ context ‖ hook_data.
pub fn encode_call(ctx: &HookContext, hook_data: &[u8], out: &mut [u8]) -> Option<usize> {
    let callback = ctx.callback()?;
    let len = 8 + CONTEXT_LEN + hook_data.len();
    if hook_data.len() > MAX_HOOK_DATA || out.len() < len {
        return None;
    }
    out[..8].copy_from_slice(&callback.discriminator());
    out[8..8 + CONTEXT_LEN].copy_from_slice(bytemuck::bytes_of(ctx));
    out[8 + CONTEXT_LEN..len].copy_from_slice(hook_data);
    Some(len)
}

/// A decoded callback instruction.
pub struct Call<'a> {
    pub callback: Callback,
    pub ctx: HookContext,
    pub hook_data: &'a [u8],
}

/// Decodes callback instruction data. Returns `None` for non-callback instructions so a hook can
/// fall through to its own instruction set.
pub fn decode_call(data: &[u8]) -> Option<Call<'_>> {
    let callback = Callback::from_discriminator(data)?;
    let body = data.get(8..8 + CONTEXT_LEN)?;
    let ctx: HookContext = bytemuck::pod_read_unaligned(body);
    let hook_data = &data[8 + CONTEXT_LEN..];
    let consistent = ctx.version == INTERFACE_VERSION
        && ctx.callback == callback as u8
        && ctx.hook_data_len as usize == hook_data.len()
        && hook_data.len() <= MAX_HOOK_DATA;
    consistent.then_some(Call { callback, ctx, hook_data })
}

pub fn callback_authority(hook_program: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[CALLBACK_SEED, hook_program.as_ref()], &CORE_PROGRAM_ID)
}

/// The PDA a hook signs with for hook-initiated core operations (derived under the hook).
pub fn hook_authority(hook_program: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[HOOK_AUTHORITY_SEED], hook_program)
}

/// The check every hook must run first: the call really comes from the core, for this hook, about
/// the pool named in the context. Without it anyone can invoke the hook with a forged context.
pub fn verify_core_call(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    ctx: &HookContext,
) -> Result<(), ProgramError> {
    let [authority, pool, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !authority.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    // Any bump is safe here: only the core can sign for its own PDAs, and it signs with the
    // canonical one.
    let expected = Pubkey::create_program_address(
        &[CALLBACK_SEED, program_id.as_ref(), &[ctx.authority_bump]],
        &CORE_PROGRAM_ID,
    )
    .map_err(|_| ProgramError::InvalidArgument)?;
    if *authority.key != expected {
        return Err(ProgramError::InvalidArgument);
    }
    if pool.key.to_bytes() != ctx.pool || *pool.owner != CORE_PROGRAM_ID {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn discriminators_match_namespace_hash() {
        for c in Callback::ALL {
            let h = Sha256::digest(format!("market-hooks-v1:{}", c.name()).as_bytes());
            assert_eq!(&h[..8], &c.discriminator(), "{}", c.name());
            assert_eq!(Callback::from_discriminator(&c.discriminator()), Some(c));
        }
    }

    #[test]
    fn frozen_layout_offsets() {
        let mut ctx = HookContext::zeroed();
        ctx.permissions = 0x0403_0201;
        ctx.pool = [9; 32];
        ctx.reserve_0 = 0x1122;
        ctx.fee_ppm = 3000;
        ctx.hook_data_len = 7;
        ctx.args[6] = u64::MAX;
        let b = bytemuck::bytes_of(&ctx);
        assert_eq!(&b[4..8], &[1, 2, 3, 4]);
        assert_eq!(b[8], 9);
        assert_eq!(&b[136..138], &[0x22, 0x11]);
        assert_eq!(&b[184..188], &3000u32.to_le_bytes());
        assert_eq!(&b[188..192], &7u32.to_le_bytes());
        assert_eq!(&b[240..248], &[0xff; 8]);

        let r = HookResult::reject(5).with_fee(42).with_deltas(-1, 2).with_quote(3);
        let b = bytemuck::bytes_of(&r);
        assert_eq!(b[0], 1);
        assert_eq!(b[1], 1);
        assert_eq!(&b[2..4], &5u16.to_le_bytes());
        assert_eq!(&b[8..12], &42u32.to_le_bytes());
        assert_eq!(&b[16..24], &(-1i64).to_le_bytes());
        assert_eq!(&b[32..40], &3u64.to_le_bytes());
    }

    #[test]
    fn result_decode_is_strict() {
        let good = HookResult::ok();
        assert_eq!(HookResult::decode(bytemuck::bytes_of(&good)), Some(good));
        assert!(HookResult::decode(&[]).is_none());
        assert!(HookResult::decode(&[0u8; 41]).is_none());
        for (off, val) in [(0usize, 2u8), (1, 2), (4, 2), (12, 1)] {
            let mut b = bytemuck::bytes_of(&good).to_vec();
            b[off] = val;
            assert!(HookResult::decode(&b).is_none(), "offset {off}");
        }
    }

    #[test]
    fn call_roundtrip_and_consistency() {
        let mut ctx = HookContext::zeroed();
        ctx.version = 1;
        ctx.callback = Callback::BeforeSwap as u8;
        ctx.hook_data_len = 3;
        let mut buf = [0u8; 8 + CONTEXT_LEN + MAX_HOOK_DATA];
        let n = encode_call(&ctx, &[1, 2, 3], &mut buf).unwrap();
        let call = decode_call(&buf[..n]).unwrap();
        assert_eq!(call.callback, Callback::BeforeSwap);
        assert_eq!(call.hook_data, &[1, 2, 3]);
        // Truncated hook data no longer matches the declared length.
        assert!(decode_call(&buf[..n - 1]).is_none());
        // A context naming a different callback than the discriminator is rejected.
        buf[8 + 1] = Callback::AfterSwap as u8;
        assert!(decode_call(&buf[..n]).is_none());
    }

    #[test]
    fn permission_rules() {
        use perm::*;
        assert!(is_valid(0));
        assert!(is_valid(BEFORE_SWAP | BEFORE_SWAP_RETURNS_DELTA | DYNAMIC_FEE | CUSTOM_CURVE));
        assert!(!is_valid(BEFORE_SWAP_RETURNS_DELTA));
        assert!(!is_valid(AFTER_REMOVE_LIQUIDITY_RETURNS_DELTA | BEFORE_REMOVE_LIQUIDITY));
        assert!(!is_valid(1 << 11));
        assert!(!is_valid(1 << 21));
        assert_eq!(
            Callback::AfterSwap.allowed_deltas(AFTER_SWAP | AFTER_SWAP_RETURNS_DELTA),
            (false, true)
        );
        assert_eq!(Callback::BeforeSwap.allowed_deltas(BEFORE_SWAP), (false, false));
    }
}
