//! Whole-filing parsing: header, cover (summary) line, and every body line
//! in between, dispatched to the right format table via
//! [`crate::parser::form`].
//!
//! Real consumers of FEC data (an ingestion pipeline, an API, ad-hoc
//! analysis) almost always want every line parsed up front, so
//! [`Filing::parse`] returns them all in [`Filing::lines`].
//!
//! # Fidelity
//!
//! Field values are preserved **verbatim** apart from two wire
//! conventions -- surrounding ASCII whitespace and one pair of wrapping
//! double quotes (see [`crate::parser::utils`]); nothing is upper-cased or
//! stripped. Interpretation of codes (form-type tokens, entity types, memo
//! flags) is case-insensitive at the point of use. Every [`ParsedLine`]
//! remembers the [`Layout`] it was parsed with, which is what makes the
//! writer (`Filing::to_fec`) an exact inverse of parsing.
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
//! * The `[BEGINTEXT]` / `[ENDTEXT]` convention, used by Form 99 (and
//!   documented in the FEC's `FecFileManual`, validation errors #43-44),
//!   carries a multi-line, non-delimited text block. Real samples confirm
//!   the `text` column of the F99 record itself is left blank and the
//!   content lives in this block, so the block is spliced into the `text`
//!   field of whichever record precedes it. An unterminated block is an
//!   error.
//! * `TEXT` records (a normal delimited line type, distinct from
//!   `[BEGINTEXT]`) carry their text in-line and parse through ordinary
//!   dispatch.

use std::fmt;

use compact_str::CompactString;

use crate::parser::error::{FecError, Result};
use crate::parser::form;
use crate::parser::header::Header;
use crate::parser::schema::{Layout, SpecVersion, TableMarker, Typed};
use crate::parser::tables::Table;
use crate::parser::typed::{TypedView, TypedViewError};
use crate::parser::utils::{is_memo_code, normalize_field, normalize_form_type};

/// FEC electronic filings (spec version 6.0 onward) delimit fields with
/// ASCII 28 (File Separator). Versions before that used a comma, requiring
/// full CSV quoting rules -- see [`Filing::parse`].
pub const NEW_DELIMITER: char = '\u{1c}';

// ---------------------------------------------------------------------------
// ParsedLine
// ---------------------------------------------------------------------------

/// One parsed record: the cover line, or a schedule / sub-form / `TEXT`
/// line from the body of a filing.
///
/// A line stores its values as a flat slice parallel to the fields of the
/// [`Layout`] that parsed it, so field *names* are shared `&'static str`s
/// rather than per-line allocations, and the line can always be written
/// back at the right columns.
#[derive(Clone)]
pub struct ParsedLine {
    /// The form-type token from column 0, upper-cased (the spec defines
    /// these tokens as case-insensitive and real filings contain e.g.
    /// `SB21b`), e.g. `"SA11AI"` or `"SC/10"`. The `form_type` *field*
    /// keeps the token exactly as filed.
    pub raw_form_type: String,
    /// 1-based physical line number in the source file (0 if synthetic).
    pub line_no: u64,
    layout: &'static Layout,
    values: Box<[CompactString]>,
}

impl ParsedLine {
    /// Parses one already-split record with the layout for `table` at
    /// `version`. Fails only with [`FecError::NoMatchingVersionBucket`] if
    /// no layout covers that version.
    ///
    /// Each field takes the cell at its layout column, normalised (see
    /// [`crate::parser::utils::normalize_field`]); a cell the record does
    /// not carry (short line) is blank, and cells beyond the layout's
    /// columns are ignored. `raw_form_type` is the first cell upper-cased
    /// (blank if there is none); the table is `table`, whatever the token
    /// says -- dispatch happens before this call.
    pub fn from_cells(
        table: Table,
        version: SpecVersion,
        line_no: u64,
        cells: &[&str],
    ) -> Result<Self> {
        let layout = table
            .layout(version)
            .ok_or(FecError::NoMatchingVersionBucket {
                table,
                version,
                line_no: Some(line_no),
            })?;
        Ok(Self::with_layout(layout, line_no, cells))
    }

    fn with_layout(layout: &'static Layout, line_no: u64, cells: &[&str]) -> Self {
        let values: Box<[CompactString]> = layout
            .fields
            .iter()
            .map(|f| {
                cells
                    .get(usize::from(f.column))
                    .map(|c| CompactString::new(normalize_field(c)))
                    .unwrap_or_default()
            })
            .collect();
        let raw_form_type = normalize_form_type(cells.first().copied().unwrap_or(""));
        Self {
            raw_form_type,
            line_no,
            layout,
            values,
        }
    }

