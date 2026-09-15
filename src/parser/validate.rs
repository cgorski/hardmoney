//! `Filing::validate`: the FEC's acceptance rules, runnable anywhere.
//!
//! The FEC runs every electronic filing through a validator before
//! accepting it (the same engine ships as *WebCheck* and inside FECFile).
//! It emits **failing** messages, which reject the filing, and **warning**
//! messages, which do not. This module reimplements the checks that can be
//! evaluated from the filing alone -- structure, IDs, field types, lengths,
//! character set, dates, amounts, code lists, transaction-ID integrity --
//! driven by the bundled spec workbook ([`FieldSpec`]) rather than by
//! hard-coded lengths, so a spec change is a data change.
//!
//! ```no_run
//! use hardmoney::Filing;
//!
//! let filing = Filing::parse_bytes(&std::fs::read("filing.fec").unwrap()).unwrap();
//! let report = filing.validate();
//! for finding in report.errors() {
//!     println!("{finding}");
//! }
//! assert!(report.is_acceptable());
//! ```
//!
//! # What is (and is not) checked
//!
//! Every [`Rule`] documents the FEC message it corresponds to and its
//! severity ([`Rule::severity`], [`Rule::fec_message`]). Deliberate
//! deviations from the FEC's own validator:
//!
//! * [`Rule::CurrentFormat`] is a **warning** here, not a failure. hardmoney
//!   knowingly parses every spec version since 3.x; refusing a 2004 filing
//!   would defeat the purpose. The FEC rejects anything but the current
//!   format at upload time. Because the bundled workbook's *required*
//!   levels describe the current format only (real, accepted 3.x and 5.x
//!   filings leave `entity_type` blank), [`Rule::RequiredFieldEmpty`] is
//!   reported at warning severity on such filings. Every other check is
//!   format-stable (character set, date and amount syntax, lengths, IDs,
//!   transaction-ID integrity) and keeps its severity.
//! * *Leading blanks* (FEC failing message #6/#31) cannot be detected: the
//!   parser trims surrounding ASCII whitespace from every field before the
//!   validator sees it (see [`crate::parser::utils`]).
//! * Characters in a Form 99's free-text block are checked at **warning**
//!   level ([`Rule::F99IllegalCharacter`]) because the FEC has accepted
//!   real F99 filings containing out-of-range bytes there.
//! * Summary-page arithmetic (warnings 3-4, "Subtotal not supported by
//!   Schedule") lives in [`crate::parser::reconcile`], not here.
//! * Cross-filing checks (the transaction-ID uniqueness "for the life of
//!   the report", report-type-vs-form consistency) need data this crate
//!   does not have in a single file.
//!
//! # Surrounding quotes
//!
//! The FEC's validator accepts a field wrapped in double quotes
//! (`"SMITH"`) and reads the content between the quotes; some vendors emit
//! every field that way. Every per-field check here does the same
//! ([`effective_value`]), and [`Rule::EmbeddedDoubleQuote`] fires only for
//! a quote *inside* a quote-wrapped value, exactly as the FEC's message #30
//! describes.

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use regex::Regex;

use crate::parser::filing::{Filing, Lenient, ParsedLine, SkippedLine};
use crate::parser::header::Header;
use crate::parser::schema::{FieldKind, FieldSpec, Requirement, SpecVersion};
use crate::parser::tables::{BUNDLED_SPEC_VERSION, Table};
use crate::parser::typed::{parse_fec_date, parse_money};

// ---------------------------------------------------------------------------
// Severity, Rule, Finding, Validation
// ---------------------------------------------------------------------------

/// How the FEC treats a finding: a **failing** message rejects the filing,
/// a **warning** is reported but the filing is accepted.
///
/// Orders `Warning < Error`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, strum::Display, strum::EnumString,
)]
#[strum(serialize_all = "lowercase")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Severity {
    /// Reported; the FEC still accepts the filing.
    Warning,
    /// The FEC rejects the filing.
    Error,
}

/// One validation rule. Each maps to a message in the FEC's published list
/// of validation messages (see [`Rule::fec_message`]) and has a fixed
/// [`Severity`] (see [`Rule::severity`]).
///
/// `Display`/`FromStr` use `snake_case` names (`required_field_empty`),
/// which is also the serde representation.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::EnumIter,
)]
#[strum(serialize_all = "snake_case")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum Rule {
    // -- Structure ----------------------------------------------------------
    /// The first record's type is not `HDR` (FEC failing #14).
    HeaderFirst,
    /// The second record is not a top-level form cover page (FEC failing #15).
    CoverSecond,
    /// The header's filing type is not `FEC` (FEC failing #16).
    FilingTypeFec,
    /// The filing's spec version is not the current one (FEC failing #11;
    /// a **warning** here -- see the module docs).
    CurrentFormat,
    /// An amendment whose header carries no well-formed `FEC-<n>` report id
    /// (FEC failing #17).
    AmendmentNeedsOriginalId,
    /// An amendment whose header carries no numeric amendment number (FEC
    /// failing #9).
    AmendmentNeedsNumber,
    /// A new/termination report whose header nonetheless carries a report
    /// id or amendment number (FEC failing #8; a warning here because the
    /// data is merely surplus).
    HeaderInconsistentWithAmendmentStatus,
    /// A second top-level form record in the body (FEC failing #23).
    MultipleForms,
    /// A schedule the spec does not associate with this filing's form (FEC
    /// failing #19).
    ScheduleNotAllowedWithForm,
    /// A body line whose form-type token matches no known record type; the
    /// FEC ignores the record (FEC failing #18). Only produced from
    /// [`Lenient::validate`] / [`Validation::note_skipped`].
    UnrecognizedFormType,

    // -- IDs ----------------------------------------------------------------
    /// The cover page's filer ID is not a well-formed FEC committee or
    /// candidate ID (FEC failing #5).
    FilerIdFormat,
    /// A body line's filer ID differs from the cover page's (FEC failing #21).
    FilerIdMismatch,

    // -- Per-field, driven by `FieldSpec` -----------------------------------
    /// A field the spec marks `X (error)` is blank (FEC failing #1-#4).
    RequiredFieldEmpty,
    /// A field the spec marks `X (warning)` is blank (FEC warning, e.g. #27
    /// "Street Address is Missing").
    RecommendedFieldEmpty,
    /// A conditionally required field is blank and its condition holds
    /// (FEC warning #1).
    ConditionallyRequiredFieldEmpty,
    /// More characters than the spec's type allows (FEC failing #7).
    FieldTooLong,
    /// A character outside the FEC's allowed set (FEC failing #12/#27).
    IllegalCharacter,
    /// A character outside the FEC's allowed set in a Form 99's free text
    /// (FEC failing #13/#28; a warning here -- see the module docs).
    F99IllegalCharacter,
    /// A double quote inside a quote-wrapped field (FEC failing #30).
    EmbeddedDoubleQuote,
    /// A date field that is not eight digits (FEC failing #32).
    BadDateFormat,
    /// An eight-digit date that is not a calendar date (FEC failing #33).
    NotARealDate,
    /// A date outside 1960-2099 (FEC warning #2).
    DateOutOfRange,
    /// An amount that is not `[-]digits[.dd]` -- dollar signs and commas
    /// included (FEC failing #34).
    InvalidAmount,
    /// A non-digit in a numeric (non-date) field (FEC failing #35).
    NonNumeric,
    /// A value outside the spec's published list for the field (a warning:
    /// the lists come from the FEC's own filing software and can lag).
    InvalidAllowedValue,
    /// A value that does not match the spec's published regex for the field.
    PatternMismatch,
    /// Not a USPS state/territory code (FEC warning #29).
    InvalidStateCode,
    /// An `entity_type` outside `CAN CCM COM IND ORG PAC PTY` (FEC warning #45).
    InvalidEntityType,
    /// A Schedule E / Form 57 / Form 76 support/oppose code other than `S`
    /// or `O` (FEC warning #36).
    InvalidSupportOpposeCode,

    // -- Cross-line ---------------------------------------------------------
    /// The same transaction ID (case-insensitively) on two lines (FEC
    /// failing #40).
    DuplicateTransactionId,
    /// A back reference to a transaction ID that is not in the filing (FEC
    /// failing #10/#41).
    BackReferenceNotFound,

    // -- Form 99 ------------------------------------------------------------
    /// A Form 99 text block over 20,000 characters (FEC failing #29).
    F99TextTooLong,
}

impl Rule {
    /// The severity every finding of this rule carries.
    #[must_use]
    pub const fn severity(self) -> Severity {
        match self {
            Rule::CurrentFormat
            | Rule::HeaderInconsistentWithAmendmentStatus
            | Rule::UnrecognizedFormType
            | Rule::RecommendedFieldEmpty
            | Rule::ConditionallyRequiredFieldEmpty
            | Rule::F99IllegalCharacter
            | Rule::DateOutOfRange
            | Rule::InvalidAllowedValue
            | Rule::PatternMismatch
            | Rule::InvalidStateCode
            | Rule::InvalidEntityType
            | Rule::InvalidSupportOpposeCode => Severity::Warning,
            Rule::HeaderFirst
            | Rule::CoverSecond
            | Rule::FilingTypeFec
            | Rule::AmendmentNeedsOriginalId
            | Rule::AmendmentNeedsNumber
            | Rule::MultipleForms
            | Rule::ScheduleNotAllowedWithForm
            | Rule::FilerIdFormat
            | Rule::FilerIdMismatch
            | Rule::RequiredFieldEmpty
            | Rule::FieldTooLong
            | Rule::IllegalCharacter
            | Rule::EmbeddedDoubleQuote
            | Rule::BadDateFormat
            | Rule::NotARealDate
            | Rule::InvalidAmount
            | Rule::NonNumeric
            | Rule::DuplicateTransactionId
            | Rule::BackReferenceNotFound
            | Rule::F99TextTooLong => Severity::Error,
        }
    }

