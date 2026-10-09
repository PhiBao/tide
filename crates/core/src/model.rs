//! The user-facing model: loads, scenarios, and schedules.
//!
//! A [`Scenario`] is the complete input to a decision, and it is
//! **URL-encodable by design**. A saved scenario is not a database row — it is
//! a short, self-describing value the user can share. That removes the need
//! for accounts, keeps the test suite hermetic (no cross-test state pollution,
//! which is the single largest source of flaky e2e tests), and makes the
//! "compare two scenarios" workflow trivial.
//!
//! # The load model
//!
//! A [`Load`] is a thing that needs energy by a deadline. Everything else
//! about it is either derived or a preference:
//!
//! | Field | Meaning |
//! |---|---|
//! | `energy_wh` | How much energy it needs. A requirement. |
//! | `max_power_w` | How fast it can draw. A limit, not a constant. |
//! | `deadline_slot` | The slot by which it must be finished. A requirement. |
//! | `earliest_slot` | The first slot it may run in. Defaults to 0. |
//!
//! Loads may **split** across slots. A real EV charger, dishwasher or battery
//! does exactly this, and forbidding it would make the optimiser's answer
//! obviously suboptimal on day one.

use crate::money::{MicroUsd, Wh};
use crate::timegrid::SlotGrid;
use serde::{Deserialize, Serialize};

/// Stable identifier for a load, so a schedule can be traced back to its input.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LoadId(pub String);

impl LoadId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl core::fmt::Display for LoadId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Something that must consume energy before a deadline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Load {
    pub id: LoadId,
    pub label: String,
    /// Total energy the load must receive, in watt-hours.
    pub energy_wh: Wh,
    /// Maximum draw, in watts.
    pub max_power_w: u32,
    /// The load must be complete by the end of this slot (inclusive).
    pub deadline_slot: u32,
    /// The load may not run before this slot. Defaults to 0.
    pub earliest_slot: u32,
    /// Whether the user wants a single contiguous run. Defaults to `false`.
    ///
    /// Contiguity is a *preference* because it is what some appliances want
    /// (a heat pump restarting repeatedly wastes energy). The solver honours it
    /// when it can and reports when it could not.
    pub prefer_contiguous: bool,

    /// The slot at which the user would *naturally* start this load, without
    /// planning — when they get home and plug the car in, when they run the
    /// dishwasher after dinner.
    ///
    /// This is what the baseline schedule uses, and getting it right is what
    /// makes the saving figure honest. A baseline of "as early as possible"
    /// would start at midnight and accidentally land in the cheap trough,
    /// showing a saving of zero for a household that is in fact overpaying
    /// every single night.
    pub natural_start_slot: u32,
}

impl Load {
    /// Energy drawn in one slot at full power.
    #[must_use]
    pub fn energy_per_slot(&self, grid: &SlotGrid) -> Wh {
        Wh((u128::from(self.max_power_w) * u128::from(grid.slot_minutes) / 60) as u64)
    }

    /// Number of full-power slots needed, rounded up.
    #[must_use]
    pub fn slots_needed(&self, grid: &SlotGrid) -> u32 {
        let per_slot = self.energy_per_slot(grid).0;
        if per_slot == 0 {
            return 0;
        }
        u32::try_from(self.energy_wh.0.div_ceil(per_slot)).unwrap_or(u32::MAX)
    }

    /// The first slot at which running is possible, clamped to the grid.
    #[must_use]
    pub fn window_start(&self, grid: &SlotGrid) -> u32 {
        self.earliest_slot.min(grid.slot_count)
    }

    /// The last slot in which the load may still be running.
    #[must_use]
    pub fn window_end_inclusive(&self, grid: &SlotGrid) -> u32 {
        self.deadline_slot.min(grid.slot_count.saturating_sub(1))
    }
}

/// A shareable, self-describing decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scenario {
    pub id: ScenarioId,
    pub name: String,
    pub tariff_id: String,
    /// Epoch minute at which the grid starts.
    pub grid_start_epoch_minutes: i64,
    pub slot_minutes: u16,
    pub slots: u32,
    /// Site-wide power ceiling in watts. Instantaneous sum of running loads
    /// must never exceed this.
    pub site_cap_w: u32,
    pub loads: Vec<Load>,
}

/// Stable identifier for a scenario.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScenarioId(pub String);