    /// Builds a line from `(field, value)` pairs -- for tests, synthetic
    /// fixtures, and editors. Every name must exist in the layout for
    /// `table` at `version` (else [`FecError::UnknownField`]); unspecified
    /// fields are blank. The column-0 token field (`form_type`, or
    /// `rec_type` on a `TEXT` record), if not given, defaults to the
    /// table's name -- which dispatches back to the table for every table
    /// except the schedules, whose tokens carry a line number (`SA11AI`);
    /// give `form_type` explicitly for those.
    pub fn from_pairs<'p>(
        table: Table,
        version: SpecVersion,
        line_no: u64,
        pairs: impl IntoIterator<Item = (&'p str, &'p str)>,
    ) -> Result<Self> {
        let mut line = Self::from_cells(table, version, line_no, &[])?;
        for (name, value) in pairs {
            line.set(name, value)?;
        }
        if line.raw_form_type.is_empty()
            && let Some(token_field) = line.layout.token_field()
        {
            let _ = line.set(token_field, table.as_str());
        }
        Ok(line)
    }

    /// Which format table this line was parsed with.
    #[must_use]
    pub fn table(&self) -> Table {
        self.layout.table
    }

    /// The version-bucket layout this line was parsed with.
    #[must_use]
    pub fn layout(&self) -> &'static Layout {
        self.layout
    }

    /// The value of `field`: `Some("")` if the field exists in this layout
    /// but was blank or beyond the end of the physical line, `None` if the
    /// field does not exist in this layout at all.
    #[must_use]
    pub fn get(&self, field: &str) -> Option<&str> {
        self.layout
            .index_of(field)
            .and_then(|i| self.values.get(i))
            .map(CompactString::as_str)
    }

    /// Like [`get`](Self::get) but `None` for blank values too.
    #[must_use]
    pub fn get_non_empty(&self, field: &str) -> Option<&str> {
        self.get(field).filter(|v| !v.is_empty())
    }

    /// Sets `field` (normalised like a parsed value: trimmed, one pair of
    /// wrapping quotes removed); fails with [`FecError::UnknownField`] if
    /// the layout has no such field. Setting the column-0 token field
    /// (`form_type`, or `rec_type` on a `TEXT` record) updates
    /// `raw_form_type` too, so the line keeps dispatching to the table it
    /// was built with only if the new token still names that table --
    /// [`Filing::from_parts`] checks exactly that. A `TEXT` record's
    /// `form_type` on spec 3.x-5.x is column 2 (the form the text
    /// annotates) and does not touch `raw_form_type`.
    pub fn set(&mut self, field: &str, value: &str) -> Result<()> {
        let i = self
            .layout
            .index_of(field)
            .ok_or_else(|| FecError::UnknownField {
                table: self.layout.table,
                field: field.to_string(),
            })?;
        if let Some(slot) = self.values.get_mut(i) {
            *slot = CompactString::new(normalize_field(value));
        }
        if self.layout.fields.get(i).is_some_and(|f| f.column == 0) {
            self.raw_form_type = normalize_form_type(value);
        }
        Ok(())
    }

    /// Every `(field, value)` in layout (table) order, including blanks.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&'static str, &str)> + '_ {
        self.layout
            .fields
            .iter()
            .zip(self.values.iter())
            .map(|(f, v)| (f.name, v.as_str()))
    }

    /// Field names in layout order (shared with every line of this layout).
    #[must_use]
    pub fn field_names(&self) -> impl ExactSizeIterator<Item = &'static str> + '_ {
        self.layout.fields.iter().map(|f| f.name)
    }

    /// The record as delimited cells in wire order: `layout.width` cells,
    /// with fields placed at their columns and unassigned columns blank.
    #[must_use]
    pub fn to_cells(&self) -> Vec<&str> {
        let mut cells = vec![""; usize::from(self.layout.width)];
        for (f, v) in self.layout.fields.iter().zip(self.values.iter()) {
            if let Some(slot) = cells.get_mut(usize::from(f.column)) {
                *slot = v.as_str();
            }
        }
        cells
    }

    /// Whether this line is a memo entry (`memo_code == "X"`,
    /// case-insensitive). Memo entries are informational and are excluded
    /// from every cover-page total. Lines whose table has no `memo_code`
    /// column are never memos.
    #[must_use]
    pub fn is_memo(&self) -> bool {
        self.get("memo_code").is_some_and(is_memo_code)
    }

    /// A compile-time-checked view of this line as table `T`.
    ///
    /// ```
    /// use hardmoney::parser::tables::{sch_a, markers::SchA};
    /// # use hardmoney::Filing;
    /// # let text = "HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF3XN\u{1c}C00123456\nSA11AI\u{1c}C00123456";
    /// # let filing = Filing::parse(text).unwrap();
    /// for line in &filing.lines {
    ///     if let Ok(a) = line.typed::<SchA>() {
    ///         println!("{:?}", a.money(sch_a::CONTRIBUTION_AMOUNT));
    ///     }
    /// }
    /// ```
    pub fn typed<T: TableMarker>(&self) -> std::result::Result<Typed<'_, T>, TypedViewError> {
        Typed::new(self)
    }

    /// Converts this line into a domain view such as
    /// [`ScheduleA`](crate::ScheduleA), refusing if the line belongs to a
    /// different table than `V` expects.
    pub fn view<V: TypedView>(&self) -> std::result::Result<V, TypedViewError> {
        V::from_line(self)
    }
}

impl PartialEq for ParsedLine {
    /// Two lines are equal when they were parsed with the same layout
    /// (compared by identity -- layouts are statics) and hold the same
    /// values; `line_no` and `raw_form_type` are included too.
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.layout, other.layout)
            && self.line_no == other.line_no
            && self.raw_form_type == other.raw_form_type
            && self.values == other.values
    }
}
impl Eq for ParsedLine {}

impl fmt::Debug for ParsedLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParsedLine")
            .field("raw_form_type", &self.raw_form_type)
            .field("table", &self.layout.table)
            .field("line_no", &self.line_no)
            .field("fields", &format_args!("{}", NonEmptyFields(self)))
            .finish()
    }
}

/// Debug helper: prints only the non-blank fields of a line.
struct NonEmptyFields<'a>(&'a ParsedLine);

impl fmt::Display for NonEmptyFields<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut m = f.debug_map();
        for (k, v) in self.0.iter().filter(|(_, v)| !v.is_empty()) {
            m.entry(&k, &v);
        }
        m.finish()
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for ParsedLine {
    /// Serialises as `{ "raw_form_type", "table", "line_no", "fields": {name: value, …} }`
    /// with every field of the layout present (blanks as `""`), in layout order.
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeStruct};
        struct Fields<'a>(&'a ParsedLine);
        impl serde::Serialize for Fields<'_> {
            fn serialize<S: serde::Serializer>(
                &self,
                s: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                let mut m = s.serialize_map(Some(self.0.values.len()))?;
                for (k, v) in self.0.iter() {
                    m.serialize_entry(k, v)?;
                }
                m.end()
            }
        }
        let mut st = serializer.serialize_struct("ParsedLine", 4)?;
        st.serialize_field("raw_form_type", &self.raw_form_type)?;
        st.serialize_field("table", &self.layout.table)?;
        st.serialize_field("line_no", &self.line_no)?;
        st.serialize_field("fields", &Fields(self))?;
        st.end()
    }
}

// ---------------------------------------------------------------------------
// Filing
// ---------------------------------------------------------------------------

/// A fully parsed FEC electronic filing (`.fec` file).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Filing {
    /// The parsed `HDR` record (software name/version, report id, etc.).
    pub header: Header,
    /// The filing's FEC spec version -- drives which column layout every
    /// line below is parsed with. Same as `header.version`.
    pub version: SpecVersion,
    /// The top-level form type as filed, upper-cased, e.g. `"F3XA"`.
    pub raw_form_type: String,
    /// `raw_form_type` with any trailing amendment/new/termination
    /// designator stripped, e.g. `"F3X"`.
    pub base_form_type: String,
    /// True if `raw_form_type` designates an amendment (ends in `A`).
    pub is_amendment: bool,
    /// The filing number this filing amends, from the header's `report_id`
    /// (`"FEC-1234567"` -> `1234567`). `None` when the header does not carry
    /// a well-formed reference -- including on amendments, which the FEC's
    /// own validator would reject (`hardmoney validate` reports it).
    pub amends_filing: Option<u64>,
    /// The cover/summary line: physical line 2 of an ASCII-28 filing
    /// (which must carry it there), or the first non-blank line after the
    /// header of a comma-delimited one (`summary.line_no` says which). For
    /// a Form 99, this is also where any `[BEGINTEXT]` block's content
    /// ends up (spliced into the `text` field).
    pub summary: ParsedLine,
    /// Every subsequent body line (schedules, sub-forms, `TEXT` records),
    /// in file order.
    pub lines: Vec<ParsedLine>,
}

/// What to do when a body line cannot be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum OnUnparseableLine {
    /// Fail the whole parse with the first such error (the default).
    #[default]
    Fail,
    /// Record it as a [`SkippedLine`] and keep going.
    Skip,
}

