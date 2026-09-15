//! `hardmoney filings`: find filings through openFEC's `/filings/` and,
//! optionally, download, validate, reconcile, or ingest each one.
//!
//! Also home to the per-filing action helpers (`Actions`, `apply`) that
//! `hardmoney efile` shares, so a filing found by either route is
//! processed and reported identically.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use clap::Args;
use hardmoney::Cycle;
use hardmoney::db::Namespace;
use hardmoney::fec::openfec::{FilingRecord, FilingsQuery, OpenFec};
use hardmoney::fec::{Cache, fetch_filing_bytes};
use hardmoney::parser::reconcile::{Coverage, ReportChain, coverage};
use hardmoney::{Filing, ParseOptions};
use serde::Serialize;

use super::CliResult;
use super::db_args::DbArgs;
use super::shared::columns;

/// `--cache-dir`, shared by every command that downloads raw filings.
#[derive(Args, Debug, Clone)]
pub struct CacheArgs {
    /// Where downloaded filings and daily archives are kept across runs.
    #[arg(long, env = Cache::ENV_VAR, default_value_os_t = default_cache_root())]
    pub cache_dir: PathBuf,
}

fn default_cache_root() -> PathBuf {
    Cache::from_env().root().to_path_buf()
}

impl CacheArgs {
    pub fn cache(&self) -> Cache {
        Cache::at(&self.cache_dir)
    }
}

/// Optional `--database-url` / `--schema`, required only with `--ingest`.
#[derive(Args, Debug, Clone)]
pub struct OptionalDbArgs {
    /// Postgres connection URL for --ingest. Include a username:
    /// postgres://user@host/db
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: Option<String>,

    /// Namespace (Postgres schema) for --ingest. Default: public.
    #[arg(long, env = "HARDMONEY_SCHEMA", default_value = "public")]
    pub schema: Namespace,
}

impl OptionalDbArgs {
    /// Connects if `ingest` was asked for; errors if it was but no URL is
    /// known.
    pub async fn pool_if(&self, ingest: bool) -> CliResult<Option<sqlx::PgPool>> {
        if !ingest {
            return Ok(None);
        }
        let Some(url) = self.database_url.clone() else {
            return Err("--ingest needs --database-url (or DATABASE_URL)".into());
        };
        let db = DbArgs {
            database_url: url,
            schema: self.schema.clone(),
        };
        Ok(Some(db.connect_current().await?))
    }
}

/// What to do with each filing after its bytes are in hand.
pub struct Actions {
    /// Copy the raw `.fec` here as `<id>.fec`.
    pub out_dir: Option<PathBuf>,
    pub validate: bool,
    pub reconcile: bool,
    /// Ingest into this pool (lenient parse, chain resolved).
    pub pool: Option<sqlx::PgPool>,
    /// Run `PROGRAM <id> <path>` with `HARDMONEY_FILING_*` set.
    pub exec: Option<PathBuf>,
}

impl Actions {
    pub fn any(&self) -> bool {
        self.out_dir.is_some()
            || self.validate
            || self.reconcile
            || self.pool.is_some()
            || self.exec.is_some()
    }
}

