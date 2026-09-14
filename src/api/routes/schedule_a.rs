use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::api::error::ApiError;
use crate::api::pagination::Pagination;

/// A row from Schedule A (itemized individual contributions to a
/// committee), sourced from the FEC's `indiv.txt` bulk file.
#[derive(Serialize, sqlx::FromRow)]
pub struct ScheduleAEntry {
    pub sub_id: i64,
    pub cycle: i32,
    pub cmte_id: Option<String>,
    pub name: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub employer: Option<String>,
    pub occupation: Option<String>,
    pub transaction_dt: Option<String>,
    pub transaction_amt: Option<f64>,
    pub transaction_tp: Option<String>,
    pub memo_text: Option<String>,
}

#[derive(Deserialize)]
pub struct SearchParams {
    pub cmte_id: Option<String>,
    pub cycle: Option<i32>,
    /// Case-insensitive substring match against the contributor `name`.
    pub name: Option<String>,
    pub employer: Option<String>,
    pub min_amount: Option<f64>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn search(State(pool): State<PgPool>, Query(params): Query<SearchParams>) -> Result<Json<Vec<ScheduleAEntry>>, ApiError> {
    let page = Pagination::new(params.limit, params.offset);
    let rows = sqlx::query_as::<_, ScheduleAEntry>(
        "SELECT sub_id, cycle, cmte_id, name, city, state, employer, occupation, \
                transaction_dt, transaction_amt::float8 AS transaction_amt, transaction_tp, memo_text \
         FROM schedule_a \
         WHERE ($1::text IS NULL OR cmte_id = $1) \
           AND ($2::int IS NULL OR cycle = $2) \
           AND ($3::text IS NULL OR name ILIKE '%' || $3 || '%') \
           AND ($4::text IS NULL OR employer ILIKE '%' || $4 || '%') \
           AND ($5::float8 IS NULL OR transaction_amt >= $5) \
         ORDER BY transaction_dt DESC NULLS LAST \
         LIMIT $6 OFFSET $7",
    )
    .bind(params.cmte_id)
    .bind(params.cycle)
    .bind(params.name)
    .bind(params.employer)
    .bind(params.min_amount)
    .bind(page.limit())
    .bind(page.offset())
    .fetch_all(&pool)
    .await?;
    Ok(Json(rows))
}
