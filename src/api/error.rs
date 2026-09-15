//! API error type. Database errors are logged server-side and rendered to
//! the client as an opaque 500 -- `sqlx::Error`'s `Display` includes table
//! and column names and, for constraint violations, row data.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Errors returned by API handlers, rendered as `{"error": "..."}` with an
/// appropriate status code.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApiError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    /// The requested data source has not been loaded into this namespace.
    #[error("{0}")]
    NotLoaded(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::Database(e) => {
                tracing::error!(error = %e, "database error while serving request");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal database error".to_string(),
                )
            }
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::NotLoaded(msg) => (StatusCode::SERVICE_UNAVAILABLE, msg),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

/// Maps "relation does not exist" (SQLSTATE 42P01) for `relation` to a
/// friendly 503, and everything else to a 500.
pub fn missing_relation(e: sqlx::Error, relation: &str, hint: &str) -> ApiError {
    match &e {
        sqlx::Error::Database(db)
            if db.code().as_deref() == Some("42P01") && db.message().contains(relation) =>
        {
            ApiError::NotLoaded(format!("{relation} is not available: {hint}"))
        }
        _ => ApiError::Database(e),
    }
}