/// The result of applying [`Actions`] to one filing. Serialised into the
/// `actions` object of `--json` output.
#[derive(Debug, Default, Serialize)]
pub struct Outcome {
    /// Why nothing was done (a paper filing has no raw `.fec`). Not a
    /// failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ValidationSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reconciliation: Option<ReconcileSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingest: Option<IngestSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exec: Option<ExecSummary>,
    /// Errors from individual steps (ingest failure, exec failure); the
    /// run continues with the next filing and exits 1 at the end.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

impl Outcome {
    pub fn failed(&self) -> bool {
        self.parse_error.is_some() || !self.errors.is_empty()
    }
}

#[derive(Debug, Serialize)]
pub struct ValidationSummary {
    pub form_type: String,
    pub acceptable: bool,
    pub errors: usize,
    pub warnings: usize,
}

#[derive(Debug, Serialize)]
pub struct ReconcileSummary {
    pub form: String,
    /// `None` when the form has no reconciliation rules.
    pub checks: Option<usize>,
    pub disagreeing: Option<usize>,
    pub balances: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct IngestSummary {
    pub form_type: String,
    pub schedule_e_lines: usize,
    pub skipped: usize,
    pub chain_rows_resolved: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ExecSummary {
    pub program: PathBuf,
    pub exit_code: Option<i32>,
}

/// Runs every requested action on one filing. `path` is where the bytes
/// live on disk (the cache), for `--exec`.
pub async fn apply(actions: &Actions, id: u64, bytes: &[u8], path: Option<&Path>) -> Outcome {
    let mut out = Outcome::default();

    if let Some(dir) = &actions.out_dir {
        let dest = dir.join(format!("{id}.fec"));
        match std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&dest, bytes)) {
            Ok(()) => out.saved = Some(dest),
            Err(e) => out
                .errors
                .push(format!("could not write {}: {e}", dest.display())),
        }
    }

    if actions.validate || actions.reconcile {
        match Filing::parse_bytes_with(bytes, &ParseOptions::LENIENT) {
            Ok(lenient) => {
                if actions.validate {
                    let v = lenient.validate();
                    out.validation = Some(ValidationSummary {
                        form_type: lenient.value().raw_form_type.to_string(),
                        acceptable: v.is_acceptable(),
                        errors: v.error_count(),
                        warnings: v.warning_count(),
                    });
                }
                if actions.reconcile {
                    let filing = lenient.value();
                    out.reconciliation = Some(match filing.reconcile() {
                        Ok(r) => ReconcileSummary {
                            form: r.form.to_string(),
                            checks: Some(r.checks.len()),
                            disagreeing: Some(r.mismatches().count()),
                            balances: Some(r.balances()),
                        },
                        Err(_) => ReconcileSummary {
                            form: filing.summary.table().to_string(),
                            checks: None,
                            disagreeing: None,
                            balances: None,
                        },
                    });
                }
            }
            Err(e) => out.parse_error = Some(e.to_string()),
        }
    }

    if let Some(pool) = &actions.pool {
        match i64::try_from(id) {
            Ok(filing_id) => {
                match hardmoney::bulk::ingest::ingest_filing_bytes(
                    pool,
                    filing_id,
                    bytes,
                    &ParseOptions::LENIENT,
                )
                .await
                {
                    Ok(r) => {
                        out.ingest = Some(IngestSummary {
                            form_type: r.form_type.to_string(),
                            schedule_e_lines: r.schedule_e_lines,
                            skipped: r.skipped.len(),
                            chain_rows_resolved: r.chain_rows_resolved,
                        });
                    }
                    Err(e) => out.errors.push(format!("ingest failed: {e}")),
                }
            }
            Err(_) => out
                .errors
                .push(format!("filing id {id} does not fit the database's bigint")),
        }
    }

    if let Some(program) = &actions.exec {
        let path_arg = path.map(Path::to_path_buf).unwrap_or_default();
        let status = std::process::Command::new(program)
            .arg(id.to_string())
            .arg(&path_arg)
            .env("HARDMONEY_FILING_ID", id.to_string())
            .env("HARDMONEY_FILING_PATH", &path_arg)
            .status();
        match status {
            Ok(s) => {
                if !s.success() {
                    out.errors
                        .push(format!("{} exited with {s}", program.display()));
                }
                out.exec = Some(ExecSummary {
                    program: program.clone(),
                    exit_code: s.code(),
                });
            }
            Err(e) => out
                .errors
                .push(format!("could not run {}: {e}", program.display())),
        }
    }

