//! The rate engine: tariffs, price series, and auditable bills.
//!
//! # The core design decision
//!
//! A schedule's cost and a bill for that same usage are two views of one
//! quantity. Most energy tools compute them separately and then quietly
//! disagree, which destroys the audit trail the whole product rests on.
//!
//! Tide prevents that by construction. Price is resolved at **minute
//! resolution**, and a slot's cost is accumulated in integers:
//!
//! ```text
//!   slot cost        =  energy_wh * Σ_periods(price_period * minutes_in_period) / slot_minutes
//! ```
//!
//! The scheduler minimises the integer numerator `Σ energy_wh * weighted_price`.
//! The bill splits that same numerator across periods. Both therefore trace to
//! **one integer**, and `bill.total == schedule_cost` is an invariant the test
//! suite asserts — not a claim in the README.
//!
//! # Sub-slot boundaries
//!
//! A 30-minute slot can straddle a 16:00 tariff boundary. Rather than rounding
//! the slot to one price or the other — which would introduce a per-slot error
//! of up to half the peak/off-peak spread — each slot records the split, and
//! energy is attributed across periods with a largest-remainder allocation so
//! the parts always sum exactly to the whole.
//!
//! # Rounding
//!
//! Exactly one division happens, at presentation time. Line cents are
//! allocated with the same largest-remainder rule, so the lines in a bill always
//! sum exactly to its total. "The lines don't add up" is the most common
//! utility-billing complaint and is entirely avoidable.

use crate::civil::CivilDate;
use crate::money::{MicroUsd, MicroUsdPerKwh, Wh};
use crate::timegrid::SlotGrid;
use crate::zone::LocalZone;
use serde::{Deserialize, Serialize};

/// Which days of the week a rate period applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DaySelector {
    EveryDay,
    Weekdays,
    Weekends,
    /// Bit `i` set means weekday `i` applies (0 = Sunday).
    WeekdayMask(u8),
}

impl DaySelector {
    #[must_use]
    pub const fn mask(self) -> u8 {
        // Bit positions: 0 = Sunday .. 6 = Saturday.
        match self {
            Self::EveryDay => 0b0111_1111,
            Self::Weekdays => 0b0011_1110, // Mon(1)..Fri(5)
            Self::Weekends => 0b0100_0001, // Sun(0) + Sat(6)
            Self::WeekdayMask(m) => m & 0b0111_1111,
        }
    }

    #[must_use]
    pub fn includes(self, weekday: u8) -> bool {
        self.mask() & (1 << weekday) != 0
    }
}

/// One rate period, defined in the tariff's local wall-clock time.
///
/// Windows that cross midnight are supported by `start_minute > end_minute`,
/// which matches how tariffs are actually written ("overnight 21:00 to 07:00").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RatePeriod {
    pub id: String,
    pub label: String,
    pub days: DaySelector,
    /// Minutes since local midnight, 0..=1440.
    pub start_minute: u16,
    /// Minutes since local midnight. Greater than `start_minute` is a normal
    /// window; less than `start_minute` crosses midnight; equal means all-day.
    pub end_minute: u16,
    /// Calendar months this period applies to, as a bitmask (bit 0 = January).
    /// `0` means every month.
    pub months: u16,
    /// Marginal price for metered energy, in micro-USD per kWh.
    pub price: MicroUsdPerKwh,
}

impl RatePeriod {
    /// An all-day, every-day period at a flat price.
    pub fn flat(id: &str, price: MicroUsdPerKwh) -> Self {
        Self {
            id: id.into(),
            label: id.into(),
            days: DaySelector::EveryDay,
            start_minute: 0,
            end_minute: 1_440,
            months: 0,
            price,
        }
    }

