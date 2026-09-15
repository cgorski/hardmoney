//! Export a filing's records as tabular data: CSV, JSON Lines, Parquet, or
//! SQLite (`hardmoney export`).
//!
//! A `.fec` file is a stack of heterogeneous records -- one cover line, then
//! Schedule A lines, Schedule B lines, `TEXT` records, and so on -- each with
//! its own column layout. Nothing downstream of the parser (a spreadsheet,
//! DuckDB, pandas, `sqlite3`) wants that shape. This module writes **one
//! output table per [`Table`]** that occurs in the filing, named after the
//! table (`SchA`, `SchB`, `F3X`, `TEXT`, ...), so a filing becomes a small
//! directory of homogeneous files or a single SQLite database.
//!
//! # Columns
//!
//! Every line of one table in one filing is parsed with the same [`Layout`]
//! (the filing has one spec version), so each output table has a stable
//! column set, in this order:
//!
//! 1. `filing_id` -- only with [`ExportOptions::include_filing_id`]; the
//!    numeric FEC filing id, usually derived from the file name
//!    ([`filing_id_from_path`]).
//! 2. `line_no` -- the 1-based physical line in the source file, so any row
//!    can be traced back to the raw bytes (the cover line is always line 2).
//! 3. The layout's canonical fields in FEC column order, starting with
//!    `form_type` -- the same `lower_snake_case` names
//!    [`ParsedLine::get`] uses, every field present, blanks as empty strings
//!    (or `NULL` for the typed Parquet columns).
//!
//! The cover line is written as the single row of its own table (`F3X` for
//! a Form 3X, `F1` for a Form 1, ...).
//!
//! # Formats and types
//!
//! | Format | Output | Field types |
//! |---|---|---|
//! | [`Format::Csv`] | `<out>/<Table>.csv`, RFC 4180 quoting, header row | all text, exactly as filed |
//! | [`Format::Jsonl`] | `<out>/<Table>.jsonl`, one object per row | all strings; `filing_id`/`line_no` are numbers |
//! | [`Format::Parquet`] | `<out>/<Table>.parquet`, zstd | typed: see below |
//! | [`Format::Sqlite`] | one `<out>` database, one table per `Table` plus `filings` | all `TEXT`; `filing_id`/`line_no` are `INTEGER` |
//!
//! Parquet is the one format with a real type system, and columns are typed
//! from the FEC's own field specifications
//! ([`FieldSpec::kind`](crate::parser::FieldSpec::kind)):
//!
//! | FEC type | Arrow / Parquet type | Blank or unparseable |
//! |---|---|---|
//! | `AMT-n` ([`FieldKind::Amount`](crate::parser::FieldKind::Amount)) | `Decimal128(38, 2)` via [`parse_money`](crate::parse_money) | `NULL` |
//! | `NUM-8` ([`FieldKind::Numeric`](crate::parser::FieldKind::Numeric), `max_len == 8`; every such field is a `YYYYMMDD` date) | `Date32` via [`parse_fec_date`](crate::parse_fec_date) | `NULL` |
//! | everything else, including other `NUM-n` (ZIP codes keep their leading zeros) | `Utf8` | `""` |
//! | `line_no`, `filing_id` | `Int64` | -- |
//!
//! **Money is never a float**, here or anywhere else in this crate. Parquet
//! amounts are exact decimals with two digits of scale; every other format
//! carries the amount as the string the filer wrote (`"15.00"`), which is
//! also exact. See the book chapter *Exporting a Filing* for how to
//! aggregate those in DuckDB and `sqlite3` without going through `REAL`.
//!
//! # Streaming
//!
//! Export runs on [`FilingReader`], so memory does not grow with the size
//! of the filing: one parsed record plus each open table's write buffer.
//! For CSV, JSON Lines, and SQLite that is a few kilobytes per table (the
//! 135 MB, 704,652-line presidential filing 2010101 exports to CSV in a
//! 13 MB peak and to SQLite in 17 MB). Parquet accumulates one
//! [`RecordBatch`](arrow::record_batch::RecordBatch) of 16,384 rows per
//! table before encoding it, and the `parquet` writer holds the encoded
//! pages of the current row group (up to 1,048,576 rows) until it is
//! flushed, so its peak is bounded by a row group rather than by the file
//! (104 MB for the same filing).
//!
//! # Example
//!
//! ```no_run
//! use std::path::Path;
//!
//! use hardmoney::Table;
//! use hardmoney::export::{ExportOptions, Format, export_path};
//!
//! let opts = ExportOptions::new(Format::Parquet)
//!     .only([Table::SchA, Table::SchB])
//!     .include_filing_id_from(Path::new("2011821.fec"))?;
//! let report = export_path(Path::new("2011821.fec"), Path::new("out"), &opts)?;
//! for t in &report.tables {
//!     println!("{}: {} rows", t.table, t.rows);
//! }
//! # Ok::<(), hardmoney::export::ExportError>(())
//! ```

