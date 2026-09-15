//! Whole-filing parsing: header, top-level summary line, and every detail
//! ("body") line in between, dispatched to the right format table via
//! [`crate::parser::form`].
//!
//! This intentionally goes further than nyt-pyfec's `filing.py`, which only
//! extracts the header + summary-line financial totals and leaves
//! schedule-level parsing (Schedule A contributions, Schedule B
//! disbursements, etc.) to external code that calls `get_body_row` in a
//! loop. Real consumers of FEC data (an ingestion pipeline, an API, ad-hoc
//! analysis) almost always want every line parsed up front, so
//! [`Filing::parse`] returns them all in [`Filing::lines`].
//!
//! # Strict vs. lenient parsing
//!
//! [`Filing::parse`] / [`Filing::parse_bytes`] are **strict**: the first
//! body line that cannot be parsed (unknown form-type token, or a spec
//! version with no column layout for that table) fails the whole filing.
//! That is the right default for a library -- silently dropping lines is
//! how data goes missing -- but real-world ingestion often wants "parse what
//! you can and tell me what you skipped". [`Filing::parse_with`] takes
//! [`ParseOptions`] and returns a [`Lenient<Filing>`], which cannot be
//! opened without also receiving (or explicitly discarding) the list of
//! [`SkippedLine`]s.
//!
//! # Free-text blocks
//!
//! Two on-the-wire quirks are handled here that pyfec never implemented:
//!
//! * The `[BEGINTEXT]` / `[ENDTEXT]` free-text block convention, used by
//!   Form 99 (and documented in the FEC's own `FecFileManual`, validation
//!   errors #43-44) to carry a multi-line, non-delimited text block. Real
//!   samples confirm the `text` column of the F99 record itself is left
//!   blank and the actual content lives in this block instead, so we
//!   splice the slurped text back into the `text` field of whichever
//!   record precedes the block. An unterminated block is an error.
//! * `TEXT` records (a normal delimited line type, distinct from
//!   `[BEGINTEXT]`) that carry a `back_reference_tran_id_number` pointing
//!   at an earlier transaction -- these parse through the ordinary
//!   dispatch path and need no special handling.

use std::sync::LazyLock;

use regex::Regex;

use crate::parser::error::{FecError, Result};
use crate::parser::form;
use crate::parser::format_data::Table;
use crate::parser::header::{self, HeaderMap};
use crate::parser::typed::TypedView;
use crate::parser::utils::{clean_entry, utf8_clean};

/// FEC electronic filings (spec version 6.0 onward) delimit fields with
/// ASCII 28 (File Separator). Versions before that used a comma, requiring
/// full CSV quoting rules -- see [`Filing::parse`].
pub const NEW_DELIMITER: char = '\u{1c}';

/// Port of pyfec's `re.search('^FEC\s*-\s*(\d+)', report_id)`, used to
/// recover the original filing number an amendment refers to.
static AMENDS_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^FEC\s*-\s*(\d+)").ok());

/// One parsed detail/schedule/summary line from the body of a filing, e.g.
/// a Schedule A contribution, a Schedule B disbursement, or a Schedule E
/// independent expenditure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ParsedLine {
    /// The raw form-type string exactly as it appeared in column 0 of the
    /// line before dispatch, e.g. `"SA11AI"` or `"SC/10"`.
    pub raw_form_type: String,
    /// Which format table this line was parsed with, e.g. [`Table::SchA`].
    pub table: Table,
    /// 1-based physical line number in the source file.
    pub line_no: u64,
    /// Canonical field name -> cleaned value, per the fec-csv-sources
    /// column-position table for this filing's spec version.
    pub fields: indexmap::IndexMap<String, String>,
}

impl ParsedLine {
    /// Constructs a line directly. Mostly useful in tests; parsing code
    /// produces these via [`Filing::parse`].
    pub fn new(
        raw_form_type: impl Into<String>,
        table: Table,
        line_no: u64,
        fields: indexmap::IndexMap<String, String>,
    ) -> Self {
        Self {
            raw_form_type: raw_form_type.into(),
            table,
            line_no,
            fields,
        }
    }

