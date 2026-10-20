//! Settlement engine: pure accounting with hook calls behind a trait, so the delta rules are
//! testable without a VM. The instruction handlers do token movement and invariant checks around
//! it. See `docs/engines/pool-manager.md` §4.

use anchor_lang::prelude::*;
use hooks_interface::{math, perm, Callback, HookResult, SWAP_EXACT_IN, SWAP_ZERO_FOR_ONE};

use crate::errors::CoreError;

/// The numbers the core accounts for, per pool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Book {
    pub reserve: [u64; 2],
    pub credit: [u64; 2],
    pub protocol_fee: [u64; 2],
    pub lp_supply: u64,
    pub fee_ppm: u32,
    pub max_fee_ppm: u32,
    pub permissions: u32,
}

impl Book {
    /// Tokens the vault must hold for each side.
    pub fn owed(&self, i: usize) -> Option<u64> {
        self.reserve[i].checked_add(self.credit[i])?.checked_add(self.protocol_fee[i])
    }
    pub fn permits(&self, bits: u32) -> bool {
        self.permissions & bits == bits
    }
}

pub trait Hooks {
    /// Invokes `cb` on the pool's hook. Only called when the pool's permissions include `cb`.
    fn call(&mut self, cb: Callback, book: &Book, swap_flags: u8, args: [u64; 7]) -> Result<HookResult>;
}

/// For pools without a hook, and for hook-initiated operations (which skip callbacks).
pub struct NoHooks;
impl Hooks for NoHooks {
    fn call(&mut self, _: Callback, _: &Book, _: u8, _: [u64; 7]) -> Result<HookResult> {
        Ok(HookResult::ok())
    }
}

/// Calls `cb` if permitted and enforces what its result may contain.
pub fn invoke(
    hooks: &mut impl Hooks,
    cb: Callback,
    book: &Book,
    swap_flags: u8,
    args: [u64; 7],
) -> Result<Option<HookResult>> {
    if !book.permits(cb.permission()) {
        return Ok(None);
    }
    let r = hooks.call(cb, book, swap_flags, args)?;
    if r.rejected() {
        msg!("hook rejected {}: reason {}", cb.name(), r.reason);
        return err!(CoreError::HookRejected);
    }
    let (d0, d1) = cb.allowed_deltas(book.permissions);
    let fee_ok = match r.fee_override() {
        None => true,
        Some(f) => {
            cb == Callback::BeforeSwap && book.permits(perm::DYNAMIC_FEE) && f <= book.max_fee_ppm
        }
    };
    let ok = (d0 || r.delta_0 == 0)
        && (d1 || r.delta_1 == 0)
        && fee_ok
        && (cb == Callback::Quote || r.quote == 0);
    require!(ok, CoreError::HookResultNotPermitted);
    Ok(Some(r))
}

#[derive(Clone, Copy, Debug)]
pub struct SwapArgs {
    pub zero_for_one: bool,
    pub exact_in: bool,
    /// Exact-in: input actually received by the vault. Exact-out: output the trader receives.
    pub amount: u64,
    /// Exact-in: minimum output. Exact-out: maximum input.
    pub limit: u64,
    pub protocol_fee_bps: u16,
    /// Fee from the pool's builtin policy (`dynamic-fee`), replacing the pool fee for this swap;
    /// clamped to `max_fee_ppm`. The pool hook's own dynamic fee, if any, still takes precedence.
    pub policy_fee_ppm: Option<u32>,
}