mod csv_out;
mod jsonl_out;
mod parquet_out;
mod sqlite_out;

use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::parser::form::table_for_form_type;
use crate::parser::schema::Layout;
use crate::parser::stream::Preamble;
use crate::parser::{FecError, FilingReader, ParseOptions, ParsedLine, Table};

/// Read buffer for [`export_path`]; same reasoning as `Filing::open`.
const READ_BUFFER_BYTES: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// Format
// ---------------------------------------------------------------------------

/// An output format for [`export_reader`] / [`export_path`].
///
/// `Display`/`FromStr` use the lower-case names `csv`, `jsonl`, `parquet`,
/// `sqlite` (the `--format` values of `hardmoney export`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, strum::Display, strum::EnumString)]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[non_exhaustive]
pub enum Format {
    /// One `<Table>.csv` per table in the output directory, with a header
    /// row and RFC 4180 quoting. Every value is text, exactly as filed.
    #[default]
    Csv,
    /// One `<Table>.jsonl` per table: one JSON object per row, keys in
    /// column order, blank fields as `""`.
    Jsonl,
    /// One `<Table>.parquet` per table, zstd-compressed, with amounts as
    /// `Decimal128(38, 2)` and dates as `Date32` (see the module docs).
    Parquet,
    /// A single SQLite database with one table per `Table` and a `filings`
    /// metadata table. Every field column is `TEXT`.
    Sqlite,
}

impl Format {
    /// The file extension this format writes: `csv`, `jsonl`, `parquet`, or
    /// `sqlite`.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Jsonl => "jsonl",
            Format::Parquet => "parquet",
            Format::Sqlite => "sqlite",
        }
    }

    /// Whether the output path is a directory of per-table files (`true`)
    /// or a single file (`false`, SQLite only).
    #[must_use]
    pub const fn writes_directory(self) -> bool {
        !matches!(self, Format::Sqlite)
    }

    /// The default output path for exporting `input` in this format, in the
    /// current directory: `<stem>.<extension>` -- a directory for the
    /// per-table formats, a file for SQLite. An input with no file stem
    /// (e.g. `..`) uses `export` as the stem.
    #[must_use]
    pub fn default_out(self, input: &Path) -> PathBuf {
        let stem = input
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("export");
        PathBuf::from(format!("{stem}.{}", self.extension()))
    }
}

// ---------------------------------------------------------------------------
// Options and report
// ---------------------------------------------------------------------------

/// What to export and how. Build with [`ExportOptions::new`] and the
/// chaining setters; the fields are public for reading.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ExportOptions {
    /// The output format.
    pub format: Format,
    /// Restrict the export to these tables (the cover line is included only
    /// if its table is listed). `None` exports every table in the filing.
    pub only: Option<Vec<Table>>,
    /// Prepend a `filing_id` column with this value to every table.
    pub include_filing_id: Option<u64>,
    /// Parse leniently: body lines with an unknown form-type token or no
    /// layout for the filing's version are skipped and counted in
    /// [`ExportReport::skipped`] instead of failing the export. Consulted
    /// by [`export_path`]; [`export_reader`] follows the [`ParseOptions`]
    /// its reader was built with.
    pub lenient: bool,
}

