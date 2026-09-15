//! `hardmoney dumps`: a guided way to import the FEC's Postgres dump
//! files, for people who have not used Postgres or a terminal much.
//!
//! Everything here wraps [`hardmoney::bulk::dump`] (download, restore,
//! index) and [`hardmoney::bulk::preflight`] (checks, estimates) and adds
//! plain-language explanations, a plan with time and disk estimates, a
//! confirmation, progress, and next steps. `dumps import --explain`
//! prints the exact `pg_restore` command so nothing is hidden; the
//! `bulk-restore-dump` family remains for scripts that want the raw
//! options.

use std::fmt;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::{Datelike, NaiveDate};
use clap::{Args, Subcommand};
use hardmoney::Cycle;
use hardmoney::bulk::dump::{self, DumpError, DumpSource, RemoteDump, RestoreOptions};
use hardmoney::bulk::preflight::{
    self, ConnectionSummary, ImportNeeds, PreflightInput, Status, fmt_count, fmt_duration_range,
    fmt_elapsed, fmt_rough_count, redact_url,
};
use hardmoney::db::{self, Namespace};
use sqlx::PgPool;

use super::CliResult;
use super::shared::{columns, default_dump_cache_dir};

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

#[derive(Args, Debug)]
pub struct DumpsArgs {
    #[command(flatten)]
    pub common: Common,
    #[command(subcommand)]
    pub command: Option<DumpsCommand>,
}

/// Flags every `dumps` command accepts, before or after the subcommand.
#[derive(Args, Debug, Clone)]
pub struct Common {
    /// Postgres connection URL, e.g. postgres://you@localhost/fec. Read
    /// from DATABASE_URL (or a .env file in the current folder) when not
    /// given.
    #[arg(long, env = "DATABASE_URL", global = true, value_name = "URL")]
    pub database_url: Option<String>,

    /// Namespace (Postgres schema) for hardmoney's own tables and the
    /// friendly views. The FEC's tables always land in the `disclosure`
    /// schema, which every namespace shares. Default: public.
    #[arg(
        long,
        env = "HARDMONEY_SCHEMA",
        default_value = "public",
        global = true
    )]
    pub schema: Namespace,

    /// Folder where downloaded dump files are kept between runs (an
    /// interrupted download resumes from there).
    #[arg(
        long,
        env = "HARDMONEY_CACHE_DIR",
        default_value_os_t = default_dump_cache_dir(),
        global = true,
        value_name = "DIR"
    )]
    pub cache_dir: PathBuf,

    /// Print one JSON object instead of text (for scripts). Implies no
    /// questions are asked.
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum DumpsCommand {
    /// Check that this computer and the database are ready: pg_restore,
    /// the connection, permissions, disk space, and the namespace.
    Check(CheckArgs),
    /// Download one of the FEC's files and load it into the database,
    /// after showing a plan with time and disk estimates.
    Import(ImportArgs),
    /// What is imported, what is downloaded, and whether fec.gov has
    /// newer files than you have.
    Status(StatusArgs),
    /// Re-import whatever is already imported when fec.gov has a newer
    /// file (the FEC refreshes them every weekend).
    Update(UpdateArgs),
    /// Drop an imported table and delete its downloaded file.
    Remove(RemoveArgs),
}

#[derive(Args, Debug)]
pub struct CheckArgs {
    /// Size the disk-space checks for this import (same names as `dumps
    /// import`). Default: all-small.
    #[arg(long = "for", value_name = "WHAT")]
    pub target: Option<What>,
    /// For receipts and disbursements: the cycles the sizing assumes.
    /// Default: the current cycle.
    #[arg(long, value_delimiter = ',', value_name = "YEARS")]
    pub cycles: Vec<Cycle>,
    /// Do not ask fec.gov for current file sizes; use the approximate
    /// ones.
    #[arg(long)]
    pub offline: bool,
    /// If the namespace is not set up yet, run `schema-init` without
    /// asking.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

#[derive(Args, Debug)]
pub struct ImportArgs {
    /// What to import: committees, independent-expenditures, receipts,
    /// disbursements, or all-small (the first two, a few minutes). The
    /// technical names (committee_history, schedule_e, schedule_a_full,
    /// schedule_b_full) work too.
    pub what: What,
    /// For receipts and disbursements: which two-year election cycles to
    /// load, e.g. 2024,2026. Default: the current cycle. Run again with
    /// other cycles to add them.
    #[arg(long, value_delimiter = ',', value_name = "YEARS")]
    pub cycles: Vec<Cycle>,
    /// Do not ask for confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
    /// Print the plan and the exact pg_restore command(s) that would
    /// run, then exit without changing anything.
    #[arg(long)]
    pub explain: bool,
    /// Load from a dump file you already have instead of downloading.
    #[arg(long, value_name = "PATH")]
    pub dump_file: Option<PathBuf>,
    /// pg_restore parallel workers (helps when loading several cycles).
    #[arg(long, default_value_t = 1, value_name = "N")]
    pub jobs: u8,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Do not contact fec.gov (skip the "newer file available" check).
    #[arg(long)]
    pub offline: bool,
}

#[derive(Args, Debug)]
pub struct UpdateArgs {
    /// Do not ask for confirmation (for cron).
    #[arg(long, short = 'y')]
    pub yes: bool,
    /// pg_restore parallel workers.
    #[arg(long, default_value_t = 1, value_name = "N")]
    pub jobs: u8,
}

#[derive(Args, Debug)]
pub struct RemoveArgs {
    /// What to remove (same names as `dumps import`).
    pub what: What,
    /// Do not ask for confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
    /// Drop the table but keep the downloaded file.
    #[arg(long)]
    pub keep_file: bool,
}

pub async fn run(args: DumpsArgs) -> CliResult {
    match args.command {
        None => overview(&args.common).await,
        Some(DumpsCommand::Check(a)) => check(&args.common, a).await,
        Some(DumpsCommand::Import(a)) => import(&args.common, a).await,
        Some(DumpsCommand::Status(a)) => status(&args.common, a).await,
        Some(DumpsCommand::Update(a)) => update(&args.common, a).await,
        Some(DumpsCommand::Remove(a)) => remove(&args.common, a).await,
    }
}

// ---------------------------------------------------------------------------
// Friendly names
// ---------------------------------------------------------------------------

/// What a person asks for, mapped to the FEC's dumps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    /// `committee_history`: one row per committee per cycle.
    Committees,
    /// `schedule_e`: independent expenditures.
    IndependentExpenditures,
    /// `schedule_a_full`: itemized receipts, split by cycle.
    Receipts,
    /// `schedule_b_full`: itemized disbursements, split by cycle.
    Disbursements,
    /// Both small dumps.
    AllSmall,
}

impl What {
    /// The name people type.
    pub fn name(self) -> &'static str {
        match self {
            What::Committees => "committees",
            What::IndependentExpenditures => "independent-expenditures",
            What::Receipts => "receipts",
            What::Disbursements => "disbursements",
            What::AllSmall => "all-small",
        }
    }

    /// The dumps this covers, in import order.
    pub fn sources(self) -> Vec<&'static DumpSource> {
        match self {
            What::Committees => vec![&dump::COMMITTEE_HISTORY],
            What::IndependentExpenditures => vec![&dump::SCHEDULE_E],
            What::Receipts => vec![&dump::SCHEDULE_A],
            What::Disbursements => vec![&dump::SCHEDULE_B],
            What::AllSmall => vec![&dump::COMMITTEE_HISTORY, &dump::SCHEDULE_E],
        }
    }

    /// The friendly name of one dump (`schedule_e` -> `independent-expenditures`).
    pub fn for_source(source: &DumpSource) -> &'static str {
        match source.name {
            "committee_history" => What::Committees.name(),
            "schedule_e" => What::IndependentExpenditures.name(),
            "schedule_a_full" => What::Receipts.name(),
            "schedule_b_full" => What::Disbursements.name(),
            other => other,
        }
    }
}

impl fmt::Display for What {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for What {
    type Err = String;

    /// Accepts the friendly names, the technical dump names, and a few
    /// obvious variants; case and `_`/`-` do not matter.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let key = s.trim().to_ascii_lowercase().replace('_', "-");
        Ok(match key.as_str() {
            "committees" | "committee" | "committee-history" => What::Committees,
            "independent-expenditures"
            | "independent-expenditure"
            | "ies"
            | "ie"
            | "schedule-e"
            | "sched-e" => What::IndependentExpenditures,
            "receipts" | "contributions" | "schedule-a" | "schedule-a-full" | "sched-a" => {
                What::Receipts
            }
            "disbursements" | "schedule-b" | "schedule-b-full" | "sched-b" => What::Disbursements,
            "all-small" | "small" => What::AllSmall,
            _ => {
                return Err(format!(
                    "'{s}' is not something hardmoney can import. Choose one of: committees, \
                     independent-expenditures, receipts, disbursements, all-small (the technical \
                     names committee_history, schedule_e, schedule_a_full, schedule_b_full also work)"
                ));
            }
        })
    }
}

/// The dumps in the order a person meets them: smallest first.
const ORDERED: [&DumpSource; 4] = [
    &dump::COMMITTEE_HISTORY,
    &dump::SCHEDULE_E,
    &dump::SCHEDULE_A,
    &dump::SCHEDULE_B,
];

/// One sentence on what a dump holds.
fn describe(source: &DumpSource) -> &'static str {
    match source.name {
        "committee_history" => {
            "one row per committee per election cycle (name, type, party, treasurer), as fec.gov shows it"
        }
        "schedule_e" => {
            "every independent expenditure the FEC has processed since 1975 (Schedule E: outside spending for or against candidates)"
        }
        "schedule_a_full" => {
            "every itemized receipt since 1975 (Schedule A: who gave money to which committee)"
        }
        "schedule_b_full" => {
            "every itemized disbursement since 1975 (Schedule B: what committees spent money on)"
        }
        _ => "one of the FEC's tables",
    }
}

/// A shorter version for table columns.
fn describe_short(source: &DumpSource) -> &'static str {
    match source.name {
        "committee_history" => "one row per committee per cycle",
        "schedule_e" => "independent expenditures since 1975, Schedule E",
        "schedule_a_full" => "itemized receipts since 1975, Schedule A",
        "schedule_b_full" => "itemized disbursements since 1975, Schedule B",
        _ => "",
    }
}

/// Rows observed in the September 2026 files, for the plan text. `None`
/// for the large dumps, whose counts depend on the cycles chosen.
fn approx_rows(source: &DumpSource) -> Option<u64> {
    match source.name {
        "schedule_e" => Some(549_525),
        "committee_history" => Some(262_276),
        _ => None,
    }
}

/// `about 550,000 rows`, or a phrase for the large dumps.
fn rows_phrase(source: &DumpSource, cycles: &[Cycle]) -> String {
    if let Some(n) = approx_rows(source) {
        return format!("{} rows", fmt_rough_count(n));
    }
    match (source.name, cycles.is_empty()) {
        (_, true) => "hundreds of millions of rows".to_string(),
        ("schedule_a_full", false) => "tens of millions of rows for a recent cycle".to_string(),
        (_, false) => "millions of rows for a recent cycle".to_string(),
    }
}