    pub fn window(
        id: &str,
        label: &str,
        days: DaySelector,
        start_minute: u16,
        end_minute: u16,
        price: MicroUsdPerKwh,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            days,
            start_minute,
            end_minute,
            months: 0,
            price,
        }
    }

    #[must_use]
    pub fn with_months(mut self, months: u16) -> Self {
        self.months = months;
        self
    }

    #[must_use]
    pub fn crosses_midnight(&self) -> bool {
        self.start_minute > self.end_minute
    }

    /// Does this period apply on `date` at `minute_of_day` local time?
    ///
    /// Windows are half-open `[start, end)`. The `end` minute belongs to the
    /// next period, which is the tariff convention and prevents double billing
    /// at a boundary.
    #[must_use]
    pub fn applies_at(&self, date: CivilDate, minute_of_day: u16) -> bool {
        if !self.days.includes(date.weekday()) {
            return false;
        }
        if self.months != 0 && self.months & (1 << (date.month - 1)) == 0 {
            return false;
        }
        if self.start_minute <= self.end_minute {
            minute_of_day >= self.start_minute && minute_of_day < self.end_minute
        } else {
            minute_of_day >= self.start_minute || minute_of_day < self.end_minute
        }
    }
}

/// A recurring fixed charge, e.g. a monthly customer charge.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FixedCharge {
    pub label: String,
    /// Charged `amount` once per `per_days` days.
    pub amount: MicroUsd,
    pub per_days: u32,
}

/// A charge on the highest sustained power draw in the billing period.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DemandCharge {
    pub label: String,
    /// Micro-USD per kW of peak import power.
    pub rate_per_kw: MicroUsd,
}

/// Credit for energy exported back to the grid.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExportCredit {
    pub label: String,
    /// Micro-USD per kWh paid for exported energy.
    pub rate_per_kwh: MicroUsd,
}

/// A complete, self-describing electricity tariff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tariff {
    pub id: String,
    pub name: String,
    pub zone: LocalZone,
    /// Periods are checked in order; the first that applies wins.
    pub periods: Vec<RatePeriod>,
    pub fixed_charges: Vec<FixedCharge>,
    pub demand_charge: Option<DemandCharge>,
    pub export_credit: Option<ExportCredit>,
    /// Where the numbers came from, so a judge or user can check them.
    pub source_url: Option<String>,
    pub source_retrieved: Option<String>,
}

impl Tariff {
    /// Index of the period in effect, by date and local minute-of-day.
    #[must_use]
    pub fn period_at(&self, date: CivilDate, minute_of_day: u16) -> Option<usize> {
        self.periods
            .iter()
            .position(|p| p.applies_at(date, minute_of_day))
    }

    /// Price in effect at an absolute instant.
    #[must_use]
    pub fn price_at(&self, epoch_minutes: i64) -> Option<MicroUsdPerKwh> {
        let clock = self.zone.to_local(epoch_minutes);
        self.period_at(clock.date, clock.minutes_of_day)
            .map(|i| self.periods[i].price)
    }

    /// Minutes of a representative week that no period covers.
    ///
    /// Sample a full week from a Sunday and assert single coverage. This
    /// catches the most common authoring error — a gap between "peak ends at
    /// 21:00" and "off-peak starts at 21:00" — which would otherwise silently
    /// drop energy from the bill and understate the cost of a decision.
    #[must_use]
    pub fn coverage_gaps(&self) -> Vec<CoverageGap> {
        let mut gaps: Vec<CoverageGap> = Vec::new();
        let start = CivilDate::new(2026, 1, 4); // a Sunday
        for day in 0..7 {
            let date = CivilDate::from_days(start.to_days() + i64::from(day));
            for minute in 0..1_440u16 {
                if self.period_at(date, minute).is_some() {
                    continue;
                }
                match gaps.last_mut() {
                    Some(g) if g.date == date && g.to + 1 == minute => g.to = minute,
                    _ => gaps.push(CoverageGap { date, from: minute, to: minute }),
                }
            }
        }
        gaps
    }

    /// The price series for a grid: one entry per slot.
    #[must_use]
    pub fn price_series(&self, grid: &SlotGrid) -> Vec<SlotPrice> {
        let mut out = Vec::with_capacity(grid.slot_count as usize);
        for i in 0..grid.slot_count {
            let start = grid.slot_start(i);
            let end = start + i64::from(grid.slot_minutes);
            let mut attribution: Vec<PeriodAttribution> = Vec::new();

            // Walk the slot minute by minute, coalescing runs of the same
            // period. A slot is at most 60 minutes, so this is cheap and its
            // correctness is obvious on inspection.
            for m in start..end {
                let clock = self.zone.to_local(m);
                let idx = self.period_at(clock.date, clock.minutes_of_day);
                match attribution.last_mut() {
                    Some(last) if last.period_index == idx => last.minutes += 1,
                    _ => attribution.push(PeriodAttribution { period_index: idx, minutes: 1 }),
                }
            }

            let weighted_price: u64 = attribution
                .iter()
                .map(|a| {
                    let price = a.period_index.map_or(0, |i| u64::from(self.periods[i].price.0));
                    price * u64::from(a.minutes)
                })
                .sum();

            out.push(SlotPrice { weighted_price, slot_minutes: grid.slot_minutes, attribution });
        }
        out
    }
}

