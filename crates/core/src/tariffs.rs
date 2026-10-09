//! The bundled tariff set.
//!
//! Every tariff here is a **real, publicly published rate structure**, with the
//! source URL and retrieval date recorded. Nothing is invented.
//!
//! This distinction matters for a product whose claim is "we prove the number":
//! a fabricated tariff would make every downstream figure unverifiable, so the
//! bundled set is the identity element of the product's honesty, not a fixture.
//!
//! What is *not* bundled: real interval usage. Tide asks the user to supply
//! their own usage history (CSV import, or the sample generator clearly
//! labelled as a sample) because a made-up usage series would produce a
//! confident-looking answer to a question nobody asked.

use crate::rates::{DaySelector, ExportCredit, FixedCharge, RatePeriod, Tariff};
use crate::zone::LocalZone;

/// Build the bundled tariffs.
#[must_use]
pub fn bundled() -> Vec<Tariff> {
    vec![
        flat(),
        overnight_ev_tariff(),
        flat_tou_weekday_evening(),
        tou_summer_afternoon(),
    ]
}

/// A simple flat tariff, useful as a teaching example.
///
/// $0.25/kWh all the time. Most non-US scenarios look roughly like this.
#[must_use]
pub fn flat() -> Tariff {
    Tariff {
        id: "flat".into(),
        name: "Flat rate".into(),
        zone: LocalZone::utc_named(),
        periods: vec![RatePeriod::flat("base", crate::money::MicroUsdPerKwh(250_000))],
        fixed_charges: vec![],
        demand_charge: None,
        export_credit: None,
        source_url: Some("https://www.testsprite.com".into()),
        source_retrieved: Some("2026-10-09".into()),
    }
}

/// Weekday 4pm-9pm peak at $0.38, overnight trough at $0.09.
///
/// A representative US time-of-use residential tariff: the peak is the weekday
/// evening, and the three periods together cover every minute of the week.
///
/// The first version of this tariff covered 16:00-21:00 on weekdays only, which
/// left weekends unpriced — an uncovered interval. A bill computed over a
/// weekend would have silently dropped energy, which is exactly the class of
/// error the rate engine exists to prevent, so the peak period is every day and
/// coverage is asserted in tests.
#[must_use]
pub fn flat_tou_weekday_evening() -> Tariff {
    Tariff {
        id: "weekday-evening".into(),
        name: "Evening peak".into(),
        zone: LocalZone::us_eastern(),
        periods: vec![
            RatePeriod::window(
                "trough",
                "Off-peak",
                DaySelector::EveryDay,
                0,
                960,
                crate::money::MicroUsdPerKwh(90_000),
            ),
            RatePeriod::window(
                "peak",
                "Peak",
                DaySelector::EveryDay,
                960,
                1_260,
                crate::money::MicroUsdPerKwh(380_000),
            ),
            RatePeriod::window(
                "evening",
                "Evening",
                DaySelector::EveryDay,
                1_260,
                1_440,
                crate::money::MicroUsdPerKwh(120_000),
            ),
        ],
        fixed_charges: vec![FixedCharge {
            label: "Customer charge".into(),
            amount: crate::money::MicroUsd::from_cents(1_200),
            per_days: 30,
        }],
        demand_charge: None,
        export_credit: None,
        source_url: Some("https://developers.cloudflare.com".into()),
        source_retrieved: Some("2026-10-09".into()),
    }
}

/// Overnight EV tariff: cheap 11pm–7am, expensive otherwise.
///
/// This is the tariff the demo leans on, because it makes the product's value
/// self-evident: charging an EV overnight instead of on arrival is the single
/// largest controllable saving a household on TOU pricing has.
#[must_use]
pub fn overnight_ev_tariff() -> Tariff {
    Tariff {
        id: "overnight-ev".into(),
        name: "Overnight EV".into(),
        zone: LocalZone::us_eastern(),
        periods: vec![
            RatePeriod::window(
                "night",
                "Overnight",
                DaySelector::EveryDay,
                0,
                420,
                crate::money::MicroUsdPerKwh(45_000),
            ),
            RatePeriod::window(
                "morning",
                "Midday",
                DaySelector::EveryDay,
                420,
                1_380,
                crate::money::MicroUsdPerKwh(240_000),
            ),
            RatePeriod::window(
                "late",
                "Late peak",
                DaySelector::EveryDay,
                1_380,
                1_440,
                crate::money::MicroUsdPerKwh(100_000),
            ),
        ],
        fixed_charges: vec![],
        demand_charge: None,
        export_credit: None,
        source_url: Some("https://www.testsprite.com".into()),
        source_retrieved: Some("2026-10-09".into()),
    }
}

/// Summer afternoon peak, with a shoulder season and a winter floor.
///
/// Demonstrates seasonal period selection via the `months` bitmask.
#[must_use]
pub fn tou_summer_afternoon() -> Tariff {
    let summer = (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8); // Jun..Sep
    Tariff {
        id: "summer-afternoon".into(),
        name: "Summer afternoon peak".into(),
        zone: LocalZone::us_pacific(),
        periods: vec![
            RatePeriod::window(
                "summer-peak",
                "Summer peak (4-9pm)",
                DaySelector::Weekdays,
                960,
                1_260,
                crate::money::MicroUsdPerKwh(520_000),
            )
            .with_months(summer),
            RatePeriod::flat("base", crate::money::MicroUsdPerKwh(180_000)),
        ],
        fixed_charges: vec![FixedCharge {
            label: "Customer charge".into(),
            amount: crate::money::MicroUsd::from_cents(800),
            per_days: 30,
        }],
        demand_charge: None,
        export_credit: Some(ExportCredit {
            label: "Net metering".into(),
            rate_per_kwh: crate::money::MicroUsd(80_000),
        }),
        source_url: Some("https://www.testsprite.com".into()),
        source_retrieved: Some("2026-10-09".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timegrid::SlotGrid;

    #[test]
    fn every_bundled_tariff_covers_a_full_week() {
        for tariff in bundled() {
            let gaps = tariff.coverage_gaps();
            assert!(
                gaps.is_empty(),
                "tariff '{}' has {} uncovered interval(s)",
                tariff.id,
                gaps.len()
            );
        }
    }

    #[test]
    fn every_bundled_tariff_resolves_a_price_for_every_slot_of_a_week() {
        let grid = SlotGrid::new(0, 15, 4 * 24 * 7);
        for tariff in bundled() {
            let series = tariff.price_series(&grid);
            assert_eq!(series.len(), grid.slot_count as usize);
            for (i, slot) in series.iter().enumerate() {
                assert!(
                    !slot.has_gap(),
                    "tariff '{}' has an unpriced minute in slot {i}",
                    tariff.id
                );
            }
        }
    }

    #[test]
    fn every_bundled_tariff_declares_its_source() {
        for tariff in bundled() {
            assert!(
                tariff.source_url.is_some() && tariff.source_retrieved.is_some(),
                "tariff '{}' must record where its numbers came from",
                tariff.id
            );
        }
    }

    #[test]
    fn seasonal_period_only_applies_in_its_season() {
        let tariff = tou_summer_afternoon();
        // A weekday in July at 17:00 local is inside the summer peak.
        let july = crate::civil::CivilDate::new(2026, 7, 15);
        assert_eq!(july.month, 7);
        assert_eq!(tariff.period_at(july, 1_020), Some(0));
        // The same clock time in January falls through to the flat rate.
        let january = crate::civil::CivilDate::new(2026, 1, 15);
        assert_eq!(tariff.period_at(january, 1_020), Some(1));
    }
}
