//! HTTP-level tests.
//!
//! These exercise the real router through `tower::ServiceExt`, so the routes,
//! serialisation, status codes, and error mapping that TestSprite will drive
//! in the cloud are the same ones that run here.
//!
//! The most important test is `health_and_versions_are_stable`: CI smoke-probes
//! `/api/health` against every preview deployment, so if this drifts the gate
//! red-herrings before any credits are spent.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

fn app() -> axum::Router {
    tide_api::router(tide_api::AppState::new(), ())
}

async fn post_json(uri: &str, body: Value) -> (StatusCode, Value) {
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, value)
}

async fn get_json(uri: &str) -> (StatusCode, Value) {
    let response = app()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, value)
}

/// A grid starting at 2026-10-09T00:00:00Z, 96 slots of 15 minutes.
/// Epoch day of 2026-10-09, derived from the date rather than hard-coded.
fn epoch_day() -> i64 {
    tide_core::civil::days_from_civil(2026, 10, 9)
}

fn grid() -> Value {
    json!({
        "start_epoch_minutes": epoch_day() * 1440,
        "slot_minutes": 15,
        "slots": 96
    })
}

fn scenario(loads: Vec<Value>, cap: u32) -> Value {
    json!({
        "id": "demo",
        "name": "Demo",
        "tariff_id": "overnight-ev",
        "grid_start_epoch_minutes": epoch_day() * 1440,
        "slot_minutes": 15,
        "slots": 96,
        "site_cap_w": cap,
        "loads": loads
    })
}

fn ev_load() -> Value {
    json!({
        "id": "ev",
        "label": "EV charger",
        "energy_wh": 12000,
        "max_power_w": 7000,
        "deadline_slot": 95,
        "earliest_slot": 0,
        "prefer_contiguous": false
    })
}