/// Controls how [`Filing::parse_with`] treats body lines it cannot parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ParseOptions {
    /// A body line whose form-type token matches no known table.
    pub on_unknown_line: OnUnparseableLine,
    /// A body line whose table has no column layout for this spec version.
    pub on_missing_version: OnUnparseableLine,
}

impl ParseOptions {
    /// Fail on anything unparseable (what [`Filing::parse`] uses).
    pub const STRICT: Self = Self {
        on_unknown_line: OnUnparseableLine::Fail,
        on_missing_version: OnUnparseableLine::Fail,
    };

    /// Skip anything unparseable, recording it.
    pub const LENIENT: Self = Self {
        on_unknown_line: OnUnparseableLine::Skip,
        on_missing_version: OnUnparseableLine::Skip,
    };

    /// Same as [`ParseOptions::LENIENT`].
    #[must_use]
    pub const fn lenient() -> Self {
        Self::LENIENT
    }

    const fn is_strict(&self) -> bool {
        matches!(self.on_unknown_line, OnUnparseableLine::Fail)
            && matches!(self.on_missing_version, OnUnparseableLine::Fail)
    }
}

/// Why a body line was skipped under a lenient [`ParseOptions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SkipReason {
    /// The form-type token matched no dispatch pattern.
    #[strum(serialize = "unknown form type")]
    UnknownFormType,
    /// The table exists but has no layout for this filing's spec version.
    #[strum(serialize = "no column layout for this spec version")]
    NoLayoutForVersion,
}

/// A body line that a lenient parse could not interpret.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct SkippedLine {
    /// 1-based physical line number.
    pub line_no: u64,
    /// The form-type token (column 0), upper-cased.
    pub raw_form_type: String,
    /// Why the line could not be parsed.
    pub reason: SkipReason,
}

impl fmt::Display for SkippedLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}: '{}' skipped ({})",
            self.line_no, self.raw_form_type, self.reason
        )
    }
}

/// The result of a parse that may have skipped lines.
///
/// This type is `#[must_use]` and has no `Deref` to the inner value on
/// purpose: to get the [`Filing`] out you either acknowledge the skipped
/// lines ([`into_parts`](Self::into_parts)) or assert there were none
/// ([`into_strict`](Self::into_strict)). Data does not go missing silently.
#[must_use = "a Lenient<T> may have skipped lines; call .into_parts() or .into_strict()"]
#[derive(Debug)]
pub struct Lenient<T> {
    value: T,
    skipped: Vec<SkippedLine>,
    first_error: Option<Box<FecError>>,
}

impl<T> Lenient<T> {
    /// Wraps a value with the lines skipped while producing it. `first_error`
    /// is what [`into_strict`](Self::into_strict) reports; when `None` and
    /// `skipped` is non-empty, a generic error is synthesised.
    pub(crate) fn from_parts(
        value: T,
        skipped: Vec<SkippedLine>,
        first_error: Option<Box<FecError>>,
    ) -> Self {
        Self {
            value,
            skipped,
            first_error,
        }
    }

    /// The value together with every skipped line (possibly none).
    pub fn into_parts(self) -> (T, Vec<SkippedLine>) {
        (self.value, self.skipped)
    }

    /// The value, or [`FecError::LinesSkipped`] if anything was skipped
    /// (carrying the first underlying error).
    pub fn into_strict(self) -> Result<T> {
        if self.skipped.is_empty() {
            return Ok(self.value);
        }
        // Both producers record the first skip's error alongside the skip;
        // the fallback only guards against a future producer forgetting.
        let first = self
            .first_error
            .unwrap_or_else(|| Box::new(FecError::MissingFormLine));
        Err(FecError::LinesSkipped {
            count: self.skipped.len(),
            first,
        })
    }

    /// The skipped lines, without consuming.
    #[must_use]
    pub fn skipped(&self) -> &[SkippedLine] {
        &self.skipped
    }

    /// A reference to the value, without consuming. Prefer
    /// [`into_parts`](Self::into_parts) so skipped lines are not forgotten.
    #[must_use]
    pub fn value(&self) -> &T {
        &self.value
    }
}

impl Filing {
    /// Parses a complete filing from its raw text content, strictly.
    ///
    /// Equivalent to `Filing::parse_with(content, &ParseOptions::STRICT)`
    /// followed by `.into_strict()`.
    ///
    /// The delimiter is chosen from the first line: if it contains ASCII 28
    /// the filing is spec 6.0+ (one record per line, no quoting), otherwise
    /// it is comma-delimited spec 3.x-5.x (CSV quoting, one record per
    /// line; blank lines anywhere are skipped). Fails with
    /// [`FecError::DeprecatedHeaderFormat`] on a `/*`-style pre-3.0 header,
    /// [`FecError::MissingFormLine`] if there is no header or cover line
    /// (or the cover's form-type token is blank),
    /// [`FecError::UnknownElectronicHeaderVersion`] if the header's version
    /// is not electronic 3.x-8.x, [`FecError::ParserMissing`] or
    /// [`FecError::NoMatchingVersionBucket`] for a line that cannot be
    /// dispatched (naming the line), and
    /// [`FecError::UnterminatedTextBlock`] for a `[BEGINTEXT]` without its
    /// `[ENDTEXT]`.
    pub fn parse(content: &str) -> Result<Filing> {
        Self::parse_with(content, &ParseOptions::STRICT)?.into_strict()
    }

    /// Parses a complete filing from raw bytes, decoding as UTF-8 first and
    /// falling back to Windows-1252 (the encoding older FECFile-produced
    /// filings sometimes use for filer-entered free text) if that fails;
    /// see [`decode`]. Otherwise identical to [`Filing::parse`].
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
    /// about a single body line (bad header, missing cover line,
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
    /// filing URLs). Requires the `fetch` feature. See
    /// [`Filing::fetch_bytes`] for where the request goes and what fails.
    #[cfg(feature = "fetch")]
    pub fn fetch(filing_id: u64) -> Result<Filing> {
        Self::parse_bytes(&Self::fetch_bytes(filing_id)?)
    }

