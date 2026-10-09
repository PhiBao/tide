//! The exact scheduler.
//!
//! # The problem
//!
//! Given a horizon of priced slots, loads that each need a fixed amount of
//! energy before their deadline, and a site-wide power ceiling, find the
//! cheapest feasible schedule.
//!
//! With `E_i` the energy load `i` needs, `p_s` the weighted price of slot `s`,
//! and `w_i(s) ∈ [0, W_i]` the power load `i` draws in slot `s`:
//!
//! ```text
//!   minimise   Σ_s p_s · Σ_i w_i(s)
//!   subject to Σ_s w_i(s) = E_i           for every load
//!              w_i(s) = 0                 outside load i's window
//!              Σ_i w_i(s) ≤ W_site        for every slot
//! ```
//!
//! # Why a heuristic can be *proven* correct here
//!
//! Dropping the site cap decouples the loads: each independently runs in its
//! cheapest permitted slots, and the sum of those per-load optima is a valid
//! **lower bound** on the coupled problem. If the solver's answer equals that
//! bound, it is provably the global optimum — no exhaustive search required.
//!
//! That certificate is the heart of the product. The user is shown
//! "verified: 0.00% above the exact optimum" because the number is *proved*,
//! not because an optimiser reported completion.
//!
//! # Delivering exactly the energy asked for
//!
//! A load's requirement rarely divides into whole slots: 40 kWh at 7.2 kW on a
//! 15-minute grid is 22.22 slots. Rounding up to 23 slots would deliver
//! 41.4 kWh and bill the user for 1.4 kWh they never asked for. So the last
//! slot runs at reduced power, and the placement is a list of
//! `(slot, watts)` pairs rather than a list of slots.
//!
//! The decomposition is [`crate::model::Load::decompose`], shared with the
//! oracle, so the two implementations of "serving a load" cannot drift apart.
//!
//! # Algorithm
//!
//! 1. Compute the relaxation bound (per-load cheapest slots, ignoring the cap).
//! 2. Greedy: place loads in order of urgency, cheapest feasible slots first,
//!    respecting the remaining site cap.
//! 3. Local search: repeatedly relocate power into cheaper free slots until no
//!    move improves the cost, or the round budget is exhausted.
//! 4. If the result equals the bound, report [`Optimality::Proved`].
//!
//! Every step is deterministic: iteration order is fixed by index, ties break
//! to the lower index, and there is no randomness. Identical input produces
//! byte-identical output, which the determinism tests assert.

use crate::model::{Load, Placement, Scenario, Schedule};
use crate::money::{MicroUsd, Wh};
use crate::timegrid::SlotGrid;
use serde::{Deserialize, Serialize};

/// How confident we are that the returned schedule is the cheapest possible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Optimality {
    /// The cost equals the relaxation lower bound, so the schedule is provably
    /// the global optimum.
    Proved,
    /// Local search found no improving move within its budget. The cost is
    /// above the bound, so a cheaper schedule may exist.
    Heuristic,
}

/// A solved scenario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleSolution {
    pub schedule: Schedule,
    pub optimality: Optimality,
    /// The relaxation lower bound: the cheapest conceivable cost if the loads
    /// did not have to share a power budget. Never higher than the achieved cost.
    pub lower_bound_micro_usd: MicroUsd,
    /// How far the achieved cost sits above the bound, in thousandths of a
    /// percent. Always 0 when `optimality` is [`Optimality::Proved`].
    pub gap_percent_x1000: u64,
    /// Diagnostics surfaced through the API so the number is auditable.
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleSolveError {
    Scenario(Vec<crate::model::ScenarioFault>),
    PriceLength { got: usize, expected: usize },
}

/// Rounds of local search before giving up. Bounded so a pathologically
/// coupled scenario cannot spin past a Worker's CPU budget.
const LOCAL_SEARCH_ROUNDS: u32 = 24;

/// Where one load draws power: `(slot, watts)`, ascending by slot.
type Draw = Vec<(u32, u32)>;

/// Every load's draws, in `scenario.loads` order.
type Draws = Vec<Draw>;

