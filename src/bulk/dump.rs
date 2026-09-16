//! Restoring the FEC's own official `pg_dump` archives.
//!
//! The FEC publishes `pg_dump --format=custom` archives, refreshed weekly
//! (the README says Saturdays; the September 2026 files carry a Sunday
//! `Last-Modified`), for four tables at
//! `https://www.fec.gov/files/bulk-downloads/data-dump/schedules/`:
//!
//! | dump | table | size (Sept 2026) | split by cycle? |
//! |---|---|---|---|
//! | `schedule_a_full` | `fec_fitem_sched_a` (itemized receipts, 1975-present) | ~90 GB | yes |
//! | `schedule_b_full` | `fec_fitem_sched_b` (itemized disbursements) | ~39 GB | yes |
//! | `schedule_e` | `fec_fitem_sched_e` (independent expenditures) | ~43 MB | no |
//! | `committee_history` | `ofec_committee_history` | ~14 MB | no |
//!
//! There are no dumps for Schedules C, D, or F.
//!
//! Each dump's DDL hard-codes the `disclosure` schema, so restores always
//! land there regardless of the hardmoney namespace in use. The dump is
//! reference data shared by every namespace, and [`crate::db::ensure_views`]
//! exposes it inside each namespace as `independent_expenditures` plus the
//! `dump_*` views.
//!
//! # Partitions
//!
//! The two large tables are split into one child table per two-year
//! period (`fec_fitem_sched_a_2023_2024`, ...) using table inheritance
//! (`INHERITS`, with a `CHECK` on `two_year_transaction_period`). The
//! FEC's README makes the 90 GB dump tractable by naming the parent plus
//! the wanted children with `pg_restore --table`; [`restore_with`] does
//! that from [`RestoreOptions::cycles`], discovering the child names from
//! the archive's own table of contents ([`read_toc`]) so the list stays
//! right as cycles are added.
//!
//! `pg_restore --table` restores only table definitions and data (never
//! indexes, constraints, or triggers), so a cycle-selective restore is
//! always data-only. [`create_indexes`] then adds the few indexes
//! hardmoney's views and API use, instead of the FEC's 34 per partition.
//!
//! # `pg_restore` exit status
//!
//! `pg_restore` exits non-zero whenever it *ignored* an error, and a full
//! restore of any of these dumps always ignores one: each references a
//! `BEFORE INSERT` trigger function that exists only inside the FEC's own
//! database and is not in the archive. Neither that nor "already exists"
//! on a re-run is a failure of the data load, so this module judges
//! success by whether the target tables exist and have rows afterwards,
//! not by the exit code, and drops the tables it is about to restore
//! first so a re-restore is a clean refresh rather than a pile of ignored
//! errors.
//!
//! # Where the files come from
//!
//! Each dump's URL is [`DumpSource::url`] under [`Endpoints::www_base`]
//! (`HARDMONEY_FEC_WWW_BASE`). [`remote_info_with`], [`download_with`],
//! and [`RestoreOptions::endpoints`] take the endpoints explicitly;
//! [`remote_info`], [`download`], and a `RestoreOptions` without them
//! read [`Endpoints::from_env`] and fail with [`DumpError::Endpoint`] on
//! a malformed override.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rust_decimal::Decimal;
use sqlx::PgPool;

use super::error::BulkError;
use crate::Cycle;
use crate::fec::{EndpointError, Endpoints};

/// Directory listing of the FEC's dump files, on the production host:
/// [`Endpoints::dump_readme`] at the default base.
pub const INDEX_URL: &str =
    "https://www.fec.gov/files/bulk-downloads/data-dump/schedules/README.txt";

/// Prefix of the `loads.source` value under which restores are recorded
/// (`dump:schedule_e`).
pub const LOAD_SOURCE_PREFIX: &str = "dump:";

/// One of the FEC's published dumps.
#[derive(Debug)]
#[non_exhaustive]
pub struct DumpSource {
    pub name: &'static str,
    /// The archive's file name in the FEC's dump directory
    /// (`fec_fitem_sched_e.dump`); [`DumpSource::url`] makes it a URL.
    pub file_name: &'static str,
    /// Size observed in September 2026; the two large dumps grow every
    /// week (the FEC says they double every 2.5 years).
    pub approx_size_bytes: u64,
    /// Requires `--allow-large` (or `--cycles`) on the CLI before
    /// downloading/restoring the whole thing.
    pub is_large: bool,
    /// The table this dump creates, inside the `disclosure` schema.
    pub disclosure_table: &'static str,
    /// Whether the FEC splits this table into one child table per
    /// two-year period (`<table>_2023_2024`) that `--cycles` can select.
    pub partitioned_by_cycle: bool,
    /// The indexes [`create_indexes`] builds on each restored table.
    pub indexes: &'static [IndexSpec],
}

impl DumpSource {
    /// Where to download the archive: [`DumpSource::file_name`] in the
    /// dump directory under `endpoints.www_base` ([`Endpoints::dump`]).
    #[must_use]
    pub fn url(&self, endpoints: &Endpoints) -> String {
        endpoints.dump(self.file_name)
    }
}

/// An index [`create_indexes`] creates on a restored dump table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct IndexSpec {
    /// Index name suffix; the full name is `hm_<table>_<suffix>`.
    pub suffix: &'static str,
    /// Column list as it appears inside the parentheses.
    pub columns: &'static str,
    pub unique: bool,
    /// A GIN trigram index (needs `pg_trgm`; skipped if it is unavailable).
    pub trigram: bool,
}

const SCHED_A_INDEXES: [IndexSpec; 3] = [
    IndexSpec {
        suffix: "sub_id_uidx",
        columns: "sub_id",
        unique: true,
        trigram: false,
    },
    IndexSpec {
        suffix: "cmte_dt_idx",
        columns: "cmte_id, contb_receipt_dt",
        unique: false,
        trigram: false,
    },
    IndexSpec {
        suffix: "contbr_nm_trgm_idx",
        columns: "contbr_nm",
        unique: false,
        trigram: true,
    },
];

const SCHED_B_INDEXES: [IndexSpec; 3] = [
    IndexSpec {
        suffix: "sub_id_uidx",
        columns: "sub_id",
        unique: true,
        trigram: false,
    },
    IndexSpec {
        suffix: "cmte_dt_idx",
        columns: "cmte_id, disb_dt",
        unique: false,
        trigram: false,
    },
    IndexSpec {
        suffix: "recipient_nm_trgm_idx",
        columns: "recipient_nm",
        unique: false,
        trigram: true,
    },
];

const SCHED_E_INDEXES: [IndexSpec; 5] = [
    IndexSpec {
        suffix: "sub_id_uidx",
        columns: "sub_id",
        unique: true,
        trigram: false,
    },
    IndexSpec {
        suffix: "cmte_dt_idx",
        columns: "cmte_id, exp_dt",
        unique: false,
        trigram: false,
    },
    // `/independent-expenditures?candidate_id=` and the compare helper.
    IndexSpec {
        suffix: "cand_idx",
        columns: "s_o_cand_id",
        unique: false,
        trigram: false,
    },
    IndexSpec {
        suffix: "file_num_idx",
        columns: "file_num",
        unique: false,
        trigram: false,
    },
    IndexSpec {
        suffix: "pye_nm_trgm_idx",
        columns: "pye_nm",
        unique: false,
        trigram: true,
    },
];

const COMMITTEE_HISTORY_INDEXES: [IndexSpec; 3] = [
    IndexSpec {
        suffix: "idx_uidx",
        columns: "idx",
        unique: true,
        trigram: false,
    },
    // The FEC's own sample queries join on (committee_id, cycle).
    IndexSpec {
        suffix: "committee_cycle_idx",
        columns: "committee_id, cycle",
        unique: false,
        trigram: false,
    },
    IndexSpec {
        suffix: "name_trgm_idx",
        columns: "name",
        unique: false,
        trigram: true,
    },
];

pub const SCHEDULE_E: DumpSource = DumpSource {
    name: "schedule_e",
    file_name: "fec_fitem_sched_e.dump",
    approx_size_bytes: 43_440_933,
    is_large: false,
    disclosure_table: "fec_fitem_sched_e",
    partitioned_by_cycle: false,
    indexes: &SCHED_E_INDEXES,
};

pub const COMMITTEE_HISTORY: DumpSource = DumpSource {
    name: "committee_history",
    file_name: "ofec_committee_history.dump",
    approx_size_bytes: 14_190_177,
    is_large: false,
    disclosure_table: "ofec_committee_history",
    partitioned_by_cycle: false,
    indexes: &COMMITTEE_HISTORY_INDEXES,
};

pub const SCHEDULE_A: DumpSource = DumpSource {
    name: "schedule_a_full",
    file_name: "fec_fitem_sched_a.dump",
    approx_size_bytes: 90_181_919_946,
    is_large: true,
    disclosure_table: "fec_fitem_sched_a",
    partitioned_by_cycle: true,
    indexes: &SCHED_A_INDEXES,
};

pub const SCHEDULE_B: DumpSource = DumpSource {
    name: "schedule_b_full",
    file_name: "fec_fitem_sched_b.dump",
    approx_size_bytes: 39_313_945_474,
    is_large: true,
    disclosure_table: "fec_fitem_sched_b",
    partitioned_by_cycle: true,
    indexes: &SCHED_B_INDEXES,
};

pub const ALL: &[&DumpSource] = &[&SCHEDULE_E, &COMMITTEE_HISTORY, &SCHEDULE_A, &SCHEDULE_B];

/// Looks a dump up by its hardmoney name (`schedule_e`, ...). `None` for
/// an unknown name.
#[must_use]
pub fn find(name: &str) -> Option<&'static DumpSource> {
    ALL.iter().copied().find(|s| s.name == name)
}

/// Where `source` is cached under `cache_dir` once downloaded.
#[must_use]
pub fn cache_path(source: &DumpSource, cache_dir: &Path) -> PathBuf {
    cache_dir.join(format!("{}.dump", source.name))
}

/// Errors from downloading, inspecting, restoring, indexing, or comparing
/// a dump.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DumpError {
    /// A large dump was requested in full without `allow_large`.
    #[error(
        "{name} is ~{size}; pass --cycles <years> to restore only some two-year periods, or --allow-large to restore all of it"
    )]
    LargeDumpNotAllowed { name: &'static str, size: String },

    /// `cycles` was given for a dump that is one table, not split by period.
    #[error("{name} is a single table (not split by two-year period); --cycles does not apply")]
    NotPartitioned { name: &'static str },

    /// A requested cycle has no child table in the archive.
    #[error("{dump} has no table for cycle {cycle}; the archive contains: {available}")]
    MissingPartition {
        dump: String,
        cycle: Cycle,
        available: String,
    },

    /// `dump_file` does not exist.
    #[error("dump file {0} does not exist")]
    DumpFileMissing(PathBuf),

    #[error("HTTP error fetching {url}: {detail}")]
    Http { url: String, detail: String },

    /// The connection dropped before the whole file arrived. The partial
    /// file is kept; the next run resumes from where it stopped.
    #[error(
        "download of {url} stopped at {actual} of {expected} bytes; the partial file was kept, re-run to resume"
    )]
    Incomplete {
        url: String,
        expected: u64,
        actual: u64,
    },

    /// `pg_restore` could not be run, or reported something that left the
    /// database without the expected tables.
    #[error("{tool} failed: {detail}")]
    ExternalTool { tool: &'static str, detail: String },

    /// An operation needs a dump table that has not been restored.
    #[error(
        "disclosure.{table} is not in this database; run `hardmoney bulk-restore-dump {dump}` first"
    )]
    TableMissing { table: String, dump: &'static str },

    /// [`compare_filing`] was asked about a filing that is not in `filings`.
    #[error(
        "filing {0} has not been ingested into this namespace; run `hardmoney bulk-load-filing {0}` first"
    )]
    FilingNotIngested(i64),

    /// A table name from `pg_restore --list` or `pg_inherits` was not a
    /// plain lower-case identifier; refused rather than interpolated.
    #[error("unexpected identifier {0:?} in dump metadata")]
    UnsafeIdentifier(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    /// A blocking task panicked or was cancelled.
    #[error("background task failed: {0}")]
    Task(#[from] tokio::task::JoinError),

    /// A `HARDMONEY_*` endpoint override is not a usable URL (see
    /// [`Endpoints::from_env`]). Raised before any request is made.
    #[error("{0}")]
    Endpoint(#[from] EndpointError),
}

impl From<DumpError> for BulkError {
    fn from(e: DumpError) -> Self {
        match e {
            DumpError::Io(e) => BulkError::Io(e),
            DumpError::Database(e) => BulkError::Database(e),
            DumpError::Task(e) => BulkError::Task(e),
            DumpError::Http { url, detail } => BulkError::Http { url, detail },
            // The nearest variant: the download could not even be addressed.
            DumpError::Endpoint(e) => BulkError::Http {
                url: e.value().to_string(),
                detail: e.to_string(),
            },
            DumpError::ExternalTool { tool, detail } => BulkError::ExternalTool { tool, detail },
            other => BulkError::ExternalTool {
                tool: "pg_restore",
                detail: other.to_string(),
            },
        }
    }
}

type Result<T> = std::result::Result<T, DumpError>;

// ---------------------------------------------------------------------------
// Table of contents
// ---------------------------------------------------------------------------

/// The kind of one `pg_restore --list` entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TocKind {
    Schema,
    Table,
    TableData,
    Index,
    Constraint,
    Trigger,
    Function,
    /// Any other `pg_dump` object description, verbatim (`COMMENT`,
    /// `SEQUENCE SET`, `FK CONSTRAINT`, ...).
    Other(String),
}

impl TocKind {
    fn from_desc(desc: &str) -> Self {
        match desc {
            "SCHEMA" => Self::Schema,
            "TABLE" => Self::Table,
            "TABLE DATA" => Self::TableData,
            "INDEX" => Self::Index,
            "CONSTRAINT" => Self::Constraint,
            "TRIGGER" => Self::Trigger,
            "FUNCTION" => Self::Function,
            other => Self::Other(other.to_string()),
        }
    }

    /// The `pg_dump` description (`TABLE DATA`, ...).
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Schema => "SCHEMA",
            Self::Table => "TABLE",
            Self::TableData => "TABLE DATA",
            Self::Index => "INDEX",
            Self::Constraint => "CONSTRAINT",
            Self::Trigger => "TRIGGER",
            Self::Function => "FUNCTION",
            Self::Other(s) => s,
        }
    }
}

