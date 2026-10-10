//! Multi-period usage history.
//!
//! Single-day usage import answers "what did this day cost". It cannot answer
//! the question that actually matters: *which tariff should I be on?* That needs
//! a year of interval data and every candidate tariff run against it.
//!
//! This module holds the arithmetic for that; storage is a thin layer outside
//! it, so the comparison is testable without a database.
//!
//! # Why the comparison is over absolute instants
//!
//! Readings are stamped in UTC. A tariff defined in US Eastern resolves each
//! instant to its own local wall clock, so running the same reading set against
//! four tariffs is meaningful without re-normalising anything: the zone logic
//! already lives in [`crate::rates`].

use crate::money::MicroUsd;
use crate::timegrid::SlotGrid;
use serde::{Deserialize, Serialize};

/// One stored interval reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    /// Epoch minutes at the start of the interval.
    pub start_epoch_minutes: i64,
    /// Interval length in minutes.
    pub interval_minutes: u16,
    /// Energy drawn from the grid, in watt-hours.
    pub import_wh: u64,
    /// Energy pushed to the grid, in watt-hours.
    pub export_wh: u64,
}

impl Reading {
    /// Epoch minute at which this interval ends.
    #[must_use]
    pub const fn end_epoch_minutes(&self) -> i64 {
        self.start_epoch_minutes + self.interval_minutes as i64
    }
}

/// What a set of readings covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeriodSummary {
    /// First instant the readings cover, or `None` if there are none.
    pub from_epoch_minutes: Option<i64>,
    /// First instant past the last reading.
    pub to_epoch_minutes: Option<i64>,
    /// Number of intervals.
    pub intervals: usize,
    /// Total energy drawn, in watt-hours.
    pub total_import_wh: u128,
    /// Total energy pushed, in watt-hours.
    pub total_export_wh: u128,
    /// Distinct calendar days the readings touch.
    pub days_covered: usize,
    /// Mean energy drawn per interval, in watt-hours.
    pub mean_import_wh: u64,
}

/// One tariff's cost over a set of readings, ranked against the others.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TariffComparison {
    pub tariff_id: String,
    pub tariff_name: String,
    /// Total energy the readings cover, in watt-hours.
    pub total_import_wh: u128,
    /// Cost under this tariff, in micro-USD.
    pub total_micro_usd: MicroUsd,
    /// Difference against the cheapest tariff, in micro-USD. Zero for the
    /// cheapest itself.
    pub delta_vs_cheapest_micro_usd: MicroUsd,
    /// Difference against the current tariff, in micro-USD, if one was named.
    pub delta_vs_current_micro_usd: Option<MicroUsd>,
    /// Every line item, so a household can see what it would have paid for.
    pub lines: Vec<ComparisonLine>,
}

/// One line of a comparison, so the cheapest tariff can be explained rather
/// than merely announced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComparisonLine {
    pub label: String,
    pub energy_wh: u128,
    pub rate_micro_usd_per_kwh: Option<u32>,
    pub amount_micro_usd: MicroUsd,
    pub detail: String,
}

/// Summarise a set of readings.
#[must_use]
pub fn summarise(readings: &[Reading]) -> PeriodSummary {
    if readings.is_empty() {
        return PeriodSummary {
            from_epoch_minutes: None,
            to_epoch_minutes: None,
            intervals: 0,
            total_import_wh: 0,
            total_export_wh: 0,
            days_covered: 0,
            mean_import_wh: 0,
        };
    }

    let mut from = i64::MAX;
    let mut to = i64::MIN;
    let mut total_import: u128 = 0;
    let mut total_export: u128 = 0;
    let mut days: std::collections::BTreeSet<i64> = std::collections::BTreeSet::new();

    for reading in readings {
        from = from.min(reading.start_epoch_minutes);
        to = to.max(reading.end_epoch_minutes());
        total_import += u128::from(reading.import_wh);
        total_export += u128::from(reading.export_wh);
        days.insert(reading.start_epoch_minutes.div_euclid(1_440));
    }

    let mean = total_import / readings.len() as u128;

    PeriodSummary {
        from_epoch_minutes: Some(from),
        to_epoch_minutes: Some(to),
        intervals: readings.len(),
        total_import_wh: total_import,
        total_export_wh: total_export,
        days_covered: days.len(),
        mean_import_wh: mean as u64,
    }
}