    /// Convenience accessor equivalent to `self.fields.get(field)`.
    pub fn get(&self, field: &str) -> Option<&str> {
        self.fields.get(field).map(|s| s.as_str())
    }

    /// Converts this line into a typed view, refusing if the line belongs
    /// to a different table than `V` expects.
    ///
    /// ```no_run
    /// # use hardmoney::{Filing, ScheduleA};
    /// # let filing = Filing::parse("").unwrap();
    /// for line in &filing.lines {
    ///     if let Ok(a) = line.view::<ScheduleA>() {
    ///         println!("{:?} {:?}", a.contributor_name, a.contribution_amount);
    ///     }
    /// }
    /// ```
    pub fn view<V: TypedView>(&self) -> std::result::Result<V, crate::parser::TypedViewError> {
        V::from_line(self)
    }
}

/// A fully parsed FEC electronic filing (`.fec` file).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Filing {
    /// The parsed `HDR` record (software name/version, report id, etc.).
    pub headers: HeaderMap,
    /// The filing's FEC spec version, e.g. `"8.5"` -- drives which
    /// column-position bucket every line below is parsed with.
    pub version: String,
    /// The top-level form type exactly as filed, e.g. `"F3XA"`.
    pub raw_form_type: String,
    /// `raw_form_type` with any trailing amendment/new/termination
    /// designator stripped, e.g. `"F3X"`.
    pub base_form_type: String,
    /// True if `raw_form_type` designates an amendment.
    pub is_amendment: bool,
    /// The filing number this filing amends, recovered from the header's
    /// `report_id` (e.g. `"FEC-1234567"` -> `"1234567"`). Always `None`
    /// when `is_amendment` is false.
    pub amends_filing: Option<String>,
    /// The parsed top-level summary/cover line (row 2 of the file). For a
    /// Form 99, this is also where any `[BEGINTEXT]` block's content ends
    /// up (spliced into the `text` field).
    pub summary: ParsedLine,
    /// Every subsequent body line (schedules, sub-forms, `TEXT` records),
    /// in file order.
    pub lines: Vec<ParsedLine>,
}

/// What to do when a body line cannot be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum OnUnparseableLine {
    /// Fail the whole filing with the first error (the default).
    #[default]
    Fail,
    /// Record the line in [`Lenient::skipped`] and continue.
    Skip,
}

/// Options for [`Filing::parse_with`]. Construct with [`ParseOptions::default`]
/// (identical to [`Filing::parse`]) or [`ParseOptions::lenient`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ParseOptions {
    /// A body line whose form-type token matches no format table.
    pub on_unknown_line: OnUnparseableLine,
    /// A body line whose table has no layout for the filing's spec version.
    pub on_missing_version: OnUnparseableLine,
}

impl ParseOptions {
    /// Strict: any unparseable body line fails the filing.
    pub const STRICT: ParseOptions = ParseOptions {
        on_unknown_line: OnUnparseableLine::Fail,
        on_missing_version: OnUnparseableLine::Fail,
    };

    /// Lenient: unparseable body lines are skipped and reported.
    pub const LENIENT: ParseOptions = ParseOptions {
        on_unknown_line: OnUnparseableLine::Skip,
        on_missing_version: OnUnparseableLine::Skip,
    };

    /// Same as [`ParseOptions::LENIENT`].
    pub fn lenient() -> Self {
        Self::LENIENT
    }

    fn is_strict(&self) -> bool {
        *self == Self::STRICT
    }
}

/// Why a line was skipped under lenient parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SkipReason {
    /// The form-type token matched no dispatch pattern.
    UnknownFormType,
    /// The table exists but has no layout for this spec version.
    NoLayoutForVersion,
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkipReason::UnknownFormType => f.write_str("unknown form type"),
            SkipReason::NoLayoutForVersion => f.write_str("no column layout for this spec version"),
        }
    }
}

/// A body line that could not be parsed and was skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct SkippedLine {
    /// 1-based physical line number in the source file.
    pub line_no: u64,
    /// The cleaned form-type token (column 0) of the skipped line.
    pub raw_form_type: String,
    pub reason: SkipReason,
}