/// One entry from `pg_restore --list`, e.g.
/// `358; 1259 17839 TABLE disclosure fec_fitem_sched_e fec`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TocEntry {
    /// The archive's own id for the entry (the number before `;`).
    pub dump_id: u32,
    pub kind: TocKind,
    /// Schema, or `None` where `pg_restore` prints `-`.
    pub schema: Option<String>,
    /// The entry's tag. For `CONSTRAINT` and `TRIGGER` entries `pg_dump`
    /// prints the table then the object (`fec_fitem_sched_e
    /// fec_fitem_sched_e_pkey`); see [`TocEntry::table`].
    pub name: String,
    pub owner: Option<String>,
}

impl TocEntry {
    /// Parses one entry line. `None` for header/comment lines (starting
    /// with `;`), blank lines, and anything not shaped like an entry.
    ///
    /// The description is taken to be the run of all-upper-case words
    /// after the two catalog ids, the next token is the schema (`-` for
    /// none), and the last token is the owner when at least two tokens
    /// remain. That is exact for the kinds hardmoney acts on (`TABLE`,
    /// `TABLE DATA`, `INDEX`, `CONSTRAINT`, `TRIGGER`), which `pg_dump`
    /// always prints with an owner; ownerless kinds (`COMMENT`,
    /// `ENCODING`) may mislabel their last token, and are `Other` anyway.
    #[must_use]
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            return None;
        }
        let (id, rest) = line.split_once(';')?;
        let dump_id: u32 = id.trim().parse().ok()?;
        let mut tokens = rest.split_whitespace();
        // Two catalog ids (`tableoid oid`), both numeric.
        tokens.next()?.parse::<u64>().ok()?;
        tokens.next()?.parse::<u64>().ok()?;
        let tokens: Vec<&str> = tokens.collect();
        let desc_len = tokens
            .iter()
            .take_while(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_uppercase() || c == '_'))
            .count();
        if desc_len == 0 {
            return None;
        }
        let desc = tokens.get(..desc_len)?.join(" ");
        let schema = match tokens.get(desc_len) {
            Some(&"-") => None,
            Some(s) => Some((*s).to_string()),
            None => return None,
        };
        let remaining = tokens.get(desc_len.saturating_add(1)..).unwrap_or(&[]);
        let (name, owner) = match remaining.split_last() {
            Some((last, head)) if !head.is_empty() => (head.join(" "), Some((*last).to_string())),
            Some((last, _)) => ((*last).to_string(), None),
            None => (String::new(), None),
        };
        Some(Self {
            dump_id,
            kind: TocKind::from_desc(&desc),
            schema,
            name,
            owner,
        })
    }

    /// The table this entry defines or belongs to: the entry itself for
    /// `TABLE`/`TABLE DATA`, the first tag token for `CONSTRAINT` and
    /// `TRIGGER`, `None` otherwise (an `INDEX` entry does not name its
    /// table).
    #[must_use]
    pub fn table(&self) -> Option<&str> {
        match self.kind {
            TocKind::Table | TocKind::TableData => Some(self.name.as_str()),
            TocKind::Constraint | TocKind::Trigger => self.name.split_whitespace().next(),
            _ => None,
        }
    }
}

/// A child table of a cycle-split dump table.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Partition {
    pub cycle: Cycle,
    /// `fec_fitem_sched_a_2023_2024`
    pub table: String,
}

/// The FEC's child-table name for `cycle`: `<parent>_<odd>_<even>`.
#[must_use]
pub fn partition_table(parent: &str, cycle: Cycle) -> String {
    let even = cycle.year();
    format!("{parent}_{}_{even}", even.saturating_sub(1))
}

/// The cycle a child table named `<parent>_<odd>_<even>` covers, or
/// `None` if `table` is not shaped like one of `parent`'s partitions.
#[must_use]
pub fn partition_cycle(parent: &str, table: &str) -> Option<Cycle> {
    let suffix = table.strip_prefix(parent)?.strip_prefix('_')?;
    let (first, second) = suffix.split_once('_')?;
    if first.len() != 4 || second.len() != 4 {
        return None;
    }
    let first: u16 = first.parse().ok()?;
    let second: u16 = second.parse().ok()?;
    if second != first.checked_add(1)? {
        return None;
    }
    Cycle::new(second).ok()
}

/// The parsed output of `pg_restore --list`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct DumpToc {
    /// `Archive created at ...` from the header, verbatim.
    pub archive_created: Option<String>,
    /// `Dumped from database version: ...` from the header.
    pub source_server_version: Option<String>,
    pub entries: Vec<TocEntry>,
}

impl DumpToc {
    /// Parses the text `pg_restore --list` prints. Never fails: lines that
    /// are not entries are skipped, so an unexpected format yields an
    /// empty `entries`.
    #[must_use]
    pub fn parse(listing: &str) -> Self {
        let mut toc = Self::default();
        for line in listing.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix(';') {
                let rest = rest.trim();
                if let Some(v) = rest.strip_prefix("Archive created at") {
                    toc.archive_created = Some(v.trim().to_string());
                } else if let Some(v) = rest.strip_prefix("Dumped from database version:") {
                    toc.source_server_version = Some(v.trim().to_string());
                }
                continue;
            }
            if let Some(entry) = TocEntry::parse(trimmed) {
                toc.entries.push(entry);
            }
        }
        toc
    }

    /// Every `TABLE` entry.
    pub fn tables(&self) -> impl Iterator<Item = &TocEntry> {
        self.entries.iter().filter(|e| e.kind == TocKind::Table)
    }

    /// Whether a `TABLE` entry named `table` is present.
    #[must_use]
    pub fn has_table(&self, table: &str) -> bool {
        self.tables().any(|e| e.name == table)
    }

    /// The per-cycle child tables of `parent`, sorted by cycle. Empty for
    /// a table that is not split (the Schedule E dump).
    #[must_use]
    pub fn partitions(&self, parent: &str) -> Vec<Partition> {
        let mut parts: Vec<Partition> = self
            .tables()
            .filter_map(|e| {
                partition_cycle(parent, &e.name).map(|cycle| Partition {
                    cycle,
                    table: e.name.clone(),
                })
            })
            .collect();
        parts.sort();
        parts.dedup();
        parts
    }

    /// Number of entries of `kind`.
    #[must_use]
    pub fn count(&self, kind: &TocKind) -> usize {
        self.entries.iter().filter(|e| &e.kind == kind).count()
    }

    /// `INDEX` entries belonging to `cycle`'s child table, recognised by
    /// the period in their names (the FEC names them
    /// `idx_sched_a_2023_2024_<columns>`). An `INDEX` entry does not name
    /// its table, so this is by convention, not by catalog.
    #[must_use]
    pub fn indexes_for_cycle(&self, cycle: Cycle) -> usize {
        let even = cycle.year();
        let period = format!("_{}_{even}", even.saturating_sub(1));
        self.entries
            .iter()
            .filter(|e| e.kind == TocKind::Index && e.name.contains(&period))
            .count()
    }
}

/// Runs `pg_restore --list` on `dump_path` and parses the result. Fast
/// even on the 90 GB archive: the custom format keeps its table of
/// contents at the front. Fails if `pg_restore` is not on `PATH` or
/// rejects the file.
pub fn read_toc(dump_path: &Path) -> Result<DumpToc> {
    let output = std::process::Command::new("pg_restore")
        .arg("--list")
        .arg(dump_path)
        .output()
        .map_err(|e| DumpError::ExternalTool {
            tool: "pg_restore",
            detail: format!("could not run pg_restore (is it on PATH?): {e}"),
        })?;
    if !output.status.success() {
        return Err(DumpError::ExternalTool {
            tool: "pg_restore",
            detail: format!(
                "pg_restore --list {} failed: {}",
                dump_path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    Ok(DumpToc::parse(&String::from_utf8_lossy(&output.stdout)))
}

// ---------------------------------------------------------------------------
// Download
// ---------------------------------------------------------------------------

/// What the FEC's server says about a dump file: `Content-Length`,
/// `ETag`, `Last-Modified`. Any of them may be absent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct RemoteDump {
    pub size: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<chrono::DateTime<chrono::Utc>>,
}

impl RemoteDump {
    /// The `If-Range` value that makes a resumed request conditional on
    /// the file being unchanged: the quoted ETag if there is one, else
    /// the `Last-Modified` date; `None` if the server gave neither.
    #[must_use]
    pub fn if_range_value(&self) -> Option<String> {
        match (&self.etag, self.last_modified) {
            (Some(tag), _) => Some(format!("\"{tag}\"")),
            (None, Some(t)) => Some(t.format("%a, %d %b %Y %H:%M:%S GMT").to_string()),
            (None, None) => None,
        }
    }

    fn write_meta(&self, path: &Path) -> std::io::Result<()> {
        let mut out = String::new();
        if let Some(n) = self.size {
            out.push_str(&format!("size={n}\n"));
        }
        if let Some(e) = &self.etag {
            out.push_str(&format!("etag={e}\n"));
        }
        if let Some(t) = self.last_modified {
            out.push_str(&format!("last_modified={}\n", t.to_rfc3339()));
        }
        std::fs::write(path, out)
    }

    fn read_meta(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let mut meta = Self::default();
        for line in text.lines() {
            match line.split_once('=') {
                Some(("size", v)) => meta.size = v.trim().parse().ok(),
                Some(("etag", v)) => meta.etag = Some(v.trim().to_string()),
                Some(("last_modified", v)) => {
                    meta.last_modified = chrono::DateTime::parse_from_rfc3339(v.trim())
                        .ok()
                        .map(|d| d.with_timezone(&chrono::Utc));
                }
                _ => {}
            }
        }
        Some(meta)
    }
}

/// What is on disk for one dump.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CachedDump {
    /// Where the complete file is (or would be).
    pub path: PathBuf,
    /// Size of the complete file, if present.
    pub complete_bytes: Option<u64>,
    /// Size of an interrupted download awaiting resume, if present.
    pub partial_bytes: Option<u64>,
    /// Headers recorded when the download started.
    pub meta: RemoteDump,
}

/// Inspects the cache without touching the network.
#[must_use]
pub fn cached(source: &DumpSource, cache_dir: &Path) -> CachedDump {
    let path = cache_path(source, cache_dir);
    let len = |p: &Path| std::fs::metadata(p).ok().map(|m| m.len());
    CachedDump {
        complete_bytes: len(&path),
        partial_bytes: len(&partial_path(&path)),
        meta: RemoteDump::read_meta(&meta_path(&path)).unwrap_or_default(),
        path,
    }
}

fn partial_path(dest: &Path) -> PathBuf {
    dest.with_extension("dump.partial")
}

fn meta_path(dest: &Path) -> PathBuf {
    dest.with_extension("dump.meta")
}

/// [`remote_info_with`] at [`Endpoints::from_env`]. Fails with
/// [`DumpError::Endpoint`] if a `HARDMONEY_*` override is malformed.
pub fn remote_info(source: &DumpSource) -> Result<RemoteDump> {
    remote_info_with(source, &Endpoints::from_env()?)
}

/// `HEAD`s the dump's URL under `endpoints.www_base` (following fec.gov's
/// redirect to S3). Blocking.
pub fn remote_info_with(source: &DumpSource, endpoints: &Endpoints) -> Result<RemoteDump> {
    let url = source.url(endpoints);
    let resp = ureq::head(&url).call().map_err(|e| DumpError::Http {
        url: url.clone(),
        detail: e.to_string(),
    })?;
    let header = |name: &str| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    Ok(RemoteDump {
        size: header("content-length").and_then(|s| s.trim().parse().ok()),
        etag: header("etag").map(|s| s.trim_matches('"').to_string()),
        last_modified: header("last-modified").and_then(|s| parse_http_date(&s)),
    })
}

fn parse_http_date(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc2822(s.trim())
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc))
}

