//! Ingests a single raw `.fec` filing (via [`crate::parser`], not a bulk
//! CSV) into the `filings` + `schedule_e_lines` tables. This is the
//! precise counterpart to the CSV-derived `committee_to_candidate_transactions`
//! and pg_dump-derived `independent_expenditures`: every field comes
//! straight from the filing's own bytes, with no bulk-file aggregation or
//! FEC-side re-derivation in between.
//!
//! Ingestion parses **leniently** by default: an ETL job failing on one
//! unknown line type in a 700,000-line F3P is worse than recording that
//! the line was skipped. The skip count is stored on the `filings` row
//! (`skipped_lines`) and returned in [`IngestReport`], so nothing is
//! silently lost -- it is just not fatal. Callers who want strictness pass
//! [`ParseOptions::STRICT`].
//!
//! After the row is written, the filing's **amendment chain** is
//! recomputed in the same transaction ([`crate::db::resolve_amendment_chain`]
//! has the rules), so `filings.most_recent`, `amendment_chain`, and friends
//! are correct as soon as the ingest commits -- in whichever order the
//! original and its amendments arrive. A batch loader can pass
//! [`ChainResolution::Defer`] and run
//! [`crate::db::resolve_all_amendment_chains`] once at the end instead.

use sqlx::PgPool;

use crate::parser::{Filing, ParseOptions, ParsedLine, ScheduleE, SkippedLine, Table};

use super::error::Result;

/// Picks out the genuine Schedule E (independent expenditure) lines from a
/// parsed filing, paired with their original position in `filing.lines`.
///
/// `Filing::views::<ScheduleE>` already restricts to [`Table::SchE`], so a
/// Schedule A/B/C line can never be misfiled here as an all-null "Schedule
/// E" row -- the typed view refuses lines from other tables.
fn schedule_e_lines(filing: &Filing) -> Vec<(usize, ScheduleE)> {
    filing
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.table() == Table::SchE)
        .filter_map(|(idx, line)| line.view::<ScheduleE>().ok().map(|se| (idx, se)))
        .collect()
}

/// A line's fields as a JSON object in layout order (stored in `JSONB`
/// columns, where the full record is kept for anything the typed columns
/// do not cover).
fn fields_json(line: &ParsedLine) -> serde_json::Value {
    let mut obj = serde_json::Map::with_capacity(line.iter().len());
    for (k, v) in line.iter() {
        obj.insert(k.to_string(), serde_json::Value::String(v.to_string()));
    }
    serde_json::Value::Object(obj)
}

/// Outcome of ingesting one filing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct IngestReport {
    pub filing_id: i64,
    pub form_type: String,
    pub schedule_e_lines: usize,
    /// Body lines that could not be parsed and were skipped (empty under
    /// [`ParseOptions::STRICT`], which fails instead).
    pub skipped: Vec<SkippedLine>,
    /// How many `filings` rows had their amendment-chain columns
    /// recomputed (the whole chain this filing belongs to, plus the chain
    /// it left if a re-ingest changed its header). `None` when the caller
    /// passed [`ChainResolution::Defer`].
    pub chain_rows_resolved: Option<usize>,
}

/// Whether ingestion recomputes the amendment chain after storing a filing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ChainResolution {
    /// Recompute the affected chain in the ingest's own transaction (the
    /// default): the derived columns are never observably stale.
    #[default]
    Resolve,
    /// Leave the derived columns as they are. For batch loads: ingest
    /// everything, then call [`crate::db::resolve_all_amendment_chains`]
    /// once. Until then `most_recent` and the rest are NULL on new rows
    /// and possibly wrong on their chain-mates.
    Defer,
}

/// The id of the chain a filing belongs to: the original it amends, or
/// itself. A non-amendment heads its own chain even if its header happens
/// to carry a `FEC-<n>` (some Form 99s cite the report they discuss).
fn chain_key(filing_id: i64, is_amendment: bool, amends_filing_id: Option<i64>) -> i64 {
    if is_amendment {
        amends_filing_id.unwrap_or(filing_id)
    } else {
        filing_id
    }
}