/// The `dump_*` view [`hardmoney::db::ensure_views`] creates over a dump.
fn view_name(source: &DumpSource) -> &'static str {
    match source.name {
        "committee_history" => "dump_committee_history",
        "schedule_e" => "dump_schedule_e",
        "schedule_a_full" => "dump_schedule_a",
        "schedule_b_full" => "dump_schedule_b",
        _ => "",
    }
}

/// `2026` -> `2025-2026`.
fn cycle_span(cycle: Cycle) -> String {
    let even = cycle.year();
    format!("{}-{even}", even.saturating_sub(1))
}

fn cycles_list(cycles: &[Cycle]) -> String {
    cycles
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// The cycles an import of `source` covers: none for a single-table dump
/// (and `--cycles` is refused for one), the requested ones sorted and
/// de-duplicated, or the cycle containing `today` when none were given.
fn cycles_for(source: &DumpSource, requested: &[Cycle], today: NaiveDate) -> CliResult<Vec<Cycle>> {
    if !source.partitioned_by_cycle {
        if !requested.is_empty() {
            return Err(Friendly::new(
                format!(
                    "{} is one table covering every election cycle, so --cycles does not apply to it.",
                    What::for_source(source)
                ),
                "run the command again without --cycles; the whole table is imported",
            )
            .into());
        }
        return Ok(Vec::new());
    }
    if requested.is_empty() {
        let current = Cycle::containing(today.year()).map_err(|e| {
            Friendly::new(
                format!("could not work out the current election cycle from today's date ({e})."),
                "pass --cycles explicitly, e.g. --cycles 2026",
            )
        })?;
        return Ok(vec![current]);
    }
    let mut cycles = requested.to_vec();
    cycles.sort();
    cycles.dedup();
    Ok(cycles)
}

// ---------------------------------------------------------------------------
// Errors, output, prompts
// ---------------------------------------------------------------------------

/// An error that says what happened and what to do next, in plain words,
/// with the raw message underneath for whoever needs it.
#[derive(Debug)]
pub struct Friendly {
    what: String,
    next: String,
    detail: Option<String>,
}

impl Friendly {
    fn new(what: impl Into<String>, next: impl Into<String>) -> Self {
        Self {
            what: what.into(),
            next: next.into(),
            detail: None,
        }
    }

    fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

impl fmt::Display for Friendly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.what)?;
        for (i, line) in self.next.lines().enumerate() {
            let lead = if i == 0 {
                "what to do: "
            } else {
                "            "
            };
            write!(f, "\n  {lead}{line}")?;
        }
        if let Some(d) = &self.detail {
            write!(f, "\n  details: {d}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Friendly {}

fn no_database_error() -> Friendly {
    Friendly::new(
        "No database is configured: DATABASE_URL is not set and --database-url was not given.",
        format!(
            "{}\nthen run this command again. `hardmoney dumps check` verifies the whole setup.",
            preflight::suggest_database_setup()
        ),
    )
}

/// Turns a [`DumpError`] into words, with the next step.
fn friendly_dump_error(e: DumpError, source: &DumpSource, cycles: &[Cycle]) -> Friendly {
    let name = What::for_source(source);
    let raw = e.to_string();
    match e {
        DumpError::Http { url, .. } => Friendly::new(
            format!("Could not download {name} from {url}."),
            "check your internet connection and run the same command again; an interrupted download resumes where it stopped",
        )
        .detail(raw),
        DumpError::Incomplete {
            expected, actual, ..
        } => Friendly::new(
            format!(
                "The download of {name} stopped early ({} of {} received).",
                dump::fmt_bytes(actual),
                dump::fmt_bytes(expected)
            ),
            "run the same command again; it resumes from where it stopped",
        ),
        DumpError::MissingPartition {
            cycle, available, ..
        } => Friendly::new(
            format!("The FEC's {name} file has no data for cycle {cycle}."),
            format!("choose from the cycles it does have: {available}"),
        ),
        DumpError::NotPartitioned { .. } => Friendly::new(
            format!("{name} is one table covering every cycle; --cycles does not apply to it."),
            "run the command again without --cycles",
        ),
        DumpError::LargeDumpNotAllowed { size, .. } => Friendly::new(
            format!("{name} is about {size}; hardmoney will not load all of it without being told to."),
            format!(
                "pass --cycles with the election cycles you want (e.g. --cycles {}), or use `hardmoney bulk-restore-dump {} --allow-large` for everything",
                cycles_list(cycles),
                source.name
            ),
        ),
        DumpError::DumpFileMissing(path) => Friendly::new(
            format!("The file {} does not exist.", path.display()),
            "check the path given to --dump-file",
        ),
        DumpError::TableMissing { table, .. } => Friendly::new(
            format!("The table disclosure.{table} is not in the database."),
            format!("import it first: hardmoney dumps import {name}"),
        ),
        DumpError::ExternalTool { .. } => Friendly::new(
            format!("pg_restore did not load {name} cleanly."),
            "run `hardmoney dumps check`; if every check passes, the details below are what pg_restore reported and usually name the problem (disk full, permission denied, a damaged download to delete)",
        )
        .detail(raw),
        DumpError::Database(_) => Friendly::new(
            format!("The database refused something while importing {name}."),
            "run `hardmoney dumps check` for permissions and disk space; the details below are what Postgres said",
        )
        .detail(raw),
        DumpError::Io(_) => Friendly::new(
            format!("A file operation failed while importing {name}."),
            "check free disk space and that the download folder is writable (see `hardmoney dumps check`)",
        )
        .detail(raw),
        _ => Friendly::new(
            format!("Importing {name} failed."),
            "run `hardmoney dumps check`; the details below say what went wrong",
        )
        .detail(raw),
    }
}

/// Text to stdout normally, to stderr in `--json` mode so stdout stays
/// one JSON object.
struct Out {
    json: bool,
}

impl Out {
    fn say(&self, text: impl fmt::Display) {
        if self.json {
            eprintln!("{text}");
        } else {
            println!("{text}");
        }
    }

    fn emit(&self, value: &serde_json::Value) -> CliResult {
        if self.json {
            println!("{}", serde_json::to_string_pretty(value)?);
        }
        Ok(())
    }
}

/// Whether it makes sense to ask the person a question.
fn interactive(json: bool) -> bool {
    !json && std::io::stdin().is_terminal()
}

/// Asks a yes/no question on stderr and reads one line.
fn confirm(question: &str, default_yes: bool) -> CliResult<bool> {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    eprint!("{question} {hint} ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let answer = line.trim().to_ascii_lowercase();
    Ok(match answer.as_str() {
        "" => default_yes,
        "y" | "yes" => true,
        _ => false,
    })
}

/// Prints a line every 30 seconds while something slow runs, so a long
/// `pg_restore` does not look hung. Stops when dropped.
struct Heartbeat {
    stop: Arc<AtomicBool>,
}

impl Heartbeat {
    fn start(label: String) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let started = Instant::now();
        std::thread::spawn(move || {
            loop {
                for _ in 0..30 {
                    if flag.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
                eprintln!("  {label} ... {} so far", fmt_elapsed(started.elapsed()));
            }
        });
        Self { stop }
    }
}

impl Drop for Heartbeat {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Opens a pool in the namespace, or explains why it cannot.
async fn connect(common: &Common) -> CliResult<(String, PgPool)> {
    let url = common.database_url.clone().ok_or_else(no_database_error)?;
    let pool = preflight::check_connection(&url, &common.schema, 4)
        .await
        .map_err(|check| Friendly::new(capitalize(&check.detail), check.fix.unwrap_or_default()))?;
    Ok((url, pool))
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// How the URL should appear in a command someone will paste:
/// `"$DATABASE_URL"` when that is where it came from, else the URL with
/// any password hidden.
fn url_for_display(url: &str) -> String {
    if std::env::var("DATABASE_URL").ok().as_deref() == Some(url) {
        "\"$DATABASE_URL\"".to_string()
    } else {
        format!("\"{}\"", redact_url(url))
    }
}

fn schema_flag(ns: &Namespace) -> String {
    if ns.is_public() {
        String::new()
    } else {
        format!(" --schema {ns}")
    }
}

/// A `psql` one-liner. `sql` names views bare; they are qualified with
/// the namespace unless it is `public` (which psql searches by default).
fn psql_line(url: &str, ns: &Namespace, sql: &str) -> String {
    let sql = if ns.is_public() {
        sql.to_string()
    } else {
        sql.replace(" FROM ", &format!(" FROM {ns}."))
    };
    format!("psql {} -c \"{sql}\"", url_for_display(url))
}

/// `rows` under `header`, each column padded to its widest cell, every
/// line indented two spaces and newline-terminated.
fn print_columns<const N: usize>(header: &[&str; N], rows: &[[String; N]]) -> String {
    columns(header, rows)
        .lines()
        .map(|l| format!("  {l}\n"))
        .collect()
}

// ---------------------------------------------------------------------------
// What is where (shared by the overview, status, and update)
// ---------------------------------------------------------------------------

/// Everything known about one dump: on fec.gov, on disk, in the database.
struct DumpView {
    source: &'static DumpSource,
    remote: Option<RemoteDump>,
    remote_error: Option<String>,
    cache: dump::CachedDump,
    state: Option<dump::DumpTableState>,
    last_import: Option<dump::RestoreRecord>,
}

impl DumpView {
    fn name(&self) -> &'static str {
        What::for_source(self.source)
    }

    fn table(&self) -> String {
        format!("disclosure.{}", self.source.disclosure_table)
    }

    fn exists(&self) -> bool {
        self.state.as_ref().is_some_and(|s| s.exists)
    }

    /// Cycles present as child tables, for the split dumps.
    fn cycles_present(&self) -> Vec<Cycle> {
        self.state
            .as_ref()
            .map(|s| {
                s.partitions
                    .iter()
                    .filter_map(|t| dump::partition_cycle(self.source.disclosure_table, t))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// What was recorded about the FEC file the database copy came from:
    /// the last import's ETag/date, else the downloaded file's.
    fn local_version(&self) -> Option<RemoteDump> {
        if let Some(r) = &self.last_import {
            let recorded = r.source_version();
            if recorded.has_validator() {
                return Some(recorded);
            }
        }
        self.cache
            .meta
            .has_validator()
            .then(|| self.cache.meta.clone())
    }

    /// `Some(true)` when fec.gov has a different file than the one the
    /// database copy came from; `None` when that cannot be told.
    fn fec_has_newer(&self) -> Option<bool> {
        let remote = self.remote.as_ref()?;
        let local = self.local_version()?;
        remote.is_newer_than(&local)
    }

    fn json(&self) -> serde_json::Value {
        let cycles: Vec<u16> = self.cycles_present().iter().map(|c| c.year()).collect();
        serde_json::json!({
            "name": self.name(),
            "dump": self.source.name,
            "table": self.table(),
            "view": view_name(self.source),
            "description": describe(self.source),
            "fec": self.remote.as_ref().map(|r| serde_json::json!({
                "size_bytes": r.size,
                "last_modified": r.last_modified.map(|t| t.to_rfc3339()),
                "etag": r.etag,
            })),
            "fec_error": self.remote_error,
            "approx_size_bytes": self.source.approx_size_bytes,
            "downloaded": {
                "path": self.cache.path.display().to_string(),
                "complete_bytes": self.cache.complete_bytes,
                "partial_bytes": self.cache.partial_bytes,
                "last_modified": self.cache.meta.last_modified.map(|t| t.to_rfc3339()),
            },
            "database": self.state.as_ref().map(|s| serde_json::json!({
                "exists": s.exists,
                "approx_rows": s.approx_rows,
                "index_count": s.index_count,
                "cycles": cycles,
            })),
            "last_import": self.last_import.as_ref().map(|r| serde_json::json!({
                "at": r.restored_at.to_rfc3339(),
                "rows": r.rows,
                "cycle": r.cycle.map(|c| c.year()),
                "mode": r.mode,
                "source_last_modified": r.source_last_modified.map(|t| t.to_rfc3339()),
            })),
            "fec_has_newer": self.fec_has_newer(),
        })
    }
}

enum Database {
    NotConfigured,
    Unreachable {
        what: String,
        next: String,
        raw: String,
    },
    Connected {
        url: String,
        summary: Option<ConnectionSummary>,
        namespace_ready: bool,
        pool: PgPool,
    },
}

impl Database {
    fn json(&self, ns: &Namespace) -> serde_json::Value {
        match self {
            Database::NotConfigured => serde_json::json!({ "configured": false }),
            Database::Unreachable { what, next, raw } => serde_json::json!({
                "configured": true, "reachable": false, "problem": what, "fix": next, "error": raw,
            }),
            Database::Connected {
                summary,
                namespace_ready,
                ..
            } => serde_json::json!({
                "configured": true,
                "reachable": true,
                "name": summary.as_ref().map(|s| s.database.clone()),
                "host": summary.as_ref().map(|s| s.host.clone()),
                "namespace": ns.to_string(),
                "namespace_ready": namespace_ready,
            }),
        }
    }
}

struct Snapshot {
    dumps: Vec<DumpView>,
    database: Database,
}

impl Snapshot {
    fn json(&self, common: &Common) -> serde_json::Value {
        serde_json::json!({
            "dumps": self.dumps.iter().map(DumpView::json).collect::<Vec<_>>(),
            "database": self.database.json(&common.schema),
            "cache_dir": common.cache_dir.display().to_string(),
        })
    }
}

async fn snapshot(common: &Common, offline: bool) -> CliResult<Snapshot> {
    let database = match &common.database_url {
        None => Database::NotConfigured,
        Some(url) => match preflight::check_connection(url, &common.schema, 4).await {
            Ok(pool) => {
                let namespace_ready = db::migration_status(&pool)
                    .await
                    .map(|s| s.is_current())
                    .unwrap_or(false);
                Database::Connected {
                    url: url.clone(),
                    summary: preflight::summarize_url(url),
                    namespace_ready,
                    pool,
                }
            }
            Err(check) => Database::Unreachable {
                what: check.detail.clone(),
                next: check.fix.clone().unwrap_or_default(),
                raw: check.detail,
            },
        },
    };
    let history = match &database {
        Database::Connected { pool, .. } => dump::restore_history(pool)
            .await
            .map_err(|e| friendly_dump_error(e, &dump::SCHEDULE_E, &[]))?,
        _ => Vec::new(),
    };
    let mut dumps = Vec::with_capacity(ORDERED.len());
    for source in ORDERED {
        let (remote, remote_error) = if offline {
            (None, None)
        } else {
            match tokio::task::spawn_blocking(move || dump::remote_info(source)).await? {
                Ok(r) => (Some(r), None),
                Err(e) => (None, Some(e.to_string())),
            }
        };
        let state = match &database {
            Database::Connected { pool, .. } => Some(
                dump::table_state(pool, source)
                    .await
                    .map_err(|e| friendly_dump_error(e, source, &[]))?,
            ),
            _ => None,
        };
        dumps.push(DumpView {
            source,
            remote,
            remote_error,
            cache: dump::cached(source, &common.cache_dir),
            state,
            last_import: history.iter().find(|r| r.dump == source.name).cloned(),
        });
    }
    Ok(Snapshot { dumps, database })
}

fn size_cell(view: &DumpView) -> String {
    match view.remote.as_ref().and_then(|r| r.size) {
        Some(n) => dump::fmt_bytes(n),
        None => format!("~{}", dump::fmt_bytes(view.source.approx_size_bytes)),
    }
}

fn downloaded_cell(view: &DumpView) -> String {
    match (view.cache.complete_bytes, view.cache.partial_bytes) {
        (Some(n), _) => format!("yes ({})", dump::fmt_bytes(n)),
        (None, Some(n)) => format!("partly ({} so far)", dump::fmt_bytes(n)),
        (None, None) => "no".to_string(),
    }
}

fn database_cell(view: &DumpView) -> String {
    let Some(state) = &view.state else {
        return "?".to_string();
    };
    if !state.exists {
        return "no".to_string();
    }
    let rows = fmt_count(u64::try_from(state.approx_rows).unwrap_or(0));
    let when = view
        .last_import
        .as_ref()
        .map(|r| format!(", imported {}", r.restored_at.format("%Y-%m-%d")))
        .unwrap_or_default();
    let cycles = view.cycles_present();
    if cycles.is_empty() {
        format!("about {rows} rows{when}")
    } else {
        format!(
            "cycles {} (about {rows} rows{when})",
            cycles
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

// ---------------------------------------------------------------------------
// `hardmoney dumps` (no subcommand): where am I?
// ---------------------------------------------------------------------------

async fn overview(common: &Common) -> CliResult {
    let out = Out { json: common.json };
    let snap = snapshot(common, false).await?;
    if common.json {
        return out.emit(&snap.json(common));
    }

    println!(
        "The FEC publishes four tables from its own database as Postgres \"dump\" files,\n\
         refreshed every weekend. hardmoney downloads a file and loads it into your\n\
         Postgres database, where it becomes an ordinary table you can query with SQL.\n"
    );
    let db_label = match &snap.database {
        Database::Connected { summary, .. } => summary
            .as_ref()
            .map(|s| format!("in database {}", s.database))
            .unwrap_or_else(|| "in database".to_string()),
        _ => "in database".to_string(),
    };
    let header = [
        "name",
        "what it holds",
        "on fec.gov",
        "downloaded",
        db_label.as_str(),
    ];
    let rows: Vec<[String; 5]> = snap
        .dumps
        .iter()
        .map(|v| {
            [
                v.name().to_string(),
                describe_short(v.source).to_string(),
                size_cell(v),
                downloaded_cell(v),
                database_cell(v),
            ]
        })
        .collect();
    print!("{}", print_columns(&header, &rows));
    if snap.dumps.iter().any(|v| v.remote.is_none()) {
        println!(
            "  (sizes marked ~ are approximate: fec.gov could not be reached for the current figure)"
        );
    }
    println!("\nDownloads are kept in {}.", common.cache_dir.display());

    match &snap.database {
        Database::NotConfigured => {
            println!(
                "\nNo database is configured yet, so the last column is unknown. To set one up:\n{}",
                indent(&preflight::suggest_database_setup(), "  ")
            );
            println!("\nNext:\n  hardmoney dumps check          check that this computer is ready");
        }
        Database::Unreachable { what, next, .. } => {
            println!("\nThe database could not be reached: {what}\n  what to do: {next}");
            println!(
                "\nNext:\n  hardmoney dumps check          run every check and see what to fix"
            );
        }
        Database::Connected {
            summary,
            namespace_ready,
            ..
        } => {
            let ns = &common.schema;
            println!(
                "\nDatabase: {} (namespace '{ns}'{})",
                summary
                    .as_ref()
                    .map(|s| format!(
                        "{} on {}",
                        s.database,
                        if s.host.is_empty() {
                            "localhost"
                        } else {
                            s.host.as_str()
                        }
                    ))
                    .unwrap_or_else(|| "connected".to_string()),
                if *namespace_ready {
                    ""
                } else {
                    ", not set up for hardmoney yet"
                }
            );
            let imported: Vec<&DumpView> = snap.dumps.iter().filter(|v| v.exists()).collect();
            println!("\nNext:");
            if imported.is_empty() {
                println!(
                    "  hardmoney dumps check                 check that this computer is ready"
                );
                println!(
                    "  hardmoney dumps import all-small      import the two small files (a few minutes)"
                );
            } else {
                let receipts = snap
                    .dumps
                    .iter()
                    .find(|v| v.source.name == "schedule_a_full");
                if let Some(receipts) = receipts.filter(|v| !v.exists()) {
                    println!(
                        "  hardmoney dumps import receipts       itemized receipts for the current cycle (hours; the download is {})",
                        size_cell(receipts)
                    );
                }
                println!(
                    "  hardmoney dumps status                details, and whether fec.gov has newer files"
                );
                println!(
                    "  hardmoney dumps update                re-import what you have when fec.gov has newer files"
                );
            }
        }
    }
    Ok(())
}

fn indent(text: &str, by: &str) -> String {
    text.lines()
        .map(|l| format!("{by}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// `hardmoney dumps check`
// ---------------------------------------------------------------------------

/// Sizes the import of `what` (remote sizes unless `offline`), for the
/// disk-space checks.
async fn needs_for(
    what: What,
    requested: &[Cycle],
    cache_dir: &Path,
    offline: bool,
    today: NaiveDate,
) -> CliResult<ImportNeeds> {
    let today_cycle = Cycle::containing(today.year()).map_err(|e| {
        Friendly::new(
            format!("could not work out the current cycle ({e})."),
            "pass --cycles",
        )
    })?;
    let mut total: Option<ImportNeeds> = None;
    for source in what.sources() {
        let cycles = cycles_for(source, requested, today)?;
        let cache = dump::cached(source, cache_dir);
        let (archive_bytes, download_needed) = match cache.complete_bytes {
            Some(n) => (n, false),
            None => {
                let size = if offline {
                    None
                } else {
                    tokio::task::spawn_blocking(move || dump::remote_info(source))
                        .await?
                        .ok()
                        .and_then(|r| r.size)
                };
                (size.unwrap_or(source.approx_size_bytes), true)
            }
        };
        let needs = preflight::estimate(
            source,
            archive_bytes,
            download_needed,
            &cycles,
            false,
            today_cycle,
        );
        total = Some(match total {
            Some(t) => t.plus(&needs),
            None => needs,
        });
    }
    total.ok_or_else(|| Friendly::new("Nothing to size.", "choose something to import").into())
}

async fn check(common: &Common, args: CheckArgs) -> CliResult {
    let out = Out { json: common.json };
    let what = args.target.unwrap_or(What::AllSmall);
    let today = chrono::Utc::now().date_naive();
    let needs = needs_for(what, &args.cycles, &common.cache_dir, args.offline, today).await?;
    out.say(format!(
        "Checking whether this computer can import {what} (needs about {} of disk: {}, about {} inside Postgres) ...\n",
        dump::fmt_bytes(needs.total_bytes()),
        if needs.download_bytes == 0 {
            "nothing to download".to_string()
        } else {
            format!("{} to download", dump::fmt_bytes(needs.download_bytes))
        },
        dump::fmt_bytes(needs.database_bytes)
    ));
    let pf = preflight::run(&PreflightInput {
        database_url: common.database_url.as_deref(),
        namespace: &common.schema,
        cache_dir: &common.cache_dir,
        needs: Some(&needs),
    })
    .await;
    out.say(&pf);

    // Offer to set the namespace up, which is the one warning we can fix.
    let mut namespace_fixed = false;
    if pf
        .get("namespace")
        .is_some_and(|c| c.status != Status::Pass)
        && pf.passed()
        && (args.yes
            || (interactive(common.json)
                && confirm(
                    &format!(
                        "Set up namespace '{}' for hardmoney now? (runs `hardmoney schema-init`; it only adds hardmoney's own tables)",
                        common.schema
                    ),
                    true,
                )?))
    {
        let (_, pool) = connect(common).await?;
        db::migrate(&pool).await?;
        out.say(format!("✓ namespace: '{}' is now set up.", common.schema));
        namespace_fixed = true;
    }

    if common.json {
        out.emit(&serde_json::json!({
            "ok": pf.passed(),
            "worst": pf.worst(),
            "checks": pf.checks,
            "namespace_initialised_now": namespace_fixed,
            "sized_for": what.name(),
            "needs": needs,
        }))?;
    }
    if pf.passed() {
        let warnings = pf
            .checks
            .iter()
            .filter(|c| c.status == Status::Warn)
            .count()
            .saturating_sub(usize::from(namespace_fixed));
        out.say(format!(
            "{}\nNext: hardmoney dumps import {}{}",
            if warnings == 0 {
                "Everything looks ready.".to_string()
            } else {
                format!("Ready, with {warnings} thing(s) worth knowing above (marked !).")
            },
            what.name(),
            schema_flag(&common.schema)
        ));
        Ok(())
    } else {
        Err(Friendly::new(
            format!("{} check(s) failed.", pf.failures().count()),
            "fix the items marked ✗ above, then run `hardmoney dumps check` again",
        )
        .into())
    }
}

// ---------------------------------------------------------------------------
// `hardmoney dumps import`
// ---------------------------------------------------------------------------

/// Where the archive for one step comes from.
enum Location {
    /// Not on disk yet; `resume_from` is the size of a partial download.
    Download { resume_from: Option<u64> },
    /// Already downloaded into the cache.
    Cached(PathBuf),
    /// A file the person supplied with `--dump-file`.
    File(PathBuf),
}

/// One dump to import and everything the plan says about it.
struct Step {
    source: &'static DumpSource,
    cycles: Vec<Cycle>,
    location: Location,
    archive_bytes: u64,
    /// Whether `archive_bytes` is measured (cache, file, or `HEAD`) rather
    /// than the approximate constant.
    size_exact: bool,
    needs: ImportNeeds,
    /// Whether `disclosure.<table>` is already in the database (a
    /// cycle-selective restore then leaves it alone).
    parent_exists: bool,
}

impl Step {
    fn name(&self) -> &'static str {
        What::for_source(self.source)
    }

    /// A cycle-selective restore is always data only.
    fn data_only(&self) -> bool {
        self.source.partitioned_by_cycle && !self.cycles.is_empty()
    }

    /// The tables the rows land in.
    fn target_tables(&self) -> Vec<String> {
        if self.cycles.is_empty() {
            vec![self.source.disclosure_table.to_string()]
        } else {
            self.cycles
                .iter()
                .map(|c| dump::partition_table(self.source.disclosure_table, *c))
                .collect()
        }
    }

    fn options(&self, jobs: u8) -> RestoreOptions {
        RestoreOptions::new()
            .cycles(self.cycles.iter().copied())
            .data_only(self.data_only())
            .jobs(jobs)
            .dump_file(match &self.location {
                Location::File(p) => Some(p.clone()),
                _ => None,
            })
    }

    fn archive_path(&self, cache_dir: &Path) -> PathBuf {
        match &self.location {
            Location::File(p) | Location::Cached(p) => p.clone(),
            Location::Download { .. } => dump::cache_path(self.source, cache_dir),
        }
    }

    /// The sentence describing this step in the plan.
    fn describe(&self, database: &str) -> String {
        let name = self.name();
        let size = format!(
            "{}{}",
            if self.size_exact { "" } else { "~" },
            dump::fmt_bytes(self.archive_bytes)
        );
        let fetch = match &self.location {
            Location::Download { resume_from: None } if self.source.partitioned_by_cycle => {
                format!(
                    "download the whole {size} file from fec.gov (the FEC does not offer per-cycle files; {} depending on your connection, and an interrupted download resumes)",
                    self.needs
                        .download_time
                        .map(|(lo, hi)| fmt_duration_range(lo, hi))
                        .unwrap_or_default()
                )
            }
            Location::Download { resume_from: None } => format!(
                "download {size} from fec.gov ({} on a typical connection)",
                self.needs
                    .download_time
                    .map(|(lo, hi)| fmt_duration_range(lo, hi))
                    .unwrap_or_default()
            ),
            Location::Download {
                resume_from: Some(have),
            } => format!(
                "finish downloading the {size} file from fec.gov ({} already here)",
                dump::fmt_bytes(*have)
            ),
            Location::Cached(p) => {
                format!("use the {size} file already downloaded to {}", p.display())
            }
            Location::File(p) => format!("use the {size} file {}", p.display()),
        };
        let rows = rows_phrase(self.source, &self.cycles);
        let tables = self
            .target_tables()
            .iter()
            .map(|t| format!("disclosure.{t}"))
            .collect::<Vec<_>>()
            .join(" and ");
        let (lo, hi) = self.needs.restore_time;
        let load = if self.cycles.is_empty() {
            format!(
                "load {rows} into table {tables} in database {database} ({})",
                fmt_duration_range(lo, hi)
            )
        } else {
            format!(
                "load only the {} rows ({rows}) into {tables} in database {database}, without the FEC's own indexes ({}), then add hardmoney's {} indexes per table",
                self.cycles
                    .iter()
                    .map(|c| cycle_span(*c))
                    .collect::<Vec<_>>()
                    .join(", "),
                fmt_duration_range(lo, hi),
                self.source.indexes.len()
            )
        };
        let cycles = if self.cycles.is_empty() {
            String::new()
        } else {
            format!(
                " (cycle{} {})",
                if self.cycles.len() == 1 { "" } else { "s" },
                self.cycles
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        format!("{name}{cycles}: {fetch}, then {load}.")
    }

    /// The statements and command that will run, for `--explain`.
    fn explain(&self, url: &str, cache_dir: &Path, jobs: u8) -> CliResult<Vec<String>> {
        let plan = dump::plan_restore_offline(self.source, &self.options(jobs), self.parent_exists)
            .map_err(|e| friendly_dump_error(e, self.source, &self.cycles))?;
        let mut lines = vec!["CREATE SCHEMA IF NOT EXISTS disclosure;".to_string()];
        let dropped: Vec<String> = if plan.partitions.is_empty() {
            vec![self.source.disclosure_table.to_string()]
        } else {
            plan.partitions.iter().map(|p| p.table.clone()).collect()
        };
        for t in &dropped {
            lines.push(format!("DROP TABLE IF EXISTS disclosure.{t} CASCADE;"));
        }
        lines.push(shell_join(&dump::pg_restore_command(
            &plan,
            &redact_url(url),
            &self.archive_path(cache_dir),
        )));
        if plan.data_only {
            for table in self.target_tables() {
                for spec in self.source.indexes {
                    let name = format!("hm_{table}_{}", spec.suffix);
                    lines.push(if spec.trigram {
                        format!(
                            "CREATE INDEX {name} ON disclosure.{table} USING gin ({} gin_trgm_ops);  -- skipped if pg_trgm is unavailable",
                            spec.columns
                        )
                    } else {
                        format!(
                            "CREATE {}INDEX {name} ON disclosure.{table} ({});",
                            if spec.unique { "UNIQUE " } else { "" },
                            spec.columns
                        )
                    });
                }
            }
        }
        Ok(lines)
    }

    fn json(&self, cache_dir: &Path) -> serde_json::Value {
        serde_json::json!({
            "name": self.name(),
            "dump": self.source.name,
            "tables": self.target_tables().iter().map(|t| format!("disclosure.{t}")).collect::<Vec<_>>(),
            "cycles": self.cycles.iter().map(|c| c.year()).collect::<Vec<_>>(),
            "data_only": self.data_only(),
            "archive_bytes": self.archive_bytes,
            "archive_size_exact": self.size_exact,
            "archive_path": self.archive_path(cache_dir).display().to_string(),
            "download_needed": matches!(self.location, Location::Download { .. }),
            "needs": self.needs,
        })
    }
}

/// Joins arguments for display, quoting any that contain spaces.
fn shell_join(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if a.is_empty() || a.contains(|c: char| c.is_whitespace() || c == '"' || c == '\'') {
                format!("'{}'", a.replace('\'', "'\\''"))
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What every step of one `import` or `update` run is planned against.
struct Planning<'a> {
    /// `--dump-file`, for a single-dump import.
    dump_file: Option<&'a Path>,
    cache_dir: &'a Path,
    /// For asking whether the parent table already exists.
    pool: Option<&'a PgPool>,
    /// Ignore a cached file: `update` deletes it before importing so the
    /// FEC's current one is fetched.
    fresh_download: bool,
    today: NaiveDate,
    today_cycle: Cycle,
}

/// Plans one dump.
async fn plan_step(
    source: &'static DumpSource,
    requested: &[Cycle],
    ctx: &Planning<'_>,
) -> CliResult<Step> {
    let cycles = cycles_for(source, requested, ctx.today)?;
    let cache = dump::cached(source, ctx.cache_dir);
    let complete = if ctx.fresh_download {
        None
    } else {
        cache.complete_bytes
    };
    let (location, archive_bytes, size_exact) = match (ctx.dump_file, complete) {
        (Some(path), _) => {
            let len = std::fs::metadata(path)
                .map_err(|e| {
                    Friendly::new(
                        format!("The file {} cannot be read.", path.display()),
                        "check the path given to --dump-file",
                    )
                    .detail(e.to_string())
                })?
                .len();
            (Location::File(path.to_path_buf()), len, true)
        }
        (None, Some(n)) => (Location::Cached(cache.path.clone()), n, true),
        (None, None) => {
            let remote = tokio::task::spawn_blocking(move || dump::remote_info(source))
                .await?
                .ok();
            let size = remote.and_then(|r| r.size);
            (
                Location::Download {
                    resume_from: cache.partial_bytes.filter(|_| !ctx.fresh_download),
                },
                size.unwrap_or(source.approx_size_bytes),
                size.is_some(),
            )
        }
    };
    let selective = source.partitioned_by_cycle && !cycles.is_empty();
    let needs = preflight::estimate(
        source,
        archive_bytes,
        matches!(location, Location::Download { .. }),
        &cycles,
        selective,
        ctx.today_cycle,
    );
    let parent_exists = match ctx.pool {
        Some(pool) => dump::table_state(pool, source)
            .await
            .map(|s| s.exists)
            .unwrap_or(false),
        None => false,
    };
    Ok(Step {
        source,
        cycles,
        location,
        archive_bytes,
        size_exact,
        needs,
        parent_exists,
    })
}

fn total_needs(steps: &[Step]) -> Option<ImportNeeds> {
    steps.iter().fold(None, |acc, s| {
        Some(match acc {
            Some(t) => t.plus(&s.needs),
            None => s.needs.clone(),
        })
    })
}

/// The plan, in words.
fn render_plan(steps: &[Step], database: &str, cache_dir: &Path) -> String {
    let mut text = String::from("Plan\n");
    for (i, step) in steps.iter().enumerate() {
        text.push_str(&format!("  {}. {}\n", i + 1, step.describe(database)));
    }
    if let Some(total) = total_needs(steps) {
        let (lo, hi) = total.total_time();
        let disk = if total.download_bytes == 0 {
            format!(
                "about {} inside Postgres (nothing to download)",
                dump::fmt_bytes(total.database_bytes)
            )
        } else {
            format!(
                "about {} in total ({} of downloads in {}, about {} inside Postgres)",
                dump::fmt_bytes(total.total_bytes()),
                dump::fmt_bytes(total.download_bytes),
                cache_dir.display(),
                dump::fmt_bytes(total.database_bytes)
            )
        };
        text.push_str(&format!(
            "\nDisk: {disk}.\n\
             Time: {} in total. These are estimates scaled from the FEC's own published timings;\n\
             your connection and hardware decide the real figure.\n",
            fmt_duration_range(lo, hi),
        ));
    }
    text
}

/// What a finished step reports.
struct StepResult {
    source: &'static DumpSource,
    report: dump::RestoreReport,
    indexes: Option<dump::IndexReport>,
    elapsed: Duration,
}

impl StepResult {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": What::for_source(self.source),
            "dump": self.source.name,
            "table": format!("disclosure.{}", self.report.table),
            "tables": self.report.tables.iter().map(|t| format!("disclosure.{t}")).collect::<Vec<_>>(),
            "cycles": self.report.cycles.iter().map(|c| c.year()).collect::<Vec<_>>(),
            "rows": self.report.rows,
            "data_only": self.report.data_only,
            "indexes_created": self.indexes.as_ref().map(|i| i.created.len()),
            "indexes_skipped": self.indexes.as_ref().map(|i| i.skipped.len()),
            "dump_path": self.report.dump_path.display().to_string(),
            "recorded": !self.report.load_ids.is_empty(),
            "elapsed_secs": self.elapsed.as_secs_f64(),
        })
    }
}

/// Runs one step: download if needed (progress bar), restore (heartbeat),
/// hardmoney's indexes for a data-only restore (heartbeat).
async fn execute_step(
    pool: &PgPool,
    url: &str,
    step: &Step,
    cache_dir: &Path,
    jobs: u8,
    out: &Out,
) -> CliResult<StepResult> {
    let started = Instant::now();
    let name = step.name();
    let source = step.source;
    match &step.location {
        Location::File(p) => out.say(format!("{name}: using {}", p.display())),
        Location::Cached(p) => {
            out.say(format!("{name}: using the downloaded file {}", p.display()))
        }
        Location::Download { resume_from } => {
            out.say(format!(
                "{name}: downloading {} from fec.gov{} ...",
                dump::fmt_bytes(step.archive_bytes),
                resume_from
                    .map(|n| format!(" (resuming after {})", dump::fmt_bytes(n)))
                    .unwrap_or_default()
            ));
            let dir = cache_dir.to_path_buf();
            let cached = tokio::task::spawn_blocking(move || dump::download(source, &dir))
                .await?
                .map_err(|e| friendly_dump_error(e, source, &step.cycles))?;
            out.say(format!(
                "{name}: downloaded to {} ({})",
                cached.path.display(),
                cached
                    .complete_bytes
                    .map(dump::fmt_bytes)
                    .unwrap_or_default()
            ));
        }
    }

    let tables = step.target_tables();
    out.say(format!(
        "{name}: loading into disclosure.{} with pg_restore (this is the slow part; a line appears every 30 seconds while it runs) ...",
        tables.join(", disclosure.")
    ));
    let report = {
        let _beat = Heartbeat::start(format!("{name}: still loading"));
        dump::restore_with(pool, url, source, cache_dir, &step.options(jobs)).await
    }
    .map_err(|e| friendly_dump_error(e, source, &step.cycles))?;
    out.say(format!(
        "{name}: {} rows loaded",
        fmt_count(u64::try_from(report.rows).unwrap_or(0))
    ));

    let indexes = if report.data_only {
        out.say(format!(
            "{name}: adding hardmoney's indexes (fast lookups by committee, date, and name) ..."
        ));
        let ix = {
            let _beat = Heartbeat::start(format!("{name}: still indexing"));
            dump::create_indexes(pool, source, &report.cycles).await
        }
        .map_err(|e| friendly_dump_error(e, source, &step.cycles))?;
        out.say(format!(
            "{name}: {} index(es) added{}",
            ix.created.len(),
            if ix.skipped.is_empty() {
                String::new()
            } else {
                format!(
                    ", {} skipped ({})",
                    ix.skipped.len(),
                    ix.skipped
                        .iter()
                        .map(|s| s.reason.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            }
        ));
        Some(ix)
    } else {
        None
    };
    Ok(StepResult {
        source,
        report,
        indexes,
        elapsed: started.elapsed(),
    })
}

/// Makes sure `schema-init` has run in the namespace, asking first.
async fn ensure_namespace(
    pool: &PgPool,
    ns: &Namespace,
    yes: bool,
    ask: bool,
    out: &Out,
) -> CliResult {
    let status = db::migration_status(pool).await?;
    if status.is_current() {
        return Ok(());
    }
    if status.applied.is_empty() {
        out.say(format!(
            "Namespace '{ns}' has not been set up for hardmoney yet. (A namespace is the part of the\n\
             database where hardmoney keeps its own tables and the friendly views; setting it up\n\
             adds those tables and touches nothing else.)"
        ));
    } else {
        out.say(format!(
            "Namespace '{ns}' needs upgrading ({} pending migration(s)).",
            status.pending.len()
        ));
    }
    let go = yes || !ask || confirm("Set it up now? (runs `hardmoney schema-init`)", true)?;
    if !go {
        return Err(Friendly::new(
            "The namespace is not set up, so the import cannot be recorded.",
            format!("run `hardmoney schema-init --schema {ns}`, then this command again"),
        )
        .into());
    }
    db::migrate(pool).await?;
    out.say(format!("Namespace '{ns}' is set up."));
    Ok(())
}

/// Three copy-pasteable things to do with a freshly imported dump.
fn next_steps(source: &DumpSource, url: &str, ns: &Namespace) -> Vec<String> {
    let view = view_name(source);
    let mut lines = Vec::new();
    if source.name == "schedule_e" {
        lines.push(format!(
            "hardmoney query ies --candidate S6OH00163 --limit 5{}",
            schema_flag(ns)
        ));
        lines.push(psql_line(url, ns, &format!("SELECT count(*) FROM {view};")));
        lines.push(psql_line(
            url,
            ns,
            "SELECT expenditure_date::date, expenditure_amt, support_oppose_code, candidate_name, cmte_id FROM independent_expenditures ORDER BY expenditure_amt DESC NULLS LAST LIMIT 5;",
        ));
    } else if source.name == "committee_history" {
        lines.push(psql_line(url, ns, &format!("SELECT count(*) FROM {view};")));
        lines.push(psql_line(
            url,
            ns,
            &format!("SELECT cycle, committee_id, name, committee_type, party FROM {view} WHERE name ILIKE '%actblue%' ORDER BY cycle DESC LIMIT 5;"),
        ));
    } else {
        lines.push(psql_line(url, ns, &format!("SELECT count(*) FROM {view};")));
        lines.push(psql_line(
            url,
            ns,
            &format!("SELECT cycle, count(*) FROM {view} GROUP BY cycle ORDER BY cycle;"),
        ));
    }
    lines
}

async fn import(common: &Common, args: ImportArgs) -> CliResult {
    let out = Out { json: common.json };
    let ask = interactive(common.json);
    let sources = args.what.sources();
    if args.dump_file.is_some() && sources.len() > 1 {
        return Err(Friendly::new(
            "--dump-file names one file, but all-small imports two dumps.",
            "run `hardmoney dumps import committees --dump-file ...` and `hardmoney dumps import independent-expenditures --dump-file ...` separately",
        )
        .into());
    }
    let url = common.database_url.clone().ok_or_else(no_database_error)?;
    let summary = preflight::summarize_url(&url);
    let database = summary
        .as_ref()
        .map(|s| s.database.clone())
        .unwrap_or_else(|| "?".to_string());
    let today = chrono::Utc::now().date_naive();
    let today_cycle = Cycle::containing(today.year()).map_err(|e| {
        Friendly::new(
            format!("could not work out the current election cycle ({e})."),
            "pass --cycles explicitly",
        )
    })?;

    // A connection for planning (does the parent table exist?) if one can
    // be had; preflight reports properly if it cannot.
    let planning_pool = preflight::check_connection(&url, &common.schema, 2)
        .await
        .ok();
    let mut steps = Vec::with_capacity(sources.len());
    {
        let ctx = Planning {
            dump_file: args.dump_file.as_deref(),
            cache_dir: &common.cache_dir,
            pool: planning_pool.as_ref(),
            fresh_download: false,
            today,
            today_cycle,
        };
        for source in sources {
            steps.push(plan_step(source, &args.cycles, &ctx).await?);
        }
    }
    if let Some(p) = planning_pool {
        p.close().await;
    }
    let needs = total_needs(&steps);

    out.say("Checking this computer and the database ...\n");
    let pf = preflight::run(&PreflightInput {
        database_url: Some(&url),
        namespace: &common.schema,
        cache_dir: &common.cache_dir,
        needs: needs.as_ref(),
    })
    .await;
    out.say(&pf);
    let checks_failed = || {
        Friendly::new(
            format!(
                "{} check(s) failed, so nothing was changed.",
                pf.failures().count()
            ),
            "fix the items marked ✗ above, then run this command again",
        )
    };
    if !pf.passed() && !args.explain {
        out.emit(&serde_json::json!({ "ok": false, "checks": pf.checks }))?;
        return Err(checks_failed().into());
    }

    out.say(render_plan(&steps, &database, &common.cache_dir));
    if args.explain {
        let mut commands = Vec::new();
        out.say("What will run, exactly:");
        for step in &steps {
            out.say(format!("  {}:", step.name()));
            for line in step.explain(&url, &common.cache_dir, args.jobs)? {
                out.say(format!("    {line}"));
                commands.push(line);
            }
            out.say(format!(
                "    (then hardmoney recreates the {} in namespace '{}' and records the import in its loads table)",
                if step.source.name == "schedule_e" {
                    "views dump_schedule_e and independent_expenditures".to_string()
                } else {
                    format!("view {}", view_name(step.source))
                },
                common.schema
            ));
        }
        out.say("\nNothing was changed (--explain).");
        out.emit(&serde_json::json!({
            "ok": pf.passed(),
            "explain": true,
            "plan": steps.iter().map(|s| s.json(&common.cache_dir)).collect::<Vec<_>>(),
            "pg_restore_commands": commands.iter().filter(|c| c.starts_with("pg_restore")).collect::<Vec<_>>(),
            "statements": commands,
            "checks": pf.checks,
        }))?;
        return if pf.passed() {
            Ok(())
        } else {
            Err(checks_failed().into())
        };
    }

    if !args.yes {
        if ask {
            if !confirm("Continue?", false)? {
                out.say("Nothing was changed.");
                return Ok(());
            }
        } else if !common.json {
            out.say("(stdin is not a terminal, so no confirmation was asked; pass --yes to make that explicit)");
        }
    }

    let (url, pool) = connect(common).await?;
    ensure_namespace(&pool, &common.schema, args.yes, ask, &out).await?;

    let started = Instant::now();
    let mut results = Vec::with_capacity(steps.len());
    for (i, step) in steps.iter().enumerate() {
        if i > 0 {
            out.say("");
        }
        results.push(execute_step(&pool, &url, step, &common.cache_dir, args.jobs, &out).await?);
    }
    let elapsed = started.elapsed();

    if common.json {
        return out.emit(&serde_json::json!({
            "ok": true,
            "elapsed_secs": elapsed.as_secs_f64(),
            "imports": results.iter().map(StepResult::json).collect::<Vec<_>>(),
            "checks": pf.checks,
        }));
    }

    println!("\nDone in {}.", fmt_elapsed(elapsed));
    for r in &results {
        println!(
            "  {}: {} rows in disclosure.{}{} ({}{})",
            What::for_source(r.source),
            fmt_count(u64::try_from(r.report.rows).unwrap_or(0)),
            r.report.table,
            if r.report.cycles.is_empty() {
                String::new()
            } else {
                format!(" for cycle(s) {}", cycles_list(&r.report.cycles))
            },
            fmt_elapsed(r.elapsed),
            r.indexes
                .as_ref()
                .map(|i| format!("; {} index(es) added", i.created.len()))
                .unwrap_or_default()
        );
        if r.report.load_ids.is_empty() {
            println!(
                "    (not recorded: namespace '{}' has no loads table)",
                common.schema
            );
        }
    }
    println!("\nTry it:");
    for r in &results {
        for line in next_steps(r.source, &url, &common.schema) {
            println!("  {line}");
        }
    }
    println!("\nWhere the data lives:");
    for r in &results {
        let host = summary
            .as_ref()
            .map(|s| {
                if s.host.is_empty() {
                    "localhost".to_string()
                } else {
                    s.host.clone()
                }
            })
            .unwrap_or_else(|| "the server".to_string());
        println!(
            "  table disclosure.{} in database {database} on {host}; query it through the view {} in namespace '{}'",
            r.report.table,
            if r.source.name == "schedule_e" {
                "dump_schedule_e (or independent_expenditures)".to_string()
            } else {
                view_name(r.source).to_string()
            },
            common.schema
        );
        if args.dump_file.is_none() {
            println!(
                "  downloaded file: {} ({}); keep it so `hardmoney dumps update` can tell when fec.gov has a newer one, or delete it to free the space",
                r.report.dump_path.display(),
                std::fs::metadata(&r.report.dump_path)
                    .map(|m| dump::fmt_bytes(m.len()))
                    .unwrap_or_default()
            );
        }
    }
    for r in &results {
        if r.source.partitioned_by_cycle {
            let other = r
                .report
                .cycles
                .first()
                .and_then(|c| c.prev())
                .map(|c| c.to_string())
                .unwrap_or_else(|| "2024".to_string());
            println!(
                "\nTo add another cycle later: hardmoney dumps import {} --cycles {other}{}",
                What::for_source(r.source),
                schema_flag(&common.schema)
            );
        }
    }
    println!(
        "\nThe FEC refreshes these files every weekend; `hardmoney dumps update` re-imports what you have when a newer file appears."
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// `hardmoney dumps status`
// ---------------------------------------------------------------------------

async fn status(common: &Common, args: StatusArgs) -> CliResult {
    let out = Out { json: common.json };
    let snap = snapshot(common, args.offline).await?;
    if common.json {
        let mut v = snap.json(common);
        if let Some(obj) = v.as_object_mut() {
            obj.insert("offline".into(), serde_json::Value::Bool(args.offline));
        }
        return out.emit(&v);
    }

    match &snap.database {
        Database::NotConfigured => println!(
            "Database: none configured (DATABASE_URL is not set), so what is imported is unknown.\n{}\n",
            indent(&preflight::suggest_database_setup(), "  ")
        ),
        Database::Unreachable { what, next, .. } => {
            println!("Database: could not connect: {what}\n  what to do: {next}\n");
        }
        Database::Connected {
            summary,
            namespace_ready,
            ..
        } => println!(
            "Database: {} (namespace '{}'{})\n",
            summary
                .as_ref()
                .map(|s| format!(
                    "{} on {}",
                    s.database,
                    if s.host.is_empty() {
                        "localhost"
                    } else {
                        s.host.as_str()
                    }
                ))
                .unwrap_or_else(|| "connected".to_string()),
            common.schema,
            if *namespace_ready {
                ""
            } else {
                ", not set up for hardmoney yet"
            }
        ),
    }

    for v in &snap.dumps {
        println!("{} ({})", v.name(), describe_short(v.source));
        println!(
            "  table:        {} (view {})",
            v.table(),
            view_name(v.source)
        );
        let in_db = match &v.state {
            None => "unknown (no database)".to_string(),
            Some(s) if !s.exists => "no".to_string(),
            Some(s) => {
                let cycles = v.cycles_present();
                let mut text = format!(
                    "yes, about {} rows, {} index(es)",
                    fmt_count(u64::try_from(s.approx_rows).unwrap_or(0)),
                    s.index_count
                );
                if !cycles.is_empty() {
                    text.push_str(&format!(
                        "; cycles {}",
                        cycles
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if let Some(r) = &v.last_import {
                    text.push_str(&format!(
                        "; imported {} UTC from {}",
                        r.restored_at.format("%Y-%m-%d %H:%M"),
                        r.source_last_modified
                            .map(|t| format!("the fec.gov file dated {}", t.format("%Y-%m-%d")))
                            .unwrap_or_else(|| "a local file".to_string())
                    ));
                } else if matches!(snap.database, Database::Connected { .. }) {
                    text.push_str(&format!(
                        "; no import recorded in namespace '{}' (imported from another namespace, or before schema-init)",
                        common.schema
                    ));
                }
                text
            }
        };
        println!("  in database:  {in_db}");
        let downloaded = match (v.cache.complete_bytes, v.cache.partial_bytes) {
            (Some(n), _) => format!(
                "yes, {} at {}{}",
                dump::fmt_bytes(n),
                v.cache.path.display(),
                v.cache
                    .meta
                    .last_modified
                    .map(|t| format!(" (fec.gov file dated {})", t.format("%Y-%m-%d")))
                    .unwrap_or_default()
            ),
            (None, Some(n)) => format!(
                "partly: {} of an interrupted download at {} (the next import resumes it)",
                dump::fmt_bytes(n),
                v.cache.path.display()
            ),
            (None, None) => "no".to_string(),
        };
        println!("  downloaded:   {downloaded}");
        let on_fec = match (&v.remote, &v.remote_error, args.offline) {
            (_, _, true) => "not checked (--offline)".to_string(),
            (Some(r), _, _) => {
                let mut text = format!(
                    "{}, dated {}",
                    r.size
                        .map(dump::fmt_bytes)
                        .unwrap_or_else(|| "size unknown".to_string()),
                    r.last_modified
                        .map(|t| t.format("%Y-%m-%d").to_string())
                        .unwrap_or_else(|| "?".to_string())
                );
                if v.exists() {
                    text.push_str(match v.fec_has_newer() {
                        Some(true) => {
                            "; NEWER than what you imported (run `hardmoney dumps update`)"
                        }
                        Some(false) => "; you have the current file",
                        None => "; cannot tell whether it is newer than what you imported",
                    });
                }
                text
            }
            (None, Some(e), _) => format!("could not be reached ({e})"),
            (None, None, _) => "not checked".to_string(),
        };
        println!("  on fec.gov:   {on_fec}\n");
    }
    println!("Downloads are kept in {}.", common.cache_dir.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// `hardmoney dumps update`
// ---------------------------------------------------------------------------

fn cron_line(url: &str) -> String {
    format!(
        "0 6 * * 1  DATABASE_URL={} hardmoney dumps update --yes >> $HOME/hardmoney-dumps.log 2>&1",
        redact_url(url)
    )
}

async fn update(common: &Common, args: UpdateArgs) -> CliResult {
    let out = Out { json: common.json };
    let ask = interactive(common.json);
    let snap = snapshot(common, false).await?;
    let (url, pool) = match &snap.database {
        Database::Connected { url, pool, .. } => (url.clone(), pool.clone()),
        Database::NotConfigured => return Err(no_database_error().into()),
        Database::Unreachable { what, next, raw } => {
            return Err(Friendly::new(capitalize(what), next.clone())
                .detail(raw.clone())
                .into());
        }
    };

    let mut notes: Vec<serde_json::Value> = Vec::new();
    let mut to_update: Vec<&DumpView> = Vec::new();
    for v in snap.dumps.iter().filter(|v| v.exists()) {
        let name = v.name();
        match (&v.remote, v.local_version()) {
            (None, _) => {
                let e = v.remote_error.clone().unwrap_or_default();
                out.say(format!("{name}: could not reach fec.gov ({e}); skipped"));
                notes.push(serde_json::json!({ "name": name, "action": "skipped", "reason": "fec.gov unreachable", "error": e }));
            }
            (Some(_), None) => {
                out.say(format!(
                    "{name}: no record of which fec.gov file it came from (imported from a local file, or before schema-init); \
                     run `hardmoney dumps import {name}` to refresh it"
                ));
                notes.push(serde_json::json!({ "name": name, "action": "skipped", "reason": "no source recorded" }));
            }
            (Some(remote), Some(local)) => match remote.is_newer_than(&local) {
                Some(true) => to_update.push(v),
                Some(false) => {
                    out.say(format!(
                        "{name}: up to date (fec.gov file dated {})",
                        remote
                            .last_modified
                            .map(|t| t.format("%Y-%m-%d").to_string())
                            .unwrap_or_else(|| "?".to_string())
                    ));
                    notes.push(serde_json::json!({ "name": name, "action": "current" }));
                }
                None => {
                    out.say(format!("{name}: cannot compare with fec.gov (no ETag or date on either side); skipped"));
                    notes.push(serde_json::json!({ "name": name, "action": "skipped", "reason": "nothing to compare" }));
                }
            },
        }
    }
    if snap.dumps.iter().all(|v| !v.exists()) {
        out.say("Nothing is imported yet, so there is nothing to update. Start with `hardmoney dumps import all-small`.");
    }
    if to_update.is_empty() {
        out.say(format!(
            "\nNothing to do. To check every Monday morning (the FEC refreshes on weekends), add this line with `crontab -e`:\n  {}",
            cron_line(&url)
        ));
        return out.emit(&serde_json::json!({ "ok": true, "updated": [], "notes": notes, "cron": cron_line(&url) }));
    }

    out.say(format!(
        "\nfec.gov has newer files for: {}.",
        to_update
            .iter()
            .map(|v| v.name())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    let today = chrono::Utc::now().date_naive();
    let today_cycle = Cycle::containing(today.year()).map_err(|e| {
        Friendly::new(
            format!("could not work out the current cycle ({e})."),
            "try again",
        )
    })?;
    let mut steps = Vec::with_capacity(to_update.len());
    for v in &to_update {
        // Re-import what is there: the same cycles for a split table, the
        // whole table otherwise.
        let cycles = if v.source.partitioned_by_cycle {
            let present = v.cycles_present();
            if present.is_empty() {
                cycles_for(v.source, &[], today)?
            } else {
                present
            }
        } else {
            Vec::new()
        };
        let ctx = Planning {
            dump_file: None,
            cache_dir: &common.cache_dir,
            pool: Some(&pool),
            fresh_download: true,
            today,
            today_cycle,
        };
        steps.push(plan_step(v.source, &cycles, &ctx).await?);
    }
    let database = preflight::summarize_url(&url)
        .map(|s| s.database)
        .unwrap_or_else(|| "?".to_string());
    out.say(render_plan(&steps, &database, &common.cache_dir));
    if !args.yes {
        if ask {
            if !confirm("Continue?", false)? {
                out.say("Nothing was changed.");
                return Ok(());
            }
        } else if !common.json {
            out.say("(stdin is not a terminal, so no confirmation was asked; pass --yes to make that explicit)");
        }
    }
    let started = Instant::now();
    let mut results = Vec::with_capacity(steps.len());
    for (i, step) in steps.iter().enumerate() {
        if i > 0 {
            out.say("");
        }
        // The old download goes only now, after confirmation, so a "no"
        // leaves everything as it was.
        dump::evict_cached(step.source, &common.cache_dir).map_err(|e| {
            Friendly::new(
                format!("Could not delete the old download of {}.", step.name()),
                format!("check permissions on {}", common.cache_dir.display()),
            )
            .detail(e.to_string())
        })?;
        results.push(execute_step(&pool, &url, step, &common.cache_dir, args.jobs, &out).await?);
    }
    out.say(format!(
        "\nDone in {}. To do this automatically every Monday morning, add this line with `crontab -e`:\n  {}",
        fmt_elapsed(started.elapsed()),
        cron_line(&url)
    ));
    out.emit(&serde_json::json!({
        "ok": true,
        "updated": results.iter().map(StepResult::json).collect::<Vec<_>>(),
        "notes": notes,
        "cron": cron_line(&url),
        "elapsed_secs": started.elapsed().as_secs_f64(),
    }))
}

// ---------------------------------------------------------------------------
// `hardmoney dumps remove`
// ---------------------------------------------------------------------------

async fn remove(common: &Common, args: RemoveArgs) -> CliResult {
    let out = Out { json: common.json };
    let ask = interactive(common.json);
    let (_, pool) = connect(common).await?;
    struct Target {
        source: &'static DumpSource,
        state: dump::DumpTableState,
        cache: dump::CachedDump,
    }
    let mut targets = Vec::new();
    for source in args.what.sources() {
        let state = dump::table_state(&pool, source)
            .await
            .map_err(|e| friendly_dump_error(e, source, &[]))?;
        let cache = dump::cached(source, &common.cache_dir);
        targets.push(Target {
            source,
            state,
            cache,
        });
    }
    let file_bytes = |c: &dump::CachedDump| {
        c.complete_bytes
            .or(c.partial_bytes)
            .filter(|_| !args.keep_file)
    };
    let anything = targets
        .iter()
        .any(|t| t.state.exists || file_bytes(&t.cache).is_some());
    if !anything {
        let kept_file = args.keep_file
            && targets
                .iter()
                .any(|t| t.cache.complete_bytes.or(t.cache.partial_bytes).is_some());
        out.say(if kept_file {
            format!(
                "{} is not imported, and --keep-file leaves the downloaded file alone; nothing to remove.",
                args.what
            )
        } else {
            format!(
                "{} is not imported and not downloaded; nothing to remove.",
                args.what
            )
        });
        return out.emit(&serde_json::json!({ "ok": true, "removed": [] }));
    }

    out.say("This will:");
    for t in &targets {
        let name = What::for_source(t.source);
        if t.state.exists {
            out.say(format!(
                "  drop table disclosure.{} (about {} rows{}) and the views over it in every namespace",
                t.source.disclosure_table,
                fmt_count(u64::try_from(t.state.approx_rows).unwrap_or(0)),
                if t.state.partitions.is_empty() {
                    String::new()
                } else {
                    format!(", {} cycle tables", t.state.partitions.len())
                }
            ));
        } else {
            out.say(format!("  ({name} is not in the database)"));
        }
        match file_bytes(&t.cache) {
            Some(n) => out.say(format!(
                "  delete the downloaded file {} ({})",
                t.cache.path.display(),
                dump::fmt_bytes(n)
            )),
            None if args.keep_file => {
                out.say("  keep the downloaded file (--keep-file)".to_string())
            }
            None => {}
        }
    }
    out.say("The record of past imports in the namespace's loads table is kept.");
    if !args.yes {
        if !ask {
            return Err(Friendly::new(
                "Removing data needs confirmation, and no terminal is attached to ask on.",
                "run the same command with --yes",
            )
            .into());
        }
        if !confirm("Continue?", false)? {
            out.say("Nothing was changed.");
            return Ok(());
        }
    }

    let mut removed = Vec::new();
    for t in &targets {
        let dropped = dump::drop_restored(&pool, t.source)
            .await
            .map_err(|e| friendly_dump_error(e, t.source, &[]))?;
        let file_deleted = if args.keep_file {
            false
        } else {
            dump::evict_cached(t.source, &common.cache_dir).map_err(|e| {
                Friendly::new(
                    format!("Could not delete {}.", t.cache.path.display()),
                    "delete it by hand if you want the space back",
                )
                .detail(e.to_string())
            })?
        };
        let name = What::for_source(t.source);
        out.say(format!(
            "{name}: table {}, file {}",
            if dropped { "dropped" } else { "was not there" },
            if file_deleted {
                "deleted"
            } else if args.keep_file {
                "kept"
            } else {
                "was not there"
            }
        ));
        removed.push(serde_json::json!({
            "name": name,
            "table": format!("disclosure.{}", t.source.disclosure_table),
            "table_dropped": dropped,
            "approx_rows_dropped": if dropped { t.state.approx_rows } else { 0 },
            "file_deleted": file_deleted,
        }));
    }
    out.emit(&serde_json::json!({ "ok": true, "removed": removed }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn friendly_names_map_both_ways() {
        let parse = |s: &str| s.parse::<What>();
        assert_eq!(parse("committees").unwrap(), What::Committees);
        assert_eq!(parse("committee_history").unwrap(), What::Committees);
        assert_eq!(parse("Committee-History").unwrap(), What::Committees);
        assert_eq!(
            parse("independent-expenditures").unwrap(),
            What::IndependentExpenditures
        );
        assert_eq!(parse("schedule_e").unwrap(), What::IndependentExpenditures);
        assert_eq!(parse("ies").unwrap(), What::IndependentExpenditures);
        assert_eq!(parse("receipts").unwrap(), What::Receipts);
        assert_eq!(parse("schedule_a_full").unwrap(), What::Receipts);
        assert_eq!(parse("schedule-a").unwrap(), What::Receipts);
        assert_eq!(parse("contributions").unwrap(), What::Receipts);
        assert_eq!(parse("disbursements").unwrap(), What::Disbursements);
        assert_eq!(parse("schedule_b_full").unwrap(), What::Disbursements);
        assert_eq!(parse("all-small").unwrap(), What::AllSmall);
        assert_eq!(parse(" ALL_SMALL ").unwrap(), What::AllSmall);
        let err = parse("everything").unwrap_err();
        assert!(
            err.contains(
                "committees, independent-expenditures, receipts, disbursements, all-small"
            ),
            "{err}"
        );
        assert!(err.contains("schedule_a_full"), "{err}");

        for what in [
            What::Committees,
            What::IndependentExpenditures,
            What::Receipts,
            What::Disbursements,
            What::AllSmall,
        ] {
            assert_eq!(what.name().parse::<What>().unwrap(), what);
            assert_eq!(what.to_string(), what.name());
            assert!(!what.sources().is_empty());
        }
        assert_eq!(What::AllSmall.sources().len(), 2);
        assert!(What::AllSmall.sources().iter().all(|s| !s.is_large));
        for source in dump::ALL {
            // Every dump has a friendly name that parses back to a `What`
            // covering it, and its technical name is accepted too.
            let friendly = What::for_source(source);
            assert_ne!(friendly, source.name);
            let what: What = friendly.parse().unwrap();
            assert!(what.sources().iter().any(|s| s.name == source.name));
            let by_technical: What = source.name.parse().unwrap();
            assert_eq!(by_technical, what);
            assert!(!describe(source).is_empty());
            assert!(!describe_short(source).is_empty());
            assert!(view_name(source).starts_with("dump_"));
        }
    }

    #[test]
    fn cycles_default_to_the_current_one() {
        let a = &dump::SCHEDULE_A;
        let e = &dump::SCHEDULE_E;
        let c = |y: u16| Cycle::new(y).unwrap();
        // Even year: that cycle. Odd year: the cycle it belongs to.
        assert_eq!(cycles_for(a, &[], day(2026, 9, 15)).unwrap(), vec![c(2026)]);
        assert_eq!(cycles_for(a, &[], day(2025, 3, 1)).unwrap(), vec![c(2026)]);
        assert_eq!(
            cycles_for(a, &[], day(2024, 12, 31)).unwrap(),
            vec![c(2024)]
        );
        // Explicit cycles are sorted and de-duplicated.
        assert_eq!(
            cycles_for(a, &[c(2026), c(2022), c(2026)], day(2026, 1, 1)).unwrap(),
            vec![c(2022), c(2026)]
        );
        // Single-table dumps take no cycles and refuse them.
        assert!(cycles_for(e, &[], day(2026, 9, 15)).unwrap().is_empty());
        let err = cycles_for(e, &[c(2024)], day(2026, 9, 15))
            .unwrap_err()
            .to_string();
        assert!(err.contains("--cycles does not apply"), "{err}");
        assert!(err.contains("independent-expenditures"), "{err}");
        assert_eq!(cycle_span(c(2026)), "2025-2026");
        assert_eq!(cycles_list(&[c(2024), c(2026)]), "2024,2026");
    }

    fn step(
        source: &'static DumpSource,
        cycles: Vec<Cycle>,
        location: Location,
        bytes: u64,
    ) -> Step {
        let today = Cycle::new(2026).unwrap();
        let selective = source.partitioned_by_cycle && !cycles.is_empty();
        let needs = preflight::estimate(
            source,
            bytes,
            matches!(location, Location::Download { .. }),
            &cycles,
            selective,
            today,
        );
        Step {
            source,
            cycles,
            location,
            archive_bytes: bytes,
            size_exact: true,
            needs,
            parent_exists: false,
        }
    }

    #[test]
    fn plan_text_names_sizes_counts_and_tables() {
        let e = step(
            &dump::SCHEDULE_E,
            vec![],
            Location::Download { resume_from: None },
            43_440_933,
        );
        let text = e.describe("fec");
        assert!(
            text.starts_with("independent-expenditures: download 43.4 MB from fec.gov"),
            "{text}"
        );
        assert!(text.contains("under a minute"), "{text}");
        assert!(text.contains("about 550,000 rows"), "{text}");
        assert!(
            text.contains("table disclosure.fec_fitem_sched_e in database fec"),
            "{text}"
        );
        assert!(text.contains("about 1 to 2 minutes"), "{text}");

        let cached = step(
            &dump::COMMITTEE_HISTORY,
            vec![],
            Location::Cached(PathBuf::from("/c/committee_history.dump")),
            14_190_177,
        );
        let text = cached.describe("fec");
        assert!(
            text.contains("use the 14.1 MB file already downloaded to /c/committee_history.dump"),
            "{text}"
        );
        assert!(text.contains("disclosure.ofec_committee_history"), "{text}");

        let a = step(
            &dump::SCHEDULE_A,
            vec![Cycle::new(2026).unwrap()],
            Location::Download { resume_from: None },
            90_181_919_946,
        );
        let text = a.describe("fec");
        assert!(
            text.starts_with("receipts (cycle 2026): download the whole 90.1 GB file"),
            "{text}"
        );
        assert!(text.contains("does not offer per-cycle files"), "{text}");
        assert!(text.contains("load only the 2025-2026 rows (tens of millions of rows for a recent cycle) into disclosure.fec_fitem_sched_a_2025_2026"), "{text}");
        assert!(text.contains("without the FEC's own indexes"), "{text}");
        assert!(text.contains("hours"), "{text}");
        assert!(text.contains("hardmoney's 3 indexes"), "{text}");
        assert!(a.data_only());
        assert!(!e.data_only());
        assert_eq!(a.target_tables(), vec!["fec_fitem_sched_a_2025_2026"]);

        let resumed = step(
            &dump::SCHEDULE_B,
            vec![Cycle::new(2024).unwrap(), Cycle::new(2026).unwrap()],
            Location::Download {
                resume_from: Some(20_000_000_000),
            },
            39_313_945_474,
        );
        let text = resumed.describe("fec");
        assert!(
            text.starts_with(
                "disbursements (cycles 2024, 2026): finish downloading the 39.3 GB file"
            ),
            "{text}"
        );
        assert!(text.contains("20.0 GB already here"), "{text}");
        assert!(
            text.contains(
                "disclosure.fec_fitem_sched_b_2023_2024 and disclosure.fec_fitem_sched_b_2025_2026"
            ),
            "{text}"
        );

        let plan = render_plan(&[e, cached], "fec", Path::new("/c"));
        assert!(
            plan.starts_with("Plan\n  1. independent-expenditures"),
            "{plan}"
        );
        assert!(plan.contains("\n  2. committees"), "{plan}");
        assert!(plan.contains("Disk: about "), "{plan}");
        assert!(plan.contains("43.4 MB of downloads in /c"), "{plan}");
        assert!(plan.contains("Time: about "), "{plan}");
        assert!(plan.contains("estimates"), "{plan}");
    }

    #[test]
    fn explain_shows_the_real_pg_restore_command() {
        let a = step(
            &dump::SCHEDULE_A,
            vec![Cycle::new(2026).unwrap()],
            Location::Download { resume_from: None },
            90_181_919_946,
        );
        let lines = a
            .explain(
                "postgres://alice:secret@localhost/fec",
                Path::new("/cache"),
                2,
            )
            .unwrap();
        assert_eq!(lines[0], "CREATE SCHEMA IF NOT EXISTS disclosure;");
        assert_eq!(
            lines[1],
            "DROP TABLE IF EXISTS disclosure.fec_fitem_sched_a_2025_2026 CASCADE;"
        );
        assert_eq!(
            lines[2],
            "pg_restore --no-owner --no-acl --jobs=2 --table=fec_fitem_sched_a --table=fec_fitem_sched_a_2025_2026 -d postgres://alice:***@localhost/fec /cache/schedule_a_full.dump"
        );
        assert!(lines.iter().any(|l| l == "CREATE UNIQUE INDEX hm_fec_fitem_sched_a_2025_2026_sub_id_uidx ON disclosure.fec_fitem_sched_a_2025_2026 (sub_id);"), "{lines:?}");
        assert!(
            lines.iter().any(|l| l.contains("gin_trgm_ops")),
            "{lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains("secret")),
            "password leaked: {lines:?}"
        );

        let e = step(
            &dump::SCHEDULE_E,
            vec![],
            Location::File(PathBuf::from("/tmp/my dump.dump")),
            1,
        );
        let lines = e
            .explain("postgres://u@localhost/fec", Path::new("/cache"), 1)
            .unwrap();
        assert_eq!(
            lines[1],
            "DROP TABLE IF EXISTS disclosure.fec_fitem_sched_e CASCADE;"
        );
        assert_eq!(
            lines[2],
            "pg_restore --no-owner --no-acl -d postgres://u@localhost/fec '/tmp/my dump.dump'"
        );
        assert_eq!(lines.len(), 3, "a whole restore adds no indexes: {lines:?}");
    }

    #[test]
    fn friendly_errors_say_what_to_do() {
        let f = no_database_error().to_string();
        assert!(f.starts_with("No database is configured"), "{f}");
        assert!(f.contains("what to do: if Postgres is installed"), "{f}");
        assert!(f.contains("createdb fec"), "{f}");
        assert!(f.contains("export DATABASE_URL=postgres://"), "{f}");

        let e = friendly_dump_error(
            DumpError::Incomplete {
                url: "u".into(),
                expected: 90_000_000_000,
                actual: 30_000_000_000,
            },
            &dump::SCHEDULE_A,
            &[],
        )
        .to_string();
        assert!(
            e.contains("receipts stopped early (30.0 GB of 90.0 GB received)"),
            "{e}"
        );
        assert!(e.contains("resumes"), "{e}");

        let e = friendly_dump_error(
            DumpError::MissingPartition {
                dump: "x".into(),
                cycle: Cycle::new(2030).unwrap(),
                available: "1976, 2026".into(),
            },
            &dump::SCHEDULE_A,
            &[],
        )
        .to_string();
        assert!(e.contains("no data for cycle 2030"), "{e}");
        assert!(e.contains("1976, 2026"), "{e}");

        let e = friendly_dump_error(
            DumpError::ExternalTool {
                tool: "pg_restore",
                detail: "raw output".into(),
            },
            &dump::SCHEDULE_E,
            &[],
        )
        .to_string();
        assert!(
            e.contains("pg_restore did not load independent-expenditures cleanly"),
            "{e}"
        );
        assert!(e.contains("details: pg_restore failed: raw output"), "{e}");

        let e = friendly_dump_error(
            DumpError::LargeDumpNotAllowed {
                name: "schedule_a_full",
                size: "90.1 GB".into(),
            },
            &dump::SCHEDULE_A,
            &[Cycle::new(2026).unwrap()],
        )
        .to_string();
        assert!(e.contains("--cycles 2026"), "{e}");
    }

    #[test]
    fn command_lines_for_people() {
        let public = Namespace::public();
        let tut = Namespace::new("tut").unwrap();
        // The URL is shown as $DATABASE_URL only when that is where it came
        // from; the tests do not set it, so the literal (redacted) form.
        let line = psql_line("postgres://a:pw@h/db", &public, "SELECT 1;");
        assert_eq!(line, "psql \"postgres://a:***@h/db\" -c \"SELECT 1;\"");
        let line = psql_line(
            "postgres://a@h/db",
            &tut,
            "SELECT count(*) FROM dump_schedule_e;",
        );
        assert_eq!(
            line,
            "psql \"postgres://a@h/db\" -c \"SELECT count(*) FROM tut.dump_schedule_e;\""
        );
        assert_eq!(schema_flag(&public), "");
        assert_eq!(schema_flag(&tut), " --schema tut");
        let steps = next_steps(&dump::SCHEDULE_E, "postgres://a@h/db", &tut);
        assert_eq!(steps.len(), 3);
        assert!(steps[0].starts_with("hardmoney query ies"), "{steps:?}");
        assert!(steps[0].ends_with("--schema tut"), "{steps:?}");
        assert!(
            steps[1].contains("SELECT count(*) FROM tut.dump_schedule_e;"),
            "{steps:?}"
        );
        let steps = next_steps(&dump::COMMITTEE_HISTORY, "postgres://a@h/db", &public);
        assert!(steps[0].contains("dump_committee_history"), "{steps:?}");
        let steps = next_steps(&dump::SCHEDULE_A, "postgres://a@h/db", &public);
        assert!(steps[1].contains("GROUP BY cycle"), "{steps:?}");
        assert!(
            cron_line("postgres://a:pw@h/db")
                .contains("DATABASE_URL=postgres://a:***@h/db hardmoney dumps update --yes")
        );
        assert_eq!(
            shell_join(&["a".into(), "b c".into(), "it's".into()]),
            "a 'b c' 'it'\\''s'"
        );
        assert_eq!(capitalize("the x"), "The x");
        assert_eq!(capitalize(""), "");
        assert_eq!(indent("a\nb", "  "), "  a\n  b");
        assert_eq!(rows_phrase(&dump::SCHEDULE_E, &[]), "about 550,000 rows");
        assert_eq!(
            rows_phrase(&dump::SCHEDULE_A, &[]),
            "hundreds of millions of rows"
        );
        assert_eq!(
            rows_phrase(&dump::SCHEDULE_A, &[Cycle::new(2026).unwrap()]),
            "tens of millions of rows for a recent cycle"
        );
    }

    #[test]
    fn columns_line_up() {
        let text = print_columns(
            &["a", "bb"],
            &[["x".into(), "y".into()], ["long".into(), "z".into()]],
        );
        assert_eq!(text, "  a     bb\n  x     y\n  long  z\n");
    }
}