/// Effective power a load draws when running: its own limit, capped by the site.
#[inline]
fn effective_power(load: &Load, site_cap_w: u32) -> u32 {
    if site_cap_w == 0 {
        load.max_power_w
    } else {
        load.max_power_w.min(site_cap_w)
    }
}

/// Watt-hours delivered by drawing `watts` for one slot.
#[inline]
fn energy_of(watts: u32, slot_minutes: u16) -> u128 {
    u128::from(watts) * u128::from(slot_minutes) / 60
}

/// Slack: how many more slots the load has than it strictly needs.
#[inline]
fn slack(load: &Load, grid: &SlotGrid) -> i64 {
    let start = load.window_start(grid);
    let end = load.window_end_inclusive(grid);
    let available = end as i64 - start as i64 + 1;
    let needed = load.decompose(grid).total_slots();
    available - i64::from(needed)
}

/// The cheapest possible cost for one load if it ran alone.
///
/// Used to build the relaxation lower bound. Because every slot in the window
/// costs the same per unit of energy, the cheapest choice is simply the
/// `total_slots` lowest-priced slots in the window.
fn relaxed_load_bound(load: &Load, grid: &SlotGrid, prices: &[u64]) -> u128 {
    let power = effective_power(load, 0);
    if power == 0 {
        return 0;
    }
    let decomp = load.decompose(grid);
    let total = decomp.total_slots() as usize;
    let start = load.window_start(grid) as usize;
    let end = load.window_end_inclusive(grid) as usize;
    if start > end || total > (end - start + 1) {
        return 0;
    }

    let mut candidates: Vec<u64> = (start..=end).map(|s| prices[s]).collect();
    candidates.sort_unstable();

    // The full-power slots dominate; the reduced-power slot is cheap enough
    // that placing it in the most expensive of the chosen slots is never better
    // than the bound we are constructing. We therefore charge the partial slot
    // at its share of the *cheapest* slots, which is a valid lower bound.
    let full_slots = decomp.full_slots as usize;
    let has_partial = decomp.partial_watts > 0;
    let limit = (full_slots + usize::from(has_partial)).min(candidates.len());

    let mut numer: u128 = 0;
    // Full-power energy times price, for the cheapest full slots.
    for i in 0..full_slots.min(limit) {
        numer += energy_of(power, grid.slot_minutes) * u128::from(candidates[i]);
    }
    if has_partial && limit > full_slots {
        numer += energy_of(decomp.partial_watts, grid.slot_minutes)
            * u128::from(candidates[full_slots.min(limit - 1)]);
    }
    numer
}

/// Solve a scenario against a weighted price series.
///
/// `weighted_prices[s]` is slot `s`'s weighted price in micro-USD/kWh·minute,
/// exactly as [`crate::rates::Tariff::price_series`] produces. A slot's cost is
/// `energy_wh × weighted_price / (1000 × slot_minutes)` micro-USD, which matches
/// [`crate::rates::Bill::compute`] to the integer.
pub fn solve(
    scenario: &Scenario,
    grid: &SlotGrid,
    weighted_prices: &[u64],
) -> Result<ScheduleSolution, ScheduleSolveError> {
    if !scenario.is_valid() {
        return Err(ScheduleSolveError::Scenario(scenario.validate()));
    }
    if weighted_prices.len() != grid.slot_count as usize {
        return Err(ScheduleSolveError::PriceLength {
            got: weighted_prices.len(),
            expected: grid.slot_count as usize,
        });
    }

    // --- step 1: relaxation lower bound -----------------------------------
    // Every load independently takes its cheapest permitted slots. The sum is a
    // valid lower bound on the coupled problem, because the site cap can only
    // ever make a schedule more expensive.
    let bound_numer: u128 = scenario
        .loads
        .iter()
        .map(|l| relaxed_load_bound(l, grid, weighted_prices))
        .sum();

    // --- step 2: greedy placement -----------------------------------------
    let mut draws = greedy_place(scenario, grid, weighted_prices);

    // --- step 3: local search ---------------------------------------------
    let mut improved = true;
    let mut rounds = 0u32;
    while improved && rounds < LOCAL_SEARCH_ROUNDS {
        improved = false;
        rounds += 1;
        for i in 0..scenario.loads.len() {
            if try_relocate(&mut draws, i, scenario, grid, weighted_prices) {
                improved = true;
            }
        }
    }

    // --- step 4: assemble and certify -------------------------------------
    let cost_numer = cost_numerator(&draws, grid, weighted_prices);
    let gap_percent_x1000 = if bound_numer == 0 {
        0
    } else {
        ((cost_numer.saturating_sub(bound_numer) * 100_000) / bound_numer) as u64
    };
    let proved = cost_numer <= bound_numer;

    let mut diagnostics = Vec::new();
    if !proved {
        diagnostics.push(format!(
            "Cost is {}.{:03}% above the relaxation bound, so a cheaper schedule \
             may exist. Raising the site cap or loosening a deadline usually closes it.",
            gap_percent_x1000 / 1_000,
            gap_percent_x1000 % 1_000
        ));
    }

    Ok(ScheduleSolution {
        schedule: build_schedule(scenario, grid, &draws, cost_numer),
        optimality: if proved {
            Optimality::Proved
        } else {
            Optimality::Heuristic
        },
        lower_bound_micro_usd: numer_to_micro_usd(bound_numer, grid),
        gap_percent_x1000,
        diagnostics,
    })
}

