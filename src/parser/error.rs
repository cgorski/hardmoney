use thiserror::Error;

#[derive(Debug, Error)]
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

    #[error("can't find original filing number in amended report {filing_number} (report_id was '{report_id}')")]
    AmendmentOriginalNotFound {
        filing_number: String,
        report_id: String,
    },

    #[error("no known format table found for form '{form}'")]
    UnknownForm { form: String },

    #[error("can't find column-position data to parse line type='{form}' version='{version}'")]
    NoMatchingVersionBucket { form: String, version: String },

    #[error("couldn't find a line parser for form type '{form_type}' (version {version})")]
    ParserMissing { form_type: String, version: String },

    #[error("unimplemented header-summary processing for form type '{0}'")]
    UnimplementedFormProcessing(String),

    /// A fec-csv-sources format table assigns the same canonical field name
    /// to two different column positions within the same version bucket.
    /// `IndexMap::insert` would silently keep only the later one, quietly
    /// dropping real data from real filings (this happened for real in
    /// F3X.csv/F3P.csv/F4.csv/F2.csv/SchC1.csv/SchL.csv -- see CHANGELOG).
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

    #[cfg(feature = "fetch")]
    #[error("network error fetching filing: {0}")]
    Fetch(#[from] ureq::Error),
}

pub type Result<T> = std::result::Result<T, FecError>;