    out
}

/// Prints an outcome as indented lines under the filing's own line.
pub fn print_outcome(id: u64, out: &Outcome) {
    if let Some(why) = &out.skipped {
        println!("  {id}: skipped: {why}");
    }
    if let Some(p) = &out.saved {
        println!("  {id}: saved {}", p.display());
    }
    if let Some(e) = &out.parse_error {
        println!("  {id}: parse failed: {e}");
    }
    if let Some(v) = &out.validation {
        println!(
            "  {id}: validate {}: {}, {} error(s), {} warning(s)",
            if v.acceptable {
                "ACCEPTABLE"
            } else {
                "NOT ACCEPTABLE"
            },
            v.form_type,
            v.errors,
            v.warnings
        );
    }
    if let Some(r) = &out.reconciliation {
        match (r.checks, r.disagreeing) {
            (Some(n), Some(0)) => println!("  {id}: reconcile {}: balances ({n} line(s))", r.form),
            (Some(n), Some(d)) => {
                println!("  {id}: reconcile {}: {d} of {n} line(s) disagree", r.form);
            }
            _ => println!(
                "  {id}: reconcile {}: no reconciliation rules for this form",
                r.form
            ),
        }
    }
    if let Some(i) = &out.ingest {
        let chain = match i.chain_rows_resolved {
            Some(n) => format!(", amendment chain resolved ({n} filing(s))"),
            None => String::new(),
        };
        println!(
            "  {id}: ingested {}: {} Schedule E line(s), {} skipped{chain}",
            i.form_type, i.schedule_e_lines, i.skipped
        );
    }
    if let Some(x) = &out.exec {
        println!(
            "  {id}: ran {} (exit {})",
            x.program.display(),
            x.exit_code
                .map_or_else(|| "signal".to_string(), |c| c.to_string())
        );
    }
    for e in &out.errors {
        println!("  {id}: error: {e}");
    }
}

#[derive(Args, Debug)]
pub struct FilingsArgs {
    /// Committee id, e.g. C00554709.
    #[arg(long, value_name = "ID")]
    pub committee: Option<String>,

    /// Candidate id, e.g. H4CA11114.
    #[arg(long, value_name = "ID")]
    pub candidate: Option<String>,

    /// Two-year cycle, e.g. 2026.
    #[arg(long)]
    pub cycle: Option<Cycle>,

    /// Base form type, e.g. F3X, F3, F24 (no amendment suffix).
    #[arg(long, value_name = "FORM")]
    pub form_type: Option<String>,

    /// Report type code, e.g. Q2, 12G, M9.
    #[arg(long, value_name = "CODE")]
    pub report_type: Option<String>,

    /// Only the latest version of each report.
    #[arg(long)]
    pub most_recent: bool,

    /// Only filings received on or after this date (YYYY-MM-DD).
    #[arg(long, value_name = "DATE")]
    pub since: Option<chrono::NaiveDate>,

    /// Only filings received on or before this date (YYYY-MM-DD).
    #[arg(long, value_name = "DATE")]
    pub until: Option<chrono::NaiveDate>,

    /// Stop after this many filings (pages are fetched as needed).
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,

    /// Print the records as a JSON array (openFEC's field names) instead
    /// of a table. With an action flag, each record gains an `actions`
    /// object.
    #[arg(long)]
    pub json: bool,

    /// Download each filing's raw .fec (cache-first) into this directory
    /// as <id>.fec.
    #[arg(long, value_name = "DIR")]
    pub fetch: Option<PathBuf>,

    /// Run the FEC's acceptance rules on each filing and print the
    /// error/warning counts.
    #[arg(long)]
    pub validate: bool,

    /// Recompute each report's cover-page totals from its schedules and
    /// print whether it balances.
    #[arg(long)]
    pub reconcile: bool,

    /// Order the reports found by coverage period and check each one
    /// against the reports before it: Column B against the schedules
    /// summed over the year (Form 3X) or cycle (Forms 3 and 3P), and cash
    /// on hand carried forward from the prior report's close. One line per
    /// report. Use with --most-recent so an original and its amendment are
    /// not both in the chain.
    #[arg(long)]
    pub reconcile_chain: bool,

    /// Ingest each filing into Postgres (needs --database-url).
    #[arg(long)]
    pub ingest: bool,

    #[command(flatten)]
    pub db: OptionalDbArgs,

