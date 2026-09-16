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
//! | `GET /filings` | list directly-ingested filings; `committee_id`, `most_recent`, `form_type` filters |
//! | `GET /filings/{filing_id}` | a directly-ingested `.fec` filing's header/summary and amendment chain |
//! | `GET /filings/{filing_id}/schedule-e` | that filing's own Schedule E line items |
//! | `GET /filings/{filing_id}/processing` | how far the FEC has processed the filing, asked of openFEC live (needs an API key on the server; 503 without one; no database row required) |
//!
//! All list routes accept `limit` (default 50, max 500) and `offset`.
//!
//! With [`ApiConfig::ui`] set (`hardmoney serve --ui`) the server also
//! mounts the browser UI at `/ui` ([`crate::ui`]) and the database-free
//! filing tools at `/tools/*` ([`routes::tools`]).
//!
//! # Hardening
//!
//! [`ApiConfig`] controls request timeouts, CORS origins, an optional
//! static API key (`X-Api-Key` header or `?api_key=`), and a request body
//! cap. Defaults are suitable for local development (permissive CORS, no
//! key); set an allow-list and a key before exposing the server publicly.
//! Database errors are never echoed to clients (see [`error::ApiError`]),
//! and the request log records `?api_key=` as `api_key=REDACTED`.
//!
//! # FEC hosts
//!
//! [`ApiConfig::endpoints`] says where the FEC's document store is: the
//! `fec_url` on every `/filings` response is built from its
//! `docquery_base`, and `GET /tools/fetch/{id}` downloads from there. The
//! default is production; `hardmoney serve --docquery-base` (or
//! `HARDMONEY_DOCQUERY_BASE`) points a deployment at a mirror.
//! `/filings/{id}/processing` reads its openFEC host from the environment
//! at request time ([`crate::fec::openfec::OpenFec::from_env`]).

pub mod error;
pub mod pagination;
pub mod routes;

use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::extract::{Extension, Request};
use axum::http::{HeaderValue, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use sqlx::PgPool;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::fec::Endpoints;

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
    /// Maximum request body size in bytes. Without the UI the API is
    /// read-only, so [`ApiConfig::DEFAULT_MAX_BODY_BYTES`] only bounds
    /// abuse; with it, this is also the largest filing `POST /tools/parse`
    /// accepts, and [`ApiConfig::ui`] raises it to
    /// [`ApiConfig::UI_MAX_BODY_BYTES`] unless it was set explicitly.
    pub max_body_bytes: usize,
    /// Serve the browser UI at `/ui` and the filing tools at `/tools/*`.
    /// Off by default.
    pub ui: bool,
    /// Where the FEC's hosts are. `docquery_base` is what the `fec_url` of
    /// every `/filings` row is built from and where `GET /tools/fetch/{id}`
    /// downloads. Production by default; [`ApiConfig::new`] does not read
    /// the environment, so pass [`Endpoints::from_env`] to honour
    /// `HARDMONEY_DOCQUERY_BASE`.
    pub endpoints: Endpoints,
}

impl ApiConfig {
    /// The body cap without the UI: 64 KiB.
    pub const DEFAULT_MAX_BODY_BYTES: usize = 64 * 1024;
    /// The body cap [`ApiConfig::ui`] applies: 32 MiB, enough for every
    /// periodic report short of the largest presidential filings, which
    /// belong in the CLI anyway.
    pub const UI_MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

    pub fn new(bind: SocketAddr) -> Self {
        Self {
            bind,
            request_timeout: Duration::from_secs(30),
            cors_origins: Vec::new(),
            api_key: None,
            max_body_bytes: Self::DEFAULT_MAX_BODY_BYTES,
            ui: false,
            endpoints: Endpoints::default(),
        }
    }