/// One ranged `GET`, abstracted so the resume logic can be tested
/// without a network.
pub(crate) trait RangeFetch {
    /// Requests the resource from byte `from` (`Range: bytes=from-`,
    /// omitted when `from == 0`), conditioned on `if_range` (a quoted
    /// ETag or an HTTP date, see [`RemoteDump::if_range_value`]) so a
    /// server whose file has changed answers with the whole new file.
    fn fetch(&mut self, from: u64, if_range: Option<&str>) -> Result<RangeResponse>;
}

/// The server's answer to [`RangeFetch::fetch`].
pub(crate) struct RangeResponse {
    /// Byte offset the body starts at: `from` for a `206`, `0` for a
    /// `200` (the server ignored the range, or the file changed).
    pub start: u64,
    /// Total size of the resource (`Content-Range` total, or
    /// `Content-Length` on a `200`), if the server said.
    pub total: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<chrono::DateTime<chrono::Utc>>,
    pub body: Box<dyn Read + Send>,
}

struct HttpRange<'a> {
    url: &'a str,
}

impl RangeFetch for HttpRange<'_> {
    fn fetch(&mut self, from: u64, if_range: Option<&str>) -> Result<RangeResponse> {
        let http_err = |detail: String| DumpError::Http {
            url: self.url.to_string(),
            detail,
        };
        let mut req = ureq::get(self.url)
            .config()
            .http_status_as_error(false)
            .build();
        if from > 0 {
            req = req.header("Range", format!("bytes={from}-"));
            if let Some(validator) = if_range {
                req = req.header("If-Range", validator);
            }
        }
        let resp = req.call().map_err(|e| http_err(e.to_string()))?;
        let status = resp.status().as_u16();
        let header = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let etag = header("etag").map(|s| s.trim_matches('"').to_string());
        let last_modified = header("last-modified").and_then(|s| parse_http_date(&s));
        match status {
            200 => {
                let total = resp.body().content_length();
                Ok(RangeResponse {
                    start: 0,
                    total,
                    etag,
                    last_modified,
                    body: Box::new(resp.into_body().into_reader()),
                })
            }
            206 => {
                let (start, total) = header("content-range")
                    .and_then(|s| parse_content_range(&s))
                    .ok_or_else(|| {
                        http_err("206 Partial Content without a usable Content-Range".into())
                    })?;
                if start != from {
                    return Err(http_err(format!(
                        "asked for bytes from {from}, server answered from {start}"
                    )));
                }
                Ok(RangeResponse {
                    start,
                    total,
                    etag,
                    last_modified,
                    body: Box::new(resp.into_body().into_reader()),
                })
            }
            // The partial file already holds every byte (or more): nothing
            // to append; the caller checks the length against the total.
            416 => Ok(RangeResponse {
                start: from,
                total: header("content-range").and_then(|s| parse_unsatisfied_range(&s)),
                etag,
                last_modified,
                body: Box::new(std::io::empty()),
            }),
            other => Err(http_err(format!("HTTP {other}"))),
        }
    }
}

/// `bytes 100-199/43440933` -> `(100, Some(43440933))`; total `*` -> `None`.
fn parse_content_range(value: &str) -> Option<(u64, Option<u64>)> {
    let spec = value.trim().strip_prefix("bytes")?.trim();
    let (range, total) = spec.split_once('/')?;
    let (start, _end) = range.split_once('-')?;
    let start: u64 = start.trim().parse().ok()?;
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse().ok()?),
    };
    Some((start, total))
}

/// `bytes */43440933` -> `Some(43440933)`.
fn parse_unsatisfied_range(value: &str) -> Option<u64> {
    let spec = value.trim().strip_prefix("bytes")?.trim();
    let (_, total) = spec.split_once('/')?;
    total.trim().parse().ok()
}

/// Downloads `url` to `dest`, resuming a previous partial download if
/// one is present next to `dest`, and refusing to call the file complete
/// unless its length matches what the server declared.
///
/// Layout: bytes accumulate in `<dest>.partial`; the first response's
/// `ETag`/`Last-Modified` go in `<dest>.meta` and are sent back as
/// `If-Range` on resume, so a dump the FEC replaced mid-download restarts
/// from zero instead of splicing two weeks together. On success the
/// partial is renamed to `dest`; on a short read it is kept and
/// [`DumpError::Incomplete`] says how far it got.
pub(crate) fn download_resumable(
    fetch: &mut dyn RangeFetch,
    url: &str,
    dest: &Path,
    show_progress: bool,
) -> Result<RemoteDump> {
    let partial = partial_path(dest);
    let meta_file = meta_path(dest);
    let saved = RemoteDump::read_meta(&meta_file).unwrap_or_default();
    let validator = saved.if_range_value();
    // A partial with nothing to validate it against (a file left by an
    // older hardmoney, or a server that sent neither header) cannot be
    // resumed safely: the FEC may have replaced the file since.
    let existing = match (std::fs::metadata(&partial), &validator) {
        (Ok(m), Some(_)) => m.len(),
        _ => 0,
    };

    let mut resp = fetch.fetch(existing, validator.as_deref())?;
    // `If-Range` obliges a compliant server to answer 200 with the whole
    // new file when the validator no longer matches. A server that
    // honours `Range` but ignores `If-Range` would splice a new week's
    // bytes onto last week's partial; catching a changed ETag here and
    // starting over is the belt to that suspender.
    if resp.start > 0
        && let (Some(now), Some(then)) = (&resp.etag, &saved.etag)
        && now != then
    {
        drop(resp);
        let _ = std::fs::remove_file(&partial);
        let _ = std::fs::remove_file(&meta_file);
        resp = fetch.fetch(0, None)?;
        if resp.start != 0 {
            return Err(DumpError::Http {
                url: url.to_string(),
                detail: format!(
                    "the file changed on the server since the partial download began, and a fresh request was answered from byte {} instead of 0",
                    resp.start
                ),
            });
        }
    }
    let (mut file, position) = if resp.start == 0 {
        (std::fs::File::create(&partial)?, 0)
    } else if resp.start == existing {
        (
            std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&partial)?,
            existing,
        )
    } else {
        return Err(DumpError::Http {
            url: url.to_string(),
            detail: format!(
                "server resumed at byte {} but the partial file holds {existing}",
                resp.start
            ),
        });
    };

    let remote = RemoteDump {
        size: resp.total,
        etag: resp.etag.clone(),
        last_modified: resp.last_modified,
    };
    if position == 0 || !meta_file.exists() {
        remote.write_meta(&meta_file)?;
    }

    let pb = match (show_progress, resp.total) {
        (false, _) => indicatif::ProgressBar::hidden(),
        (true, Some(len)) => indicatif::ProgressBar::new(len).with_style(download_style()),
        (true, None) => indicatif::ProgressBar::new_spinner(),
    };
    pb.set_position(position);
    let mut reader = pb.wrap_read(resp.body);
    let copied = std::io::copy(&mut reader, &mut file);
    pb.finish_and_clear();
    // A dropped connection surfaces here; keep the partial for next time.
    copied?;
    file.flush()?;
    file.sync_all()?;
    let actual = file.metadata()?.len();
    drop(file);

    if let Some(expected) = resp.total {
        if actual < expected {
            return Err(DumpError::Incomplete {
                url: url.to_string(),
                expected,
                actual,
            });
        }
        if actual > expected {
            // Only possible if the remote file shrank under a saved partial
            // (a smaller replacement dump plus a server that ignored
            // If-Range). Nothing in it can be trusted.
            let _ = std::fs::remove_file(&partial);
            let _ = std::fs::remove_file(&meta_file);
            return Err(DumpError::Http {
                url: url.to_string(),
                detail: format!(
                    "partial file held {actual} bytes but the server reports {expected}; discarded it, re-run to download afresh"
                ),
            });
        }
    }
    std::fs::rename(&partial, dest)?;
    Ok(RemoteDump {
        size: Some(actual),
        ..remote
    })
}

fn download_style() -> indicatif::ProgressStyle {
    indicatif::ProgressStyle::with_template(
        "{msg} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, ETA {eta})",
    )
    .unwrap_or_else(|_| indicatif::ProgressStyle::default_bar())
    .progress_chars("#>-")
}

/// [`download_with`] at [`Endpoints::from_env`]. Fails with
/// [`DumpError::Endpoint`] if a `HARDMONEY_*` override is malformed.
pub fn download(source: &DumpSource, cache_dir: &Path) -> Result<CachedDump> {
    download_with(source, cache_dir, &Endpoints::from_env()?)
}

/// Downloads `source` from `endpoints.www_base` into `cache_dir` unless a
/// complete copy is already there, resuming a partial one if present.
/// Blocking; shows a progress bar on stderr. Returns the cache state
/// afterwards.
///
/// Bytes accumulate in `<name>.dump.partial`; the first response's
/// `ETag`/`Last-Modified`/`Content-Length` go in `<name>.dump.meta` and
/// the ETag is sent back as `If-Range` on resume, so a dump the FEC
/// replaced mid-download restarts from zero instead of splicing two
/// weeks together. On success the partial is renamed to `<name>.dump`;
/// on a short read it is kept and [`DumpError::Incomplete`] says how
/// far it got.
pub fn download_with(
    source: &DumpSource,
    cache_dir: &Path,
    endpoints: &Endpoints,
) -> Result<CachedDump> {
    std::fs::create_dir_all(cache_dir)?;
    let dest = cache_path(source, cache_dir);
    if !dest.exists() {
        let url = source.url(endpoints);
        let mut http = HttpRange { url: &url };
        download_resumable(&mut http, &url, &dest, true)?;
    }
    Ok(cached(source, cache_dir))
}

// ---------------------------------------------------------------------------
// Restore
// ---------------------------------------------------------------------------

/// How to restore a dump. `RestoreOptions::new()` is a complete restore
/// of a cached-or-downloaded archive, indexes included.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct RestoreOptions {
    /// Restore only these two-year periods (the parent table plus the
    /// named child tables). Empty means the whole archive. Implies
    /// `data_only`, because `pg_restore --table` never restores indexes.
    pub cycles: Vec<Cycle>,
    /// Skip the archive's post-data section (indexes, primary keys,
    /// triggers). The FEC's numbers for Schedule A: 5 h data-only vs 35 h
    /// with indexes. Add hardmoney's indexes afterwards with
    /// [`create_indexes`].
    pub data_only: bool,
    /// Permit a full restore of a dump marked [`DumpSource::is_large`].
    pub allow_large: bool,
    /// Restore from this already-downloaded archive instead of the cache.
    pub dump_file: Option<PathBuf>,
    /// `pg_restore --jobs`: parallel workers, useful when several child
    /// tables are being restored. `0` or `1` means single-threaded.
    pub jobs: u8,
    /// Where to download from. `None` (the default) reads
    /// [`Endpoints::from_env`] when a download is needed and fails with
    /// [`DumpError::Endpoint`] on a malformed override; not consulted at
    /// all when [`RestoreOptions::dump_file`] is set.
    pub endpoints: Option<Endpoints>,
}

impl RestoreOptions {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Downloads from these endpoints instead of reading the environment.
    #[must_use]
    pub fn endpoints(mut self, endpoints: Endpoints) -> Self {
        self.endpoints = Some(endpoints);
        self
    }

    #[must_use]
    pub fn cycles(mut self, cycles: impl IntoIterator<Item = Cycle>) -> Self {
        self.cycles = cycles.into_iter().collect();
        self.cycles.sort();
        self.cycles.dedup();
        self
    }

    #[must_use]
    pub fn data_only(mut self, yes: bool) -> Self {
        self.data_only = yes;
        self
    }

    #[must_use]
    pub fn allow_large(mut self, yes: bool) -> Self {
        self.allow_large = yes;
        self
    }

    #[must_use]
    pub fn dump_file(mut self, path: Option<PathBuf>) -> Self {
        self.dump_file = path;
        self
    }

    #[must_use]
    pub fn jobs(mut self, n: u8) -> Self {
        self.jobs = n;
        self
    }
}

/// The `pg_restore` invocation [`restore_with`] settled on.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RestorePlan {
    /// `--table` arguments; empty means the whole archive.
    pub tables: Vec<String>,
    /// The child tables being restored (empty for a whole-archive restore).
    pub partitions: Vec<Partition>,
    /// Whether the archive's post-data section is skipped.
    pub data_only: bool,
    pub jobs: u8,
}

impl RestorePlan {
    /// The arguments before `-d <url> <file>`.
    #[must_use]
    pub fn pg_restore_args(&self) -> Vec<String> {
        let mut args = vec!["--no-owner".to_string(), "--no-acl".to_string()];
        if self.jobs > 1 {
            args.push(format!("--jobs={}", self.jobs));
        }
        if self.tables.is_empty() {
            if self.data_only {
                args.push("--section=pre-data".to_string());
                args.push("--section=data".to_string());
            }
        } else {
            for t in &self.tables {
                args.push(format!("--table={t}"));
            }
        }
        args
    }
}