impl ExportOptions {
    /// Options for `format`: every table, no `filing_id` column, strict.
    #[must_use]
    pub fn new(format: Format) -> Self {
        Self {
            format,
            ..Self::default()
        }
    }

    /// Export only `tables`. An empty set exports nothing but (for SQLite)
    /// the `filings` row.
    #[must_use]
    pub fn only(mut self, tables: impl IntoIterator<Item = Table>) -> Self {
        self.only = Some(tables.into_iter().collect());
        self
    }

    /// Prepend a `filing_id` column holding `filing_id` to every table.
    #[must_use]
    pub fn include_filing_id(mut self, filing_id: u64) -> Self {
        self.include_filing_id = Some(filing_id);
        self
    }

    /// Like [`include_filing_id`](Self::include_filing_id) with the id
    /// derived from `path`'s file name by [`filing_id_from_path`]. Fails
    /// with [`ExportError::FilingIdNotInFilename`] if the name contains no
    /// run of four or more digits.
    pub fn include_filing_id_from(self, path: &Path) -> Result<Self, ExportError> {
        match filing_id_from_path(path) {
            Some(id) => Ok(self.include_filing_id(id)),
            None => Err(ExportError::FilingIdNotInFilename {
                path: path.to_path_buf(),
            }),
        }
    }

    /// Skip unparseable body lines instead of failing (see
    /// [`ExportOptions::lenient`]).
    #[must_use]
    pub fn lenient(mut self, lenient: bool) -> Self {
        self.lenient = lenient;
        self
    }

    fn parse_options(&self) -> ParseOptions {
        if self.lenient {
            ParseOptions::LENIENT
        } else {
            ParseOptions::STRICT
        }
    }
}

/// Rows and bytes written for one output table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TableStats {
    /// The table (also the output file's stem, or the SQLite table name).
    pub table: Table,
    /// Rows written, including the cover line if this is its table.
    pub rows: u64,
    /// Bytes of this table's output file. `None` for SQLite, where every
    /// table shares one database file (see [`ExportReport::bytes`]).
    pub bytes: Option<u64>,
}

impl TableStats {
    /// Stats for `table`: `rows` written, `bytes` of its own file (`None`
    /// when it shares a file with other tables).
    #[must_use]
    pub fn new(table: Table, rows: u64, bytes: Option<u64>) -> Self {
        Self { table, rows, bytes }
    }
}

/// What an export wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ExportReport {
    /// One entry per table written, in order of first appearance in the
    /// filing (so the cover line's table comes first).
    pub tables: Vec<TableStats>,
    /// Body lines skipped under lenient parsing; always 0 when strict.
    pub skipped: usize,
    /// Total bytes on disk: the sum of the per-table files, or the size of
    /// the SQLite database.
    pub bytes: u64,
}

impl ExportReport {
    /// Total rows written across every table.
    #[must_use]
    pub fn rows(&self) -> u64 {
        self.tables.iter().map(|t| t.rows).sum()
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Any failure while exporting a filing.
///
/// `#[non_exhaustive]`: new variants may be added in minor releases.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExportError {
    /// The filing could not be parsed (bad header, unknown line under strict
    /// options, unterminated text block, ...). Carries the line number
    /// where one applies.
    #[error(transparent)]
    Fec(#[from] FecError),

    /// A file or directory could not be created, written, or read. The
    /// message names the path.
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    /// The CSV writer failed (a record with the wrong number of fields, or
    /// an underlying write failure).
    #[error("csv error: {0}")]
    Csv(#[from] csv::Error),

    /// A JSON Lines row could not be serialised.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// An Arrow record batch could not be assembled.
    #[error("arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    /// The Parquet writer failed.
    #[error("parquet error: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),

    /// The SQLite database could not be opened or written.
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// A `--only` token named neither a table nor an upper-case form-type
    /// token (see [`resolve_table`]).
    #[error(
        "'{0}' is not a table name (SchA, F3X, TEXT, ...) or an upper-case form-type token (SA, SB21B, SE, ...)"
    )]
    UnknownTable(String),

    /// A `filing_id` column was requested but the file name has no run of
    /// four or more digits to take it from.
    #[error(
        "cannot derive a filing id from '{}': the file name has no run of 4 or more digits (FEC downloads are named <id>.fec); pass the id explicitly",
        path.display()
    )]
    FilingIdNotInFilename { path: PathBuf },

    /// A `filing_id` does not fit the signed 64-bit integer every output
    /// format stores it in.
    #[error("filing id {0} does not fit in a 64-bit signed integer")]
    FilingIdOutOfRange(u64),
}

