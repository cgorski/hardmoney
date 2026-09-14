//! Ingests a single raw `.fec` filing (via [`crate::parser`], not a bulk
//! CSV) into the `filings` + `schedule_e_lines` tables. This is the
//! precise counterpart to the CSV-derived `committee_to_candidate_transactions`
//! and pg_dump-derived `independent_expenditures`: every field comes
//! straight from the filing's own bytes, with no bulk-file aggregation or
//! FEC-side re-derivation in between.

use sqlx::PgPool;

use crate::parser::{Filing, ParsedLine, ScheduleE};

use super::error::Result;

/// Picks out the genuine Schedule E (independent expenditure) lines from a
/// parsed filing, paired with their original position in `filing.lines`.
///
/// This restricts to `table == "SchE"` before attempting the typed-view
/// conversion. `ScheduleE::try_from` only requires `filer_committee_id_number`
/// to succeed -- a field present on nearly every schedule (SchA, SchB, SchC,
/// ...) -- so calling it on lines from other tables would misfile them here
/// as all-null "Schedule E" rows instead of being skipped as not applicable.
fn schedule_e_lines(filing: &Filing) -> Vec<(usize, ScheduleE)> {
    filing
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l): &(usize, &ParsedLine)| l.table == "SchE")
        .filter_map(|(idx, line)| ScheduleE::try_from(line).ok().map(|se| (idx, se)))
        .collect()
}

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

    let extracted = schedule_e_lines(filing);
    let mut inserted = 0usize;
    for (idx, se) in &extracted {
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
        .bind(*idx as i32)
        .bind(&se.payee_name)
        .bind(amt)
        .bind(se.disbursement_date.or(se.dissemination_date))
        .bind(&se.support_oppose_code)
        .bind(&se.candidate_id_number)
        .bind(&se.candidate_name)
        .bind(&se.candidate_state)
        .bind(indexmap_to_json(&filing.lines[*idx].fields))
        .execute(&mut *tx)
        .await?;
        inserted += 1;
    }

    tx.commit().await?;

    Ok(IngestReport {
        form_type: filing.raw_form_type.clone(),
        schedule_e_lines: inserted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::filing::ParsedLine as PL;

    fn line(table: &'static str, fields: &[(&str, &str)]) -> PL {
        let mut map = indexmap::IndexMap::new();
        for (k, v) in fields {
            map.insert(k.to_string(), v.to_string());
        }
        PL {
            raw_form_type: table.to_string(),
            table,
            fields: map,
        }
    }

    // Regression test for the bug where `schedule_e_lines` accepted ANY
    // line with a `filer_committee_id_number` (which is nearly every
    // schedule), silently misfiling Schedule A/B/C rows as all-null
    // "Schedule E" lines. Only genuine `table == "SchE"` lines must pass.
    #[test]
    fn only_genuine_schedule_e_lines_are_extracted() {
        let filing = Filing {
            raw_form_type: "F3A".to_string(),
            base_form_type: "F3A".to_string(),
            version: "8.5".to_string(),
            is_amendment: false,
            amends_filing: None,
            headers: indexmap::IndexMap::new(),
            summary: indexmap::IndexMap::new(),
            lines: vec![
                line(
                    "SchA",
                    &[
                        ("filer_committee_id_number", "C00111111"),
                        ("contribution_amount", "100.00"),
                    ],
                ),
                line(
                    "SchE",
                    &[
                        ("filer_committee_id_number", "C00111111"),
                        ("payee_organization_name", "ACME MEDIA"),
                        ("expenditure_amount", "250.00"),
                    ],
                ),
                line(
                    "SchB",
                    &[
                        ("filer_committee_id_number", "C00111111"),
                        ("expenditure_amount", "75.00"),
                    ],
                ),
            ],
        };

        let extracted = schedule_e_lines(&filing);
        assert_eq!(extracted.len(), 1, "only the SchE line should be extracted");
        let (idx, se) = &extracted[0];
        assert_eq!(*idx, 1);
        assert_eq!(se.payee_name.as_deref(), Some("ACME MEDIA"));
        assert_eq!(se.expenditure_amount_cents, Some(25_000));
    }
}