/// Decides what to ask `pg_restore` for. `parent_exists` says whether
/// `disclosure.<table>` is already in the database (then a cycle-selective
/// restore leaves it alone rather than re-copying the parent's own rows).
///
/// Fails with [`DumpError::NotPartitioned`] if cycles were requested for a
/// single-table dump and [`DumpError::MissingPartition`] if the archive
/// has no child table for a requested cycle.
pub fn plan_restore(
    source: &DumpSource,
    toc: &DumpToc,
    options: &RestoreOptions,
    parent_exists: bool,
) -> Result<RestorePlan> {
    if options.cycles.is_empty() {
        return Ok(RestorePlan {
            tables: Vec::new(),
            partitions: Vec::new(),
            data_only: options.data_only,
            jobs: options.jobs,
        });
    }
    let available = toc.partitions(source.disclosure_table);
    if available.is_empty() {
        return Err(DumpError::NotPartitioned { name: source.name });
    }
    let mut partitions = Vec::with_capacity(options.cycles.len());
    for cycle in &options.cycles {
        match available.iter().find(|p| p.cycle == *cycle) {
            Some(p) => partitions.push(p.clone()),
            None => {
                return Err(DumpError::MissingPartition {
                    dump: format!("{}.dump", source.disclosure_table),
                    cycle: *cycle,
                    available: available
                        .iter()
                        .map(|p| p.cycle.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                });
            }
        }
    }
    let mut tables = Vec::with_capacity(partitions.len().saturating_add(1));
    if !parent_exists {
        tables.push(source.disclosure_table.to_string());
    }
    tables.extend(partitions.iter().map(|p| p.table.clone()));
    Ok(RestorePlan {
        tables,
        partitions,
        data_only: true,
        jobs: options.jobs,
    })
}

/// Outcome of a successful [`restore`] / [`restore_with`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RestoreReport {
    pub dump_path: PathBuf,
    /// The parent table, inside `disclosure`.
    pub table: &'static str,
    /// The tables whose rows were counted: the child tables for a
    /// cycle-selective restore, else just `table`.
    pub tables: Vec<String>,
    pub cycles: Vec<Cycle>,
    /// Total rows in `tables` after the restore.
    pub rows: i64,
    /// Whether the archive's indexes, primary keys, and triggers were
    /// skipped (`--no-indexes` or `--cycles`).
    pub data_only: bool,
    /// `INDEX` entries in the archive that were not restored: all of them
    /// for a whole data-only restore, those named for the selected
    /// periods for a cycle-selective one ([`DumpToc::indexes_for_cycle`]).
    pub indexes_skipped: usize,
    /// `loads` rows written (one per cycle, or one for a whole restore);
    /// empty if the namespace has no `loads` table yet.
    pub load_ids: Vec<i64>,
    /// Non-fatal messages `pg_restore` emitted (the expected trigger
    /// warning and the like), for the operator's information.
    pub warnings: Vec<String>,
}

/// Downloads (if not already cached in `cache_dir`) and restores `source`
/// into the `disclosure` schema of `pool`'s database, replacing any
/// previous restore of the same table, then refreshes hardmoney's views
/// over it in the pool's namespace. `pg_restore` must be on `PATH`.
///
/// The whole-archive form of [`restore_with`]; kept for callers of the
/// 2.x signature.
pub async fn restore(
    pool: &PgPool,
    database_url: &str,
    source: &'static DumpSource,
    cache_dir: &Path,
    allow_large: bool,
) -> super::error::Result<RestoreReport> {
    Ok(restore_with(
        pool,
        database_url,
        source,
        cache_dir,
        &RestoreOptions::new().allow_large(allow_large),
    )
    .await?)
}

/// Restores `source` according to `options`.
///
/// Steps: refuse a whole large dump without `allow_large`; locate the
/// archive (`options.dump_file`, else the cache, downloading with resume
/// if needed); read its table of contents; plan (`--table` list for
/// cycles, sections for `data_only`); drop what is about to be restored
/// (the parent with `CASCADE` for a whole restore, only the selected
/// child tables for a cycle-selective one); run `pg_restore`; verify the
/// tables exist and count their rows; rebuild the namespace's views;
/// record one `loads` row per cycle (source `dump:<name>`).
///
/// Fails with [`DumpError::ExternalTool`] if `pg_restore` cannot run or
/// leaves a table missing (its output is in the message), and with the
/// planning errors described on [`plan_restore`].
pub async fn restore_with(
    pool: &PgPool,
    database_url: &str,
    source: &'static DumpSource,
    cache_dir: &Path,
    options: &RestoreOptions,
) -> Result<RestoreReport> {
    if source.is_large && !options.allow_large && options.cycles.is_empty() {
        return Err(DumpError::LargeDumpNotAllowed {
            name: source.name,
            size: fmt_bytes(source.approx_size_bytes),
        });
    }

    let (dump_path, source_url, remote) = match &options.dump_file {
        Some(path) => {
            if !path.is_file() {
                return Err(DumpError::DumpFileMissing(path.clone()));
            }
            (path.clone(), None, RemoteDump::default())
        }
        None => {
            let endpoints = match &options.endpoints {
                Some(e) => e.clone(),
                None => Endpoints::from_env()?,
            };
            let dir = cache_dir.to_path_buf();
            let url = source.url(&endpoints);
            let cached =
                tokio::task::spawn_blocking(move || download_with(source, &dir, &endpoints))
                    .await??;
            (cached.path, Some(url), cached.meta)
        }
    };

    let toc_path = dump_path.clone();
    let toc = match tokio::task::spawn_blocking(move || read_toc(&toc_path)).await? {
        Ok(toc) => toc,
        // A cached file `pg_restore` rejects is not going to get better on
        // the next run; say which file to remove rather than remove tens
        // of gigabytes on the user's behalf.
        Err(DumpError::ExternalTool { tool, detail }) if options.dump_file.is_none() => {
            return Err(DumpError::ExternalTool {
                tool,
                detail: format!(
                    "{detail}\nif the cached file is damaged, delete {} and re-run to download it again",
                    dump_path.display()
                ),
            });
        }
        Err(e) => return Err(e),
    };
    if !toc.has_table(source.disclosure_table) {
        return Err(DumpError::ExternalTool {
            tool: "pg_restore",
            detail: format!(
                "{} does not contain a TABLE entry for disclosure.{}; is it the {} dump?",
                dump_path.display(),
                source.disclosure_table,
                source.name
            ),
        });
    }

    sqlx::query("CREATE SCHEMA IF NOT EXISTS disclosure")
        .execute(pool)
        .await?;
    // Best-effort: the dumps' post-data indexes need these. If unavailable,
    // the tables, data, and primary keys still restore.
    for ext in ["pg_trgm", "btree_gin"] {
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE EXTENSION IF NOT EXISTS {ext} WITH SCHEMA public"
        )))
        .execute(pool)
        .await;
    }

    let parent_exists = table_exists(pool, source.disclosure_table).await?;
    let plan = plan_restore(source, &toc, options, parent_exists)?;

    // Clean refresh: a re-restore into an existing table would fail every
    // CREATE and duplicate every row. A whole restore drops the parent
    // (CASCADE takes the inheriting children and dependent views with
    // it); a cycle-selective one drops only the children being replaced.
    if plan.partitions.is_empty() {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP TABLE IF EXISTS disclosure.{} CASCADE",
            source.disclosure_table
        )))
        .execute(pool)
        .await?;
    } else {
        for p in &plan.partitions {
            check_ident(&p.table)?;
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "DROP TABLE IF EXISTS disclosure.{} CASCADE",
                p.table
            )))
            .execute(pool)
            .await?;
        }
    }

    let url = database_url.to_string();
    let path = dump_path.clone();
    let run_plan = plan.clone();
    let warnings =
        tokio::task::spawn_blocking(move || run_pg_restore(&url, &path, &run_plan)).await??;

    let counted: Vec<String> = if plan.partitions.is_empty() {
        vec![source.disclosure_table.to_string()]
    } else {
        plan.partitions.iter().map(|p| p.table.clone()).collect()
    };
    let mut rows: i64 = 0;
    let mut per_table: Vec<(Option<Cycle>, i64)> = Vec::with_capacity(counted.len());
    for table in &counted {
        if !table_exists(pool, table).await? {
            return Err(DumpError::ExternalTool {
                tool: "pg_restore",
                detail: format!(
                    "disclosure.{table} does not exist after restore; pg_restore output:\n{}",
                    warnings.join("\n")
                ),
            });
        }
        // `ONLY` so a child's count excludes nothing and the parent's (for
        // a whole restore) includes every child.
        let only = if plan.partitions.is_empty() {
            ""
        } else {
            "ONLY "
        };
        let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {only}disclosure.{table}"
        )))
        .fetch_one(pool)
        .await?;
        rows = rows.saturating_add(n);
        let cycle = plan
            .partitions
            .iter()
            .find(|p| &p.table == table)
            .map(|p| p.cycle);
        per_table.push((cycle, n));
    }

    // The dropped table cascaded away any dependent views; rebuild them.
    crate::db::ensure_views(pool).await?;

    let mut load_ids = Vec::with_capacity(per_table.len());
    for (cycle, n) in &per_table {
        if let Some(id) = record_restore(
            pool,
            source,
            *cycle,
            plan.data_only,
            *n,
            &source_url,
            &remote,
        )
        .await?
        {
            load_ids.push(id);
        }
    }

    Ok(RestoreReport {
        dump_path,
        table: source.disclosure_table,
        tables: counted,
        cycles: plan.partitions.iter().map(|p| p.cycle).collect(),
        rows,
        data_only: plan.data_only,
        indexes_skipped: match (plan.data_only, plan.partitions.is_empty()) {
            (false, _) => 0,
            (true, true) => toc.count(&TocKind::Index),
            (true, false) => plan
                .partitions
                .iter()
                .map(|p| toc.indexes_for_cycle(p.cycle))
                .sum(),
        },
        load_ids,
        warnings,
    })
}

/// Runs `pg_restore`, returning its stderr lines as warnings. Only a
/// failure to *launch* the tool is an error here; whether the restore
/// worked is judged by the caller against the database.
fn run_pg_restore(database_url: &str, dump_path: &Path, plan: &RestorePlan) -> Result<Vec<String>> {
    let output = std::process::Command::new("pg_restore")
        // No `--exit-on-error`: keep going past the expected ignorable
        // errors (missing trigger function) and report them as warnings.
        .args(plan.pg_restore_args())
        .args(["-d", database_url])
        .arg(dump_path)
        .output()
        .map_err(|e| DumpError::ExternalTool {
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

async fn table_exists(pool: &PgPool, table: &str) -> Result<bool> {
    check_ident(table)?;
    let (exists,): (bool,) =
        sqlx::query_as("SELECT to_regclass('disclosure.' || $1::text) IS NOT NULL")
            .bind(table)
            .fetch_one(pool)
            .await?;
    Ok(exists)
}

/// Whether this namespace has a `loads` table (it will not until
/// `schema-init` has run here).
async fn loads_table_exists(pool: &PgPool) -> Result<bool> {
    let (exists,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
                        WHERE table_schema = current_schema() AND table_name = 'loads')",
    )
    .fetch_one(pool)
    .await?;
    Ok(exists)
}

async fn record_restore(
    pool: &PgPool,
    source: &DumpSource,
    cycle: Option<Cycle>,
    data_only: bool,
    rows: i64,
    source_url: &Option<String>,
    remote: &RemoteDump,
) -> Result<Option<i64>> {
    if !loads_table_exists(pool).await? {
        return Ok(None);
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO loads (source, cycle, mode, row_count, row_limit, dates_nulled, \
                            source_url, source_etag, source_last_modified, hardmoney_version) \
         VALUES ($1, $2, $3, $4, NULL, 0, $5, $6, $7, $8) RETURNING load_id",
    )
    .bind(format!("{LOAD_SOURCE_PREFIX}{}", source.name))
    .bind(cycle.map(i32::from))
    .bind(if data_only {
        "restore-data-only"
    } else {
        "restore"
    })
    .bind(rows)
    .bind(source_url)
    .bind(&remote.etag)
    .bind(remote.last_modified)
    .bind(env!("CARGO_PKG_VERSION"))
    .fetch_one(pool)
    .await?;
    Ok(Some(id))
}

/// Refuses anything but `[a-z_][a-z0-9_]*` of at most 63 bytes before it
/// is interpolated into SQL.
fn check_ident(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let ok = match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c == '_' => {
            chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        }
        _ => false,
    };
    if ok && name.len() <= 63 {
        Ok(())
    } else {
        Err(DumpError::UnsafeIdentifier(name.to_string()))
    }
}

/// `43440933` -> `43.4 MB`; `90181919946` -> `90.1 GB`. Decimal units,
/// integer arithmetic.
#[must_use]
pub fn fmt_bytes(n: u64) -> String {
    const UNITS: [(&str, u64); 3] = [("GB", 1_000_000_000), ("MB", 1_000_000), ("kB", 1_000)];
    for (unit, div) in UNITS {
        if n >= div {
            let whole = n / div;
            let tenth = (n % div) * 10 / div;
            return format!("{whole}.{tenth} {unit}");
        }
    }
    format!("{n} B")
}

// ---------------------------------------------------------------------------
// Indexes
// ---------------------------------------------------------------------------

/// One index [`create_indexes`] built.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CreatedIndex {
    pub name: String,
    pub table: String,
    pub elapsed: Duration,
}