impl ScenarioId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl core::fmt::Display for ScenarioId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Scenario {
    #[must_use]
    pub fn grid(&self) -> SlotGrid {
        SlotGrid::new(self.grid_start_epoch_minutes, self.slot_minutes, self.slots)
    }

    /// Structural problems with this scenario, in a stable order.
    #[must_use]
    pub fn validate(&self) -> Vec<ScenarioFault> {
        let mut faults = Vec::new();
        faults.extend(self.grid().validate().into_iter().map(ScenarioFault::Grid));

        let grid = self.grid();
        for load in &self.loads {
            if load.energy_wh.is_zero() || load.max_power_w == 0 {
                faults.push(ScenarioFault::DegenerateLoad {
                    load: load.id.clone(),
                });
            }
            if load.window_start(&grid) > load.window_end_inclusive(&grid) {
                faults.push(ScenarioFault::EmptyWindow {
                    load: load.id.clone(),
                });
            }
            if load.slots_needed(&grid)
                > load.window_end_inclusive(&grid) - load.window_start(&grid) + 1
            {
                faults.push(ScenarioFault::WindowTooShort {
                    load: load.id.clone(),
                    needed: load.slots_needed(&grid),
                    available: load.window_end_inclusive(&grid) - load.window_start(&grid) + 1,
                });
            }
            if self.site_cap_w > 0 && u64::from(load.max_power_w) > u64::from(self.site_cap_w) {
                faults.push(ScenarioFault::ExceedsSiteCap {
                    load: load.id.clone(),
                });
            }
            if load.natural_start_slot > grid.slot_count {
                faults.push(ScenarioFault::NaturalStartOutOfRange {
                    load: load.id.clone(),
                });
            }
        }
        faults
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.validate().is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScenarioFault {
    Grid(crate::timegrid::GridFault),
    DegenerateLoad {
        load: LoadId,
    },
    EmptyWindow {
        load: LoadId,
    },
    WindowTooShort {
        load: LoadId,
        needed: u32,
        available: u32,
    },
    ExceedsSiteCap {
        load: LoadId,
    },
    NaturalStartOutOfRange {
        load: LoadId,
    },
}

impl core::fmt::Display for ScenarioFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Grid(g) => write!(f, "{g}"),
            Self::DegenerateLoad { load } => {
                write!(f, "load {load} needs either zero energy or zero power")
            }
            Self::EmptyWindow { load } => {
                write!(f, "load {load} starts after its deadline")
            }
            Self::WindowTooShort { load, needed, available } => write!(
                f,
                "load {load} needs {needed} slots but only {available} are available before its deadline"
            ),
            Self::ExceedsSiteCap { load } => {
                write!(f, "load {load} draws more than the site cap on its own")
            }
            Self::NaturalStartOutOfRange { load } => {
                write!(f, "load {load} would naturally start outside the horizon")
            }
        }
    }
}

/// The result of solving a scenario: when each load runs, and what it costs.
///
/// Serialisable so the API can return it directly, and so a scenario's solution
/// can be embedded in a shareable URL — the product has no accounts, so a link
/// *is* the persistence layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    /// One entry per load, in scenario order. Each entry lists the slots the
    /// load is running in, in ascending order.
    pub placements: Vec<Placement>,
    /// Total site draw per slot, in watts. Never exceeds the site cap.
    pub site_draw_w: Vec<u32>,
    /// Exact cost, in micro-USD, of the scheduled usage.
    ///
    /// Uses the *same* integer numerator as [`crate::rates::Bill::compute`], so
    /// a bill computed over this schedule totals exactly this figure. The test
    /// suite asserts it.
    pub cost_micro_usd: MicroUsd,
}

/// Where one load runs.
///
/// `slots` and `watts` are parallel arrays: the load draws `watts[i]` watts
/// during slot `slots[i]`. Both are needed because a load's requirement rarely
/// divides into whole slots, so its final slot typically runs at reduced power.
///
/// Carrying the power alongside the slot index is what lets an *independent*
/// consumer — the oracle's verifier, the API's bill — recompute the cost
/// exactly. A placement listing only slot indices would force every consumer to
/// assume full power, silently over-charging for the partial slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub load: LoadId,
    /// Ascending slot indices the load draws power in.
    pub slots: Vec<u32>,
    /// Power drawn in each of those slots, in watts. Same length as `slots`.
    pub watts: Vec<u32>,
    /// Energy actually delivered, in watt-hours. Always equals the load's
    /// requirement unless the load is reported unmet.
    pub delivered_wh: Wh,
    /// True if the load could not be fully satisfied.
    pub unmet: bool,
    /// True if the load runs as a single unbroken run, when it asked to.
    pub contiguous: bool,
}

