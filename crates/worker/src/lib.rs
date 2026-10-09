//! Cloudflare Worker entrypoint for Tide.
//!
//! The same [`tide_api::router`] that runs in tests runs here, on
//! `wasm32-unknown-unknown`, behind the Workers `http` adapter. There is
//! exactly one router definition in this repository.
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

use tide_api::{router, AppState};
use tower_service::Service as _;
use worker::*;

#[event(fetch)]
pub async fn fetch(
    req: HttpRequest,
    _env: Env,
    _ctx: Context,
) -> Result<http::Response<axum::body::Body>> {
    Ok(router(AppState::new()).call(req).await?)
}
