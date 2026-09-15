//! A read-only REST API over the tables `bulk` populates.
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use hardmoney::db::{DbConfig, connect, migrate};
//! use hardmoney::api::{ApiConfig, serve};
//!
//! let pool = connect(&DbConfig::new("postgres://user@localhost/fec")).await?;
//! migrate(&pool).await?;
//! serve(pool, ApiConfig::new("0.0.0.0:8080".parse()?)).await?;
//! # Ok(())
//! # }
//! ```
//!
//! | Route | Description |
//! |---|---|
//! | `GET /health` | liveness check |
//! | `GET /schema` | namespace, migration state, and the latest load per source/cycle |
//! | `GET /candidates` | list, filter by `cycle`/`state`/`office`/`q` |
//! | `GET /candidates/{cand_id}` | one candidate across cycles |
//! | `GET /committees` | list, filter by `cycle`/`cmte_tp`/`q` |
//! | `GET /committees/{cmte_id}` | one committee across cycles |
//! | `GET /schedule-a` | itemized contributions; `cmte_id`, `cycle`, `name`, `employer`, `occupation`, `state`, `zip_code`, `min_amount`, `max_amount`, `min_date`, `max_date` |
//! | `GET /disbursements` | Schedule B; `cmte_id`, `cycle`, `name`, `city`, `state`, `purpose`, `min_amount`, `max_amount`, `min_date`, `max_date` |
//! | `GET /independent-expenditures` | from the FEC's own Schedule E pg_dump; `candidate_id`/`cmte_id`/`support_oppose_code` |
//! | `GET /filings/{filing_id}` | a directly-ingested `.fec` filing's header/summary |
//! | `GET /filings/{filing_id}/schedule-e` | that filing's own Schedule E line items |
//!
//! All list routes accept `limit` (default 50, max 500) and `offset`.
//!
//! # Hardening
//!
//! [`ApiConfig`] controls request timeouts, CORS origins, an optional
//! static API key (`X-Api-Key` header or `?api_key=`), and a request body
//! cap. Defaults are suitable for local development (permissive CORS, no
//! key); set an allow-list and a key before exposing the server publicly.
//! Database errors are never echoed to clients (see [`error::ApiError`]).

pub mod error;
pub mod pagination;
pub mod routes;

use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use sqlx::PgPool;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

pub use error::ApiError;

/// Server configuration.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ApiConfig {
    pub bind: SocketAddr,
    /// Wall-clock cap per request; a slow query returns 408.
    pub request_timeout: Duration,
    /// Allowed CORS origins. Empty means permissive (any origin).
    pub cors_origins: Vec<String>,
    /// If set, every request must carry this value in `X-Api-Key` or
    /// `?api_key=`. `/health` is always open.
    pub api_key: Option<String>,
    /// Maximum request body size in bytes (the API is read-only, so this
    /// only bounds abuse).
    pub max_body_bytes: usize,
}

impl ApiConfig {
    pub fn new(bind: SocketAddr) -> Self {
        Self {
            bind,
            request_timeout: Duration::from_secs(30),
            cors_origins: Vec::new(),
            api_key: None,
            max_body_bytes: 64 * 1024,
        }
    }

    pub fn request_timeout(mut self, d: Duration) -> Self {
        self.request_timeout = d;
        self
    }

    pub fn cors_origins(mut self, origins: Vec<String>) -> Self {
        self.cors_origins = origins;
        self
    }

    pub fn api_key(mut self, key: Option<String>) -> Self {
        self.api_key = key.filter(|k| !k.is_empty());
        self
    }
}

/// Builds the full [`Router`], with `pool` as shared state.
pub fn router(pool: PgPool, config: &ApiConfig) -> Router {
    let cors = if config.cors_origins.is_empty() {
        CorsLayer::permissive()
    } else {
        let origins: Vec<HeaderValue> = config
            .cors_origins
            .iter()
            .filter_map(|o| HeaderValue::from_str(o).ok())
            .collect();
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods([axum::http::Method::GET, axum::http::Method::HEAD])
    };

    let protected = Router::new()
        .route("/schema", get(routes::status::schema))
        .route("/candidates", get(routes::candidates::list))
        .route("/candidates/{cand_id}", get(routes::candidates::get))
        .route("/committees", get(routes::committees::list))
        .route("/committees/{cmte_id}", get(routes::committees::get))
        .route("/schedule-a", get(routes::schedule_a::search))
        .route("/disbursements", get(routes::disbursements::search))
        .route(
            "/independent-expenditures",
            get(routes::independent_expenditures::search),
        )
        .route("/filings/{filing_id}", get(routes::filings::get))
        .route(
            "/filings/{filing_id}/schedule-e",
            get(routes::filings::schedule_e),
        )
        .layer(middleware::from_fn_with_state(
            config.api_key.clone(),
            require_api_key,
        ));

    Router::new()
        .route("/health", get(routes::status::health))
        .merge(protected)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            config.request_timeout,
        ))
        .layer(RequestBodyLimitLayer::new(config.max_body_bytes))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(pool)
}

/// Rejects requests lacking the configured key. No-op when no key is set.
async fn require_api_key(
    axum::extract::State(expected): axum::extract::State<Option<String>>,
    req: Request,
    next: Next,
) -> Response {
    let Some(expected) = expected else {
        return next.run(req).await;
    };
    let from_header = req
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let from_query = req.uri().query().and_then(|q| {
        q.split('&')
            .find_map(|kv| kv.strip_prefix("api_key=").map(str::to_string))
    });
    let presented = from_header.or(from_query);
    if presented
        .as_deref()
        .is_some_and(|k| constant_time_eq(k, &expected))
    {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "missing or invalid API key" })),
        )
            .into_response()
    }
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Runs the API until the process receives SIGINT/SIGTERM.
pub async fn serve(pool: PgPool, config: ApiConfig) -> std::io::Result<()> {
    let app = router(pool, &config);
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(
        bind = %config.bind,
        cors = if config.cors_origins.is_empty() { "permissive" } else { "allow-list" },
        api_key = config.api_key.is_some(),
        "hardmoney API listening"
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_basic() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn api_key_empty_string_means_none() {
        let c = ApiConfig::new("127.0.0.1:0".parse().unwrap()).api_key(Some(String::new()));
        assert!(c.api_key.is_none());
    }
}
