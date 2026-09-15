use axum::Json;
use axum::extract::{Query, State};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::api::error::ApiError;
use crate::api::pagination::{Pagination, cycle_param};

/// A row from Schedule A (itemized individual contributions to a
/// committee), sourced from the FEC's `indiv.txt` bulk file.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ScheduleAEntry {
    pub sub_id: i64,
    pub cycle: i32,
    pub cmte_id: Option<String>,
    pub name: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub zip_code: Option<String>,
    pub employer: Option<String>,
    pub occupation: Option<String>,
    /// The FEC's raw date string, exactly as shipped (`MMDDYYYY`).
    pub transaction_dt: Option<String>,
    /// The same date parsed; `null` if the raw value was blank or invalid.
    pub transaction_date: Option<NaiveDate>,
    /// Read straight from the `NUMERIC` column as an exact `Decimal` --
    /// never cast through `::float8`.
    pub transaction_amt: Option<Decimal>,
    pub transaction_tp: Option<String>,
    pub entity_tp: Option<String>,
    pub memo_cd: Option<String>,
    pub memo_text: Option<String>,
    pub file_num: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub cmte_id: Option<String>,
    pub cycle: Option<i32>,
    /// Case-insensitive substring match against the contributor `name`.
    pub name: Option<String>,
    pub employer: Option<String>,
    pub occupation: Option<String>,
    pub state: Option<String>,
    pub zip_code: Option<String>,
    pub min_amount: Option<Decimal>,
    pub max_amount: Option<Decimal>,
    pub min_date: Option<NaiveDate>,
    pub max_date: Option<NaiveDate>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn search(
    State(pool): State<PgPool>,
    Query(params): Query<SearchParams>,
) -> Result<Json<Vec<ScheduleAEntry>>, ApiError> {
    let page = Pagination::new(params.limit, params.offset);
    let cycle = cycle_param(params.cycle)?;
    let rows = sqlx::query_as::<_, ScheduleAEntry>(
        "SELECT sub_id, cycle, cmte_id, name, city, state, zip_code, employer, occupation, \
                transaction_dt, transaction_date, transaction_amt, transaction_tp, entity_tp, \
                memo_cd, memo_text, file_num \
         FROM schedule_a \
         WHERE ($1::text IS NULL OR cmte_id = $1) \
           AND ($2::int IS NULL OR cycle = $2) \
           AND ($3::text IS NULL OR name ILIKE '%' || $3 || '%') \
           AND ($4::text IS NULL OR employer ILIKE '%' || $4 || '%') \
           AND ($5::text IS NULL OR occupation ILIKE '%' || $5 || '%') \
           AND ($6::text IS NULL OR state = $6) \
           AND ($7::text IS NULL OR zip_code LIKE $7 || '%') \
           AND ($8::numeric IS NULL OR transaction_amt >= $8) \
           AND ($9::numeric IS NULL OR transaction_amt <= $9) \
           AND ($10::date IS NULL OR transaction_date >= $10) \
           AND ($11::date IS NULL OR transaction_date <= $11) \
         ORDER BY transaction_date DESC NULLS LAST, sub_id DESC \
         LIMIT $12 OFFSET $13",
    )
    .bind(params.cmte_id)
    .bind(cycle)
    .bind(params.name)
    .bind(params.employer)
    .bind(params.occupation)
    .bind(params.state.map(|s| s.to_uppercase()))
    .bind(params.zip_code)
    .bind(params.min_amount)
    .bind(params.max_amount)
    .bind(params.min_date)
    .bind(params.max_date)
    .bind(page.limit())
    .bind(page.offset())
    .fetch_all(&pool)
    .await?;
    Ok(Json(rows))
}
