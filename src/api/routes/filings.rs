use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use sqlx::PgPool;

use crate::api::error::ApiError;

/// A raw `.fec` filing previously ingested via `hardmoney bulk load-filing`
/// (which uses the `parser` module directly against the filing's own
/// bytes, not a bulk CSV derivation).
#[derive(Serialize, sqlx::FromRow)]
pub struct Filing {
    pub filing_id: i64,
    pub form_type: String,
    pub fec_version: Option<String>,
    pub committee_id: Option<String>,
    pub is_amendment: bool,
    pub amends_filing_id: Option<i64>,
    pub header: Option<serde_json::Value>,
    pub summary: Option<serde_json::Value>,
}

pub async fn get(
    State(pool): State<PgPool>,
    Path(filing_id): Path<i64>,
) -> Result<Json<Filing>, ApiError> {
    let row = sqlx::query_as::<_, Filing>(
        "SELECT filing_id, form_type, fec_version, committee_id, is_amendment, amends_filing_id, header, summary \
         FROM filings WHERE filing_id = $1",
    )
    .bind(filing_id)
    .fetch_optional(&pool)
    .await?;
    row.map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("no filing with filing_id {filing_id}")))
}

/// A Schedule E line item extracted directly from a filing's own bytes by
/// the `parser` module -- the precise (not bulk-aggregated) counterpart
/// to `independent_expenditures`.
#[derive(Serialize, sqlx::FromRow)]
pub struct ScheduleELine {
    pub line_index: i32,
    pub payee_name: Option<String>,
    pub expenditure_amt: Option<f64>,
    pub expenditure_date: Option<chrono::NaiveDate>,
    pub support_oppose_code: Option<String>,
    pub candidate_id: Option<String>,
    pub candidate_name: Option<String>,
    pub candidate_office_state: Option<String>,
}

pub async fn schedule_e(
    State(pool): State<PgPool>,
    Path(filing_id): Path<i64>,
) -> Result<Json<Vec<ScheduleELine>>, ApiError> {
    let rows = sqlx::query_as::<_, ScheduleELine>(
        "SELECT line_index, payee_name, expenditure_amt::float8 AS expenditure_amt, expenditure_date, \
                support_oppose_code, candidate_id, candidate_name, candidate_office_state \
         FROM schedule_e_lines WHERE filing_id = $1 ORDER BY line_index",
    )
    .bind(filing_id)
    .fetch_all(&pool)
    .await?;
    Ok(Json(rows))
}