/// An index [`create_indexes`] did not build, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SkippedIndex {
    pub name: String,
    pub table: String,
    pub reason: String,
}

/// Outcome of [`create_indexes`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct IndexReport {
    /// The tables that were indexed.
    pub tables: Vec<String>,
    pub created: Vec<CreatedIndex>,
    /// Indexes that were already present (from an earlier run).
    pub existing: Vec<String>,
    pub skipped: Vec<SkippedIndex>,
}

/// Creates hardmoney's indexes ([`DumpSource::indexes`]) on the restored
/// tables of `source`: the child tables for the given `cycles`, every
/// existing child table when `cycles` is empty and the table is split, or
/// the table itself. Idempotent: an index that already exists is
/// reported under `existing`; a unique index on a column that already has
/// one (the archive's primary key) and a trigram index without `pg_trgm`
/// are reported under `skipped`.
///
/// Fails with [`DumpError::TableMissing`] if a target table has not been
/// restored and [`DumpError::NotPartitioned`] if `cycles` is given for a
/// single-table dump.
pub async fn create_indexes(
    pool: &PgPool,
    source: &'static DumpSource,
    cycles: &[Cycle],
) -> Result<IndexReport> {
    let tables = index_targets(pool, source, cycles).await?;
    let _ = sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_trgm WITH SCHEMA public")
        .execute(pool)
        .await;
    let trgm_schema: Option<String> = sqlx::query_scalar(
        "SELECT n.nspname::text FROM pg_extension e \
         JOIN pg_namespace n ON n.oid = e.extnamespace WHERE e.extname = 'pg_trgm'",
    )
    .fetch_optional(pool)
    .await?;
    if let Some(s) = &trgm_schema {
        check_ident(s)?;
    }

    let mut report = IndexReport {
        tables: tables.clone(),
        ..IndexReport::default()
    };
    for table in &tables {
        for spec in source.indexes {
            let name = format!("hm_{table}_{}", spec.suffix);
            check_ident(&name)?;
            if table_exists(pool, &name).await? {
                report.existing.push(name);
                continue;
            }
            if spec.unique && has_unique_index_on(pool, table, spec.columns).await? {
                report.skipped.push(SkippedIndex {
                    name,
                    table: table.clone(),
                    reason: format!(
                        "a unique index on {} already exists (the archive's primary key)",
                        spec.columns
                    ),
                });
                continue;
            }
            let sql = match (spec.trigram, &trgm_schema) {
                (true, None) => {
                    report.skipped.push(SkippedIndex {
                        name,
                        table: table.clone(),
                        reason: "pg_trgm is not installed".to_string(),
                    });
                    continue;
                }
                (true, Some(schema)) => format!(
                    "CREATE INDEX {name} ON disclosure.{table} USING gin ({} {schema}.gin_trgm_ops)",
                    spec.columns
                ),
                (false, _) => format!(
                    "CREATE {}INDEX {name} ON disclosure.{table} ({})",
                    if spec.unique { "UNIQUE " } else { "" },
                    spec.columns
                ),
            };
            let started = Instant::now();
            sqlx::query(sqlx::AssertSqlSafe(sql)).execute(pool).await?;
            report.created.push(CreatedIndex {
                name,
                table: table.clone(),
                elapsed: started.elapsed(),
            });
        }
    }
    Ok(report)
}

async fn index_targets(
    pool: &PgPool,
    source: &'static DumpSource,
    cycles: &[Cycle],
) -> Result<Vec<String>> {
    if !cycles.is_empty() {
        if !source.partitioned_by_cycle {
            return Err(DumpError::NotPartitioned { name: source.name });
        }
        let mut tables = Vec::with_capacity(cycles.len());
        for cycle in cycles {
            let table = partition_table(source.disclosure_table, *cycle);
            if !table_exists(pool, &table).await? {
                return Err(DumpError::TableMissing {
                    table,
                    dump: source.name,
                });
            }
            tables.push(table);
        }
        tables.sort();
        tables.dedup();
        return Ok(tables);
    }
    if !table_exists(pool, source.disclosure_table).await? {
        return Err(DumpError::TableMissing {
            table: source.disclosure_table.to_string(),
            dump: source.name,
        });
    }
    let children = child_tables(pool, source.disclosure_table).await?;
    Ok(if children.is_empty() {
        vec![source.disclosure_table.to_string()]
    } else {
        children
    })
}

/// The inheriting child tables of `disclosure.<parent>`, sorted.
async fn child_tables(pool: &PgPool, parent: &str) -> Result<Vec<String>> {
    check_ident(parent)?;
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT c.relname::text FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid \
         WHERE i.inhparent = to_regclass('disclosure.' || $1::text) ORDER BY 1",
    )
    .bind(parent)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (name,) in rows {
        check_ident(&name)?;
        out.push(name);
    }
    Ok(out)
}

async fn has_unique_index_on(pool: &PgPool, table: &str, column: &str) -> Result<bool> {
    let (exists,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (\
            SELECT 1 FROM pg_index i \
            WHERE i.indrelid = to_regclass('disclosure.' || $1::text) \
              AND i.indisunique AND i.indnatts = 1 \
              AND (SELECT a.attname FROM pg_attribute a \
                   WHERE a.attrelid = i.indrelid AND a.attnum = i.indkey[0]) = $2)",
    )
    .bind(table)
    .bind(column)
    .fetch_one(pool)
    .await?;
    Ok(exists)
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// What the database holds for one dump.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DumpTableState {
    pub table: &'static str,
    pub exists: bool,
    /// Inheriting child tables present (`fec_fitem_sched_a_2023_2024`, ...).
    pub partitions: Vec<String>,
    /// `pg_stat_user_tables.n_live_tup` summed over the table and its
    /// children: maintained by inserts, so right after a restore it is
    /// close but not exact.
    pub approx_rows: i64,
    /// Indexes on the table and its children.
    pub index_count: i64,
}

/// Inspects `disclosure` for `source`'s table, its children, an estimated
/// row count, and its index count. Cheap: catalog queries only.
pub async fn table_state(pool: &PgPool, source: &'static DumpSource) -> Result<DumpTableState> {
    let table = source.disclosure_table;
    let exists = table_exists(pool, table).await?;
    if !exists {
        return Ok(DumpTableState {
            table,
            exists: false,
            partitions: Vec::new(),
            approx_rows: 0,
            index_count: 0,
        });
    }
    let partitions = child_tables(pool, table).await?;
    let (approx_rows,): (i64,) = sqlx::query_as(
        "SELECT coalesce(sum(s.n_live_tup), 0)::bigint FROM pg_stat_user_tables s \
         WHERE s.schemaname = 'disclosure' \
           AND (s.relid = to_regclass('disclosure.' || $1::text) \
                OR s.relid IN (SELECT inhrelid FROM pg_inherits \
                               WHERE inhparent = to_regclass('disclosure.' || $1::text)))",
    )
    .bind(table)
    .fetch_one(pool)
    .await?;
    let (index_count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM pg_index i \
         WHERE i.indrelid = to_regclass('disclosure.' || $1::text) \
            OR i.indrelid IN (SELECT inhrelid FROM pg_inherits \
                              WHERE inhparent = to_regclass('disclosure.' || $1::text))",
    )
    .bind(table)
    .fetch_one(pool)
    .await?;
    Ok(DumpTableState {
        table,
        exists: true,
        partitions,
        approx_rows,
        index_count,
    })
}

/// One `loads` row written by a restore.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RestoreRecord {
    /// The dump's hardmoney name (`schedule_e`).
    pub dump: String,
    /// The cycle for a cycle-selective restore; `None` for a whole one.
    pub cycle: Option<Cycle>,
    pub restored_at: chrono::DateTime<chrono::Utc>,
    /// `restore` or `restore-data-only`.
    pub mode: String,
    pub rows: i64,
    pub source_url: Option<String>,
    pub source_etag: Option<String>,
    pub source_last_modified: Option<chrono::DateTime<chrono::Utc>>,
    pub hardmoney_version: String,
}

/// Restores recorded in this namespace's `loads` table (source
/// `dump:*`), newest first. Empty if the namespace has no `loads` table.
/// Restores run from another namespace are not listed: the data is
/// shared, the record is not.
pub async fn restore_history(pool: &PgPool) -> Result<Vec<RestoreRecord>> {
    if !loads_table_exists(pool).await? {
        return Ok(Vec::new());
    }
    type Row = (
        String,
        Option<i32>,
        chrono::DateTime<chrono::Utc>,
        String,
        i64,
        Option<String>,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
        String,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT source, cycle, loaded_at, mode, row_count, source_url, source_etag, \
                source_last_modified, hardmoney_version \
         FROM loads WHERE source LIKE $1 ORDER BY loaded_at DESC, load_id DESC",
    )
    .bind(format!("{LOAD_SOURCE_PREFIX}%"))
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(source, cycle, at, mode, rows, url, etag, last_modified, version)| RestoreRecord {
                dump: source
                    .strip_prefix(LOAD_SOURCE_PREFIX)
                    .unwrap_or(&source)
                    .to_string(),
                cycle: cycle.and_then(|c| Cycle::try_from(c).ok()),
                restored_at: at,
                mode,
                rows,
                source_url: url,
                source_etag: etag,
                source_last_modified: last_modified,
                hardmoney_version: version,
            },
        )
        .collect())
}

// ---------------------------------------------------------------------------
// Raw vs. processed comparison
// ---------------------------------------------------------------------------

/// A transaction present on both sides with different amounts.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AmountMismatch {
    pub transaction_id: String,
    pub raw: Option<Decimal>,
    pub dump: Option<Decimal>,
}

/// Outcome of [`compare_filing`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CompareReport {
    pub filing_id: i64,
    pub form_type: String,
    pub committee_id: Option<String>,
    /// Schedule E lines hardmoney parsed from the `.fec` file.
    pub raw_rows: i64,
    pub raw_total: Decimal,
    /// Rows in `disclosure.fec_fitem_sched_e` with this `file_num`.
    pub dump_rows: i64,
    pub dump_total: Decimal,
    /// Transaction ids present only in the raw filing.
    pub only_raw: Vec<String>,
    /// Transaction ids present only in the FEC's processed table.
    pub only_dump: Vec<String>,
    pub amount_mismatches: Vec<AmountMismatch>,
    /// The highest `file_num` anywhere in the restored dump: a filing
    /// numbered above it was received after the dump was taken.
    pub dump_newest_file_num: Option<i64>,
}

impl CompareReport {
    /// Rows matched by transaction id on both sides.
    #[must_use]
    pub fn matched(&self) -> i64 {
        let only_raw = i64::try_from(self.only_raw.len()).unwrap_or(i64::MAX);
        self.raw_rows.saturating_sub(only_raw)
    }

    /// Whether the filing's number is above every `file_num` in the dump.
    #[must_use]
    pub fn newer_than_dump(&self) -> bool {
        self.dump_newest_file_num
            .is_some_and(|newest| self.filing_id > newest)
    }
}

