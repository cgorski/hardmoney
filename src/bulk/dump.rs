//! Restoring the FEC's own official `pg_dump` archives.
//!
//! The FEC publishes `pg_dump --format=custom` archives, updated weekly
//! (Saturdays), for four tables at
//! `https://www.fec.gov/files/bulk-downloads/data-dump/schedules/`:
//!
//! | file | approx. size | practical to restore here? |
//! |---|---|---|
//! | `fec_fitem_sched_a.dump` (itemized receipts, 1975-present) | ~90 GB | no |
//! | `fec_fitem_sched_b.dump` (itemized disbursements) | ~39 GB | no |
//! | `fec_fitem_sched_e.dump` (independent expenditures) | ~43 MB | **yes** |
//! | `ofec_committee_history.dump` (committee history) | ~14 MB | **yes** |
//!
//! There are no dumps for Schedules C, D, or F.
//!
//! Each dump's DDL hard-codes the `disclosure` schema, so restores always
//! land there regardless of the hardmoney namespace in use -- the dump is
//! reference data shared by every namespace, and
//! [`crate::db::ensure_views`] exposes it inside each namespace as the
//! `independent_expenditures` view.
//!
//! # `pg_restore` exit status
//!
//! `pg_restore` exits non-zero whenever it *ignored* an error, and a first
//! restore of `fec_fitem_sched_e.dump` always ignores one: the dump
//! references a trigger function that does not exist outside the FEC's
//! own database. A second restore additionally ignores "already exists"
//! for every object. Neither is a failure of the data load, so this module
//! judges success by whether the target table exists and has rows after
//! the run, not by the exit code -- and drops the table first so a
//! re-restore is a clean refresh rather than a pile of ignored errors.

use std::path::{Path, PathBuf};

use sqlx::PgPool;

use super::error::{BulkError, Result};

#[derive(Debug)]
pub struct DumpSource {
    pub name: &'static str,
    pub url: &'static str,
    pub approx_size_bytes: u64,
    /// Requires `--allow-large` on the CLI before downloading/restoring.
    pub is_large: bool,
    /// The table this dump creates, inside the `disclosure` schema.
    pub disclosure_table: &'static str,
}

pub const SCHEDULE_E: DumpSource = DumpSource {
    name: "schedule_e",
    url: "https://www.fec.gov/files/bulk-downloads/data-dump/schedules/fec_fitem_sched_e.dump",
    approx_size_bytes: 43_440_933,
    is_large: false,
    disclosure_table: "fec_fitem_sched_e",
};

pub const COMMITTEE_HISTORY: DumpSource = DumpSource {
    name: "committee_history",
    url: "https://www.fec.gov/files/bulk-downloads/data-dump/schedules/ofec_committee_history.dump",
    approx_size_bytes: 14_190_177,
    is_large: false,
    disclosure_table: "ofec_committee_history",
};

pub const SCHEDULE_A: DumpSource = DumpSource {
    name: "schedule_a_full",
    url: "https://www.fec.gov/files/bulk-downloads/data-dump/schedules/fec_fitem_sched_a.dump",
    approx_size_bytes: 90_181_919_946,
    is_large: true,
    disclosure_table: "fec_fitem_sched_a",
};

pub const SCHEDULE_B: DumpSource = DumpSource {
    name: "schedule_b_full",
    url: "https://www.fec.gov/files/bulk-downloads/data-dump/schedules/fec_fitem_sched_b.dump",
    approx_size_bytes: 39_313_945_474,
    is_large: true,
    disclosure_table: "fec_fitem_sched_b",
};

pub const ALL: &[&DumpSource] = &[&SCHEDULE_E, &COMMITTEE_HISTORY, &SCHEDULE_A, &SCHEDULE_B];

pub fn find(name: &str) -> Option<&'static DumpSource> {
    ALL.iter().copied().find(|s| s.name == name)
}

/// Outcome of a successful [`restore`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RestoreReport {
    pub dump_path: PathBuf,
    pub table: &'static str,
    pub rows: i64,
    /// Non-fatal messages `pg_restore` emitted (the expected trigger
    /// warning and the like), for the operator's information.
    pub warnings: Vec<String>,
}

