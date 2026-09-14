use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::api::error::ApiError;
use crate::api::pagination::Pagination;

/// A row from the `independent_expenditures` view (see
/// `src/db/schema.sql`), which -- when present -- is backed by the FEC's
/// own official, weekly-updated `fec_fitem_sched_e.dump` pg_dump archive
/// (restored via `hardmoney bulk restore-dump schedule_e`), not an
/// approximation derived from another bulk file. If that dump hasn't been
/// restored yet, this endpoint returns a 500 explaining so.
#[derive(Serialize, sqlx::FromRow)]
pub struct IndependentExpenditure {
    pub sub_id: i64,
    pub cmte_id: Option<String>,
    pub committee_name: Option<String>,
    pub payee_name: Option<String>,
    pub candidate_id: Option<String>,
    pub candidate_name: Option<String>,
    pub candidate_office_state: Option<String>,
    pub support_oppose_code: Option<String>,
    pub support_oppose_desc: Option<String>,
    pub expenditure_amt: Option<f64>,
    pub expenditure_date: Option<chrono::NaiveDateTime>,
    pub expenditure_description: Option<String>,
    pub rpt_yr: Option<f64>,
    pub election_cycle: Option<f64>,
}

#[derive(Deserialize)]
pub struct SearchParams {
    pub candidate_id: Option<String>,
    pub cmte_id: Option<String>,
    /// `S` = support, `O` = oppose (per the FEC's own support/oppose code).
    pub support_oppose_code: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn search(
    State(pool): State<PgPool>,
    Query(params): Query<SearchParams>,
) -> Result<Json<Vec<IndependentExpenditure>>, ApiError> {
    let page = Pagination::new(params.limit, params.offset);
    let rows = sqlx::query_as::<_, IndependentExpenditure>(
        "SELECT sub_id, cmte_id, committee_name, payee_name, candidate_id, candidate_name, \
                candidate_office_state, support_oppose_code, support_oppose_desc, \
                expenditure_amt::float8 AS expenditure_amt, expenditure_date, \
                expenditure_description, rpt_yr::float8 AS rpt_yr, election_cycle::float8 AS election_cycle \
         FROM independent_expenditures \
         WHERE ($1::text IS NULL OR candidate_id = $1) \
           AND ($2::text IS NULL OR cmte_id = $2) \
           AND ($3::text IS NULL OR support_oppose_code = $3) \
         ORDER BY expenditure_date DESC NULLS LAST \
         LIMIT $4 OFFSET $5",
    )
    .bind(params.candidate_id)
    .bind(params.cmte_id)
    .bind(params.support_oppose_code)
    .bind(page.limit())
    .bind(page.offset())
    .fetch_all(&pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db_err) if db_err.message().contains("independent_expenditures") => ApiError::NotFound(
            "the independent_expenditures view doesn't exist yet -- run \
             `hardmoney bulk restore-dump schedule_e` to load the FEC's official \
             Schedule E pg_dump archive first"
                .to_string(),
        ),
        _ => ApiError::Database(e),
    })?;
    Ok(Json(rows))
}
