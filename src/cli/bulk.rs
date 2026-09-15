//! `hardmoney bulk-*`: the FEC bulk-data ETL and single-filing ingestion.

use std::path::PathBuf;

use clap::{Args, ValueEnum};
use hardmoney::Cycle;
use hardmoney::bulk::{self, Input, LoadMode, LoadOptions, LoadReport};

use super::db_args::DbArgs;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ModeArg {
    /// Delete this cycle's existing rows, then load, in one transaction
    /// (reproduces the FEC's current file exactly, including deletions).
    Replace,
    /// Load on top of existing rows; fails on any duplicate key.
    Append,
}

impl From<ModeArg> for LoadMode {
    fn from(m: ModeArg) -> Self {
        match m {
            ModeArg::Replace => LoadMode::Replace,
            ModeArg::Append => LoadMode::Append,
        }
    }
}

#[derive(Args, Debug, Clone)]
pub struct LoadFlags {
    /// 4-digit even-year election cycle, e.g. 2026.
    #[arg(long)]
    pub cycle: Cycle,
    /// Cap the number of rows read (samples a multi-gigabyte source
    /// without downloading all of it). Recorded as a sample load.
    #[arg(long)]
    pub limit: Option<u64>,
    #[arg(long, value_enum, default_value_t = ModeArg::Replace)]
    pub mode: ModeArg,
    /// Confirm a `replace` that would delete more than 1,000,000 rows.
    #[arg(long)]
    pub yes: bool,
    /// Skip the load if the FEC's file is unchanged (ETag/Last-Modified)
    /// since the last full load of this source and cycle.
    #[arg(long)]
    pub if_changed: bool,
}

impl LoadFlags {
    fn options(&self) -> LoadOptions {
        LoadOptions::new(self.cycle)
            .mode(self.mode.into())
            .limit(self.limit)
            .confirmed(self.yes)
            .if_changed(self.if_changed)
    }
}

#[derive(Args, Debug)]
pub struct BulkLoadArgs {
    #[command(flatten)]
    pub db: DbArgs,
    /// One of: candidates, committees, candidate_committee_links,
    /// schedule_a, committee_to_committee_transactions,
    /// committee_to_candidate_transactions, disbursements,
    /// candidate_summary, house_senate_summary, pac_party_summary.
    pub source: String,
    #[command(flatten)]
    pub flags: LoadFlags,
    /// Load from a local zip instead of downloading from fec.gov.
    #[arg(long)]
    pub file: Option<PathBuf>,
}

pub async fn load(args: BulkLoadArgs) -> super::CliResult {
    let pool = args.db.connect_current().await?;
    let src = bulk::find_source(&args.source)
        .ok_or_else(|| bulk::BulkError::UnknownSource(args.source.clone()))?;
    let input = match args.file {
        Some(path) => Input::LocalFile(path),
        None => Input::Url(bulk::source::download_url(src, args.flags.cycle)),
    };
    let report = bulk::load(&pool, src, input, args.flags.options()).await?;
    print_report(&report);
    Ok(())
}

#[derive(Args, Debug)]
pub struct BulkLoadAllArgs {
    #[command(flatten)]
    pub db: DbArgs,
    #[command(flatten)]
    pub flags: LoadFlags,
    /// Stop at the first source that fails (default: continue and exit
    /// non-zero at the end if any failed).
    #[arg(long)]
    pub fail_fast: bool,
}

