//! Cloudflare Worker entrypoint for Tide.
//!
//! Most of the HTTP surface is the same [`tide_api::router`] that runs in
//! tests, on `wasm32-unknown-unknown`, behind the Workers `http` adapter.
//! There is exactly one router definition in this repository.
//!
//! The `/api/history/*` routes are served from here instead. They need D1, and
//! D1's futures are not `Send` on `wasm32-unknown-unknown` — they hold
//! `Rc<RefCell<...>>` — while `axum::Handler` requires a `Send` future. A
//! `fetch` entrypoint is not `Send`-constrained, so those routes are
//! dispatched below. The logic stays in `tide-api` as plain async functions
//! over [`Store`], which keeps it testable without a datastore.
//!
//! The worker also owns the session cookie. History is partitioned per browser
//! session, so a test run's synthetic readings and a visitor's real ones never
//! share a bucket; this file reads, validates, and mints the partition key and
//! attaches it to every response.
//!
//! Static assets come from the `ASSETS` binding, which Cloudflare populates
//! from the Next.js static export. Because `run_worker_first` is configured for
//! `/api/*` only, this handler is invoked for API routes and never for static
//! files, so the asset-serving path never depends on Rust code being loaded.
//!
//! The router is rebuilt per request rather than cached in a global: every
//! endpoint is a pure function of its request, so there is nothing worth
//! caching, and a Worker isolate is shared between concurrent requests, so
//! global mutable state would be a correctness hazard rather than a saving.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use http::{header, HeaderValue, StatusCode};
use tide_api::{
    history_bills, history_clear, history_compare, history_import, ApiError, AppState,
    HistoryCompareRequest, HistoryImportRequest, SessionId, Store, StoredBill,
};
use tide_core::history::Reading;
use tower_service::Service as _;
use worker::{wasm_bindgen::JsValue, *};

/// The D1-backed [`Store`].
///
/// The `?Send` on the impl is load-bearing: D1's futures are not `Send`, and
/// that is precisely why this type cannot be reached through the axum router.
struct D1Store {
    db: D1Database,
}

/// One row of either history table.
///
/// A single permissive shape serves both queries. Every field is optional
/// because the row decoder rejects fields the struct does not declare, so the
/// columns one projection omits would otherwise fail the other's decode.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct Row {
    start_epoch_minutes: Option<i64>,
    interval_minutes: Option<u16>,
    import_wh: Option<u64>,
    export_wh: Option<u64>,
    from_epoch_minutes: Option<i64>,
    to_epoch_minutes: Option<i64>,
    tariff_id: Option<String>,
    total_micro_usd: Option<i64>,
    total_import_wh: Option<u64>,
    created_at: Option<String>,
}

impl Row {
    /// Interpret this row as a reading, or discard it if the projection did
    /// not carry the fields a reading needs.
    fn into_reading(self) -> Option<Reading> {
        Some(Reading {
            start_epoch_minutes: self.start_epoch_minutes?,
            interval_minutes: self.interval_minutes?,
            import_wh: self.import_wh?,
            export_wh: self.export_wh.unwrap_or(0),
        })
    }

    /// Interpret this row as a stored bill, or discard it if the projection
    /// did not carry the fields a bill needs.
    fn into_bill(self) -> Option<StoredBill> {
        Some(StoredBill {
            from_epoch_minutes: self.from_epoch_minutes?,
            to_epoch_minutes: self.to_epoch_minutes?,
            tariff_id: self.tariff_id?,
            total_micro_usd: self.total_micro_usd?,
            total_import_wh: self.total_import_wh?,
            created_at: self.created_at?,
        })
    }
}

