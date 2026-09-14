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
//! Two on-the-wire quirks are handled here that pyfec never implemented at
//! all (confirmed by grepping the upstream source -- zero references):
//!
//! * The `[BEGINTEXT]` / `[ENDTEXT]` free-text block convention, used by
//!   Form 99 (and documented in the FEC's own `FecFileManual`, validation
//!   errors #43-44) to carry a multi-line, non-delimited text block. Real
//!   samples confirm the `text` column of the F99 record itself is left
//!   blank and the actual content lives in this block instead, so we
//!   splice the slurped text back into the `text` field of whichever
//!   record precedes the block.
//! * `TEXT` records (a normal delimited line type, distinct from
//!   `[BEGINTEXT]`) that carry a `back_reference_tran_id_number` pointing
//!   at an earlier transaction -- these parse through the ordinary
//!   dispatch path (`^TEXT` -> the `TEXT` format table) and need no special
//!   handling, but are worth calling out since they're easy to confuse
//!   with the bracketed block.

use once_cell::sync::Lazy;
use regex::Regex;

use crate::parser::error::{FecError, Result};
use crate::parser::form;
use crate::parser::header::{self, HeaderMap};
use crate::parser::utils::{clean_entry, utf8_clean};

/// FEC electronic filings (spec version 6.0 onward) delimit fields with
/// ASCII 28 (File Separator). Versions before that used a comma, requiring
/// full CSV quoting rules -- see [`Filing::parse`].
const NEW_DELIMITER: char = '\u{1c}';

/// Port of pyfec's `re.search('^FEC\s*-\s*(\d+)', report_id)`, used to
/// recover the original filing number an amendment refers to.
static AMENDS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^FEC\s*-\s*(\d+)").unwrap());

/// One parsed detail/schedule/summary line from the body of a filing, e.g.
/// a Schedule A contribution, a Schedule B disbursement, or a Schedule E
/// independent expenditure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLine {
    /// The raw form-type string exactly as it appeared in column 0 of the
    /// line before dispatch, e.g. `"SA11AI"` or `"SC/10"`.
    pub raw_form_type: String,
    /// Which format table ([`form::table_name_for_form_type`]'s result)
    /// this line was parsed with, e.g. `"SchA"`, `"SchB"`, `"F3X"`.
    pub table: &'static str,
    /// Canonical field name -> cleaned value, per the fec-csv-sources
    /// column-position table for this filing's spec version.
    pub fields: indexmap::IndexMap<String, String>,
}

impl ParsedLine {
    /// Convenience accessor equivalent to `self.fields.get(field)`.
    pub fn get(&self, field: &str) -> Option<&str> {
        self.fields.get(field).map(|s| s.as_str())
    }
}

/// A fully parsed FEC electronic filing (`.fec` file).
#[derive(Debug, Clone)]
pub struct Filing {
    /// The parsed `HDR` record (software name/version, report id, etc.).
    pub headers: HeaderMap,
    /// The filing's FEC spec version, e.g. `"8.5"` -- drives which
    /// column-position bucket every line below is parsed with.
    pub version: String,
    /// The top-level form type exactly as filed, e.g. `"F3XA"`.
    pub raw_form_type: String,
    /// `raw_form_type` with any trailing amendment/new/termination
    /// designator stripped, e.g. `"F3X"`. Port of pyfec's
    /// `Filing.get_form_type`.
    pub base_form_type: String,
    /// True if `raw_form_type` designates an amendment (i.e. pyfec's
    /// `form_last_char == 'A'` check).
    pub is_amendment: bool,
    /// The filing number this filing amends, recovered from the header's
    /// `report_id` (e.g. `"FEC-1234567"` -> `"1234567"`). Always `None`
    /// when `is_amendment` is false.
    pub amends_filing: Option<String>,
    /// The parsed top-level summary/cover line (row 2 of the file). For a
    /// Form 99, this is also where any `[BEGINTEXT]` block's content ends
    /// up (spliced into the `text` field).
    pub summary: indexmap::IndexMap<String, String>,
    /// Every subsequent body line (schedules, sub-forms, `TEXT` records),
    /// in file order.
    pub lines: Vec<ParsedLine>,
}

impl Filing {
    /// Parses a complete filing from its raw text content (the full
    /// contents of a `.fec` file, already decoded to UTF-8 -- see
    /// [`Filing::parse_bytes`] if you have raw bytes that might be
    /// Windows-1252 instead).
    pub fn parse(content: &str) -> Result<Filing> {
        if content.starts_with("/*") {
            return Err(FecError::DeprecatedHeaderFormat);
        }

        let first_line = content.lines().next().unwrap_or("");
        if first_line.contains(NEW_DELIMITER) {
            Self::parse_new_delimited(content)
        } else {
            Self::parse_old_delimited(content)
        }
    }