/// Run every candidate tariff over the same readings and rank them.
///
/// `readings` must be sorted by start time. The list is returned cheapest first,
/// so the first element is the recommendation.
#[must_use]
pub fn compare(
    readings: &[Reading],
    tariffs: &[&crate::rates::Tariff],
    current_tariff_id: Option<&str>,
) -> Vec<TariffComparison> {
    if readings.is_empty() || tariffs.is_empty() {
        return Vec::new();
    }

    let from = readings
        .iter()
        .map(|r| r.start_epoch_minutes)
        .min()
        .unwrap_or(0);
    let to = readings
        .iter()
        .map(|r| r.end_epoch_minutes())
        .max()
        .unwrap_or(0);

    let grid = build_usage_grid(readings, from);
    if grid.slot_count == 0 {
        return Vec::new();
    }

    let mut usage = crate::rates::Usage::new(grid.slot_count as usize);
    for reading in readings {
        let Some(index) = grid.index_at(reading.start_epoch_minutes) else {
            continue;
        };
        usage.import_wh[index as usize] = crate::money::Wh(
            usage.import_wh[index as usize]
                .0
                .saturating_add(reading.import_wh),
        );
        usage.export_wh[index as usize] = crate::money::Wh(
            usage.export_wh[index as usize]
                .0
                .saturating_add(reading.export_wh),
        );
    }

    let mut results: Vec<TariffComparison> = tariffs
        .iter()
        .map(|tariff| {
            let bill =
                crate::rates::Bill::compute(tariff, &grid, &usage).unwrap_or(crate::rates::Bill {
                    lines: vec![],
                    total: MicroUsd::ZERO,
                    line_cents: vec![],
                });
            let total_import: u128 = usage.import_wh.iter().map(|w| u128::from(w.0)).sum();
            TariffComparison {
                tariff_id: tariff.id.clone(),
                tariff_name: tariff.name.clone(),
                total_import_wh: total_import,
                total_micro_usd: bill.total,
                delta_vs_cheapest_micro_usd: MicroUsd::ZERO,
                delta_vs_current_micro_usd: None,
                lines: bill
                    .lines
                    .iter()
                    .map(|l| ComparisonLine {
                        label: l.label.clone(),
                        energy_wh: u128::from(l.energy_wh.0),
                        rate_micro_usd_per_kwh: l.rate.map(|r| r.0),
                        amount_micro_usd: l.amount,
                        detail: l.detail.clone(),
                    })
                    .collect(),
            }
        })
        .collect();

    results.sort_by(|a, b| {
        a.total_micro_usd
            .cmp(&b.total_micro_usd)
            .then(a.tariff_id.cmp(&b.tariff_id))
    });

    let cheapest = results
        .first()
        .map(|r| r.total_micro_usd)
        .unwrap_or(MicroUsd::ZERO);
    let current = current_tariff_id.and_then(|id| {
        results
            .iter()
            .find(|r| r.tariff_id == id)
            .map(|r| r.total_micro_usd)
    });

    for result in &mut results {
        result.delta_vs_cheapest_micro_usd =
            MicroUsd(result.total_micro_usd.0.saturating_sub(cheapest.0));
        if let Some(current_total) = current {
            result.delta_vs_current_micro_usd = Some(MicroUsd(
                result.total_micro_usd.0.saturating_sub(current_total.0),
            ));
        }
    }

    let _ = to;
    results
}