    #[command(flatten)]
    pub cache: CacheArgs,
}

/// One report's place in a chain, for `--reconcile-chain`. Serialised as
/// the `chain` object of `--json` output.
#[derive(Debug, Default, Serialize)]
pub struct ChainSummary {
    pub form: String,
    pub coverage: Option<Coverage>,
    /// Reports whose schedules were summed for Column B (this one
    /// included).
    pub reports_summed: usize,
    pub column_a_checks: usize,
    pub column_a_disagreeing: usize,
    pub column_b_checks: usize,
    pub column_b_disagreeing: usize,
    /// `None` for the first report of the chain (nothing to carry from).
    pub carry_forward_ok: Option<bool>,
    /// Periods before this report that no report in the chain covers.
    pub gaps: Vec<Coverage>,
    /// Why the report could not be chained (download, parse, or chain
    /// error). The other fields are then defaults.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Downloads (cache-first) and parses every e-filed record, orders the
/// reports by coverage, and reconciles each against the ones before it on
/// the same form. Returns one summary per filing id, in coverage order.
fn reconcile_chain(records: &[FilingRecord], cache: &Cache) -> Vec<(u64, ChainSummary)> {
    let mut loaded: Vec<(u64, Filing, Coverage)> = Vec::new();
    let mut out: Vec<(u64, ChainSummary)> = Vec::new();
    for record in records {
        let Some(id) = record.filing_id() else {
            continue;
        };
        let failed = |error: String| {
            (
                id,
                ChainSummary {
                    form: record.form_type.clone().unwrap_or_default(),
                    error: Some(error),
                    ..ChainSummary::default()
                },
            )
        };
        let bytes = match fetch_filing_bytes(id, cache, record.raw_url().as_deref()) {
            Ok(b) => b,
            Err(e) => {
                out.push(failed(format!("download failed: {e}")));
                continue;
            }
        };
        let filing = match Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT) {
            Ok(lenient) => lenient.value().clone(),
            Err(e) => {
                out.push(failed(format!("parse failed: {e}")));
                continue;
            }
        };
        match coverage(&filing) {
            Some(cov) => loaded.push((id, filing, cov)),
            None => out.push(failed(
                "the cover line has no usable coverage dates".to_string(),
            )),
        }
    }
    loaded.sort_by_key(|(_, filing, cov)| (filing.summary.table().to_string(), *cov));