impl Placement {
    /// Energy this placement draws in `slot`, in watt-hours.
    #[must_use]
    pub fn energy_in_slot(&self, slot: u32, slot_minutes: u16) -> Wh {
        let watts: u128 = self
            .slots
            .iter()
            .zip(self.watts.iter())
            .filter(|(&s, _)| s == slot)
            .map(|(_, &w)| u128::from(w))
            .sum();
        Wh((watts * u128::from(slot_minutes) / 60) as u64)
    }
}

impl Schedule {
    /// Build a `Usage` series from this schedule, for billing.
    #[must_use]
    pub fn to_usage(&self, _scenario: &Scenario, grid: &SlotGrid) -> crate::rates::Usage {
        let mut usage = crate::rates::Usage::new(grid.slot_count as usize);
        for placement in &self.placements {
            for (&slot, &watts) in placement.slots.iter().zip(placement.watts.iter()) {
                if (slot as usize) >= grid.slot_count as usize {
                    continue;
                }
                let wh = u128::from(watts) * u128::from(grid.slot_minutes) / 60;
                usage.import_wh[slot as usize] = Wh(usage.import_wh[slot as usize].0 + wh as u64);
            }
        }
        usage
    }

    /// A baseline "run it as early as possible" schedule, for comparison.
    ///
    /// This is the honest counterfactual: what the household's current habits
    /// cost. It respects every constraint the optimiser does — the site cap,
    /// deadlines, windows — it just prefers the earliest slots instead of the
    /// cheapest.
    ///
    /// The baseline lives in [`crate::oracle::solve_baseline`], next to the
    /// verifier, so the two halves of the comparison share the same schedule
    /// representation and cannot drift apart.
    #[must_use]
    pub fn baseline(&self, scenario: &Scenario, grid: &SlotGrid, weighted_prices: &[u64]) -> Self {
        crate::oracle::solve_baseline(scenario, grid, weighted_prices)
    }
}

/// Deterministic, stable ordering key for scenario and load IDs, so that any
/// output derived from iteration order is reproducible.
pub fn canonical_load_order(loads: &mut [Load]) {
    loads.sort_by(|a, b| a.id.0.cmp(&b.id.0));
}

/// How a load's energy requirement decomposes into slots.
///
/// This is the single definition of "what it means to serve a load", shared by
/// the solver and the oracle so the two cannot disagree.
///
/// A load needing 40 kWh at 7.2 kW on a 15-minute grid occupies 22 full slots
/// (39.6 kWh) plus one slot drawing 6 kW for 15 minutes (1.5 kWh), totalling
/// exactly 40 kWh. Rounding the slot count *up* instead would deliver 41.4 kWh
/// — quietly charging the user for energy they never asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decomposition {
    /// Slots run at the load's full power.
    pub full_slots: u32,
    /// Power of one final reduced-power slot, in watts. Zero when the
    /// requirement divides evenly into whole slots.
    pub partial_watts: u32,
}

impl Decomposition {
    /// Total slots occupied: `full_slots`, plus one more if there is a
    /// remainder to deliver.
    #[must_use]
    pub fn total_slots(&self) -> u32 {
        self.full_slots + u32::from(self.partial_watts > 0)
    }

    /// Energy delivered by this decomposition, in watt-hours.
    #[must_use]
    pub fn delivered_wh(&self, power_w: u32, slot_minutes: u16) -> u64 {
        let per_slot = u128::from(power_w) * u128::from(slot_minutes) / 60;
        let partial = u128::from(self.partial_watts) * u128::from(slot_minutes) / 60;
        (per_slot * u128::from(self.full_slots) + partial) as u64
    }
}

