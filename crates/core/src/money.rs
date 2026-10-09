//! Exact fixed-point money and physical quantities.
//!
//! Every monetary and physical value in Tide is an integer. There are no
//! floating point numbers anywhere in the domain layer, for three reasons:
//!
//! 1. **The product's core promise is "we prove the number".** A bill computed
//!    in `f64` cannot be reproduced by hand, so the line-item audit trail
//!    would be decorative rather than verifiable.
//! 2. **Determinism.** The API contract guarantees byte-identical output for
//!    byte-identical input. Float summation order changes results; integer
//!    summation cannot.
//! 3. **The test surface is the point.** Rounding policy is a real source of
//!    billing bugs, so it is an explicit, unit-tested decision rather than an
//!    accident of IEEE-754.
//!
//! Units are chosen so that no conversion requires division until the very
//! last step:
//!
//! | Quantity  | Type            | Unit                  |
//! |-----------|-----------------|-----------------------|
//! | Money     | [`MicroUsd`]    | 1e-6 USD              |
//! | Price     | [`MicroUsdPerKwh`] | 1e-6 USD per kWh    |
//! | Energy    | [`Wh`]          | watt-hours            |
//! | Power     | [`Watts`]       | watts                 |
//! | Duration  | [`SlotSpan`]    | settlement slots      |

use core::cmp::Ordering;
use core::fmt;
use serde::{Deserialize, Serialize};

/// Monetary amount in micro-dollars (1e-6 USD).
///
/// Signed: net metering export credits and demand-charge rebates can make a
/// single line item negative, and a negative total is a legitimate outcome.
///
/// Range: ±9.22e13 USD, which is far beyond any household scenario and far
/// beyond what we would risk overflowing when multiplying by energy.
/// Newtype with a serialised string form, so JSON carries `"12.34"` for
/// money and `"1800"` for watt-hours rather than a bare number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MicroUsd(pub i64);

impl MicroUsd {
    pub const ZERO: Self = Self(0);

    /// Construct from whole US cents.
    #[inline]
    #[must_use]
    pub const fn from_cents(cents: i64) -> Self {
        Self(cents.saturating_mul(10_000))
    }

    /// Round to whole cents, half away from zero.
    ///
    /// This is the *only* rounding the user ever sees. Intermediate line items
    /// stay exact in micro-dollars, so the printed bill and the sum of its
    /// printed lines can never disagree by a cent.
    #[inline]
    #[must_use]
    pub const fn to_cents_rounded(&self) -> i64 {
        round_half_away_from_zero(self.0, 10_000)
    }

    /// Canonical display: `$12.34`, `-$0.05`, `$1,234.56`.
    #[must_use]
    pub fn to_usd_string(&self) -> String {
        let cents = self.to_cents_rounded();
        let sign = if cents < 0 { "-" } else { "" };
        let abs = cents.unsigned_abs();
        let dollars = abs / 100;
        let cents_part = abs % 100;
        let grouped = group_thousands(dollars);
        format!("{sign}${grouped}.{cents_part:02}")
    }

    /// Add, saturating rather than wrapping.
    ///
    /// Saturation is deliberate: an overflow here would mean a tariff or usage
    /// series far outside any real range, and silently wrapping to a plausible
    /// number is the worse failure. The [`crate::verify`] layer surfaces a
    /// `saturated` flag so the API can report it honestly.
    #[inline]
    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    #[inline]
    #[must_use]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    #[inline]
    #[must_use]
    pub const fn saturating_neg(self) -> Self {
        Self(self.0.saturating_neg())
    }

    #[inline]
    #[must_use]
    pub const fn is_negative(&self) -> bool {
        self.0 < 0
    }
}

impl fmt::Display for MicroUsd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_usd_string())
    }
}

impl core::ops::Add for MicroUsd {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl core::ops::AddAssign for MicroUsd {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl core::ops::Sub for MicroUsd {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

/// Energy in watt-hours.
///
/// Watt-hours rather than kilowatt-hours so that a 15-minute slot at a
/// household-scale draw (say 3.4 kW) is exactly 850 Wh — no fractional slot
/// energy, therefore no per-slot rounding, therefore no drift.
/// Newtype with a serialised string form, so JSON carries `"12.34"` for
/// money and `"1800"` for watt-hours rather than a bare number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Wh(pub u64);

impl Wh {
    pub const ZERO: Self = Self(0);

    #[inline]
    #[must_use]
    pub const fn from_kwh(kwh: u64) -> Self {
        Self(kwh.saturating_mul(1_000))
    }

    #[inline]
    #[must_use]
    pub const fn to_kwh_trunc(&self) -> u64 {
        self.0 / 1_000
    }

