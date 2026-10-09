//! Tide's HTTP surface.
//!
//! A single [`Router`] serves every endpoint and compiles to **both** native
//! and `wasm32-unknown-unknown`, so the router used in tests is the router that
//! ships. There is no separate "server" crate to drift.
//!
//! # Route shape
//!
//! | Route | Purpose |
//! |---|---|
//! | `GET /api/health` | Liveness, with the API version. Used by CI to smoke-test a deploy. |
//! | `GET /api/tariffs` | The bundled, publicly-sourced tariffs. |
//! | `GET /api/tariffs/{id}` | One tariff's full definition. |
//! | `POST /api/prices` | The weighted price series for a grid. |
//! | `POST /api/solve` | The optimiser: schedule + cost + optimality certificate. |
//! | `POST /api/solve/verify` | The same problem solved by brute force, for comparison. |
//! | `POST /api/bills` | A line-item bill over a usage series. |
//!
//! # Error handling
//!
//! Every failure is a typed [`ApiError`] mapped to a status code, with a stable
//! machine-readable `code` plus a human `message`. A 422 always means "your
//! input is unusable", never "something broke" — so a client can distinguish a
//! bad request from a bad day without parsing prose.

use axum::{
    extract::{Json, Path, State},
    http::{header, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tide_core::model::{Load, LoadId, Scenario, ScenarioId};
use tide_core::money::{MicroUsd, Wh};
use tide_core::oracle::{solve_baseline, verify as oracle_verify, DEFAULT_NODE_BUDGET};
use tide_core::rates::{Bill, Tariff, Usage};
use tide_core::solver::{solve, Optimality};
use tide_core::timegrid::{SlotGrid, MAX_SLOTS};
use tide_core::API_VERSION;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    tariffs: Arc<Vec<Tariff>>,
}

impl AppState {
    /// Build state with the bundled tariffs.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tariffs: Arc::new(tide_core::tariffs::bundled()),
        }
    }

    #[must_use]
    pub fn with_tariffs(tariffs: Vec<Tariff>) -> Self {
        Self {
            tariffs: Arc::new(tariffs),
        }
    }

    #[must_use]
    pub fn tariff(&self, id: &str) -> Option<&Tariff> {
        self.tariffs.iter().find(|t| t.id == id)
    }

    #[must_use]
    pub fn tariffs(&self) -> &[Tariff] {
        &self.tariffs
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the router. Same router on every target.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/tariffs", get(list_tariffs))
        .route("/api/tariffs/{id}", get(get_tariff))
        .route("/api/prices", post(prices))
        .route("/api/horizon", post(horizon))
        .route("/api/solve", post(solve_scenario))
        .route("/api/solve/verify", post(verify_scenario))
        .route("/api/bills", post(bills))
        .fallback(not_found)
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_headers([
                    header::CONTENT_TYPE,
                    header::HeaderName::from_static("x-tide-api-version"),
                ]),
        )
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health() -> Response {
    Json(HealthResponse {
        status: "ok",
        api_version: API_VERSION,
        service: "tide",
    })
    .into_response()
}

async fn list_tariffs(State(state): State<AppState>) -> Response {
    let items: Vec<TariffSummary> = state
        .tariffs()
        .iter()
        .map(|t| TariffSummary {
            id: t.id.clone(),
            name: t.name.clone(),
            zone: t.zone.name.clone(),
            periods: t.periods.len(),
            source_url: t.source_url.clone(),
            source_retrieved: t.source_retrieved.clone(),
        })
        .collect();
    Json(TariffListResponse { tariffs: items }).into_response()
}

async fn get_tariff(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.tariff(&id) {
        Some(t) => Json(t.clone()).into_response(),
        None => ApiError::not_found(format!("no tariff with id '{id}'")).into_response(),
    }
}

async fn prices(State(state): State<AppState>, Json(req): Json<PriceRequest>) -> Response {
    match resolve_grid(&req.grid) {
        Ok(grid) => {
            let Some(tariff) = state.tariff(&req.tariff_id) else {
                return ApiError::not_found(format!("no tariff with id '{}'", req.tariff_id))
                    .into_response();
            };
            if let Some(resp) = gap_response(tariff) {
                return resp;
            }
            Json(price_response(tariff, &grid)).into_response()
        }
        Err(e) => e.into_response(),
    }
}