    /// Downloads a filing's raw bytes from the FEC's document store:
    /// `https://docquery.fec.gov/dcdev/posted/<id>.fec`, or the same path
    /// under `HARDMONEY_DOCQUERY_BASE` when that variable is set
    /// ([`crate::fec::Endpoints::from_env`]).
    ///
    /// Fails with [`FecError::Fetch`] if an endpoint override is not a
    /// usable URL (the message names the variable) or on a transport
    /// error, and [`FecError::Io`] if the body cannot be read. Redirects
    /// are followed. A non-2xx status is a `Fetch` error too (`ureq`'s
    /// default). No cache is involved; [`crate::fec::fetch_filing_bytes`]
    /// is the cached, openFEC-aware download.
    #[cfg(feature = "fetch")]
    pub fn fetch_bytes(filing_id: u64) -> Result<Vec<u8>> {
        // A malformed override is a configuration error, not a network
        // one, but `Fetch` is the variant that fits it without widening
        // the error type; ureq's `BadUri` carries the explanation.
        let endpoints = crate::fec::Endpoints::from_env()
            .map_err(|e| FecError::Fetch(ureq::Error::BadUri(e.to_string())))?;
        let url = endpoints.docquery_filing(filing_id);
        let response = ureq::get(&url).call()?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut response.into_body().into_reader(), &mut bytes)?;
        Ok(bytes)
    }

    /// Whether this filing's base form type is one this crate knows how to
    /// interpret end-to-end.
    #[must_use]
    pub fn is_allowed(&self) -> bool {
        form::is_allowed_top_level_form(&self.base_form_type)
    }

    /// Iterates the body lines that belong to `table`.
    pub fn lines_for(&self, table: Table) -> impl Iterator<Item = &ParsedLine> + '_ {
        self.lines.iter().filter(move |l| l.table() == table)
    }

    /// Iterates the body lines of `V`'s table as domain views, skipping any
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

    /// The cover line as a compile-time-checked view of table `T`.
    ///
    /// ```
    /// use hardmoney::parser::tables::{f3x, markers::F3X};
    /// # use hardmoney::Filing;
    /// # let text = "HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF3XN\u{1c}C00123456";
    /// # let filing = Filing::parse(text).unwrap();
    /// if let Ok(cover) = filing.summary_as::<F3X>() {
    ///     println!("{:?}", cover.money(f3x::COL_A_TOTAL_RECEIPTS));
    /// }
    /// ```
    pub fn summary_as<T: TableMarker>(&self) -> std::result::Result<Typed<'_, T>, TypedViewError> {
        self.summary.typed::<T>()
    }

    fn parse_new_delimited(content: &str, options: &ParseOptions) -> Result<Lenient<Filing>> {
        let mut lines = numbered_lines(content);

        let (_, header_raw) = lines.next().ok_or(FecError::MissingFormLine)?;
        let (_, summary_raw) = lines.next().ok_or(FecError::MissingFormLine)?;
        let header_fields = split_new_delimited(header_raw);
        let summary_fields = split_new_delimited(summary_raw);

        let mut parsed = Self::parse_header_and_summary(&header_fields, &summary_fields, 2)?;
        let mut acc = BodyAccumulator::new(options);

        while let Some((line_no, raw)) = lines.next() {
            if raw.trim().is_empty() {
                continue;
            }

            if raw.trim().eq_ignore_ascii_case("[BEGINTEXT]") {
                let mut collected: Vec<&str> = Vec::new();
                let mut terminated = false;
                for (_, text_raw) in lines.by_ref() {
                    if text_raw.trim().eq_ignore_ascii_case("[ENDTEXT]") {
                        terminated = true;
                        break;
                    }
                    collected.push(text_raw.trim_end_matches('\r'));
                }
                if !terminated {
                    return Err(FecError::UnterminatedTextBlock { line_no });
                }
                acc.attach_free_text(&mut parsed.summary, &collected.join("\n"));
                continue;
            }

            let fields = split_new_delimited(raw);
            if fields.iter().all(|f| f.trim().is_empty()) {
                continue;
            }
            acc.push_body_line(&fields, parsed.version, line_no)?;
        }

        Ok(acc.finish(parsed))
    }

    /// Spec 3.x-5.x: comma-delimited with CSV quoting, **one record per
    /// physical line**. Each line is split on its own (see
    /// [`split_old_delimited`]), so a stray unbalanced quote at the end of
    /// one record -- which real 3.00 filings from at least one vendor
    /// carry on every Schedule H4 line -- cannot swallow the rest of the
    /// file into a single field, and `line_no` is always the physical line.
    /// Blank lines are skipped, including any before the header.
    fn parse_old_delimited(content: &str, options: &ParseOptions) -> Result<Lenient<Filing>> {
        let mut lines = numbered_lines(content).filter(|(_, raw)| !raw.trim().is_empty());

        let (_, header_raw) = lines.next().ok_or(FecError::MissingFormLine)?;
        let (summary_line_no, summary_raw) = lines.next().ok_or(FecError::MissingFormLine)?;
        let header_record = split_old_delimited(header_raw)?;
        let summary_record = split_old_delimited(summary_raw)?;
        let header_fields: Vec<&str> = header_record.iter().collect();
        let summary_fields: Vec<&str> = summary_record.iter().collect();

        let parsed =
            Self::parse_header_and_summary(&header_fields, &summary_fields, summary_line_no)?;
        let mut acc = BodyAccumulator::new(options);

        // Pre-6.0 filings (the only ones using the comma delimiter) predate
        // the [BEGINTEXT]/[ENDTEXT] convention, so no slurp-mode handling
        // is needed here.
        for (line_no, raw) in lines {
            let record = split_old_delimited(raw)?;
            let fields: Vec<&str> = record.iter().collect();
            if fields.iter().all(|f| f.trim().is_empty()) {
                continue;
            }
            acc.push_body_line(&fields, parsed.version, line_no)?;
        }

        Ok(acc.finish(parsed))
    }

    /// Assembles a filing from a header, a cover line, and body lines --
    /// for editors and for callers that build filings rather than parse
    /// them (`Filing` is `#[non_exhaustive]`, so this is the only way to
    /// construct one outside the crate).
    ///
    /// `version`, `raw_form_type`, `base_form_type`, `is_amendment`, and
    /// `amends_filing` are derived exactly as parsing derives them: from
    /// `header.version`, `summary.raw_form_type`, and
    /// [`Header::original_filing_id`].
    ///
    /// Fails with [`FecError::MissingFormLine`] if the cover line's
    /// form-type token is blank, with [`FecError::ParserMissing`] if any
    /// line's token does not dispatch to the table that line was built
    /// with (a `SchA` line whose `form_type` is `SB21B` would be written
    /// with the wrong columns), and with
    /// [`FecError::NoMatchingVersionBucket`] if any line's layout is not
    /// the one for `header.version`. Each error names the line.
    pub fn from_parts(header: Header, summary: ParsedLine, lines: Vec<ParsedLine>) -> Result<Self> {
        let version = header.version;
        let raw_form_type = summary.raw_form_type.clone();
        if raw_form_type.is_empty() {
            return Err(FecError::MissingFormLine);
        }
        for line in std::iter::once(&summary).chain(&lines) {
            if form::table_for_form_type(&line.raw_form_type) != Some(line.table()) {
                return Err(FecError::ParserMissing {
                    form_type: line.raw_form_type.clone(),
                    version,
                    line_no: Some(line.line_no),
                });
            }
            if !line.layout().supports(version) {
                return Err(FecError::NoMatchingVersionBucket {
                    table: line.table(),
                    version,
                    line_no: Some(line.line_no),
                });
            }
        }
        let base_form_type = strip_ant_suffix(&raw_form_type);
        let is_amendment = raw_form_type.ends_with('A');
        let amends_filing = header.original_filing_id();
        Ok(Filing {
            header,
            version,
            raw_form_type,
            base_form_type,
            is_amendment,
            amends_filing,
            summary,
            lines,
        })
    }

    /// Shared header + cover-line parsing, common to both delimiter
    /// styles. `summary_line_no` is the cover line's physical line (2 on
    /// the ASCII-28 path, which requires it there; the comma path skips
    /// blank lines). Returns a `Filing` with an empty `lines` vec --
    /// callers fill that in afterward.
    fn parse_header_and_summary(
        header_fields: &[&str],
        summary_fields: &[&str],
        summary_line_no: u64,
    ) -> Result<Filing> {
        let header = Header::from_fields(header_fields)?;
        let version = header.version;

        let raw_form_type = normalize_form_type(summary_fields.first().copied().unwrap_or(""));
        if raw_form_type.is_empty() {
            return Err(FecError::MissingFormLine);
        }
        let table =
            form::table_for_form_type(&raw_form_type).ok_or_else(|| FecError::ParserMissing {
                form_type: raw_form_type.clone(),
                version,
                line_no: Some(summary_line_no),
            })?;
        let summary = ParsedLine::from_cells(table, version, summary_line_no, summary_fields)?;
        Self::from_parts(header, summary, Vec::new())
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

    fn push_body_line(
        &mut self,
        fields: &[&str],
        version: SpecVersion,
        line_no: u64,
    ) -> Result<()> {
        let form_type = normalize_form_type(fields.first().copied().unwrap_or(""));
        if form_type.is_empty() {
            // A line whose first column is blank carries no record type;
            // treat it like a blank line.
            return Ok(());
        }

        let Some(table) = form::table_for_form_type(&form_type) else {
            let err = FecError::ParserMissing {
                form_type: form_type.clone(),
                version,
                line_no: Some(line_no),
            };
            return self.skip_or_fail(
                self.options.on_unknown_line,
                err,
                line_no,
                form_type,
                SkipReason::UnknownFormType,
            );
        };

        match ParsedLine::from_cells(table, version, line_no, fields) {
            Ok(parsed) => {
                self.lines.push(parsed);
                Ok(())
            }
            Err(e @ FecError::NoMatchingVersionBucket { .. }) => self.skip_or_fail(
                self.options.on_missing_version,
                e,
                line_no,
                form_type,
                SkipReason::NoLayoutForVersion,
            ),
            Err(e) => Err(e),
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
    /// cover line (the common case -- Form 99's own free text). A record
    /// whose layout has no `text` field cannot carry it; the block is then
    /// dropped, which `hardmoney validate` reports.
    fn attach_free_text(&mut self, summary: &mut ParsedLine, text: &str) {
        let target = self.lines.last_mut().unwrap_or(summary);
        let _ = target.set("text", text);
    }

    fn finish(self, mut filing: Filing) -> Lenient<Filing> {
        debug_assert!(!self.options.is_strict() || self.skipped.is_empty());
        filing.lines = self.lines;
        Lenient::from_parts(filing, self.skipped, self.first_error)
    }
}

/// Physical lines of `content` with their 1-based numbers, split as
/// [`str::lines`] does (`\n` or `\r\n`; a lone `\r` is not a break). The
/// count is a `u64` because line numbers appear in errors and in
/// [`ParsedLine::line_no`]; `usize -> u64` cannot lose range on any
/// supported target.
fn numbered_lines(content: &str) -> impl Iterator<Item = (u64, &str)> {
    content
        .lines()
        .enumerate()
        .map(|(i, l)| (u64::try_from(i).unwrap_or(u64::MAX).saturating_add(1), l))
}

/// Decodes raw filing bytes: UTF-8 if valid, else Windows-1252.
///
/// The two encodings are distinguishable in practice. Every byte sequence
/// that Windows-1252 software emits for a *single* non-ASCII character
/// (`0x80..=0xFF`, e.g. `0x92` for a curly apostrophe, `0xE9` for `é`) is
/// invalid on its own as UTF-8, so such files fall through to the
/// fallback. The reverse ambiguity -- a valid multi-byte UTF-8 sequence
/// such as `C3 A9` (`é`) that is *also* two printable Windows-1252
/// characters (`Ã©`) -- is resolved in favour of UTF-8, because no filer
/// types `Ã©` and every UTF-8 filing would otherwise be misread. A file
/// that mixes both encodings decodes entirely as Windows-1252 (its UTF-8
/// sequences then read as mojibake); the streaming reader, which decides
/// per line, keeps the UTF-8 lines intact in that case.
///
/// A leading UTF-8 byte-order mark is dropped: it is an artefact of the
/// editor or exporter, not part of the `HDR` token, and would otherwise
/// come back as `header.record_type == "\u{feff}HDR"`.
///
/// Note that decoding never fails: every byte is a character in
/// Windows-1252 (`encoding_rs` maps the five undefined positions to the
/// corresponding C1 controls), so no input is rejected here.
#[must_use]
pub fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let (decoded, _) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(bytes);
            decoded.into_owned()
        }
    }
}