/// An I/O error that names the file it concerns.
fn io_at(path: &Path, e: io::Error) -> ExportError {
    ExportError::Io(io::Error::new(e.kind(), format!("{}: {e}", path.display())))
}

// ---------------------------------------------------------------------------
// Public helpers
// ---------------------------------------------------------------------------

/// Derives a filing id from a local `.fec` path.
///
/// FEC document-store downloads are named `<id>.fec`; this crate's own
/// fixtures are `<FORM>_<id>[_v<spec>].fec`. The id is the **last** run of
/// ASCII digits in the file stem that is at least four digits long, so
/// `F24N_2011823.fec` is 2011823 (not 242011823) and `F3XA_27789_v3.fec`
/// is 27789. Returns `None` if there is no such run, or if the stem is not
/// valid Unicode. (The same rule as `bulk::ingest::filing_id_from_path`,
/// kept separate so the `export` feature does not depend on `bulk`.)
#[must_use]
pub fn filing_id_from_path(path: &Path) -> Option<u64> {
    let stem = path.file_stem()?.to_str()?;
    stem.split(|c: char| !c.is_ascii_digit())
        .rfind(|run| run.len() >= 4)
        .and_then(|run| run.parse().ok())
}

/// Resolves a `--only` token to a [`Table`]: a table name as
/// [`Table`]'s `FromStr` accepts it (`SchA`, `f3x`, `TEXT`; case-insensitive),
/// or an **upper-case** form-type token as it appears in column 0 of a
/// filing line (`SA`, `SB21B`, `SE`, `F3XN`), dispatched through
/// [`table_for_form_type`].
///
/// Tokens must be upper case because dispatch is prefix-based and
/// case-insensitive: a mistyped table name such as `ScheA` would otherwise
/// silently become Schedule C (`SC`). Fails with
/// [`ExportError::UnknownTable`] on anything else.
pub fn resolve_table(token: &str) -> Result<Table, ExportError> {
    let trimmed = token.trim();
    if let Ok(table) = trimmed.parse::<Table>() {
        return Ok(table);
    }
    let is_upper_token = !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '/');
    if is_upper_token && let Some(table) = table_for_form_type(trimmed) {
        return Ok(table);
    }
    Err(ExportError::UnknownTable(token.to_string()))
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Exports every record a [`FilingReader`] yields -- the cover line from its
/// preamble, then the body -- to `out` in `opts.format`.
///
/// `out` is a directory for CSV, JSON Lines, and Parquet (created if
/// missing; existing `<Table>.<ext>` files of the same names are replaced)
/// and a database file for SQLite (created if missing; rows are appended to
/// existing tables with the same columns). With
/// [`ExportOptions::only`], the reader is restricted with
/// [`FilingReader::filter_tables`] and the cover line is written only if its
/// table is listed.
///
/// The reader's own [`ParseOptions`] decide what happens to unparseable
/// lines; [`ExportReport::skipped`] is its skipped count. The SQLite
/// `filings.path` column is `NULL` here because a reader has no path; use
/// [`export_path`] to record one.
///
/// Fails with [`ExportError::Fec`] on a parse error, [`ExportError::Io`]
/// (naming the path) if the output cannot be created or written,
/// [`ExportError::FilingIdOutOfRange`] if `include_filing_id` exceeds
/// `i64::MAX`, and with the format's own error otherwise. On failure the
/// output is left as far as it got, except that an SQLite export is rolled
/// back as a whole.
pub fn export_reader<R: BufRead>(
    reader: FilingReader<R>,
    out: &Path,
    opts: &ExportOptions,
) -> Result<ExportReport, ExportError> {
    export_inner(reader, out, opts, None)
}

