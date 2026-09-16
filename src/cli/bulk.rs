//! `hardmoney bulk-*`: the FEC bulk-data ETL and single-filing ingestion.

use std::path::PathBuf;

use clap::{Args, ValueEnum};
use hardmoney::Cycle;
use hardmoney::bulk::{self, Input, LoadMode, LoadOptions, LoadReport, dump};

use super::db_args::DbArgs;
use super::shared::{EndpointArgs, columns, default_dump_cache_dir};

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
    #[command(flatten)]
    pub endpoints: EndpointArgs,
}

pub async fn load(args: BulkLoadArgs) -> super::CliResult {
    let endpoints = args.endpoints.resolve()?;
    let pool = args.db.connect_current().await?;
    let src = bulk::find_source(&args.source)
        .ok_or_else(|| bulk::BulkError::UnknownSource(args.source.clone()))?;
    let input = match args.file {
        Some(path) => Input::LocalFile(path),
        None => Input::Url(bulk::source::download_url_with(
            src,
            args.flags.cycle,
            &endpoints,
        )),
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
    #[command(flatten)]
    pub endpoints: EndpointArgs,
}

pub async fn load_all(args: BulkLoadAllArgs) -> super::CliResult {
    let endpoints = args.endpoints.resolve()?;
    let pool = args.db.connect_current().await?;
    let mut failures: Vec<(&str, String)> = Vec::new();
    for src in bulk::source::ALL {
        let url = bulk::source::download_url_with(src, args.flags.cycle, &endpoints);
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
    /// Restore only these two-year periods of a cycle-split dump
    /// (schedule_a_full, schedule_b_full), e.g. `--cycles 2024,2026`:
    /// the parent table plus the named child tables, data only. Repeat
    /// with other cycles later to add them.
    #[arg(long, value_delimiter = ',')]
    pub cycles: Vec<Cycle>,
    /// Skip the archive's indexes, primary keys, and triggers (the FEC's
    /// figures for Schedule A: about 5 hours instead of 35). Add
    /// hardmoney's indexes afterwards with `bulk-dump-index`.
    #[arg(long)]
    pub no_indexes: bool,
    /// Required to restore all of schedule_a_full / schedule_b_full
    /// (tens of gigabytes); not needed with --cycles.
    #[arg(long)]
    pub allow_large: bool,
    /// Restore from an already-downloaded .dump file instead of the cache.
    #[arg(long)]
    pub dump_file: Option<PathBuf>,
    /// pg_restore parallel workers (helps when restoring several cycles).
    #[arg(long, default_value_t = 1)]
    pub jobs: u8,
    /// Where to keep downloaded dumps (re-used across runs; an interrupted
    /// download resumes from its .partial file).
    #[arg(long, env = "HARDMONEY_CACHE_DIR", default_value_os_t = default_dump_cache_dir())]
    pub cache_dir: PathBuf,
    #[command(flatten)]
    pub endpoints: EndpointArgs,
}

pub async fn restore_dump(args: BulkRestoreDumpArgs) -> super::CliResult {
    let endpoints = args.endpoints.resolve()?;
    let src = bulk::dump::find(&args.name)
        .ok_or_else(|| bulk::BulkError::UnknownDump(args.name.clone()))?;
    let pool = args.db.connect_current().await?;
    let options = dump::RestoreOptions::new()
        .cycles(args.cycles.iter().copied())
        .data_only(args.no_indexes)
        .allow_large(args.allow_large)
        .dump_file(args.dump_file.clone())
        .jobs(args.jobs)
        .endpoints(endpoints);
    let report =
        dump::restore_with(&pool, &args.db.database_url, src, &args.cache_dir, &options).await?;

    let where_from = if args.dump_file.is_some() {
        format!("from {}", report.dump_path.display())
    } else {
        format!("dump cached at {}", report.dump_path.display())
    };
    if report.cycles.is_empty() {
        println!(
            "restored {} rows into disclosure.{} ({where_from})",
            report.rows, report.table
        );
    } else {
        println!(
            "restored {} rows into disclosure.{} for cycle(s) {} ({where_from})",
            report.rows,
            report.table,
            report
                .cycles
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
        for t in &report.tables {
            println!("  disclosure.{t}");
        }
    }
    if report.data_only {
        println!(
            "data only: {} archive index(es), primary keys, and triggers skipped; \
             run `hardmoney bulk-dump-index {}{}` to add hardmoney's indexes",
            report.indexes_skipped,
            src.name,
            if report.cycles.is_empty() {
                String::new()
            } else {
                format!(
                    " --cycles {}",
                    report
                        .cycles
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
        );
    }
    if !report.warnings.is_empty() {
        eprintln!(
            "pg_restore reported {} non-fatal message(s); first: {}",
            report.warnings.len(),
            report.warnings.first().map(String::as_str).unwrap_or("")
        );
    }
    if report.load_ids.is_empty() {
        eprintln!(
            "note: namespace '{}' has no loads table, so this restore was not recorded",
            args.db.schema
        );
    }
    println!(
        "views over disclosure refreshed in namespace '{}'",
        args.db.schema
    );
    Ok(())
}

#[derive(Args, Debug)]
pub struct BulkDumpInfoArgs {
    #[command(flatten)]
    pub db: DbArgs,
    /// Skip the HEAD requests to fec.gov (sizes and dates come from the
    /// cache's sidecar files only).
    #[arg(long)]
    pub offline: bool,
    /// Where downloaded dumps are kept.
    #[arg(long, env = "HARDMONEY_CACHE_DIR", default_value_os_t = default_dump_cache_dir())]
    pub cache_dir: PathBuf,
    #[command(flatten)]
    pub endpoints: EndpointArgs,
}

/// `bulk-dump-info`: the four dumps, what the FEC is serving now, what is
/// cached, what is in the database, and the restores this namespace has
/// recorded.
pub async fn dump_info(args: BulkDumpInfoArgs) -> super::CliResult {
    let endpoints = args.endpoints.resolve()?;
    let pool = args.db.connect_current().await?;

    let header = [
        "dump",
        "table",
        "remote size",
        "last-modified (UTC)",
        "cached",
        "in database",
    ];
    let mut rows: Vec<[String; 6]> = Vec::with_capacity(dump::ALL.len());
    for src in dump::ALL {
        let remote = if args.offline {
            None
        } else {
            let endpoints = endpoints.clone();
            match tokio::task::spawn_blocking(move || dump::remote_info_with(src, &endpoints))
                .await?
            {
                Ok(r) => Some(r),
                Err(e) => {
                    eprintln!("{}: {e}", src.name);
                    None
                }
            }
        };
        let cache = dump::cached(src, &args.cache_dir);
        let remote = remote.unwrap_or_else(|| cache.meta.clone());
        let state = dump::table_state(&pool, src).await?;

        let cached_col = match (cache.complete_bytes, cache.partial_bytes) {
            (Some(n), _) => {
                let stale = matches!((cache.meta.etag.as_deref(), remote.etag.as_deref()),
                    (Some(a), Some(b)) if a != b);
                format!(
                    "{}{}",
                    dump::fmt_bytes(n),
                    if stale { " (FEC has a newer file)" } else { "" }
                )
            }
            (None, Some(n)) => format!("partial {}", dump::fmt_bytes(n)),
            (None, None) => "-".to_string(),
        };
        let db_col = if !state.exists {
            "-".to_string()
        } else if state.partitions.is_empty() {
            format!(
                "~{} rows, {} index(es)",
                state.approx_rows, state.index_count
            )
        } else {
            let cycles: Vec<String> = state
                .partitions
                .iter()
                .filter_map(|t| dump::partition_cycle(src.disclosure_table, t))
                .map(|c| c.to_string())
                .collect();
            format!(
                "~{} rows, {} index(es), cycles {}",
                state.approx_rows,
                state.index_count,
                cycles.join(",")
            )
        };
        rows.push([
            src.name.to_string(),
            src.disclosure_table.to_string(),
            remote
                .size
                .map(dump::fmt_bytes)
                .unwrap_or_else(|| format!("~{}", dump::fmt_bytes(src.approx_size_bytes))),
            remote
                .last_modified
                .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "?".to_string()),
            cached_col,
            db_col,
        ]);
    }
    print_columns(&header, &rows);
    println!("cache: {}", args.cache_dir.display());

    let history = dump::restore_history(&pool).await?;
    if history.is_empty() {
        println!(
            "no restores recorded in namespace '{}' (records live in the namespace the restore ran from)",
            args.db.schema
        );
        return Ok(());
    }
    println!("restores recorded in namespace '{}':", args.db.schema);
    let header = [
        "restored (UTC)",
        "dump",
        "cycle",
        "mode",
        "rows",
        "source last-modified",
    ];
    let rows: Vec<[String; 6]> = history
        .iter()
        .take(20)
        .map(|r| {
            [
                r.restored_at.format("%Y-%m-%d %H:%M").to_string(),
                r.dump.clone(),
                r.cycle
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "all".into()),
                r.mode.clone(),
                r.rows.to_string(),
                r.source_last_modified
                    .map(|t| t.format("%Y-%m-%d").to_string())
                    .unwrap_or_else(|| "local file".into()),
            ]
        })
        .collect();
    print_columns(&header, &rows);
    if history.len() > 20 {
        println!("({} older record(s) not shown)", history.len() - 20);
    }
    Ok(())
}

#[derive(Args, Debug)]
pub struct BulkDumpIndexArgs {
    #[command(flatten)]
    pub db: DbArgs,
    /// One of: schedule_e, committee_history, schedule_a_full, schedule_b_full.
    pub name: String,
    /// Index only these cycles' child tables (default: every restored one).
    #[arg(long, value_delimiter = ',')]
    pub cycles: Vec<Cycle>,
}

/// `bulk-dump-index`: hardmoney's indexes on a restored dump table.
pub async fn dump_index(args: BulkDumpIndexArgs) -> super::CliResult {
    let src = bulk::dump::find(&args.name)
        .ok_or_else(|| bulk::BulkError::UnknownDump(args.name.clone()))?;
    let pool = args.db.connect_current().await?;
    eprintln!(
        "indexing disclosure.{}{} ...",
        src.disclosure_table,
        if args.cycles.is_empty() {
            String::new()
        } else {
            format!(
                " for cycle(s) {}",
                args.cycles
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    );
    let report = dump::create_indexes(&pool, src, &args.cycles).await?;
    for c in &report.created {
        println!(
            "created {} on disclosure.{} ({:.1}s)",
            c.name,
            c.table,
            c.elapsed.as_secs_f64()
        );
    }
    for name in &report.existing {
        println!("exists  {name}");
    }
    for s in &report.skipped {
        println!("skipped {} on disclosure.{}: {}", s.name, s.table, s.reason);
    }
    println!(
        "{} table(s): {} created, {} already present, {} skipped",
        report.tables.len(),
        report.created.len(),
        report.existing.len(),
        report.skipped.len()
    );
    Ok(())
}

#[derive(Args, Debug)]
pub struct BulkDumpCompareArgs {
    #[command(flatten)]
    pub db: DbArgs,
    /// A filing already ingested with `bulk-load-filing`.
    pub filing_id: i64,
}

/// `bulk-dump-compare`: the raw `.fec` Schedule E lines of one ingested
/// filing against the FEC's processed rows for it in the Schedule E dump.
pub async fn dump_compare(args: BulkDumpCompareArgs) -> super::CliResult {
    let pool = args.db.connect_current().await?;
    let r = dump::compare_filing(&pool, args.filing_id).await?;
    println!(
        "filing {} ({}{})",
        r.filing_id,
        r.form_type,
        r.committee_id
            .as_deref()
            .map(|c| format!(", {c}"))
            .unwrap_or_default()
    );
    let header = [
        "",
        "raw .fec (schedule_e_lines)",
        "FEC dump (fec_fitem_sched_e)",
    ];
    let rows = [
        [
            "rows".to_string(),
            r.raw_rows.to_string(),
            r.dump_rows.to_string(),
        ],
        [
            "total amount".to_string(),
            format!("{:.2}", r.raw_total),
            format!("{:.2}", r.dump_total),
        ],
        [
            "matched by tran_id".to_string(),
            r.matched().to_string(),
            r.matched().to_string(),
        ],
    ];
    print_columns(&header, &rows);
    println!("only in raw filing:  {}", list_or_none(&r.only_raw));
    println!("only in FEC dump:    {}", list_or_none(&r.only_dump));
    if r.amount_mismatches.is_empty() {
        println!("amount mismatches:   (none)");
    } else {
        println!("amount mismatches:");
        for m in &r.amount_mismatches {
            println!(
                "  {}: raw {} vs dump {}",
                m.transaction_id,
                m.raw
                    .map(|d| format!("{d:.2}"))
                    .unwrap_or_else(|| "NULL".into()),
                m.dump
                    .map(|d| format!("{d:.2}"))
                    .unwrap_or_else(|| "NULL".into())
            );
        }
    }
    match r.dump_newest_file_num {
        Some(newest) if r.newer_than_dump() => println!(
            "newest file_num in dump: {newest}; this filing is newer than the dump (weekly snapshot lag)"
        ),
        Some(newest) => println!("newest file_num in dump: {newest}"),
        None => println!("the Schedule E dump has no rows"),
    }
    if r.dump_rows == 0 && r.form_type.starts_with("F24") {
        println!(
            "note: the FEC's Schedule E dump holds periodic reports only (F3X, F5, F3, F3P); \
             Form 24 filings never appear in it"
        );
    }
    Ok(())
}

fn list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        return "(none)".to_string();
    }
    let shown: Vec<&str> = items.iter().take(20).map(String::as_str).collect();
    let more = items.len().saturating_sub(shown.len());
    if more > 0 {
        format!("{} (+{more} more)", shown.join(", "))
    } else {
        shown.join(", ")
    }
}

/// Prints `rows` under `header` with each column padded to its widest
/// cell.
fn print_columns<const N: usize>(header: &[&str; N], rows: &[[String; N]]) {
    println!("{}", columns(header, rows));
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
    /// Skip recomputing the filing's amendment chain (`most_recent`,
    /// `amendment_chain`, ...) after the insert. For scripted batch loads:
    /// ingest every file with this flag, then resolve the whole table once.
    #[arg(long)]
    pub no_resolve: bool,
    #[command(flatten)]
    pub endpoints: EndpointArgs,
}

pub async fn load_filing(args: BulkLoadFilingArgs) -> super::CliResult {
    let endpoints = args.endpoints.resolve()?;
    let pool = args.db.connect_current().await?;
    let options = if args.strict {
        hardmoney::ParseOptions::STRICT
    } else {
        hardmoney::ParseOptions::LENIENT
    };

    let (filing_id, bytes) = if let Ok(id) = args.filing.parse::<u64>() {
        let id_i64 = i64::try_from(id).map_err(|_| format!("filing id {id} out of range"))?;
        eprintln!(
            "fetching filing {id} from {} ...",
            endpoints.docquery_filing(id)
        );
        (
            args.filing_id.unwrap_or(id_i64),
            hardmoney::fec::download_filing_bytes(id, &endpoints)?,
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

    let resolution = if args.no_resolve {
        bulk::ingest::ChainResolution::Defer
    } else {
        bulk::ingest::ChainResolution::Resolve
    };
    let report =
        bulk::ingest::ingest_filing_bytes_with(&pool, filing_id, &bytes, &options, resolution)
            .await?;
    let chain = match report.chain_rows_resolved {
        Some(n) => format!(", amendment chain resolved ({n} filing(s))"),
        None => {
            ", amendment chain not resolved (--no-resolve; run `hardmoney bulk-resolve-chains` when the batch is done)"
                .to_string()
        }
    };
    println!(
        "ingested filing {} ({}): {} Schedule E line(s), {} skipped{chain}",
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

#[derive(Args, Debug)]
pub struct BulkResolveChainsArgs {
    #[command(flatten)]
    pub db: DbArgs,
}

/// `bulk-resolve-chains`: recompute the amendment-chain columns of every
/// ingested filing in one statement. The follow-up to a batch of
/// `bulk-load-filing --no-resolve`, and the fix-up for a namespace that
/// held filings before migration 0003 (their chain columns are NULL until
/// this runs).
pub async fn resolve_chains(args: BulkResolveChainsArgs) -> super::CliResult {
    let pool = args.db.connect_current().await?;
    let rows = hardmoney::db::resolve_all_amendment_chains(&pool).await?;
    let unresolved: i64 = sqlx::query_scalar("SELECT count(*) FROM filings WHERE chain_unresolved")
        .fetch_one(&pool)
        .await?;
    println!(
        "namespace '{}': amendment chains recomputed for {rows} filing(s); {unresolved} amendment(s) still name an original that is not ingested (chain_unresolved)",
        args.db.schema
    );
    Ok(())
}