/// Compares the Schedule E lines hardmoney ingested for `filing_id`
/// (`schedule_e_lines`, from the raw `.fec` file) with the FEC's
/// processed rows for the same `file_num` in
/// `disclosure.fec_fitem_sched_e`: counts, amount totals, transaction
/// ids on one side only, and amount disagreements on matched ids.
///
/// Differences are expected, not errors: the dump is a weekly snapshot
/// (a filing received after it was taken has no rows yet), it contains no
/// Form 24 rows at all, and the FEC's processing drops or edits lines.
///
/// Fails with [`DumpError::FilingNotIngested`] if the filing is not in
/// this namespace and [`DumpError::TableMissing`] if the Schedule E dump
/// has not been restored.
pub async fn compare_filing(pool: &PgPool, filing_id: i64) -> Result<CompareReport> {
    let filing: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT form_type, committee_id FROM filings WHERE filing_id = $1")
            .bind(filing_id)
            .fetch_optional(pool)
            .await?;
    let (form_type, committee_id) = filing.ok_or(DumpError::FilingNotIngested(filing_id))?;
    if !table_exists(pool, SCHEDULE_E.disclosure_table).await? {
        return Err(DumpError::TableMissing {
            table: SCHEDULE_E.disclosure_table.to_string(),
            dump: SCHEDULE_E.name,
        });
    }

    let (raw_rows, raw_total): (i64, Decimal) = sqlx::query_as(
        "SELECT count(*), coalesce(sum(expenditure_amt), 0)::numeric \
         FROM schedule_e_lines WHERE filing_id = $1",
    )
    .bind(filing_id)
    .fetch_one(pool)
    .await?;
    let (dump_rows, dump_total): (i64, Decimal) = sqlx::query_as(
        "SELECT count(*), coalesce(sum(exp_amt), 0)::numeric \
         FROM disclosure.fec_fitem_sched_e WHERE file_num = $1",
    )
    .bind(filing_id)
    .fetch_one(pool)
    .await?;

    let only_raw: Vec<(String,)> = sqlx::query_as(
        "SELECT coalesce(r.raw->>'transaction_id', '(line ' || r.line_index || ')') \
         FROM schedule_e_lines r \
         WHERE r.filing_id = $1 \
           AND NOT EXISTS (SELECT 1 FROM disclosure.fec_fitem_sched_e d \
                           WHERE d.file_num = $1 \
                             AND d.tran_id = r.raw->>'transaction_id') \
         ORDER BY r.line_index",
    )
    .bind(filing_id)
    .fetch_all(pool)
    .await?;
    let only_dump: Vec<(String,)> = sqlx::query_as(
        "SELECT coalesce(d.tran_id, '(sub_id ' || d.sub_id::text || ')') \
         FROM disclosure.fec_fitem_sched_e d \
         WHERE d.file_num = $1 \
           AND NOT EXISTS (SELECT 1 FROM schedule_e_lines r \
                           WHERE r.filing_id = $1 \
                             AND r.raw->>'transaction_id' = d.tran_id) \
         ORDER BY d.sub_id",
    )
    .bind(filing_id)
    .fetch_all(pool)
    .await?;
    let mismatches: Vec<(String, Option<Decimal>, Option<Decimal>)> = sqlx::query_as(
        "SELECT d.tran_id, r.expenditure_amt, d.exp_amt \
         FROM schedule_e_lines r \
         JOIN disclosure.fec_fitem_sched_e d \
           ON d.file_num = $1 AND d.tran_id = r.raw->>'transaction_id' \
         WHERE r.filing_id = $1 AND r.expenditure_amt IS DISTINCT FROM d.exp_amt \
         ORDER BY d.tran_id",
    )
    .bind(filing_id)
    .fetch_all(pool)
    .await?;
    let dump_newest_file_num: Option<i64> =
        sqlx::query_scalar("SELECT max(file_num)::bigint FROM disclosure.fec_fitem_sched_e")
            .fetch_one(pool)
            .await?;

    Ok(CompareReport {
        filing_id,
        form_type,
        committee_id,
        raw_rows,
        raw_total,
        dump_rows,
        dump_total,
        only_raw: only_raw.into_iter().map(|(t,)| t).collect(),
        only_dump: only_dump.into_iter().map(|(t,)| t).collect(),
        amount_mismatches: mismatches
            .into_iter()
            .map(|(transaction_id, raw, dump)| AmountMismatch {
                transaction_id,
                raw,
                dump,
            })
            .collect(),
        dump_newest_file_num,
    })
}

// ---------------------------------------------------------------------------
// Helpers for the guided `hardmoney dumps` commands
// ---------------------------------------------------------------------------

impl RemoteDump {
    /// Whether the server's file is a different (newer) one than `local`,
    /// where `local` is what was recorded when the file was downloaded or
    /// restored. ETags are compared when both sides have one, else
    /// `Last-Modified` dates; `None` when neither side has anything to
    /// compare, or the local side has nothing recorded at all.
    #[must_use]
    pub fn is_newer_than(&self, local: &RemoteDump) -> Option<bool> {
        if let (Some(a), Some(b)) = (&self.etag, &local.etag) {
            return Some(a != b);
        }
        match (self.last_modified, local.last_modified) {
            (Some(remote), Some(local)) => Some(remote > local),
            _ => None,
        }
    }

    /// Whether anything is recorded (an ETag or a date).
    #[must_use]
    pub fn has_validator(&self) -> bool {
        self.etag.is_some() || self.last_modified.is_some()
    }
}

impl RestoreRecord {
    /// The FEC file this restore came from, as recorded (`source_etag`,
    /// `source_last_modified`; no size), for comparing with
    /// [`remote_info`] via [`RemoteDump::is_newer_than`]. Empty for a
    /// restore from a local file.
    #[must_use]
    pub fn source_version(&self) -> RemoteDump {
        RemoteDump {
            size: None,
            etag: self.source_etag.clone(),
            last_modified: self.source_last_modified,
        }
    }
}

/// Deletes `source`'s cached archive and its `.partial` and `.meta`
/// sidecars from `cache_dir`, so the next [`download`] fetches the FEC's
/// current file. Returns whether any file was removed. Missing files are
/// not an error; any other I/O failure is.
pub fn evict_cached(source: &DumpSource, cache_dir: &Path) -> std::io::Result<bool> {
    let dest = cache_path(source, cache_dir);
    let mut removed = false;
    for path in [dest.clone(), partial_path(&dest), meta_path(&dest)] {
        match std::fs::remove_file(&path) {
            Ok(()) => removed = true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(removed)
}

/// Drops `disclosure.<table>` for `source` (`CASCADE`: its per-cycle
/// child tables and every namespace's views over it go too), then
/// rebuilds this namespace's views so the ones for other dumps remain.
/// Returns whether the table existed. The `loads` history is kept.
pub async fn drop_restored(pool: &PgPool, source: &DumpSource) -> Result<bool> {
    let existed = table_exists(pool, source.disclosure_table).await?;
    if existed {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP TABLE IF EXISTS disclosure.{} CASCADE",
            source.disclosure_table
        )))
        .execute(pool)
        .await?;
    }
    crate::db::ensure_views(pool).await?;
    Ok(existed)
}

/// The plan [`restore_with`] will settle on, worked out without the
/// archive: child-table names come from [`partition_table`] instead of
/// the table of contents. For showing someone the `pg_restore` command
/// before a 90 GB download starts. `parent_exists` as for
/// [`plan_restore`].
///
/// Fails with [`DumpError::NotPartitioned`] if cycles were requested for
/// a single-table dump and [`DumpError::LargeDumpNotAllowed`] for a whole
/// large dump without `allow_large`. A cycle the real archive turns out
/// not to have is caught later by [`plan_restore`].
pub fn plan_restore_offline(
    source: &DumpSource,
    options: &RestoreOptions,
    parent_exists: bool,
) -> Result<RestorePlan> {
    if options.cycles.is_empty() {
        if source.is_large && !options.allow_large {
            return Err(DumpError::LargeDumpNotAllowed {
                name: source.name,
                size: fmt_bytes(source.approx_size_bytes),
            });
        }
        return Ok(RestorePlan {
            tables: Vec::new(),
            partitions: Vec::new(),
            data_only: options.data_only,
            jobs: options.jobs,
        });
    }
    if !source.partitioned_by_cycle {
        return Err(DumpError::NotPartitioned { name: source.name });
    }
    let partitions: Vec<Partition> = options
        .cycles
        .iter()
        .map(|c| Partition {
            cycle: *c,
            table: partition_table(source.disclosure_table, *c),
        })
        .collect();
    let mut tables = Vec::with_capacity(partitions.len().saturating_add(1));
    if !parent_exists {
        tables.push(source.disclosure_table.to_string());
    }
    tables.extend(partitions.iter().map(|p| p.table.clone()));
    Ok(RestorePlan {
        tables,
        partitions,
        data_only: true,
        jobs: options.jobs,
    })
}