/// Opens the filing at `path` (strictly, or leniently with
/// [`ExportOptions::lenient`]) and exports it to `out` as
/// [`export_reader`] does.
///
/// Fails with [`ExportError::Io`] naming `path` if it cannot be opened,
/// and otherwise as [`export_reader`] fails.
pub fn export_path(
    path: &Path,
    out: &Path,
    opts: &ExportOptions,
) -> Result<ExportReport, ExportError> {
    let file = File::open(path).map_err(|e| io_at(path, e))?;
    let reader = FilingReader::with_options(
        BufReader::with_capacity(READ_BUFFER_BYTES, file),
        opts.parse_options(),
    )?;
    export_inner(reader, out, opts, Some(path))
}

fn export_inner<R: BufRead>(
    mut reader: FilingReader<R>,
    out: &Path,
    opts: &ExportOptions,
    source: Option<&Path>,
) -> Result<ExportReport, ExportError> {
    let filing_id = match opts.include_filing_id {
        Some(id) => Some(i64::try_from(id).map_err(|_| ExportError::FilingIdOutOfRange(id))?),
        None => None,
    };
    let mut sink = open_sink(opts.format, out, filing_id, reader.preamble(), source)?;

    let summary = &reader.preamble().summary;
    let wanted = |table: Table| opts.only.as_ref().is_none_or(|only| only.contains(&table));
    if wanted(summary.table()) {
        sink.write(summary)?;
    }
    if let Some(only) = &opts.only {
        reader = reader.filter_tables(only.iter().copied());
    }
    for line in reader.by_ref() {
        sink.write(&line?)?;
    }
    let skipped = reader.skipped().len();

    let (tables, bytes) = sink.finish()?;
    Ok(ExportReport {
        tables,
        skipped,
        bytes,
    })
}

fn open_sink(
    format: Format,
    out: &Path,
    filing_id: Option<i64>,
    preamble: &Preamble,
    source: Option<&Path>,
) -> Result<Box<dyn Sink>, ExportError> {
    if format.writes_directory() {
        std::fs::create_dir_all(out).map_err(|e| io_at(out, e))?;
    }
    Ok(match format {
        Format::Csv => Box::new(PerTableFiles::<csv_out::CsvTable>::new(
            out, format, filing_id,
        )),
        Format::Jsonl => Box::new(PerTableFiles::<jsonl_out::JsonlTable>::new(
            out, format, filing_id,
        )),
        Format::Parquet => Box::new(PerTableFiles::<parquet_out::ParquetTable>::new(
            out, format, filing_id,
        )),
        Format::Sqlite => Box::new(sqlite_out::SqliteSink::open(
            out, filing_id, preamble, source,
        )?),
    })
}

// ---------------------------------------------------------------------------
// Sinks (internal)
// ---------------------------------------------------------------------------

/// Where records go. One implementation per family of formats: per-table
/// files ([`PerTableFiles`]) and the single-database SQLite sink.
trait Sink {
    /// Writes one record to its table's output, opening that output on the
    /// table's first record.
    fn write(&mut self, line: &ParsedLine) -> Result<(), ExportError>;
    /// Flushes and closes every output; returns per-table stats and the
    /// total bytes on disk.
    fn finish(self: Box<Self>) -> Result<(Vec<TableStats>, u64), ExportError>;
}

/// The column set of one output table: an optional leading `filing_id`,
/// then `line_no`, then the layout's fields in order.
pub(crate) struct Columns {
    pub(crate) filing_id: Option<i64>,
    pub(crate) layout: &'static Layout,
}

impl Columns {
    /// Column names in output order.
    pub(crate) fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.filing_id
            .is_some()
            .then_some("filing_id")
            .into_iter()
            .chain(std::iter::once("line_no"))
            .chain(self.layout.fields.iter().map(|f| f.name))
    }
}

/// One record ready to be written as a row.
pub(crate) struct Row<'a> {
    pub(crate) filing_id: Option<i64>,
    pub(crate) line: &'a ParsedLine,
}