impl std::fmt::Display for SkippedLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "line {}: '{}' skipped ({})",
            self.line_no, self.raw_form_type, self.reason
        )
    }
}

/// The result of a lenient parse: a value plus the lines that were skipped
/// to produce it.
///
/// This type is deliberately opaque. There is no `Deref` to `T` and no
/// public fields: the only ways out are [`into_parts`](Self::into_parts)
/// (which hands you the skipped lines alongside the value) and
/// [`into_strict`](Self::into_strict) (which turns any skips into an
/// error). You cannot accidentally forget that lines were dropped.
#[must_use = "a Lenient<T> holds skipped-line information; call into_parts() or into_strict()"]
#[derive(Debug)]
pub struct Lenient<T> {
    value: T,
    skipped: Vec<SkippedLine>,
    first_error: Option<Box<FecError>>,
}

impl<T> Lenient<T> {
    /// Splits into the parsed value and the lines that were skipped.
    pub fn into_parts(self) -> (T, Vec<SkippedLine>) {
        (self.value, self.skipped)
    }

    /// Returns the value only if nothing was skipped; otherwise
    /// [`FecError::LinesSkipped`] carrying the count and first error.
    pub fn into_strict(self) -> Result<T> {
        match (self.skipped.len(), self.first_error) {
            (0, _) => Ok(self.value),
            (count, Some(first)) => Err(FecError::LinesSkipped { count, first }),
            // Unreachable in practice: a skip always records its error.
            (count, None) => Err(FecError::LinesSkipped {
                count,
                first: Box::new(FecError::MissingFormLine),
            }),
        }
    }

    /// The skipped lines, without consuming the wrapper.
    pub fn skipped(&self) -> &[SkippedLine] {
        &self.skipped
    }

    /// Borrow the value. Prefer [`into_parts`](Self::into_parts) -- this is
    /// for inspection, and does not discharge the `#[must_use]`.
    pub fn value(&self) -> &T {
        &self.value
    }
}

impl Filing {
    /// Parses a complete filing from its raw text content, strictly.
    ///
    /// Equivalent to `Filing::parse_with(content, &ParseOptions::STRICT)`
    /// followed by `.into_strict()`.
    pub fn parse(content: &str) -> Result<Filing> {
        Self::parse_with(content, &ParseOptions::STRICT)?.into_strict()
    }

    /// Parses a complete filing from raw bytes, decoding as UTF-8 first and
    /// falling back to Windows-1252 (the encoding older FECFile-produced
    /// filings sometimes use for filer-entered free text) if that fails.
    pub fn parse_bytes(bytes: &[u8]) -> Result<Filing> {
        Self::parse_bytes_with(bytes, &ParseOptions::STRICT)?.into_strict()
    }

    /// Like [`Filing::parse_bytes`] with explicit [`ParseOptions`].
    pub fn parse_bytes_with(bytes: &[u8], options: &ParseOptions) -> Result<Lenient<Filing>> {
        Self::parse_with(&decode(bytes), options)
    }

    /// Parses a complete filing with explicit [`ParseOptions`].
    ///
    /// With [`ParseOptions::STRICT`] the returned [`Lenient`] never has
    /// skipped lines. With [`ParseOptions::LENIENT`], unparseable body
    /// lines are recorded instead of failing the parse. Errors that are not
    /// about a single body line (bad header, missing summary line,
    /// unterminated `[BEGINTEXT]`) always fail.
    pub fn parse_with(content: &str, options: &ParseOptions) -> Result<Lenient<Filing>> {
        if content.starts_with("/*") {
            return Err(FecError::DeprecatedHeaderFormat);
        }

        let first_line = content.lines().next().unwrap_or("");
        if first_line.contains(NEW_DELIMITER) {
            Self::parse_new_delimited(content, options)
        } else {
            Self::parse_old_delimited(content, options)
        }
    }