pub async fn load_all(args: BulkLoadAllArgs) -> super::CliResult {
    let pool = args.db.connect_current().await?;
    let mut failures: Vec<(&str, String)> = Vec::new();
    for src in bulk::source::ALL {
        let url = bulk::source::download_url(src, args.flags.cycle);
        eprintln!("loading {} from {url} ...", src.name);
        match bulk::load(&pool, src, Input::Url(url), args.flags.options()).await {
            Ok(report) => print_report(&report),
            Err(e) => {
                eprintln!("  failed: {e}");
                if args.fail_fast {
                    return Err(e.into());
                }
                failures.push((src.name, e.to_string()));
            }
        }
    }
    if !failures.is_empty() {
        return Err(format!(
            "{} source(s) failed: {}",
            failures.len(),
            failures
                .iter()
                .map(|(n, _)| *n)
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into());
    }
    Ok(())
}

fn print_report(r: &LoadReport) {
    if r.skipped_unchanged {
        println!("{}: unchanged since last full load, skipped", r.table);
        return;
    }
    let replaced = if r.rows_replaced > 0 {
        format!(", replaced {} existing", r.rows_replaced)
    } else {
        String::new()
    };
    let nulled = if r.dates_nulled > 0 {
        format!(", {} unparseable date(s) set NULL", r.dates_nulled)
    } else {
        String::new()
    };
    println!(
        "{}: loaded {} rows for cycle {} ({}{replaced}{nulled})",
        r.table,
        r.rows_loaded,
        r.cycle,
        r.mode.as_str()
    );
}

#[derive(Args, Debug)]
pub struct BulkRestoreDumpArgs {
    #[command(flatten)]
    pub db: DbArgs,
    /// One of: schedule_e, committee_history, schedule_a_full, schedule_b_full.
    pub name: String,
    /// Required for the multi-gigabyte schedule_a_full / schedule_b_full.
    #[arg(long)]
    pub allow_large: bool,
    /// Where to keep downloaded dumps (re-used across runs).
    #[arg(long, env = "HARDMONEY_CACHE_DIR", default_value_os_t = default_cache_dir())]
    pub cache_dir: PathBuf,
}

fn default_cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("hardmoney")
        .join("dumps")
}

pub async fn restore_dump(args: BulkRestoreDumpArgs) -> super::CliResult {
    let src = bulk::dump::find(&args.name)
        .ok_or_else(|| bulk::BulkError::UnknownDump(args.name.clone()))?;
    let pool = args.db.connect_current().await?;
    let report = bulk::dump::restore(
        &pool,
        &args.db.database_url,
        src,
        &args.cache_dir,
        args.allow_large,
    )
    .await?;
    println!(
        "restored {} rows into disclosure.{} (dump cached at {})",
        report.rows,
        report.table,
        report.dump_path.display()
    );
    if !report.warnings.is_empty() {
        eprintln!(
            "pg_restore reported {} non-fatal message(s); first: {}",
            report.warnings.len(),
            report.warnings[0]
        );
    }
    println!(
        "independent_expenditures view refreshed in namespace '{}'",
        args.db.schema
    );
    Ok(())
}

#[derive(Args, Debug)]
pub struct BulkLoadFilingArgs {
    #[command(flatten)]
    pub db: DbArgs,
    /// A local `.fec` file path, or a numeric filing id to download from
    /// the FEC's document store.
    pub filing: String,
    /// Fail on any unparseable body line instead of skipping and recording
    /// the count (the default is lenient, because one unknown line type
    /// should not fail an ingestion job).
    #[arg(long)]
    pub strict: bool,
    /// Override the filing id derived from the filename.
    #[arg(long)]
    pub filing_id: Option<i64>,
}

pub async fn load_filing(args: BulkLoadFilingArgs) -> super::CliResult {
    let pool = args.db.connect_current().await?;
    let options = if args.strict {
        hardmoney::ParseOptions::STRICT
    } else {
        hardmoney::ParseOptions::LENIENT
    };

    let (filing_id, bytes) = if let Ok(id) = args.filing.parse::<u64>() {
        let id_i64 = i64::try_from(id).map_err(|_| format!("filing id {id} out of range"))?;
        eprintln!("fetching filing {id} from docquery.fec.gov ...");
        (
            args.filing_id.unwrap_or(id_i64),
            hardmoney::Filing::fetch_bytes(id)?,
        )
    } else {
        let path = std::path::Path::new(&args.filing);
        let id = match args.filing_id.or_else(|| bulk::filing_id_from_path(path)) {
            Some(id) => id,
            None => {
                return Err(format!(
                    "could not derive a filing id from '{}'; pass --filing-id",
                    path.display()
                )
                .into());
            }
        };
        (id, std::fs::read(path)?)
    };

    let report = bulk::ingest_filing_bytes(&pool, filing_id, &bytes, &options).await?;
    println!(
        "ingested filing {} ({}): {} Schedule E line(s), {} skipped",
        report.filing_id,
        report.form_type,
        report.schedule_e_lines,
        report.skipped.len()
    );
    for s in report.skipped.iter().take(10) {
        eprintln!("  skipped {s}");
    }
    Ok(())
}