impl Row<'_> {
    /// `line_no` as the `Int64`/`INTEGER` every format stores it in.
    /// Saturates at `i64::MAX` (no real file has that many lines).
    pub(crate) fn line_no(&self) -> i64 {
        i64::try_from(self.line.line_no).unwrap_or(i64::MAX)
    }
}

/// One output file holding one table, for the per-table-file formats.
pub(crate) trait TableFile: Sized {
    /// Creates (truncating) the file at `path` with the header, schema, or
    /// whatever the format needs up front for `columns`.
    fn open(path: &Path, columns: &Columns) -> Result<Self, ExportError>;
    /// Appends one row.
    fn write(&mut self, row: Row<'_>) -> Result<(), ExportError>;
    /// Flushes and closes the file; returns the bytes written to it.
    fn finish(self) -> Result<u64, ExportError>;
}

struct OpenTable<T> {
    table: Table,
    file: T,
    rows: u64,
}

/// The per-table-file sink: a directory, one [`TableFile`] per table seen,
/// opened lazily on the table's first record.
struct PerTableFiles<T> {
    dir: PathBuf,
    format: Format,
    filing_id: Option<i64>,
    open: Vec<OpenTable<T>>,
}

impl<T: TableFile> PerTableFiles<T> {
    fn new(dir: &Path, format: Format, filing_id: Option<i64>) -> Self {
        Self {
            dir: dir.to_path_buf(),
            format,
            filing_id,
            open: Vec::new(),
        }
    }

    fn path_for(&self, table: Table) -> PathBuf {
        self.dir
            .join(format!("{}.{}", table.as_str(), self.format.extension()))
    }
}

impl<T: TableFile> Sink for PerTableFiles<T> {
    fn write(&mut self, line: &ParsedLine) -> Result<(), ExportError> {
        let row = Row {
            filing_id: self.filing_id,
            line,
        };
        if let Some(open) = self.open.iter_mut().find(|o| o.table == line.table()) {
            open.file.write(row)?;
            open.rows += 1;
            return Ok(());
        }
        // First record of this table: its layout is the table's layout for
        // the whole filing (one spec version per filing), so the header /
        // schema written here fits every later record of the table.
        let path = self.path_for(line.table());
        let columns = Columns {
            filing_id: self.filing_id,
            layout: line.layout(),
        };
        let mut file = T::open(&path, &columns)?;
        file.write(row)?;
        self.open.push(OpenTable {
            table: line.table(),
            file,
            rows: 1,
        });
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<(Vec<TableStats>, u64), ExportError> {
        let mut stats = Vec::with_capacity(self.open.len());
        let mut total = 0u64;
        for open in self.open {
            let bytes = open.file.finish()?;
            total = total.saturating_add(bytes);
            stats.push(TableStats {
                table: open.table,
                rows: open.rows,
                bytes: Some(bytes),
            });
        }
        Ok((stats, total))
    }
}

/// A buffered file that counts the bytes written through it, so the report
/// can say how big each output is without a second `stat`.
pub(crate) struct CountingFile {
    inner: io::BufWriter<File>,
    bytes: u64,
}

impl CountingFile {
    /// Creates (truncating) `path`. The error names the path.
    pub(crate) fn create(path: &Path) -> Result<Self, ExportError> {
        let file = File::create(path).map_err(|e| io_at(path, e))?;
        Ok(Self {
            inner: io::BufWriter::new(file),
            bytes: 0,
        })
    }

    /// Flushes and returns the bytes written.
    pub(crate) fn finish(mut self) -> Result<u64, ExportError> {
        self.inner.flush()?;
        Ok(self.bytes)
    }
}

impl Write for CountingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.bytes = self
            .bytes
            .saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_names_round_trip_and_default_paths_follow_the_stem() {
        for (f, name) in [
            (Format::Csv, "csv"),
            (Format::Jsonl, "jsonl"),
            (Format::Parquet, "parquet"),
            (Format::Sqlite, "sqlite"),
        ] {
            assert_eq!(f.to_string(), name);
            assert_eq!(name.parse::<Format>().ok(), Some(f));
            assert_eq!(name.to_uppercase().parse::<Format>().ok(), Some(f));
            assert_eq!(f.extension(), name);
            assert_eq!(
                f.default_out(Path::new("dir/F3XA_2011821.fec")),
                PathBuf::from(format!("F3XA_2011821.{name}"))
            );
        }
        assert!("xlsx".parse::<Format>().is_err());
        assert_eq!(
            Format::Csv.default_out(Path::new("..")),
            PathBuf::from("export.csv")
        );
        assert!(Format::Csv.writes_directory());
        assert!(!Format::Sqlite.writes_directory());
    }