/// Place loads in order of urgency, cheapest feasible slots first.
///
/// Urgency-first matters: a load with little slack has few options, and placing
/// it last is what makes a greedy schedule fail.
fn greedy_place(scenario: &Scenario, grid: &SlotGrid, weighted_prices: &[u64]) -> Draws {
    let slots = grid.slot_count as usize;
    let mut draws: Draws = vec![Vec::new(); scenario.loads.len()];
    // A site cap of 0 means "unlimited".
    let unlimited = scenario.site_cap_w == 0;
    let mut remaining_cap = vec![
        if unlimited {
            u32::MAX
        } else {
            scenario.site_cap_w
        };
        slots
    ];

    let mut order: Vec<usize> = (0..scenario.loads.len()).collect();
    order.sort_by(|&a, &b| {
        slack(&scenario.loads[a], grid)
            .cmp(&slack(&scenario.loads[b], grid))
            .then(a.cmp(&b))
    });

    for &i in &order {
        let load = &scenario.loads[i];
        let power = if unlimited {
            load.max_power_w
        } else {
            load.max_power_w.min(scenario.site_cap_w)
        };
        if power == 0 {
            continue;
        }
        let decomp = load.decompose(grid);
        if decomp.total_slots() == 0 {
            continue;
        }
        let start = load.window_start(grid) as usize;
        let end = load.window_end_inclusive(grid) as usize;

        // Full-power slots first, cheapest available.
        let mut candidates: Vec<usize> = (start..=end)
            .filter(|&s| remaining_cap[s] >= power)
            .collect();
        candidates.sort_by(|&a, &b| weighted_prices[a].cmp(&weighted_prices[b]).then(a.cmp(&b)));

        let mut draw: Draw = Vec::with_capacity(decomp.total_slots() as usize);
        for &s in &candidates {
            if draw.len() >= decomp.full_slots as usize {
                break;
            }
            draw.push((s as u32, power));
            remaining_cap[s] -= power;
        }

        // Then the reduced-power remainder slot, if any. It must be a slot this
        // load does not already occupy: a load draws at one power per slot, and
        // stacking 7 kW and 6 kW into the same 15 minutes would produce a
        // physically impossible 13 kW draw.
        if decomp.partial_watts > 0 {
            let used: Vec<u32> = draw.iter().map(|&(s, _)| s).collect();
            let mut rest: Vec<usize> = (start..=end)
                .filter(|&s| {
                    remaining_cap[s] >= decomp.partial_watts && !used.contains(&(s as u32))
                })
                .collect();
            rest.sort_by(|&a, &b| weighted_prices[a].cmp(&weighted_prices[b]).then(a.cmp(&b)));
            if let Some(&s) = rest.first() {
                draw.push((s as u32, decomp.partial_watts));
                remaining_cap[s] -= decomp.partial_watts;
            }
        }
        draw.sort_by_key(|&(s, _)| s);
        draws[i] = draw;
    }
    draws
}