/// The complete command line [`restore_with`] runs for `plan`, as one
/// string per argument: `pg_restore`, the plan's arguments, `-d`, the
/// database URL, and the archive path. Exactly what
/// `hardmoney dumps import --explain` prints.
#[must_use]
pub fn pg_restore_command(plan: &RestorePlan, database_url: &str, dump_path: &Path) -> Vec<String> {
    let mut argv = Vec::with_capacity(plan.tables.len().saturating_add(6));
    argv.push("pg_restore".to_string());
    argv.extend(plan.pg_restore_args());
    argv.push("-d".to_string());
    argv.push(database_url.to_string());
    argv.push(dump_path.display().to_string());
    argv
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// The real `pg_restore --list` of `fec_fitem_sched_e.dump` (13 Sept 2026).
    const SCHED_E_LIST: &str = "\
;
; Archive created at 2026-09-13 12:01:50 EDT
;     dbname: fec
;     TOC Entries: 8
;     Compression: gzip
;     Dump Version: 1.14-0
;     Format: CUSTOM
;     Integer: 4 bytes
;     Offset: 8 bytes
;     Dumped from database version: 15.10
;     Dumped by pg_dump version: 15.16
;
;
; Selected TOC Entries:
;
358; 1259 17839 TABLE disclosure fec_fitem_sched_e fec
8792; 0 17839 TABLE DATA disclosure fec_fitem_sched_e fec
8395; 2606 19168 CONSTRAINT disclosure fec_fitem_sched_e fec_fitem_sched_e_pkey fec
8396; 2620 19532 TRIGGER disclosure fec_fitem_sched_e tri_fec_fitem_sched_e fec
";

    /// A Schedule A-shaped listing (synthetic: names follow the FEC
    /// README's partition and index naming).
    const SCHED_A_LIST: &str = "\
; Archive created at 2026-09-13 20:00:00 EDT
5; 2615 16389 SCHEMA - disclosure fec
300; 1255 17000 FUNCTION disclosure fec_fitem_sched_a_insert() fec
358; 1259 17839 TABLE disclosure fec_fitem_sched_a fec
359; 1259 17840 TABLE disclosure fec_fitem_sched_a_1975_1976 fec
360; 1259 17841 TABLE disclosure fec_fitem_sched_a_2023_2024 fec
361; 1259 17842 TABLE disclosure fec_fitem_sched_a_2025_2026 fec
362; 1259 17843 TABLE disclosure fec_fitem_sched_a_notes fec
8792; 0 17839 TABLE DATA disclosure fec_fitem_sched_a fec
8793; 0 17841 TABLE DATA disclosure fec_fitem_sched_a_2023_2024 fec
8100; 1259 18000 INDEX disclosure idx_sched_a_2023_2024_amt_dt_sub_id fec
8101; 1259 18001 INDEX disclosure idx_sched_a_2023_2024_cmte_id_dt_sub_id_desc fec
8395; 2606 19168 CONSTRAINT disclosure fec_fitem_sched_a_2023_2024 fec_fitem_sched_a_2023_2024_pkey fec
8396; 2620 19532 TRIGGER disclosure fec_fitem_sched_a_2023_2024 tri_fec_fitem_sched_a_2023_2024 fec
9000; 0 0 COMMENT - EXTENSION pg_trgm
";

    #[test]
    fn parses_real_schedule_e_listing() {
        let toc = DumpToc::parse(SCHED_E_LIST);
        assert_eq!(
            toc.archive_created.as_deref(),
            Some("2026-09-13 12:01:50 EDT")
        );
        assert_eq!(toc.source_server_version.as_deref(), Some("15.10"));
        assert_eq!(toc.entries.len(), 4);
        let kinds: Vec<&TocKind> = toc.entries.iter().map(|e| &e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &TocKind::Table,
                &TocKind::TableData,
                &TocKind::Constraint,
                &TocKind::Trigger
            ]
        );
        let table = &toc.entries[0];
        assert_eq!(table.dump_id, 358);
        assert_eq!(table.schema.as_deref(), Some("disclosure"));
        assert_eq!(table.name, "fec_fitem_sched_e");
        assert_eq!(table.owner.as_deref(), Some("fec"));
        let pkey = &toc.entries[2];
        assert_eq!(pkey.name, "fec_fitem_sched_e fec_fitem_sched_e_pkey");
        assert_eq!(pkey.table(), Some("fec_fitem_sched_e"));
        assert_eq!(toc.entries[3].table(), Some("fec_fitem_sched_e"));
        assert!(toc.has_table("fec_fitem_sched_e"));
        assert!(toc.partitions("fec_fitem_sched_e").is_empty());
        assert_eq!(toc.count(&TocKind::Index), 0);
    }

    #[test]
    fn parses_partitioned_listing() {
        let toc = DumpToc::parse(SCHED_A_LIST);
        assert_eq!(toc.entries.len(), 14);
        let parts = toc.partitions("fec_fitem_sched_a");
        assert_eq!(
            parts.iter().map(|p| p.cycle.year()).collect::<Vec<_>>(),
            vec![1976, 2024, 2026]
        );
        assert_eq!(parts[1].table, "fec_fitem_sched_a_2023_2024");
        assert_eq!(toc.count(&TocKind::Index), 2);
        assert_eq!(toc.count(&TocKind::Function), 1);
        let schema = &toc.entries[0];
        assert_eq!(schema.kind, TocKind::Schema);
        assert_eq!(schema.schema, None);
        assert_eq!(schema.name, "disclosure");
        let func = &toc.entries[1];
        assert_eq!(func.name, "fec_fitem_sched_a_insert()");
        assert_eq!(func.table(), None);
        let index = toc
            .entries
            .iter()
            .find(|e| e.kind == TocKind::Index)
            .unwrap();
        assert_eq!(index.name, "idx_sched_a_2023_2024_amt_dt_sub_id");
        assert_eq!(index.table(), None);
        assert_eq!(toc.indexes_for_cycle(Cycle::new(2024).unwrap()), 2);
        assert_eq!(toc.indexes_for_cycle(Cycle::new(2026).unwrap()), 0);
        let comment = toc.entries.last().unwrap();
        assert_eq!(comment.kind, TocKind::Other("COMMENT".into()));
    }

    #[test]
    fn toc_entry_rejects_non_entries() {
        assert_eq!(TocEntry::parse(""), None);
        assert_eq!(TocEntry::parse("; Archive created at ..."), None);
        assert_eq!(TocEntry::parse("not a toc line"), None);
        assert_eq!(
            TocEntry::parse("358; abc 17839 TABLE disclosure x fec"),
            None
        );
        assert_eq!(TocEntry::parse("358; 1259 17839"), None);
        assert_eq!(TocEntry::parse("358; 1259 17839 lowercase x"), None);
        let ownerless = TocEntry::parse("2; 0 0 ENCODING - ENCODING").unwrap();
        assert_eq!(ownerless.kind, TocKind::Other("ENCODING".into()));
        assert_eq!(ownerless.name, "ENCODING");
        assert_eq!(ownerless.owner, None);
    }

    #[test]
    fn partition_names_round_trip() {
        let c2024 = Cycle::new(2024).unwrap();
        assert_eq!(
            partition_table("fec_fitem_sched_a", c2024),
            "fec_fitem_sched_a_2023_2024"
        );
        assert_eq!(
            partition_table("fec_fitem_sched_b", Cycle::MIN),
            "fec_fitem_sched_b_1975_1976"
        );
        assert_eq!(
            partition_cycle("fec_fitem_sched_a", "fec_fitem_sched_a_2023_2024"),
            Some(c2024)
        );
        for bad in [
            "fec_fitem_sched_a",
            "fec_fitem_sched_a_2023_2025",
            "fec_fitem_sched_a_2024_2025",
            "fec_fitem_sched_a_2023-2024",
            "fec_fitem_sched_a_23_24",
            "fec_fitem_sched_a_notes",
            "fec_fitem_sched_b_2023_2024",
            "fec_fitem_sched_a_2023_2024_old",
        ] {
            assert_eq!(partition_cycle("fec_fitem_sched_a", bad), None, "{bad}");
        }
    }

    #[test]
    fn plan_whole_and_selective_restores() {
        let toc = DumpToc::parse(SCHED_A_LIST);
        let whole = plan_restore(&SCHEDULE_A, &toc, &RestoreOptions::new(), false).unwrap();
        assert!(whole.tables.is_empty());
        assert!(!whole.data_only);
        assert_eq!(whole.pg_restore_args(), vec!["--no-owner", "--no-acl"]);

        let data_only = plan_restore(
            &SCHEDULE_A,
            &toc,
            &RestoreOptions::new().data_only(true).jobs(4),
            false,
        )
        .unwrap();
        assert_eq!(
            data_only.pg_restore_args(),
            vec![
                "--no-owner",
                "--no-acl",
                "--jobs=4",
                "--section=pre-data",
                "--section=data"
            ]
        );

        let cycles = [Cycle::new(2026).unwrap(), Cycle::new(2024).unwrap()];
        let selective = plan_restore(
            &SCHEDULE_A,
            &toc,
            &RestoreOptions::new().cycles(cycles),
            false,
        )
        .unwrap();
        assert!(selective.data_only, "--table never restores post-data");
        assert_eq!(
            selective.tables,
            vec![
                "fec_fitem_sched_a",
                "fec_fitem_sched_a_2023_2024",
                "fec_fitem_sched_a_2025_2026"
            ]
        );
        assert_eq!(
            selective.pg_restore_args(),
            vec![
                "--no-owner",
                "--no-acl",
                "--table=fec_fitem_sched_a",
                "--table=fec_fitem_sched_a_2023_2024",
                "--table=fec_fitem_sched_a_2025_2026"
            ]
        );
        // With the parent already in place, only the children are named.
        let incremental = plan_restore(
            &SCHEDULE_A,
            &toc,
            &RestoreOptions::new().cycles([Cycle::new(2026).unwrap()]),
            true,
        )
        .unwrap();
        assert_eq!(incremental.tables, vec!["fec_fitem_sched_a_2025_2026"]);
    }

    #[test]
    fn plan_rejects_missing_or_inapplicable_cycles() {
        let toc = DumpToc::parse(SCHED_A_LIST);
        let err = plan_restore(
            &SCHEDULE_A,
            &toc,
            &RestoreOptions::new().cycles([Cycle::new(2000).unwrap()]),
            false,
        )
        .unwrap_err();
        match err {
            DumpError::MissingPartition {
                cycle, available, ..
            } => {
                assert_eq!(cycle.year(), 2000);
                assert_eq!(available, "1976, 2024, 2026");
            }
            other => panic!("{other}"),
        }
        let e_toc = DumpToc::parse(SCHED_E_LIST);
        let err = plan_restore(
            &SCHEDULE_E,
            &e_toc,
            &RestoreOptions::new().cycles([Cycle::new(2024).unwrap()]),
            false,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            DumpError::NotPartitioned { name: "schedule_e" }
        ));
    }

    #[test]
    fn bytes_format_and_identifiers() {
        assert_eq!(fmt_bytes(43_440_933), "43.4 MB");
        assert_eq!(fmt_bytes(90_181_919_946), "90.1 GB");
        assert_eq!(fmt_bytes(14_190_177), "14.1 MB");
        assert_eq!(fmt_bytes(1_500), "1.5 kB");
        assert_eq!(fmt_bytes(999), "999 B");
        assert!(check_ident("fec_fitem_sched_a_2023_2024").is_ok());
        assert!(check_ident("_x").is_ok());
        for bad in ["", "2024", "Fec", "a-b", "a;drop", "a b", "a.b"] {
            assert!(check_ident(bad).is_err(), "{bad}");
        }
        assert!(check_ident(&"a".repeat(64)).is_err());
    }

    #[test]
    fn content_range_parsing() {
        assert_eq!(
            parse_content_range("bytes 100-199/43440933"),
            Some((100, Some(43440933)))
        );
        assert_eq!(parse_content_range("bytes 5-9/*"), Some((5, None)));
        assert_eq!(parse_content_range("items 0-1/2"), None);
        assert_eq!(parse_unsatisfied_range("bytes */43440933"), Some(43440933));
    }

    #[test]
    fn large_dump_needs_allow_large_only_for_whole_restore() {
        // Pure check of the guard, mirrored from `restore_with`.
        let guard =
            |o: &RestoreOptions| SCHEDULE_A.is_large && !o.allow_large && o.cycles.is_empty();
        assert!(guard(&RestoreOptions::new()));
        assert!(!guard(&RestoreOptions::new().allow_large(true)));
        assert!(!guard(
            &RestoreOptions::new().cycles([Cycle::new(2026).unwrap()])
        ));
    }

    #[test]
    fn dump_error_maps_into_bulk_error() {
        let e: BulkError = DumpError::NotPartitioned { name: "schedule_e" }.into();
        assert!(matches!(
            e,
            BulkError::ExternalTool {
                tool: "pg_restore",
                ..
            }
        ));
        assert!(e.to_string().contains("--cycles does not apply"));
        let e: BulkError = DumpError::Http {
            url: "u".into(),
            detail: "d".into(),
        }
        .into();
        assert!(matches!(e, BulkError::Http { .. }));
    }

    // -- Range resume with a fake server --------------------------------

    /// A server holding `bytes`; `honor_range` false makes it answer every
    /// request with the whole body (a server that ignores `Range`, or a
    /// changed file failing `If-Range`); `ignore_if_range` makes it serve
    /// the range even when the validator does not match (a server that
    /// implements `Range` but not `If-Range`); `cut_after` truncates the
    /// body it sends to simulate a dropped connection.
    struct FakeServer {
        bytes: Vec<u8>,
        etag: &'static str,
        honor_range: bool,
        ignore_if_range: bool,
        cut_after: Option<usize>,
        requests: Vec<(u64, Option<String>)>,
    }

    impl RangeFetch for FakeServer {
        fn fetch(&mut self, from: u64, if_range: Option<&str>) -> Result<RangeResponse> {
            self.requests.push((from, if_range.map(str::to_string)));
            let total = self.bytes.len() as u64;
            let quoted = format!("\"{}\"", self.etag);
            let validator_ok = self.ignore_if_range || if_range == Some(quoted.as_str());
            let start = if from > 0 && self.honor_range && validator_ok {
                from
            } else {
                0
            };
            if start > total {
                return Ok(RangeResponse {
                    start,
                    total: Some(total),
                    etag: Some(self.etag.into()),
                    last_modified: None,
                    body: Box::new(std::io::empty()),
                });
            }
            let mut body = self.bytes[start as usize..].to_vec();
            if let Some(cut) = self.cut_after {
                body.truncate(cut);
            }
            Ok(RangeResponse {
                start,
                total: Some(total),
                etag: Some(self.etag.into()),
                last_modified: None,
                body: Box::new(Cursor::new(body)),
            })
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hardmoney-dump-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("x.dump")
    }

    #[test]
    fn resume_appends_after_a_dropped_connection() {
        let dest = scratch("resume");
        let payload: Vec<u8> = (0..=255u8).cycle().take(10_000).collect();
        let mut server = FakeServer {
            bytes: payload.clone(),
            etag: "abc-6",
            honor_range: true,
            ignore_if_range: false,
            cut_after: Some(4_000),
            requests: Vec::new(),
        };
        let err = download_resumable(&mut server, "u", &dest, false).unwrap_err();
        match err {
            DumpError::Incomplete {
                expected, actual, ..
            } => {
                assert_eq!(expected, 10_000);
                assert_eq!(actual, 4_000);
            }
            other => panic!("{other}"),
        }
        assert!(!dest.exists());
        assert_eq!(std::fs::metadata(partial_path(&dest)).unwrap().len(), 4_000);
        let meta = RemoteDump::read_meta(&meta_path(&dest)).unwrap();
        assert_eq!(meta.etag.as_deref(), Some("abc-6"));

        server.cut_after = None;
        let remote = download_resumable(&mut server, "u", &dest, false).unwrap();
        assert_eq!(remote.size, Some(10_000));
        assert_eq!(std::fs::read(&dest).unwrap(), payload);
        assert!(!partial_path(&dest).exists());
        // Second request resumed at 4000 and carried the saved ETag.
        assert_eq!(server.requests[1], (4_000, Some("\"abc-6\"".to_string())));
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }

    #[test]
    fn partial_without_a_validator_is_not_resumed() {
        let dest = scratch("novalidator");
        // A `.partial` from hardmoney 2.1.0: no `.meta` next to it.
        std::fs::write(partial_path(&dest), b"half of last week's file").unwrap();
        let mut server = FakeServer {
            bytes: b"this week's file".to_vec(),
            etag: "w2",
            honor_range: true,
            ignore_if_range: false,
            cut_after: None,
            requests: Vec::new(),
        };
        download_resumable(&mut server, "u", &dest, false).unwrap();
        assert_eq!(server.requests, vec![(0, None)]);
        assert_eq!(std::fs::read(&dest).unwrap(), b"this week's file");
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }

    #[test]
    fn if_range_prefers_etag_then_date() {
        let date = chrono::DateTime::parse_from_rfc3339("2026-09-13T11:03:27Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let both = RemoteDump {
            size: None,
            etag: Some("abc-6".into()),
            last_modified: Some(date),
        };
        assert_eq!(both.if_range_value().as_deref(), Some("\"abc-6\""));
        let date_only = RemoteDump {
            etag: None,
            ..both.clone()
        };
        assert_eq!(
            date_only.if_range_value().as_deref(),
            Some("Sun, 13 Sep 2026 11:03:27 GMT")
        );
        assert_eq!(RemoteDump::default().if_range_value(), None);
    }

    #[test]
    fn resume_restarts_when_server_ignores_range() {
        let dest = scratch("restart");
        std::fs::write(partial_path(&dest), b"stale bytes from last week").unwrap();
        RemoteDump {
            size: None,
            etag: Some("old".into()),
            last_modified: None,
        }
        .write_meta(&meta_path(&dest))
        .unwrap();
        let mut server = FakeServer {
            bytes: b"fresh file".to_vec(),
            etag: "new",
            honor_range: true,
            ignore_if_range: false,
            cut_after: None,
            requests: Vec::new(),
        };
        let remote = download_resumable(&mut server, "u", &dest, false).unwrap();
        assert_eq!(remote.etag.as_deref(), Some("new"));
        assert_eq!(std::fs::read(&dest).unwrap(), b"fresh file");
        // Asked to resume with the old tag; the 200 answer replaced the partial.
        assert_eq!(server.requests[0], (26, Some("\"old\"".to_string())));
        assert_eq!(
            RemoteDump::read_meta(&meta_path(&dest))
                .unwrap()
                .etag
                .as_deref(),
            Some("new")
        );
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }

    /// A server that serves the requested range but ignores `If-Range`
    /// would splice a new file onto an old partial; the changed ETag on
    /// its 206 is caught and the download starts over.
    #[test]
    fn resume_restarts_when_the_etag_changed_under_a_206() {
        let dest = scratch("etag206");
        std::fs::write(partial_path(&dest), b"stale bytes from last week").unwrap();
        RemoteDump {
            size: None,
            etag: Some("old".into()),
            last_modified: None,
        }
        .write_meta(&meta_path(&dest))
        .unwrap();
        let mut server = FakeServer {
            bytes: b"fresh file that is longer than the stale partial".to_vec(),
            etag: "new",
            honor_range: true,
            ignore_if_range: true,
            cut_after: None,
            requests: Vec::new(),
        };
        let remote = download_resumable(&mut server, "u", &dest, false).unwrap();
        assert_eq!(remote.etag.as_deref(), Some("new"));
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"fresh file that is longer than the stale partial"
        );
        // The 206 with the wrong tag was abandoned for a fresh request.
        assert_eq!(server.requests.len(), 2);
        assert_eq!(server.requests[0], (26, Some("\"old\"".to_string())));
        assert_eq!(server.requests[1], (0, None));
        assert_eq!(
            RemoteDump::read_meta(&meta_path(&dest))
                .unwrap()
                .etag
                .as_deref(),
            Some("new")
        );
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }

    #[test]
    fn resume_with_complete_partial_just_renames() {
        let dest = scratch("complete");
        std::fs::write(partial_path(&dest), b"all here").unwrap();
        RemoteDump {
            size: Some(8),
            etag: Some("t".into()),
            last_modified: None,
        }
        .write_meta(&meta_path(&dest))
        .unwrap();
        let mut server = FakeServer {
            bytes: b"all here".to_vec(),
            etag: "t",
            honor_range: true,
            ignore_if_range: false,
            cut_after: None,
            requests: Vec::new(),
        };
        let remote = download_resumable(&mut server, "u", &dest, false).unwrap();
        assert_eq!(remote.size, Some(8));
        assert_eq!(std::fs::read(&dest).unwrap(), b"all here");
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }

    #[test]
    fn oversized_partial_is_discarded() {
        let dest = scratch("oversized");
        std::fs::write(partial_path(&dest), b"twelve bytes").unwrap();
        RemoteDump {
            size: None,
            etag: Some("t".into()),
            last_modified: None,
        }
        .write_meta(&meta_path(&dest))
        .unwrap();
        let mut server = FakeServer {
            bytes: b"short".to_vec(),
            etag: "t",
            honor_range: true,
            ignore_if_range: false,
            cut_after: None,
            requests: Vec::new(),
        };
        let err = download_resumable(&mut server, "u", &dest, false).unwrap_err();
        assert!(matches!(err, DumpError::Http { .. }), "{err}");
        assert!(!partial_path(&dest).exists());
        assert!(!dest.exists());
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }

    #[test]
    fn meta_sidecar_round_trips() {
        let dest = scratch("meta");
        let meta = RemoteDump {
            size: Some(42),
            etag: Some("e-1".into()),
            last_modified: Some(
                chrono::DateTime::parse_from_rfc3339("2026-09-13T11:03:27Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            ),
        };
        meta.write_meta(&meta_path(&dest)).unwrap();
        assert_eq!(RemoteDump::read_meta(&meta_path(&dest)).unwrap(), meta);
        assert_eq!(RemoteDump::read_meta(&dest.with_extension("nope")), None);
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }

    #[test]
    fn cached_reports_partial_and_complete() {
        let dir = scratch("cached").parent().unwrap().to_path_buf();
        let state = cached(&SCHEDULE_E, &dir);
        assert_eq!(state.complete_bytes, None);
        assert_eq!(state.partial_bytes, None);
        std::fs::write(partial_path(&state.path), b"abc").unwrap();
        assert_eq!(cached(&SCHEDULE_E, &dir).partial_bytes, Some(3));
        std::fs::rename(partial_path(&state.path), &state.path).unwrap();
        let done = cached(&SCHEDULE_E, &dir);
        assert_eq!(done.complete_bytes, Some(3));
        assert_eq!(done.partial_bytes, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sources_are_consistent() {
        let production = Endpoints::default();
        let mirror = production
            .clone()
            .with_www_base(crate::fec::Url::parse("https://mirror.example.gov/fec").unwrap());
        for s in ALL {
            assert_eq!(find(s.name).map(|f| f.name), Some(s.name));
            assert!(s.file_name.ends_with(".dump"));
            assert!(!s.file_name.contains('/'));
            assert_eq!(
                s.url(&production),
                format!(
                    "https://www.fec.gov/files/bulk-downloads/data-dump/schedules/{}",
                    s.file_name
                )
            );
            assert_eq!(
                s.url(&mirror),
                format!(
                    "https://mirror.example.gov/fec/files/bulk-downloads/data-dump/schedules/{}",
                    s.file_name
                )
            );
            assert!(!s.indexes.is_empty());
            assert!(check_ident(s.disclosure_table).is_ok());
            for spec in s.indexes {
                let name = format!(
                    "hm_{}_{}",
                    partition_table(s.disclosure_table, Cycle::MAX),
                    spec.suffix
                );
                assert!(check_ident(&name).is_ok(), "{name} exceeds 63 bytes");
            }
        }
        assert!(find("nope").is_none());
        assert_eq!(INDEX_URL, production.dump_readme());
        let large: Vec<&str> = ALL.iter().filter(|s| s.is_large).map(|s| s.name).collect();
        assert_eq!(large, vec!["schedule_a_full", "schedule_b_full"]);
        let split: Vec<&str> = ALL
            .iter()
            .filter(|s| s.partitioned_by_cycle)
            .map(|s| s.name)
            .collect();
        assert_eq!(split, large);
    }

    #[test]
    fn compare_report_helpers() {
        let r = CompareReport {
            filing_id: 2011823,
            form_type: "F24N".into(),
            committee_id: None,
            raw_rows: 3,
            raw_total: Decimal::ZERO,
            dump_rows: 0,
            dump_total: Decimal::ZERO,
            only_raw: vec!["a".into(), "b".into(), "c".into()],
            only_dump: Vec::new(),
            amount_mismatches: Vec::new(),
            dump_newest_file_num: Some(2011113),
        };
        assert_eq!(r.matched(), 0);
        assert!(r.newer_than_dump());
    }
}

#[cfg(test)]
mod guided_helper_tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hardmoney-dump-guided-{tag}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn remote_newer_than_local() {
        let at = |s: &str| {
            chrono::DateTime::parse_from_rfc3339(s)
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        let remote = RemoteDump {
            size: Some(1),
            etag: Some("b".into()),
            last_modified: Some(at("2026-09-20T11:00:00Z")),
        };
        let same = RemoteDump {
            etag: Some("b".into()),
            ..RemoteDump::default()
        };
        let older = RemoteDump {
            etag: Some("a".into()),
            last_modified: Some(at("2026-09-13T11:00:00Z")),
            size: None,
        };
        assert_eq!(remote.is_newer_than(&same), Some(false));
        assert_eq!(remote.is_newer_than(&older), Some(true));
        // ETag wins even when the dates would disagree.
        let same_tag_older_date = RemoteDump {
            etag: Some("b".into()),
            last_modified: Some(at("2020-01-01T00:00:00Z")),
            size: None,
        };
        assert_eq!(remote.is_newer_than(&same_tag_older_date), Some(false));
        // Dates only.
        let dated = RemoteDump {
            last_modified: Some(at("2026-09-13T11:00:00Z")),
            ..RemoteDump::default()
        };
        assert_eq!(remote.is_newer_than(&dated), Some(true));
        assert_eq!(dated.is_newer_than(&remote), Some(false));
        // Nothing to compare.
        assert_eq!(remote.is_newer_than(&RemoteDump::default()), None);
        assert_eq!(RemoteDump::default().is_newer_than(&remote), None);
        assert!(remote.has_validator());
        assert!(dated.has_validator());
        assert!(!RemoteDump::default().has_validator());
        assert!(
            !RemoteDump {
                size: Some(3),
                ..RemoteDump::default()
            }
            .has_validator()
        );
    }

    #[test]
    fn evict_removes_every_cache_file() {
        let dir = scratch("evict");
        assert!(
            !evict_cached(&SCHEDULE_E, &dir).unwrap(),
            "nothing there yet"
        );
        let dest = cache_path(&SCHEDULE_E, &dir);
        std::fs::write(&dest, b"x").unwrap();
        std::fs::write(partial_path(&dest), b"y").unwrap();
        std::fs::write(meta_path(&dest), b"etag=z\n").unwrap();
        assert!(cached(&SCHEDULE_E, &dir).complete_bytes.is_some());
        assert!(evict_cached(&SCHEDULE_E, &dir).unwrap());
        let after = cached(&SCHEDULE_E, &dir);
        assert!(after.complete_bytes.is_none());
        assert!(after.partial_bytes.is_none());
        assert_eq!(after.meta, RemoteDump::default());
        assert!(!evict_cached(&SCHEDULE_E, &dir).unwrap());
        // Another dump's files in the same directory are untouched.
        let other = cache_path(&COMMITTEE_HISTORY, &dir);
        std::fs::write(&other, b"keep").unwrap();
        assert!(!evict_cached(&SCHEDULE_E, &dir).unwrap());
        assert!(other.is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn offline_plan_matches_the_toc_plan() {
        let c2024 = Cycle::new(2024).unwrap();
        let c2026 = Cycle::new(2026).unwrap();
        let opts = RestoreOptions::new().cycles([c2026, c2024]).jobs(2);
        let plan = plan_restore_offline(&SCHEDULE_A, &opts, false).unwrap();
        assert_eq!(
            plan.tables,
            vec![
                "fec_fitem_sched_a",
                "fec_fitem_sched_a_2023_2024",
                "fec_fitem_sched_a_2025_2026"
            ]
        );
        assert!(plan.data_only);
        assert_eq!(plan.jobs, 2);
        assert_eq!(plan.partitions.len(), 2);
        let with_parent = plan_restore_offline(&SCHEDULE_A, &opts, true).unwrap();
        assert_eq!(
            with_parent.tables,
            vec!["fec_fitem_sched_a_2023_2024", "fec_fitem_sched_a_2025_2026"]
        );

        // Same answer as the TOC-based planner when the archive has the
        // cycles.
        let listing = "\
1; 1259 1 TABLE disclosure fec_fitem_sched_a fec
2; 1259 2 TABLE disclosure fec_fitem_sched_a_2023_2024 fec
3; 1259 3 TABLE disclosure fec_fitem_sched_a_2025_2026 fec
";
        let toc = DumpToc::parse(listing);
        assert_eq!(plan_restore(&SCHEDULE_A, &toc, &opts, false).unwrap(), plan);

        // Whole small dump.
        let whole = plan_restore_offline(&SCHEDULE_E, &RestoreOptions::new(), false).unwrap();
        assert!(whole.tables.is_empty());
        assert!(!whole.data_only);
        let data_only =
            plan_restore_offline(&SCHEDULE_E, &RestoreOptions::new().data_only(true), false)
                .unwrap();
        assert!(data_only.data_only);

        // Refusals.
        let err = plan_restore_offline(&SCHEDULE_E, &RestoreOptions::new().cycles([c2024]), false)
            .unwrap_err();
        assert!(matches!(err, DumpError::NotPartitioned { .. }), "{err}");
        let err = plan_restore_offline(&SCHEDULE_A, &RestoreOptions::new(), false).unwrap_err();
        assert!(
            matches!(err, DumpError::LargeDumpNotAllowed { .. }),
            "{err}"
        );
        assert!(
            plan_restore_offline(&SCHEDULE_A, &RestoreOptions::new().allow_large(true), false)
                .is_ok()
        );
    }

    #[test]
    fn command_line_is_what_restore_runs() {
        let plan = plan_restore_offline(
            &SCHEDULE_A,
            &RestoreOptions::new().cycles([Cycle::new(2026).unwrap()]),
            false,
        )
        .unwrap();
        let argv = pg_restore_command(
            &plan,
            "postgres://u@localhost/fec",
            Path::new("/tmp/schedule_a_full.dump"),
        );
        assert_eq!(
            argv,
            vec![
                "pg_restore",
                "--no-owner",
                "--no-acl",
                "--table=fec_fitem_sched_a",
                "--table=fec_fitem_sched_a_2025_2026",
                "-d",
                "postgres://u@localhost/fec",
                "/tmp/schedule_a_full.dump",
            ]
        );
        let whole =
            plan_restore_offline(&SCHEDULE_E, &RestoreOptions::new().data_only(true), false)
                .unwrap();
        let argv = pg_restore_command(&whole, "postgres://u@localhost/fec", Path::new("e.dump"));
        assert_eq!(
            argv,
            vec![
                "pg_restore",
                "--no-owner",
                "--no-acl",
                "--section=pre-data",
                "--section=data",
                "-d",
                "postgres://u@localhost/fec",
                "e.dump",
            ]
        );
    }
}