/// Parses `bytes` as a `.fec` filing with `options` and stores it (plus
/// any Schedule E lines it contains) under `filing_id`, replacing any
/// prior ingestion of the same id, then resolves its amendment chain
/// ([`ChainResolution::Resolve`]).
pub async fn ingest_filing_bytes(
    pool: &PgPool,
    filing_id: i64,
    bytes: &[u8],
    options: &ParseOptions,
) -> Result<IngestReport> {
    ingest_filing_bytes_with(pool, filing_id, bytes, options, ChainResolution::Resolve).await
}

/// [`ingest_filing_bytes`] with an explicit [`ChainResolution`].
pub async fn ingest_filing_bytes_with(
    pool: &PgPool,
    filing_id: i64,
    bytes: &[u8],
    options: &ParseOptions,
    resolution: ChainResolution,
) -> Result<IngestReport> {
    let (filing, skipped) = Filing::parse_bytes_with(bytes, options)?.into_parts();
    ingest_filing_with(pool, filing_id, &filing, skipped, resolution).await
}

/// Stores an already-parsed filing and resolves its amendment chain
/// ([`ChainResolution::Resolve`]). `skipped` is whatever the lenient parse
/// reported (pass an empty `Vec` for a strict parse).
pub async fn ingest_filing(
    pool: &PgPool,
    filing_id: i64,
    filing: &Filing,
    skipped: Vec<SkippedLine>,
) -> Result<IngestReport> {
    ingest_filing_with(pool, filing_id, filing, skipped, ChainResolution::Resolve).await
}