    /// The FEC's own message template for this rule, as published in
    /// *Validation errors explained* (blanks shown as `____`). Finding
    /// messages are worded after these so filing vendors recognise them.
    #[must_use]
    pub const fn fec_message(self) -> &'static str {
        match self {
            Rule::HeaderFirst => "HDR record must be First in File",
            Rule::CoverSecond => "\"Cover\" (eg. F3A, F3XN, ...) must be 2nd in File",
            Rule::FilingTypeFec => "Filing must be an \"FEC\" Type of filing",
            Rule::CurrentFormat => "Filing must be in the current FEC format",
            Rule::AmendmentNeedsOriginalId => "Amended filing must have an ID of the \"Original\"",
            Rule::AmendmentNeedsNumber => "Amended filing must have an \"Amendment Number\"",
            Rule::HeaderInconsistentWithAmendmentStatus => {
                "Header (HDR) inconsistent with Orig/Amend status"
            }
            Rule::MultipleForms => "Multi-Form Filings are NOT Allowed",
            Rule::ScheduleNotAllowedWithForm => "Schedule does not belong with Form ____",
            Rule::UnrecognizedFormType => "Unrecognized Form Type / Record Ignored",
            Rule::FilerIdFormat => "ID# _________ NOT Correct FEC ID# Format",
            Rule::FilerIdMismatch => "ID# _________ NOT SAME AS Cover Page ID# _________",
            Rule::RequiredFieldEmpty => "{field} is Required, but field is Empty",
            Rule::RecommendedFieldEmpty => "{field} is Missing",
            Rule::ConditionallyRequiredFieldEmpty => "Conditionally Required field is Empty",
            Rule::FieldTooLong => "{field} exceeds maximum length of ______",
            Rule::IllegalCharacter => "Illegal character(s) found in text field",
            Rule::F99IllegalCharacter => {
                "Illegal character(s) found in text line #____ (Used for F99's)"
            }
            Rule::EmbeddedDoubleQuote => "Embedded double-quotes (\") not allowed",
            Rule::BadDateFormat => "Bad Date - ________ not YYYYMMDD format",
            Rule::NotARealDate => "________ is not a Real Date",
            Rule::DateOutOfRange => "__{date}__ is outside range of 1960-2099",
            Rule::InvalidAmount => "Invalid Amount format: ____________",
            Rule::NonNumeric => "Non-numeric data in Numeric Field",
            Rule::InvalidAllowedValue => "Value \"_\" is Invalid for this field",
            Rule::PatternMismatch => "Value \"_\" does not match the required format",
            Rule::InvalidStateCode => "__ not a valid 2-character USPS State Code",
            Rule::InvalidEntityType => "Entity Type [___] is not an acceptable value",
            Rule::InvalidSupportOpposeCode => "Sup/Opp Code \"___\" Invalid (Valid Codes: S, O)",
            Rule::DuplicateTransactionId => "Tran ID is NOT UNIQUE - This one is same as other(s)",
            Rule::BackReferenceNotFound => "Back-Reference TRAN-ID does not match Sched TRAN-ID",
            Rule::F99TextTooLong => {
                "Body of text exceeds maximum of 20,000 characters (F99 filings)"
            }
        }
    }
}

/// One validation message, tied to a line (and usually a field) of the
/// filing.
///
/// `severity` is `rule.severity()` with one exception: on a filing in a
/// superseded spec version, [`Rule::RequiredFieldEmpty`] is reported at
/// [`Severity::Warning`] (see the module docs). Filter on `severity`, not
/// on `rule.severity()`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Finding {
    pub severity: Severity,
    pub rule: Rule,
    /// 1-based physical line in the `.fec` file (1 = `HDR`, 2 = cover).
    pub line_no: u64,
    /// The record's form-type token as filed, upper-cased (`"HDR"`,
    /// `"F3XN"`, `"SA11AI"`).
    pub form_type: String,
    /// The canonical field name, when the finding is about one field.
    pub field: Option<&'static str>,
    /// A complete sentence a filer could act on, worded after the FEC's.
    pub message: String,
}

impl Finding {
    /// Builds a finding; `severity` is derived from `rule`.
    #[must_use]
    pub fn new(
        rule: Rule,
        line_no: u64,
        form_type: impl Into<String>,
        field: Option<&'static str>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: rule.severity(),
            rule,
            line_no,
            form_type: form_type.into(),
            field,
            message: message.into(),
        }
    }
}

impl From<&SkippedLine> for Finding {
    /// A skipped body line becomes an [`Rule::UnrecognizedFormType`] warning.
    fn from(s: &SkippedLine) -> Self {
        Finding::new(
            Rule::UnrecognizedFormType,
            s.line_no,
            s.raw_form_type.clone(),
            Some("form_type"),
            format!(
                "Unrecognized Form Type / Record Ignored ('{}': {})",
                s.raw_form_type, s.reason
            ),
        )
    }
}

impl fmt::Display for Finding {
    /// `ERROR line 12 SA11AI contributor_last_name: <message>` -- one line,
    /// like a row of the FEC's WebCheck output.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            Severity::Error => "ERROR",
            Severity::Warning => "WARN ",
        };
        write!(f, "{severity} line {} {}", self.line_no, self.form_type)?;
        if let Some(field) = self.field {
            write!(f, " {field}")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// The result of [`Filing::validate`]: every finding, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[must_use]
pub struct Validation {
    /// All findings, sorted by line number (stable, so findings on one line
    /// keep the order the rules ran in).
    pub findings: Vec<Finding>,
}

impl Validation {
    /// True when there are no [`Severity::Error`] findings -- the FEC would
    /// accept the filing (possibly with warnings).
    #[must_use]
    pub fn is_acceptable(&self) -> bool {
        self.error_count() == 0
    }

    /// The error-severity findings.
    pub fn errors(&self) -> impl Iterator<Item = &Finding> + '_ {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
    }

    /// The warning-severity findings.
    pub fn warnings(&self) -> impl Iterator<Item = &Finding> + '_ {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Warning)
    }

    #[must_use]
    pub fn error_count(&self) -> usize {
        self.errors().count()
    }

    #[must_use]
    pub fn warning_count(&self) -> usize {
        self.warnings().count()
    }

    /// Every finding, in line order.
    pub fn iter(&self) -> std::slice::Iter<'_, Finding> {
        self.findings.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.findings.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.findings.is_empty()
    }

    /// Number of findings per rule, for summaries.
    #[must_use]
    pub fn counts_by_rule(&self) -> HashMap<Rule, usize> {
        let mut m = HashMap::new();
        for f in &self.findings {
            *m.entry(f.rule).or_insert(0) += 1;
        }
        m
    }

    /// Adds one finding, keeping line order.
    pub fn push(&mut self, finding: Finding) {
        let at = self
            .findings
            .partition_point(|f| f.line_no <= finding.line_no);
        self.findings.insert(at, finding);
    }

    /// Records body lines a lenient parse skipped as
    /// [`Rule::UnrecognizedFormType`] warnings (FEC failing #18: the FEC
    /// ignores such records rather than rejecting the filing).
    pub fn note_skipped(&mut self, skipped: &[SkippedLine]) {
        for s in skipped {
            self.push(Finding::from(s));
        }
    }
}

impl fmt::Display for Validation {
    /// One finding per line (see [`Finding`]'s `Display`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for finding in &self.findings {
            writeln!(f, "{finding}")?;
        }
        Ok(())
    }
}

impl IntoIterator for Validation {
    type Item = Finding;
    type IntoIter = std::vec::IntoIter<Finding>;
    fn into_iter(self) -> Self::IntoIter {
        self.findings.into_iter()
    }
}

impl<'a> IntoIterator for &'a Validation {
    type Item = &'a Finding;
    type IntoIter = std::slice::Iter<'a, Finding>;
    fn into_iter(self) -> Self::IntoIter {
        self.findings.iter()
    }
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

impl Filing {
    /// Checks this filing against the FEC's acceptance rules (see the
    /// [module docs](self) for the list and for deliberate deviations).
    ///
    /// Never fails: a filing that parsed can always be validated. Findings
    /// come back in file order; [`Validation::is_acceptable`] is the
    /// one-bit answer.
    pub fn validate(&self) -> Validation {
        let mut checker = Checker {
            filing: self,
            findings: Vec::new(),
        };
        checker.check_header();
        checker.check_cover();
        checker.check_body();
        checker.check_transaction_ids();
        checker.check_f99_text();
        let mut findings = checker.findings;
        findings.sort_by_key(|f| f.line_no);
        Validation { findings }
    }
}

impl Lenient<Filing> {
    /// [`Filing::validate`] plus one [`Rule::UnrecognizedFormType`] warning
    /// per body line the lenient parse skipped.
    pub fn validate(&self) -> Validation {
        let mut v = self.value().validate();
        v.note_skipped(self.skipped());
        v
    }
}

// ---------------------------------------------------------------------------
// Value helpers (public: useful to anyone writing their own checks)
// ---------------------------------------------------------------------------

/// The value the FEC's validator sees: surrounding ASCII whitespace
/// trimmed, and one pair of wrapping double quotes removed (then trimmed
/// again). `"SMITH"` -> `SMITH`; `""` -> empty; `"` alone is unchanged.
#[must_use]
pub fn effective_value(raw: &str) -> &str {
    let trimmed = raw.trim_matches(|c: char| c.is_ascii_whitespace());
    match quoted_inner(trimmed) {
        Some(inner) => inner.trim_matches(|c: char| c.is_ascii_whitespace()),
        None => trimmed,
    }
}

/// The content between wrapping double quotes, if `value` is at least two
/// characters and starts and ends with `"`.
fn quoted_inner(value: &str) -> Option<&str> {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value.get(1..value.len() - 1)
    } else {
        None
    }
}

/// Whether `id` is a well-formed FEC committee or candidate ID:
/// `C`/`P` + 8 digits, or `H`/`S` + digit + 2 upper-case letters + 5
/// digits -- the spec's `^(?:[PC][0-9]{8}|[HS][0-9]{1}[A-Z]{2}[0-9]{5})$`.
#[must_use]
pub fn is_fec_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    let (Some(&kind), Some(tail)) = (bytes.first(), bytes.get(1..)) else {
        return false;
    };
    if tail.len() != 8 {
        return false;
    }
    match kind {
        b'C' | b'P' => tail.iter().all(u8::is_ascii_digit),
        b'H' | b'S' => matches!(
            tail,
            [d, a, b, rest @ ..]
                if d.is_ascii_digit()
                    && a.is_ascii_uppercase()
                    && b.is_ascii_uppercase()
                    && rest.iter().all(u8::is_ascii_digit)
        ),
        _ => false,
    }
}