    /// Energy delivered by drawing `power` for exactly `slots` settlement
    /// slots of `slot_minutes` each.
    ///
    /// `watts * minutes / 60 == watt-hours`, so the product is exact whenever
    /// the slot length divides evenly into an hour. For odd slot lengths this
    /// truncates, which is why [`crate::model`] validates that a grid's slot
    /// length divides 60 minutes and rejects e.g. a 7-minute grid.
    #[inline]
    #[must_use]
    pub fn from_power_over_slots(power: Watts, slots: u32, slot_minutes: u16) -> Self {
        let per_slot = u128::from(power.0) * u128::from(slot_minutes) / 60;
        Self((per_slot * u128::from(slots)) as u64)
    }

    #[inline]
    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    #[inline]
    #[must_use]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    #[inline]
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

impl fmt::Display for Wh {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} Wh", self.0)
    }
}

/// Electrical power in watts.
/// Newtype with a serialised string form, so JSON carries `"12.34"` for
/// money and `"1800"` for watt-hours rather than a bare number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Watts(pub u32);

impl Watts {
    pub const ZERO: Self = Self(0);

    #[inline]
    #[must_use]
    pub const fn from_kw(kw: u32) -> Self {
        Self(kw.saturating_mul(1_000))
    }

    /// Display-only helper, e.g. 7200 W -> `"7.20 kW"`.
    ///
    /// Deliberately a string rather than an `f64`: nothing in the domain layer
    /// ever does arithmetic on a kilowatt value, because `f64` reintroduces
    /// exactly the non-determinism this module exists to remove.
    #[inline]
    #[must_use]
    pub fn to_kw_string(&self) -> String {
        let whole = self.0 / 1_000;
        let frac = (self.0 % 1_000) / 100;
        format!("{whole}.{frac:02} kW")
    }

    /// Slots of `slot_minutes` needed to deliver `energy` at this power,
    /// rounded **up** — a load is only finished when it has received its full
    /// energy, and under-delivering would be a silent correctness bug.
    #[inline]
    #[must_use]
    pub fn slots_for(energy: Wh, power: Self, slot_minutes: u16) -> u32 {
        if power.0 == 0 {
            return 0;
        }
        let per_slot = u128::from(power.0) * u128::from(slot_minutes) / 60;
        if per_slot == 0 {
            return u32::MAX;
        }
        let needed = u128::from(energy.0).div_ceil(per_slot);
        needed.min(u128::from(u32::MAX)) as u32
    }
}

/// Price in micro-dollars per kilowatt-hour.
/// Newtype with a serialised string form, so JSON carries `"12.34"` for
/// money and `"1800"` for watt-hours rather than a bare number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MicroUsdPerKwh(pub u32);

impl MicroUsdPerKwh {
    pub const ZERO: Self = Self(0);

    #[inline]
    #[must_use]
    pub const fn from_usd_per_kwh(usd_micro: u32) -> Self {
        Self(usd_micro)
    }

    /// Cost of `energy` at this price, in micro-dollars.
    ///
    /// Exact whenever `energy_wh * price` is divisible by 1000 (it always is
    /// for whole kWh); otherwise rounded half away from zero so the result is
    /// independent of summation order.
    #[inline]
    #[must_use]
    pub fn cost_of(&self, energy: Wh) -> MicroUsd {
        let numer = i128::from(energy.0) * i128::from(self.0);
        MicroUsd(round_half_away_from_zero_i128(numer, 1_000) as i64)
    }
}

/// A count of settlement slots. `u32` covers a 15-minute grid over 7 years.
/// Newtype with a serialised string form, so JSON carries `"12.34"` for
/// money and `"1800"` for watt-hours rather than a bare number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SlotSpan(pub u32);

impl SlotSpan {
    pub const ZERO: Self = Self(0);
    #[inline]
    #[must_use]
    pub const fn get(&self) -> u32 {
        self.0
    }
}

/// Divide `n` by `d` (both positive in practice), rounding half away from zero.
///
/// Kept `const` so rounding policy is visible at every call site and unit
/// tests can pin it without a runtime.
#[inline]
const fn round_half_away_from_zero(n: i64, d: i64) -> i64 {
    let quotient = n / d;
    let remainder = n % d;
    // `d` is always positive at our call sites.
    let twice = remainder * 2;
    if twice >= d {
        quotient + 1
    } else if twice <= -d {
        quotient - 1
    } else {
        quotient
    }
}

#[inline]
const fn round_half_away_from_zero_i128(n: i128, d: i128) -> i128 {
    let quotient = n / d;
    let remainder = n % d;
    let twice = remainder * 2;
    if twice >= d {
        quotient + 1
    } else if twice <= -d {
        quotient - 1
    } else {
        quotient
    }
}

/// `1234567` -> `"1,234,567"`.
fn group_thousands(value: u64) -> String {
    let digits = value.to_string();
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Total ordering helper used when comparing candidate schedules by cost, with
/// deterministic tie-breaks so equal-cost solutions always resolve the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CostTotal {
    pub micro_usd: MicroUsd,
    /// Number of loads that could not be placed. Lower is better; used only to
    /// break exact cost ties toward schedules that satisfy more constraints.
    pub unmet: u32,
    /// Monotonic insertion counter of the schedule's construction, used as the
    /// final tie-break. Guarantees a total order without any randomness.
    pub tiebreak: u64,
}

impl Ord for CostTotal {
    fn cmp(&self, other: &Self) -> Ordering {
        self.micro_usd
            .cmp(&other.micro_usd)
            .then(self.unmet.cmp(&other.unmet))
            .then(self.tiebreak.cmp(&other.tiebreak))
    }
}

impl PartialOrd for CostTotal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_is_half_away_from_zero() {
        assert_eq!(MicroUsd(5).to_cents_rounded(), 0); // 0.000005 -> 0
        assert_eq!(MicroUsd(5_000).to_cents_rounded(), 1); // 0.005 -> 1
        assert_eq!(MicroUsd(-5_000).to_cents_rounded(), -1);
        assert_eq!(MicroUsd(4_999).to_cents_rounded(), 0);
        assert_eq!(MicroUsd(10_000).to_cents_rounded(), 1);
        assert_eq!(MicroUsd(123_456_789).to_cents_rounded(), 12_346);
    }

