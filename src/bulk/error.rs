use std::fmt;

/// Errors from the `bulk` module (downloading, unzipping, and loading FEC
/// bulk-data files or restoring the FEC's official pg_dump files).
#[derive(Debug)]
pub enum BulkError {
    Http(String),
    Io(std::io::Error),
    Zip(String),
    Database(sqlx::Error),
    /// A source row had fewer pipe-delimited fields than its
    /// [`crate::bulk::source::BulkSource`] declares, or more than are
    /// permitted by `allow_extra_trailing_fields`.
    MalformedRow { line_no: u64, expected: usize, found: usize },
    UnknownSource(String),
    /// `pg_restore` (or `pg_dump`'s companion tool) was not found on PATH,
    /// or exited non-zero.
    ExternalTool { tool: &'static str, detail: String },
}

impl fmt::Display for BulkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BulkError::Http(msg) => write!(f, "HTTP error: {msg}"),
            BulkError::Io(e) => write!(f, "I/O error: {e}"),
            BulkError::Zip(msg) => write!(f, "zip error: {msg}"),
            BulkError::Database(e) => write!(f, "database error: {e}"),
            BulkError::MalformedRow { line_no, expected, found } => write!(
                f,
                "malformed row at line {line_no}: expected {expected} fields, found {found}"
            ),
            BulkError::UnknownSource(name) => write!(f, "unknown bulk source: {name}"),
            BulkError::ExternalTool { tool, detail } => write!(f, "{tool} failed: {detail}"),
        }
    }
}

impl std::error::Error for BulkError {}

impl From<std::io::Error> for BulkError {
    fn from(e: std::io::Error) -> Self {
        BulkError::Io(e)
    }
}

impl From<sqlx::Error> for BulkError {
    fn from(e: sqlx::Error) -> Self {
        BulkError::Database(e)
    }
}

impl From<zip::result::ZipError> for BulkError {
    fn from(e: zip::result::ZipError) -> Self {
        BulkError::Zip(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, BulkError>;
