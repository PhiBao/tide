//! Verification: running the solver against the oracle and reporting the gap.
//!
//! This module exists so that "is it optimal?" has one answer, computed one
//! way, everywhere. It throws away solver internals and works purely on
//! scenarios and prices, so the API handler, the tests, and any future CLI all
//! agree by construction.

use crate::model::{Scenario, Schedule};
use crate::timegrid::SlotGrid;

/// The full verification report for one solved scenario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verification {
    /// The solver's claimed optimum.
    pub solver_cost_micro_usd: crate::money::MicroUsd,
    /// The oracle's independent optimum, when it could be computed.
    pub optimal_cost_micro_usd: Option<crate::money::MicroUsd>,
    /// True when the two agree exactly.
    pub is_optimal: bool,
    /// How much more expensive the solver was, in micro-USD. Zero when optimal
    /// or when the oracle bailed.
    pub excess_micro_usd: crate::money::MicroUsd,
    /// Whether the oracle actually enumerated.
    pub enumerated: bool,
    /// Nodes the oracle explored before stopping.
    pub nodes_explored: u64,
    /// The badge text the UI shows.
    pub badge: String,
}

/// Verify a solver answer against the brute-force oracle.
///
/// `solver_cost` must be expressed in the same integer numerator the solver and
/// the oracle share: `Σ energy_wh × weighted_price`. The API passes this
/// through; the tests compute it from the schedule.
#[must_use]
pub fn verify_solution(
    scenario: &Scenario,
    grid: &SlotGrid,
    weighted_prices: &[u64],
    solver_numer: u128,
    node_budget: u64,
) -> Verification {
    let verdict = crate::oracle::verify(scenario, grid, weighted_prices, solver_numer, node_budget);

    let solver_cost = crate::money::MicroUsd(numer_to_i64(solver_numer, grid));
    let optimal_cost = verdict
        .optimal_numer
        .map(|n| crate::money::MicroUsd(numer_to_i64(n, grid)));
    let excess = verdict
        .optimal_numer
        .map(|opt| {
            crate::money::MicroUsd(numer_to_i64(solver_numer.saturating_sub(opt), grid))
        })
        .unwrap_or(crate::money::MicroUsd::ZERO);

    Verification {
        solver_cost_micro_usd: solver_cost,
        optimal_cost_micro_usd: optimal_cost,
        is_optimal: verdict.is_optimal,
        excess_micro_usd: excess,
        enumerated: matches!(
            verdict.outcome,
            crate::oracle::OracleOutcome::Exhaustive { .. }
        ),
        nodes_explored: match verdict.outcome {
            crate::oracle::OracleOutcome::Exhaustive { nodes_explored, .. } => nodes_explored,
            crate::oracle::OracleOutcome::BudgetExceeded { nodes_explored } => nodes_explored,
        },
        badge: verdict.badge(),
    }
}

fn numer_to_i64(numer: u128, grid: &SlotGrid) -> i64 {
    let denom = 1_000u128 * u128::from(grid.slot_minutes);
    let quotient = numer / denom;
    i64::try_from(quotient as i128).unwrap_or(i64::MAX)
}

/// Recompute a schedule's cost numerator from its placements.
///
/// Used by the API to check a schedule that arrived from elsewhere, and by
/// tests to build the oracle's input. It reads only the placements, so a
/// schedule that claims the wrong cost is caught rather than trusted.
#[must_use]
pub fn schedule_numer(
    schedule: &Schedule,
    _scenario: &Scenario,
    grid: &SlotGrid,
    weighted_prices: &[u64],
) -> u128 {
    let mut numer: u128 = 0;
    for (i, placement) in schedule.placements.iter().enumerate() {
        let _ = i;
        for (&s, &w) in placement.slots.iter().zip(placement.watts.iter()) {
            let energy = u128::from(w) * u128::from(grid.slot_minutes) / 60;
            numer += energy * u128::from(weighted_prices[s as usize]);
        }
    }
    numer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::money::Wh;
    use crate::model::{Load, LoadId, Scenario, ScenarioId};
    use crate::solver::solve;
    use crate::timegrid::SlotGrid;

    #[test]
    fn verification_agrees_with_the_solver_on_a_small_instance() {
        let grid = SlotGrid::new(0, 15, 24);
        let mut prices = vec![0u64; 24];
        for (s, p) in prices.iter_mut().enumerate() {
            *p = if s < 12 { 100 } else { 900 };
        }
        let scenario = Scenario {
            id: ScenarioId::new("s"),
            name: "t".into(),
            tariff_id: "t".into(),
            grid_start_epoch_minutes: 0,
            slot_minutes: 15,
            slots: 24,
            site_cap_w: 0,
            loads: vec![
                Load {
                    id: LoadId::new("a"),
                    label: "a".into(),
                    energy_wh: Wh(1_800),
                    max_power_w: 2_000,
                    deadline_slot: 23,
                    earliest_slot: 0,
                    prefer_contiguous: false,
                },
                Load {
                    id: LoadId::new("b"),
                    label: "b".into(),
                    energy_wh: Wh(900),
                    max_power_w: 2_000,
                    deadline_slot: 23,
                    earliest_slot: 0,
                    prefer_contiguous: false,
                },
            ],
        };
        let solution = solve(&scenario, &grid, &prices).unwrap();
        let numer = schedule_numer(&solution.schedule, &scenario, &grid, &prices);
        let v = verify_solution(&scenario, &grid, &prices, numer, crate::oracle::DEFAULT_NODE_BUDGET);
        assert!(v.enumerated);
        assert!(v.is_optimal, "the oracle should confirm: {v:?}");
        assert_eq!(v.badge, "verified: exact optimum");
    }
}