#[tokio::test]
async fn health_reports_ok_and_a_version() {
    let (status, body) = get_json("/api/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["service"], "tide");
    assert!(
        body["api_version"].is_number(),
        "version must be a number so CI can pin it"
    );
}

#[tokio::test]
async fn tariffs_are_listed_with_their_source() {
    let (status, body) = get_json("/api/tariffs").await;
    assert_eq!(status, StatusCode::OK);
    let list = body["tariffs"].as_array().unwrap();
    assert!(
        list.len() >= 3,
        "expected the bundled tariffs, got {list:?}"
    );
    for tariff in list {
        assert!(tariff["id"].is_string());
        assert!(
            tariff["source_url"].is_string() && tariff["source_retrieved"].is_string(),
            "every tariff must record where its numbers came from"
        );
    }
}

#[tokio::test]
async fn a_missing_tariff_is_a_404_with_a_stable_code() {
    let (status, body) = get_json("/api/tariffs/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
    assert!(body["error"]["message"].as_str().unwrap().contains("nope"));
}

#[tokio::test]
async fn prices_are_returned_for_every_slot() {
    let (status, body) = post_json(
        "/api/prices",
        json!({ "tariff_id": "overnight-ev", "grid": grid() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let slots = body["slots"].as_array().unwrap();
    assert_eq!(slots.len(), 96);
    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(slot["slot"], i as u64);
        assert!(slot["start_epoch_minutes"].is_number());
        assert!(slot["weighted_price"].is_number());
    }
}

#[tokio::test]
async fn solve_returns_a_proved_schedule_and_a_baseline_to_compare() {
    let (status, body) = post_json(
        "/api/solve",
        json!({ "tariff_id": "overnight-ev", "scenario": scenario(vec![ev_load()], 0) }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got {body:?}");
    assert_eq!(body["schedule"]["placements"].as_array().unwrap().len(), 1);
    assert_eq!(body["optimality"], "Proved");
    assert_eq!(body["gap_percent_x1000"], 0);
    assert!(
        body["lower_bound_micro_usd"].as_i64() == body["schedule"]["cost_micro_usd"].as_i64(),
        "a proved result must sit on its own bound"
    );
    assert!(body["diagnostics"].as_array().unwrap().is_empty());
    // The baseline is the honest counterfactual and must be at least as dear.
    assert!(
        body["baseline_cost_micro_usd"].as_i64() >= body["schedule"]["cost_micro_usd"].as_i64()
    );
    // Delivery must be exact.
    assert_eq!(body["schedule"]["placements"][0]["delivered_wh"], 12000);
    assert_eq!(body["schedule"]["placements"][0]["unmet"], false);
}

#[tokio::test]
async fn solve_respects_the_site_cap_across_loads() {
    let (status, body) = post_json(
        "/api/solve",
        json!({
            "tariff_id": "overnight-ev",
            "scenario": scenario(
                vec![
                    json!({"id":"a","label":"a","energy_wh":5000,"max_power_w":5000,"deadline_slot":95}),
                    json!({"id":"b","label":"b","energy_wh":5000,"max_power_w":5000,"deadline_slot":95}),
                ],
                5_000
            )
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got {body:?}");
    for draw in body["schedule"]["site_draw_w"].as_array().unwrap() {
        assert!(draw.as_u64().unwrap() <= 5_000, "site cap breached: {draw}");
    }
}

#[tokio::test]
async fn verify_reports_an_exact_optimum_for_a_small_instance() {
    // A grid small enough for the oracle to exhaust.
    let small = json!({
        "id": "s",
        "name": "s",
        "tariff_id": "flat",
        "grid_start_epoch_minutes": 0,
        "slot_minutes": 15,
        "slots": 12,
        "site_cap_w": 0,
        "loads": [
            {"id":"a","label":"a","energy_wh":1000,"max_power_w":2000,"deadline_slot":11},
            {"id":"b","label":"b","energy_wh":500,"max_power_w":2000,"deadline_slot":11}
        ]
    });
    let (status, body) = post_json(
        "/api/solve/verify",
        json!({
            "tariff_id": "flat",
            "scenario": small,
            "oracle_node_budget": 50_000
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got {body:?}");
    assert_eq!(body["enumerated"], true, "the oracle must have run");
    assert_eq!(body["is_optimal"], true);
    assert!(body["optimal_cost_micro_usd"].is_number());
    assert!(
        body["badge"].as_str().unwrap().contains("exact optimum"),
        "badge should confirm optimality: {}",
        body["badge"]
    );
}

#[tokio::test]
async fn verify_refuses_rather_than_inventing_an_optimum() {
    // Six 20 kWh loads on a 96-slot grid: far beyond exhaustive enumeration.
    let (status, body) = post_json(
        "/api/solve/verify",
        json!({
            "tariff_id": "overnight-ev",
            "scenario": scenario(
                (0..6)
                    .map(|i| json!({"id":format!("l{i}"),"label":"l","energy_wh":20000,"max_power_w":7000,"deadline_slot":95}))
                    .collect::<Vec<_>>(),
                0
            ),
            "oracle_node_budget": 500
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got {body:?}");
    assert_eq!(
        body["enumerated"], false,
        "must not claim to have enumerated"
    );
    assert_eq!(body["optimal_cost_micro_usd"], json!(null));
    assert!(
        body["badge"].as_str().unwrap().contains("too large"),
        "the badge must admit the proof was not attempted: {}",
        body["badge"]
    );
}

#[tokio::test]
async fn bills_expose_every_line_for_audit() {
    let mut import = vec![0u64; 96];
    import[0] = 5_000; // 5 kWh
    let (status, body) = post_json(
        "/api/bills",
        json!({
            "tariff_id": "overnight-ev",
            "grid": grid(),
            "import_wh": import,
            "export_wh": vec![0u64; 96]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got {body:?}");
    assert!(!body["lines"].as_array().unwrap().is_empty());
    for line in body["lines"].as_array().unwrap() {
        assert!(line["label"].is_string());
        assert!(line["energy_wh"].is_number());
        assert!(line["amount_micro_usd"].is_number());
        // Every line must justify itself.
        assert!(
            line["detail"].as_str().unwrap().len() > 1,
            "each line needs an explanation: {line}"
        );
    }
}

#[tokio::test]
async fn an_unusable_grid_is_a_422_not_a_panic() {
    let (status, body) = post_json(
        "/api/prices",
        json!({
            "tariff_id": "flat",
            "grid": { "start_epoch_minutes": 0, "slot_minutes": 7, "slots": 10 }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "invalid_input");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("slot length"));
}

#[tokio::test]
async fn an_impossible_scenario_is_a_422_naming_the_load() {
    let (status, body) = post_json(
        "/api/solve",
        json!({
            "tariff_id": "overnight-ev",
            "scenario": scenario(
                vec![json!({"id":"ev","label":"EV","energy_wh":40000,"max_power_w":7000,"deadline_slot":2})],
                0
            )
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "invalid_input");
    let message = body["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("ev"),
        "the error must name the load: {message}"
    );
}

#[tokio::test]
async fn unknown_routes_are_a_json_404_not_a_stack_trace() {
    let (status, body) = get_json("/api/does-not-exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn malformed_json_is_a_400_with_a_stable_code() {
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/solve")
                .header("content-type", "application/json")
                .body(Body::from("{not json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn horizon_starts_at_midnight_in_the_tariffs_own_zone() {
    // The chart's axis is only meaningful if the grid starts at midnight in the
    // tariff's zone — not the viewer's. 2026-10-09 15:03 UTC is 11:03 US
    // Eastern (daylight saving), so the next local midnight is 12h57m later and
    // lands at 04:00 UTC.
    //
    // The epoch day is derived from the date rather than hard-coded: an earlier
    // version of this test used a stale constant that pointed at January, where
    // the offset is standard rather than daylight, and the assertion passed for
    // the wrong reason.
    let day = tide_core::civil::days_from_civil(2026, 10, 9);
    let now = day * 1440 + 15 * 60 + 3;
    let (status, body) = post_json(
        "/api/horizon",
        json!({ "tariff_id": "overnight-ev", "slots": 96, "slot_minutes": 15, "now_minutes": now }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got {body:?}");
    assert_eq!(body["slot_minutes"], 15);
    assert_eq!(body["start_epoch_minutes"], now + 12 * 60 + 57);
    // The cheap overnight window is the FIRST part of the returned series, which
    // is the whole point of anchoring at tariff-local midnight.
    let slots = body["slots"].as_array().unwrap();
    assert_eq!(slots.len(), 96);
    let first_hour_cheap = slots[0]["mean_micro_usd_per_kwh"].as_u64().unwrap()
        < slots[40]["mean_micro_usd_per_kwh"].as_u64().unwrap();
    assert!(
        first_hour_cheap,
        "the trough must lead the day, not trail it"
    );
}

#[tokio::test]
async fn horizon_rejects_an_oversized_grid() {
    let (status, body) = post_json(
        "/api/horizon",
        json!({ "tariff_id": "flat", "slots": 999_999, "slot_minutes": 15, "now_minutes": 0 }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "invalid_input");
}

#[tokio::test]
async fn baseline_runs_loads_where_the_household_actually_would() {
    // The saving figure is the whole product, so the counterfactual has to be
    // the household's real habit. A baseline of "as early as possible" would
    // start at midnight, land in the cheap trough, and report a saving of zero
    // for a household that overpays every night.
    let (status, body) = post_json(
        "/api/solve",
        json!({
            "tariff_id": "overnight-ev",
            "scenario": {
                "id": "d",
                "name": "d",
                "tariff_id": "overnight-ev",
                "grid_start_epoch_minutes": epoch_day() * 1440,
                "slot_minutes": 15,
                "slots": 96,
                "site_cap_w": 7000,
                "loads": [{
                    "id": "ev",
                    "label": "EV",
                    "energy_wh": 12000,
                    "max_power_w": 7000,
                    "deadline_slot": 95,
                    "natural_start_slot": 72
                }]
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got {body:?}");
    let baseline = body["baseline_placements"].as_array().unwrap();
    assert_eq!(baseline.len(), 1);
    for slot in baseline[0]["slots"].as_array().unwrap() {
        assert!(
            slot.as_u64().unwrap() >= 72,
            "the baseline must run the EV from 18:00, not midnight"
        );
    }
    assert!(
        body["baseline_cost_micro_usd"].as_i64().unwrap()
            > body["schedule"]["cost_micro_usd"].as_i64().unwrap(),
        "planning must be cheaper than the household's real habit"
    );
}

#[tokio::test]
async fn malformed_json_uses_the_same_error_envelope_as_every_other_failure() {
    // Axum's default is plain text (`Failed to deserialize the JSON body ...`),
    // which breaks the contract every other error follows. A client should never
    // have to parse prose to learn what went wrong.
    // Axum distinguishes the two: a body that is not JSON at all is a 400,
    // while valid JSON that does not match the schema is a 422. Both go through
    // the same envelope, which is what is being asserted here.
    for (bad, expected) in [
        ("{not json", StatusCode::BAD_REQUEST),
        ("[]", StatusCode::UNPROCESSABLE_ENTITY),
        ("null", StatusCode::UNPROCESSABLE_ENTITY),
        ("", StatusCode::BAD_REQUEST),
    ] {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/solve")
                    .header("content-type", "application/json")
                    .body(Body::from(bad.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "body {bad:?}");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value: Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|e| panic!("body {bad:?} did not return JSON: {e}"));
        assert!(
            value["error"]["code"].is_string(),
            "body {bad:?} must carry a stable code: {value}"
        );
        assert!(
            value["error"]["message"].is_string(),
            "body {bad:?} must carry a human message: {value}"
        );
    }
}

#[tokio::test]
async fn a_missing_field_names_the_field_rather_than_guessing() {
    let (status, body) = post_json(
        "/api/solve",
        json!({ "tariff_id": "flat", "scenario": { "name": "no id" } }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let message = body["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("id"),
        "the error should name the missing field: {message}"
    );
}