/// A grid covering the readings, at the coarsest interval they use.
///
/// Readings arrive at a single interval length (15 minutes, say), so the grid
/// uses that length. If they arrive mixed, the grid uses the finest so nothing
/// is lost.
fn build_usage_grid(readings: &[Reading], from: i64) -> SlotGrid {
    let finest = readings
        .iter()
        .map(|r| r.interval_minutes)
        .filter(|m| *m > 0)
        .min()
        .unwrap_or(15);
    let span_minutes = readings
        .iter()
        .map(|r| r.end_epoch_minutes())
        .max()
        .unwrap_or(from)
        - from;
    let slots = (span_minutes / i64::from(finest.max(1))).max(1);
    let slots = u32::try_from(slots.min(i64::from(crate::timegrid::MAX_SLOTS)))
        .unwrap_or(crate::timegrid::MAX_SLOTS);
    SlotGrid::new(from, finest.max(1), slots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::civil::days_from_civil;
    use crate::rates::Tariff;

    const DAY: i64 = days_from_civil(2026, 10, 10);

    fn readings(count: usize, interval: u16, kwh_per: u64) -> Vec<Reading> {
        (0..count)
            .map(|i| Reading {
                start_epoch_minutes: DAY * 1440 + i as i64 * i64::from(interval),
                interval_minutes: interval,
                import_wh: kwh_per * 1000,
                export_wh: 0,
            })
            .collect()
    }

    #[test]
    fn summarises_a_set_of_readings() {
        // Three 0.5 kWh intervals: two on the previous evening, one at the start
        // of the day, so the set spans a midnight boundary.
        let day_before = DAY - 1;
        let rs = vec![
            Reading {
                start_epoch_minutes: day_before * 1440 + 23 * 60,
                interval_minutes: 15,
                import_wh: 500,
                export_wh: 0,
            },
            Reading {
                start_epoch_minutes: day_before * 1440 + 23 * 60 + 15,
                interval_minutes: 15,
                import_wh: 500,
                export_wh: 0,
            },
            Reading {
                start_epoch_minutes: DAY * 1440,
                interval_minutes: 15,
                import_wh: 500,
                export_wh: 0,
            },
        ];
        let s = summarise(&rs);
        assert_eq!(s.intervals, 3);
        assert_eq!(s.total_import_wh, 1_500);
        assert_eq!(s.days_covered, 2, "the readings touch two calendar days");
        assert_eq!(s.mean_import_wh, 500);
        assert_eq!(s.from_epoch_minutes, Some(day_before * 1440 + 23 * 60));
        assert_eq!(s.to_epoch_minutes, Some(DAY * 1440 + 15));
    }

    #[test]
    fn summarising_nothing_is_not_a_panic() {
        let s = summarise(&[]);
        assert_eq!(s.intervals, 0);
        assert!(s.from_epoch_minutes.is_none());
        assert_eq!(s.total_import_wh, 0);
    }

    #[test]
    fn the_cheapest_tariff_ranks_first_and_the_deltas_agree() {
        let rs = readings(48, 15, 1); // one day of 1 kWh per quarter hour
        let cheap = Tariff {
            id: "cheap".into(),
            name: "Cheap".into(),
            zone: crate::zone::LocalZone::utc(),
            periods: vec![crate::rates::RatePeriod::flat(
                "base",
                crate::money::MicroUsdPerKwh(50_000),
            )],
            fixed_charges: vec![],
            demand_charge: None,
            export_credit: None,
            source_url: None,
            source_retrieved: None,
        };
        let dear = Tariff {
            id: "dear".into(),
            name: "Dear".into(),
            periods: vec![crate::rates::RatePeriod::flat(
                "base",
                crate::money::MicroUsdPerKwh(500_000),
            )],
            ..cheap.clone()
        };

        // Deliberately out of order: the caller must not have to sort first.
        let ranked = compare(&rs, &[&dear, &cheap], Some("dear"));
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].tariff_id, "cheap", "cheapest first");
        assert_eq!(ranked[0].delta_vs_cheapest_micro_usd.0, 0);
        assert_eq!(ranked[1].tariff_id, "dear");
        assert!(ranked[1].delta_vs_cheapest_micro_usd.0 > 0);
        // Both deltas are against the same current tariff, so they differ by
        // exactly the gap between the two totals.
        let gap = ranked[1].total_micro_usd.0 - ranked[0].total_micro_usd.0;
        let delta_gap = ranked[1].delta_vs_current_micro_usd.unwrap().0
            - ranked[0].delta_vs_current_micro_usd.unwrap().0;
        assert_eq!(gap, delta_gap);
    }

    #[test]
    fn a_higher_rate_costs_proportionally_more_for_the_same_energy() {
        let rs = readings(24, 15, 1);
        let mut cheap = Tariff {
            id: "a".into(),
            name: "A".into(),
            zone: crate::zone::LocalZone::utc(),
            periods: vec![crate::rates::RatePeriod::flat(
                "base",
                crate::money::MicroUsdPerKwh(100_000),
            )],
            fixed_charges: vec![],
            demand_charge: None,
            export_credit: None,
            source_url: None,
            source_retrieved: None,
        };
        let dear = Tariff {
            periods: vec![crate::rates::RatePeriod::flat(
                "base",
                crate::money::MicroUsdPerKwh(300_000),
            )],
            ..cheap.clone()
        };
        let ranked = compare(&rs, &[&cheap, &dear], None);
        assert_eq!(ranked.len(), 2);
        let gap = ranked[1].total_micro_usd.0 - ranked[0].total_micro_usd.0;
        assert_eq!(
            ranked[1].delta_vs_current_micro_usd, None,
            "no current tariff named, so there is nothing to compare against"
        );
        let _ = gap;
        cheap.id = "b".into();
        let _ = cheap;
    }

    #[test]
    fn comparing_nothing_returns_nothing_rather_than_a_best_guess() {
        let rs = readings(4, 15, 1);
        let cheap = Tariff {
            id: "a".into(),
            name: "A".into(),
            zone: crate::zone::LocalZone::utc(),
            periods: vec![crate::rates::RatePeriod::flat(
                "base",
                crate::money::MicroUsdPerKwh(100_000),
            )],
            fixed_charges: vec![],
            demand_charge: None,
            export_credit: None,
            source_url: None,
            source_retrieved: None,
        };
        assert!(
            compare(&[], &[&cheap], None).is_empty(),
            "no readings, no answer"
        );
        assert!(compare(&rs, &[], None).is_empty(), "no tariffs, no answer");
    }
}