    #[test]
    fn filing_id_from_path_uses_the_last_long_digit_run() {
        let cases = [
            ("F24N_2011823.fec", Some(2011823)),
            ("F3XA_27789_v3.fec", Some(27789)),
            ("F6N_150000_v5.1.fec", Some(150000)),
            ("2010101.fec", Some(2010101)),
            ("/some/dir/1234567.fec", Some(1234567)),
            ("F3XA_v8.5.fec", None),
            ("F3X_123.fec", None),
            ("no_digits.fec", None),
            ("", None),
        ];
        for (path, expected) in cases {
            assert_eq!(filing_id_from_path(Path::new(path)), expected, "{path}");
        }
    }

    #[test]
    fn resolve_table_accepts_names_and_upper_case_tokens_only() {
        assert_eq!(resolve_table("SchA").ok(), Some(Table::SchA));
        assert_eq!(resolve_table("scha").ok(), Some(Table::SchA));
        assert_eq!(resolve_table(" F3X ").ok(), Some(Table::F3X));
        assert_eq!(resolve_table("text").ok(), Some(Table::Text));
        assert_eq!(resolve_table("SA").ok(), Some(Table::SchA));
        assert_eq!(resolve_table("SB21B").ok(), Some(Table::SchB));
        assert_eq!(resolve_table("SC/10").ok(), Some(Table::SchC));
        assert_eq!(resolve_table("F3XN").ok(), Some(Table::F3X));
        // A mistyped table name must not fall through to prefix dispatch.
        for bad in ["ScheA", "sa", "sb21b", "ZZZ", "", "Sch A"] {
            assert!(
                matches!(resolve_table(bad), Err(ExportError::UnknownTable(t)) if t == bad),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn include_filing_id_from_fails_without_digits() {
        let ok = ExportOptions::new(Format::Csv)
            .include_filing_id_from(Path::new("F3XA_2011821.fec"))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(ok.include_filing_id, Some(2011821));
        let err = ExportOptions::new(Format::Csv)
            .include_filing_id_from(Path::new("filing.fec"))
            .err();
        assert!(
            matches!(&err, Some(ExportError::FilingIdNotInFilename { path }) if path == Path::new("filing.fec")),
            "{err:?}"
        );
    }

    #[test]
    fn columns_start_with_filing_id_then_line_no_then_fields() {
        let layout = Table::SchA
            .layout(crate::parser::SpecVersion::electronic(8, 5))
            .unwrap_or_else(|| panic!("SchA has an 8.5 layout"));
        let with = Columns {
            filing_id: Some(1),
            layout,
        };
        let names: Vec<_> = with.names().collect();
        assert_eq!(&names[..3], &["filing_id", "line_no", "form_type"]);
        assert_eq!(names.len(), layout.fields.len() + 2);
        let without = Columns {
            filing_id: None,
            layout,
        };
        let names: Vec<_> = without.names().collect();
        assert_eq!(&names[..2], &["line_no", "form_type"]);
        assert_eq!(names.len(), layout.fields.len() + 1);
    }

    #[test]
    fn filing_id_out_of_range_is_an_error() {
        const HDR: &str = "HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF3XN\u{1c}C00123456\n";
        let reader = FilingReader::new(HDR.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
        let opts = ExportOptions::new(Format::Csv).include_filing_id(u64::MAX);
        let dir = std::env::temp_dir().join(format!(
            "hardmoney-export-unit-{}-{}",
            std::process::id(),
            line!()
        ));
        let err = export_reader(reader, &dir, &opts).err();
        assert!(
            matches!(err, Some(ExportError::FilingIdOutOfRange(id)) if id == u64::MAX),
            "{err:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
