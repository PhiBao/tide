//! **Tide** — a composer for time-varying cost.
//!
//! `tide-core` is the entire domain: tariffs, price grids, an exact scheduler,
//! and a brute-force reference oracle that can prove the scheduler's answer.
//! It has no I/O, no async runtime, and no floating point.
//!
//! # Why those constraints matter
//!
//! - **No I/O** means the same crate compiles to both `x86_64` (where the
//!   property-test corpus runs millions of instances at full speed) and
//!   `wasm32-unknown-unknown` (where the identical arithmetic ships to
//!   Cloudflare Workers). Production and test cannot drift apart.
//! - **No floating point** means every result is reproducible by hand from the
//!   documented rules, which is what makes the line-item audit trail real
//!   rather than decorative.
//! - **No async** keeps the crate trivially callable from any runtime.
//!
//! # The module map
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`money`] | Exact fixed-point types and the rounding policy |
//! | [`civil`] | Proleptic Gregorian date arithmetic (dependency-free) |
//! | [`zone`] | Time zones, DST rules, UTC ↔ local wall clock |
//! | [`timegrid`] | The absolute settlement grid and its validation |
//! | [`rates`] | Tariffs, price series, and auditable bills |
//! | [`model`] | Loads, scenarios, and the request/response contract |
//! | [`solver`] | The exact scheduler |
//! | [`oracle`] | A brute-force reference optimum, for proving the solver |
//! | [`verify`] | Running solver and oracle against each other |

pub mod civil;
pub mod model;
pub mod money;
pub mod oracle;
pub mod rates;
pub mod solver;
pub mod tariffs;
pub mod timegrid;
pub mod usage;
pub mod verify;
pub mod zone;

/// The API contract version. Bump when a response shape changes incompatibly.
///
/// Sent as a response header so the frontend can detect a mismatch instead of
/// misreading a payload.
pub const API_VERSION: u32 = 1;

pub use model::{Load, LoadId, Scenario, ScenarioId, Schedule};
pub use money::{MicroUsd, MicroUsdPerKwh, Watts, Wh};
pub use oracle::OracleVerdict;
pub use rates::{Bill, BillLine, Tariff, Usage};
pub use solver::ScheduleSolution;
pub use timegrid::SlotGrid;
pub use usage::{EnergyUnit, ImportError, TimeColumn, UsageImport};

/// Every error this crate can return, in one enum.
///
/// Deliberately exhaustive rather than a string: an HTTP layer must be able to
/// map each variant to a status code without pattern-matching prose, and a
/// test must be able to assert on the exact failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    /// The settlement grid is structurally unusable.
    Grid(Vec<timegrid::GridFault>),
    /// Usage series do not match the grid.
    Usage(rates::UsageFault),
    /// The tariff leaves part of the week unpriced.
    TariffGap { gaps: usize },
    /// A load's constraints cannot be satisfied in this horizon.
    Infeasible {
        load: LoadId,
        reason: InfeasibleReason,
    },
    /// The oracle was asked to solve an instance larger than it can exhaust.
    OracleTooLarge { slots: u32, loads: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfeasibleReason {
    /// The load needs more slots than its window contains.
    WindowTooShort,
    /// Drawn power exceeds the site cap on its own.
    ExceedsSiteCap,
    /// Zero energy or zero power.
    DegenerateLoad,
    /// The horizon is too short to finish at the required rate.
    HorizonTooShort,
}

impl core::fmt::Display for DomainError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Grid(faults) => {
                write!(f, "invalid settlement grid: ")?;
                for (i, fault) in faults.iter().enumerate() {
                    if i > 0 {
                        f.write_str("; ")?;
                    }
                    write!(f, "{fault}")?;
                }
                Ok(())
            }
            Self::Usage(u) => write!(f, "usage series does not match the grid: {u:?}"),
            Self::TariffGap { gaps } => write!(
                f,
                "tariff leaves {gaps} uncovered interval(s); every minute needs a price"
            ),
            Self::Infeasible { load, reason } => {
                write!(f, "load {} cannot be placed: {reason:?}", load.0)
            }
            Self::OracleTooLarge { slots, loads } => write!(
                f,
                "oracle cannot exhaust a {slots}-slot, {loads}-load instance; use the solver result and its certificate instead"
            ),
        }
    }
}

impl std::error::Error for DomainError {}
