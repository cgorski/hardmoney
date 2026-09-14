//! Ingests a single raw `.fec` filing (via [`crate::parser`], not a bulk
//! CSV) into the `filings` + `schedule_e_lines` tables. This is the
//! precise counterpart to the CSV-derived `committee_to_candidate_transactions`
//! and pg_dump-derived `independent_expenditures`: every field comes
//! straight from the filing's own bytes, with no bulk-file aggregation or
//! FEC-side re-derivation in between.

use sqlx::PgPool;

use crate::parser::{Filing, ScheduleE};

use super::error::Result;

fn indexmap_to_json(map: &indexmap::IndexMap<String, String>) -> serde_json::Value {
    let mut obj = serde_json::Map::with_capacity(map.len());
    for (k, v) in map {
        obj.insert(k.clone(), serde_json::Value::String(v.clone()));
    }
    serde_json::Value::Object(obj)
}

/// Parses `bytes` as a `.fec` filing and stores it (plus any Schedule E
/// lines it contains) under `filing_id`, replacing any prior ingestion of
/// the same id.
pub async fn ingest_filing_bytes(
    pool: &PgPool,
    filing_id: i64,
    bytes: &[u8],
) -> Result<IngestReport> {
    let filing =
        Filing::parse_bytes(bytes).map_err(|e| super::error::BulkError::Zip(e.to_string()))?;
    ingest_filing(pool, filing_id, &filing).await
}

pub struct IngestReport {
    pub form_type: String,
    pub schedule_e_lines: usize,
}

pub async fn ingest_filing(pool: &PgPool, filing_id: i64, filing: &Filing) -> Result<IngestReport> {
    let header_json = indexmap_to_json(&filing.headers);
    let summary_json = indexmap_to_json(&filing.summary);
    let committee_id = filing.summary.get("filer_committee_id_number").cloned();
    let amends_filing_id: Option<i64> =
        filing.amends_filing.as_deref().and_then(|s| s.parse().ok());

    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO filings (filing_id, form_type, fec_version, committee_id, is_amendment, amends_filing_id, header, summary) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         ON CONFLICT (filing_id) DO UPDATE SET \
            form_type = EXCLUDED.form_type, fec_version = EXCLUDED.fec_version, \
            committee_id = EXCLUDED.committee_id, is_amendment = EXCLUDED.is_amendment, \
            amends_filing_id = EXCLUDED.amends_filing_id, header = EXCLUDED.header, summary = EXCLUDED.summary",
    )
    .bind(filing_id)
    .bind(&filing.raw_form_type)
    .bind(&filing.version)
    .bind(&committee_id)
    .bind(filing.is_amendment)
    .bind(amends_filing_id)
    .bind(&header_json)
    .bind(&summary_json)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM schedule_e_lines WHERE filing_id = $1")
        .bind(filing_id)
        .execute(&mut *tx)
        .await?;

    let mut schedule_e_lines = 0usize;
    for (idx, line) in filing.lines.iter().enumerate() {
        let Ok(se) = ScheduleE::try_from(line) else {
            continue;
        };
        let amt = se.expenditure_amount_cents.map(|c| c as f64 / 100.0);
        sqlx::query(
            "INSERT INTO schedule_e_lines \
                (filing_id, line_index, payee_name, expenditure_amt, expenditure_date, \
                 support_oppose_code, candidate_id, candidate_name, candidate_office_state, raw) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
             ON CONFLICT (filing_id, line_index) DO UPDATE SET \
                payee_name = EXCLUDED.payee_name, expenditure_amt = EXCLUDED.expenditure_amt, \
                expenditure_date = EXCLUDED.expenditure_date, support_oppose_code = EXCLUDED.support_oppose_code, \
                candidate_id = EXCLUDED.candidate_id, candidate_name = EXCLUDED.candidate_name, \
                candidate_office_state = EXCLUDED.candidate_office_state, raw = EXCLUDED.raw",
        )
        .bind(filing_id)
        .bind(idx as i32)
        .bind(&se.payee_name)
        .bind(amt)
        .bind(se.disbursement_date.or(se.dissemination_date))
        .bind(&se.support_oppose_code)
        .bind(&se.candidate_id_number)
        .bind(&se.candidate_name)
        .bind(&se.candidate_state)
        .bind(indexmap_to_json(&line.fields))
        .execute(&mut *tx)
        .await?;
        schedule_e_lines += 1;
    }

    tx.commit().await?;

    Ok(IngestReport {
        form_type: filing.raw_form_type.clone(),
        schedule_e_lines,
    })
}
