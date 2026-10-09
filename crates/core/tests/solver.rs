//! Solver tests.
//!
//! These are the tests that justify the product's central claim. Two matter
//! most:
//!
//! 1. `solver_cost_and_bill_agree_exactly` — the schedule's cost and a bill
//!    computed over that schedule's usage are the same integer. If this ever
//!    breaks, the "two bills side by side" comparison the product is built on
//!    becomes a lie.
//! 2. `solver_is_never_worse_than_baseline` — the whole point of the product.

use tide_core::model::{Load, LoadId, Schedule, Scenario, ScenarioId};
use tide_core::oracle::{solve_baseline, verify};
use tide_core::rates::{Tariff, Usage};
use tide_core::solver::{solve, Optimality};
use tide_core::timegrid::SlotGrid;
use tide_core::money::Wh;

fn grid() -> SlotGrid {
    SlotGrid::new(20_471 * 1_440, 15, 96)
}

fn load(id: &str, kwh: u64, kw: u32, deadline: u32) -> Load {
    Load {
        id: LoadId::new(id),
        label: id.into(),
        energy_wh: Wh::from_kwh(kwh),
        max_power_w: kw * 1_000,
        deadline_slot: deadline,
        earliest_slot: 0,
        prefer_contiguous: false,
    }
}

fn scenario(loads: Vec<Load>, cap: u32) -> Scenario {
    Scenario {
        id: ScenarioId::new("s"),
        name: "test".into(),
        tariff_id: "t".into(),
        grid_start_epoch_minutes: 20_471 * 1_440,
        slot_minutes: 15,
        slots: 96,
        site_cap_w: cap,
        loads,
    }
}

/// Cheap overnight, expensive all day. Prices are weighted_price values:
/// $0.05/kWh overnight = 50_000 µ$/kWh, $0.40/kWh daytime = 400_000.
fn overnight_prices() -> Vec<u64> {
    let mut p = vec![0u64; 96];
    for (s, price) in p.iter_mut().enumerate() {
        let hour = (s * 15) / 60;
        *price = if hour < 7 { 50_000 } else { 400_000 };
    }
    p
}

fn overnight_tariff() -> Tariff {
    Tariff {
        id: "overnight".into(),
        name: "Overnight".into(),
        zone: tide_core::zone::LocalZone::utc(),
        periods: vec![
            tide_core::rates::RatePeriod::window(
                "night",
                "Overnight",
                tide_core::rates::DaySelector::EveryDay,
                0,
                420,
                tide_core::money::MicroUsdPerKwh(50_000),
            ),
            tide_core::rates::RatePeriod::window(
                "day",
                "Daytime",
                tide_core::rates::DaySelector::EveryDay,
                420,
                1_440,
                tide_core::money::MicroUsdPerKwh(400_000),
            ),
        ],
        fixed_charges: vec![],
        demand_charge: None,
        export_credit: None,
        source_url: None,
        source_retrieved: None,
    }
}

fn usage_from_schedule(schedule: &Schedule, scenario: &Scenario, grid: &SlotGrid) -> Usage {
    let _ = scenario;
    let mut usage = Usage::new(grid.slot_count as usize);
    // Accumulate and honour per-slot power: two loads may share a slot, and a
    // load's final slot typically runs at reduced power. Overwriting either
    // would lose energy and make the bill disagree with the schedule for a
    // reason that has nothing to do with the rate engine.
    for placement in schedule.placements.iter() {
        for (&s, &w) in placement.slots.iter().zip(placement.watts.iter()) {
            let wh = (u128::from(w) * u128::from(grid.slot_minutes) / 60) as u64;
            usage.import_wh[s as usize] = Wh(usage.import_wh[s as usize].0 + wh);
        }
    }
    usage
}