// D1 stores 64-bit integers as JavaScript numbers, so every bind converts
// through `f64` — the same conversion the `worker` crate's own D1 support uses.
#[async_trait(?Send)]
impl Store for D1Store {
    async fn insert_readings(
        &self,
        session: &SessionId,
        readings: &[Reading],
    ) -> Result<usize, String> {
        let statement = self.db.prepare(
            "INSERT OR IGNORE INTO interval_reading \
             (session_id, start_epoch_minutes, interval_minutes, import_wh, export_wh) \
             VALUES (?, ?, ?, ?, ?)",
        );
        let mut inserted = 0usize;
        for reading in readings {
            let bound = statement
                .clone()
                .bind(&[
                    JsValue::from_str(&session.0),
                    JsValue::from_f64(reading.start_epoch_minutes as f64),
                    JsValue::from_f64(f64::from(reading.interval_minutes)),
                    JsValue::from_f64(reading.import_wh as f64),
                    JsValue::from_f64(reading.export_wh as f64),
                ])
                .map_err(|error| error.to_string())?;
            let result = bound.run().await.map_err(|error| error.to_string())?;
            inserted += changed_rows(&result);
        }
        Ok(inserted)
    }

    async fn readings_between(
        &self,
        session: &SessionId,
        from: i64,
        to: i64,
    ) -> Result<Vec<Reading>, String> {
        let rows = self
            .db
            .prepare(
                "SELECT start_epoch_minutes, interval_minutes, import_wh, export_wh \
                 FROM interval_reading \
                 WHERE session_id = ? AND start_epoch_minutes >= ? AND start_epoch_minutes < ? \
                 ORDER BY start_epoch_minutes",
            )
            .bind(&[
                JsValue::from_str(&session.0),
                JsValue::from_f64(from as f64),
                JsValue::from_f64(to as f64),
            ])
            .map_err(|error| error.to_string())?
            .all()
            .await
            .map_err(|error| error.to_string())?
            .results::<Row>()
            .map_err(|error| error.to_string())?;
        Ok(rows.into_iter().filter_map(Row::into_reading).collect())
    }

    async fn record_bill(
        &self,
        session: &SessionId,
        from: i64,
        to: i64,
        tariff_id: &str,
        total_micro_usd: i64,
        total_import_wh: u128,
    ) -> Result<(), String> {
        let stored_wh = i64::try_from(total_import_wh).unwrap_or(i64::MAX);
        self.db
            .prepare(
                "INSERT INTO bill \
                 (session_id, from_epoch_minutes, to_epoch_minutes, tariff_id, total_micro_usd, \
                  total_import_wh) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&[
                JsValue::from_str(&session.0),
                JsValue::from_f64(from as f64),
                JsValue::from_f64(to as f64),
                JsValue::from_str(tariff_id),
                JsValue::from_f64(total_micro_usd as f64),
                JsValue::from_f64(stored_wh as f64),
            ])
            .map_err(|error| error.to_string())?
            .run()
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    async fn recent_bills(
        &self,
        session: &SessionId,
        limit: usize,
    ) -> Result<Vec<StoredBill>, String> {
        let rows = self
            .db
            .prepare(
                "SELECT from_epoch_minutes, to_epoch_minutes, tariff_id, total_micro_usd, \
                 total_import_wh, created_at \
                 FROM bill WHERE session_id = ? ORDER BY id DESC LIMIT ?",
            )
            .bind(&[
                JsValue::from_str(&session.0),
                JsValue::from_f64(limit as f64),
            ])
            .map_err(|error| error.to_string())?
            .all()
            .await
            .map_err(|error| error.to_string())?
            .results::<Row>()
            .map_err(|error| error.to_string())?;
        Ok(rows.into_iter().filter_map(Row::into_bill).collect())
    }

    async fn clear_session(&self, session: &SessionId) -> Result<usize, String> {
        let readings = self
            .delete_session_rows("DELETE FROM interval_reading WHERE session_id = ?", session)
            .await?;
        let bills = self
            .delete_session_rows("DELETE FROM bill WHERE session_id = ?", session)
            .await?;
        Ok(readings + bills)
    }
}

impl D1Store {
    /// Run one session-scoped delete and report how many rows it removed.
    ///
    /// The SQL is a literal at every call site, never assembled from input.
    async fn delete_session_rows(
        &self,
        sql: &'static str,
        session: &SessionId,
    ) -> Result<usize, String> {
        let result = self
            .db
            .prepare(sql)
            .bind(&[JsValue::from_str(&session.0)])
            .map_err(|error| error.to_string())?
            .run()
            .await
            .map_err(|error| error.to_string())?;
        Ok(changed_rows(&result))
    }
}