/// The UTF-8 encoding of U+FEFF.
pub(crate) const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Splits one already-decoded raw line on the ASCII-28 delimiter. A
/// trailing `\r` (CRLF line ending) is dropped; fields are otherwise
/// returned exactly as filed -- per-field trimming happens in
/// [`ParsedLine::from_cells`]. Never fails and never returns an empty
/// vector (an empty line is one empty field).
#[must_use]
pub fn split_new_delimited(raw: &str) -> Vec<&str> {
    raw.trim_end_matches('\r').split(NEW_DELIMITER).collect()
}

/// Splits one physical line of a comma-delimited (spec 3.x-5.x) filing
/// into its fields, applying CSV quoting rules: a field may be wrapped in
/// double quotes, inside which commas are data and `""` is one quote.
///
/// The line is parsed on its own, so a record can never span lines. That
/// matches the FEC's format (one record per line, no line breaks inside
/// fields) and every other parser of the era; it also means the
/// malformations real filings contain degrade gracefully instead of
/// derailing the parse: an unterminated quote runs to the end of the line
/// (`"","","` yields three empty fields), a quote that closes mid-field is
/// dropped (`"ab"c` yields `abc`), and a quote inside an unquoted field is
/// data (`ab"c`). A lone `\r` inside the line is data too; a trailing one
/// (CRLF) is dropped. An empty or all-whitespace line yields a record with
/// a single blank field.
///
/// Returns the `csv` crate's record; the `Result` is for API symmetry with
/// the crate, which cannot fail on an in-memory UTF-8 line.
pub(crate) fn split_old_delimited(raw: &str) -> Result<csv::StringRecord> {
    let raw = raw.trim_end_matches('\r');
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        // Only `\n` ends a record, and the line carries none: a bare `\r`
        // inside a field stays in the field.
        .terminator(csv::Terminator::Any(b'\n'))
        // The reader buffers the whole line once; the default 8 KiB buffer
        // would be allocated per line.
        .buffer_capacity(raw.len().max(1))
        .from_reader(raw.as_bytes());
    let mut record = csv::StringRecord::new();
    if !reader.read_record(&mut record)? {
        // Only an empty input yields no record; represent it as one blank
        // field so callers see the same shape as `split_new_delimited`.
        record.push_field("");
    }
    Ok(record)
}

