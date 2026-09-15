use axum::Json;
use axum::extract::{Query, State};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::api::error::ApiError;
use crate::api::pagination::{Pagination, cycle_param};

/// A row from Schedule B (itemized operating expenditures), sourced from
/// the FEC's `oppexp.txt` bulk file.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Disbursement {
    pub sub_id: i64,
    pub cycle: i32,
    pub cmte_id: Option<String>,
    pub name: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub zip_code: Option<String>,
    /// The FEC's raw date string, exactly as shipped (`MM/DD/YYYY` in this file).
    pub transaction_dt: Option<String>,
    /// The same date parsed; `null` if the raw value was blank or invalid.
    pub transaction_date: Option<NaiveDate>,
    /// Exact `Decimal`, never cast through `::float8`.
    pub transaction_amt: Option<Decimal>,
    pub purpose: Option<String>,
    pub category: Option<String>,
    pub category_desc: Option<String>,
    pub entity_tp: Option<String>,
    pub memo_cd: Option<String>,
    pub memo_text: Option<String>,
    pub file_num: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub cmte_id: Option<String>,
    pub cycle: Option<i32>,
    /// Case-insensitive substring match against the payee `name`.
    pub name: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    /// Case-insensitive substring match against `purpose`.
    pub purpose: Option<String>,
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
) -> Result<Json<Vec<Disbursement>>, ApiError> {
    let page = Pagination::new(params.limit, params.offset).validate()?;
    let cycle = cycle_param(params.cycle)?;
    let rows = sqlx::query_as::<_, Disbursement>(
        "SELECT sub_id, cycle, cmte_id, name, city, state, zip_code, transaction_dt, \
                transaction_date, transaction_amt, purpose, category, category_desc, \
                entity_tp, memo_cd, memo_text, file_num \
         FROM disbursements \
         WHERE ($1::text IS NULL OR cmte_id = $1) \
           AND ($2::int IS NULL OR cycle = $2) \
           AND ($3::text IS NULL OR name ILIKE '%' || $3 || '%') \
           AND ($4::text IS NULL OR city ILIKE $4) \
           AND ($5::text IS NULL OR state = $5) \
           AND ($6::text IS NULL OR purpose ILIKE '%' || $6 || '%') \
           AND ($7::numeric IS NULL OR transaction_amt >= $7) \
           AND ($8::numeric IS NULL OR transaction_amt <= $8) \
           AND ($9::date IS NULL OR transaction_date >= $9) \
           AND ($10::date IS NULL OR transaction_date <= $10) \
         ORDER BY transaction_date DESC NULLS LAST, sub_id DESC \
         LIMIT $11 OFFSET $12",
    )
    .bind(params.cmte_id)
    .bind(cycle)
    .bind(params.name)
    .bind(params.city)
    .bind(params.state.map(|s| s.to_uppercase()))
    .bind(params.purpose)
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