/// Try to move one load's power into a cheaper slot. Returns true on success.
fn try_relocate(
    draws: &mut Draws,
    i: usize,
    scenario: &Scenario,
    grid: &SlotGrid,
    weighted_prices: &[u64],
) -> bool {
    let slots = grid.slot_count as usize;
    let load = &scenario.loads[i];
    let unlimited = scenario.site_cap_w == 0;
    let power = effective_power(load, scenario.site_cap_w);
    if power == 0 || draws[i].is_empty() {
        return false;
    }
    let start = load.window_start(grid) as usize;
    let end = load.window_end_inclusive(grid) as usize;

    // Draw contributed by every other load.
    let mut site_used = vec![0u32; slots];
    for (j, draw) in draws.iter().enumerate() {
        if j == i {
            continue;
        }
        for &(s, w) in draw {
            site_used[s as usize] += w;
        }
    }

    // Consider moving each (slot, watts) draw of this load into a cheaper free
    // slot, at the same power.
    let mut best: Option<((u32, u32), (u32, u32), u64)> = None;
    for &(from, w) in &draws[i] {
        let from_cost = weighted_prices[from as usize];
        for to in start..=end {
            if draws[i].iter().any(|&(s, _)| s == to as u32) {
                continue;
            }
            if !unlimited
                && u64::from(site_used[to]) + u64::from(w) > u64::from(scenario.site_cap_w)
            {
                continue;
            }
            let saving = from_cost.saturating_sub(weighted_prices[to]);
            if saving > 0 && best.map_or(true, |(_, _, b)| saving > b) {
                best = Some(((from, w), (to as u32, w), saving));
            }
        }
    }

    if let Some(((from, _), (to, w), _)) = best {
        let draw = &mut draws[i];
        draw.retain(|&(s, _)| s != from);
        draw.push((to, w));
        draw.sort_by_key(|&(s, _)| s);
        true
    } else {
        false
    }
}

/// Total cost numerator of a placement.
fn cost_numerator(draws: &Draws, grid: &SlotGrid, weighted_prices: &[u64]) -> u128 {
    let mut numer: u128 = 0;
    for draw in draws {
        for &(s, w) in draw {
            numer += energy_of(w, grid.slot_minutes) * u128::from(weighted_prices[s as usize]);
        }
    }
    numer
}

/// Convert a cost numerator to micro-USD.
fn numer_to_micro_usd(numerator: u128, grid: &SlotGrid) -> MicroUsd {
    let denom = 1_000u128 * u128::from(grid.slot_minutes);
    if denom == 0 {
        return MicroUsd::ZERO;
    }
    MicroUsd(i64::try_from((numerator / denom) as i128).unwrap_or(i64::MAX))
}

fn build_schedule(
    scenario: &Scenario,
    grid: &SlotGrid,
    draws: &Draws,
    cost_numer: u128,
) -> Schedule {
    let slots = grid.slot_count as usize;
    let mut out = Vec::with_capacity(scenario.loads.len());
    let mut site_draw = vec![0u32; slots];

    for (i, load) in scenario.loads.iter().enumerate() {
        let mut ordered = draws[i].clone();
        ordered.sort_by_key(|&(s, _)| s);

        let delivered: u128 = ordered
            .iter()
            .map(|&(_, w)| energy_of(w, grid.slot_minutes))
            .sum();
        for &(s, w) in &ordered {
            if (s as usize) < slots {
                site_draw[s as usize] += w;
            }
        }

        let slots_only: Vec<u32> = ordered.iter().map(|&(s, _)| s).collect();
        let watts_only: Vec<u32> = ordered.iter().map(|&(_, w)| w).collect();
        let contiguous = slots_only.len() <= 1 || slots_only.windows(2).all(|w| w[1] == w[0] + 1);

        out.push(Placement {
            load: load.id.clone(),
            slots: slots_only,
            watts: watts_only,
            delivered_wh: Wh(delivered.min(u128::from(load.energy_wh.0)) as u64),
            unmet: delivered < u128::from(load.energy_wh.0),
            contiguous,
        });
    }

    Schedule {
        placements: out,
        site_draw_w: site_draw,
        cost_micro_usd: numer_to_micro_usd(cost_numer, grid),
    }
}