/// [`ingest_filing`] with an explicit [`ChainResolution`].
///
/// Besides the header/summary `JSONB`, the row records what the chain
/// resolver and the API's filing list need without opening the JSON:
/// `report_id` (the header's `FEC-<n>` as filed), `amendment_number`
/// (the header's report number), `report_code` (the cover line's
/// `report_code`, or `report_type` on the forms -- F24, F5 -- that carry
/// that instead), and `coverage_from` / `coverage_through` (parsed with
/// [`crate::parse_fec_date`]; NULL when blank or malformed).
///
/// # Errors
///
/// Any database error rolls back the whole ingest, including the chain
/// update -- a filing is never half-stored.
pub async fn ingest_filing_with(
    pool: &PgPool,
    filing_id: i64,
    filing: &Filing,
    skipped: Vec<SkippedLine>,
    resolution: ChainResolution,
) -> Result<IngestReport> {
    let header_json = serde_json::to_value(&filing.header)?;
    let summary_json = fields_json(&filing.summary);
    let committee_id = filing
        .summary
        .get_non_empty("filer_committee_id_number")
        .map(str::to_string);
    let amends_filing_id: Option<i64> = filing.amends_filing.and_then(|n| i64::try_from(n).ok());
    let version = filing.version.to_string();
    let skipped_count = i32::try_from(skipped.len()).unwrap_or(i32::MAX);
    let report_id = Some(filing.header.report_id.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let report_code = filing
        .summary
        .get_non_empty("report_code")
        .or_else(|| filing.summary.get_non_empty("report_type"))
        .map(str::to_string);
    let coverage_from = filing
        .summary
        .get_non_empty("coverage_from_date")
        .and_then(crate::parse_fec_date);
    let coverage_through = filing
        .summary
        .get_non_empty("coverage_through_date")
        .and_then(crate::parse_fec_date);
    let amendment_number: Option<i32> = filing
        .header
        .amendment_number()
        .and_then(|n| i32::try_from(n).ok());

    let mut tx = pool.begin().await?;

    // A re-ingest may move the filing to a different chain (a corrected
    // header) or change whether it is an amendment at all; the chain it
    // *left* must be recomputed too, or its `most_recent` goes stale.
    let previous_key: Option<i64> = sqlx::query_as::<_, (bool, Option<i64>)>(
        "SELECT is_amendment, amends_filing_id FROM filings WHERE filing_id = $1 FOR UPDATE",
    )
    .bind(filing_id)
    .fetch_optional(&mut *tx)
    .await?
    .map(|(was_amendment, amended)| chain_key(filing_id, was_amendment, amended));

    sqlx::query(
        "INSERT INTO filings (filing_id, form_type, fec_version, committee_id, is_amendment, \
                              amends_filing_id, header, summary, skipped_lines, \
                              report_id, report_code, coverage_from, coverage_through, \
                              amendment_number) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
         ON CONFLICT (filing_id) DO UPDATE SET \
            form_type = EXCLUDED.form_type, fec_version = EXCLUDED.fec_version, \
            committee_id = EXCLUDED.committee_id, is_amendment = EXCLUDED.is_amendment, \
            amends_filing_id = EXCLUDED.amends_filing_id, header = EXCLUDED.header, \
            summary = EXCLUDED.summary, skipped_lines = EXCLUDED.skipped_lines, \
            report_id = EXCLUDED.report_id, report_code = EXCLUDED.report_code, \
            coverage_from = EXCLUDED.coverage_from, \
            coverage_through = EXCLUDED.coverage_through, \
            amendment_number = EXCLUDED.amendment_number, \
            ingested_at = now()",
    )
    .bind(filing_id)
    .bind(&filing.raw_form_type)
    .bind(&version)
    .bind(&committee_id)
    .bind(filing.is_amendment)
    .bind(amends_filing_id)
    .bind(&header_json)
    .bind(&summary_json)
    .bind(skipped_count)
    .bind(&report_id)
    .bind(&report_code)
    .bind(coverage_from)
    .bind(coverage_through)
    .bind(amendment_number)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM schedule_e_lines WHERE filing_id = $1")
        .bind(filing_id)
        .execute(&mut *tx)
        .await?;

    let extracted = schedule_e_lines(filing);
    for (idx, se) in &extracted {
        let Some(line) = filing.lines.get(*idx) else {
            continue;
        };
        // Bound directly as `Decimal` -- `sqlx`'s native `rust_decimal`
        // support encodes it straight to Postgres `NUMERIC` wire format,
        // so the exact value `ScheduleE::expenditure_amount` already holds
        // is what lands in the database, with no `f64` in between.
        sqlx::query(
            "INSERT INTO schedule_e_lines \
                (filing_id, line_index, payee_name, expenditure_amt, expenditure_date, \
                 support_oppose_code, candidate_id, candidate_name, candidate_office_state, raw) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(filing_id)
        .bind(i32::try_from(*idx).unwrap_or(i32::MAX))
        .bind(&se.payee_name)
        .bind(se.expenditure_amount)
        .bind(se.disbursement_date.or(se.dissemination_date))
        .bind(se.support_oppose_code())
        .bind(&se.candidate_id_number)
        .bind(&se.candidate_name)
        .bind(&se.candidate_state)
        .bind(fields_json(line))
        .execute(&mut *tx)
        .await?;
    }

    let chain_rows_resolved = match resolution {
        ChainResolution::Resolve => {
            let key = chain_key(filing_id, filing.is_amendment, amends_filing_id);
            let old = previous_key.filter(|k| *k != key);
            // Both chain locks up front, in ascending order, so two
            // concurrent moves between the same pair of chains cannot
            // deadlock (see `db::chain_lock`).
            if let Some(old) = old {
                crate::db::chain_lock(&mut tx, key.min(old)).await?;
                crate::db::chain_lock(&mut tx, key.max(old)).await?;
            }
            let mut rows = crate::db::resolve_amendment_chain_in(&mut tx, key).await?;
            if let Some(old) = old {
                rows =
                    rows.saturating_add(crate::db::resolve_amendment_chain_in(&mut tx, old).await?);
            }
            Some(rows)
        }
        ChainResolution::Defer => None,
    };

    tx.commit().await?;

    Ok(IngestReport {
        filing_id,
        form_type: filing.raw_form_type.clone(),
        schedule_e_lines: extracted.len(),
        skipped,
        chain_rows_resolved,
    })
}

/// Derives a filing id from a local `.fec` path.
///
/// FEC document-store downloads are named `<id>.fec`; this crate's own
/// fixtures are `<FORM>_<id>[_v<spec>].fec`. The id is the **last** run of
/// digits in the stem that is at least 4 digits long, so `F24N_2011823.fec`
/// is 2011823 (not 242011823, which is what "all the digits" produced) and
/// `F3XA_27789_v3.fec` is 27789. Returns `None` if there is no such run
/// (or the run does not fit an `i64`), rather than silently defaulting
/// to 0. The same rule as `export::filing_id_from_path`, which returns
/// `u64`; the two are kept separate so neither feature depends on the
/// other.
pub fn filing_id_from_path(path: &std::path::Path) -> Option<i64> {
    let stem = path.file_stem()?.to_str()?;
    stem.split(|c: char| !c.is_ascii_digit())
        .rfind(|run| run.len() >= 4)
        .and_then(|run| run.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::SpecVersion;
    use rust_decimal_macros::dec;
    use std::path::Path;

    fn line(table: Table, n: u64, fields: &[(&str, &str)]) -> ParsedLine {
        ParsedLine::from_pairs(
            table,
            SpecVersion::electronic(8, 5),
            n,
            fields.iter().copied(),
        )
        .unwrap()
    }

    fn filing_with(lines: Vec<ParsedLine>) -> Filing {
        let text = [
            "HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\u{1c}\u{1c}",
            "F3XN\u{1c}C00111111\u{1c}NAME",
        ]
        .join("\n");
        let mut f = Filing::parse(&text).unwrap();
        f.lines = lines;
        f
    }

    // Regression test for the bug where `schedule_e_lines` accepted ANY
    // line with a `filer_committee_id_number` (which is nearly every
    // schedule), silently misfiling Schedule A/B/C rows as all-null
    // "Schedule E" lines. Only genuine `Table::SchE` lines must pass.
    #[test]
    fn only_genuine_schedule_e_lines_are_extracted() {
        let filing = filing_with(vec![
            line(
                Table::SchA,
                3,
                &[
                    ("filer_committee_id_number", "C00111111"),
                    ("contribution_amount", "100.00"),
                ],
            ),
            line(
                Table::SchE,
                4,
                &[
                    ("filer_committee_id_number", "C00111111"),
                    ("payee_organization_name", "ACME MEDIA"),
                    ("expenditure_amount", "250.00"),
                    ("support_oppose_code", "O"),
                ],
            ),
            line(
                Table::SchB,
                5,
                &[
                    ("filer_committee_id_number", "C00111111"),
                    ("expenditure_amount", "75.00"),
                ],
            ),
        ]);

        let extracted = schedule_e_lines(&filing);
        assert_eq!(extracted.len(), 1, "only the SchE line should be extracted");
        let (idx, se) = &extracted[0];
        assert_eq!(*idx, 1);
        assert_eq!(se.payee_name.as_deref(), Some("ACME MEDIA"));
        assert_eq!(se.expenditure_amount, Some(dec!(250.00)));
        assert_eq!(se.support_oppose_code(), Some("O"));
    }

    #[test]
    fn filing_id_from_path_uses_the_last_long_digit_run() {
        let cases = [
            ("F24N_2011823.fec", Some(2011823)),
            ("F3XA_27789_v3.fec", Some(27789)),
            ("F3XN_210000_v5.3.fec", Some(210000)),
            ("F3A_767339_v8.0.fec", Some(767339)),
            ("2011823.fec", Some(2011823)),
            ("/some/dir/2011823.fec", Some(2011823)),
            ("F99.fec", None),
            ("notes.fec", None),
            ("F3_v8.fec", None),
        ];
        for (p, want) in cases {
            assert_eq!(filing_id_from_path(Path::new(p)), want, "{p}");
        }
    }
}