/// The number of rows a D1 statement changed, when the runtime reports it.
///
/// `changes` is two layers deep: D1 may omit the meta object, and may omit the
/// count inside it even when present.
fn changed_rows(result: &D1Result) -> usize {
    result
        .meta()
        .ok()
        .flatten()
        .and_then(|meta| meta.changes)
        .unwrap_or(0)
}

#[event(fetch)]
pub async fn fetch(
    mut req: worker::Request,
    env: Env,
    _ctx: Context,
) -> Result<http::Response<worker::Body>> {
    let path = req.path();
    let method = req.method();
    // Resolved before the body is consumed. The response carries the cookie, so
    // the next request from the same client lands in the same partition.
    let session = resolve_session(&req);
    let store: Option<Arc<dyn Store>> = env
        .d1("tide")
        .ok()
        .map(|db| Arc::new(D1Store { db }) as Arc<dyn Store>);

    let mut response = if path.starts_with("/api/history/") {
        let text = req.text().await.unwrap_or_default();
        let result = history_result(&path, &method, &text, store, &session).await;
        match result {
            Ok(value) => json_response(StatusCode::OK, value),
            Err(error) => json_response(error.status, error.to_json()),
        }
    } else {
        let http_req: worker::HttpRequest = req.try_into()?;
        let response = tide_api::router(AppState::new(), ()).call(http_req).await?;
        worker_response(response)
    };

    set_session_cookie(&mut response, &session);
    Ok(response)
}

/// Dispatch one `/api/history/*` request.
///
/// Returns JSON rather than an axum response because it is called from
/// `fetch`, where D1's `!Send` futures are legal.
async fn history_result(
    path: &str,
    method: &Method,
    body: &str,
    store: Option<Arc<dyn Store>>,
    session: &SessionId,
) -> Result<serde_json::Value, ApiError> {
    let Some(store) = store else {
        return Err(ApiError::unprocessable(
            "persistence is not configured for this deployment",
        ));
    };
    let store: &dyn Store = &*store;

    match (path, method) {
        ("/api/history/import", Method::Post) => {
            let request: HistoryImportRequest = serde_json::from_str(body)
                .map_err(|error| ApiError::bad_request(format!("{error}")))?;
            to_json(history_import(store, session, request).await?)
        }
        ("/api/history/compare", Method::Post) => {
            let request: HistoryCompareRequest = serde_json::from_str(body)
                .map_err(|error| ApiError::bad_request(format!("{error}")))?;
            let tariffs = tide_core::tariffs::bundled();
            to_json(history_compare(store, session, &tariffs, request).await?)
        }
        ("/api/history/bills", Method::Get) => to_json(history_bills(store, session).await?),
        ("/api/history/session", Method::Delete) => to_json(history_clear(store, session).await?),
        _ => Err(ApiError::not_found("no such route")),
    }
}

/// The cookie that carries a visitor's partition between requests.
const SESSION_COOKIE: &str = "tide_session";

/// An explicit override for callers that cannot hold a cookie, such as an
/// automated test pinning its own partition. This is **not authentication**:
/// anyone may present any id, and the id decides only which bucket their own
/// requests read and write. It must never be treated as an identity.
const SESSION_HEADER: &str = "x-tide-session";

/// Resolve the caller's session: explicit header first, then cookie, else a
/// freshly minted id.
///
/// A presented value that fails [`valid_session`] is replaced rather than
/// repaired, so a malformed cookie or header cannot silently collide with a
/// real session.
fn resolve_session(req: &worker::Request) -> SessionId {
    if let Ok(Some(presented)) = req.headers().get(SESSION_HEADER) {
        return if valid_session(&presented) {
            SessionId(presented)
        } else {
            SessionId(mint_session(req))
        };
    }
    if let Some(presented) = cookie_value(req, SESSION_COOKIE) {
        if valid_session(&presented) {
            return SessionId(presented);
        }
    }
    SessionId(mint_session(req))
}