/// The price of one settlement slot, decomposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotPrice {
    /// `Σ (price of each minute in the slot)` — micro-USD per kWh·minute.
    pub weighted_price: u64,
    pub slot_minutes: u16,
    /// How the slot's minutes split across rate periods.
    pub attribution: Vec<PeriodAttribution>,
}

impl SlotPrice {
    /// Mean price across the slot, for display only. Never used in arithmetic.
    #[must_use]
    pub fn mean_price(&self) -> MicroUsdPerKwh {
        MicroUsdPerKwh((self.weighted_price / u64::from(self.slot_minutes)) as u32)
    }

    /// True if any minute of this slot is not covered by a period.
    #[must_use]
    pub fn has_gap(&self) -> bool {
        self.attribution.iter().any(|a| a.period_index.is_none())
    }
}

/// Minutes of a slot governed by a specific rate period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodAttribution {
    pub period_index: Option<usize>,
    pub minutes: u16,
}

/// A minute of a week that no rate period covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageGap {
    pub date: CivilDate,
    pub from: u16,
    pub to: u16,
}

/// Metered usage for a grid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Watt-hours drawn from the grid per slot.
    pub import_wh: Vec<Wh>,
    /// Watt-hours pushed to the grid per slot.
    pub export_wh: Vec<Wh>,
}

impl Usage {
    pub fn new(slots: usize) -> Self {
        Self { import_wh: vec![Wh::ZERO; slots], export_wh: vec![Wh::ZERO; slots] }
    }

