use axum::Json;
use axum::extract::{Query, State};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::api::error::ApiError;
use crate::api::pagination::Pagination;

/// A row from the `independent_expenditures` view (created per namespace by
/// `hardmoney::db::ensure_views`), which -- when present -- is backed by the FEC's
/// own official, weekly-updated `fec_fitem_sched_e.dump` pg_dump archive
/// (restored via `hardmoney bulk-restore-dump schedule_e`), not an
/// approximation derived from another bulk file. If that dump hasn't been
/// restored yet, this endpoint returns a 503 explaining so.
#[derive(Debug, Serialize, sqlx::FromRow)]
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
    /// Read straight from the source `NUMERIC` column as an exact
    /// `Decimal` -- never cast through `::float8`, which would
    /// round-trip the amount through binary floating point and risk
    /// corrupting it.
    pub expenditure_amt: Option<Decimal>,
    pub expenditure_date: Option<chrono::NaiveDateTime>,
    pub expenditure_description: Option<String>,
    /// A calendar year, so an integer -- not `f64`, which was never the
    /// right type here even before the decimal-money cleanup (a year
    /// has no fractional part to preserve or lose).
    pub rpt_yr: Option<i32>,
    pub election_cycle: Option<i32>,
}

#[derive(Debug, Deserialize)]
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
    let page = Pagination::new(params.limit, params.offset).validate()?;
    let rows = sqlx::query_as::<_, IndependentExpenditure>(
        "SELECT sub_id, cmte_id, committee_name, payee_name, candidate_id, candidate_name, \
                candidate_office_state, support_oppose_code, support_oppose_desc, \
                expenditure_amt, expenditure_date, \
                expenditure_description, rpt_yr::int4 AS rpt_yr, election_cycle::int4 AS election_cycle \
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
    .map_err(|e| {
        crate::api::error::missing_relation(
            e,
            "independent_expenditures",
            "run `hardmoney bulk-restore-dump schedule_e` to load the FEC's official \
             Schedule E pg_dump archive, then `hardmoney schema-init`",
        )
    })?;
    Ok(Json(rows))
}