impl SwapArgs {
    pub fn flags(&self) -> u8 {
        (if self.zero_for_one { SWAP_ZERO_FOR_ONE } else { 0 })
            | (if self.exact_in { SWAP_EXACT_IN } else { 0 })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SwapResult {
    /// Input the trader pays (exact-in: `amount`).
    pub user_in: u64,
    /// Output the trader receives (exact-out: `amount`).
    pub user_out: u64,
    pub curve_in: u64,
    pub curve_out: u64,
    pub fee: u64,
    pub protocol_fee: u64,
    pub fee_ppm: u32,
    pub delta_specified: i64,
    pub delta_unspecified: i64,
}

fn to_u64(v: i128) -> Result<u64> {
    u64::try_from(v).map_err(|_| error!(CoreError::MathOverflow))
}

/// Runs a swap against `book`. On success `book` holds the new state; on error it is unchanged.
pub fn swap(book: &mut Book, a: &SwapArgs, hooks: &mut impl Hooks) -> Result<SwapResult> {
    require!(a.amount > 0, CoreError::ZeroAmount);
    let (i, o) = if a.zero_for_one { (0, 1) } else { (1, 0) };
    let flags = a.flags();
    let mut b = *book;
    let mut fee_ppm = a.policy_fee_ppm.map_or(b.fee_ppm, |f| f.min(b.max_fee_ppm));
    let mut credit = [b.credit[0] as i128, b.credit[1] as i128];

    let (s, u1) = match invoke(hooks, Callback::BeforeSwap, &b, flags, [a.amount, a.limit, 0, 0, 0, 0, 0])? {
        Some(r) => {
            if let Some(f) = r.fee_override() {
                fee_ppm = f;
            }
            (r.delta_0, r.delta_1)
        }
        None => (0, 0),
    };
    let custom = b.permits(perm::CUSTOM_CURVE);
    let (r_in, r_out) = (b.reserve[i], b.reserve[o]);

    let quote = |hooks: &mut _, b: &Book, x: u64, fee: u64| -> Result<u64> {
        let r = invoke(hooks, Callback::Quote, b, flags, [x, fee, 0, 0, 0, 0, 0])?
            .ok_or(error!(CoreError::HookResultInvalid))?;
        Ok(r.quote)
    };

    let (curve_in, curve_out, fee) = if a.exact_in {
        let curve_in = a.amount as i128 - s as i128;
        require!(curve_in >= 0, CoreError::DeltaExceedsAmount);
        let curve_in = to_u64(curve_in)?;
        if curve_in == 0 {
            (0, 0, 0)
        } else {
            let fee = math::fee_on_input(curve_in, fee_ppm).ok_or(error!(CoreError::MathOverflow))?;
            let net = curve_in - fee;
            let out = if custom {
                quote(hooks, &b, net, fee)?
            } else {
                math::cp_amount_out(net, r_in, r_out).ok_or(error!(CoreError::InsufficientLiquidity))?
            };
            require!(out < r_out, CoreError::QuoteOutOfRange);
            (curve_in, out, fee)
        }
    } else {
        let curve_out = a.amount as i128 + s as i128;
        require!(curve_out >= 0, CoreError::DeltaExceedsAmount);
        let curve_out = to_u64(curve_out)?;
        if curve_out == 0 {
            (0, 0, 0)
        } else {
            require!(curve_out < r_out, CoreError::InsufficientLiquidity);
            let net = if custom {
                let q = quote(hooks, &b, curve_out, 0)?;
                require!(q > 0, CoreError::QuoteOutOfRange);
                q
            } else {
                math::cp_amount_in(curve_out, r_in, r_out).ok_or(error!(CoreError::InsufficientLiquidity))?
            };
            let gross = math::gross_up(net, fee_ppm).ok_or(error!(CoreError::MathOverflow))?;
            (gross, curve_out, gross - net)
        }
    };

    let protocol_fee = math::protocol_share(fee, a.protocol_fee_bps);
    b.reserve[i] = b.reserve[i]
        .checked_add(curve_in - protocol_fee)
        .ok_or(error!(CoreError::MathOverflow))?;
    b.reserve[o] -= curve_out;
    b.protocol_fee[i] = b.protocol_fee[i].checked_add(protocol_fee).ok_or(error!(CoreError::MathOverflow))?;
    // The specified token is the input for exact-in, the output for exact-out.
    let (spec, unspec) = if a.exact_in { (i, o) } else { (o, i) };
    credit[spec] += s as i128;
    credit[unspec] += u1 as i128;

    let u2 = match invoke(
        hooks,
        Callback::AfterSwap,
        &b,
        flags,
        [a.amount, a.limit, curve_in, curve_out, fee, s as u64, u1 as u64],
    )? {
        Some(r) => r.delta_1,
        None => 0,
    };
    credit[unspec] += u2 as i128;
    let unspec_total = u1 as i128 + u2 as i128;

    let (user_in, user_out) = if a.exact_in {
        let out = curve_out as i128 - unspec_total;
        require!(out >= 0 && out >= a.limit as i128, CoreError::Slippage);
        (a.amount, to_u64(out)?)
    } else {
        let pay = curve_in as i128 + unspec_total;
        require!(pay >= 0 && pay <= a.limit as i128, CoreError::Slippage);
        (to_u64(pay)?, a.amount)
    };

    require!(credit[0] >= 0 && credit[1] >= 0, CoreError::InsufficientCredit);
    b.credit = [to_u64(credit[0])?, to_u64(credit[1])?];
    b.fee_ppm = book.fee_ppm;
    *book = b;
    Ok(SwapResult {
        user_in,
        user_out,
        curve_in,
        curve_out,
        fee,
        protocol_fee,
        fee_ppm,
        delta_specified: s,
        delta_unspecified: i64::try_from(unspec_total).map_err(|_| error!(CoreError::MathOverflow))?,
    })
}

/// Applies a liquidity delta returned by an after-liquidity hook. Positive: the hook takes from
/// the LP into its credit. Negative: the hook pays the LP from its credit. Returns the LP's
/// adjusted amount (`base + d` for deposits, `base - d` for withdrawals).
pub fn apply_lp_delta(book: &mut Book, side: usize, base: u64, d: i64, deposit: bool) -> Result<u64> {
    let c = book.credit[side] as i128 + d as i128;
    require!(c >= 0, CoreError::InsufficientCredit);
    let lp = if deposit { base as i128 + d as i128 } else { base as i128 - d as i128 };
    require!(lp >= 0, CoreError::DeltaExceedsAmount);
    book.credit[side] = to_u64(c)?;
    to_u64(lp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hooks_interface::perm::*;

    struct Script(Vec<(Callback, HookResult)>, Vec<(Callback, [u64; 7])>);
    impl Hooks for Script {
        fn call(&mut self, cb: Callback, _: &Book, _: u8, args: [u64; 7]) -> Result<HookResult> {
            self.1.push((cb, args));
            Ok(self.0.iter().find(|(c, _)| *c == cb).map(|(_, r)| *r).unwrap_or(HookResult::ok()))
        }
    }

    fn book(perms: u32) -> Book {
        Book {
            reserve: [1_000_000_000, 2_000_000_000],
            credit: [0, 0],
            protocol_fee: [0, 0],
            lp_supply: 1_414_213_562,
            fee_ppm: 3_000,
            max_fee_ppm: 100_000,
            permissions: perms,
        }
    }

    fn args(exact_in: bool, amount: u64, limit: u64) -> SwapArgs {
        SwapArgs { zero_for_one: true, exact_in, amount, limit, protocol_fee_bps: 0, policy_fee_ppm: None }
    }

    fn total(b: &Book, i: usize) -> u64 {
        b.owed(i).unwrap()
    }

    #[test]
    fn plain_exact_in_matches_constant_product() {
        let mut b = book(0);
        let r = swap(&mut b, &args(true, 1_000_000, 0), &mut NoHooks).unwrap();
        assert_eq!(r.fee, 3_000);
        assert_eq!(r.user_out, math::cp_amount_out(997_000, 1_000_000_000, 2_000_000_000).unwrap());
        assert_eq!(b.reserve, [1_001_000_000, 2_000_000_000 - r.user_out]);
    }

    #[test]
    fn slippage_and_book_unchanged_on_error() {
        let mut b = book(0);
        let before = b;
        assert!(swap(&mut b, &args(true, 1_000_000, 10_000_000), &mut NoHooks).is_err());
        assert_eq!(b, before);
    }

    #[test]
    fn exact_out_costs_at_least_exact_in() {
        let mut b = book(0);
        let r = swap(&mut b, &args(false, 1_000_000, u64::MAX), &mut NoHooks).unwrap();
        assert_eq!(r.user_out, 1_000_000);
        let mut b2 = book(0);
        let back = swap(&mut b2, &args(true, r.user_in, 0), &mut NoHooks).unwrap();
        assert!(back.user_out >= 1_000_000);
    }

    #[test]
    fn noop_takeover_hook_takes_whole_input_and_pays_output() {
        let mut b = book(BEFORE_SWAP | BEFORE_SWAP_RETURNS_DELTA);
        b.credit[1] = 5_000;
        let mut h = Script(vec![(Callback::BeforeSwap, HookResult::ok().with_deltas(1_000, -900))], vec![]);
        let r = swap(&mut b, &args(true, 1_000, 0), &mut h).unwrap();
        assert_eq!((r.curve_in, r.curve_out, r.user_out), (0, 0, 900));
        assert_eq!(b.credit, [1_000, 4_100]);
        assert_eq!(b.reserve, book(0).reserve);
    }

    #[test]
    fn hook_cannot_overdraw_credit_or_flip_swap_type() {
        let p = BEFORE_SWAP | BEFORE_SWAP_RETURNS_DELTA;
        let mut b = book(p);
        let mut pay = Script(vec![(Callback::BeforeSwap, HookResult::ok().with_deltas(0, -1))], vec![]);
        assert!(swap(&mut b, &args(true, 1_000, 0), &mut pay).is_err());
        let mut flip = Script(vec![(Callback::BeforeSwap, HookResult::ok().with_deltas(1_001, 0))], vec![]);
        assert!(swap(&mut b, &args(true, 1_000, 0), &mut flip).is_err());
        let mut flip_out = Script(vec![(Callback::BeforeSwap, HookResult::ok().with_deltas(-1_001, 0))], vec![]);
        b.credit[1] = 10_000;
        assert!(swap(&mut b, &args(false, 1_000, u64::MAX), &mut flip_out).is_err());
    }

    #[test]
    fn unpermitted_result_fields_are_rejected() {
        let mut fee = Script(vec![(Callback::BeforeSwap, HookResult::ok().with_fee(10))], vec![]);
        assert!(swap(&mut book(BEFORE_SWAP), &args(true, 1_000, 0), &mut fee).is_err());
        let mut delta = Script(vec![(Callback::BeforeSwap, HookResult::ok().with_deltas(1, 0))], vec![]);
        assert!(swap(&mut book(BEFORE_SWAP), &args(true, 1_000, 0), &mut delta).is_err());
        let mut after0 = Script(vec![(Callback::AfterSwap, HookResult::ok().with_deltas(1, 0))], vec![]);
        let p = AFTER_SWAP | AFTER_SWAP_RETURNS_DELTA;
        assert!(swap(&mut book(p), &args(true, 1_000, 0), &mut after0).is_err());
        let mut q = Script(vec![(Callback::BeforeSwap, HookResult::ok().with_quote(1))], vec![]);
        assert!(swap(&mut book(BEFORE_SWAP), &args(true, 1_000, 0), &mut q).is_err());
        let mut over = Script(vec![(Callback::BeforeSwap, HookResult::ok().with_fee(100_001))], vec![]);
        assert!(swap(&mut book(BEFORE_SWAP | DYNAMIC_FEE), &args(true, 1_000, 0), &mut over).is_err());
        let mut rej = Script(vec![(Callback::BeforeSwap, HookResult::reject(7))], vec![]);
        assert!(swap(&mut book(BEFORE_SWAP), &args(true, 1_000, 0), &mut rej).is_err());
    }

    #[test]
    fn dynamic_fee_applies_to_this_swap_only() {
        let mut b = book(BEFORE_SWAP | DYNAMIC_FEE);
        let mut h = Script(vec![(Callback::BeforeSwap, HookResult::ok().with_fee(50_000))], vec![]);
        let r = swap(&mut b, &args(true, 1_000_000, 0), &mut h).unwrap();
        assert_eq!((r.fee, r.fee_ppm, b.fee_ppm), (50_000, 50_000, 3_000));
    }

    #[test]
    fn custom_curve_prices_via_quote_and_is_bounded() {
        let mut b = book(CUSTOM_CURVE);
        let mut h = Script(vec![(Callback::Quote, HookResult::ok().with_quote(1_234))], vec![]);
        let r = swap(&mut b, &args(true, 1_000, 0), &mut h).unwrap();
        assert_eq!(r.user_out, 1_234);
        assert_eq!(h.1[0], (Callback::Quote, [997, 3, 0, 0, 0, 0, 0]));
        let mut drain = Script(vec![(Callback::Quote, HookResult::ok().with_quote(2_000_000_000))], vec![]);
        assert!(swap(&mut book(CUSTOM_CURVE), &args(true, 1_000, 0), &mut drain).is_err());
    }

    #[test]
    fn after_swap_fee_and_protocol_share() {
        let p = AFTER_SWAP | AFTER_SWAP_RETURNS_DELTA;
        let mut b = book(p);
        let mut h = Script(vec![(Callback::AfterSwap, HookResult::ok().with_deltas(0, 100))], vec![]);
        let mut a = args(true, 1_000_000, 0);
        a.protocol_fee_bps = 2_000;
        let r = swap(&mut b, &a, &mut h).unwrap();
        assert_eq!(b.credit, [0, 100]);
        assert_eq!(r.protocol_fee, 600);
        assert_eq!(b.protocol_fee, [600, 0]);
        assert_eq!(r.user_out + 100, r.curve_out);
    }

    proptest::proptest! {
        /// Whatever a hook returns, tokens are conserved: vault_in grows by exactly what the
        /// trader pays, vault_out shrinks by exactly what the trader receives, credits ≥ 0.
        #[test]
        fn conservation(
            exact_in: bool, zfo: bool,
            amount in 1u64..10_000_000,
            s in -2_000_000i64..2_000_000, u1 in -2_000_000i64..2_000_000, u2 in -2_000_000i64..2_000_000,
            fee in 0u32..100_000, bps in 0u16..2_500,
            c0 in 0u64..5_000_000, c1 in 0u64..5_000_000,
        ) {
            let p = BEFORE_SWAP | BEFORE_SWAP_RETURNS_DELTA | AFTER_SWAP | AFTER_SWAP_RETURNS_DELTA | DYNAMIC_FEE;
            let mut b = book(p);
            b.credit = [c0, c1];
            let start = b;
            let mut h = Script(vec![
                (Callback::BeforeSwap, HookResult::ok().with_fee(fee).with_deltas(s, u1)),
                (Callback::AfterSwap, HookResult::ok().with_deltas(0, u2)),
            ], vec![]);
            let a = SwapArgs { zero_for_one: zfo, exact_in, amount, limit: if exact_in { 0 } else { u64::MAX }, protocol_fee_bps: bps, policy_fee_ppm: None };
            let (i, o) = if zfo { (0, 1) } else { (1, 0) };
            if let Ok(r) = swap(&mut b, &a, &mut h) {
                proptest::prop_assert_eq!(total(&b, i), total(&start, i) + r.user_in);
                proptest::prop_assert_eq!(total(&b, o) + r.user_out, total(&start, o));
                let k0 = start.reserve[0] as u128 * start.reserve[1] as u128;
                proptest::prop_assert!(b.reserve[0] as u128 * b.reserve[1] as u128 >= k0);
            } else {
                proptest::prop_assert_eq!(b, start);
            }
        }
    }
}
