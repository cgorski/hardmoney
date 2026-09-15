//! Error types for the parser.
//!
//! Every error that can be traced to a specific physical line of a filing
//! carries its 1-based `line_no`, so a user can find the offending record
//! in a 100 MB filing without bisecting it.

use thiserror::Error;

/// Any failure while parsing a `.fec` filing or its format tables.
///
/// `#[non_exhaustive]`: new variants (and new fields on existing variants)
/// may be added in minor releases; match with a wildcard arm.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FecError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("csv error: {0}")]
    Csv(#[from] csv::Error),

    #[error("invalid regex '{pattern}': {source}")]
    Regex {
        pattern: String,
        #[source]
        source: regex::Error,
    },

    #[error("filing is too old / uses a deprecated header format (starts with '/*')")]
    DeprecatedHeaderFormat,

    #[error("couldn't find a header parser for electronic version '{0}'")]
    UnknownElectronicHeaderVersion(String),

    #[error("couldn't find a header parser for paper version '{0}'")]
    UnknownPaperHeaderVersion(String),

    #[error("filing has no summary/form line")]
    MissingFormLine,

    /// The filing's form type ends in `A` (amendment) but the header's
    /// `report_id` does not contain the `FEC-<n>` reference to the original.
    #[error("can't find original filing number in amended report (report_id was '{report_id}')")]
    AmendmentOriginalNotFound { report_id: String },

    #[error("no known format table found for form '{form}'")]
    UnknownForm { form: String },

    /// The table exists but has no column layout for this spec version --
    /// typically a filing from a spec version newer than the bundled tables.
    #[error(
        "no column-position data to parse table {table} at spec version '{version}'{}",
        fmt_line(*line_no)
    )]
    NoMatchingVersionBucket {
        table: &'static str,
        version: String,
        line_no: Option<u64>,
    },

    /// The line's form-type token (column 0) matched no dispatch pattern.
    #[error(
        "couldn't find a line parser for form type '{form_type}' (spec version '{version}'){}",
        fmt_line(*line_no)
    )]
    ParserMissing {
        form_type: String,
        version: String,
        line_no: Option<u64>,
    },

    /// A `[BEGINTEXT]` block was opened but the file ended before `[ENDTEXT]`.
    #[error("unterminated [BEGINTEXT] block starting at line {line_no}")]
    UnterminatedTextBlock { line_no: u64 },

    /// A fec-csv-sources format table assigns the same canonical field name
    /// to two different column positions within the same version bucket.
    /// `IndexMap::insert` would silently keep only the later one, quietly
    /// dropping real data from real filings (this happened for real in
    /// F3X.csv/F3P.csv/F4.csv/F2.csv/SchC1.csv/SchL.csv -- see NOTICE).
    /// Surfacing it as a hard error means any future upstream data update
    /// that reintroduces a collision fails loudly instead of silently.
    #[error(
        "canonical field '{canonical}' is assigned to two different columns ({first_position} and {second_position}) in form '{form}' version bucket '{version_bucket}' -- this is a data error in the format table, not a valid duplicate"
    )]
    DuplicateCanonicalField {
        form: String,
        version_bucket: String,
        canonical: String,
        first_position: usize,
        second_position: usize,
    },

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
    pub fn line_no(&self) -> Option<u64> {
        match self {
            FecError::NoMatchingVersionBucket { line_no, .. }
            | FecError::ParserMissing { line_no, .. } => *line_no,
            FecError::UnterminatedTextBlock { line_no } => Some(*line_no),
            _ => None,
        }
    }

    /// Attaches a line number to an error that supports one; a no-op for
    /// errors that are not about a specific line.
    pub(crate) fn at_line(mut self, n: u64) -> Self {
        match &mut self {
            FecError::NoMatchingVersionBucket { line_no, .. }
            | FecError::ParserMissing { line_no, .. } => *line_no = Some(n),
            _ => {}
        }
        self
    }
}

pub type Result<T> = std::result::Result<T, FecError>;