/// Whether a presented id can be used as a partition key.
///
/// The rules keep the id safe to echo into a cookie header without quoting and
/// to bind as an opaque SQL string. They do not authenticate anyone: the
/// [`SESSION_HEADER`] override exists precisely so a caller can choose an id.
fn valid_session(id: &str) -> bool {
    // The reserved fallback bucket is addressable even though its name is
    // shorter than the minimum: without this the default bucket could never be
    // reached explicitly, so legacy rows in it would be invisible and
    // undeletable.
    if id == tide_api::SessionId::demo_name() {
        return true;
    }
    (8..=64).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// One cookie's value, if the request's `Cookie` header carries it.
///
/// Parsed by hand rather than with a cookie jar: one opaque key is all this
/// needs, and a full parser would be more dependency than the contract
/// justifies.
fn cookie_value(req: &worker::Request, name: &str) -> Option<String> {
    let header = req.headers().get("cookie").ok().flatten()?;
    header.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == name).then(|| value.trim().to_owned())
    })
}

/// Mint a fresh session id: 32 hex characters.
///
/// This is a partition key, not a security token. Uniqueness matters — two
/// callers must not share a bucket by accident — while unpredictability does
/// not, so a process counter mixed with the request's `cf-ray` or a
/// millisecond clock is sufficient. A random-number crate would add a
/// dependency for a property this value does not need.
fn mint_session(req: &worker::Request) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let tick = COUNTER.fetch_add(1, Ordering::Relaxed);
    let ray = req.headers().get("cf-ray").ok().flatten();
    let millis = worker::Date::now().as_millis();
    let seed = format!("{}|{tick}|{millis}", ray.as_deref().unwrap_or(""));

    // FNV-1a over the seed, then splitmix64 to spread the one 64-bit value
    // into the two halves of the id.
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in seed.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!(
        "{:016x}{:016x}",
        splitmix64(hash),
        splitmix64(hash ^ 0x9e37_79b9_7f4a_7c15)
    )
}

/// The splitmix64 finaliser: a cheap, well-mixed permutation of 64 bits.
fn splitmix64(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Attach the session cookie to a response.
///
/// No `HttpOnly`: the cookie holds a partition key, not a credential, so
/// hiding it from scripts buys nothing, and leaving it readable lets a
/// developer see which partition they are in. `SameSite=Lax` keeps a
/// cross-site form post from writing into the visitor's partition.
fn set_session_cookie(response: &mut http::Response<worker::Body>, session: &SessionId) {
    let cookie = format!(
        "{SESSION_COOKIE}={}; Path=/; Max-Age=31536000; SameSite=Lax",
        session.0
    );
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
}

/// Serialise one of the history responses.
///
/// The response types are plain structs of numbers and strings, so a failure
/// here would be a bug in a hand-written `Serialize`; it is reported inside the
/// normal error envelope rather than panicking in a request.
fn to_json<T: serde::Serialize>(value: T) -> Result<serde_json::Value, ApiError> {
    serde_json::to_value(value).map_err(|error| ApiError::unprocessable(format!("{error}")))
}

/// A JSON response with the given status.
fn json_response(status: StatusCode, value: serde_json::Value) -> http::Response<worker::Body> {
    let mut response = http::Response::new(json_body(value));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

/// Encode a JSON value as a worker body.
fn json_body(value: serde_json::Value) -> worker::Body {
    // `Value`'s `Display` is its canonical compact JSON, and cannot fail.
    let bytes = value.to_string().into_bytes();
    worker_body(futures_util::stream::iter([Ok::<Vec<u8>, Error>(bytes)]))
}

/// Retype an axum response body for the worker's HTTP adapter.
fn worker_response(response: http::Response<axum::body::Body>) -> http::Response<worker::Body> {
    let (parts, body) = response.into_parts();
    http::Response::from_parts(parts, worker_body(body.into_data_stream()))
}

/// Wrap a Rust stream as a worker body.
///
/// `worker::Body` wraps a JavaScript `ReadableStream`, so a stream is the only
/// construction path the crate offers. `from_stream` has no failing path in
/// 0.8.7, but its signature is fallible; an empty body is a quieter fallback
/// than a panic in a request path.
fn worker_body<S>(stream: S) -> worker::Body
where
    S: futures_util::TryStream + 'static,
    S::Ok: Into<Vec<u8>>,
    S::Error: std::fmt::Debug,
{
    worker::Body::from_stream(stream).unwrap_or_else(|_| worker::Body::empty())
}
