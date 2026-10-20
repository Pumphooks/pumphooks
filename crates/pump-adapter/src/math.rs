//! Fee-free constant-product estimates. Before-hooks only need a size estimate; after-hooks see the
//! amounts Pump actually executed (observed token deltas), so these never decide settlement.

/// Base out for `quote_in` against `(base, quote)` reserves.
pub fn cp_base_out(base_reserve: u64, quote_reserve: u64, quote_in: u64) -> u64 {
    let (b, q, i) = (base_reserve as u128, quote_reserve as u128, quote_in as u128);
    if q + i == 0 {
        return 0;
    }
    (b * i / (q + i)) as u64
}

/// Quote out for `base_in`.
pub fn cp_quote_out(base_reserve: u64, quote_reserve: u64, base_in: u64) -> u64 {
    cp_base_out(quote_reserve, base_reserve, base_in)
}

/// Quote needed to take `base_out` (rounded up). `None` if `base_out >= base_reserve`.
pub fn cp_quote_in(base_reserve: u64, quote_reserve: u64, base_out: u64) -> Option<u64> {
    if base_out >= base_reserve {
        return None;
    }
    let (b, q, o) = (base_reserve as u128, quote_reserve as u128, base_out as u128);
    let num = q * o;
    let den = b - o;
    u64::try_from(num.div_ceil(den)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp_roundtrip_and_extremes() {
        assert_eq!(cp_base_out(1_000, 1_000, 1_000), 500);
        assert_eq!(cp_quote_out(1_000, 1_000, 1_000), 500);
        assert_eq!(cp_quote_in(1_000, 1_000, 500), Some(1_000));
        assert_eq!(cp_quote_in(1_000, 1_000, 1_000), None);
        assert_eq!(cp_base_out(0, 0, 0), 0);
        assert!(cp_base_out(u64::MAX, u64::MAX, u64::MAX) > 0);
        assert_eq!(cp_quote_in(u64::MAX, u64::MAX, u64::MAX - 1), None, "result exceeds u64");
    }
}
