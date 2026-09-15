//! Error types for the parser.
//!
//! Every error that can be traced to a specific physical line of a filing
//! carries its 1-based `line_no`, so a user can find the offending record
//! in a 100 MB filing without bisecting it.

use thiserror::Error;

use crate::parser::schema::SpecVersion;
use crate::parser::tables::Table;

/// Any failure while parsing or writing a `.fec` filing.
///
/// `#[non_exhaustive]`: new variants (and new fields on existing variants)
/// may be added in minor releases; match with a wildcard arm.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FecError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Spec 3.x-5.x filings are comma-delimited CSV; this is a CSV-level
    /// failure (e.g. an unterminated quote) in one of them.
    #[error("csv error: {0}")]
    Csv(#[from] csv::Error),

    #[error("filing is too old / uses a deprecated header format (starts with '/*')")]
    DeprecatedHeaderFormat,

    /// The header's version is not one this crate knows (electronic 3.x-8.x).
    #[error("unsupported or malformed FEC version in header: '{0}'")]
    UnknownElectronicHeaderVersion(String),

    #[error("filing has no cover/summary line")]
    MissingFormLine,

    /// The table exists but has no column layout for this spec version --
    /// typically a filing from a spec version newer than the bundled tables.
    #[error(
        "no column layout for table {table} at spec version {version}{}",
        fmt_line(*line_no)
    )]
    NoMatchingVersionBucket {
        table: Table,
        version: SpecVersion,
        line_no: Option<u64>,
    },

    /// The line's form-type token (column 0) matched no dispatch pattern.
    #[error(
        "no format table for form type '{form_type}' (spec version {version}){}",
        fmt_line(*line_no)
    )]
    ParserMissing {
        form_type: String,
        version: SpecVersion,
        line_no: Option<u64>,
    },

    /// A `[BEGINTEXT]` block was opened but the file ended before `[ENDTEXT]`.
    #[error("unterminated [BEGINTEXT] block starting at line {line_no}")]
    UnterminatedTextBlock { line_no: u64 },

    /// A field name that does not exist in the line's layout was used with
    /// [`ParsedLine::set`](crate::parser::ParsedLine::set) or
    /// [`ParsedLine::from_pairs`](crate::parser::ParsedLine::from_pairs).
    #[error("table {table} has no field named '{field}'")]
    UnknownField { table: Table, field: String },

    /// One or more body lines were skipped under a lenient
    /// [`ParseOptions`](crate::parser::ParseOptions) and the caller asked
    /// for strict semantics via [`Lenient::into_strict`](crate::parser::Lenient::into_strict).
    #[error("{count} line(s) could not be parsed; first: {first}")]
    LinesSkipped { count: usize, first: Box<FecError> },

    #[cfg(feature = "fetch")]
    #[error("network error fetching filing: {0}")]
    Fetch(#[from] ureq::Error),
}

fn fmt_line(line_no: Option<u64>) -> String {
    match line_no {
        Some(n) => format!(" at line {n}"),
        None => String::new(),
    }
}

impl FecError {
    /// The 1-based physical line number this error refers to, if known.
    #[must_use]
    pub fn line_no(&self) -> Option<u64> {
        match self {
            FecError::NoMatchingVersionBucket { line_no, .. }
            | FecError::ParserMissing { line_no, .. } => *line_no,
            FecError::UnterminatedTextBlock { line_no } => Some(*line_no),
            FecError::LinesSkipped { first, .. } => first.line_no(),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, FecError>;