/// A grid chosen by the server, in the tariff's own timezone.
///
/// This endpoint exists because the grid's start must be **tariff-local
/// midnight**, not the browser's. A browser in UTC+8 asking for "midnight" and
/// a tariff evaluated in US Eastern describe different instants, and the
/// resulting chart would be labelled with a clock that disagrees with the prices
/// it is showing. Deciding the horizon on the server keeps every timezone rule
/// in one place — the same rule the rate engine already relies on — so the
/// client never has to know one.
async fn horizon(State(state): State<AppState>, Json(req): Json<HorizonRequest>) -> Response {
    let Some(tariff) = state.tariff(&req.tariff_id) else {
        return ApiError::not_found(format!("no tariff with id '{}'", req.tariff_id))
            .into_response();
    };
    if req.slots == 0 || req.slots > MAX_SLOTS {
        return ApiError::unprocessable(format!(
            "slots must be between 1 and {MAX_SLOTS}, got {}",
            req.slots
        ))
        .into_response();
    }
    let grid = match tariff.next_horizon(req.slots, req.slot_minutes, req.now_minutes) {
        Ok(g) => g,
        Err(e) => return ApiError::unprocessable(e.to_string()).into_response(),
    };
    Json(price_response(tariff, &grid)).into_response()
}

/// Build the price response shared by `/api/prices` and `/api/horizon`.
fn price_response(tariff: &Tariff, grid: &SlotGrid) -> PriceResponse {
    let series = tariff.price_series(grid);
    PriceResponse {
        slot_minutes: grid.slot_minutes,
        start_epoch_minutes: grid.start_epoch_minutes,
        slots: series
            .iter()
            .enumerate()
            .map(|(i, p)| SlotPriceResponse {
                slot: i as u32,
                start_epoch_minutes: grid.slot_start(i as u32),
                mean_micro_usd_per_kwh: p.mean_price().0,
                weighted_price: p.weighted_price,
            })
            .collect(),
    }
}

async fn solve_scenario(State(state): State<AppState>, Json(req): Json<SolveRequest>) -> Response {
    let Some(scenario) = build_scenario(&req.scenario) else {
        return ApiError::bad_request("scenario.id is required").into_response();
    };
    let grid = scenario.grid();
    if let Some(resp) = fault_response(&grid) {
        return resp;
    }
    let Some(tariff) = state.tariff(&req.tariff_id) else {
        return ApiError::not_found(format!("no tariff with id '{}'", req.tariff_id))
            .into_response();
    };

    match solve(&scenario, &grid, &weighted_prices(tariff, &grid)) {
        Ok(solution) => {
            let baseline = solve_baseline(&scenario, &grid, &weighted_prices(tariff, &grid));
            Json(SolveResponse {
                schedule: ScheduleResponse {
                    placements: solution
                        .schedule
                        .placements
                        .iter()
                        .map(|p| PlacementResponse {
                            load: p.load.0.clone(),
                            slots: p.slots.clone(),
                            watts: p.watts.clone(),
                            delivered_wh: p.delivered_wh.0,
                            unmet: p.unmet,
                            contiguous: p.contiguous,
                        })
                        .collect(),
                    site_draw_w: solution.schedule.site_draw_w.clone(),
                    cost_micro_usd: solution.schedule.cost_micro_usd.0,
                },
                optimality: solution.optimality,
                lower_bound_micro_usd: solution.lower_bound_micro_usd.0,
                gap_percent_x1000: solution.gap_percent_x1000,
                baseline_cost_micro_usd: baseline.cost_micro_usd.0,
                baseline_placements: baseline
                    .placements
                    .iter()
                    .map(|p| BaselinePlacement {
                        load: p.load.0.clone(),
                        slots: p.slots.clone(),
                        watts: p.watts.clone(),
                    })
                    .collect(),
                diagnostics: solution.diagnostics,
            })
            .into_response()
        }
        Err(e) => ApiError::from_solve(e).into_response(),
    }
}