    /// Downloads and parses a filing directly from the FEC's document
    /// store, given its numeric filing id (the same id used in FEC.gov
    /// filing URLs). Requires the `fetch` feature.
    #[cfg(feature = "fetch")]
    pub fn fetch(filing_id: u64) -> Result<Filing> {
        Self::parse_bytes(&Self::fetch_bytes(filing_id)?)
    }

    /// Downloads a filing's raw bytes from the FEC's document store.
    ///
    /// Note the FEC serves this endpoint over plain HTTP but 301-redirects
    /// to HTTPS; `ureq`'s default agent follows redirects automatically.
    #[cfg(feature = "fetch")]
    pub fn fetch_bytes(filing_id: u64) -> Result<Vec<u8>> {
        let url = format!("http://docquery.fec.gov/dcdev/posted/{filing_id}.fec");
        let response = ureq::get(&url).call()?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut response.into_body().into_reader(), &mut bytes)?;
        Ok(bytes)
    }

    /// Whether this filing's base form type is one this crate knows how to
    /// interpret end-to-end.
    pub fn is_allowed(&self) -> bool {
        form::is_allowed_top_level_form(&self.base_form_type)
    }

    /// Iterates the body lines that belong to `table`.
    pub fn lines_for(&self, table: Table) -> impl Iterator<Item = &ParsedLine> + '_ {
        self.lines.iter().filter(move |l| l.table == table)
    }

    /// Iterates the body lines of `V`'s table as typed views, skipping any
    /// that fail conversion (which, for a line of the right table, only
    /// happens if a genuinely required field is blank).
    ///
    /// ```no_run
    /// # use hardmoney::{Filing, ScheduleA};
    /// # let filing = Filing::parse("").unwrap();
    /// let total: rust_decimal::Decimal = filing
    ///     .views::<ScheduleA>()
    ///     .filter_map(|a| a.contribution_amount)
    ///     .sum();
    /// ```
    pub fn views<V: TypedView>(&self) -> impl Iterator<Item = V> + '_ {
        self.lines_for(V::TABLE)
            .filter_map(|l| V::from_line(l).ok())
    }

    fn parse_new_delimited(content: &str, options: &ParseOptions) -> Result<Lenient<Filing>> {
        // (line_no, raw text) pairs; line numbers are 1-based.
        let mut lines = content.lines().enumerate().map(|(i, l)| (i as u64 + 1, l));

        let (_, header_raw) = lines.next().ok_or(FecError::MissingFormLine)?;
        let (_, summary_raw) = lines.next().ok_or(FecError::MissingFormLine)?;
        let header_fields = split_new_delimited(header_raw);
        let summary_fields = split_new_delimited(summary_raw);

        let mut parsed = Self::parse_headers_and_summary(&header_fields, &summary_fields)?;
        let mut acc = BodyAccumulator::new(options);

        while let Some((line_no, raw)) = lines.next() {
            if raw.trim().is_empty() {
                continue;
            }

            if raw.trim().eq_ignore_ascii_case("[BEGINTEXT]") {
                let mut collected: Vec<String> = Vec::new();
                let mut terminated = false;
                for (_, text_raw) in lines.by_ref() {
                    if text_raw.trim().eq_ignore_ascii_case("[ENDTEXT]") {
                        terminated = true;
                        break;
                    }
                    collected.push(utf8_clean(text_raw));
                }
                if !terminated {
                    return Err(FecError::UnterminatedTextBlock { line_no });
                }
                acc.attach_free_text(&mut parsed.summary, collected.join("\n"));
                continue;
            }

            let fields = split_new_delimited(raw);
            if fields.iter().all(|f| f.is_empty()) {
                continue;
            }
            acc.push_body_line(&fields, &parsed.version, line_no)?;
        }

        Ok(acc.finish(parsed))
    }

    fn parse_old_delimited(content: &str, options: &ParseOptions) -> Result<Lenient<Filing>> {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .from_reader(content.as_bytes());
        let mut records = reader.records();

        let header_record = records
            .next()
            .transpose()?
            .ok_or(FecError::MissingFormLine)?;
        let summary_record = records
            .next()
            .transpose()?
            .ok_or(FecError::MissingFormLine)?;
        let header_fields: Vec<String> = header_record.iter().map(utf8_clean).collect();
        let summary_fields: Vec<String> = summary_record.iter().map(utf8_clean).collect();

        let parsed = Self::parse_headers_and_summary(&header_fields, &summary_fields)?;
        let mut acc = BodyAccumulator::new(options);

        // Pre-6.0 filings (the only ones using the comma delimiter) predate
        // the [BEGINTEXT]/[ENDTEXT] convention, so no slurp-mode handling
        // is needed here.
        for record in records {
            let record = record?;
            // csv reports the 1-based line of the record's first byte.
            let line_no = record.position().map(|p| p.line()).unwrap_or(0);
            let fields: Vec<String> = record.iter().map(utf8_clean).collect();
            if fields.iter().all(|f| f.trim().is_empty()) {
                continue;
            }
            acc.push_body_line(&fields, &parsed.version, line_no)?;
        }

        Ok(acc.finish(parsed))
    }

    /// Shared header + summary-line parsing, common to both delimiter
    /// styles. Returns a `Filing` with an empty `lines` vec -- callers fill
    /// that in afterward.
    fn parse_headers_and_summary(
        header_fields: &[String],
        summary_fields: &[String],
    ) -> Result<Filing> {
        let headers = header::parse(header_fields, false)?;
        let version = clean_entry(headers.get("fec_version").map(|s| s.as_str()).unwrap_or(""));

        let raw_form_type = clean_entry(summary_fields.first().map(|s| s.as_str()).unwrap_or(""));
        if raw_form_type.is_empty() {
            return Err(FecError::MissingFormLine);
        }
        let base_form_type = strip_ant_suffix(&raw_form_type);

        // Amendment discovery: a form ending in 'A' is an amendment; the
        // filing it amends is embedded in the header's report_id as
        // "FEC-<original filing number>".
        let is_amendment = raw_form_type.ends_with('A');
        let amends_filing = if is_amendment {
            let report_id = headers.get("report_id").map(|s| s.as_str()).unwrap_or("");
            let captured = AMENDS_RE
                .as_ref()
                .and_then(|re| re.captures(report_id))
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_string());
            Some(captured.ok_or_else(|| FecError::AmendmentOriginalNotFound {
                report_id: report_id.to_string(),
            })?)
        } else {
            None
        };

        let (table, summary_parser) = form::dispatch(&raw_form_type).map_err(|e| e.at_line(2))?;
        let summary_fields_map = summary_parser
            .parse_line(summary_fields, &version)
            .map_err(|e| e.at_line(2))?;
        let summary = ParsedLine::new(raw_form_type.clone(), table, 2, summary_fields_map);

        Ok(Filing {
            headers,
            version,
            raw_form_type,
            base_form_type,
            is_amendment,
            amends_filing,
            summary,
            lines: Vec::new(),
        })
    }
}

