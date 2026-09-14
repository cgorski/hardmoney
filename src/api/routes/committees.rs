use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::api::error::ApiError;
use crate::api::pagination::Pagination;

#[derive(Serialize, sqlx::FromRow)]
pub struct Committee {
    pub cmte_id: String,
    pub cycle: i32,
    pub cmte_nm: Option<String>,
    pub tres_nm: Option<String>,
    pub cmte_city: Option<String>,
    pub cmte_st: Option<String>,
    pub cmte_dsgn: Option<String>,
    pub cmte_tp: Option<String>,
    pub cmte_pty_affiliation: Option<String>,
    pub org_tp: Option<String>,
    pub connected_org_nm: Option<String>,
    pub cand_id: Option<String>,
}

#[derive(Deserialize)]
pub struct ListParams {
    pub cycle: Option<i32>,
    pub cmte_tp: Option<String>,
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn list(
    State(pool): State<PgPool>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<Committee>>, ApiError> {
    let page = Pagination::new(params.limit, params.offset);
    let rows = sqlx::query_as::<_, Committee>(
        "SELECT cmte_id, cycle, cmte_nm, tres_nm, cmte_city, cmte_st, cmte_dsgn, cmte_tp, \
                cmte_pty_affiliation, org_tp, connected_org_nm, cand_id \
         FROM committees \
         WHERE ($1::int IS NULL OR cycle = $1) \
           AND ($2::text IS NULL OR cmte_tp = $2) \
           AND ($3::text IS NULL OR cmte_nm ILIKE '%' || $3 || '%') \
         ORDER BY cmte_id, cycle LIMIT $4 OFFSET $5",
    )
    .bind(params.cycle)
    .bind(params.cmte_tp)
    .bind(params.q)
    .bind(page.limit())
    .bind(page.offset())
    .fetch_all(&pool)
    .await?;
    Ok(Json(rows))
}

pub async fn get(
    State(pool): State<PgPool>,
    Path(cmte_id): Path<String>,
) -> Result<Json<Vec<Committee>>, ApiError> {
    let rows = sqlx::query_as::<_, Committee>(
        "SELECT cmte_id, cycle, cmte_nm, tres_nm, cmte_city, cmte_st, cmte_dsgn, cmte_tp, \
                cmte_pty_affiliation, org_tp, connected_org_nm, cand_id \
         FROM committees WHERE cmte_id = $1 ORDER BY cycle DESC",
    )
    .bind(&cmte_id)
    .fetch_all(&pool)
    .await?;
    if rows.is_empty() {
        return Err(ApiError::NotFound(format!(
            "no committee with cmte_id {cmte_id}"
        )));
    }
    Ok(Json(rows))
}