async fn verify_scenario(State(state): State<AppState>, Json(req): Json<SolveRequest>) -> Response {
    let Some(scenario) = build_scenario(&req.scenario) else {
        return ApiError::bad_request("scenario.id is required").into_response();
    };
    let grid = scenario.grid();
    if let Some(resp) = fault_response(&grid) {
        return resp;
    }
    let Some(tariff) = state.tariff(&req.tariff_id) else {
        return ApiError::not_found(format!("no tariff with id '{}'", req.tariff_id))
            .into_response();
    };
    let prices = weighted_prices(tariff, &grid);

    match solve(&scenario, &grid, &prices) {
        Ok(solution) => {
            let solver_numer = solution_numer(&solution.schedule, &scenario, &grid, &prices);
            let budget = req.oracle_node_budget.unwrap_or(DEFAULT_NODE_BUDGET);
            let verdict = oracle_verify(&scenario, &grid, &prices, solver_numer, budget);
            Json(VerifyResponse {
                solver_cost_micro_usd: solution.schedule.cost_micro_usd.0,
                optimal_cost_micro_usd: verdict
                    .optimal_numer
                    .map(|n| numer_to_micro_usd(n, &grid).0),
                is_optimal: verdict.is_optimal,
                badge: verdict.badge(),
                nodes_explored: match verdict.outcome {
                    tide_core::oracle::OracleOutcome::Exhaustive { nodes_explored, .. } => {
                        nodes_explored
                    }
                    tide_core::oracle::OracleOutcome::BudgetExceeded { nodes_explored } => {
                        nodes_explored
                    }
                },
                enumerated: matches!(
                    verdict.outcome,
                    tide_core::oracle::OracleOutcome::Exhaustive { .. }
                ),
            })
            .into_response()
        }
        Err(e) => ApiError::from_solve(e).into_response(),
    }
}

async fn bills(State(state): State<AppState>, Json(req): Json<BillRequest>) -> Response {
    let grid = match resolve_grid(&req.grid) {
        Ok(g) => g,
        Err(e) => return e.into_response(),
    };
    let Some(tariff) = state.tariff(&req.tariff_id) else {
        return ApiError::not_found(format!("no tariff with id '{}'", req.tariff_id))
            .into_response();
    };
    if let Some(resp) = gap_response(tariff) {
        return resp;
    }
    let usage = Usage {
        import_wh: req.import_wh.iter().copied().map(Wh).collect(),
        export_wh: req.export_wh.iter().copied().map(Wh).collect(),
    };
    match Bill::compute(tariff, &grid, &usage) {
        Ok(bill) => Json(BillResponse {
            total_micro_usd: bill.total.0,
            lines: bill
                .lines
                .iter()
                .map(|l| BillLineResponse {
                    label: l.label.clone(),
                    energy_wh: l.energy_wh.0,
                    rate_micro_usd_per_kwh: l.rate.map(|r| r.0),
                    amount_micro_usd: l.amount.0,
                    detail: l.detail.clone(),
                })
                .collect(),
        })
        .into_response(),
        Err(e) => ApiError::unprocessable(format!("{e:?}")).into_response(),
    }
}