/// The characters Windows-1252 bytes `0x80`-`0x9C` decode to (`€ ‚ ƒ „ … †
/// ‡ ˆ ‰ Š ‹ Œ Ž ‘ ’ “ ” • – — ˜ ™ š › œ`). The FEC's rule is stated in
/// bytes (32-168 and 173, excluding 127 and 157-159); a filing in that code
/// page reaches us decoded, so these are legal too. Bytes 157-159 (`ž Ÿ`
/// and an undefined slot) are excluded by the FEC and so absent here.
const CP1252_HIGH_CHARS: &[char] = &[
    '\u{20AC}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}', '\u{02C6}',
    '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{017D}', '\u{2018}', '\u{2019}', '\u{201C}',
    '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}',
    '\u{0153}',
];

/// Whether `c` is in the FEC's allowed character set for a delimited field:
/// ASCII 32-126, code points 128-168 and 173 (excluding 127 and 157-159),
/// plus the characters Windows-1252 bytes 0x80-0x9C decode to (`€ … ‘ ’ “
/// ” – — ™` and the like), since a filing in that code page reaches the
/// validator decoded. TAB and newlines are *not* allowed here; see
/// [`is_legal_f99_text_char`].
#[must_use]
pub fn is_legal_char(c: char) -> bool {
    matches!(u32::from(c), 32..=126 | 128..=156 | 160..=168 | 173) || CP1252_HIGH_CHARS.contains(&c)
}

/// [`is_legal_char`], plus TAB (which the FEC allows in Form 99 text) and
/// the line breaks the parser uses to join a `[BEGINTEXT]` block.
#[must_use]
pub fn is_legal_f99_text_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r') || is_legal_char(c)
}

/// USPS two-letter codes for the states, DC, territories, freely associated
/// states, and military post offices, plus the FEC's `ZZ` for foreign
/// addresses -- the FEC's "Edit: ST" list.
pub const STATE_CODES: &[&str] = &[
    "AA", "AE", "AK", "AL", "AP", "AR", "AS", "AZ", "CA", "CO", "CT", "DC", "DE", "FL", "FM", "GA",
    "GU", "HI", "IA", "ID", "IL", "IN", "KS", "KY", "LA", "MA", "MD", "ME", "MH", "MI", "MN", "MO",
    "MP", "MS", "MT", "NC", "ND", "NE", "NH", "NJ", "NM", "NV", "NY", "OH", "OK", "OR", "PA", "PR",
    "PW", "RI", "SC", "SD", "TN", "TX", "UM", "UT", "VA", "VI", "VT", "WA", "WI", "WV", "WY", "ZZ",
];

/// The FEC's entity-type codes.
pub const ENTITY_TYPES: &[&str] = &["CAN", "CCM", "COM", "IND", "ORG", "PAC", "PTY"];

/// Maximum characters in a Form 99's text block (FEC failing #29).
pub const F99_TEXT_MAX_CHARS: usize = 20_000;

/// The FEC's dates must fall in this range of years (FEC warning #2).
pub const DATE_YEAR_RANGE: std::ops::RangeInclusive<i32> = 1960..=2099;

/// Every distinct `FieldSpec::pattern` in the bundled spec, compiled once.
/// A pattern that fails to compile is simply absent (never a panic); the
/// `every_spec_pattern_compiles` test catches that.
static PATTERNS: LazyLock<HashMap<&'static str, Regex>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    for table in Table::ALL {
        for spec in table.specs() {
            if let Some(p) = spec.pattern
                && !m.contains_key(p)
                && let Ok(re) = Regex::new(p)
            {
                m.insert(p, re);
            }
        }
    }
    m
});

/// The parsed [`BUNDLED_SPEC_VERSION`], if it parses (it does; a test
/// pins it).
fn bundled_version() -> Option<SpecVersion> {
    BUNDLED_SPEC_VERSION.parse().ok()
}

/// Top-level form tables: the ones that may appear on the cover line and
/// therefore may *not* appear in the body (FEC failing #23). Everything
/// else -- schedules, sub-forms (F3S, F3Z, F1S, F56, F65, H1-H6, ...),
/// `TEXT` -- is a legitimate body record.
fn is_top_level_form(table: Table) -> bool {
    matches!(
        table,
        Table::F1
            | Table::F10
            | Table::F13
            | Table::F1M
            | Table::F2
            | Table::F24
            | Table::F3
            | Table::F3L
            | Table::F3P
            | Table::F3X
            | Table::F4
            | Table::F5
            | Table::F6
            | Table::F7
            | Table::F8
            | Table::F9
            | Table::F99
    )
}

/// Whether the header's amendment number is meaningfully set. FECfile and
/// other vendors write `0` on original reports; the FEC accepts that, so
/// `0` counts as absent.
fn has_amendment_number(h: &Header) -> bool {
    !h.report_number.is_empty() && h.amendment_number() != Some(0)
}

/// Whether a spec row describes a state-code field: the workbook marks
/// these with `AK,AL,...,ZZ` as the value reference or `Edit: ST` as the
/// rule. The length guard matters because on one sheet (Form 6) the
/// workbook's rule column is shifted by a row, putting `Edit: ST` on CITY.
fn is_state_field(spec: &FieldSpec) -> bool {
    spec.max_len == Some(2)
        && (spec.value_reference.is_some_and(|v| v.starts_with("AK,AL"))
            || spec.rule.is_some_and(|r| {
                r.trim()
                    .get(..8)
                    .is_some_and(|p| p.eq_ignore_ascii_case("edit: st"))
            }))
}

/// Requirement conditions the workbook leaves implicit. Form 9 marks its
/// filer-name columns `X (error)` with no rule, while the otherwise
/// identical Form 5 columns say "If not Organization" / "If not
/// Individual"; the filer is one or the other, never both. Only applies to
/// tables that carry an `entity_type` column (Form 7's `organization_name`
/// is unconditional).
fn implied_condition(line: &ParsedLine, name: &str) -> Option<Condition> {
    line.get("entity_type")?;
    match name {
        "individual_last_name" | "individual_first_name" => Some(Condition::EntityIn(IND_CAN)),
        "organization_name" => Some(Condition::EntityNotIn(IND_CAN)),
        _ => None,
    }
}

/// Whether a spec row is a `YYYYMMDD` date: every `NUM-8` column in the
/// workbook is one.
fn is_date_field(spec: &FieldSpec) -> bool {
    spec.kind == FieldKind::Numeric && spec.max_len == Some(8)
}

fn transaction_id(line: &ParsedLine) -> Option<&str> {
    line.get("transaction_id")
        .or_else(|| line.get("transaction_id_number"))
        .map(effective_value)
        .filter(|s| !s.is_empty())
}

// ---------------------------------------------------------------------------
// Requirement conditions
// ---------------------------------------------------------------------------

/// The subset of the workbook's requirement conditions that can be
/// evaluated from the line itself.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Condition {
    /// Unconditionally required.
    Always,
    /// Required when the line's `entity_type` is one of these codes.
    EntityIn(&'static [&'static str]),
    /// Required when the line's `entity_type` is present and none of these.
    EntityNotIn(&'static [&'static str]),
    /// Required when the filing's raw form type is this token.
    FormTypeIs(String),
    /// Required when the line's `report_code` starts with one of these.
    ReportCodeStartsWith(&'static [&'static str]),
    /// Prose this validator cannot evaluate; the check is skipped.
    Unknown,
}

const IND_CAN: &[&str] = &["IND", "CAN"];

static FORM_TYPE_CONDITION: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"form ?type ?= ?([a-z0-9]+)").ok());

/// Interprets the workbook's rule / condition prose. `default` is what an
/// absent or condition-free text means: `Always` for `X (error)` /
/// `X (warning)` rows, `Unknown` for `Conditional` rows (whose text is
/// *always* a condition, even when it is just "Conditional Warning").
fn parse_condition(text: Option<&str>, default: Condition) -> Condition {
    let Some(text) = text else {
        return default;
    };
    let norm: String = text
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    // "Warning if Code is missing; Error if Coded incorrectly." is the
    // workbook's way of saying "required at warning level".
    if norm.contains("if code is missing") {
        return Condition::Always;
    }
    if norm.contains("not [ind|can]") || norm.contains("if not ind or can") {
        return Condition::EntityNotIn(IND_CAN);
    }
    if norm.contains("[ind|can]")
        || norm.contains("if ind, or can")
        || norm.contains("entity=ind or can")
    {
        return Condition::EntityIn(IND_CAN);
    }
    if norm.contains("if not individual") {
        return Condition::EntityNotIn(&["IND"]);
    }
    if norm.contains("if not organization") {
        return Condition::EntityNotIn(&["ORG"]);
    }
    if norm.contains("ccm, pac, or pty") {
        return Condition::EntityIn(&["CCM", "PAC", "PTY"]);
    }
    if norm.contains("x(warning) if ind") {
        return Condition::EntityIn(&["IND"]);
    }
    if let Some(re) = FORM_TYPE_CONDITION.as_ref()
        && let Some(cap) = re.captures(&norm)
        && let Some(token) = cap.get(1)
    {
        return Condition::FormTypeIs(token.as_str().to_ascii_uppercase());
    }
    if norm.contains("report code=[12?|30?]") {
        return Condition::ReportCodeStartsWith(&["12", "30"]);
    }
    if norm.contains("report code=12[?]") {
        return Condition::ReportCodeStartsWith(&["12"]);
    }
    if norm
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| w == "if")
        || norm.contains("depends")
        || norm.contains("required for")
        || norm.contains("see rule ref")
    {
        return Condition::Unknown;
    }
    default
}

// ---------------------------------------------------------------------------
// The checker
// ---------------------------------------------------------------------------