#[test]
fn solver_prefers_the_cheap_window() {
    let g = grid();
    let prices = overnight_prices();
    let sc = scenario(vec![load("ev", 5, 2, 95)], 0);
    let sol = solve(&sc, &g, &prices).unwrap();
    assert_eq!(sol.optimality, Optimality::Proved, "an uncoupled load must be provably optimal");
    assert_eq!(sol.gap_percent_x1000, 0);

    // Every chosen slot must be in the cheap 00:00-07:00 window (slots 0..27).
    let slots = &sol.schedule.placements[0].slots;
    assert_eq!(slots, &vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    assert!(!sol.schedule.placements[0].unmet);
}

#[test]
fn solver_never_worse_than_baseline() {
    let g = grid();
    let prices = overnight_prices();
    let sc = scenario(
        vec![
            load("ev", 10, 7, 95),
            load("dishwasher", 2, 2, 80),
            load("water", 3, 2, 95),
        ],
        7_000,
    );
    let sol = solve(&sc, &g, &prices).unwrap();
    let baseline = solve_baseline(&sc, &g, &prices);
    assert!(
        sol.schedule.cost_micro_usd.0 <= baseline.cost_micro_usd.0,
        "the optimiser must never cost more than the household's habits ({} vs {})",
        sol.schedule.cost_micro_usd,
        baseline.cost_micro_usd
    );
}

#[test]
fn solver_cost_and_bill_agree_exactly() {
    // The single most important invariant in the product.
    let g = grid();
    let tariff = overnight_tariff();
    let prices: Vec<u64> = tariff
        .price_series(&g)
        .iter()
        .map(|p| p.weighted_price)
        .collect();
    let sc = scenario(
        vec![load("ev", 12, 7, 95), load("dish", 1, 2, 90)],
        7_000,
    );
    let sol = solve(&sc, &g, &prices).unwrap();
    let usage = usage_from_schedule(&sol.schedule, &sc, &g);
    let bill = tide_core::rates::Bill::compute(&tariff, &g, &usage).unwrap();

    assert_eq!(
        bill.total, sol.schedule.cost_micro_usd,
        "the bill and the schedule must be the same number"
    );
}

#[test]
fn solver_respects_the_site_cap_everywhere() {
    let g = grid();
    let prices = vec![100u64; 96];
    let sc = scenario(
        vec![
            load("a", 5, 5, 95),
            load("b", 5, 5, 95),
            load("c", 5, 5, 95),
        ],
        5_000,
    );
    let sol = solve(&sc, &g, &prices).unwrap();
    for (s, &draw) in sol.schedule.site_draw_w.iter().enumerate() {
        assert!(draw <= 5_000, "slot {s} drew {draw} W over the cap");
    }
}

#[test]
fn solver_serves_every_load_fully() {
    let g = grid();
    let prices = overnight_prices();
    let sc = scenario(
        vec![load("ev", 20, 7, 95), load("dish", 2, 2, 95), load("water", 3, 3, 95)],
        7_000,
    );
    let sol = solve(&sc, &g, &prices).unwrap();
    for (i, p) in sol.schedule.placements.iter().enumerate() {
        assert!(!p.unmet, "load {} went unmet", sc.loads[i].id);
        assert_eq!(p.delivered_wh, sc.loads[i].energy_wh, "load {} under-delivered", sc.loads[i].id);
    }
}

#[test]
fn solver_honours_deadlines() {
    let g = grid();
    let prices = vec![100u64; 96];
    let sc = scenario(vec![load("ev", 4, 2, 20)], 0);
    let sol = solve(&sc, &g, &prices).unwrap();
    for &s in &sol.schedule.placements[0].slots {
        assert!(s <= 20, "slot {s} is past the deadline");
    }
}

#[test]
fn solver_respects_earliest_start() {
    let g = grid();
    let prices = vec![100u64; 96];
    let sc = scenario(vec![load("ev", 4, 2, 95)], 0);
    let mut sc = sc;
    sc.loads[0].earliest_slot = 50;
    let sol = solve(&sc, &g, &prices).unwrap();
    for &s in &sol.schedule.placements[0].slots {
        assert!(s >= 50, "slot {s} precedes the earliest start");
    }
}

    #[test]
    fn solver_matches_the_oracle_on_many_small_instances() {
        // The product's central claim: on every instance the oracle can
        // exhaust, the solver's answer is exactly optimal. The corpus uses
        // small grids and energies that are exact multiples of a slot's
        // energy, so the oracle genuinely enumerates the whole space instead
        // of bailing out and quietly proving nothing.
        let g = SlotGrid::new(0, 15, 12);
        let mut checked = 0;
        for price_pattern in 0..6u64 {
            let mut prices = vec![0u64; 12];
            for (s, p) in prices.iter_mut().enumerate() {
                *p = match price_pattern {
                    0 => 100,
                    1 => (s as u64 % 3) * 137,
                    2 => 23 * s as u64,
                    3 => if s < 6 { 50 } else { 400 },
                    4 => ((s * 7) % 5) as u64 * 1000 + 1,
                    _ => (s as u64 / 4) * 999,
                };
            }
            for cap in [0u32, 2_000, 4_000] {
                // 2000 W for a 15-minute slot is exactly 500 Wh, so these
                // requirements divide into whole slots with no remainder.
                let sc = Scenario {
                    id: ScenarioId::new("s"),
                    name: "t".into(),
                    tariff_id: "t".into(),
                    grid_start_epoch_minutes: 0,
                    slot_minutes: 15,
                    slots: 12,
                    site_cap_w: cap,
                    loads: vec![
                        Load {
                            id: LoadId::new("a"),
                            label: "a".into(),
                            energy_wh: Wh(1_000), // 2 slots at 500 Wh
                            max_power_w: 2_000,
                            deadline_slot: 11,
                            earliest_slot: 0,
                            prefer_contiguous: false,
                        },
                        Load {
                            id: LoadId::new("b"),
                            label: "b".into(),
                            energy_wh: Wh(500), // 1 slot at 500 Wh
                            max_power_w: 2_000,
                            deadline_slot: 11,
                            earliest_slot: 0,
                            prefer_contiguous: false,
                        },
                    ],
                };
                let sol = solve(&sc, &g, &prices).unwrap();
                let solver_numer = numer_of(&sol.schedule, &sc, &g, &prices);
                let verdict = verify(&sc, &g, &prices, solver_numer, 50_000);
                assert!(
                    verdict.is_optimal,
                    "solver was not optimal: pattern={price_pattern} cap={cap} verdict={verdict:?}"
                );
                checked += 1;
            }
        }
        assert!(checked >= 18, "expected a meaningful corpus, checked {checked}");
    }

fn numer_of(schedule: &Schedule, _sc: &Scenario, g: &SlotGrid, prices: &[u64]) -> u128 {
    let mut numer = 0u128;
    for placement in schedule.placements.iter() {
        for (&s, &w) in placement.slots.iter().zip(placement.watts.iter()) {
            let energy = u128::from(w) * u128::from(g.slot_minutes) / 60;
            numer += energy * u128::from(prices[s as usize]);
        }
    }
    numer
}

#[test]
fn solver_rejects_invalid_scenarios_with_a_reason() {
    let g = grid();
    let prices = vec![100u64; 96];
    let sc = scenario(vec![load("ev", 40, 7, 2)], 0);
    let err = solve(&sc, &g, &prices).unwrap_err();
    match err {
        tide_core::solver::ScheduleSolveError::Scenario(faults) => {
            assert!(!faults.is_empty(), "must explain why the scenario is unusable");
        }
        other => panic!("expected a scenario fault, got {other:?}"),
    }
}

#[test]
fn solver_determinism_identical_input_identical_output() {
    let g = grid();
    let prices = overnight_prices();
    let sc = scenario(
        vec![load("ev", 12, 7, 95), load("dish", 2, 2, 80), load("heat", 4, 3, 95)],
        7_000,
    );
    let a = solve(&sc, &g, &prices).unwrap();
    let b = solve(&sc, &g, &prices).unwrap();
    assert_eq!(a, b, "solving the same scenario twice must give identical output");
    assert_eq!(
        serde_json::to_string(&a.schedule).unwrap(),
        serde_json::to_string(&b.schedule).unwrap()
    );
}

#[test]
fn gap_is_zero_when_proved() {
    let g = grid();
    let prices = overnight_prices();
    let sc = scenario(vec![load("ev", 5, 2, 95)], 0);
    let sol = solve(&sc, &g, &prices).unwrap();
    assert_eq!(sol.optimality, Optimality::Proved);
    assert_eq!(sol.gap_percent_x1000, 0);
    assert!(sol.diagnostics.is_empty(), "a proved result needs no caveat");
    assert_eq!(sol.lower_bound_micro_usd, sol.schedule.cost_micro_usd);
}
