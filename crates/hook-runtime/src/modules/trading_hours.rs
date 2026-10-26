//! Trading Hours: routed trades only inside a daily window on chosen weekdays, in a fixed UTC
//! offset with optional US or EU daylight saving. Windows may cross midnight (open > close);
//! the weekday is then the day the session opened.
//!
//! Applies to routed trades only; a direct Pump trade is not affected.

use hook_types::reason::trading_hours as r;
use hook_types::slot::le;
use hook_types::{HookDecision, ModuleId};

use super::{bad_config, HookEnv, PumpHook};
use crate::executor::HookError;

pub const DST_NONE: u8 = 0;
pub const DST_US: u8 = 1;
pub const DST_EU: u8 = 2;
pub const SIDE_BUY: u8 = 1;
pub const SIDE_SELL: u8 = 2;
const DAY: i64 = 86_400;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// Bit 0 = Monday … bit 6 = Sunday; 0 = every day.
    pub weekdays: u8,
    pub dst: u8,
    /// Standard-time offset from UTC in minutes (-720..=840).
    pub tz_offset_min: i16,
    /// Minutes after local midnight, `0..1440`; `open != close`.
    pub open_min: u16,
    pub close_min: u16,
    /// `SIDE_BUY | SIDE_SELL` the window applies to; 0 = both.
    pub sides: u8,
}

impl Config {
    pub fn decode(b: &[u8; 48]) -> Self {
        Self {
            weekdays: b[0],
            dst: b[1],
            tz_offset_min: le::u16_at(b, 2) as i16,
            open_min: le::u16_at(b, 4),
            close_min: le::u16_at(b, 6),
            sides: b[8],
        }
    }
    pub fn encode(&self) -> [u8; 48] {
        let mut b = [0u8; 48];
        b[0] = self.weekdays;
        b[1] = self.dst;
        le::put_u16(&mut b, 2, self.tz_offset_min as u16);
        le::put_u16(&mut b, 4, self.open_min);
        le::put_u16(&mut b, 6, self.close_min);
        b[8] = self.sides;
        b
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `(year, month)` of a day count.
pub fn civil_from_days(z: i64) -> (i64, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m)
}

/// Monday = 0 … Sunday = 6.
pub fn weekday(days: i64) -> u32 {
    (days + 3).rem_euclid(7) as u32
}

fn nth_sunday(y: i64, m: u32, n: i64) -> i64 {
    let first = days_from_civil(y, m, 1);
    first + (6 - weekday(first) as i64) + 7 * (n - 1)
}

fn last_sunday(y: i64, m: u32) -> i64 {
    let last = if m == 12 { days_from_civil(y + 1, 1, 1) } else { days_from_civil(y, m + 1, 1) } - 1;
    last - ((weekday(last) as i64 + 1) % 7)
}

/// Whether daylight saving is in effect at UTC time `ts` for a zone with standard offset `std`.
pub fn dst_active(rule: u8, ts: i64, std_offset_secs: i64) -> bool {
    match rule {
        // Second Sunday of March 02:00 local standard time to first Sunday of November
        // 02:00 local daylight time (01:00 standard).
        DST_US => {
            let local = ts + std_offset_secs;
            let (y, _) = civil_from_days(local.div_euclid(DAY));
            let start = nth_sunday(y, 3, 2) * DAY + 2 * 3600;
            let end = nth_sunday(y, 11, 1) * DAY + 3600;
            local >= start && local < end
        }
        // Last Sunday of March 01:00 UTC to last Sunday of October 01:00 UTC.
        DST_EU => {
            let (y, _) = civil_from_days(ts.div_euclid(DAY));
            ts >= last_sunday(y, 3) * DAY + 3600 && ts < last_sunday(y, 10) * DAY + 3600
        }
        _ => false,
    }
}

/// Whether the market is open at UTC time `ts`.
pub fn is_open(c: &Config, ts: i64) -> bool {
    let std = c.tz_offset_min as i64 * 60;
    let local = ts + std + if dst_active(c.dst, ts, std) { 3600 } else { 0 };
    let mut day = local.div_euclid(DAY);
    let minute = (local.rem_euclid(DAY) / 60) as u16;
    let (open, close) = (c.open_min, c.close_min);
    let in_window = if open < close {
        minute >= open && minute < close
    } else if minute >= open {
        true
    } else if minute < close {
        day -= 1;
        true
    } else {
        false
    };
    in_window && (c.weekdays == 0 || c.weekdays & (1 << weekday(day)) != 0)
}

pub struct TradingHours;

impl PumpHook for TradingHours {
    const MODULE_ID: u16 = ModuleId::TradingHours as u16;

    fn validate_config(config: &[u8; 48]) -> Result<(), HookError> {
        let c = Config::decode(config);
        if c.weekdays & 0x80 != 0
            || c.dst > DST_EU
            || !(-720..=840).contains(&c.tz_offset_min)
            || c.open_min >= 1440
            || c.close_min >= 1440
            || c.open_min == c.close_min
            || c.sides & !(SIDE_BUY | SIDE_SELL) != 0
            || config[9..].iter().any(|&x| x != 0)
        {
            return Err(bad_config(Self::MODULE_ID));
        }
        Ok(())
    }

    fn on_callback(env: &mut HookEnv<'_>) -> Result<HookDecision, HookError> {
        let c = Config::decode(env.config);
        let ctx = env.ctx;
        let side = if ctx.is_buy() { SIDE_BUY } else { SIDE_SELL };
        if !env.callback.is_before() || (c.sides != 0 && c.sides & side == 0) || is_open(&c, ctx.timestamp) {
            return Ok(HookDecision::allow());
        }
        Ok(HookDecision::deny(r::MARKET_CLOSED))
    }
}