    for (i, (id, filing, cov)) in loaded.iter().enumerate() {
        let form = filing.summary.table();
        let priors = loaded[..i]
            .iter()
            .filter(|(_, f, _)| f.summary.table() == form)
            .map(|(_, f, _)| f);
        let mut summary = ChainSummary {
            form: form.to_string(),
            coverage: Some(*cov),
            ..ChainSummary::default()
        };
        match filing.reconcile() {
            Ok(r) => {
                summary.column_a_checks = r.checks.len();
                summary.column_a_disagreeing = r.mismatches().count();
            }
            Err(e) => {
                summary.error = Some(e.to_string());
                out.push((*id, summary));
                continue;
            }
        }
        match ReportChain::new(filing, priors) {
            Ok(chain) => {
                summary.reports_summed = chain.period_reports().len();
                summary.gaps = chain.gaps();
                let b = chain.column_b_checks();
                summary.column_b_checks = b.len();
                summary.column_b_disagreeing = b.iter().filter(|c| !c.matches()).count();
                let carry = chain.carry_forward_checks();
                summary.carry_forward_ok =
                    (!carry.is_empty()).then(|| carry.iter().all(|c| c.matches()));
            }
            Err(e) => summary.error = Some(e.to_string()),
        }
        out.push((*id, summary));
    }
    out
}

fn print_chain(rows: &[(u64, ChainSummary)], records: &[FilingRecord]) {
    let report_type = |id: u64| -> String {
        records
            .iter()
            .find(|r| r.filing_id() == Some(id))
            .and_then(|r| r.report_type.clone())
            .unwrap_or_default()
    };
    let verdict = |checks: usize, disagreeing: usize| -> String {
        if disagreeing == 0 {
            format!("balances ({checks})")
        } else {
            format!("{disagreeing} of {checks} disagree")
        }
    };
    let table: Vec<[String; 6]> = rows
        .iter()
        .map(|(id, s)| {
            let (a, b, cash) = match &s.error {
                Some(e) => (format!("error: {e}"), String::new(), String::new()),
                None => (
                    verdict(s.column_a_checks, s.column_a_disagreeing),
                    format!(
                        "{}, {} report(s) summed",
                        verdict(s.column_b_checks, s.column_b_disagreeing),
                        s.reports_summed
                    ),
                    match s.carry_forward_ok {
                        None => "first report".to_string(),
                        Some(true) => "ok".to_string(),
                        Some(false) => "DIFF".to_string(),
                    },
                ),
            };
            [
                id.to_string(),
                report_type(*id),
                s.coverage.map(|c| c.to_string()).unwrap_or_default(),
                a,
                b,
                cash,
            ]
        })
        .collect();
    println!(
        "{}",
        columns(
            &[
                "file_number",
                "report",
                "coverage",
                "column A",
                "column B (chain)",
                "cash carried"
            ],
            &table
        )
    );
    for (id, s) in rows {
        for gap in &s.gaps {
            println!(
                "  {id}: no report in the chain covers {gap}; its Column B sums are short by that period"
            );
        }
    }
}

pub async fn run(args: FilingsArgs) -> CliResult {
    let has_filter = args.committee.is_some()
        || args.candidate.is_some()
        || args.cycle.is_some()
        || args.form_type.is_some()
        || args.report_type.is_some()
        || args.since.is_some()
        || args.until.is_some();
    if !has_filter && args.limit.is_none() {
        return Err(
            "refusing to page through every filing the FEC has; add a filter \
                    (--committee, --candidate, --cycle, --form-type, --since, ...) or --limit"
                .into(),
        );
    }

    // Fail on a missing key or database before any request is made.
    let api = OpenFec::from_env()?;
    let pool = args.db.pool_if(args.ingest).await?;
    let mut query = FilingsQuery::new();
    if let Some(c) = &args.committee {
        query = query.committee_id(c.trim().to_ascii_uppercase());
    }
    if let Some(c) = &args.candidate {
        query = query.candidate_id(c.trim().to_ascii_uppercase());
    }
    if let Some(c) = args.cycle {
        query = query.cycle(c.year());
    }
    if let Some(f) = &args.form_type {
        query = query.form_type(f.trim().to_ascii_uppercase());
    }
    if let Some(r) = &args.report_type {
        query = query.report_type(r.trim().to_ascii_uppercase());
    }
    if args.most_recent {
        query = query.most_recent(true);
    }
    if let Some(d) = args.since {
        query = query.min_receipt_date(d);
    }
    if let Some(d) = args.until {
        query = query.max_receipt_date(d);
    }
    if let Some(limit) = args.limit {
        let per_page = u32::try_from(limit.clamp(1, 100)).unwrap_or(100);
        query = query.per_page(per_page);
    }

    let limit = args.limit.unwrap_or(usize::MAX);
    let mut records: Vec<FilingRecord> = Vec::new();
    for record in api.filings_all(&query) {
        records.push(record?);
        if records.len() >= limit {
            break;
        }
    }

    let actions = Actions {
        out_dir: args.fetch.clone(),
        validate: args.validate,
        reconcile: args.reconcile,
        pool,
        exec: None,
    };
    let cache = args.cache.cache();

    if !args.json {
        print_table(&records);
    }

    let mut json_rows = Vec::new();
    let mut failures = 0usize;
    for record in &records {
        let mut outcome = None;
        if actions.any() {
            outcome = Some(match record.filing_id() {
                Some(id) => match fetch_filing_bytes(id, &cache, record.raw_url().as_deref()) {
                    Ok(bytes) => apply(&actions, id, &bytes, Some(&cache.filing_path(id))).await,
                    Err(e) => Outcome {
                        errors: vec![format!("download failed: {e}")],
                        ..Outcome::default()
                    },
                },
                None => Outcome {
                    skipped: Some(if record.is_paper() {
                        "paper filing; there is no raw .fec".to_string()
                    } else {
                        "the record has no file_number to fetch".to_string()
                    }),
                    ..Outcome::default()
                },
            });
        }
        if let Some(o) = &outcome {
            if o.failed() {
                failures += 1;
            }
            if !args.json {
                print_outcome(record.filing_id().unwrap_or(0), o);
            }
        }
        if args.json {
            let mut v = serde_json::to_value(record)?;
            if let (Some(o), serde_json::Value::Object(map)) = (&outcome, &mut v) {
                map.insert("actions".to_string(), serde_json::to_value(o)?);
            }
            json_rows.push(v);
        }
    }

    if args.reconcile_chain {
        let rows = reconcile_chain(&records, &cache);
        if args.json {
            let by_id: HashMap<u64, &ChainSummary> = rows.iter().map(|(id, s)| (*id, s)).collect();
            for row in &mut json_rows {
                let id = row.get("file_number").and_then(serde_json::Value::as_u64);
                if let (Some(id), serde_json::Value::Object(map)) = (id, row)
                    && let Some(summary) = by_id.get(&id)
                {
                    map.insert("chain".to_string(), serde_json::to_value(summary)?);
                }
            }
        } else {
            if actions.any() {
                println!();
            }
            print_chain(&rows, &records);
        }
        failures += rows
            .iter()
            .filter(|(_, s)| {
                s.error.as_deref().is_some_and(|e| {
                    e.starts_with("download failed") || e.starts_with("parse failed")
                })
            })
            .count();
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&json_rows)?);
    } else if actions.any() {
        println!(
            "{} filing(s), {} action failure(s)",
            records.len(),
            failures
        );
    }
    if failures > 0 {
        return Err(format!("{failures} filing(s) could not be processed").into());
    }
    Ok(())
}