/// Collects body lines during a parse, applying [`ParseOptions`].
struct BodyAccumulator<'o> {
    options: &'o ParseOptions,
    lines: Vec<ParsedLine>,
    skipped: Vec<SkippedLine>,
    first_error: Option<Box<FecError>>,
}

impl<'o> BodyAccumulator<'o> {
    fn new(options: &'o ParseOptions) -> Self {
        Self {
            options,
            lines: Vec::new(),
            skipped: Vec::new(),
            first_error: None,
        }
    }

    fn push_body_line(&mut self, fields: &[String], version: &str, line_no: u64) -> Result<()> {
        let form_type = clean_entry(fields.first().map(String::as_str).unwrap_or(""));
        if form_type.is_empty() {
            // Tolerant blank-line skipping, as pyfec did.
            return Ok(());
        }

        let (table, parser) = match form::table_for_form_type(&form_type) {
            Some(table) => (table, form::line_parser(table)?),
            None => {
                let err = FecError::ParserMissing {
                    form_type: form_type.clone(),
                    version: version.to_string(),
                    line_no: Some(line_no),
                };
                return self.skip_or_fail(
                    self.options.on_unknown_line,
                    err,
                    line_no,
                    form_type,
                    SkipReason::UnknownFormType,
                );
            }
        };

        match parser.parse_line(fields, version) {
            Ok(parsed) => {
                self.lines
                    .push(ParsedLine::new(form_type, table, line_no, parsed));
                Ok(())
            }
            Err(e @ FecError::NoMatchingVersionBucket { .. }) => self.skip_or_fail(
                self.options.on_missing_version,
                e.at_line(line_no),
                line_no,
                form_type,
                SkipReason::NoLayoutForVersion,
            ),
            Err(e) => Err(e.at_line(line_no)),
        }
    }