    /// Sets [`ApiConfig::endpoints`].
    #[must_use]
    pub fn endpoints(mut self, endpoints: Endpoints) -> Self {
        self.endpoints = endpoints;
        self
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

    /// Enables the browser UI and the `/tools/*` routes. Because those
    /// routes accept whole filings, `max_body_bytes` is raised from
    /// [`ApiConfig::DEFAULT_MAX_BODY_BYTES`] to
    /// [`ApiConfig::UI_MAX_BODY_BYTES`] if it is still at the default; a
    /// value set explicitly is kept.
    pub fn ui(mut self, on: bool) -> Self {
        self.ui = on;
        if on && self.max_body_bytes == Self::DEFAULT_MAX_BODY_BYTES {
            self.max_body_bytes = Self::UI_MAX_BODY_BYTES;
        }
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
        let mut methods = vec![axum::http::Method::GET, axum::http::Method::HEAD];
        if config.ui {
            methods.push(axum::http::Method::POST);
        }
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods(methods)
    };

    let mut protected = Router::new()
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
        .route("/filings", get(routes::filings::list))
        .route("/filings/{filing_id}", get(routes::filings::get))
        .route(
            "/filings/{filing_id}/schedule-e",
            get(routes::filings::schedule_e),
        )
        .route(
            "/filings/{filing_id}/processing",
            get(routes::filings::processing),
        );
    if config.ui {
        protected = protected.merge(routes::tools::router(routes::tools::Limits {
            max_body_bytes: config.max_body_bytes,
        }));
    }
    let protected = protected
        .layer(middleware::from_fn_with_state(
            config.api_key.clone(),
            require_api_key,
        ))
        // The routes that build or fetch a document-store URL read this.
        .layer(Extension(config.endpoints.clone()));

    let mut app = Router::new()
        .route("/health", get(routes::status::health))
        .merge(protected);
    if config.ui {
        // The shell and its assets must load before the user can enter a
        // key, so they sit outside the key middleware.
        app = app.merge(crate::ui::router());
    }
    app.layer(TimeoutLayer::with_status_code(
        StatusCode::REQUEST_TIMEOUT,
        config.request_timeout,
    ))
    .layer(RequestBodyLimitLayer::new(config.max_body_bytes))
    .layer(TraceLayer::new_for_http().make_span_with(request_span))
    .layer(cors)
    .with_state(pool)
}

/// The per-request tracing span: what `TraceLayer`'s default records
/// (method, URI, HTTP version), except that an `api_key` query value is
/// replaced with `REDACTED`, so `RUST_LOG=debug` never writes the key.
fn request_span<B>(request: &axum::http::Request<B>) -> tracing::Span {
    tracing::debug_span!(
        "request",
        method = %request.method(),
        uri = %redact_api_key(request.uri()),
        version = ?request.version(),
    )
}

/// `uri` as a string with the value of any `api_key` query parameter
/// replaced by `REDACTED`. Only whole parameters named `api_key` are
/// touched (`x_api_key=` is left alone).
fn redact_api_key(uri: &Uri) -> String {
    let Some(query) = uri.query() else {
        return uri.to_string();
    };
    if !query.contains("api_key=") {
        return uri.to_string();
    }
    let redacted: Vec<&str> = query
        .split('&')
        .map(|kv| {
            if kv.starts_with("api_key=") {
                "api_key=REDACTED"
            } else {
                kv
            }
        })
        .collect();
    let mut out = uri.path().to_string();
    out.push('?');
    out.push_str(&redacted.join("&"));
    out
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
        ui = config.ui,
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
    fn request_log_redacts_the_api_key_query_value() {
        let uri: Uri = "/candidates?limit=1&api_key=s3cret&cycle=2026"
            .parse()
            .unwrap();
        assert_eq!(
            redact_api_key(&uri),
            "/candidates?limit=1&api_key=REDACTED&cycle=2026"
        );
        let uri: Uri = "/candidates?api_key=s3cret".parse().unwrap();
        assert_eq!(redact_api_key(&uri), "/candidates?api_key=REDACTED");
        let uri: Uri = "/candidates?x_api_key=keep&limit=2".parse().unwrap();
        assert_eq!(redact_api_key(&uri), "/candidates?x_api_key=keep&limit=2");
        let uri: Uri = "/health".parse().unwrap();
        assert_eq!(redact_api_key(&uri), "/health");
    }

    #[test]
    fn api_key_empty_string_means_none() {
        let c = ApiConfig::new("127.0.0.1:0".parse().unwrap()).api_key(Some(String::new()));
        assert!(c.api_key.is_none());
    }

    #[test]
    fn endpoints_default_to_production_and_can_be_set() {
        let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
        assert!(ApiConfig::new(bind).endpoints.is_default());
        let mirror = Endpoints::default()
            .with_docquery_base(crate::fec::Url::parse("https://mirror.example.gov/dq").unwrap());
        let c = ApiConfig::new(bind).endpoints(mirror.clone());
        assert_eq!(c.endpoints, mirror);
        assert_eq!(
            c.endpoints.docquery_filing_prefix(),
            "https://mirror.example.gov/dq/dcdev/posted/"
        );
    }

    #[test]
    fn ui_raises_the_default_body_cap_but_keeps_an_explicit_one() {
        let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let off = ApiConfig::new(bind);
        assert!(!off.ui);
        assert_eq!(off.max_body_bytes, ApiConfig::DEFAULT_MAX_BODY_BYTES);

        let on = ApiConfig::new(bind).ui(true);
        assert!(on.ui);
        assert_eq!(on.max_body_bytes, ApiConfig::UI_MAX_BODY_BYTES);

        let mut explicit = ApiConfig::new(bind);
        explicit.max_body_bytes = 1_000;
        let explicit = explicit.ui(true);
        assert_eq!(explicit.max_body_bytes, 1_000);

        assert!(!ApiConfig::new(bind).ui(false).ui);
    }
}
