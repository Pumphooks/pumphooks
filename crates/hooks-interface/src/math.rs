//! Integer math shared by the core, hooks and off-chain simulators. Every function rounds in the
//! pool's favour and returns `None` instead of overflowing or dividing by zero.

use crate::PPM;

/// Shares permanently locked by the first deposit, so the share price can't be inflated from 1.
pub const MIN_LIQUIDITY: u64 = 1_000;

fn to_u64(v: u128) -> Option<u64> {
    u64::try_from(v).ok()
}

fn ceil_div(n: u128, d: u128) -> Option<u128> {
    if d == 0 {
        return None;
    }
    Some(n.div_ceil(d))
}

/// LP fee charged on an exact input, rounded up.
pub fn fee_on_input(amount_in: u64, fee_ppm: u32) -> Option<u64> {
    if fee_ppm > PPM {
        return None;
    }
    to_u64(ceil_div(amount_in as u128 * fee_ppm as u128, PPM as u128)?)
}

/// Gross input whose fee-net equals `net_in`: `ceil(net · 1e6 / (1e6 − fee))`. Always charges at
/// least what [`fee_on_input`] would charge on the result.
pub fn gross_up(net_in: u64, fee_ppm: u32) -> Option<u64> {
    if fee_ppm >= PPM {
        return None;
    }
    to_u64(ceil_div(net_in as u128 * PPM as u128, (PPM - fee_ppm) as u128)?)
}

/// Constant-product output for a fee-net input, rounded down.
pub fn cp_amount_out(net_in: u64, reserve_in: u64, reserve_out: u64) -> Option<u64> {
    let denom = reserve_in as u128 + net_in as u128;
    if denom == 0 {
        return None;
    }
    to_u64(net_in as u128 * reserve_out as u128 / denom)
}

/// Constant-product fee-net input for an exact output, rounded up. Requires `out < reserve_out`.
pub fn cp_amount_in(amount_out: u64, reserve_in: u64, reserve_out: u64) -> Option<u64> {
    if amount_out >= reserve_out {
        return None;
    }
    to_u64(ceil_div(
        amount_out as u128 * reserve_in as u128,
        (reserve_out - amount_out) as u128,
    )?)
}

/// Protocol share of an LP fee, rounded down (LPs keep the remainder).
pub fn protocol_share(fee: u64, share_bps: u16) -> u64 {
    (fee as u128 * share_bps.min(10_000) as u128 / 10_000) as u64
}

pub fn isqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    let mut x = 1u128 << ((128 - n.leading_zeros()).div_ceil(2));
    loop {
        let y = (x + n / x) / 2;
        if y >= x {
            return x;
        }
        x = y;
    }
}

/// Shares for the first deposit: `isqrt(a0·a1)`, of which [`MIN_LIQUIDITY`] is locked.
/// Returns `(total_minted_to_supply, shares_to_depositor)`.
pub fn initial_shares(amount_0: u64, amount_1: u64) -> Option<(u64, u64)> {
    let total = to_u64(isqrt(amount_0 as u128 * amount_1 as u128))?;
    let user = total.checked_sub(MIN_LIQUIDITY).filter(|s| *s > 0)?;
    Some((total, user))
}

/// Shares for a deposit into a live pool: the smaller of the two proportional amounts.
pub fn shares_for_deposit(
    amount_0: u64,
    amount_1: u64,
    reserve_0: u64,
    reserve_1: u64,
    supply: u64,
) -> Option<u64> {
    if reserve_0 == 0 || reserve_1 == 0 || supply == 0 {
        return None;
    }
    let s0 = amount_0 as u128 * supply as u128 / reserve_0 as u128;
    let s1 = amount_1 as u128 * supply as u128 / reserve_1 as u128;
    to_u64(s0.min(s1))
}

/// Exact amounts required for `shares` in a live pool, rounded up.
pub fn amounts_for_shares_up(
    shares: u64,
    reserve_0: u64,
    reserve_1: u64,
    supply: u64,
) -> Option<(u64, u64)> {
    let a0 = ceil_div(shares as u128 * reserve_0 as u128, supply as u128)?;
    let a1 = ceil_div(shares as u128 * reserve_1 as u128, supply as u128)?;
    Some((to_u64(a0)?, to_u64(a1)?))
}

/// Amounts returned for burning `shares`, rounded down.
pub fn amounts_for_shares_down(
    shares: u64,
    reserve_0: u64,
    reserve_1: u64,
    supply: u64,
) -> Option<(u64, u64)> {
    if supply == 0 || shares > supply {
        return None;
    }
    let a0 = shares as u128 * reserve_0 as u128 / supply as u128;
    let a1 = shares as u128 * reserve_1 as u128 / supply as u128;
    Some((to_u64(a0)?, to_u64(a1)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isqrt_exact() {
        for n in [0u128, 1, 2, 3, 4, 15, 16, 17, 1 << 64, u64::MAX as u128 * u64::MAX as u128] {
            let r = isqrt(n);
            assert!(r * r <= n, "{n}");
            assert!((r + 1).checked_mul(r + 1).map_or(true, |sq| sq > n), "{n}");
        }
    }

    #[test]
    fn exact_out_never_cheaper_than_exact_in() {
        let (ri, ro) = (1_000_000_000u64, 2_000_000_000u64);
        for fee in [0u32, 1, 3000, 10_000, 499_999] {
            for out in [1u64, 7, 1_000, 999_999, 500_000_000] {
                let net = cp_amount_in(out, ri, ro).unwrap();
                let gross = gross_up(net, fee).unwrap();
                let back = cp_amount_out(gross - fee_on_input(gross, fee).unwrap(), ri, ro).unwrap();
                assert!(back >= out, "fee {fee} out {out}: paid {gross} gets {back}");
            }
        }
    }

    #[test]
    fn k_never_decreases() {
        let (ri, ro) = (123_456_789u64, 987_654_321u64);
        for x in [1u64, 2, 999, 1_000_000, 50_000_000_000] {
            let out = cp_amount_out(x, ri, ro).unwrap();
            assert!((ri as u128 + x as u128) * (ro - out) as u128 >= ri as u128 * ro as u128);
        }
        assert_eq!(cp_amount_in(ro, ri, ro), None);
    }

    #[test]
    fn liquidity_rounding_favours_pool() {
        let (t, u) = initial_shares(1_000_000, 4_000_000).unwrap();
        assert_eq!((t, u), (2_000_000, 1_999_000));
        assert_eq!(initial_shares(10, 10), None);
        let (r0, r1, s) = (1_000_003u64, 4_000_001u64, 2_000_000u64);
        let sh = shares_for_deposit(1000, 1000, r0, r1, s).unwrap();
        let (a0, a1) = amounts_for_shares_up(sh, r0, r1, s).unwrap();
        let (b0, b1) = amounts_for_shares_down(sh, r0 + a0, r1 + a1, s + sh).unwrap();
        assert!(b0 <= a0 && b1 <= a1);
    }
}