    fn skip_or_fail(
        &mut self,
        policy: OnUnparseableLine,
        err: FecError,
        line_no: u64,
        raw_form_type: String,
        reason: SkipReason,
    ) -> Result<()> {
        match policy {
            OnUnparseableLine::Fail => Err(err),
            OnUnparseableLine::Skip => {
                if self.first_error.is_none() {
                    self.first_error = Some(Box::new(err));
                }
                self.skipped.push(SkippedLine {
                    line_no,
                    raw_form_type,
                    reason,
                });
                Ok(())
            }
        }
    }

    /// Splices a slurped `[BEGINTEXT]...[ENDTEXT]` block into whichever
    /// record precedes it: the most recent body line if any, else the
    /// top-level summary line (the common case -- Form 99's own free text).
    fn attach_free_text(&mut self, summary: &mut ParsedLine, text: String) {
        match self.lines.last_mut() {
            Some(line) => line.fields.insert("text".to_string(), text),
            None => summary.fields.insert("text".to_string(), text),
        };
    }

    fn finish(self, mut filing: Filing) -> Lenient<Filing> {
        debug_assert!(!self.options.is_strict() || self.skipped.is_empty());
        filing.lines = self.lines;
        Lenient {
            value: filing,
            skipped: self.skipped,
            first_error: self.first_error,
        }
    }
}

/// Decodes raw filing bytes: UTF-8 if valid, else Windows-1252.
pub fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
            decoded.into_owned()
        }
    }
}

/// Splits one already-utf8-decoded raw line on the ASCII-28 delimiter,
/// applying `utf8_clean` to each field.
pub fn split_new_delimited(raw: &str) -> Vec<String> {
    raw.trim_end_matches('\r')
        .split(NEW_DELIMITER)
        .map(utf8_clean)
        .collect()
}