async fn not_found() -> Response {
    ApiError::not_found("no such route").into_response()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn weighted_prices(tariff: &Tariff, grid: &SlotGrid) -> Vec<u64> {
    tariff
        .price_series(grid)
        .into_iter()
        .map(|p| p.weighted_price)
        .collect()
}

fn resolve_grid(g: &GridRequest) -> Result<SlotGrid, ApiError> {
    let grid = SlotGrid::new(g.start_epoch_minutes, g.slot_minutes, g.slots);
    let faults = grid.validate();
    if faults.is_empty() {
        Ok(grid)
    } else {
        Err(ApiError::unprocessable(
            faults
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join("; "),
        ))
    }
}

fn fault_response(grid: &SlotGrid) -> Option<Response> {
    let faults = grid.validate();
    if faults.is_empty() {
        None
    } else {
        Some(
            ApiError::unprocessable(
                faults
                    .iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join("; "),
            )
            .into_response(),
        )
    }
}

fn gap_response(tariff: &Tariff) -> Option<Response> {
    let gaps = tariff.coverage_gaps();
    if gaps.is_empty() {
        None
    } else {
        Some(
            ApiError::unprocessable(format!(
                "tariff '{}' leaves {} interval(s) unpriced; every minute needs a price",
                tariff.id,
                gaps.len()
            ))
            .into_response(),
        )
    }
}

fn build_scenario(req: &ScenarioRequest) -> Option<Scenario> {
    let loads: Vec<Load> = req
        .loads
        .iter()
        .map(|l| Load {
            id: LoadId::new(l.id.clone()),
            label: l.label.clone().unwrap_or_else(|| l.id.clone()),
            energy_wh: Wh(l.energy_wh),
            max_power_w: l.max_power_w,
            deadline_slot: l.deadline_slot,
            earliest_slot: l.earliest_slot.unwrap_or(0),
            prefer_contiguous: l.prefer_contiguous.unwrap_or(false),
            natural_start_slot: l.natural_start_slot.unwrap_or(0),
        })
        .collect();
    Some(Scenario {
        id: ScenarioId::new(req.id.clone()),
        name: req.name.clone().unwrap_or_else(|| "Untitled".into()),
        tariff_id: req.tariff_id.clone(),
        grid_start_epoch_minutes: req.grid_start_epoch_minutes,
        slot_minutes: req.slot_minutes,
        slots: req.slots,
        site_cap_w: req.site_cap_w,
        loads,
    })
}

/// Recompute a solver cost from its placements, to compare against the oracle.
fn solution_numer(
    schedule: &tide_core::model::Schedule,
    _scenario: &Scenario,
    grid: &SlotGrid,
    prices: &[u64],
) -> u128 {
    let mut numer = 0u128;
    for (i, placement) in schedule.placements.iter().enumerate() {
        let _ = i;
        for (&s, &w) in placement.slots.iter().zip(placement.watts.iter()) {
            let energy = u128::from(w) * u128::from(grid.slot_minutes) / 60;
            numer += energy * u128::from(prices[s as usize]);
        }
    }
    numer
}

fn numer_to_micro_usd(numerator: u128, grid: &SlotGrid) -> MicroUsd {
    let denom = 1_000u128 * u128::from(grid.slot_minutes);
    MicroUsd(i64::try_from((numerator / denom) as i128).unwrap_or(i64::MAX))
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// A typed API error.
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad_request",
            message: message.into(),
        }
    }

    fn unprocessable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "invalid_input",
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: message.into(),
        }
    }

    fn from_solve(e: tide_core::solver::ScheduleSolveError) -> Self {
        match e {
            tide_core::solver::ScheduleSolveError::Scenario(faults) => Self::unprocessable(
                faults
                    .iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
            tide_core::solver::ScheduleSolveError::PriceLength { got, expected } => {
                Self::bad_request(format!("expected {expected} prices, got {got}"))
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "error": {
                "code": self.code,
                "message": self.message,
                "api_version": API_VERSION,
            }
        });
        let json = serde_json::to_string(&body).unwrap_or_else(|_| {
            "{\"error\":{\"code\":\"serialization\",\"message\":\"failed to encode error\"}}".into()
        });
        let mut response = Response::new(json.into());
        *response.status_mut() = self.status;
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        response.headers_mut().insert(
            "x-tide-api-version",
            HeaderValue::from_str(&API_VERSION.to_string())
                .unwrap_or(HeaderValue::from_static("1")),
        );
        response
    }
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    api_version: u32,
    service: &'static str,
}

#[derive(Debug, Serialize)]
struct TariffSummary {
    id: String,
    name: String,
    zone: String,
    periods: usize,
    source_url: Option<String>,
    source_retrieved: Option<String>,
}

#[derive(Debug, Serialize)]
struct TariffListResponse {
    tariffs: Vec<TariffSummary>,
}

