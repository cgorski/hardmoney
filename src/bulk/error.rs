//! Errors from the `bulk` module: downloading, unzipping, and loading FEC
//! bulk-data files, restoring the FEC's official pg_dump files, and
//! ingesting individual filings.

use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BulkError {
    #[error("HTTP error fetching {url}: {detail}")]
    Http { url: String, detail: String },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("zip archive error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("archive for {source_name} contains no entries")]
    EmptyArchive { source_name: &'static str },

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    /// The namespace's schema is behind the embedded migration set. Bulk
    /// loads refuse to write into a stale schema; run `schema-init`.
    #[error("schema is behind: {pending} pending migration(s); run `hardmoney schema-init` first")]
    SchemaOutOfDate { pending: usize },

    #[error("filing parse error: {0}")]
    Parse(#[from] crate::parser::FecError),

    /// Serialising a parsed record to JSON for a `JSONB` column failed.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// A source row had fewer pipe-delimited fields than its
    /// [`crate::bulk::source::BulkSource`] declares, or more than are
    /// permitted by `allow_extra_trailing_fields`.
    #[error("malformed row at line {line_no}: expected {expected} fields, found {found}")]
    MalformedRow {
        line_no: u64,
        expected: usize,
        found: usize,
    },

    #[error("unknown bulk source '{0}'")]
    UnknownSource(String),

    #[error("unknown dump '{0}'")]
    UnknownDump(String),

    /// `pg_restore` was not found on PATH, or exited with a real error.
    #[error("{tool} failed: {detail}")]
    ExternalTool { tool: &'static str, detail: String },

    /// `--replace` would delete more rows than the confirmation threshold
    /// and the caller did not pass `--yes`.
    #[error(
        "replacing {table} for cycle {cycle} would delete {existing} existing rows; pass --yes to confirm"
    )]
    ConfirmationRequired {
        table: &'static str,
        cycle: i32,
        existing: i64,
    },

    /// A blocking task panicked or was cancelled.
    #[error("background task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

pub type Result<T> = std::result::Result<T, BulkError>;
