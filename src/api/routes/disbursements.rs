use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::api::error::ApiError;
use crate::api::pagination::Pagination;

/// A row from Schedule B (itemized operating expenditures), sourced from
/// the FEC's `oppexp.txt` bulk file.
#[derive(Serialize, sqlx::FromRow)]
pub struct Disbursement {
    pub sub_id: i64,
    pub cycle: i32,
    pub cmte_id: Option<String>,
    pub name: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub transaction_dt: Option<String>,
    pub transaction_amt: Option<f64>,
    pub purpose: Option<String>,
    pub category_desc: Option<String>,
    pub memo_text: Option<String>,
}

#[derive(Deserialize)]
pub struct SearchParams {
    pub cmte_id: Option<String>,
    pub cycle: Option<i32>,
    /// Case-insensitive substring match against the payee `name`.
    pub name: Option<String>,
    pub min_amount: Option<f64>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn search(State(pool): State<PgPool>, Query(params): Query<SearchParams>) -> Result<Json<Vec<Disbursement>>, ApiError> {
    let page = Pagination::new(params.limit, params.offset);
    let rows = sqlx::query_as::<_, Disbursement>(
        "SELECT sub_id, cycle, cmte_id, name, city, state, transaction_dt, \
                transaction_amt::float8 AS transaction_amt, purpose, category_desc, memo_text \
         FROM disbursements \
         WHERE ($1::text IS NULL OR cmte_id = $1) \
           AND ($2::int IS NULL OR cycle = $2) \
           AND ($3::text IS NULL OR name ILIKE '%' || $3 || '%') \
           AND ($4::float8 IS NULL OR transaction_amt >= $4) \
         ORDER BY transaction_dt DESC NULLS LAST \
         LIMIT $5 OFFSET $6",
    )
    .bind(params.cmte_id)
    .bind(params.cycle)
    .bind(params.name)
    .bind(params.min_amount)
    .bind(page.limit())
    .bind(page.offset())
    .fetch_all(&pool)
    .await?;
    Ok(Json(rows))
}
