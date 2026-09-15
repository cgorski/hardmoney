//! Operational status routes: liveness and what data this namespace holds.

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use sqlx::PgPool;

use crate::api::error::ApiError;

/// One row of `loads`: a recorded bulk load.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct LoadRecord {
    pub load_id: i64,
    pub source: String,
    pub cycle: Option<i32>,
    pub loaded_at: chrono::DateTime<chrono::Utc>,
    pub mode: String,
    pub row_count: i64,
    pub row_limit: Option<i64>,
    pub dates_nulled: i64,
    pub source_etag: Option<String>,
    pub source_last_modified: Option<chrono::DateTime<chrono::Utc>>,
    pub hardmoney_version: String,
}

#[derive(Debug, Serialize)]
pub struct SchemaStatus {
    pub hardmoney_version: &'static str,
    pub namespace: String,
    pub migrations_applied: Vec<i64>,
    pub migrations_pending: Vec<i64>,
    /// The most recent load per (source, cycle).
    pub loads: Vec<LoadRecord>,
    pub independent_expenditures_available: bool,
}

pub async fn schema(State(pool): State<PgPool>) -> Result<Json<SchemaStatus>, ApiError> {
    let status = crate::db::migration_status(&pool).await?;
    let (namespace,): (String,) = sqlx::query_as("SELECT current_schema()::text")
        .fetch_one(&pool)
        .await?;
    let loads = sqlx::query_as::<_, LoadRecord>(
        "SELECT DISTINCT ON (source, cycle) \
                load_id, source, cycle, loaded_at, mode, row_count, row_limit, dates_nulled, \
                source_etag, source_last_modified, hardmoney_version \
         FROM loads ORDER BY source, cycle, loaded_at DESC",
    )
    .fetch_all(&pool)
    .await
    .or_else(|e| match &e {
        // Namespace has never been migrated: no `loads` table yet.
        sqlx::Error::Database(db) if db.code().as_deref() == Some("42P01") => Ok(vec![]),
        _ => Err(e),
    })?;
    let (ie,): (bool,) =
        sqlx::query_as("SELECT to_regclass('independent_expenditures') IS NOT NULL")
            .fetch_one(&pool)
            .await?;
    Ok(Json(SchemaStatus {
        hardmoney_version: env!("CARGO_PKG_VERSION"),
        namespace,
        migrations_applied: status.applied,
        migrations_pending: status.pending,
        loads,
        independent_expenditures_available: ie,
    }))
}

pub async fn health() -> &'static str {
    "ok"
}