#[derive(Debug, Deserialize)]
struct PriceRequest {
    tariff_id: String,
    grid: GridRequest,
}

#[derive(Debug, Deserialize)]
struct GridRequest {
    start_epoch_minutes: i64,
    slot_minutes: u16,
    slots: u32,
}

#[derive(Debug, Deserialize)]
struct HorizonRequest {
    tariff_id: String,
    slots: u32,
    slot_minutes: u16,
    /// The caller's current instant, as epoch minutes since 1970-01-01T00:00Z.
    ///
    /// Supplied by the client because the domain layer cannot read a clock on
    /// wasm32. It carries no timezone semantics — the zone logic that decides
    /// what "midnight" means is entirely server-side.
    now_minutes: i64,
}

#[derive(Debug, Serialize)]
struct PriceResponse {
    slot_minutes: u16,
    /// Included so the client can label the axis without ever doing timezone
    /// arithmetic of its own.
    start_epoch_minutes: i64,
    slots: Vec<SlotPriceResponse>,
}

#[derive(Debug, Serialize)]
struct SlotPriceResponse {
    slot: u32,
    start_epoch_minutes: i64,
    mean_micro_usd_per_kwh: u32,
    weighted_price: u64,
}

#[derive(Debug, Deserialize)]
struct SolveRequest {
    tariff_id: String,
    scenario: ScenarioRequest,
    oracle_node_budget: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ScenarioRequest {
    id: String,
    name: Option<String>,
    tariff_id: String,
    grid_start_epoch_minutes: i64,
    slot_minutes: u16,
    slots: u32,
    site_cap_w: u32,
    loads: Vec<LoadRequest>,
}

#[derive(Debug, Deserialize)]
struct LoadRequest {
    id: String,
    label: Option<String>,
    energy_wh: u64,
    max_power_w: u32,
    deadline_slot: u32,
    earliest_slot: Option<u32>,
    prefer_contiguous: Option<bool>,
    natural_start_slot: Option<u32>,
}

#[derive(Debug, Serialize)]
struct SolveResponse {
    schedule: ScheduleResponse,
    optimality: Optimality,
    lower_bound_micro_usd: i64,
    gap_percent_x1000: u64,
    baseline_cost_micro_usd: i64,
    /// The counterfactual's placements, so the client can bill exactly the
    /// schedule the server costed. Re-deriving it in the browser would use
    /// different assumptions and could show a saving the engine disagrees with.
    baseline_placements: Vec<BaselinePlacement>,
    diagnostics: Vec<String>,
}

/// The counterfactual's placements. Only what the client needs to bill it.
#[derive(Debug, Serialize)]
struct BaselinePlacement {
    load: String,
    slots: Vec<u32>,
    watts: Vec<u32>,
}

#[derive(Debug, Serialize)]
struct ScheduleResponse {
    placements: Vec<PlacementResponse>,
    site_draw_w: Vec<u32>,
    cost_micro_usd: i64,
}

#[derive(Debug, Serialize)]
struct PlacementResponse {
    load: String,
    slots: Vec<u32>,
    watts: Vec<u32>,
    delivered_wh: u64,
    unmet: bool,
    contiguous: bool,
}

#[derive(Debug, Serialize)]
struct VerifyResponse {
    solver_cost_micro_usd: i64,
    optimal_cost_micro_usd: Option<i64>,
    is_optimal: bool,
    badge: String,
    nodes_explored: u64,
    enumerated: bool,
}

#[derive(Debug, Deserialize)]
struct BillRequest {
    tariff_id: String,
    grid: GridRequest,
    import_wh: Vec<u64>,
    export_wh: Vec<u64>,
}

#[derive(Debug, Serialize)]
struct BillResponse {
    total_micro_usd: i64,
    lines: Vec<BillLineResponse>,
}

#[derive(Debug, Serialize)]
struct BillLineResponse {
    label: String,
    energy_wh: u64,
    rate_micro_usd_per_kwh: Option<u32>,
    amount_micro_usd: i64,
    detail: String,
}