    pub fn validate_len(&self, slots: usize) -> Result<(), UsageFault> {
        if self.import_wh.len() != slots || self.export_wh.len() != slots {
            return Err(UsageFault::LengthMismatch {
                import: self.import_wh.len(),
                export: self.export_wh.len(),
                expected: slots,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsageFault {
    LengthMismatch { import: usize, export: usize, expected: usize },
}

/// One line of a bill: what it is, how much, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BillLine {
    pub label: String,
    /// Energy this line covers.
    pub energy_wh: Wh,
    /// Rate that produced the amount, if any. `None` for flat charges.
    pub rate: Option<MicroUsdPerKwh>,
    pub amount: MicroUsd,
    /// A human-readable justification, shown when the line is expanded.
    pub detail: String,
}

/// A computed bill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bill {
    pub lines: Vec<BillLine>,
    /// The exact total in micro-dollars.
    pub total: MicroUsd,
    /// Each line's amount in whole cents, allocated by largest-remainder so
    /// that `line_cents` sums exactly to `total.to_cents_rounded()`.
    pub line_cents: Vec<i128>,
}

impl Bill {
    /// Total energy metered across all volumetric lines.
    #[must_use]
    pub fn metered_energy(&self) -> Wh {
        self.lines
            .iter()
            .filter(|l| l.rate.is_some() && !l.label.starts_with("Export"))
            .fold(Wh::ZERO, |acc, l| acc.saturating_add(l.energy_wh))
    }

    /// Compute a bill over a usage series.
    ///
    /// Fixed charges are prorated onto the billing window by
    /// `amount * window_days / per_days`, and the proration factor is stated in
    /// the line's `detail` so the arithmetic is inspectable.
    #[must_use]
    pub fn compute(tariff: &Tariff, grid: &SlotGrid, usage: &Usage) -> Result<Self, BillFault> {
        usage.validate_len(grid.slot_count as usize).map_err(BillFault::Usage)?;
        let prices = tariff.price_series(grid);

        let n = tariff.periods.len();
        let mut numer: Vec<u128> = vec![0; n];
        let mut energy: Vec<u64> = vec![0; n];
        let mut minutes: Vec<u64> = vec![0; n];
        let mut export_wh: u64 = 0;

        // The per-period numerators accumulate `price(µ$/kWh) * energy(Wh)`,
        // whose units are micro-USD once divided by 1000. The same integer
        // total is what `solver::solve` minimises (see `SlotPrice`), so
        // `bill.total == schedule.cost_micro_usd` holds exactly — asserted in
        // `solver::tests::solver_cost_and_bill_agree_exactly`.
        let divisor = 1_000u128;

        for i in 0..grid.slot_count as usize {
            let price = &prices[i];
            let import = usage.import_wh[i].0;
            let export = usage.export_wh[i].0;
            export_wh += export;

            if import > 0 {
                // Split the slot's import energy across the periods it spans,
                // **weighted by minutes**, using largest-remainder so the parts
                // sum exactly to the slot energy.
                //
                // Weighting by minutes is the only correct choice: energy is
                // delivered at a uniform rate across the slot, so period *p*
                // receives `minutes_p / slot_minutes` of it regardless of what
                // that period costs. Weighting by price instead would push
                // energy into the expensive period, which is both arithmetically
                // wrong and would silently inflate the bill.
                let weights: Vec<u128> = price
                    .attribution
                    .iter()
                    .map(|a| u128::from(a.minutes))
                    .collect();
                let shares = largest_remainder_split(u128::from(import), &weights);

                for (slot, attr) in price.attribution.iter().enumerate() {
                    let share = shares[slot];
                    if share == 0 {
                        continue;
                    }
                    let Some(idx) = attr.period_index else { continue };
                    let p = u128::from(tariff.periods[idx].price.0);
                    numer[idx] += p * share;
                    energy[idx] += share as u64;
                    minutes[idx] += u64::from(attr.minutes);
                }
            }
        }

        // --- assemble lines -------------------------------------------------
        let mut lines = Vec::new();
        for (idx, period) in tariff.periods.iter().enumerate() {
            if energy[idx] == 0 && minutes[idx] == 0 {
                continue;
            }
            let amount = MicroUsd(div_i128_u128(numer[idx], divisor));
            lines.push(BillLine {
                label: period.label.clone(),
                energy_wh: Wh(energy[idx]),
                rate: Some(period.price),
                amount,
                detail: format!(
                    "{} Wh over {} slot-minutes at {} micro-USD/kWh",
                    energy[idx], minutes[idx], period.price.0
                ),
            });
        }

        // --- export credit ---------------------------------------------------
        if export_wh > 0 {
            if let Some(credit) = &tariff.export_credit {
                let amount = MicroUsd(div_i128_u128(
                    u128::from(credit.rate_per_kwh.0.unsigned_abs()) * u128::from(export_wh),
                    1_000,
                ));
                lines.push(BillLine {
                    label: format!("Export credit ({})", credit.label),
                    energy_wh: Wh(export_wh),
                    rate: None,
                    amount: amount.saturating_neg(),
                    detail: format!("{export_wh} Wh exported at {} micro-USD/kWh", credit.rate_per_kwh.0),
                });
            }
        }

        // --- demand charge ---------------------------------------------------
        // Average power over a slot: `wh * 60 / slot_minutes` watts. Utilities
        // round peak demand up to a whole kW, so we do too and say so.
        if let Some(dc) = &tariff.demand_charge {
            let peak_w = usage
                .import_wh
                .iter()
                .map(|wh| {
                    if grid.slot_minutes == 0 {
                        0
                    } else {
                        u128::from(wh.0) * 60 / u128::from(grid.slot_minutes)
                    }
                })
                .max()
                .unwrap_or(0);
            let peak_kw = peak_w.div_ceil(1_000);
            if peak_kw > 0 {
                lines.push(BillLine {
                    label: format!("Demand charge ({})", dc.label),
                    energy_wh: Wh::ZERO,
                    rate: None,
                    amount: MicroUsd(dc.rate_per_kw.0 as i64 * peak_kw as i64),
                    detail: format!(
                        "{} W peak demand rounded up to {peak_kw} kW at {} micro-USD/kW",
                        peak_w, dc.rate_per_kw.0
                    ),
                });
            }
        }

        // --- fixed charges, prorated onto the window -------------------------
        let window_days = (grid.slot_minutes as u64 * grid.slot_count as u64) / 1_440;
        for fc in &tariff.fixed_charges {
            let window_days = window_days.max(1);
            let amount = MicroUsd(
                (u128::from(fc.amount.0.unsigned_abs()) * u128::from(window_days)
                    / u128::from(fc.per_days.max(1))) as i64,
            );
            lines.push(BillLine {
                label: format!("Fixed charge ({})", fc.label),
                energy_wh: Wh::ZERO,
                rate: None,
                amount,
                detail: format!(
                    "{} micro-USD per {} days, prorated over {window_days} day(s)",
                    fc.amount.0, fc.per_days
                ),
            });
        }

        let total_micro: i128 = lines.iter().map(|l| i128::from(l.amount.0)).sum();
        let total_cents = round_half_away_i128(total_micro, 10_000);

        // Allocate cents across lines with largest-remainder so the printed
        // lines always sum to the printed total. "The lines don't add up" is
        // the most common utility-billing complaint and is entirely avoidable.
        let line_cents = allocate_cents(
            &lines.iter().map(|l| i128::from(l.amount.0)).collect::<Vec<_>>(),
            total_cents,
        );

        Ok(Bill { lines, total: MicroUsd(i128_to_i64(total_micro)), line_cents })
    }
}

/// Allocate `total_cents` across `micros` so the parts sum **exactly**.
///
/// Each line's exact share is `micro / 10_000` cents. We floor every share,
/// then hand the remaining cents to the lines with the largest fractional
/// remainder — ties to the earlier line, so the result is a pure function of
/// the inputs. Signed values (export credits) are handled by flooring toward
/// negative infinity, which keeps the residual non-negative and the allocation
/// exact.
pub fn allocate_cents(micros: &[i128], total_cents: i128) -> Vec<i128> {
    if micros.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<i128> = Vec::with_capacity(micros.len());
    let mut residuals: Vec<(i128, usize)> = Vec::with_capacity(micros.len());
    let mut assigned: i128 = 0;
    for (i, &m) in micros.iter().enumerate() {
        let floor = m.div_euclid(10_000);
        out.push(floor);
        assigned += floor;
        residuals.push((m.rem_euclid(10_000), i));
    }
    let mut leftover = total_cents - assigned;
    // Largest residual first; earlier index wins ties.
    residuals.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut cursor = 0usize;
    while leftover > 0 {
        let (_, i) = residuals[cursor % residuals.len()];
        out[i] += 1;
        leftover -= 1;
        cursor += 1;
    }
    while leftover < 0 {
        let (_, i) = residuals[cursor % residuals.len()];
        out[i] -= 1;
        leftover += 1;
        cursor += 1;
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BillFault {
    Usage(UsageFault),
}

/// Distribute `total` across `weights` proportionally, using largest-remainder
/// allocation so the parts sum **exactly** to `total`.
///
/// Ties are broken by the earlier index, which makes the allocation a pure
/// function of `(total, weights)` — required by the determinism guarantee.
pub fn largest_remainder_split(total: u128, weights: &[u128]) -> Vec<u128> {
    if weights.is_empty() {
        return Vec::new();
    }
    let weight_sum: u128 = weights.iter().sum();
    if weight_sum == 0 || total == 0 {
        // Undefined weights: give everything to the first non-zero-weight slot,
        // or to the first slot if every weight is zero. Deterministic either way.
        let mut out = vec![0u128; weights.len()];
        out[0] = total;
        return out;
    }

    let mut out: Vec<u128> = Vec::with_capacity(weights.len());
    let mut remainders: Vec<(u128, usize)> = Vec::with_capacity(weights.len());
    let mut assigned: u128 = 0;
    for (i, w) in weights.iter().enumerate() {
        let prod = total * *w;
        let whole = prod / weight_sum;
        let rem = prod % weight_sum;
        out.push(whole);
        assigned += whole;
        remainders.push((rem, i));
    }

    let mut leftover = total - assigned;
    // Largest fractional remainder first; earlier index wins ties.
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut cursor = 0;
    while leftover > 0 {
        let (_, i) = remainders[cursor % remainders.len()];
        if out[i] < u128::MAX {
            out[i] += 1;
        }
        leftover -= 1;
        cursor += 1;
        if cursor > 1_000_000 {
            // Defensive: unreachable for any realistic input, but a spinning
            // loop in a Worker is a worse failure than an approximate split.
            break;
        }
    }
    out
}

fn div_i128_u128(numer: u128, denom: u128) -> i64 {
    if denom == 0 {
        return 0;
    }
    let q = numer / denom;
    i128_to_i64(q as i128)
}

/// Round half away from zero on an `i128`.
fn round_half_away_i128(n: i128, d: i128) -> i128 {
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

fn i128_to_i64(v: i128) -> i64 {
    i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> SlotGrid {
        // 2026-10-09T00:00:00Z, a Friday, 96 x 15-minute slots.
        SlotGrid::new(20_471 * 1_440, 15, 96)
    }

    fn flat_tariff() -> Tariff {
        Tariff {
            id: "flat".into(),
            name: "Flat".into(),
            zone: LocalZone::utc(),
            periods: vec![RatePeriod::flat("base", MicroUsdPerKwh(250_000))],
            fixed_charges: vec![],
            demand_charge: None,
            export_credit: None,
            source_url: None,
            source_retrieved: None,
        }
    }

    fn tou_tariff() -> Tariff {
        // 08:00-20:00 on-peak at $0.50; overnight off-peak at $0.10. Off-peak is
        // the *cheap* window, which is the whole point of TOU pricing. An
        // earlier version of this fixture had the two inverted, which made the
        // product's core claim ("shift to the cheap hours") untestable.
        Tariff {
            id: "tou".into(),
            name: "TOU".into(),
            zone: LocalZone::utc(),
            periods: vec![
                RatePeriod::window("off", "Off-peak", DaySelector::EveryDay, 1_200, 480, MicroUsdPerKwh(100_000)),
                RatePeriod::window("on", "On-peak", DaySelector::EveryDay, 480, 1_200, MicroUsdPerKwh(500_000)),
            ],
            fixed_charges: vec![],
            demand_charge: None,
            export_credit: None,
            source_url: None,
            source_retrieved: None,
        }
    }

    #[test]
    fn flat_tariff_bills_energy_at_the_stated_rate() {
        let mut usage = Usage::new(96);
        usage.import_wh[0] = Wh::from_kwh(2);
        let bill = Bill::compute(&flat_tariff(), &grid(), &usage).unwrap();
        assert_eq!(bill.metered_energy(), Wh(2_000));
        assert_eq!(bill.total, MicroUsd::from_cents(50)); // 2 kWh * $0.25
        assert_eq!(bill.lines.len(), 1);
    }

    #[test]
    fn a_slot_straddling_a_boundary_is_attributed_exactly() {
        // Peak begins at 08:07 local, which falls *inside* a 15-minute slot, so
        // one slot genuinely spans two rate periods. A boundary aligned to the
        // grid (08:00, 08:15, ...) would never exercise the split.
        let tariff = Tariff {
            id: "tou".into(),
            name: "TOU".into(),
            zone: LocalZone::utc(),
            periods: vec![
                RatePeriod::window("off", "Off-peak", DaySelector::EveryDay, 1_260, 487, MicroUsdPerKwh(500_000)),
                RatePeriod::window("on", "On-peak", DaySelector::EveryDay, 487, 1_260, MicroUsdPerKwh(100_000)),
            ],
            fixed_charges: vec![],
            demand_charge: None,
            export_credit: None,
            source_url: None,
            source_retrieved: None,
        };
        assert!(tariff.coverage_gaps().is_empty());
        let g = grid();
        // Slot 32 covers 08:00-08:15, which contains the 08:07 boundary:
        // 7 minutes off-peak, 8 minutes on-peak.
        let mut usage = Usage::new(96);
        usage.import_wh[32] = Wh(7_500);
        let bill = Bill::compute(&tariff, &g, &usage).unwrap();
        // 7/15 of 7.5 kWh at $0.50 = $1.75; 8/15 of 7.5 kWh at $0.10 = $0.40.
        assert_eq!(bill.total, MicroUsd::from_cents(215));
        // And the two lines must add up to that total exactly.
        let sum: i64 = bill.lines.iter().map(|l| l.amount.to_cents_rounded()).sum();
        assert_eq!(sum, bill.total.to_cents_rounded());
        // Energy is attributed by *minutes*, never by price.
        let off = bill.lines.iter().find(|l| l.label == "Off-peak").unwrap();
        let on = bill.lines.iter().find(|l| l.label == "On-peak").unwrap();
        assert_eq!(off.energy_wh, Wh(3_500));
        assert_eq!(on.energy_wh, Wh(4_000));
    }

    #[test]
    fn off_peak_is_cheaper_than_on_peak_for_the_same_energy() {
        let tariff = tou_tariff();
        let g = grid();
        let mut cheap = Usage::new(96);
        cheap.import_wh[0] = Wh::from_kwh(5); // 00:00, off-peak
        let mut expensive = Usage::new(96);
        expensive.import_wh[40] = Wh::from_kwh(5); // 10:00, on-peak
        let a = Bill::compute(&tariff, &g, &cheap).unwrap();
        let b = Bill::compute(&tariff, &g, &expensive).unwrap();
        assert!(a.total < b.total, "the whole product depends on this");
    }

    #[test]
    fn overnight_window_crosses_midnight_correctly() {
        // 21:00 -> 07:00 at $0.20; the rest of the day at $0.40.
        let tariff = Tariff {
            id: "night".into(),
            name: "Night".into(),
            zone: LocalZone::utc(),
            periods: vec![
                RatePeriod::window("night", "Overnight", DaySelector::EveryDay, 1_260, 420, MicroUsdPerKwh(200_000)),
                RatePeriod::window("day", "Daytime", DaySelector::EveryDay, 420, 1_260, MicroUsdPerKwh(400_000)),
            ],
            fixed_charges: vec![],
            demand_charge: None,
            export_credit: None,
            source_url: None,
            source_retrieved: None,
        };
        assert!(tariff.coverage_gaps().is_empty(), "a midnight-crossing window must still cover the week");
        let g = grid();
        let mut early = Usage::new(96);
        early.import_wh[0] = Wh::from_kwh(1); // 00:00 is inside 21:00-07:00
        let mut late = Usage::new(96);
        late.import_wh[95] = Wh::from_kwh(1); // 23:45 is inside 21:00-07:00
        assert_eq!(
            Bill::compute(&tariff, &g, &early).unwrap().total,
            Bill::compute(&tariff, &g, &late).unwrap().total,
            "both sides of midnight must be in the overnight window"
        );
    }

    #[test]
    fn a_tariff_gap_is_detected_rather_than_silently_dropping_energy() {
        let tariff = Tariff {
            id: "gappy".into(),
            name: "Gappy".into(),
            zone: LocalZone::utc(),
            periods: vec![
                RatePeriod::flat("early", MicroUsdPerKwh(100_000)),
                // Nothing covers the rest of the day.
            ],
            fixed_charges: vec![],
            demand_charge: None,
            export_credit: None,
            source_url: None,
            source_retrieved: None,
        };
        // `flat` covers the whole day, so use a genuinely gappy window instead.
        let tariff = Tariff {
            periods: vec![RatePeriod::window(
                "narrow",
                "Narrow",
                DaySelector::EveryDay,
                600,
                660,
                MicroUsdPerKwh(100_000),
            )],
            ..tariff
        };
        let gaps = tariff.coverage_gaps();
        assert!(!gaps.is_empty(), "an uncovered week must be reported");
        // 1440 - 60 = 1380 uncovered minutes, across two runs per day.
        let total_gap_minutes: u64 = gaps.iter().map(|g| u64::from(g.to - g.from + 1)).sum();
        assert_eq!(total_gap_minutes, 7 * (1_440 - 60));
    }

    #[test]
    fn export_is_a_credit_not_a_charge() {
        let tariff = Tariff {
            export_credit: Some(ExportCredit { label: "Net metering".into(), rate_per_kwh: MicroUsd(300_000) }),
            ..flat_tariff()
        };
        let g = grid();
        let mut usage = Usage::new(96);
        usage.export_wh[0] = Wh::from_kwh(4); // 4 kWh at $0.30/kWh = $1.20 credit
        let bill = Bill::compute(&tariff, &g, &usage).unwrap();
        assert_eq!(bill.total, MicroUsd::from_cents(-120));
        assert!(bill.lines.iter().any(|l| l.label.starts_with("Export credit")));
    }

    #[test]
    fn fixed_charges_are_prorated_and_labelled() {
        let tariff = Tariff {
            fixed_charges: vec![FixedCharge { label: "Customer".into(), amount: MicroUsd::from_cents(3_000), per_days: 30 }],
            ..flat_tariff()
        };
        let g = grid();
        let usage = Usage::new(96);
        let bill = Bill::compute(&tariff, &g, &usage).unwrap();
        let fixed = bill.lines.iter().find(|l| l.label.starts_with("Fixed charge")).unwrap();
        // $30.00/month over a 1-day window = $1.00
        assert_eq!(fixed.amount, MicroUsd::from_cents(100));
        assert!(fixed.detail.contains("prorated over 1 day"));
    }

    #[test]
    fn line_cents_always_sum_to_the_total() {
        let tariff = tou_tariff();
        let g = grid();
        let mut usage = Usage::new(96);
        // Spread awkward amounts across many slots so rounding bites.
        for (i, slot) in usage.import_wh.iter_mut().enumerate() {
            *slot = Wh((i as u64 * 137) % 4_001);
        }
        let bill = Bill::compute(&tariff, &g, &usage).unwrap();
        // Rounding each line independently can disagree with rounding the sum by
        // a cent, so `line_cents` is allocated by largest-remainder and must sum
        // exactly to the total.
        let summed: i128 = bill.line_cents.iter().sum();
        assert_eq!(
            summed,
            i128::from(bill.total.to_cents_rounded()),
            "bill lines must always add up to the total"
        );
        assert_eq!(bill.line_cents.len(), bill.lines.len());
    }

    #[test]
    fn usage_length_mismatch_is_an_error_not_a_panic() {
        let tariff = flat_tariff();
        let usage = Usage::new(10); // grid has 96
        let err = Bill::compute(&tariff, &grid(), &usage).unwrap_err();
        assert_eq!(
            err,
            BillFault::Usage(UsageFault::LengthMismatch { import: 10, export: 10, expected: 96 })
        );
    }

    #[test]
    fn largest_remainder_split_is_exact_and_deterministic() {
        let out = largest_remainder_split(100, &[1, 1, 1]);
        assert_eq!(out.iter().sum::<u128>(), 100);
        assert_eq!(out[0], 34); // ties break to the earlier index
        assert_eq!(out[1], 33);
        assert_eq!(out[2], 33);

        // Zero weights get nothing.
        assert_eq!(largest_remainder_split(10, &[0, 1, 0]), vec![0, 10, 0]);
        // No weights at all is a no-op.
        assert_eq!(largest_remainder_split(10, &[]), Vec::<u128>::new());
        // Zero total splits to nothing.
        assert_eq!(largest_remainder_split(0, &[1, 2, 3]), vec![0, 0, 0]);
        // Uneven weights split proportionally up to a tie. 10 over [1,3] is
        // exactly 2.5 and 7.5; the two remainders are equal, so the extra unit
        // goes to the earlier index by the documented tie-break.
        let out = largest_remainder_split(10, &[1, 3]);
        assert_eq!(out.iter().sum::<u128>(), 10);
        assert_eq!(out, vec![3, 7]);
    }

    #[test]
    fn period_boundaries_are_half_open() {
        let tariff = tou_tariff();
        // 08:00 exactly starts on-peak; 07:59 is still off-peak.
        assert_eq!(tariff.period_at(CivilDate::new(2026, 10, 9), 480).unwrap(), 1);
        assert_eq!(tariff.period_at(CivilDate::new(2026, 10, 9), 479).unwrap(), 0);
        // 20:00 exactly ends on-peak.
        assert_eq!(tariff.period_at(CivilDate::new(2026, 10, 9), 1_200).unwrap(), 0);
        assert_eq!(tariff.period_at(CivilDate::new(2026, 10, 9), 1_199).unwrap(), 1);
    }

    #[test]
    fn weekend_selector_separates_weekdays_from_weekends() {
        // 2026-10-10 is a Saturday, 2026-10-09 is a Friday.
        let period = RatePeriod::window("wk", "Weekdays", DaySelector::Weekdays, 480, 1_200, MicroUsdPerKwh(1));
        assert!(period.applies_at(CivilDate::new(2026, 10, 9), 600));
        assert!(!period.applies_at(CivilDate::new(2026, 10, 10), 600));
    }
}