/// Strips a trailing amendment/new/termination designator by finding the
/// *first* occurrence of `A`, `N`, `T` (or a literal `|`, per the Python
/// character-class-vs-alternation quirk noted in `form.rs`) and returning
/// everything before it. Returns the input unchanged if none of those
/// characters appear at all (e.g. `"F99"`, `"F5"`).
pub fn strip_ant_suffix(raw_form_type: &str) -> String {
    for (i, ch) in raw_form_type.char_indices() {
        if matches!(ch, 'A' | 'N' | 'T' | '|') {
            return raw_form_type[..i].to_string();
        }
    }
    raw_form_type.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_delim_sample() -> String {
        [
            "HDR\u{1c}FEC\u{1c}8.5\u{1c}FECfile\u{1c}8.5.1.0(f34)\u{1c}\u{1c}",
            "F3XN\u{1c}C00123456\u{1c}COMMITTEE NAME",
            "SA11AI\u{1c}C00123456\u{1c}IND\u{1c}SMITH, JANE",
        ]
        .join("\n")
    }

    #[test]
    fn parses_header_and_summary() {
        let filing = Filing::parse(&new_delim_sample()).unwrap();
        assert_eq!(filing.version, "8.5");
        assert_eq!(filing.raw_form_type, "F3XN");
        assert_eq!(filing.base_form_type, "F3X");
        assert_eq!(filing.summary.table, Table::F3X);
        assert_eq!(filing.summary.line_no, 2);
        assert!(!filing.is_amendment);
        assert!(filing.amends_filing.is_none());
        assert!(filing.is_allowed());
    }

    #[test]
    fn dispatches_body_lines_with_line_numbers() {
        let filing = Filing::parse(&new_delim_sample()).unwrap();
        assert_eq!(filing.lines.len(), 1);
        assert_eq!(filing.lines[0].table, Table::SchA);
        assert_eq!(filing.lines[0].raw_form_type, "SA11AI");
        assert_eq!(filing.lines[0].line_no, 3);
    }

    #[test]
    fn amendment_recovers_original_filing_number() {
        let content = [
            "HDR\u{1c}FEC\u{1c}8.5\u{1c}FECfile\u{1c}8.5.1.0(f34)\u{1c}FEC-1234567\u{1c}1",
            "F3XA\u{1c}C00123456\u{1c}COMMITTEE NAME",
        ]
        .join("\n");
        let filing = Filing::parse(&content).unwrap();
        assert!(filing.is_amendment);
        assert_eq!(filing.amends_filing.as_deref(), Some("1234567"));
    }

    #[test]
    fn amendment_without_report_id_errors() {
        let content = [
            "HDR\u{1c}FEC\u{1c}8.5\u{1c}FECfile\u{1c}8.5.1.0(f34)\u{1c}\u{1c}",
            "F3XA\u{1c}C00123456\u{1c}COMMITTEE NAME",
        ]
        .join("\n");
        assert!(matches!(
            Filing::parse(&content),
            Err(FecError::AmendmentOriginalNotFound { .. })
        ));
    }

    #[test]
    fn begintext_block_attaches_to_preceding_summary_line() {
        let content = [
            "HDR\u{1c}FEC\u{1c}8.5\u{1c}FECfile\u{1c}8.5.1.0(f34)\u{1c}\u{1c}",
            "F99\u{1c}C00944124\u{1c}REVIVE OREGON\u{1c}PO BOX 26141\u{1c}\u{1c}ALEXANDRIA\u{1c}VA\u{1c}22313\u{1c}MARSTON\u{1c}CHRIS\u{1c}\u{1c}\u{1c}\u{1c}20260914\u{1c}MST\u{1c}\u{1c}",
            "[BEGINTEXT]",
            "This is the free text explanation.",
            "It spans two lines.",
            "[ENDTEXT]",
        ]
        .join("\n");
        let filing = Filing::parse(&content).unwrap();
        assert_eq!(
            filing.summary.get("text"),
            Some("This is the free text explanation.\nIt spans two lines.")
        );
    }

    #[test]
    fn unterminated_begintext_is_an_error() {
        let content = [
            "HDR\u{1c}FEC\u{1c}8.5\u{1c}FECfile\u{1c}8.5.1.0(f34)\u{1c}\u{1c}",
            "F99\u{1c}C00944124\u{1c}REVIVE OREGON",
            "[BEGINTEXT]",
            "never closed",
        ]
        .join("\n");
        assert!(matches!(
            Filing::parse(&content),
            Err(FecError::UnterminatedTextBlock { line_no: 3 })
        ));
    }

    #[test]
    fn old_delimiter_comma_filing_parses() {
        let content = "HDR,FEC,5.3,SoftCo,1.0\nF3XN,C00123456,COMMITTEE NAME\n";
        let filing = Filing::parse(content).unwrap();
        assert_eq!(filing.version, "5.3");
        assert_eq!(filing.raw_form_type, "F3XN");
    }

    #[test]
    fn deprecated_header_format_errors() {
        assert!(Filing::parse("/* old format\n").is_err());
    }

    #[test]
    fn empty_and_truncated_inputs_error_not_panic() {
        for input in [
            "",
            "HDR\u{1c}FEC\u{1c}8.5",
            "HDR,FEC,5.3\n",
            "\u{1c}\u{1c}\u{1c}\n\n",
        ] {
            assert!(Filing::parse(input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn strict_parse_fails_on_unknown_body_line_with_line_number() {
        let content = format!("{}\nZZZ\u{1c}junk", new_delim_sample());
        match Filing::parse(&content) {
            Err(FecError::ParserMissing {
                form_type,
                version,
                line_no,
            }) => {
                assert_eq!(form_type, "ZZZ");
                assert_eq!(version, "8.5");
                assert_eq!(line_no, Some(4));
            }
            other => panic!("expected ParserMissing, got {other:?}"),
        }
    }

    #[test]
    fn lenient_parse_records_skipped_lines() {
        let content = format!(
            "{}\nZZZ\u{1c}junk\nSB21B\u{1c}C00123456\u{1c}ORG\u{1c}VENDOR",
            new_delim_sample()
        );
        let (filing, skipped) = Filing::parse_with(&content, &ParseOptions::LENIENT)
            .unwrap()
            .into_parts();
        assert_eq!(filing.lines.len(), 2);
        assert_eq!(filing.lines[1].table, Table::SchB);
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].line_no, 4);
        assert_eq!(skipped[0].raw_form_type, "ZZZ");
        assert_eq!(skipped[0].reason, SkipReason::UnknownFormType);
    }

    #[test]
    fn lenient_into_strict_surfaces_skips_as_error() {
        let content = format!("{}\nZZZ\u{1c}junk", new_delim_sample());
        let lenient = Filing::parse_with(&content, &ParseOptions::LENIENT).unwrap();
        assert_eq!(lenient.skipped().len(), 1);
        match lenient.into_strict() {
            Err(FecError::LinesSkipped { count, first }) => {
                assert_eq!(count, 1);
                assert!(matches!(*first, FecError::ParserMissing { .. }));
            }
            other => panic!("expected LinesSkipped, got {other:?}"),
        }
    }

    #[test]
    fn lenient_parse_with_no_skips_into_strict_is_ok() {
        let filing = Filing::parse_with(&new_delim_sample(), &ParseOptions::LENIENT)
            .unwrap()
            .into_strict()
            .unwrap();
        assert_eq!(filing.lines.len(), 1);
    }

    #[test]
    fn unknown_version_is_skippable() {
        let content = [
            "HDR\u{1c}FEC\u{1c}8.5\u{1c}FECfile\u{1c}8.5.1.0\u{1c}\u{1c}",
            "F3XN\u{1c}C00123456\u{1c}COMMITTEE NAME",
            // Schedule I has no 8.5 layout (the FEC dropped it), so this
            // dispatches fine but has no version bucket.
            "SI\u{1c}C00123456\u{1c}LEVIN",
        ]
        .join("\n");
        assert!(matches!(
            Filing::parse(&content),
            Err(FecError::NoMatchingVersionBucket {
                table: "SchI",
                line_no: Some(3),
                ..
            })
        ));
        let (filing, skipped) = Filing::parse_with(&content, &ParseOptions::LENIENT)
            .unwrap()
            .into_parts();
        assert!(filing.lines.is_empty());
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].reason, SkipReason::NoLayoutForVersion);
    }

    #[test]
    fn lines_for_and_views_filter_by_table() {
        let content = format!(
            "{}\nSB21B\u{1c}C00123456\u{1c}ORG\u{1c}VENDOR",
            new_delim_sample()
        );
        let filing = Filing::parse(&content).unwrap();
        assert_eq!(filing.lines_for(Table::SchA).count(), 1);
        assert_eq!(filing.lines_for(Table::SchB).count(), 1);
        assert_eq!(filing.lines_for(Table::SchE).count(), 0);
        assert_eq!(filing.views::<crate::parser::ScheduleA>().count(), 1);
        assert_eq!(filing.views::<crate::parser::ScheduleB>().count(), 1);
        assert_eq!(filing.views::<crate::parser::ScheduleE>().count(), 0);
    }

    #[test]
    fn strip_ant_suffix_matches_pyfec_semantics() {
        assert_eq!(strip_ant_suffix("F3XN"), "F3X");
        assert_eq!(strip_ant_suffix("F3A"), "F3");
        assert_eq!(strip_ant_suffix("F24N"), "F24");
        assert_eq!(strip_ant_suffix("F99"), "F99");
        assert_eq!(strip_ant_suffix("F5"), "F5");
        assert_eq!(strip_ant_suffix("F13N"), "F13");
        assert_eq!(strip_ant_suffix("F1MN"), "F1M");
        assert_eq!(strip_ant_suffix("F2A"), "F2");
    }
}