struct Checker<'f> {
    filing: &'f Filing,
    findings: Vec<Finding>,
}

impl Checker<'_> {
    fn push(
        &mut self,
        rule: Rule,
        line_no: u64,
        form_type: &str,
        field: Option<&'static str>,
        message: String,
    ) {
        self.findings
            .push(Finding::new(rule, line_no, form_type, field, message));
    }

    fn push_line(
        &mut self,
        rule: Rule,
        line: &ParsedLine,
        field: Option<&'static str>,
        message: String,
    ) {
        self.push(rule, line.line_no, &line.raw_form_type, field, message);
    }

    // -- Structure ----------------------------------------------------------

    fn check_header(&mut self) {
        let filing = self.filing;
        let h = &filing.header;
        const HDR: &str = "HDR";

        if !h.record_type.eq_ignore_ascii_case("HDR") {
            self.push(
                Rule::HeaderFirst,
                1,
                HDR,
                Some("record_type"),
                format!(
                    "HDR record must be First in File (found '{}')",
                    h.record_type
                ),
            );
        }
        if !h.ef_type.eq_ignore_ascii_case("FEC") {
            self.push(
                Rule::FilingTypeFec,
                1,
                HDR,
                Some("ef_type"),
                format!(
                    "Filing must be an \"FEC\" Type of filing (found '{}')",
                    h.ef_type
                ),
            );
        }
        if let Some(current) = bundled_version()
            && filing.version != current
        {
            self.push(
                Rule::CurrentFormat,
                1,
                HDR,
                Some("fec_version"),
                format!(
                    "Filing must be in the current FEC format (found {}, current is {})",
                    h.fec_version_raw, BUNDLED_SPEC_VERSION
                ),
            );
        }
        if filing.is_amendment {
            if filing.amends_filing.is_none() {
                self.push(
                    Rule::AmendmentNeedsOriginalId,
                    1,
                    HDR,
                    Some("report_id"),
                    format!(
                        "Amended filing must have an ID of the \"Original\" (expected FEC-<filing number>, found '{}')",
                        h.report_id
                    ),
                );
            }
            if h.amendment_number().is_none() {
                self.push(
                    Rule::AmendmentNeedsNumber,
                    1,
                    HDR,
                    Some("report_number"),
                    format!(
                        "Amended filing must have an \"Amendment Number\" (found '{}')",
                        h.report_number
                    ),
                );
            }
        } else if !h.report_id.is_empty() || has_amendment_number(h) {
            self.push(
                Rule::HeaderInconsistentWithAmendmentStatus,
                1,
                HDR,
                Some("report_id"),
                format!(
                    "Header (HDR) inconsistent with Orig/Amend status: {} is not an amendment but the header carries report id '{}' and amendment number '{}'",
                    filing.raw_form_type, h.report_id, h.report_number
                ),
            );
        }
    }

    fn check_cover(&mut self) {
        let filing = self.filing;
        let cover = &filing.summary;

        if !is_top_level_form(cover.table()) {
            self.push_line(
                Rule::CoverSecond,
                cover,
                Some("form_type"),
                format!(
                    "\"Cover\" (eg. F3A, F3XN, ...) must be 2nd in File (found '{}', a {} record)",
                    filing.raw_form_type,
                    cover.table()
                ),
            );
        }

        if let Some(raw) = cover.get("filer_committee_id_number") {
            let id = effective_value(raw);
            // A blank ID is reported by the required-field check.
            if !id.is_empty() && !is_fec_id(id) {
                self.push_line(
                    Rule::FilerIdFormat,
                    cover,
                    Some("filer_committee_id_number"),
                    format!("ID# {id} NOT Correct FEC ID# Format"),
                );
            }
        }

        self.check_fields(cover);
    }

    fn check_body(&mut self) {
        let filing = self.filing;
        let cover_id = filing
            .summary
            .get("filer_committee_id_number")
            .map(effective_value)
            .unwrap_or("");

        for line in &filing.lines {
            if is_top_level_form(line.table()) {
                self.push_line(
                    Rule::MultipleForms,
                    line,
                    Some("form_type"),
                    format!(
                        "Multi-Form Filings are NOT Allowed ({} record in the body of a {} filing)",
                        line.raw_form_type, filing.raw_form_type
                    ),
                );
            } else if let Some(spec) = line.table().spec("form_type")
                && !spec.forms.is_empty()
                && !spec
                    .forms
                    .iter()
                    .any(|f| f.eq_ignore_ascii_case(&filing.base_form_type))
            {
                self.push_line(
                    Rule::ScheduleNotAllowedWithForm,
                    line,
                    Some("form_type"),
                    format!(
                        "Schedule does not belong with Form {} ({} is filed with {})",
                        filing.base_form_type,
                        line.table(),
                        spec.forms.join(", ")
                    ),
                );
            }

            if let Some(raw) = line.get("filer_committee_id_number") {
                let id = effective_value(raw);
                if !id.is_empty() && !cover_id.is_empty() && id != cover_id {
                    self.push_line(
                        Rule::FilerIdMismatch,
                        line,
                        Some("filer_committee_id_number"),
                        format!("ID# {id} NOT SAME AS Cover Page ID# {cover_id}"),
                    );
                }
            }

            self.check_fields(line);
        }
    }

    // -- Per-field ----------------------------------------------------------

    /// Applies every `FieldSpec` of the line's table to the line. Tables the
    /// bundled spec no longer documents (historical forms, Schedule I) have
    /// no specs and are skipped; fields a spec names but this version's
    /// layout lacks are skipped too.
    fn check_fields(&mut self, line: &ParsedLine) {
        for spec in line.table().specs() {
            let Some(name) = spec.canonical else {
                continue;
            };
            let Some(raw) = line.get(name) else {
                continue;
            };
            let value = effective_value(raw);
            if value.is_empty() {
                self.check_required(line, spec, name);
                continue;
            }

            if let Some(inner) = quoted_inner(raw.trim())
                && inner.contains('"')
            {
                self.push_line(
                    Rule::EmbeddedDoubleQuote,
                    line,
                    Some(name),
                    format!(
                        "Embedded double-quotes (\") not allowed inside a quoted {}",
                        spec.description
                    ),
                );
            }

            // A date of the wrong length is reported once, as BadDateFormat.
            let len = value.chars().count();
            if !is_date_field(spec)
                && let Some(max) = spec.max_len
                && len > usize::from(max)
            {
                self.push_line(
                    Rule::FieldTooLong,
                    line,
                    Some(name),
                    format!(
                        "{} exceeds maximum length of {max} ({len} characters)",
                        spec.description
                    ),
                );
            }

            self.check_characters(line, name, value);

            match spec.kind {
                FieldKind::Numeric if is_date_field(spec) => self.check_date(line, name, value),
                FieldKind::Numeric => {
                    if !value.bytes().all(|b| b.is_ascii_digit()) {
                        self.push_line(
                            Rule::NonNumeric,
                            line,
                            Some(name),
                            format!(
                                "Non-numeric data in Numeric Field {}: '{value}'",
                                spec.description
                            ),
                        );
                    }
                }
                FieldKind::Amount => {
                    if parse_money(value).is_none() {
                        self.push_line(
                            Rule::InvalidAmount,
                            line,
                            Some(name),
                            format!(
                                "Invalid Amount format: '{value}' in {} (digits with an optional sign and up to two decimals; no $ or commas)",
                                spec.description
                            ),
                        );
                    }
                }
                FieldKind::Alpha | FieldKind::AlphaNumeric | FieldKind::Unknown => {}
            }

            match name {
                "entity_type" => {
                    if !ENTITY_TYPES.iter().any(|e| e.eq_ignore_ascii_case(value)) {
                        self.push_line(
                            Rule::InvalidEntityType,
                            line,
                            Some(name),
                            format!(
                                "Entity Type [{value}] is not an acceptable value (valid: {})",
                                ENTITY_TYPES.join(", ")
                            ),
                        );
                    }
                }
                "support_oppose_code" => {
                    if !(value.eq_ignore_ascii_case("S") || value.eq_ignore_ascii_case("O")) {
                        self.push_line(
                            Rule::InvalidSupportOpposeCode,
                            line,
                            Some(name),
                            format!("Sup/Opp Code \"{value}\" Invalid (Valid Codes: S, O)"),
                        );
                    }
                }
                _ => {
                    if !spec.allowed_values.is_empty()
                        && !spec
                            .allowed_values
                            .iter()
                            .any(|a| a.eq_ignore_ascii_case(value))
                    {
                        self.push_line(
                            Rule::InvalidAllowedValue,
                            line,
                            Some(name),
                            format!(
                                "Value \"{value}\" is Invalid for {} (valid: {})",
                                spec.description,
                                spec.allowed_values.join(", ")
                            ),
                        );
                    }
                }
            }

            // The filer ID's format is FilerIdFormat's job (on the cover) and
            // FilerIdMismatch's (in the body); do not report it twice.
            if name != "filer_committee_id_number"
                && let Some(pattern) = spec.pattern
                && let Some(re) = PATTERNS.get(pattern)
                && !re.is_match(value)
            {
                self.push_line(
                    Rule::PatternMismatch,
                    line,
                    Some(name),
                    format!(
                        "Value \"{value}\" does not match the required format for {} ({pattern})",
                        spec.description
                    ),
                );
            }

            if is_state_field(spec) && !STATE_CODES.iter().any(|s| s.eq_ignore_ascii_case(value)) {
                self.push_line(
                    Rule::InvalidStateCode,
                    line,
                    Some(name),
                    format!("{value} not a valid 2-character USPS State Code"),
                );
            }
        }
    }

    fn check_required(&mut self, line: &ParsedLine, spec: &FieldSpec, name: &'static str) {
        let (rule, mut condition) = match spec.required {
            Requirement::None => return,
            Requirement::Error => (
                Rule::RequiredFieldEmpty,
                parse_condition(spec.rule, Condition::Always),
            ),
            Requirement::Warning => (
                Rule::RecommendedFieldEmpty,
                parse_condition(spec.rule, Condition::Always),
            ),
            Requirement::Conditional(text) => (
                Rule::ConditionallyRequiredFieldEmpty,
                parse_condition(Some(text), Condition::Unknown),
            ),
        };
        if condition == Condition::Always
            && let Some(implied) = implied_condition(line, name)
        {
            condition = implied;
        }
        if self.condition_holds(line, &condition) != Some(true) {
            return;
        }
        let message = match rule {
            Rule::RequiredFieldEmpty => {
                format!("{} is Required, but field is Empty", spec.description)
            }
            Rule::RecommendedFieldEmpty => format!("{} is Missing", spec.description),
            _ => format!(
                "Conditionally Required field is Empty: {} ({})",
                spec.description,
                match spec.required {
                    Requirement::Conditional(text) => text.trim(),
                    _ => "",
                }
            ),
        };
        let mut finding =
            Finding::new(rule, line.line_no, &line.raw_form_type, Some(name), message);
        // The workbook's requirement levels describe the current format;
        // earlier formats demonstrably differed (spec 3.x-5.x filings with
        // blank entity types were accepted). Report, but do not reject.
        if rule == Rule::RequiredFieldEmpty && !self.current_format() {
            finding.severity = Severity::Warning;
        }
        self.findings.push(finding);
    }

    /// Whether the filing is in the bundled (current) spec version.
    fn current_format(&self) -> bool {
        bundled_version().is_some_and(|v| v == self.filing.version)
    }

    /// `Some(true)` if the condition holds for this line, `Some(false)` if
    /// it does not, `None` if it cannot be evaluated (unknown prose, or the
    /// line's layout lacks the field the condition refers to).
    fn condition_holds(&self, line: &ParsedLine, condition: &Condition) -> Option<bool> {
        let code = |field: &str| {
            line.get(field)
                .map(effective_value)
                .map(str::to_ascii_uppercase)
        };
        match condition {
            Condition::Always => Some(true),
            Condition::Unknown => None,
            Condition::EntityIn(set) => {
                let entity = code("entity_type")?;
                Some(set.contains(&entity.as_str()))
            }
            Condition::EntityNotIn(set) => {
                let entity = code("entity_type")?;
                if entity.is_empty() {
                    // A blank entity type is its own finding; do not guess.
                    return None;
                }
                Some(!set.contains(&entity.as_str()))
            }
            Condition::FormTypeIs(token) => Some(self.filing.raw_form_type == *token),
            Condition::ReportCodeStartsWith(prefixes) => {
                let report_code = code("report_code")?;
                Some(prefixes.iter().any(|p| report_code.starts_with(p)))
            }
        }
    }

    fn check_characters(&mut self, line: &ParsedLine, name: &'static str, value: &str) {
        let mut bad: Vec<char> = Vec::new();
        for c in value.chars() {
            if !is_legal_char(c) && !bad.contains(&c) {
                bad.push(c);
            }
        }
        if !bad.is_empty() {
            self.push_line(
                Rule::IllegalCharacter,
                line,
                Some(name),
                format!(
                    "Illegal character(s) found in text field: {}",
                    describe_chars(&bad)
                ),
            );
        }
    }

    fn check_date(&mut self, line: &ParsedLine, name: &'static str, value: &str) {
        if value.len() != 8 || !value.bytes().all(|b| b.is_ascii_digit()) {
            self.push_line(
                Rule::BadDateFormat,
                line,
                Some(name),
                format!("Bad Date - {value} not YYYYMMDD format"),
            );
            return;
        }
        match parse_fec_date(value) {
            None => self.push_line(
                Rule::NotARealDate,
                line,
                Some(name),
                format!("{value} is not a Real Date"),
            ),
            Some(date) => {
                use chrono::Datelike;
                if !DATE_YEAR_RANGE.contains(&date.year()) {
                    self.push_line(
                        Rule::DateOutOfRange,
                        line,
                        Some(name),
                        format!(
                            "{value} is outside range of {}-{}",
                            DATE_YEAR_RANGE.start(),
                            DATE_YEAR_RANGE.end()
                        ),
                    );
                }
            }
        }
    }

    // -- Cross-line ---------------------------------------------------------

    fn check_transaction_ids(&mut self) {
        let filing = self.filing;
        // Upper-cased ID -> first line that used it.
        let mut seen: HashMap<String, u64> = HashMap::new();
        for line in &filing.lines {
            let Some(id) = transaction_id(line) else {
                continue;
            };
            let key = id.to_ascii_uppercase();
            match seen.get(&key) {
                Some(&first) => {
                    let field = if line.get("transaction_id").is_some() {
                        "transaction_id"
                    } else {
                        "transaction_id_number"
                    };
                    self.push_line(
                        Rule::DuplicateTransactionId,
                        line,
                        Some(field),
                        format!(
                            "Tran ID {id} is NOT UNIQUE - This one is same as other(s) (first used on line {first})"
                        ),
                    );
                }
                None => {
                    seen.insert(key, line.line_no);
                }
            }
        }

        for line in &filing.lines {
            let Some(raw) = line.get("back_reference_tran_id_number") else {
                continue;
            };
            let back_ref = effective_value(raw);
            if back_ref.is_empty() {
                continue;
            }
            if !seen.contains_key(&back_ref.to_ascii_uppercase()) {
                self.push_line(
                    Rule::BackReferenceNotFound,
                    line,
                    Some("back_reference_tran_id_number"),
                    format!(
                        "Back-Reference TRAN-ID {back_ref} does not match Sched TRAN-ID (no transaction in this filing has that ID)"
                    ),
                );
            }
        }
    }

    // -- Form 99 ------------------------------------------------------------

    /// The F99 `text` field has no row in the spec workbook (it is the
    /// `[BEGINTEXT]` block, spliced in by the parser), so it gets its own
    /// length and character checks with the FEC's F99-specific rules.
    fn check_f99_text(&mut self) {
        let cover = &self.filing.summary;
        if cover.table() != Table::F99 {
            return;
        }
        let Some(text) = cover.get("text") else {
            return;
        };
        let len = text.chars().count();
        if len > F99_TEXT_MAX_CHARS {
            self.push_line(
                Rule::F99TextTooLong,
                cover,
                Some("text"),
                format!(
                    "Body of text exceeds maximum of 20,000 characters (F99 filings): {len} characters"
                ),
            );
        }
        for (i, text_line) in text.split('\n').enumerate() {
            let mut bad: Vec<char> = Vec::new();
            for c in text_line.chars() {
                if !is_legal_f99_text_char(c) && !bad.contains(&c) {
                    bad.push(c);
                }
            }
            if !bad.is_empty() {
                self.push_line(
                    Rule::F99IllegalCharacter,
                    cover,
                    Some("text"),
                    format!(
                        "Illegal character(s) found in text line #{} (Used for F99's): {}",
                        i + 1,
                        describe_chars(&bad)
                    ),
                );
            }
        }
    }
}