/// Strips a trailing amendment (`A`), new (`N`), or termination (`T`)
/// designator from a top-level form-type token, returning the upper-cased
/// base form: `"F3XA"` -> `"F3X"`, `"F1MN"` -> `"F1M"`, `"F99"` -> `"F99"`.
///
/// Only a *trailing* designator is stripped, and only when what remains
/// still looks like a form (`F…`), so `"TEXT"` is never mangled.
#[must_use]
pub fn strip_ant_suffix(raw_form_type: &str) -> String {
    let token = raw_form_type.trim().to_ascii_uppercase();
    match token.strip_suffix(['A', 'N', 'T']) {
        Some(base) if !base.is_empty() && base.starts_with('F') => base.to_string(),
        _ => token,
    }
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

    /// Builds a delimited line for `table` at 8.5 with the given fields.
    fn line_85(table: Table, pairs: &[(&str, &str)]) -> String {
        ParsedLine::from_pairs(
            table,
            SpecVersion::electronic(8, 5),
            0,
            pairs.iter().copied(),
        )
        .unwrap()
        .to_cells()
        .join(&NEW_DELIMITER.to_string())
    }

    #[test]
    fn parses_header_and_summary() {
        let filing = Filing::parse(&new_delim_sample()).unwrap();
        assert_eq!(filing.version, SpecVersion::electronic(8, 5));
        assert_eq!(filing.header.soft_name, "FECfile");
        assert_eq!(filing.raw_form_type, "F3XN");
        assert_eq!(filing.base_form_type, "F3X");
        assert_eq!(filing.summary.table(), Table::F3X);
        assert_eq!(filing.summary.line_no, 2);
        assert!(!filing.is_amendment);
        assert!(filing.amends_filing.is_none());
        assert!(filing.is_allowed());
    }

    #[test]
    fn dispatches_body_lines_with_line_numbers() {
        let filing = Filing::parse(&new_delim_sample()).unwrap();
        assert_eq!(filing.lines.len(), 1);
        assert_eq!(filing.lines[0].table(), Table::SchA);
        assert_eq!(filing.lines[0].raw_form_type, "SA11AI");
        assert_eq!(filing.lines[0].line_no, 3);
    }

    #[test]
    fn from_parts_derives_the_same_fields_as_parsing() {
        let parsed = Filing::parse(&new_delim_sample()).unwrap();
        let rebuilt = Filing::from_parts(
            parsed.header.clone(),
            parsed.summary.clone(),
            parsed.lines.clone(),
        )
        .unwrap();
        assert_eq!(rebuilt.version, parsed.version);
        assert_eq!(rebuilt.raw_form_type, parsed.raw_form_type);
        assert_eq!(rebuilt.base_form_type, parsed.base_form_type);
        assert_eq!(rebuilt.is_amendment, parsed.is_amendment);
        assert_eq!(rebuilt.amends_filing, parsed.amends_filing);
        assert_eq!(rebuilt.summary, parsed.summary);
        assert_eq!(rebuilt.lines, parsed.lines);
        assert_eq!(rebuilt.to_fec_string(), parsed.to_fec_string());

        let v85 = SpecVersion::electronic(8, 5);
        let header = Header::from_fields(&["HDR", "FEC", "8.5", "X", "1", "FEC-42", "1"]).unwrap();
        let cover = ParsedLine::from_pairs(
            Table::F3X,
            v85,
            2,
            [
                ("form_type", "F3XA"),
                ("filer_committee_id_number", "C00123456"),
            ],
        )
        .unwrap();
        let f = Filing::from_parts(header, cover, Vec::new()).unwrap();
        assert_eq!(f.base_form_type, "F3X");
        assert!(f.is_amendment);
        assert_eq!(f.amends_filing, Some(42));
    }

    #[test]
    fn from_parts_rejects_inconsistent_parts_naming_the_line() {
        let v85 = SpecVersion::electronic(8, 5);
        let header = Header::from_fields(&["HDR", "FEC", "8.5", "X", "1"]).unwrap();
        let cover = ParsedLine::from_pairs(Table::F3X, v85, 2, [("form_type", "F3XN")]).unwrap();

        // A cover token that dispatches to no table (the `from_pairs`
        // default of the bare table name is not a valid cover token).
        let bare = ParsedLine::from_pairs(Table::F3X, v85, 2, []).unwrap();
        assert!(matches!(
            Filing::from_parts(header.clone(), bare, Vec::new()),
            Err(FecError::ParserMissing {
                line_no: Some(2),
                ..
            })
        ));

        // A body line whose token belongs to a different table.
        let wrong = ParsedLine::from_pairs(Table::SchA, v85, 7, [("form_type", "SB21B")]).unwrap();
        assert!(matches!(
            Filing::from_parts(header.clone(), cover.clone(), vec![wrong]),
            Err(FecError::ParserMissing {
                line_no: Some(7),
                ..
            })
        ));

        // A body line built with another version's layout.
        let old = ParsedLine::from_pairs(
            Table::SchA,
            SpecVersion::electronic(5, 3),
            9,
            [("form_type", "SA11AI")],
        )
        .unwrap();
        assert!(matches!(
            Filing::from_parts(header.clone(), cover.clone(), vec![old]),
            Err(FecError::NoMatchingVersionBucket {
                table: Table::SchA,
                line_no: Some(9),
                ..
            })
        ));

        // A blank cover token.
        let mut blank = cover.clone();
        blank.set("form_type", "").unwrap();
        assert!(matches!(
            Filing::from_parts(header, blank, Vec::new()),
            Err(FecError::MissingFormLine)
        ));
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
        assert_eq!(filing.amends_filing, Some(1_234_567));
        assert_eq!(filing.header.amendment_number(), Some(1));
    }

    #[test]
    fn amendment_without_report_id_parses_with_none() {
        // The FEC's validator rejects this; the parser does not -- see
        // `hardmoney validate`.
        let content = [
            "HDR\u{1c}FEC\u{1c}8.5\u{1c}FECfile\u{1c}8.5.1.0(f34)\u{1c}\u{1c}",
            "F3XA\u{1c}C00123456\u{1c}COMMITTEE NAME",
        ]
        .join("\n");
        let filing = Filing::parse(&content).unwrap();
        assert!(filing.is_amendment);
        assert_eq!(filing.amends_filing, None);
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
        let content = "HDR,FEC,5.3,SoftCo,1.0\nF3XN,C00123456,\"COMMITTEE, INC.\"\n";
        let filing = Filing::parse(content).unwrap();
        assert_eq!(filing.version, SpecVersion::electronic(5, 3));
        assert_eq!(filing.raw_form_type, "F3XN");
        assert_eq!(
            filing.summary.get("committee_name"),
            Some("COMMITTEE, INC.")
        );
        assert_eq!(filing.summary.line_no, 2);
    }

    /// Comma-delimited records are one per physical line, and `line_no`
    /// is the physical line even when blank lines are skipped (the whole-
    /// file CSV reader hardmoney used before 2.3 reported the line *before*
    /// the skipped blanks, and let the cover sit at "line 2" regardless).
    #[test]
    fn old_delimiter_line_numbers_are_physical_and_blank_lines_are_skipped() {
        let content = "\nHDR,FEC,3.00,SoftCo,1.0,^,,\n\n\"F3XN\",\"C00123456\"\n\n\n\"SB21B\",\"C00123456\"\n\"SB21B\",\"C00123456\"\n\n\"H2\",\"C00123456\"\n";
        let filing = Filing::parse(content).unwrap();
        assert_eq!(filing.summary.line_no, 4);
        assert_eq!(
            filing.lines.iter().map(|l| l.line_no).collect::<Vec<_>>(),
            [7, 8, 10]
        );
        // Errors name the physical line too.
        let bad = format!("{content}\n\nZZZ,C00123456\n");
        match Filing::parse(&bad) {
            Err(FecError::ParserMissing { line_no, .. }) => assert_eq!(line_no, Some(13)),
            other => panic!("{other:?}"),
        }
    }

    /// A real 3.00 vendor (Aristotle CM4) ended every Schedule H4 record
    /// with an unbalanced `"`. Read as one CSV stream that quote opens a
    /// field which swallows every following line; read per line each
    /// record survives and the stray quote is an empty last field.
    #[test]
    fn old_delimiter_stray_trailing_quote_does_not_swallow_following_records() {
        let content = "HDR,FEC,3.00,SoftCo,1.0,^,,\nF3XN,C00123456\n\"H4\",\"C00123456\",\"\",\"Payee One\",\"\",\"\",\"\",\"\",\"\",\"\",20010102,10.00,5.00,5.00,\"X\",\"\",\"\",\"\",10.00,\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",,\"\",\"T1\",\"\",\"\",\"\",\"\n\"H4\",\"C00123456\",\"\",\"Payee Two\",\"\",\"\",\"\",\"\",\"\",\"\",20010103,20.00,10.00,10.00,\"X\",\"\",\"\",\"\",20.00,\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",,\"\",\"T2\",\"\",\"\",\"\",\"\n";
        let filing = Filing::parse(content).unwrap();
        assert_eq!(filing.lines.len(), 2);
        assert_eq!(filing.lines[0].get("payee_name"), Some("Payee One"));
        assert_eq!(filing.lines[1].get("payee_name"), Some("Payee Two"));
        assert_eq!(filing.lines[1].line_no, 4);
        for l in &filing.lines {
            assert!(l.iter().all(|(_, v)| !v.contains('"')), "{l:?}");
        }
    }

    #[test]
    fn split_old_delimited_applies_csv_quoting_per_line() {
        let cells = |raw: &str| -> Vec<String> {
            split_old_delimited(raw)
                .unwrap()
                .iter()
                .map(str::to_string)
                .collect()
        };
        assert_eq!(cells("a,\"b,c\",d\r"), ["a", "b,c", "d"]);
        assert_eq!(cells("a,\"say \"\"hi\"\"\",d"), ["a", "say \"hi\"", "d"]);
        assert_eq!(cells("\"\",\"\",\""), ["", "", ""]);
        assert_eq!(cells("a,\"un\"closed,d"), ["a", "unclosed", "d"]);
        assert_eq!(cells("a,un\"quoted,d"), ["a", "un\"quoted", "d"]);
        assert_eq!(cells("a,b\rc"), ["a", "b\rc"]);
        assert_eq!(cells(""), [""]);
        assert_eq!(cells("   "), ["   "]);
        assert_eq!(cells(","), ["", ""]);
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
            "HDR\u{1c}FEC\u{1c}8.5\n\u{1c}\u{1c}",
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
                assert_eq!(version, SpecVersion::electronic(8, 5));
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
        assert_eq!(filing.lines[1].table(), Table::SchB);
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].line_no, 4);
        assert_eq!(skipped[0].raw_form_type, "ZZZ");
        assert_eq!(skipped[0].reason, SkipReason::UnknownFormType);
        assert_eq!(
            skipped[0].to_string(),
            "line 4: 'ZZZ' skipped (unknown form type)"
        );
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
                table: Table::SchI,
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
    fn strip_ant_suffix_strips_only_a_trailing_designator() {
        assert_eq!(strip_ant_suffix("F3XN"), "F3X");
        assert_eq!(strip_ant_suffix("F3A"), "F3");
        assert_eq!(strip_ant_suffix("F3T"), "F3");
        assert_eq!(strip_ant_suffix("F24N"), "F24");
        assert_eq!(strip_ant_suffix("F99"), "F99");
        assert_eq!(strip_ant_suffix("F5"), "F5");
        assert_eq!(strip_ant_suffix("F5N"), "F5");
        assert_eq!(strip_ant_suffix("F13N"), "F13");
        assert_eq!(strip_ant_suffix("F1MN"), "F1M");
        assert_eq!(strip_ant_suffix("F1M"), "F1M");
        assert_eq!(strip_ant_suffix("F2A"), "F2");
        assert_eq!(strip_ant_suffix("f3xa"), "F3X");
        assert_eq!(strip_ant_suffix("TEXT"), "TEXT");
        assert_eq!(strip_ant_suffix("A"), "A");
        assert_eq!(strip_ant_suffix(""), "");
    }

    /// Every top-level form the crate accepts, with every designator the
    /// FEC defines (`N`ew, `A`mendment, `T`ermination) and none, in both
    /// cases, must strip back to its base and report `is_amendment` only
    /// for `A`. No base form ends in A/N/T, so a bare base is never
    /// mangled; the tokens seen on line 2 across the audit corpus (F3A,
    /// F3N, F3T, F3XN, F3XA, F3XT, F3PN, F3PA, F99, F6N, F24N, F1A, F1N,
    /// F1MN, F1MA, F2A, F9N, F7N, F5N, F3LN, F13N) are all covered.
    #[test]
    fn every_allowed_form_strips_its_designator_and_flags_amendments() {
        for base in form::ALLOWED_TOP_LEVEL_FORMS {
            assert!(
                !base.ends_with(['A', 'N', 'T']),
                "{base}: a base form ending in a designator letter would be ambiguous"
            );
            // Only the periodic reports have a termination designator; the
            // other forms are filed as N/A (F99 has no designator at all).
            let has_t = matches!(*base, "F3" | "F3X" | "F3P" | "F4");
            let bare_is_filed = matches!(*base, "F99" | "F6" | "F2" | "F1M" | "F8" | "F10");
            for (suffix, amends) in [("", false), ("N", false), ("A", true), ("T", false)] {
                for token in [
                    format!("{base}{suffix}"),
                    format!("{base}{suffix}").to_lowercase(),
                ] {
                    assert_eq!(strip_ant_suffix(&token), *base, "{token}");
                    let raw = normalize_form_type(&token);
                    assert_eq!(raw.ends_with('A'), amends, "{token}");
                    let real_token = match suffix {
                        "" => bare_is_filed,
                        "T" => has_t,
                        _ => *base != "F99",
                    };
                    if real_token {
                        assert!(
                            form::table_for_form_type(&raw).is_some(),
                            "{token} should dispatch to a table"
                        );
                    }
                }
            }
        }
        // Sub-form and schedule tokens are left alone, even when they end
        // in a designator letter.
        for token in [
            "F3ZT", "TEXT", "SA11AI", "SB21B", "SA17A", "SC/10", "H4", "F1S", "F2S", "F3PS",
            "F3P31", "F3S", "F65", "F8II",
        ] {
            let stripped = strip_ant_suffix(token);
            assert!(
                stripped == token || token == "F3ZT",
                "{token} -> {stripped}"
            );
        }
        assert_eq!(strip_ant_suffix("F3ZT"), "F3Z");
    }

    #[test]
    fn field_values_are_preserved_verbatim_except_trimming() {
        let sb = line_85(
            Table::SchB,
            &[
                ("form_type", "sb21b"),
                ("filer_committee_id_number", "C00123456"),
                ("payee_organization_name", "  Vendor, Inc.  "),
            ],
        );
        let content = [
            "HDR\u{1c}FEC\u{1c}8.5\u{1c}FECfile\u{1c}8.5.1.0(f34)\u{1c}\u{1c}".to_string(),
            "F3XN\u{1c}C00123456\u{1c}  Friends of AT&T <PAC> \"Official\" \\ a|b  ".to_string(),
            sb,
        ]
        .join("\r\n");
        let filing = Filing::parse(&content).unwrap();
        assert_eq!(filing.header.soft_name, "FECfile");
        assert_eq!(
            filing.summary.get("committee_name"),
            Some("Friends of AT&T <PAC> \"Official\" \\ a|b")
        );
        // Tokens are normalised for dispatch, but the field keeps the
        // as-filed spelling.
        assert_eq!(filing.lines[0].raw_form_type, "SB21B");
        assert_eq!(filing.lines[0].get("form_type"), Some("sb21b"));
        assert_eq!(
            filing.lines[0].get("payee_organization_name"),
            Some("Vendor, Inc.")
        );
    }

    #[test]
    fn get_distinguishes_blank_from_absent() {
        let filing = Filing::parse(&new_delim_sample()).unwrap();
        let sa = &filing.lines[0];
        assert_eq!(sa.get("contribution_amount"), Some(""));
        assert_eq!(sa.get_non_empty("contribution_amount"), None);
        assert_eq!(sa.get("no_such_field"), None);
        assert_eq!(sa.iter().len(), sa.layout().fields.len());
    }

    #[test]
    fn set_rejects_unknown_fields_and_tracks_form_type() {
        let mut line = ParsedLine::from_pairs(
            Table::SchA,
            SpecVersion::electronic(8, 5),
            0,
            [("contribution_amount", " 12.50 ")],
        )
        .unwrap();
        assert_eq!(line.raw_form_type, "SCHA");
        assert_eq!(line.get("contribution_amount"), Some("12.50"));
        line.set("form_type", "sa11ai").unwrap();
        assert_eq!(line.raw_form_type, "SA11AI");
        assert!(matches!(
            line.set("bogus", "x"),
            Err(FecError::UnknownField { table: Table::SchA, field }) if field == "bogus"
        ));
        assert!(ParsedLine::from_pairs(Table::SchI, SpecVersion::electronic(8, 5), 0, []).is_err());
    }

    #[test]
    fn to_cells_places_fields_at_layout_columns() {
        let line = ParsedLine::from_pairs(
            Table::SchA,
            SpecVersion::electronic(8, 5),
            0,
            [("form_type", "SA11AI"), ("memo_code", "X")],
        )
        .unwrap();
        let cells = line.to_cells();
        let layout = Table::SchA.layout(SpecVersion::electronic(8, 5)).unwrap();
        assert_eq!(cells.len(), usize::from(layout.width));
        assert_eq!(cells[0], "SA11AI");
        let memo_col = usize::from(layout.field("memo_code").unwrap().column);
        assert_eq!(cells[memo_col], "X");
        assert!(
            cells
                .iter()
                .enumerate()
                .all(|(i, c)| c.is_empty() || i == 0 || i == memo_col)
        );
    }

    #[test]
    fn is_memo_is_case_insensitive_and_table_aware() {
        let sa = line_85(
            Table::SchA,
            &[
                ("form_type", "SA11AI"),
                ("filer_committee_id_number", "C1"),
                ("memo_code", "x"),
            ],
        );
        let content = format!("{}\n{sa}", new_delim_sample());
        let filing = Filing::parse(&content).unwrap();
        assert!(!filing.lines[0].is_memo());
        assert!(filing.lines[1].is_memo(), "{:?}", filing.lines[1]);
        assert!(!filing.summary.is_memo());
    }

    #[test]
    fn typed_view_checks_table_once_then_fields_at_compile_time() {
        use crate::parser::tables::{f3x, markers::F3X, markers::SchA, sch_a};
        let filing = Filing::parse(&new_delim_sample()).unwrap();
        let cover = filing.summary_as::<F3X>().unwrap();
        assert_eq!(cover.get(f3x::FILER_COMMITTEE_ID_NUMBER), Some("C00123456"));
        assert_eq!(cover.money(f3x::COL_A_TOTAL_RECEIPTS), None);
        assert!(matches!(
            filing.summary.typed::<SchA>(),
            Err(TypedViewError::WrongTable {
                expected: Table::SchA,
                found: Table::F3X,
                line_no: 2
            })
        ));
        let a = filing.lines[0].typed::<SchA>().unwrap();
        assert_eq!(a.get(sch_a::FILER_COMMITTEE_ID_NUMBER), Some("C00123456"));
        assert_eq!(a.get(sch_a::CONTRIBUTION_AMOUNT), None);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serializes_fields_as_a_map_in_layout_order() {
        let filing = Filing::parse(&new_delim_sample()).unwrap();
        let v = serde_json::to_value(&filing.lines[0]).unwrap();
        assert_eq!(v["table"], "SchA");
        assert_eq!(v["line_no"], 3);
        // `new_delim_sample` puts "IND" in column 2, which is
        // `transaction_id` at 8.5.
        assert_eq!(v["fields"]["transaction_id"], "IND");
        assert_eq!(v["fields"]["contribution_amount"], "");
        let keys: Vec<&str> = v["fields"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys[0], "form_type");
    }
}