fn print_table(records: &[FilingRecord]) {
    let header = [
        "file_number",
        "form",
        "report",
        "coverage",
        "receipt_date",
        "amend",
        "most_recent",
        "total_receipts",
        "committee",
    ];
    let rows: Vec<[String; 9]> = records
        .iter()
        .map(|r| {
            [
                match r.file_number {
                    Some(n) if n < 0 => format!("{n} (paper)"),
                    other => opt(other),
                },
                r.form_type.clone().unwrap_or_default(),
                r.report_type.clone().unwrap_or_default(),
                match (r.coverage_start_date, r.coverage_end_date) {
                    (Some(a), Some(b)) => format!("{a}..{b}"),
                    (Some(a), None) => format!("{a}.."),
                    (None, Some(b)) => format!("..{b}"),
                    (None, None) => String::new(),
                },
                r.receipt_date
                    .map(|d| d.date().to_string())
                    .unwrap_or_default(),
                opt(r.amendment_version),
                match r.most_recent {
                    Some(true) => "yes".to_string(),
                    Some(false) => "no".to_string(),
                    None => String::new(),
                },
                r.total_receipts
                    .map(|d| format!("{d:.2}"))
                    .unwrap_or_default(),
                r.committee_id.clone().unwrap_or_default(),
            ]
        })
        .collect();
    let mut widths: Vec<usize> = header.iter().map(|h| h.len()).collect();
    for row in &rows {
        for (w, cell) in widths.iter_mut().zip(row.iter()) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let line = |cells: &[&str]| {
        cells
            .iter()
            .zip(&widths)
            .enumerate()
            .map(|(i, (c, w))| {
                // Money right-aligned; everything else left.
                if i == 7 {
                    format!("{c:>w$}")
                } else {
                    format!("{c:<w$}")
                }
            })
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    println!("{}", line(&header));
    for row in &rows {
        let cells: Vec<&str> = row.iter().map(String::as_str).collect();
        println!("{}", line(&cells));
    }
    if rows.is_empty() {
        println!("(no filings matched)");
    }
}

fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map(|x| x.to_string()).unwrap_or_default()
}
