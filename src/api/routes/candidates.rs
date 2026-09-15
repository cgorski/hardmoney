use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::api::error::ApiError;
use crate::api::pagination::{Pagination, cycle_param};

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Candidate {
    pub cand_id: String,
    pub cycle: i32,
    pub cand_name: Option<String>,
    pub cand_pty_affiliation: Option<String>,
    pub cand_election_yr: Option<String>,
    pub cand_office_st: Option<String>,
    pub cand_office: Option<String>,
    pub cand_office_district: Option<String>,
    pub cand_ici: Option<String>,
    pub cand_status: Option<String>,
    pub cand_pcc: Option<String>,
    pub cand_city: Option<String>,
    pub cand_st: Option<String>,
    pub cand_zip: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub cycle: Option<i32>,
    pub state: Option<String>,
    pub office: Option<String>,
    /// Case-insensitive substring match against `cand_name`.
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn list(
    State(pool): State<PgPool>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<Candidate>>, ApiError> {
    let page = Pagination::new(params.limit, params.offset);
    let cycle = cycle_param(params.cycle)?;
    let rows = sqlx::query_as::<_, Candidate>(
        "SELECT cand_id, cycle, cand_name, cand_pty_affiliation, cand_election_yr, cand_office_st, \
                cand_office, cand_office_district, cand_ici, cand_status, cand_pcc, cand_city, cand_st, cand_zip \
         FROM candidates \
         WHERE ($1::int IS NULL OR cycle = $1) \
           AND ($2::text IS NULL OR cand_office_st = $2) \
           AND ($3::text IS NULL OR cand_office = $3) \
           AND ($4::text IS NULL OR cand_name ILIKE '%' || $4 || '%') \
         ORDER BY cand_id, cycle \
         LIMIT $5 OFFSET $6",
    )
    .bind(cycle)
    .bind(params.state)
    .bind(params.office)
    .bind(params.q)
    .bind(page.limit())
    .bind(page.offset())
    .fetch_all(&pool)
    .await?;
    Ok(Json(rows))
}

pub async fn get(
    State(pool): State<PgPool>,
    Path(cand_id): Path<String>,
) -> Result<Json<Vec<Candidate>>, ApiError> {
    let rows = sqlx::query_as::<_, Candidate>(
        "SELECT cand_id, cycle, cand_name, cand_pty_affiliation, cand_election_yr, cand_office_st, \
                cand_office, cand_office_district, cand_ici, cand_status, cand_pcc, cand_city, cand_st, cand_zip \
         FROM candidates WHERE cand_id = $1 ORDER BY cycle DESC",
    )
    .bind(&cand_id)
    .fetch_all(&pool)
    .await?;
    if rows.is_empty() {
        return Err(ApiError::NotFound(format!(
            "no candidate with cand_id {cand_id}"
        )));
    }
    Ok(Json(rows))
}
