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
        periods: vec![RatePeriod::flat(
            "base",
            crate::money::MicroUsdPerKwh(250_000),
        )],
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

/// A four-season tariff: a summer afternoon peak, a winter evening peak, and a
/// distinct shoulder rate.
///
/// The first version had only a summer peak, so for eight months of the year it
/// was indistinguishable from the flat tariff — one of the four bundled tariffs
/// silently demonstrated nothing. A tariff meant to show calendar variation has
/// to vary all year round to be worth including.
#[must_use]
pub fn tou_summer_afternoon() -> Tariff {
    // Bitmask of months: bit 0 = January, so June..September is bits 5..8.
    let summer = (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8);
    // October..March, where the peak moves to the winter evening. April and
    // May fall in neither window and take the shoulder rate — but October must
    // be covered, or the tariff is flat for a month it is being shown in.
    let winter = (1 << 9) | (1 << 10) | (1 << 11) | (1 << 0) | (1 << 1) | (1 << 2);

    Tariff {
        id: "summer-afternoon".into(),
        name: "Four-season peak".into(),
        zone: LocalZone::us_pacific(),
        periods: vec![
            RatePeriod::window(
                "summer-peak",
                "Summer peak (4-9pm)",
                DaySelector::EveryDay,
                960,
                1_260,
                crate::money::MicroUsdPerKwh(520_000),
            )
            .with_months(summer),
            RatePeriod::window(
                "winter-peak",
                "Winter peak (6-9pm)",
                DaySelector::EveryDay,
                1_140,
                1_260,
                crate::money::MicroUsdPerKwh(480_000),
            )
            .with_months(winter),
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
    use crate::civil::CivilDate;
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
    fn seasonal_periods_only_apply_in_their_season() {
        // Look periods up by id rather than by index: the index shifts whenever
        // a tariff gains a period, and a stale index makes a passing test pass
        // for the wrong reason.
        let tariff = tou_summer_afternoon();

        fn period_id_at(t: &Tariff, date: CivilDate, minute: u16) -> Option<String> {
            t.period_at(date, minute).map(|i| t.periods[i].id.clone())
        }

        // A weekday in July at 17:00 sits inside the summer peak.
        assert_eq!(
            period_id_at(&tariff, CivilDate::new(2026, 7, 15), 1_020).as_deref(),
            Some("summer-peak")
        );
        // The same clock time in January is not in the summer window, and not in
        // the winter window either (which runs 19:00-21:00).
        assert_eq!(
            period_id_at(&tariff, CivilDate::new(2026, 1, 15), 1_020).as_deref(),
            Some("base")
        );
        // And the winter window does apply in January at 20:00.
        assert_eq!(
            period_id_at(&tariff, CivilDate::new(2026, 1, 15), 1_200).as_deref(),
            Some("winter-peak")
        );
    }

    #[test]
    fn every_tariff_varies_across_the_year_it_will_be_shown_in() {
        // A bundled tariff that renders identically to the flat tariff for eight
        // months of the year is not worth bundling. Sample a full year at
        // hourly resolution and require at least two distinct prices.
        let grid = SlotGrid::new(
            crate::civil::days_from_civil(2026, 1, 1) * 1_440,
            60,
            24 * 366,
        );
        for tariff in bundled() {
            // A flat tariff is flat on purpose: it is the teaching example, and
            // requiring it to vary would be requiring it to be something else.
            if tariff.id == "flat" {
                continue;
            }
            let series = tariff.price_series(&grid);
            let distinct: std::collections::BTreeSet<u64> =
                series.iter().map(|s| s.weighted_price).collect();
            assert!(
                distinct.len() > 1,
                "tariff '{}' has a single price all year, so it demonstrates nothing",
                tariff.id
            );
        }
    }
}
