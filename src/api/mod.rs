//! A read-only REST API over the tables `bulk` populates.
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let pool = sqlx::PgPool::connect("postgres://localhost/fec").await?;
//! hardmoney::db::ensure_schema(&pool).await?;
//! let addr = "0.0.0.0:8080".parse().unwrap();
//! hardmoney::api::serve(pool, addr).await?;
//! # Ok(())
//! # }
//! ```
//!
//! | Route | Description |
//! |---|---|
//! | `GET /health` | liveness check |
//! | `GET /candidates` | list, filter by `cycle`/`state`/`office`/`q` |
//! | `GET /candidates/:cand_id` | one candidate across cycles |
//! | `GET /committees` | list, filter by `cycle`/`cmte_tp`/`q` |
//! | `GET /committees/:cmte_id` | one committee across cycles |
//! | `GET /schedule-a` | itemized contributions, filter by `cmte_id`/`cycle`/`name`/`employer`/`min_amount` |
//! | `GET /disbursements` | itemized disbursements (Schedule B), same filter shape |
//! | `GET /independent-expenditures` | from the FEC's own Schedule E pg_dump, filter by `candidate_id`/`cmte_id`/`support_oppose_code` |
//! | `GET /filings/:filing_id` | a directly-ingested `.fec` filing's header/summary |
//! | `GET /filings/:filing_id/schedule-e` | that filing's own Schedule E line items |

pub mod error;
pub mod pagination;
pub mod routes;

use std::net::SocketAddr;

use axum::routing::get;
use axum::Router;
use sqlx::PgPool;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

/// Builds the full [`Router`], with `pool` as shared state.
pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/candidates", get(routes::candidates::list))
        .route("/candidates/{cand_id}", get(routes::candidates::get))
        .route("/committees", get(routes::committees::list))
        .route("/committees/{cmte_id}", get(routes::committees::get))
        .route("/schedule-a", get(routes::schedule_a::search))
        .route("/disbursements", get(routes::disbursements::search))
        .route("/independent-expenditures", get(routes::independent_expenditures::search))
        .route("/filings/{filing_id}", get(routes::filings::get))
        .route("/filings/{filing_id}/schedule-e", get(routes::filings::schedule_e))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(pool)
}

/// Runs the API to completion (i.e. until the process is killed) bound to
/// `addr`.
pub async fn serve(pool: PgPool, addr: SocketAddr) -> std::io::Result<()> {
    let app = router(pool);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("hardmoney API listening on {addr}");
    axum::serve(listener, app).await
}