impl Load {
    /// Decompose this load's requirement into full and partial slots.
    #[must_use]
    pub fn decompose(&self, grid: &SlotGrid) -> Decomposition {
        let per_slot = u128::from(self.max_power_w) * u128::from(grid.slot_minutes) / 60;
        if per_slot == 0 {
            return Decomposition {
                full_slots: 0,
                partial_watts: 0,
            };
        }
        let energy = u128::from(self.energy_wh.0);
        let full = (energy / per_slot) as u32;
        let rem = energy % per_slot;
        let partial_watts = if rem == 0 {
            0
        } else {
            // watts = rem_Wh * 60 / slot_minutes. Exact for every allowed slot
            // length, and strictly less than `max_power_w`.
            (rem * 60 / u128::from(grid.slot_minutes)) as u32
        };
        Decomposition {
            full_slots: full,
            partial_watts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            natural_start_slot: 0,
        }
    }

    #[test]
    fn energy_per_slot_is_exact_for_whole_kilowatts() {
        let g = grid();
        // The helper's `kw` parameter is multiplied by 1000, so 7 means 7 kW,
        // not 7.2 kW. 7 kW for a 15-minute slot is 1750 Wh.
        assert_eq!(load("ev", 40, 7, 90).energy_per_slot(&g), Wh(1_750));
        // And an explicit 7.2 kW load delivers 1.8 kWh per slot.
        let ev = Load {
            id: LoadId::new("ev"),
            label: "ev".into(),
            energy_wh: Wh::from_kwh(40),
            max_power_w: 7_200,
            deadline_slot: 90,
            earliest_slot: 0,
            prefer_contiguous: false,
            natural_start_slot: 0,
        };
        assert_eq!(ev.energy_per_slot(&g), Wh(1_800));
    }

    #[test]
    fn slots_needed_rounds_up_so_the_load_is_always_fully_served() {
        let g = grid();
        // 40 kWh at 1.8 kWh/slot = 22.22 -> 23 slots
        assert_eq!(load("ev", 40, 7, 90).slots_needed(&g), 23);
        // 36 kWh at 1.75 kWh/slot = 20.57 -> 21 slots
        assert_eq!(load("ev", 36, 7, 90).slots_needed(&g), 21);
        // 35 kWh at 1.75 kWh/slot is exactly 20 slots, with no remainder.
        assert_eq!(load("ev", 35, 7, 90).slots_needed(&g), 20);
    }

    #[test]
    fn window_is_clamped_to_the_grid() {
        let g = grid();
        let mut l = load("ev", 40, 7, 500);
        assert_eq!(
            l.window_end_inclusive(&g),
            95,
            "deadline past the end clamps to the last slot"
        );
        l.earliest_slot = 200;
        assert_eq!(l.window_start(&g), 96);
        assert!(l.window_start(&g) > l.window_end_inclusive(&g));
    }

    #[test]
    fn validation_reports_window_too_short_with_numbers() {
        let g = grid();
        let mut scenario = Scenario {
            id: ScenarioId::new("s1"),
            name: "Impossible".into(),
            tariff_id: "t".into(),
            grid_start_epoch_minutes: g.start_epoch_minutes,
            slot_minutes: g.slot_minutes,
            slots: g.slot_count,
            site_cap_w: 7_000,
            loads: vec![load("ev", 40, 7, 4)],
        };
        let faults = scenario.validate();
        assert!(
            matches!(
                faults.as_slice(),
                [ScenarioFault::WindowTooShort {
                    needed: 23,
                    available: 5,
                    ..
                }]
            ),
            "got {faults:?}"
        );

        // And with a wide-enough window it validates.
        scenario.loads[0].deadline_slot = 40;
        assert!(
            scenario.is_valid(),
            "should be valid once the window fits: {:?}",
            scenario.validate()
        );
    }

    #[test]
    fn validation_reports_a_load_that_exceeds_the_site_cap() {
        let g = grid();
        let scenario = Scenario {
            id: ScenarioId::new("s2"),
            name: "Too big".into(),
            tariff_id: "t".into(),
            grid_start_epoch_minutes: g.start_epoch_minutes,
            slot_minutes: g.slot_minutes,
            slots: g.slot_count,
            site_cap_w: 3_000,
            loads: vec![load("ev", 40, 7, 90)],
        };
        assert!(matches!(
            scenario.validate().as_slice(),
            [ScenarioFault::ExceedsSiteCap { .. }]
        ));
    }

    #[test]
    fn canonical_order_is_by_id_so_output_is_reproducible() {
        let mut loads = vec![
            load("c", 1, 1, 10),
            load("a", 1, 1, 10),
            load("b", 1, 1, 10),
        ];
        canonical_load_order(&mut loads);
        let ids: Vec<&str> = loads.iter().map(|l| l.id.0.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }
}
