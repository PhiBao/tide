//! Minimal, dependency-free civil-date arithmetic.
//!
//! Tide needs exactly three calendar operations: convert a day count to
//! `(year, month, day)`, the reverse, and the weekday of a day. Those are
//! implemented here instead of pulling in `chrono` for two reasons:
//!
//! 1. **WASM size.** `chrono` is the single largest dependency a Rust Worker
//!    can carry, and Cloudflare's free tier caps uncompressed Worker size at
//!    64 MiB with a 1-second startup budget.
//! 2. **Auditability.** The daylight-saving rules in [`crate::zone`] are the
//!    part of a tariff most likely to be wrong. Every conversion here has a
//!    unit test against known dates, so the arithmetic the tariff rules rest
//!    on is itself verified rather than assumed.
//!
//! The algorithms are Howard Hinnant's `civil_from_days` / `days_from_civil`,
//! which are exact for the entire proleptic Gregorian range we can represent.

/// A calendar date in the proleptic Gregorian calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilDate {
    pub year: i32,
    /// 1..=12
    pub month: u8,
    /// 1..=31
    pub day: u8,
}

impl CivilDate {
    pub const fn new(year: i32, month: u8, day: u8) -> Self {
        Self { year, month, day }
    }

    /// Days since 1970-01-01 (which is day 0).
    #[must_use]
    pub fn to_days(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }

    #[must_use]
    pub fn from_days(days: i64) -> Self {
        civil_from_days(days)
    }

    /// ISO weekday: 0 = Sunday .. 6 = Saturday.
    ///
    /// 1970-01-01 was a Thursday, so `days + 4` mod 7 gives the weekday with
    /// Sunday as 0.
    #[must_use]
    pub fn weekday(&self) -> u8 {
        (self.to_days().rem_euclid(7) + 4) as u8 % 7
    }

    pub const MONTH_NAMES: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September",
        "October", "November", "December",
    ];

    pub const WEEKDAY_NAMES: [&str; 7] = [
        "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday",
    ];

    #[must_use]
    pub fn iso_string(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl fmt::Display for CivilDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.iso_string())
    }
}

/// Which occurrence of a weekday within a month.
///
/// Used to express statutory DST transitions: US daylight saving begins on the
/// **second** Sunday in March, the EU's begins on the **last** Sunday in March.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WeekOfMonth {
    First,
    Second,
    Third,
    Fourth,
    Last,
}

impl WeekOfMonth {
    /// The day-of-month on which the `weekday`-th `self` of `month`/`year`
    /// falls.
    #[must_use]
    pub fn day_of(&self, year: i32, month: u8, weekday: u8) -> u8 {
        let first_weekday = CivilDate::new(year, month, 1).weekday();
        // Days from the 1st to the first occurrence of `weekday`.
        let offset = (weekday + 7 - first_weekday) % 7;
        let first = 1 + offset;
        let days_in_month = days_in_month(year, month);
        match self {
            Self::First => first,
            Self::Second => first + 7,
            Self::Third => first + 14,
            Self::Fourth => first + 21,
            Self::Last => {
                let mut d = first;
                while d + 7 <= days_in_month {
                    d += 7;
                }
                d
            }
        }
    }
}

#[must_use]
pub const fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        // Defensive: callers validate `month` in 1..=12, but a `const fn`
        // cannot panic, so fall back to the shortest month.
        _ => 28,
    }
}

#[must_use]
pub const fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days since the Unix epoch for a proleptic Gregorian date.
///
/// Hinnant's algorithm: shift the year so March starts the year, which makes
/// the leap-day boundary fall at the end and removes the special case.
#[must_use]
pub const fn days_from_civil(y: i32, m: u8, m_day: u8) -> i64 {
    let mut year = y as i64;
    year -= if m <= 2 { 1 } else { 0 };
    const ERA_MONTHS: i64 = 12;
    // Number of 400-year eras since year 0. The +2 / -306 magic numbers are
    // the standard "March-based" epoch offsets.
    let era = (if year >= 0 { year } else { year - 399 }) / 400;
    let yoe = year - era * 400; // [0, 399]
    let mp = (m as i64 + 9) % ERA_MONTHS; // [0, 11], March = 0
    let doy = (153 * mp + 2) / 5 + (m_day as i64) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`].
///
/// Hinnant's algorithm works in a **March-based year**: the year returned is
/// the year that *began* the previous March. For January and February the
/// result therefore has to be incremented back by one, which is exactly what the
/// final `+ (m <= 2)` does. Omitting it makes 1970-01-01 round-trip to
/// 1969-01-01, and every date in the first two months of the year shifts a year
/// backwards.
#[must_use]
pub const fn civil_from_days(days: i64) -> CivilDate {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8; // [1, 12]
    // The March-based year must be shifted forward for January and February.
    let march_shift: i64 = if month <= 2 { 1 } else { 0 };
    let year = (y + march_shift) as i32;
    CivilDate::new(year, month, day)
}

use core::fmt;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_day_is_1970_01_01() {
        assert_eq!(CivilDate::from_days(0), CivilDate::new(1970, 1, 1));
        assert_eq!(CivilDate::from_days(0).weekday(), 4); // Thursday
    }

    #[test]
    fn round_trips_known_dates() {
        let cases = [
            (1970, 1, 1),
            (2000, 2, 29), // leap century
            (2024, 2, 29),
            (2026, 3, 8),  // US DST start 2026
            (2026, 10, 25), // US DST end 2026
            (1900, 3, 1),  // not a leap year (century, not /400)
            (2100, 12, 31),
            (2026, 1, 1),
            (2026, 12, 31),
        ];
        for (y, m, d) in cases {
            let date = CivilDate::new(y, m, d);
            assert_eq!(
                CivilDate::from_days(date.to_days()),
                date,
                "round trip failed for {date}"
            );
        }
    }

    #[test]
    fn weekdays_are_correct() {
        assert_eq!(CivilDate::new(2026, 10, 9).weekday(), 5); // Friday
        assert_eq!(CivilDate::new(2026, 10, 10).weekday(), 6); // Saturday
        assert_eq!(CivilDate::new(2026, 10, 11).weekday(), 0); // Sunday
        assert_eq!(CivilDate::new(2026, 3, 8).weekday(), 0); // Sunday
    }

    #[test]
    fn leap_years() {
        assert!(is_leap_year(2000));
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(1900));
        assert!(!is_leap_year(2100));
        assert!(!is_leap_year(2026));
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2026, 4), 30);
    }

    #[test]
    fn week_of_month_resolves_statutory_dst_rules() {
        // US: second Sunday in March 2026 -> March 8
        assert_eq!(WeekOfMonth::Second.day_of(2026, 3, 0), 8);
        // US: first Sunday in November 2026 -> November 1
        assert_eq!(WeekOfMonth::First.day_of(2026, 11, 0), 1);
        // EU: last Sunday in March 2026 -> March 29
        assert_eq!(WeekOfMonth::Last.day_of(2026, 3, 0), 29);
        // EU: last Sunday in October 2026 -> October 25
        assert_eq!(WeekOfMonth::Last.day_of(2026, 10, 0), 25);
        // First Saturday in May 2026 -> May 2
        assert_eq!(WeekOfMonth::First.day_of(2026, 5, 6), 2);
        // Last Thursday in November 2026 -> Nov 26 (US Thanksgiving)
        assert_eq!(WeekOfMonth::Last.day_of(2026, 11, 4), 26);
    }
}