    /// Parses a complete filing from raw bytes, decoding as UTF-8 first and
    /// falling back to Windows-1252 (the encoding older FECFile-produced
    /// filings sometimes use for filer-entered free text) if that fails.
    pub fn parse_bytes(bytes: &[u8]) -> Result<Filing> {
        let content = match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => {
                let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
                decoded.into_owned()
            }
        };
        Self::parse(&content)
    }

    /// Downloads and parses a filing directly from the FEC's document
    /// store, given its numeric filing id (the same id used in FEC.gov
    /// filing URLs). Requires the `fetch` feature.
    ///
    /// Note the FEC serves this endpoint over plain HTTP but 301-redirects
    /// to HTTPS; `ureq`'s default agent follows redirects automatically.
    #[cfg(feature = "fetch")]
    pub fn fetch(filing_id: u64) -> Result<Filing> {
        let url = format!("http://docquery.fec.gov/dcdev/posted/{filing_id}.fec");
        let response = ureq::get(&url).call()?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut response.into_body().into_reader(), &mut bytes)?;
        Self::parse_bytes(&bytes)
    }

    /// Whether this filing's base form type is one this crate knows how to
    /// interpret end-to-end (mirrors pyfec's `Form.is_allowed_form`).
    pub fn is_allowed(&self) -> bool {
        form::is_allowed_top_level_form(&self.base_form_type)
    }

    fn parse_new_delimited(content: &str) -> Result<Filing> {
        let mut lines = content.lines();

        let header_raw = lines.next().ok_or(FecError::MissingFormLine)?;
        let summary_raw = lines.next().ok_or(FecError::MissingFormLine)?;
        let header_fields = split_new_delimited(header_raw);
        let summary_fields = split_new_delimited(summary_raw);

        let mut parsed = Self::parse_headers_and_summary(&header_fields, &summary_fields)?;

        let mut body_lines: Vec<ParsedLine> = Vec::new();
        // 0 means "attach a following [BEGINTEXT] block to the summary
        // line"; N means "attach it to body_lines[N - 1]" instead.
        let mut last_target: usize = 0;

        while let Some(raw) = lines.next() {
            if raw.trim().is_empty() {
                continue;
            }

            if raw.trim().eq_ignore_ascii_case("[BEGINTEXT]") {
                let mut collected: Vec<String> = Vec::new();
                for text_raw in lines.by_ref() {
                    if text_raw.trim().eq_ignore_ascii_case("[ENDTEXT]") {
                        break;
                    }
                    collected.push(utf8_clean(text_raw));
                }
                let text = collected.join("\n");
                attach_free_text(&mut parsed.summary, &mut body_lines, last_target, text);
                continue;
            }

            let fields = split_new_delimited(raw);
            if fields.iter().all(|f| f.is_empty()) {
                continue;
            }

            if let Some(parsed_line) = parse_body_line(&fields, &parsed.version)? {
                body_lines.push(parsed_line);
                last_target = body_lines.len();
            }
        }

        parsed.lines = body_lines;
        Ok(parsed)
    }

    fn parse_old_delimited(content: &str) -> Result<Filing> {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .from_reader(content.as_bytes());
        let mut records = reader.records();

        let header_record = records.next().transpose()?.ok_or(FecError::MissingFormLine)?;
        let summary_record = records.next().transpose()?.ok_or(FecError::MissingFormLine)?;
        let header_fields: Vec<String> = header_record.iter().map(utf8_clean).collect();
        let summary_fields: Vec<String> = summary_record.iter().map(utf8_clean).collect();

        let mut parsed = Self::parse_headers_and_summary(&header_fields, &summary_fields)?;

        // Pre-8.3 filings (the only ones using the comma delimiter) predate
        // the [BEGINTEXT]/[ENDTEXT] convention, so no slurp-mode handling
        // is needed here.
        let mut body_lines: Vec<ParsedLine> = Vec::new();
        for record in records {
            let record = record?;
            let fields: Vec<String> = record.iter().map(utf8_clean).collect();
            if fields.iter().all(|f| f.trim().is_empty()) {
                continue;
            }
            if let Some(parsed_line) = parse_body_line(&fields, &parsed.version)? {
                body_lines.push(parsed_line);
            }
        }

        parsed.lines = body_lines;
        Ok(parsed)
    }

    /// Shared header + summary-line parsing, common to both delimiter
    /// styles. Returns a `Filing` with an empty `lines` vec -- callers fill
    /// that in afterward.
    fn parse_headers_and_summary(header_fields: &[String], summary_fields: &[String]) -> Result<Filing> {
        let headers = header::parse(header_fields, false)?;
        let version = clean_entry(headers.get("fec_version").map(|s| s.as_str()).unwrap_or(""));

        let raw_form_type = clean_entry(summary_fields.first().map(|s| s.as_str()).unwrap_or(""));
        if raw_form_type.is_empty() {
            return Err(FecError::MissingFormLine);
        }
        let base_form_type = strip_ant_suffix(&raw_form_type);

        // Amendment discovery: port of pyfec's `parse_headers`. A form
        // ending in 'A' is an amendment; the filing it amends is embedded
        // in the header's report_id as "FEC-<original filing number>".
        let is_amendment = raw_form_type.ends_with('A');
        let amends_filing = if is_amendment {
            let report_id = headers.get("report_id").map(|s| s.as_str()).unwrap_or("");
            let captures = AMENDS_RE.captures(report_id).ok_or_else(|| FecError::AmendmentOriginalNotFound {
                filing_number: String::new(),
                report_id: report_id.to_string(),
            })?;
            Some(captures.get(1).unwrap().as_str().to_string())
        } else {
            None
        };

        let summary_parser = form::line_parser_for_form_type(&raw_form_type)?;
        let summary = summary_parser.parse_line(summary_fields, &version)?;

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

/// Splits one already-utf8-decoded raw line on the ASCII-28 delimiter,
/// applying `utf8_clean` to each field -- port of pyfec's
/// `get_next_fields` for the new-delimiter path (`utf8_clean(i) for i in
/// nextline.split(new_delimiter)`).
fn split_new_delimited(raw: &str) -> Vec<String> {
    raw.trim_end_matches('\r').split(NEW_DELIMITER).map(utf8_clean).collect()
}

/// Dispatches one non-blank body line to its format table and parses it.
/// Returns `Ok(None)` for a line with an empty form-type column (should not
/// occur in well-formed filings, but mirrors pyfec's tolerant blank-line
/// skipping rather than hard-failing on it).
fn parse_body_line(fields: &[String], version: &str) -> Result<Option<ParsedLine>> {
    let form_type = clean_entry(&fields[0]);
    if form_type.is_empty() {
        return Ok(None);
    }
    let table = form::table_name_for_form_type(&form_type);
    let parser = form::line_parser_for_form_type(&form_type)?;
    let parsed = parser.parse_line(fields, version)?;
    Ok(Some(ParsedLine {
        raw_form_type: form_type,
        table: table.expect("line_parser_for_form_type only succeeds when table_name_for_form_type does"),
        fields: parsed,
    }))
}

/// Splices a slurped `[BEGINTEXT]...[ENDTEXT]` block into whichever record
/// precedes it: the top-level summary line if no body lines have been seen
/// yet (the common case -- Form 99's own free text), or the most recent
/// body line otherwise.
fn attach_free_text(
    summary: &mut indexmap::IndexMap<String, String>,
    body_lines: &mut [ParsedLine],
    last_target: usize,
    text: String,
) {
    if last_target == 0 {
        summary.insert("text".to_string(), text);
    } else if let Some(line) = body_lines.get_mut(last_target - 1) {
        line.fields.insert("text".to_string(), text);
    }
}

/// Port of pyfec's `Filing.get_form_type`: strips a trailing amendment/new/
/// termination designator by finding the *first* occurrence of `A`, `N`,
/// `T` (or a literal `|`, per the Python character-class-vs-alternation
/// quirk noted in `form.rs`) and returning everything before it. Returns
/// the input unchanged if none of those characters appear at all (e.g.
/// `"F99"`, `"F5"`).
fn strip_ant_suffix(raw_form_type: &str) -> String {
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
        assert!(!filing.is_amendment);
        assert!(filing.amends_filing.is_none());
        assert!(filing.is_allowed());
    }

    #[test]
    fn dispatches_body_lines() {
        let filing = Filing::parse(&new_delim_sample()).unwrap();
        assert_eq!(filing.lines.len(), 1);
        assert_eq!(filing.lines[0].table, "SchA");
        assert_eq!(filing.lines[0].raw_form_type, "SA11AI");
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
            filing.summary.get("text").map(|s| s.as_str()),
            Some("This is the free text explanation.\nIt spans two lines.")
        );
        // The free-text content must not be uppercased like ordinary
        // delimited fields (clean_entry would otherwise mangle a filer's
        // prose).
        assert!(filing.summary.get("text").unwrap().contains("free text"));
    }

    #[test]
    fn old_delimiter_comma_filing_parses() {
        // Old-style (pre-6.0) electronic headers are comma-delimited and
        // carry an extra `ef_type` column before the version, per
        // `header::OLD_EHEADERS`.
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
    fn strip_ant_suffix_matches_pyfec_semantics() {
        assert_eq!(strip_ant_suffix("F3XN"), "F3X");
        assert_eq!(strip_ant_suffix("F3A"), "F3");
        assert_eq!(strip_ant_suffix("F24N"), "F24");
        assert_eq!(strip_ant_suffix("F99"), "F99");
        assert_eq!(strip_ant_suffix("F5"), "F5");
        assert_eq!(strip_ant_suffix("F13N"), "F13");
    }
}