/// Downloads (if not already cached in `cache_dir`) and restores `source`
/// into the `disclosure` schema of `pool`'s database, replacing any
/// previous restore of the same table, then refreshes hardmoney's views
/// over it in the pool's namespace. `pg_restore` must be on `PATH`.
pub async fn restore(
    pool: &PgPool,
    database_url: &str,
    source: &'static DumpSource,
    cache_dir: &Path,
    allow_large: bool,
) -> Result<RestoreReport> {
    if source.is_large && !allow_large {
        return Err(BulkError::ExternalTool {
            tool: "pg_restore",
            detail: format!(
                "{} is ~{:.1} GB; pass --allow-large to proceed",
                source.name,
                source.approx_size_bytes as f64 / 1e9
            ),
        });
    }

    std::fs::create_dir_all(cache_dir)?;
    let dump_path = cache_dir.join(format!("{}.dump", source.name));
    if !dump_path.exists() {
        let url = source.url.to_string();
        let dest = dump_path.clone();
        tokio::task::spawn_blocking(move || download_dump(&url, &dest)).await??;
    }

    sqlx::query("CREATE SCHEMA IF NOT EXISTS disclosure")
        .execute(pool)
        .await?;
    // Best-effort: the dumps' DDL references indexes that need these.
    // If unavailable, the table + data + primary key still restore.
    for ext in ["pg_trgm", "btree_gin"] {
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE EXTENSION IF NOT EXISTS {ext}"
        )))
        .execute(pool)
        .await;
    }
    // Clean refresh: a re-restore into an existing table would fail every
    // CREATE and duplicate every row.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP TABLE IF EXISTS disclosure.{} CASCADE",
        source.disclosure_table
    )))
    .execute(pool)
    .await?;

    let url = database_url.to_string();
    let path = dump_path.clone();
    let warnings = tokio::task::spawn_blocking(move || run_pg_restore(&url, &path)).await??;

    let rows: Option<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM disclosure.{}",
        source.disclosure_table
    )))
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let rows = rows.ok_or_else(|| BulkError::ExternalTool {
        tool: "pg_restore",
        detail: format!(
            "disclosure.{} does not exist after restore; pg_restore output:\n{}",
            source.disclosure_table,
            warnings.join("\n")
        ),
    })?;

    // The dropped table cascaded away any dependent views; rebuild them.
    crate::db::ensure_views(pool).await?;

    Ok(RestoreReport {
        dump_path,
        table: source.disclosure_table,
        rows,
        warnings,
    })
}

fn download_dump(url: &str, dest: &Path) -> Result<()> {
    let resp = ureq::get(url).call().map_err(|e| BulkError::Http {
        url: url.to_string(),
        detail: e.to_string(),
    })?;
    let body = resp.into_body();
    let pb = match body.content_length() {
        Some(len) => indicatif::ProgressBar::new(len).with_style(download_style()),
        None => indicatif::ProgressBar::new_spinner(),
    };
    let mut reader = pb.wrap_read(body.into_reader());
    // Write to a sibling temp file and rename, so an interrupted download
    // never leaves a truncated file that a later run would trust.
    let tmp = dest.with_extension("dump.partial");
    let mut file = std::fs::File::create(&tmp)?;
    std::io::copy(&mut reader, &mut file)?;
    file.sync_all()?;
    std::fs::rename(&tmp, dest)?;
    pb.finish_and_clear();
    Ok(())
}

fn download_style() -> indicatif::ProgressStyle {
    indicatif::ProgressStyle::with_template(
        "{msg} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, ETA {eta})",
    )
    .unwrap_or_else(|_| indicatif::ProgressStyle::default_bar())
    .progress_chars("#>-")
}

/// Runs `pg_restore`, returning its stderr lines as warnings. Only a
/// failure to *launch* the tool is an error here; whether the restore
/// worked is judged by the caller against the database.
fn run_pg_restore(database_url: &str, dump_path: &Path) -> Result<Vec<String>> {
    let output = std::process::Command::new("pg_restore")
        // No `--exit-on-error`: keep going past the expected ignorable
        // errors (missing trigger function) and report them as warnings.
        .args(["--no-owner", "--no-acl", "-d", database_url])
        .arg(dump_path)
        .output()
        .map_err(|e| BulkError::ExternalTool {
            tool: "pg_restore",
            detail: format!("could not run pg_restore (is it on PATH?): {e}"),
        })?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}