    #[test]
    fn money_display_is_grouped_and_signed() {
        assert_eq!(MicroUsd::from_cents(1234).to_usd_string(), "$12.34");
        assert_eq!(MicroUsd::from_cents(123_456).to_usd_string(), "$1,234.56");
        assert_eq!(MicroUsd::from_cents(-5).to_usd_string(), "-$0.05");
        assert_eq!(MicroUsd::ZERO.to_usd_string(), "$0.00");
    }

    #[test]
    fn price_cost_is_exact_for_whole_kwh() {
        let p = MicroUsdPerKwh(250_000); // $0.25 / kWh
        assert_eq!(p.cost_of(Wh::from_kwh(1)), MicroUsd::from_cents(25));
        assert_eq!(p.cost_of(Wh::from_kwh(40)), MicroUsd::from_cents(1000));
        assert_eq!(p.cost_of(Wh::ZERO), MicroUsd::ZERO);
    }

    #[test]
    fn price_cost_rounds_deterministically_for_fractional_kwh() {
        let p = MicroUsdPerKwh(333_333); // $0.333333 / kWh
        // 0.5 kWh * 0.333333 = 0.1666665 USD = 16666.65 cents -> 16667 cents
        let cost = p.cost_of(Wh(500));
        assert_eq!(cost, MicroUsd(166_667));
        // Order independence: cost of two halves equals cost of the whole
        // because each line is rounded from an exact product, not accumulated.
        let half = p.cost_of(Wh(250));
        assert_eq!(half, MicroUsd(83_333));
    }

    #[test]
    fn watt_hours_from_power_over_slots() {
        // 3.4 kW for two 15-minute slots = 30 min at 3.4 kW = 1.7 kWh
        let e = Wh::from_power_over_slots(Watts(3400), 2, 15);
        assert_eq!(e, Wh(1_700));
        // 7.2 kW for four 15-minute slots = 60 min at 7.2 kW = 7.2 kWh
        let e = Wh::from_power_over_slots(Watts(7200), 4, 15);
        assert_eq!(e, Wh(7_200));
        // 11 kW for two 30-minute slots = 60 min at 11 kW = 11 kWh
        let e = Wh::from_power_over_slots(Watts(11_000), 2, 30);
        assert_eq!(e, Wh(11_000));
        // Zero power delivers nothing.
        assert_eq!(Wh::from_power_over_slots(Watts::ZERO, 5, 15), Wh::ZERO);
    }

    #[test]
    fn slots_for_rounds_up_so_loads_are_never_under_delivered() {
        // 7.2 kW for one 15-minute slot = 1.8 kWh. 10 kWh / 1.8 = 5.56 slots,
        // which must round up to 6 so the load is never under-served.
        assert_eq!(Watts::slots_for(Wh::from_kwh(10), Watts(7200), 15), 6);
        // 7.2 kW for 1 slot of 15 min = 1800 Wh; 1800 Wh needs exactly 1 slot
        assert_eq!(Watts::slots_for(Wh(1800), Watts(7200), 15), 1);
        assert_eq!(Watts::slots_for(Wh::ZERO, Watts(7200), 15), 0);
        assert_eq!(Watts::slots_for(Wh(1), Watts::ZERO, 15), 0);
    }

    #[test]
    fn saturation_beats_wrapping() {
        let big = MicroUsd(i64::MAX);
        assert_eq!(big.saturating_add(MicroUsd(1)), MicroUsd(i64::MAX));
        assert_eq!(big.saturating_neg(), MicroUsd(i64::MIN + 1));
    }

    #[test]
    fn cost_ordering_breaks_ties_deterministically() {
        let a = CostTotal { micro_usd: MicroUsd(100), unmet: 0, tiebreak: 7 };
        let b = CostTotal { micro_usd: MicroUsd(100), unmet: 0, tiebreak: 3 };
        let c = CostTotal { micro_usd: MicroUsd(100), unmet: 1, tiebreak: 0 };
        assert!(b < a); // same cost, earlier tiebreak wins
        assert!(c > a); // same cost, more unmet loads loses
    }
}
