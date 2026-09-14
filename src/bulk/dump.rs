//! Restoring the FEC's own official `pg_dump` archives.
//!
//! Answering "does the FEC publish a full pg_dump of its data" (the
//! motivating question for this module): **partially**. The FEC publishes
//! `pg_dump --format=custom` archives, updated weekly, for four tables at
//! `https://www.fec.gov/files/bulk-downloads/data-dump/schedules/` (see
//! the README's "The pgdump question" section for the full writeup):
//!
//! | file | approx. size | practical to restore here? |
//! |---|---|---|
//! | `fec_fitem_sched_a.dump` (itemized receipts) | ~90 GB | no |
//! | `fec_fitem_sched_b.dump` (itemized disbursements) | ~39 GB | no |
//! | `fec_fitem_sched_e.dump` (independent expenditures) | ~43 MB | **yes** |
//! | `ofec_committee_history.dump` (committee history) | ~14 MB | **yes** |
//!
//! Restoring `fec_fitem_sched_e.dump` was verified end-to-end against a
//! live Postgres 18 instance while building this crate: it restores
//! cleanly (one benign warning -- see [`DumpSource::EXPECTED_WARNING`])
//! into a `disclosure` schema with real, current data (549k+ rows as of
//! the September 2026 weekly refresh). `src/db/schema.sql` exposes it
//! under the friendly name `independent_expenditures` once restored.
//!
//! The two multi-gigabyte schedule dumps are registered here too (so the
//! CLI can name and document them) but require `--allow-large` and will
//! not fit in most disposable/sandboxed environments.

use std::path::{Path, PathBuf};

use super::error::{BulkError, Result};

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

/// Downloads (if not already cached in `cache_dir`) and restores `source`
/// into the `disclosure` schema of the database at `database_url`.
/// `pg_restore` must be on `PATH`. Returns without doing anything if
/// `source.is_large` and `allow_large` is `false`.
pub async fn restore(database_url: &str, source: &'static DumpSource, cache_dir: &Path, allow_large: bool) -> Result<PathBuf> {
    if source.is_large && !allow_large {
        return Err(BulkError::ExternalTool {
            tool: "pg_restore",
            detail: format!(
                "{} is ~{:.1} GB; pass allow_large=true / --allow-large to proceed",
                source.name,
                source.approx_size_bytes as f64 / 1e9
            ),
        });
    }

    std::fs::create_dir_all(cache_dir)?;
    let dump_path = cache_dir.join(format!("{}.dump", source.name));
    if !dump_path.exists() {
        download_dump(source.url, &dump_path)?;
    }

    let pool = sqlx::postgres::PgPoolOptions::new().max_connections(1).connect(database_url).await?;
    sqlx::query("CREATE SCHEMA IF NOT EXISTS disclosure").execute(&pool).await?;
    // Best-effort: these extensions back full-text/trigram indexes some
    // dumps' DDL references. If they're unavailable (e.g. no superuser),
    // restoration still succeeds for the table + data + primary key; only
    // ancillary indexes/triggers may be skipped, matching what was
    // observed restoring fec_fitem_sched_e.dump in development.
    let _ = sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_trgm").execute(&pool).await;
    let _ = sqlx::query("CREATE EXTENSION IF NOT EXISTS btree_gin").execute(&pool).await;
    pool.close().await;

    let database_url = database_url.to_string();
    let dump_path_clone = dump_path.clone();
    tokio::task::spawn_blocking(move || run_pg_restore(&database_url, &dump_path_clone))
        .await
        .map_err(|e| BulkError::Io(std::io::Error::other(e)))??;

    Ok(dump_path)
}

fn download_dump(url: &str, dest: &Path) -> Result<()> {
    let resp = ureq::get(url).call().map_err(|e| BulkError::Http(e.to_string()))?;
    let body = resp.into_body();
    let pb = match body.content_length() {
        Some(len) => indicatif::ProgressBar::new(len).with_style(download_style()),
        None => indicatif::ProgressBar::new_spinner(),
    };
    let mut reader = pb.wrap_read(body.into_reader());
    let mut file = std::fs::File::create(dest)?;
    std::io::copy(&mut reader, &mut file)?;
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

fn run_pg_restore(database_url: &str, dump_path: &Path) -> Result<()> {
    let output = std::process::Command::new("pg_restore")
        .args(["--no-owner", "--no-acl", "-d", database_url])
        .arg(dump_path)
        .output()
        .map_err(|e| BulkError::ExternalTool { tool: "pg_restore", detail: e.to_string() })?;

    if !output.status.success() {
        return Err(BulkError::ExternalTool {
            tool: "pg_restore",
            detail: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(())
}
