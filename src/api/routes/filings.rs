use axum::Json;
use axum::extract::{Path, Query, State};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::api::error::ApiError;
use crate::api::pagination::Pagination;

/// A raw `.fec` filing previously ingested via `hardmoney bulk-load-filing`
/// (which uses the `parser` module directly against the filing's own
/// bytes, not a bulk CSV derivation).
///
/// The amendment-chain fields use openFEC's names and semantics (see
/// [`crate::db::resolve_amendment_chain`]) so a client written against
/// `api.open.fec.gov/v1/filings/` reads them unchanged. They are `null`
/// on a row that was ingested before the chain columns existed and has
/// not been re-resolved (`resolve_all_amendment_chains`).
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Filing {
    pub filing_id: i64,
    pub form_type: String,
    pub fec_version: Option<String>,
    pub committee_id: Option<String>,
    pub is_amendment: bool,
    pub amends_filing_id: Option<i64>,
    /// `"A"` for an amendment, `"N"` otherwise (openFEC's code).
    pub amendment_indicator: String,
    /// Position in the chain: 0 for the original (or a filing that stands
    /// alone), 1, 2, ... for successive amendments.
    pub amendment_version: Option<i32>,
    /// Every filing id in the chain up to and including this one, in
    /// version order.
    pub amendment_chain: Option<Vec<i64>>,
    /// True only on the latest version of the report.
    pub most_recent: Option<bool>,
    /// The latest version's filing id (itself when `most_recent`).
    pub most_recent_file_number: Option<i64>,
    /// The version before this one; the original points at itself.
    pub previous_file_number: Option<i64>,
    /// An amendment whose header did not name an ingested original: it is
    /// shown as its own one-filing chain, and this flag says so.
    pub chain_unresolved: bool,
    /// The cover line's report code (`12G`, `Q1`, `YE`; `24`/`48` on the
    /// notice forms).
    pub report_type: Option<String>,
    pub coverage_start_date: Option<chrono::NaiveDate>,
    pub coverage_end_date: Option<chrono::NaiveDate>,
    /// Where to download the filing's raw bytes from the FEC.
    pub fec_url: String,
    pub header: Option<serde_json::Value>,
    pub summary: Option<serde_json::Value>,
    /// Body lines the lenient parser skipped when this filing was ingested.
    pub skipped_lines: i32,
    pub ingested_at: chrono::DateTime<chrono::Utc>,
}

/// The column list behind [`Filing`], shared by the get and list routes so
/// the two cannot drift. Storage names are aliased to openFEC's. A macro
/// (not a `const`) so the queries stay `concat!`-able string literals.
macro_rules! filing_columns {
    () => {
        "filing_id, form_type, fec_version, committee_id, is_amendment, amends_filing_id, \
         CASE WHEN is_amendment THEN 'A' ELSE 'N' END AS amendment_indicator, \
         amendment_version, amendment_chain, most_recent, \
         most_recent_filing_id AS most_recent_file_number, \
         previous_filing_id AS previous_file_number, chain_unresolved, \
         report_code AS report_type, coverage_from AS coverage_start_date, \
         coverage_through AS coverage_end_date, \
         'https://docquery.fec.gov/dcdev/posted/' || filing_id || '.fec' AS fec_url, \
         header, summary, skipped_lines, ingested_at"
    };
}

pub async fn get(
    State(pool): State<PgPool>,
    Path(filing_id): Path<i64>,
) -> Result<Json<Filing>, ApiError> {
    let row = sqlx::query_as::<_, Filing>(concat!(
        "SELECT ",
        filing_columns!(),
        " FROM filings WHERE filing_id = $1"
    ))
    .bind(filing_id)
    .fetch_optional(&pool)
    .await?;
    row.map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("no filing with filing_id {filing_id}")))
}

/// `GET /filings` query parameters.
#[derive(Debug, Deserialize)]
pub struct ListParams {
    /// Exact match on the cover line's filer committee id.
    pub committee_id: Option<String>,
    /// `true` keeps only the latest version of each report (the
    /// `filings_current` view); `false` keeps only superseded versions.
    /// Rows whose chain has not been resolved match neither.
    pub most_recent: Option<bool>,
    /// A base form (`F3X` matches `F3XN`, `F3XA`, `F3XT`) or an exact
    /// as-filed token (`F3XA`). Case-insensitive.
    pub form_type: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Lists ingested filings, newest filing id first.
pub async fn list(
    State(pool): State<PgPool>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<Filing>>, ApiError> {
    let page = Pagination::new(params.limit, params.offset).validate()?;
    let form_type = params
        .form_type
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_uppercase);
    let rows = sqlx::query_as::<_, Filing>(concat!(
        "SELECT ",
        filing_columns!(),
        " FROM filings \
         WHERE ($1::text IS NULL OR committee_id = $1) \
           AND ($2::boolean IS NULL OR most_recent = $2) \
           AND ($3::text IS NULL OR form_type = ANY (ARRAY[$3, $3 || 'N', $3 || 'A', $3 || 'T'])) \
         ORDER BY filing_id DESC LIMIT $4 OFFSET $5"
    ))
    .bind(params.committee_id)
    .bind(params.most_recent)
    .bind(form_type)
    .bind(page.limit())
    .bind(page.offset())
    .fetch_all(&pool)
    .await?;
    Ok(Json(rows))
}

/// A Schedule E line item extracted directly from a filing's own bytes by
/// the `parser` module -- the precise (not bulk-aggregated) counterpart
/// to `independent_expenditures`.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ScheduleELine {
    pub line_index: i32,
    pub payee_name: Option<String>,
    /// Read straight from the `NUMERIC` column as an exact `Decimal` --
    /// never cast through `::float8`, which would round-trip the amount
    /// through binary floating point and risk corrupting it.
    pub expenditure_amt: Option<Decimal>,
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
        "SELECT line_index, payee_name, expenditure_amt, expenditure_date, \
                support_oppose_code, candidate_id, candidate_name, candidate_office_state \
         FROM schedule_e_lines WHERE filing_id = $1 ORDER BY line_index",
    )
    .bind(filing_id)
    .fetch_all(&pool)
    .await?;
    Ok(Json(rows))
}