/// `U+2019 (’), U+0007` -- up to three characters, for messages.
fn describe_chars(chars: &[char]) -> String {
    let mut parts: Vec<String> = chars
        .iter()
        .take(3)
        .map(|&c| {
            if c.is_control() {
                format!("U+{:04X}", u32::from(c))
            } else {
                format!("U+{:04X} ({c})", u32::from(c))
            }
        })
        .collect();
    if chars.len() > 3 {
        parts.push(format!("and {} more", chars.len() - 3));
    }
    parts.join(", ")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    fn v85() -> SpecVersion {
        SpecVersion::electronic(8, 5)
    }

    /// A minimal, clean 8.5 F3X with one Schedule A line.
    fn clean_text() -> String {
        [
            "HDR\u{1c}FEC\u{1c}8.5\u{1c}FECfile\u{1c}8.5.1.0(f34)\u{1c}\u{1c}\u{1c}",
            // F3X 8.5 columns: 0 form_type, 1 filer id, 2 committee name, 3 change of
            // address, 4-5 street, 6 city, 7 state, 8 zip, 9 report code, 10 election
            // code, 11 date of election, 12 state of election, 13-14 coverage,
            // 15 qualified, 16-20 treasurer name parts, 21 date signed.
            "F3XN\u{1c}C00123456\u{1c}COMMITTEE NAME\u{1c}\u{1c}123 MAIN ST\u{1c}\u{1c}ANYTOWN\u{1c}VA\u{1c}22201\u{1c}Q1\u{1c}\u{1c}\u{1c}\u{1c}20260101\u{1c}20260331\u{1c}\u{1c}SMITH\u{1c}JANE\u{1c}\u{1c}\u{1c}\u{1c}20260415",
            "SA11AI\u{1c}C00123456\u{1c}A1\u{1c}\u{1c}\u{1c}IND\u{1c}\u{1c}DOE\u{1c}JOHN\u{1c}\u{1c}\u{1c}\u{1c}1 ELM ST\u{1c}\u{1c}ANYTOWN\u{1c}VA\u{1c}22201\u{1c}P2026\u{1c}\u{1c}20260201\u{1c}250.00\u{1c}250.00\u{1c}\u{1c}ACME\u{1c}ENGINEER",
        ]
        .join("\n")
    }

    fn parse(text: &str) -> Filing {
        Filing::parse(text).unwrap_or_else(|e| panic!("{e}"))
    }

    fn rules(v: &Validation) -> Vec<Rule> {
        v.findings.iter().map(|f| f.rule).collect()
    }

    fn has(v: &Validation, rule: Rule) -> bool {
        v.findings.iter().any(|f| f.rule == rule)
    }

    fn sched_a(pairs: &[(&str, &str)]) -> ParsedLine {
        let mut all = vec![
            ("form_type", "SA11AI"),
            ("filer_committee_id_number", "C00123456"),
            ("transaction_id", "T1"),
            ("entity_type", "IND"),
            ("contributor_last_name", "DOE"),
            ("contributor_first_name", "JOHN"),
            ("contributor_street_1", "1 ELM ST"),
            ("contributor_city", "ANYTOWN"),
            ("contributor_state", "VA"),
            ("contributor_zip_code", "22201"),
            ("contribution_date", "20260201"),
            ("contribution_amount", "250.00"),
            ("contribution_aggregate", "250.00"),
        ];
        for (k, v) in pairs {
            if let Some(slot) = all.iter_mut().find(|(name, _)| name == k) {
                slot.1 = v;
            } else {
                all.push((k, v));
            }
        }
        ParsedLine::from_pairs(Table::SchA, v85(), 3, all).unwrap_or_else(|e| panic!("{e}"))
    }

    fn with_lines(lines: Vec<ParsedLine>) -> Filing {
        let mut filing = parse(&clean_text());
        filing.lines = lines;
        filing
    }

    // -- Enum plumbing ------------------------------------------------------

    #[test]
    fn every_rule_has_a_message_and_a_stable_name() {
        for rule in Rule::iter() {
            assert!(!rule.fec_message().is_empty(), "{rule:?}");
            let name = rule.to_string();
            assert_eq!(name.parse::<Rule>().ok(), Some(rule), "{name}");
            assert!(
                name.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "{name}"
            );
        }
        assert_eq!(Rule::F99TextTooLong.to_string(), "f99_text_too_long");
        assert!(Severity::Warning < Severity::Error);
        assert_eq!("error".parse::<Severity>().ok(), Some(Severity::Error));
    }

    #[test]
    fn every_spec_pattern_compiles() {
        let mut expected = std::collections::HashSet::new();
        for table in Table::ALL {
            for spec in table.specs() {
                if let Some(p) = spec.pattern {
                    expected.insert(p);
                }
            }
        }
        assert!(!expected.is_empty());
        for p in expected {
            assert!(PATTERNS.contains_key(p), "pattern failed to compile: {p}");
        }
        assert!(FORM_TYPE_CONDITION.is_some());
        assert!(bundled_version().is_some());
    }

    // -- Value helpers ------------------------------------------------------

    #[test]
    fn effective_value_unwraps_one_pair_of_quotes() {
        assert_eq!(effective_value("\"SMITH\""), "SMITH");
        assert_eq!(effective_value("\"\""), "");
        assert_eq!(effective_value("\"  \""), "");
        assert_eq!(effective_value(" \" x \" "), "x");
        assert_eq!(effective_value("\""), "\"");
        assert_eq!(effective_value("say \"hi\""), "say \"hi\"");
        assert_eq!(effective_value("\"\"\""), "\"");
        assert_eq!(effective_value("plain"), "plain");
    }

    #[test]
    fn fec_id_format_matches_the_spec_regex() {
        let re = Regex::new(r"^(?:[PC][0-9]{8}|[HS][0-9]{1}[A-Z]{2}[0-9]{5})$").unwrap();
        let cases = [
            "C00123456",
            "P80001571",
            "H2OH17158",
            "S4NY00123",
            "c00123456",
            "C0012345",
            "C001234567",
            "H2oh17158",
            "H2OH1715A",
            "X00123456",
            "",
            "C0012345é",
        ];
        for id in cases {
            assert_eq!(is_fec_id(id), re.is_match(id), "{id}");
        }
    }

    #[test]
    fn legal_character_set() {
        for c in [
            ' ', 'A', '~', '\u{a0}', '§', '\u{ad}', '’', '–', '€', '\u{81}',
        ] {
            assert!(is_legal_char(c), "{c:?} should be legal");
        }
        for c in [
            '\u{7f}', '\u{9d}', 'ž', 'Ÿ', 'é', '\t', '\n', '\u{fffd}', '中', '\u{1c}',
        ] {
            assert!(!is_legal_char(c), "{c:?} should be illegal");
        }
        assert!(is_legal_f99_text_char('\t'));
        assert!(is_legal_f99_text_char('\n'));
        assert!(!is_legal_f99_text_char('\u{7f}'));
    }

    #[test]
    fn condition_prose_is_interpreted() {
        use Condition::*;
        let p = |t: &str| parse_condition(Some(t), Always);
        assert_eq!(p("Required if [IND|CAN]"), EntityIn(IND_CAN));
        assert_eq!(p("Required if NOT [IND|CAN]"), EntityNotIn(IND_CAN));
        assert_eq!(p("Required if IND, or CAN"), EntityIn(IND_CAN));
        assert_eq!(
            p("Required if CCM, PAC, or PTY"),
            EntityIn(&["CCM", "PAC", "PTY"])
        );
        assert_eq!(
            p("Warning if Code is missing;\nError if Coded incorrectly."),
            Always
        );
        assert_eq!(p("= 11ai + 11aii"), Always);
        assert_eq!(p("Required if PDF is attached"), Unknown);
        assert_eq!(p("X (error) If New Orig."), Unknown);
        assert_eq!(
            p("X (error if Form Type=F24A)"),
            FormTypeIs("F24A".to_string())
        );
        assert_eq!(
            p("Required if FormType=F13A"),
            FormTypeIs("F13A".to_string())
        );
        assert_eq!(
            p("X (warn if REPORT CODE=[12?|30?])"),
            ReportCodeStartsWith(&["12", "30"])
        );
        assert_eq!(
            p("X (warn if REPORT CODE=12[?])"),
            ReportCodeStartsWith(&["12"])
        );
        assert_eq!(p("X (error) If not Individual"), EntityNotIn(&["IND"]));
        assert_eq!(p("X(Warning) if IND"), EntityIn(&["IND"]));
        assert_eq!(parse_condition(None, Always), Always);
        assert_eq!(
            parse_condition(Some("Conditional Warning"), Unknown),
            Unknown
        );
        assert_eq!(
            parse_condition(Some("X(err if 51st Contrib)"), Unknown),
            Unknown
        );
    }

    // -- Clean filing -------------------------------------------------------

    #[test]
    fn clean_synthetic_filing_is_acceptable() {
        let v = parse(&clean_text()).validate();
        assert!(v.is_acceptable(), "{v}");
        assert!(v.is_empty(), "{v}");
        assert_eq!(v.error_count(), 0);
        assert_eq!(v.warning_count(), 0);
        assert_eq!(v.iter().count(), 0);
    }

    // -- Header rules -------------------------------------------------------

    #[test]
    fn header_first_and_filing_type_fec() {
        let text = clean_text().replacen("HDR\u{1c}FEC", "XDR\u{1c}XYZ", 1);
        let v = parse(&text).validate();
        assert!(has(&v, Rule::HeaderFirst), "{v}");
        assert!(has(&v, Rule::FilingTypeFec), "{v}");
        assert!(!v.is_acceptable());
        let first = &v.findings[0];
        assert_eq!(first.line_no, 1);
        assert_eq!(first.form_type, "HDR");
        assert_eq!(first.severity, Severity::Error);
        assert!(
            first
                .to_string()
                .starts_with("ERROR line 1 HDR record_type: ")
        );
    }

    #[test]
    fn old_format_is_a_warning_not_an_error() {
        let text = "HDR\u{1c}FEC\u{1c}8.4\u{1c}X\u{1c}1\nF3XN\u{1c}C00123456";
        let v = parse(text).validate();
        assert!(has(&v, Rule::CurrentFormat), "{v}");
        assert_eq!(Rule::CurrentFormat.severity(), Severity::Warning);
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::CurrentFormat)
            .unwrap();
        assert!(
            f.message.contains("8.4") && f.message.contains("8.5"),
            "{f}"
        );
    }

    #[test]
    fn amendment_needs_original_id_and_number() {
        let text = clean_text().replacen("F3XN", "F3XA", 1);
        let v = parse(&text).validate();
        assert!(has(&v, Rule::AmendmentNeedsOriginalId), "{v}");
        assert!(has(&v, Rule::AmendmentNeedsNumber), "{v}");
        assert!(!has(&v, Rule::HeaderInconsistentWithAmendmentStatus));

        let ok = text.replacen(
            "8.5.1.0(f34)\u{1c}\u{1c}\u{1c}",
            "8.5.1.0(f34)\u{1c}FEC-1234567\u{1c}1\u{1c}",
            1,
        );
        let v = parse(&ok).validate();
        assert!(v.is_acceptable(), "{v}");
        assert!(v.is_empty(), "{v}");
    }

    #[test]
    fn non_amendment_with_report_id_is_inconsistent_warning() {
        let text = clean_text().replacen(
            "8.5.1.0(f34)\u{1c}\u{1c}\u{1c}",
            "8.5.1.0(f34)\u{1c}FEC-1234567\u{1c}1\u{1c}",
            1,
        );
        let v = parse(&text).validate();
        assert_eq!(rules(&v), [Rule::HeaderInconsistentWithAmendmentStatus]);
        assert!(v.is_acceptable());
        // FECfile writes amendment number 0 on originals; the FEC accepts it.
        let text = clean_text().replacen(
            "8.5.1.0(f34)\u{1c}\u{1c}\u{1c}",
            "8.5.1.0(f34)\u{1c}\u{1c}0\u{1c}",
            1,
        );
        assert!(parse(&text).validate().is_empty());
    }

    #[test]
    fn required_field_empty_is_demoted_on_superseded_formats() {
        // Same Schedule A line with no entity type, filed as 8.5 vs 8.4.
        let make = |version: &str| {
            let mut filing = parse(&clean_text().replacen(
                "\u{1c}8.5\u{1c}",
                &format!("\u{1c}{version}\u{1c}"),
                1,
            ));
            filing.lines = vec![sched_a(&[("entity_type", "")])];
            filing
        };
        let current = make("8.5").validate();
        let f = current
            .findings
            .iter()
            .find(|f| f.rule == Rule::RequiredFieldEmpty && f.field == Some("entity_type"))
            .unwrap_or_else(|| panic!("{current}"));
        assert_eq!(f.severity, Severity::Error);

        let old = make("8.4").validate();
        let f = old
            .findings
            .iter()
            .find(|f| f.rule == Rule::RequiredFieldEmpty && f.field == Some("entity_type"))
            .unwrap_or_else(|| panic!("{old}"));
        assert_eq!(f.severity, Severity::Warning);
        assert!(old.is_acceptable(), "{old}");
    }

    #[test]
    fn form9_filer_name_requirements_follow_entity_type() {
        let cover = |pairs: &[(&str, &str)]| {
            let mut filing = parse(&clean_text());
            filing.lines.clear();
            filing.summary =
                ParsedLine::from_pairs(Table::F9, v85(), 2, pairs.iter().copied()).unwrap();
            filing
        };
        let base = [
            ("form_type", "F9N"),
            ("filer_committee_id_number", "C30001655"),
        ];
        let mut org = base.to_vec();
        org.extend([("entity_type", "PAC"), ("organization_name", "CROSSROADS")]);
        let v = cover(&org).validate();
        let fields: Vec<_> = v
            .findings
            .iter()
            .filter(|f| f.rule == Rule::RequiredFieldEmpty)
            .map(|f| f.field)
            .collect();
        assert!(!fields.contains(&Some("individual_last_name")), "{v}");
        assert!(!fields.contains(&Some("organization_name")), "{v}");

        let mut ind = base.to_vec();
        ind.extend([("entity_type", "IND"), ("individual_last_name", "DOE")]);
        let v = cover(&ind).validate();
        let fields: Vec<_> = v
            .findings
            .iter()
            .filter(|f| f.rule == Rule::RequiredFieldEmpty)
            .map(|f| f.field)
            .collect();
        assert!(fields.contains(&Some("individual_first_name")), "{v}");
        assert!(!fields.contains(&Some("organization_name")), "{v}");
    }

    #[test]
    fn state_check_needs_a_two_character_field() {
        // Form 6's workbook sheet has `Edit: ST` shifted onto CITY.
        let city = Table::F6.spec("city").unwrap();
        assert!(city.rule.is_some_and(|r| r.starts_with("Edit: ST")));
        assert!(!is_state_field(city));
        assert!(is_state_field(
            Table::SchA.spec("contributor_state").unwrap()
        ));
        assert!(is_state_field(Table::F3X.spec("state").unwrap()));
    }

    // -- Cover / body structure --------------------------------------------

    #[test]
    fn schedule_as_cover_is_cover_second() {
        let text = "HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nSA11AI\u{1c}C00123456\u{1c}T1";
        let v = parse(text).validate();
        assert!(has(&v, Rule::CoverSecond), "{v}");
        assert!(!v.is_acceptable());
    }

    #[test]
    fn filer_id_format_and_mismatch() {
        let text = clean_text().replacen("F3XN\u{1c}C00123456", "F3XN\u{1c}c00123456", 1);
        let v = parse(&text).validate();
        assert!(has(&v, Rule::FilerIdFormat), "{v}");
        assert!(has(&v, Rule::FilerIdMismatch), "{v}");
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::FilerIdMismatch)
            .unwrap();
        assert_eq!(f.line_no, 3);
        assert_eq!(f.form_type, "SA11AI");
        assert_eq!(f.field, Some("filer_committee_id_number"));
        assert!(f.message.contains("C00123456") && f.message.contains("c00123456"));
    }

    #[test]
    fn candidate_ids_are_valid_filer_ids() {
        let text = clean_text()
            .replacen("F3XN\u{1c}C00123456", "F3XN\u{1c}H2OH17158", 1)
            .replacen("SA11AI\u{1c}C00123456", "SA11AI\u{1c}H2OH17158", 1);
        let v = parse(&text).validate();
        assert!(!has(&v, Rule::FilerIdFormat), "{v}");
        assert!(!has(&v, Rule::FilerIdMismatch), "{v}");
    }

    #[test]
    fn second_form_record_is_multiple_forms() {
        let text = format!("{}\nF3XN\u{1c}C00123456\u{1c}AGAIN", clean_text());
        let v = parse(&text).validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::MultipleForms)
            .unwrap();
        assert_eq!(f.line_no, 4);
        assert_eq!(f.severity, Severity::Error);
    }

    #[test]
    fn sub_forms_are_not_multiple_forms() {
        for (table, token) in [
            (Table::F3S, "F3S"),
            (Table::F3Z, "F3ZT"),
            (Table::F1S, "F1S"),
            (Table::F56, "F56"),
            (Table::F65, "F65"),
            (Table::H4, "H4"),
            (Table::Text, "TEXT"),
        ] {
            assert!(!is_top_level_form(table), "{token}");
        }
        for table in [
            Table::F3X,
            Table::F3,
            Table::F3P,
            Table::F99,
            Table::F1,
            Table::F24,
        ] {
            assert!(is_top_level_form(table));
        }
    }

    #[test]
    fn schedule_a_does_not_belong_with_f24() {
        let text = clean_text().replacen("F3XN\u{1c}C00123456", "F24N\u{1c}C00123456", 1);
        let v = parse(&text).validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::ScheduleNotAllowedWithForm)
            .unwrap_or_else(|| panic!("{v}"));
        assert_eq!(f.line_no, 3);
        assert!(f.message.contains("F24"), "{f}");
    }

    // -- Per-field rules ----------------------------------------------------

    #[test]
    fn required_field_empty_respects_entity_type() {
        // IND without a first name: FEC failing #1.
        let v = with_lines(vec![sched_a(&[("contributor_first_name", "")])]).validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::RequiredFieldEmpty)
            .unwrap();
        assert_eq!(f.field, Some("contributor_first_name"));
        assert_eq!(f.severity, Severity::Error);
        assert!(
            f.message
                .contains("CONTRIBUTOR FIRST NAME is Required, but field is Empty")
        );
        // ...but an organization needs no first name, and needs an org name.
        let v = with_lines(vec![sched_a(&[
            ("entity_type", "ORG"),
            ("contributor_first_name", ""),
            ("contributor_last_name", ""),
        ])])
        .validate();
        let fields: Vec<_> = v
            .findings
            .iter()
            .filter(|f| f.rule == Rule::RequiredFieldEmpty)
            .map(|f| f.field)
            .collect();
        assert_eq!(fields, [Some("contributor_organization_name")], "{v}");
        // Unconditionally required: the transaction id.
        let v = with_lines(vec![sched_a(&[("transaction_id", "")])]).validate();
        assert!(
            v.findings
                .iter()
                .any(|f| f.rule == Rule::RequiredFieldEmpty && f.field == Some("transaction_id")),
            "{v}"
        );
    }

    #[test]
    fn recommended_field_empty_is_a_warning() {
        let v = with_lines(vec![sched_a(&[("contributor_street_1", "")])]).validate();
        assert_eq!(rules(&v), [Rule::RecommendedFieldEmpty]);
        assert!(v.is_acceptable());
        assert!(
            v.findings[0]
                .message
                .contains("CONTRIBUTOR STREET 1 is Missing")
        );
    }

    #[test]
    fn conditionally_required_field_empty_when_condition_evaluates_true() {
        // F24A needs an original amendment date.
        let text = clean_text()
            .replacen("F3XN\u{1c}C00123456", "F24A\u{1c}C00123456", 1)
            .replacen(
                "8.5.1.0(f34)\u{1c}\u{1c}\u{1c}",
                "8.5.1.0(f34)\u{1c}FEC-1234567\u{1c}1\u{1c}",
                1,
            );
        let filing = parse(&text);
        let mut filing = filing;
        filing.lines.clear();
        let v = filing.validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::ConditionallyRequiredFieldEmpty)
            .unwrap_or_else(|| panic!("{v}"));
        assert_eq!(f.field, Some("original_amendment_date"));
        assert_eq!(f.severity, Severity::Warning);
        // An F3X pre-election report (12P) should carry an election date.
        let v = parse(&clean_text().replacen("\u{1c}Q1\u{1c}", "\u{1c}12P\u{1c}", 1)).validate();
        assert!(
            v.findings
                .iter()
                .any(|f| f.rule == Rule::ConditionallyRequiredFieldEmpty
                    && f.field == Some("date_of_election")),
            "{v}"
        );
        // Quarterly: no such warning.
        let v = parse(&clean_text()).validate();
        assert!(!has(&v, Rule::ConditionallyRequiredFieldEmpty));
    }

    #[test]
    fn field_too_long() {
        let long = "X".repeat(31);
        let v = with_lines(vec![sched_a(&[("contributor_last_name", &long)])]).validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::FieldTooLong)
            .unwrap();
        assert!(
            f.message
                .contains("exceeds maximum length of 30 (31 characters)"),
            "{f}"
        );
        // Length is in characters, not bytes.
        let thirty_a = "\u{a7}".repeat(30);
        let v = with_lines(vec![sched_a(&[("contributor_last_name", &thirty_a)])]).validate();
        assert!(!has(&v, Rule::FieldTooLong), "{v}");
        // Wrapping quotes do not count.
        let quoted = format!("\"{}\"", "X".repeat(30));
        let v = with_lines(vec![sched_a(&[("contributor_last_name", &quoted)])]).validate();
        assert!(!has(&v, Rule::FieldTooLong), "{v}");
    }

    #[test]
    fn illegal_character_and_embedded_quote() {
        let v = with_lines(vec![sched_a(&[("contributor_last_name", "DOE\u{7f}")])]).validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::IllegalCharacter)
            .unwrap();
        assert!(f.message.contains("U+007F"), "{f}");
        assert_eq!(f.severity, Severity::Error);

        let v = with_lines(vec![sched_a(&[("contributor_last_name", "O\u{2019}NEIL")])]).validate();
        assert!(!has(&v, Rule::IllegalCharacter), "{v}");

        let v = with_lines(vec![sched_a(&[("contributor_last_name", "\"O\"NEIL\"")])]).validate();
        assert!(has(&v, Rule::EmbeddedDoubleQuote), "{v}");
        // A bare quote in an unquoted field is fine (FEC #30).
        let v = with_lines(vec![sched_a(&[(
            "contributor_first_name",
            "JOHN \"JACK\"",
        )])])
        .validate();
        assert!(!has(&v, Rule::EmbeddedDoubleQuote), "{v}");
        // Fully quoted fields, as some vendors emit, are fine.
        let v = with_lines(vec![sched_a(&[
            ("contributor_last_name", "\"DOE\""),
            ("contributor_organization_name", "\"\""),
        ])])
        .validate();
        assert!(v.is_empty(), "{v}");
    }

    #[test]
    fn dates() {
        let v = with_lines(vec![sched_a(&[("contribution_date", "2026-02-01")])]).validate();
        assert!(has(&v, Rule::BadDateFormat), "{v}");
        assert!(!has(&v, Rule::NonNumeric), "{v}");
        let v = with_lines(vec![sched_a(&[("contribution_date", "20260231")])]).validate();
        assert!(has(&v, Rule::NotARealDate), "{v}");
        let v = with_lines(vec![sched_a(&[("contribution_date", "00000000")])]).validate();
        assert!(has(&v, Rule::NotARealDate), "{v}");
        let v = with_lines(vec![sched_a(&[("contribution_date", "19590101")])]).validate();
        assert_eq!(rules(&v), [Rule::DateOutOfRange]);
        assert!(v.is_acceptable());
        let v = with_lines(vec![sched_a(&[("contribution_date", "21000101")])]).validate();
        assert_eq!(rules(&v), [Rule::DateOutOfRange]);
    }

    #[test]
    fn amounts_and_numerics() {
        for bad in ["$250.00", "1,000", "250.001", "abc", "--5"] {
            let v = with_lines(vec![sched_a(&[("contribution_amount", bad)])]).validate();
            assert!(has(&v, Rule::InvalidAmount), "{bad}: {v}");
        }
        for ok in ["250", "250.5", "-250.00", "+1.00", "0.00"] {
            let v = with_lines(vec![sched_a(&[("contribution_amount", ok)])]).validate();
            assert!(!has(&v, Rule::InvalidAmount), "{ok}: {v}");
        }
        let v = with_lines(vec![sched_a(&[("donor_candidate_district", "1A")])]).validate();
        assert!(has(&v, Rule::NonNumeric), "{v}");
        assert_eq!(Rule::NonNumeric.severity(), Severity::Error);
    }

    #[test]
    fn code_lists_and_patterns_are_warnings() {
        let v = with_lines(vec![sched_a(&[("entity_type", "XYZ")])]).validate();
        // XYZ is also "not IND/CAN", so the organization name becomes required.
        assert!(has(&v, Rule::InvalidEntityType), "{v}");
        assert_eq!(
            v.findings
                .iter()
                .find(|f| f.rule == Rule::InvalidEntityType)
                .map(|f| f.severity),
            Some(Severity::Warning)
        );

        let v = with_lines(vec![sched_a(&[("contributor_state", "XX")])]).validate();
        assert_eq!(rules(&v), [Rule::InvalidStateCode]);
        for ok in ["VA", "DC", "PR", "GU", "AE", "ZZ", "va"] {
            let v = with_lines(vec![sched_a(&[("contributor_state", ok)])]).validate();
            assert!(!has(&v, Rule::InvalidStateCode), "{ok}: {v}");
        }

        let v = with_lines(vec![sched_a(&[("donor_committee_fec_id", "NOT-AN-ID")])]).validate();
        assert!(has(&v, Rule::PatternMismatch), "{v}");
        assert!(v.is_acceptable());

        // F24's 24/48-hour report type is one of the workbook's enumerated lists.
        let mut filing = parse(&clean_text());
        filing.lines.clear();
        filing.summary = ParsedLine::from_pairs(
            Table::F24,
            v85(),
            2,
            [
                ("form_type", "F24N"),
                ("filer_committee_id_number", "C00123456"),
                ("report_type", "72"),
            ],
        )
        .unwrap();
        let v = filing.validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::InvalidAllowedValue)
            .unwrap_or_else(|| panic!("{v}"));
        assert_eq!(f.field, Some("report_type"));
        assert!(f.message.contains("24, 48"), "{f}");
        filing.summary.set("report_type", "48").unwrap();
        assert!(!has(&filing.validate(), Rule::InvalidAllowedValue));
    }

    #[test]
    fn support_oppose_code() {
        let line = ParsedLine::from_pairs(
            Table::SchE,
            v85(),
            3,
            [
                ("form_type", "SE"),
                ("filer_committee_id_number", "C00123456"),
                ("transaction_id_number", "E1"),
                ("support_oppose_code", "X"),
            ],
        )
        .unwrap();
        let v = with_lines(vec![line]).validate();
        assert!(has(&v, Rule::InvalidSupportOpposeCode), "{v}");
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::InvalidSupportOpposeCode)
            .unwrap();
        assert_eq!(f.severity, Severity::Warning);
        assert!(f.message.contains("Sup/Opp Code \"X\" Invalid"));
    }

    // -- Cross-line rules ---------------------------------------------------

    #[test]
    fn duplicate_transaction_ids_are_case_insensitive_and_cross_table() {
        let mut b = sched_a(&[("transaction_id", "abc")]);
        b.line_no = 4;
        let sb = ParsedLine::from_pairs(
            Table::SchB,
            v85(),
            5,
            [
                ("form_type", "SB21B"),
                ("filer_committee_id_number", "C00123456"),
                ("transaction_id_number", "ABC"),
                ("entity_type", "ORG"),
                ("payee_organization_name", "ACME"),
                ("payee_street_1", "1 ELM"),
                ("payee_city", "X"),
                ("payee_state", "VA"),
                ("payee_zip_code", "22201"),
                ("expenditure_date", "20260201"),
                ("expenditure_amount", "10.00"),
                ("expenditure_purpose_descrip", "STUFF"),
            ],
        )
        .unwrap();
        let v = with_lines(vec![sched_a(&[("transaction_id", "ABC")]), b, sb]).validate();
        let dups: Vec<_> = v
            .findings
            .iter()
            .filter(|f| f.rule == Rule::DuplicateTransactionId)
            .collect();
        assert_eq!(dups.len(), 2, "{v}");
        assert_eq!(dups[0].line_no, 4);
        assert_eq!(dups[0].field, Some("transaction_id"));
        assert_eq!(dups[1].line_no, 5);
        assert_eq!(dups[1].field, Some("transaction_id_number"));
        assert!(dups[0].message.contains("first used on line 3"));
    }

    #[test]
    fn back_reference_must_resolve() {
        let mut memo = sched_a(&[
            ("transaction_id", "M1"),
            ("back_reference_tran_id_number", "T1"),
            ("back_reference_sched_name", "SA11AI"),
            ("memo_code", "X"),
        ]);
        memo.line_no = 4;
        let v = with_lines(vec![sched_a(&[]), memo.clone()]).validate();
        assert!(!has(&v, Rule::BackReferenceNotFound), "{v}");

        memo.set("back_reference_tran_id_number", "NOPE").unwrap();
        let v = with_lines(vec![sched_a(&[]), memo]).validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::BackReferenceNotFound)
            .unwrap();
        assert_eq!(f.line_no, 4);
        assert!(f.message.contains("NOPE"));
    }

    // -- F99 ----------------------------------------------------------------

    fn f99(text_block: &str) -> Filing {
        let text = format!(
            "HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF99\u{1c}C00123456\u{1c}CMTE\u{1c}1 ST\u{1c}\u{1c}CITY\u{1c}VA\u{1c}22201\u{1c}DOE\u{1c}JANE\u{1c}\u{1c}\u{1c}\u{1c}20260101\u{1c}MST\u{1c}\u{1c}\n[BEGINTEXT]\n{text_block}\n[ENDTEXT]\n"
        );
        parse(&text)
    }

    #[test]
    fn f99_text_rules() {
        let v = f99("Dear FEC,\n\tAll is well.").validate();
        assert!(v.is_empty(), "{v}");

        let v = f99(&"x".repeat(F99_TEXT_MAX_CHARS + 1)).validate();
        assert_eq!(rules(&v), [Rule::F99TextTooLong]);
        assert!(!v.is_acceptable());

        let v = f99("fine\nbad \u{fffd} here").validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::F99IllegalCharacter)
            .unwrap();
        assert_eq!(f.severity, Severity::Warning);
        assert!(f.message.contains("text line #2"), "{f}");
    }

    // -- Validation container ----------------------------------------------

    #[test]
    fn skipped_lines_become_unrecognized_form_type_warnings() {
        let text = format!("{}\nZZZ\u{1c}C00123456\u{1c}?", clean_text());
        let lenient = Filing::parse_with(&text, &crate::parser::ParseOptions::LENIENT).unwrap();
        let v = lenient.validate();
        assert_eq!(rules(&v), [Rule::UnrecognizedFormType]);
        assert_eq!(v.findings[0].line_no, 4);
        assert_eq!(v.findings[0].form_type, "ZZZ");
        assert!(v.is_acceptable());
    }

    #[test]
    fn validation_display_counts_and_ordering() {
        let mut v = Validation::default();
        v.push(Finding::new(
            Rule::FieldTooLong,
            5,
            "SA11AI",
            Some("x"),
            "too long",
        ));
        v.push(Finding::new(Rule::CurrentFormat, 1, "HDR", None, "old"));
        v.push(Finding::new(
            Rule::NonNumeric,
            5,
            "SA11AI",
            Some("y"),
            "nan",
        ));
        assert_eq!(v.iter().map(|f| f.line_no).collect::<Vec<_>>(), [1, 5, 5]);
        assert_eq!(v.findings[1].field, Some("x"));
        assert_eq!(v.error_count(), 2);
        assert_eq!(v.warning_count(), 1);
        assert_eq!(v.len(), 3);
        assert!(!v.is_acceptable());
        assert_eq!(v.counts_by_rule().get(&Rule::NonNumeric), Some(&1));
        let text = v.to_string();
        assert_eq!(text.lines().count(), 3);
        assert_eq!(text.lines().next(), Some("WARN  line 1 HDR: old"));
        assert_eq!(text.lines().nth(1), Some("ERROR line 5 SA11AI x: too long"));
        assert_eq!((&v).into_iter().count(), 3);
        assert_eq!(v.into_iter().count(), 3);
    }

    #[test]
    fn describe_chars_is_bounded() {
        assert_eq!(describe_chars(&['\u{7f}']), "U+007F");
        assert_eq!(describe_chars(&['é']), "U+00E9 (é)");
        assert_eq!(
            describe_chars(&['a', 'b', 'c', 'd', 'e']),
            "U+0061 (a), U+0062 (b), U+0063 (c), and 2 more"
        );
    }
}
