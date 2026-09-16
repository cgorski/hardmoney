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
//!   format at upload time. Because the bundled workbook describes the
//!   current format only, the rules for which real, accepted 3.x-6.x
//!   filings demonstrably differ ([`Rule::demoted_on_superseded_format`]:
//!   blank required fields, accented Latin-1 letters, one-digit districts,
//!   pre-BCRA H3 event codes) are reported at warning severity on such
//!   filings. Every other check is format-stable (date and amount syntax,
//!   lengths, IDs, transaction-ID integrity) and keeps its severity.
//! * *Leading blanks* (FEC failing message #6/#31) cannot be detected: the
//!   parser trims surrounding ASCII whitespace from every field before the
//!   validator sees it (see [`crate::parser::utils`]).
//! * Characters in a Form 99's free-text block are checked at **warning**
//!   level ([`Rule::F99IllegalCharacter`]) because the FEC has accepted
//!   real F99 filings containing out-of-range bytes there.
//! * [`Rule::UnrecognizedFormType`] (FEC failing #18) is a warning because
//!   a line hardmoney cannot dispatch may be a token the FEC knows and
//!   hardmoney's tables do not; rejecting on our own gap would be wrong.
//! * Summary-page arithmetic (warnings 3-4, "Subtotal not supported by
//!   Schedule") lives in [`crate::parser::reconcile`], not here.
//!
//! FEC messages with no rule here, and why:
//!
//! | FEC message | Reason |
//! |---|---|
//! | #20 "Validation Terminated! - Over 32,000 Problems" | hardmoney reports every finding. |
//! | #22 "Report Type is Missing or Invalid", #38 "Wrong Report Type for this Form" | The workbook elides the code list (`12C,..., TER`, "refer to Appendix A"), which is not bundled; a missing code is [`Rule::RecommendedFieldEmpty`], and Form 24's `24`/`48` list is checked as [`Rule::InvalidAllowedValue`] at error severity. (Data note: the distiller also misses Form 13's two-item `90D,90S` list.) |
//! | #24 "Extraneous data follows last field", #25 "data coded in a Dummy field", #26 "Invalid double-quote surround", #49 "Field no longer used" | Record-shape checks the parser resolves before a [`ParsedLine`] exists; a strict parse fails on the worst of them. |
//! | #37 "Invalid Rate format" | Schedule C's interest rate is `A/N-15` free text and accepted filings carry `Prime -1`, `SOFR+2.32`, `9.00% APR`. |
//! | #41 "Back/Cross-Reference ... not valid", #42-#44 (H1 redundancy, pre-BCRA H3 D/E codes), #46-#48 (version-3 amendment codes, `[BEGINTEXT]`, Schedule I links) | Legacy or Schedule I rules; Schedule I is not in the current workbook. |
//! | W26 "Election Code missing", W33-W35, W37-W42, W44, W46-W51 (Form 7 codes, ratio codes, Form 1 party codes, creditor codes, yes/no fields, H1 point values, delimited names, H3/H5 breakdown totals, superfluous data, Schedule C line references) | The workbook gives these code lists as prose (`13A = Form 3; Sum Pg #13(a)`), not as `allowed_values`; the H3/H5 breakdown identities belong with [`crate::parser::reconcile`]. |
//!
//! Cross-filing checks (transaction-ID uniqueness "for the life of the
//! report", across amendments) need data this crate does not have in a
//! single file.
//!
//! # Surrounding quotes
//!
//! The FEC's validator accepts a field wrapped in double quotes
//! (`"SMITH"`) and reads the content between the quotes; some vendors emit
//! every field that way. The parser already removes one such pair (see
//! [`crate::parser::utils::normalize_field`]), so by the time a value
//! reaches the validator the wrapping is gone. Any double quote that
//! *remains* is therefore an embedded one, and [`Rule::EmbeddedDoubleQuote`]
//! reports it -- at warning level, because the FEC has accepted real
//! filings carrying a stray `"` in free-text fields.

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
    /// id or amendment number (FEC failing #8). An amendment number of `0`
    /// does not count: FECfile writes it on originals and the FEC accepts
    /// them.
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
    /// More characters than the spec's type allows (FEC failing #7). For
    /// an `AMT-n` field the bound is on the *digits*: the decimal point and
    /// sign are free, because the FEC accepted ActBlue's September 2020
    /// amendment (FEC-1458871, spec 8.3) with twelve-digit Column B totals
    /// written in thirteen characters (`2280311229.59`).
    FieldTooLong,
    /// A character outside the FEC's allowed set (FEC failing #12/#27).
    /// Reported at **warning** severity on superseded spec versions: the
    /// FEC's own note on the rule says only ASCII 32-126 "used to be"
    /// allowed, yet a 2001 spec-3.00 report from Puerto Rico with `á` and
    /// `í` in contributor names was accepted, so pre-8.x enforcement cannot
    /// be inferred from today's ranges.
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
    /// A four-digit year field (`NUM-4`: the F3X "year for above", the F2
    /// year of election, the F4 convention year) that is not `CCYY` (FEC
    /// failing #36).
    InvalidYear,
    /// An amount that is not `[-]digits[.dd]` -- dollar signs and commas
    /// included (FEC failing #34).
    InvalidAmount,
    /// A character other than a digit or one decimal point in a numeric
    /// field that is not a date, year, telephone, or district (FEC failing
    /// #35). The decimal point is allowed because the workbook's `NUM-5`
    /// allocation percentages on Schedules H1 and H2 are written `0.49`, and
    /// every accepted party report carries them that way.
    NonNumeric,
    /// A congressional-district field (the workbook's `01 ... 99` columns)
    /// that is not exactly two digits (FEC failing #39). Reported at
    /// **warning** severity on superseded spec versions: real, accepted
    /// 3.x filings carry one-digit districts (see the module docs).
    InvalidDistrict,
    /// A value outside the spec's published list for the field. A warning
    /// unless the workbook's rule for the field says "Error if Coded
    /// incorrectly" (report codes, FEC failing #22), in which case the
    /// finding carries [`Severity::Error`].
    InvalidAllowedValue,
    /// A value that does not match the spec's published regex for the field.
    PatternMismatch,
    /// Not a USPS state/territory code (FEC warning #29).
    InvalidStateCode,
    /// A ZIP field that is not five or nine digits (FEC warning #30). The
    /// FEC's validator flags foreign postal codes this way too; it is a
    /// warning.
    InvalidZipCode,
    /// A ten-digit telephone field (`NUM-10`) that is not ten digits (FEC
    /// warning #31).
    InvalidPhoneNumber,
    /// A candidate-office field (the workbook's `H,S,P` columns) that is not
    /// `H`, `S`, or `P` (FEC warning #32).
    InvalidOfficeCode,
    /// An `entity_type` outside `CAN CCM COM IND ORG PAC PTY` (FEC warning #45).
    InvalidEntityType,
    /// A Schedule E / Form 57 / Form 76 support/oppose code other than `S`
    /// or `O` (FEC warning #36).
    InvalidSupportOpposeCode,
    /// An election code (the workbook's `Edit: PGI` columns) that is not
    /// one of `C E G O P R S` followed by a four-digit year (FEC warning
    /// #5).
    InvalidElectionCode,
    /// A check-box column (the workbook's `Check-box` rows) holding
    /// something other than `X` (FEC warning #43).
    InvalidCheckbox,
    /// A Schedule H3 event type outside the codes the workbook lists (`AD
    /// GV DF DC EA PC`; FEC failing #45). Reported at **warning** severity
    /// on superseded spec versions, whose H3 records use the pre-BCRA
    /// one-letter codes.
    InvalidEventType,
    /// A second address line filled in while the first is blank (FEC
    /// warning #28).
    AddressInSecondLine,

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
            | Rule::UnrecognizedFormType
            | Rule::RecommendedFieldEmpty
            | Rule::ConditionallyRequiredFieldEmpty
            | Rule::F99IllegalCharacter
            | Rule::DateOutOfRange
            | Rule::InvalidAllowedValue
            | Rule::PatternMismatch
            | Rule::InvalidStateCode
            | Rule::InvalidZipCode
            | Rule::InvalidPhoneNumber
            | Rule::InvalidOfficeCode
            | Rule::InvalidEntityType
            | Rule::InvalidSupportOpposeCode
            | Rule::InvalidElectionCode
            | Rule::InvalidCheckbox
            | Rule::AddressInSecondLine
            | Rule::EmbeddedDoubleQuote => Severity::Warning,
            Rule::HeaderFirst
            | Rule::CoverSecond
            | Rule::FilingTypeFec
            | Rule::AmendmentNeedsOriginalId
            | Rule::AmendmentNeedsNumber
            | Rule::HeaderInconsistentWithAmendmentStatus
            | Rule::MultipleForms
            | Rule::ScheduleNotAllowedWithForm
            | Rule::FilerIdFormat
            | Rule::FilerIdMismatch
            | Rule::RequiredFieldEmpty
            | Rule::FieldTooLong
            | Rule::IllegalCharacter
            | Rule::BadDateFormat
            | Rule::NotARealDate
            | Rule::InvalidYear
            | Rule::InvalidAmount
            | Rule::NonNumeric
            | Rule::InvalidDistrict
            | Rule::InvalidEventType
            | Rule::DuplicateTransactionId
            | Rule::BackReferenceNotFound
            | Rule::F99TextTooLong => Severity::Error,
        }
    }

    /// Whether findings of this rule are demoted to [`Severity::Warning`]
    /// on a filing in a superseded spec version. The bundled workbook
    /// describes the *current* format; for these rules real, FEC-accepted
    /// 3.x-6.x filings demonstrably did not meet it (blank entity types,
    /// accented Latin-1 letters, one-digit districts, pre-BCRA one-letter
    /// H3 event types), so rejecting them would contradict the FEC's own
    /// acceptance. Every superseded-format filing is already reported under
    /// [`Rule::CurrentFormat`]; the FEC rejects it for that alone.
    #[must_use]
    pub const fn demoted_on_superseded_format(self) -> bool {
        matches!(
            self,
            Rule::RequiredFieldEmpty
                | Rule::IllegalCharacter
                | Rule::InvalidDistrict
                | Rule::InvalidEventType
        )
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
            Rule::InvalidYear => "____ is an Invalid Year (CCYY) Format",
            Rule::InvalidAmount => "Invalid Amount format: ____________",
            Rule::NonNumeric => "Non-numeric data in Numeric Field",
            Rule::InvalidDistrict => "District \"__\" is not 2-digit Numeric format",
            Rule::InvalidAllowedValue => "Value \"_\" is Invalid for this field",
            Rule::PatternMismatch => "Value \"_\" does not match the required format",
            Rule::InvalidStateCode => "__ not a valid 2-character USPS State Code",
            Rule::InvalidZipCode => "Zip Code is Invalid or Missing / Zip = _________",
            Rule::InvalidPhoneNumber => "Invalid Area Code/Phone Number: __________",
            Rule::InvalidOfficeCode => "Office Code \"_\" Invalid (Valid Codes: H, S, P)",
            Rule::InvalidEntityType => "Entity Type [___] is not an acceptable value",
            Rule::InvalidSupportOpposeCode => "Sup/Opp Code \"___\" Invalid (Valid Codes: S, O)",
            Rule::InvalidElectionCode => "Election Code invalid: ___ {description}",
            Rule::InvalidCheckbox => "Value \"_\" is Invalid for \"Checkbox=X\" field",
            Rule::InvalidEventType => "Event Type {__} Invalid - OK Vals: [AD|GV|DF|DC|EA] (H3)",
            Rule::AddressInSecondLine => "Single-line Address NOT in 1st delimited field",
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
/// `severity` is `rule.severity()` with two exceptions: on a filing in a
/// superseded spec version the rules for which
/// [`Rule::demoted_on_superseded_format`] is true are reported at
/// [`Severity::Warning`], and an [`Rule::InvalidAllowedValue`] finding on
/// a field whose workbook rule says "Error if Coded incorrectly" (report
/// codes) is reported at [`Severity::Error`]. Filter on `severity`, not on
/// `rule.severity()`.
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
            current_format: bundled_version().is_some_and(|v| v == self.version),
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
    crate::parser::utils::normalize_field(raw)
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

/// Whether `value` is what the FEC's `NUM` type accepts outside its
/// fixed-width uses: digits with at most one decimal point (`5`, `0.49`,
/// `.51`), never a sign, never empty.
#[must_use]
pub fn is_numeric_value(value: &str) -> bool {
    let mut digits = 0usize;
    let mut points = 0usize;
    for b in value.bytes() {
        match b {
            b'0'..=b'9' => digits = digits.saturating_add(1),
            b'.' => points = points.saturating_add(1),
            _ => return false,
        }
    }
    digits > 0 && points <= 1
}

/// Whether a spec row is a `CCYY` year: every `NUM-4` column in the
/// workbook is one (F3X "Year for Above", F2 "YEAR OF ELECTION", F4's
/// convention year).
fn is_year_field(spec: &FieldSpec) -> bool {
    spec.kind == FieldKind::Numeric && spec.max_len == Some(4)
}

/// Whether a spec row is a ten-digit telephone number: every `NUM-10`
/// column in the workbook is one (Form 1's custodian, treasurer, and agent
/// telephones).
fn is_phone_field(spec: &FieldSpec) -> bool {
    spec.kind == FieldKind::Numeric && spec.max_len == Some(10)
}

/// Whether a spec row is a congressional district: the workbook marks
/// these two-character columns with `01, ..., 99` / `01 ... 99` as the
/// value reference.
fn is_district_field(spec: &FieldSpec) -> bool {
    spec.max_len == Some(2)
        && spec
            .value_reference
            .is_some_and(|v| v.trim_start().starts_with("01"))
}

/// Whether a spec row is a ZIP code: a nine-character column whose
/// description says `ZIP`.
fn is_zip_field(spec: &FieldSpec) -> bool {
    spec.max_len == Some(9) && spec.description.to_ascii_uppercase().contains("ZIP")
}

/// Whether a spec row is a candidate-office code: the workbook gives
/// these one-character columns `H,S,P` as the value reference. (Its rule
/// column says `Edit: OFFICE`, except on the Form 6 sheet where the rules
/// are shifted by a row -- see [`is_state_field`] -- so the value
/// reference is the reliable marker.)
fn is_office_field(spec: &FieldSpec) -> bool {
    spec.max_len == Some(1)
        && spec.value_reference.is_some_and(|v| {
            v.split(|c: char| !c.is_ascii_alphabetic())
                .filter(|w| !w.is_empty())
                .eq(["H", "S", "P"])
        })
}

/// Whether a spec row is an election code: the workbook marks these with
/// `Edit: PGI` or `Values: [G|P|R|S|C|E|O]+...` as the rule. The FEC's
/// edit accepts one of `C E G O P R S` followed by a four-digit year.
fn is_election_code_field(spec: &FieldSpec) -> bool {
    spec.max_len == Some(5)
        && spec.rule.is_some_and(|r| {
            let r = r.trim_start();
            r.get(..9)
                .is_some_and(|p| p.eq_ignore_ascii_case("edit: pgi"))
                || r.to_ascii_uppercase().contains("[G|P|R|S|C|E|O]")
        })
}

/// Whether a spec row is a check-box: the workbook's rule column says
/// `Check-box` (sometimes with a qualifier such as `- Mutually exclusive`).
fn is_checkbox_field(spec: &FieldSpec) -> bool {
    spec.max_len == Some(1)
        && spec.rule.is_some_and(|r| {
            r.trim_start()
                .get(..9)
                .is_some_and(|p| p.eq_ignore_ascii_case("check-box"))
        })
}

/// The two-letter codes a value-reference such as `AD=ADministrative;
/// GV=Generic Voter Drive; ...` enumerates (Schedule H3's event type):
/// every run of upper-case letters immediately followed by `=`. Empty when
/// the reference lists none.
fn enumerated_codes(value_reference: &str) -> Vec<&str> {
    value_reference
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | ',' | ':'))
        .filter_map(|token| token.split_once('='))
        .map(|(code, _)| code)
        .filter(|code| !code.is_empty() && code.bytes().all(|b| b.is_ascii_uppercase()))
        .collect()
}

/// Whether `value` has the FEC's election-code shape: one of `C E G O P R
/// S` (convention, election-cycle/other, general, other, primary, runoff,
/// special) followed by a four-digit year, case-insensitively.
#[must_use]
pub fn is_election_code(value: &str) -> bool {
    let bytes = value.as_bytes();
    matches!(
        bytes,
        [kind, year @ ..]
            if matches!(kind.to_ascii_uppercase(), b'C' | b'E' | b'G' | b'O' | b'P' | b'R' | b'S')
                && year.len() == 4
                && year.iter().all(u8::is_ascii_digit)
    )
}

/// The canonical `..._street_1` sibling of a `..._street_2` field, if
/// `name` is one.
fn street_1_sibling(name: &str) -> Option<String> {
    name.strip_suffix("street_2")
        .map(|prefix| format!("{prefix}street_1"))
}

/// Whether the workbook's rule for a coded field promotes a wrong code to
/// a failing message ("Error if Coded incorrectly", on report codes).
fn wrong_code_is_error(spec: &FieldSpec) -> bool {
    spec.rule.is_some_and(|r| {
        r.to_ascii_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .contains("error if coded incorrectly")
    })
}

/// The line's transaction ID, from whichever of the two canonical spellings
/// its table uses (`transaction_id`; older layouts said
/// `transaction_id_number`). Blank counts as absent.
fn transaction_id(line: &ParsedLine) -> Option<&str> {
    TRANSACTION_ID_FIELDS
        .iter()
        .find_map(|f| line.get(f))
        .map(effective_value)
        .filter(|s| !s.is_empty())
}

/// The canonical name the line's table uses for its transaction ID, for
/// attributing a finding to the right field.
fn transaction_id_field(line: &ParsedLine) -> &'static str {
    TRANSACTION_ID_FIELDS
        .iter()
        .copied()
        .find(|f| line.get(f).is_some())
        .unwrap_or("transaction_id")
}

/// Candidate canonical names for the transaction-ID column, in preference
/// order (the layouts were harmonised on `transaction_id`).
const TRANSACTION_ID_FIELDS: &[&str] = &["transaction_id", "transaction_id_number"];

/// Candidate canonical names for the back-reference column.
const BACK_REFERENCE_FIELDS: &[&str] = &["back_reference_tran_id", "back_reference_tran_id_number"];

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
    if norm.contains("ccm, pac, or pty") || norm.contains("ccm, pac or pty") {
        return Condition::EntityIn(&["CCM", "PAC", "PTY"]);
    }
    if norm.contains("if can or ccm") {
        return Condition::EntityIn(&["CAN", "CCM"]);
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
// Field profiles: the classifiers' answers, once per FieldSpec
// ---------------------------------------------------------------------------

/// What the workbook says about one column, decided once per process.
///
/// The classifiers above read a [`FieldSpec`]'s prose (`description`,
/// `rule`, `value_reference`), which never changes while the program
/// runs. Before this cache they ran again for every field of every line
/// -- about a dozen string scans, several of them allocating -- and were
/// most of what `validate` cost on a large filing (5.7 s of 6.4 s on a
/// 135 MB presidential report, against 0.7 s to parse it). The fields
/// here are exactly the classifiers' results, so nothing about which
/// checks fire can change; only when the prose is read.
#[derive(Debug, Clone)]
struct FieldProfile {
    is_date: bool,
    is_year: bool,
    is_phone: bool,
    is_district: bool,
    is_zip: bool,
    is_office: bool,
    is_election_code: bool,
    is_checkbox: bool,
    is_state: bool,
    wrong_code_is_error: bool,
    /// The rule and the line-independent part of the condition that a
    /// blank value is judged by (`check_required`); `None` when the
    /// column is not required at all.
    required: Option<(Rule, Condition)>,
    /// The `..._street_1` sibling of a `..._street_2` column.
    street_1_sibling: Option<String>,
    /// Schedule H3's `event_type` codes, from the value reference; empty
    /// for every other column.
    event_codes: Vec<&'static str>,
}

impl FieldProfile {
    fn of(table: Table, spec: &'static FieldSpec) -> Self {
        let required = match spec.required {
            Requirement::None => None,
            Requirement::Error => Some((
                Rule::RequiredFieldEmpty,
                parse_condition(spec.rule, Condition::Always),
            )),
            Requirement::Warning => Some((
                Rule::RecommendedFieldEmpty,
                parse_condition(spec.rule, Condition::Always),
            )),
            Requirement::Conditional(text) => {
                // The condition is usually in the REQUIRED cell ("X (warn if
                // REPORT CODE=[12?|30?])"); when that cell says only
                // "Conditional Warning", the RULE cell carries it ("Used if
                // CCM, PAC or PTY" on Schedule A's donor committee columns),
                // and WebCheck enforces it from there.
                let mut condition = parse_condition(Some(text), Condition::Unknown);
                if condition == Condition::Unknown {
                    condition = parse_condition(spec.rule, Condition::Unknown);
                }
                Some((Rule::ConditionallyRequiredFieldEmpty, condition))
            }
        };
        let event_codes = if table == Table::H3 && spec.canonical == Some("event_type") {
            spec.value_reference
                .map(enumerated_codes)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Self {
            is_date: is_date_field(spec),
            is_year: is_year_field(spec),
            is_phone: is_phone_field(spec),
            is_district: is_district_field(spec),
            is_zip: is_zip_field(spec),
            is_office: is_office_field(spec),
            is_election_code: is_election_code_field(spec),
            is_checkbox: is_checkbox_field(spec),
            is_state: is_state_field(spec),
            wrong_code_is_error: wrong_code_is_error(spec),
            required,
            street_1_sibling: spec.canonical.and_then(street_1_sibling),
            event_codes,
        }
    }
}

/// One profile per `FieldSpec`, in `Table::specs()` order, indexed by the
/// table's discriminant.
static PROFILES: LazyLock<Vec<Vec<FieldProfile>>> = LazyLock::new(|| {
    let slots = Table::ALL
        .iter()
        .map(|t| *t as usize)
        .max()
        .map_or(0, |m| m.saturating_add(1));
    let mut all = vec![Vec::new(); slots];
    for table in Table::ALL {
        if let Some(slot) = all.get_mut(*table as usize) {
            *slot = table
                .specs()
                .iter()
                .map(|spec| FieldProfile::of(*table, spec))
                .collect();
        }
    }
    all
});

/// The profiles for `table`'s specs, parallel to `table.specs()`.
fn profiles(table: Table) -> &'static [FieldProfile] {
    PROFILES.get(table as usize).map_or(&[], Vec::as_slice)
}

// ---------------------------------------------------------------------------
// The checker
// ---------------------------------------------------------------------------

struct Checker<'f> {
    filing: &'f Filing,
    findings: Vec<Finding>,
    /// Whether the filing is in the bundled (current) spec version, for
    /// [`Checker::demote_if_superseded`]; decided once, not per finding.
    current_format: bool,
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
        let table = line.table();
        let profiles = profiles(table);
        for (i, spec) in table.specs().iter().enumerate() {
            let Some(name) = spec.canonical else {
                continue;
            };
            let Some(raw) = line.get(name) else {
                continue;
            };
            // The cache is built from the same slice, so the profile is
            // always there; the fallback only keeps this panic-free.
            let computed;
            let p: &FieldProfile = match profiles.get(i) {
                Some(p) => p,
                None => {
                    computed = FieldProfile::of(table, spec);
                    &computed
                }
            };
            let value = effective_value(raw);
            if value.is_empty() {
                self.check_required(line, spec, name, p);
                continue;
            }

            if value.contains('"') {
                self.push_line(
                    Rule::EmbeddedDoubleQuote,
                    line,
                    Some(name),
                    format!(
                        "Embedded double-quotes (\") not allowed in {}",
                        spec.description
                    ),
                );
            }

            // A date of the wrong length is reported once, as BadDateFormat.
            // An amount's bound counts digits (see `Rule::FieldTooLong`).
            let (len, unit) = if spec.kind == FieldKind::Amount {
                (value.bytes().filter(u8::is_ascii_digit).count(), "digits")
            } else {
                (value.chars().count(), "characters")
            };
            if !p.is_date
                && let Some(max) = spec.max_len
                && len > usize::from(max)
            {
                self.push_line(
                    Rule::FieldTooLong,
                    line,
                    Some(name),
                    format!(
                        "{} exceeds maximum length of {max} ({len} {unit})",
                        spec.description
                    ),
                );
            }

            self.check_characters(line, name, value);

            let all_digits = value.bytes().all(|b| b.is_ascii_digit());
            match spec.kind {
                FieldKind::Numeric if p.is_date => self.check_date(line, name, value),
                // A wrong-length year or phone number is reported once,
                // under its own rule.
                FieldKind::Numeric if p.is_year => {
                    if !(all_digits && len == 4) {
                        self.push_line(
                            Rule::InvalidYear,
                            line,
                            Some(name),
                            format!(
                                "{value} is an Invalid Year (CCYY) Format for {}",
                                spec.description
                            ),
                        );
                    }
                }
                FieldKind::Numeric if p.is_phone => {
                    if !(all_digits && len == 10) {
                        self.push_line(
                            Rule::InvalidPhoneNumber,
                            line,
                            Some(name),
                            format!(
                                "Invalid Area Code/Phone Number: {value} in {} (ten digits)",
                                spec.description
                            ),
                        );
                    }
                }
                FieldKind::Numeric if p.is_district => {}
                FieldKind::Numeric => {
                    if !is_numeric_value(value) {
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

            // Districts are `NUM-2` on most sheets and `A/N-2` on two; the
            // FEC's message is the same, so the check is by marker, not
            // type, and stands in for the generic numeric and pattern
            // checks on these columns.
            if p.is_district && !(all_digits && len == 2) {
                let mut finding = Finding::new(
                    Rule::InvalidDistrict,
                    line.line_no,
                    &line.raw_form_type,
                    Some(name),
                    format!(
                        "District \"{value}\" is not 2-digit Numeric format ({})",
                        spec.description
                    ),
                );
                self.demote_if_superseded(&mut finding);
                self.findings.push(finding);
            }

            if p.is_zip && !(all_digits && (len == 5 || len == 9)) {
                self.push_line(
                    Rule::InvalidZipCode,
                    line,
                    Some(name),
                    format!(
                        "Zip Code is Invalid or Missing / Zip = {value} ({}: five or nine digits)",
                        spec.description
                    ),
                );
            }

            if p.is_office
                && !(value.eq_ignore_ascii_case("H")
                    || value.eq_ignore_ascii_case("S")
                    || value.eq_ignore_ascii_case("P"))
            {
                self.push_line(
                    Rule::InvalidOfficeCode,
                    line,
                    Some(name),
                    format!("Office Code \"{value}\" Invalid (Valid Codes: H, S, P)"),
                );
            }

            if p.is_election_code && !is_election_code(value) {
                self.push_line(
                    Rule::InvalidElectionCode,
                    line,
                    Some(name),
                    format!(
                        "Election Code invalid: {value} {} (one of C, E, G, O, P, R, S followed by a four-digit year, e.g. P2026)",
                        spec.description
                    ),
                );
            }

            if p.is_checkbox && !value.eq_ignore_ascii_case("X") {
                self.push_line(
                    Rule::InvalidCheckbox,
                    line,
                    Some(name),
                    format!(
                        "Value \"{value}\" is Invalid for \"Checkbox=X\" field {}",
                        spec.description
                    ),
                );
            }

            if let Some(sibling) = p.street_1_sibling.as_deref()
                && line
                    .get(sibling)
                    .is_some_and(|s| effective_value(s).is_empty())
            {
                self.push_line(
                    Rule::AddressInSecondLine,
                    line,
                    Some(name),
                    format!(
                        "Single-line Address NOT in 1st delimited field: {} is '{value}' but {} is blank",
                        spec.description,
                        sibling.to_ascii_uppercase().replace('_', " ")
                    ),
                );
            }

            match name {
                "event_type" if table == Table::H3 => {
                    let codes = &p.event_codes;
                    if !codes.is_empty() && !codes.iter().any(|c| c.eq_ignore_ascii_case(value)) {
                        let mut finding = Finding::new(
                            Rule::InvalidEventType,
                            line.line_no,
                            &line.raw_form_type,
                            Some(name),
                            format!(
                                "Event Type {{{value}}} Invalid - OK Vals: [{}] (H3)",
                                codes.join("|")
                            ),
                        );
                        self.demote_if_superseded(&mut finding);
                        self.findings.push(finding);
                    }
                }
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
                        let mut finding = Finding::new(
                            Rule::InvalidAllowedValue,
                            line.line_no,
                            &line.raw_form_type,
                            Some(name),
                            format!(
                                "Value \"{value}\" is Invalid for {} (valid: {})",
                                spec.description,
                                spec.allowed_values.join(", ")
                            ),
                        );
                        // "Warning if Code is missing; Error if Coded
                        // incorrectly": the workbook's severity for a wrong
                        // report code (FEC failing #22).
                        if p.wrong_code_is_error {
                            finding.severity = Severity::Error;
                        }
                        self.findings.push(finding);
                    }
                }
            }

            // The filer ID's format is FilerIdFormat's job (on the cover) and
            // FilerIdMismatch's (in the body); a district's is
            // InvalidDistrict's and a year's InvalidYear's. Do not report
            // any of them twice.
            if name != "filer_committee_id_number"
                && !p.is_district
                && !p.is_year
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

            if p.is_state && !STATE_CODES.iter().any(|s| s.eq_ignore_ascii_case(value)) {
                self.push_line(
                    Rule::InvalidStateCode,
                    line,
                    Some(name),
                    format!("{value} not a valid 2-character USPS State Code"),
                );
            }
        }
    }

    fn check_required(
        &mut self,
        line: &ParsedLine,
        spec: &FieldSpec,
        name: &'static str,
        profile: &FieldProfile,
    ) {
        let Some((rule, condition)) = &profile.required else {
            return;
        };
        let rule = *rule;
        // `implied_condition` depends on the line, so it is applied here,
        // not in the profile.
        let implied = (*condition == Condition::Always)
            .then(|| implied_condition(line, name))
            .flatten();
        let condition = implied.as_ref().unwrap_or(condition);
        if self.condition_holds(line, condition) != Some(true) {
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
        self.demote_if_superseded(&mut finding);
        self.findings.push(finding);
    }

    /// The workbook describes the current format; earlier formats
    /// demonstrably differed (spec 3.x-5.x filings with blank entity types
    /// and one-digit districts were accepted). For the rules that say so
    /// ([`Rule::demoted_on_superseded_format`]), report but do not reject.
    fn demote_if_superseded(&self, finding: &mut Finding) {
        if finding.rule.demoted_on_superseded_format() && !self.current_format {
            finding.severity = Severity::Warning;
        }
    }

    /// `Some(true)` if the condition holds for this line, `Some(false)` if
    /// it does not, `None` if it cannot be evaluated (unknown prose, or the
    /// line's layout lacks the field the condition refers to).
    fn condition_holds(&self, line: &ParsedLine, condition: &Condition) -> Option<bool> {
        // Codes are compared case-insensitively without allocating (the
        // sets and prefixes are upper-case ASCII).
        let code = |field: &str| line.get(field).map(effective_value);
        let in_set = |value: &str, set: &[&str]| set.iter().any(|s| s.eq_ignore_ascii_case(value));
        match condition {
            Condition::Always => Some(true),
            Condition::Unknown => None,
            Condition::EntityIn(set) => {
                let entity = code("entity_type")?;
                Some(in_set(entity, set))
            }
            Condition::EntityNotIn(set) => {
                let entity = code("entity_type")?;
                if entity.is_empty() {
                    // A blank entity type is its own finding; do not guess.
                    return None;
                }
                Some(!in_set(entity, set))
            }
            Condition::FormTypeIs(token) => Some(self.filing.raw_form_type == *token),
            Condition::ReportCodeStartsWith(prefixes) => {
                let report_code = code("report_code")?.as_bytes();
                Some(prefixes.iter().any(|p| {
                    report_code
                        .get(..p.len())
                        .is_some_and(|head| head.eq_ignore_ascii_case(p.as_bytes()))
                }))
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
            let mut finding = Finding::new(
                Rule::IllegalCharacter,
                line.line_no,
                &line.raw_form_type,
                Some(name),
                format!(
                    "Illegal character(s) found in text field: {}",
                    describe_chars(&bad)
                ),
            );
            self.demote_if_superseded(&mut finding);
            self.findings.push(finding);
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
                    self.push_line(
                        Rule::DuplicateTransactionId,
                        line,
                        Some(transaction_id_field(line)),
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
            let Some((field, raw)) = BACK_REFERENCE_FIELDS
                .iter()
                .find_map(|f| line.get(f).map(|raw| (*f, raw)))
            else {
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
                    Some(field),
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
        for (line_index, text_line) in text.split('\n').enumerate() {
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
                        line_index.saturating_add(1),
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
    if let Some(more) = chars.len().checked_sub(3).filter(|&n| n > 0) {
        parts.push(format!("and {more} more"));
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
            p("Used if CCM, PAC or PTY"),
            EntityIn(&["CCM", "PAC", "PTY"])
        );
        assert_eq!(p("Used if CAN or CCM"), EntityIn(&["CAN", "CCM"]));
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
    fn non_amendment_with_report_id_is_inconsistent_and_fails() {
        let text = clean_text().replacen(
            "8.5.1.0(f34)\u{1c}\u{1c}\u{1c}",
            "8.5.1.0(f34)\u{1c}FEC-1234567\u{1c}1\u{1c}",
            1,
        );
        let v = parse(&text).validate();
        assert_eq!(rules(&v), [Rule::HeaderInconsistentWithAmendmentStatus]);
        // FEC failing #8: the header says amended, the form says new.
        assert!(!v.is_acceptable());
        assert_eq!(v.findings[0].severity, Severity::Error);
        // FECfile writes amendment number 0 (or 000) on originals; the FEC
        // accepts it, so neither is a finding.
        for zero in ["0", "000"] {
            let text = clean_text().replacen(
                "8.5.1.0(f34)\u{1c}\u{1c}\u{1c}",
                &format!("8.5.1.0(f34)\u{1c}\u{1c}{zero}\u{1c}"),
                1,
            );
            assert!(parse(&text).validate().is_empty(), "{zero}");
        }
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
                    && f.field == Some("election_date")),
            "{v}"
        );
        // Quarterly: no such warning.
        let v = parse(&clean_text()).validate();
        assert!(!has(&v, Rule::ConditionallyRequiredFieldEmpty));
    }

    /// Schedule A's donor columns say only "Conditional Warning" in the
    /// REQUIRED cell; the condition ("Used if CCM, PAC or PTY", "Used if
    /// CAN or CCM") is in the RULE cell, and WebCheck enforces it: on the
    /// Georgia Republican Party fixture it flagged a `PTY` line's blank
    /// donor committee id and a `CCM` line's blank candidate id and last
    /// name.
    #[test]
    fn donor_committee_and_candidate_columns_follow_the_rule_cell() {
        let fields = |pairs: &[(&str, &str)]| -> Vec<&'static str> {
            with_lines(vec![sched_a(pairs)])
                .validate()
                .findings
                .iter()
                .filter(|f| f.rule == Rule::ConditionallyRequiredFieldEmpty)
                .filter_map(|f| f.field)
                .collect()
        };
        let pty = fields(&[
            ("entity_type", "PTY"),
            ("contributor_organization_name", "14TH DISTRICT GOP"),
            ("contributor_last_name", ""),
            ("contributor_first_name", ""),
        ]);
        assert_eq!(
            pty,
            ["donor_committee_fec_id", "donor_committee_name"],
            "{pty:?}"
        );
        let ccm = fields(&[
            ("entity_type", "CCM"),
            ("contributor_organization_name", "COLLINS FOR SENATE"),
            ("contributor_last_name", ""),
            ("contributor_first_name", ""),
            ("donor_committee_fec_id", "C00544684"),
            ("donor_committee_name", "COLLINS FOR SENATE"),
        ]);
        assert_eq!(
            ccm,
            [
                "donor_candidate_fec_id",
                "donor_candidate_last_name",
                "donor_candidate_office"
            ],
            "{ccm:?}"
        );
        // An individual owes none of them.
        assert!(fields(&[]).is_empty());
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
        // AMT-12 bounds the digits: ActBlue's accepted 2280311229.59 (13
        // characters, 12 digits) is fine; a 13th digit is not, and the
        // decimal point and sign are never counted.
        for (amount, ok) in [
            ("2280311229.59", true),
            ("-2280311229.59", true),
            ("999999999999", true),
            ("1000000000000", false),
            ("12280311229.59", false),
        ] {
            let v = with_lines(vec![sched_a(&[("contribution_amount", amount)])]).validate();
            assert_eq!(!has(&v, Rule::FieldTooLong), ok, "{amount}: {v}");
            if !ok {
                assert!(v.to_string().contains("13 digits"), "{v}");
            }
        }
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

        // The parser strips one wrapping pair; what remains is embedded.
        let v = with_lines(vec![sched_a(&[("contributor_last_name", "\"O\"NEIL\"")])]).validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::EmbeddedDoubleQuote)
            .unwrap();
        assert_eq!(f.severity, Severity::Warning, "{f}");
        let v = with_lines(vec![sched_a(&[(
            "contributor_first_name",
            "JOHN \"JACK\"",
        )])])
        .validate();
        assert!(has(&v, Rule::EmbeddedDoubleQuote), "{v}");
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
        // A district is reported once, under its own rule (#39), not as
        // NonNumeric or as a pattern mismatch.
        let v = with_lines(vec![sched_a(&[("donor_candidate_district", "1A")])]).validate();
        assert_eq!(rules(&v), [Rule::InvalidDistrict], "{v}");
        assert_eq!(Rule::NonNumeric.severity(), Severity::Error);

        // Schedule H2's allocation percentages are NUM-5 and written 0.49:
        // a decimal point is numeric, a letter or a second point is not.
        for (value, ok) in [
            ("0.49", true),
            (".51", true),
            ("49", true),
            ("49%", false),
            ("0.4.9", false),
            (".", false),
        ] {
            let v = with_lines(vec![h2(&[("federal_percentage", value)])]).validate();
            assert_eq!(!has(&v, Rule::NonNumeric), ok, "{value}: {v}");
        }
        assert!(is_numeric_value("0"));
        assert!(!is_numeric_value(""));
        assert!(!is_numeric_value("-1"));
    }

    /// A Schedule H2 allocation-ratio line with the given overrides.
    fn h2(pairs: &[(&str, &str)]) -> ParsedLine {
        let mut all = vec![
            ("form_type", "H2"),
            ("filer_committee_id_number", "C00123456"),
            ("transaction_id", "H2.1"),
            ("activity_event_name", "SPRING GALA"),
            ("direct_fundraising", "X"),
            ("ratio_code", "N"),
            ("federal_percentage", "0.60"),
            ("nonfederal_percentage", "0.40"),
        ];
        for (k, v) in pairs {
            if let Some(slot) = all.iter_mut().find(|(name, _)| name == k) {
                slot.1 = v;
            } else {
                all.push((k, v));
            }
        }
        ParsedLine::from_pairs(Table::H2, v85(), 3, all).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn year_fields_must_be_ccyy() {
        // F3X's Column B "year for above" is the NUM-4 column on the cover.
        for (value, ok) in [
            ("2026", true),
            ("26", false),
            ("202", false),
            ("2O26", false),
        ] {
            let mut filing = parse(&clean_text());
            filing.summary.set("col_b_year", value).unwrap();
            let v = filing.validate();
            assert_eq!(!has(&v, Rule::InvalidYear), ok, "{value}: {v}");
            if !ok {
                let f = v
                    .findings
                    .iter()
                    .find(|f| f.rule == Rule::InvalidYear)
                    .unwrap();
                assert_eq!(f.severity, Severity::Error);
                assert!(f.message.contains("Invalid Year (CCYY)"), "{f}");
                // Reported once: not also as NonNumeric or a pattern mismatch.
                assert_eq!(rules(&v), [Rule::InvalidYear], "{v}");
            }
        }
    }

    #[test]
    fn district_must_be_two_digits_and_is_demoted_on_old_formats() {
        for (value, ok) in [
            ("08", true),
            ("12", true),
            ("00", true),
            ("8", false),
            ("123", false),
            ("AL", false),
        ] {
            let v = with_lines(vec![sched_a(&[("donor_candidate_district", value)])]).validate();
            assert_eq!(!has(&v, Rule::InvalidDistrict), ok, "{value}: {v}");
            if !ok {
                let f = v
                    .findings
                    .iter()
                    .find(|f| f.rule == Rule::InvalidDistrict)
                    .unwrap();
                assert_eq!(f.severity, Severity::Error);
                assert!(
                    f.message
                        .contains(&format!("District \"{value}\" is not 2-digit")),
                    "{f}"
                );
            }
        }
        // Schedule E's district is A/N-2 in the workbook; the marker is the
        // `01 ... 99` value reference, so it is checked the same way.
        assert!(is_district_field(
            Table::SchE.spec("candidate_district").unwrap()
        ));
        assert!(!is_district_field(Table::F5.spec("report_type").unwrap()));
        // A 2001 filing with one-digit districts was accepted: warning there.
        let mut filing = parse(&clean_text().replacen("\u{1c}8.5\u{1c}", "\u{1c}8.4\u{1c}", 1));
        filing.lines = vec![sched_a(&[("donor_candidate_district", "8")])];
        let v = filing.validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::InvalidDistrict)
            .unwrap();
        assert_eq!(f.severity, Severity::Warning);
        assert!(Rule::InvalidDistrict.demoted_on_superseded_format());
        assert!(!Rule::InvalidYear.demoted_on_superseded_format());
    }

    #[test]
    fn zip_codes_are_five_or_nine_digits() {
        for (value, ok) in [
            ("22201", true),
            ("222011234", true),
            ("2220", false),
            ("2220-123", false),
            ("SW1A 1AA", false),
        ] {
            let v = with_lines(vec![sched_a(&[("contributor_zip_code", value)])]).validate();
            assert_eq!(!has(&v, Rule::InvalidZipCode), ok, "{value}: {v}");
            if !ok {
                assert_eq!(rules(&v), [Rule::InvalidZipCode], "{value}: {v}");
                let f = &v.findings[0];
                assert_eq!(f.severity, Severity::Warning);
                assert!(f.message.contains(&format!("Zip = {value}")), "{f}");
                assert!(v.is_acceptable());
            }
        }
        // Over nine characters is the length rule's job as well.
        let v = with_lines(vec![sched_a(&[("contributor_zip_code", "22201-1234")])]).validate();
        assert!(
            has(&v, Rule::InvalidZipCode) && has(&v, Rule::FieldTooLong),
            "{v}"
        );
    }

    #[test]
    fn office_codes_are_h_s_p() {
        for (value, ok) in [
            ("H", true),
            ("S", true),
            ("P", true),
            ("h", true),
            ("X", false),
            ("HS", false),
        ] {
            let v = with_lines(vec![sched_a(&[
                ("entity_type", "CAN"),
                ("donor_candidate_office", value),
            ])])
            .validate();
            assert_eq!(!has(&v, Rule::InvalidOfficeCode), ok, "{value}: {v}");
        }
        let v = with_lines(vec![sched_a(&[("donor_candidate_office", "X")])]).validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::InvalidOfficeCode)
            .unwrap();
        assert_eq!(f.severity, Severity::Warning);
        assert!(
            f.message
                .contains("Office Code \"X\" Invalid (Valid Codes: H, S, P)"),
            "{f}"
        );
        // Form 6's office column carries the row-shifted `Edit: ST`; the
        // `H,S,P` value reference still identifies it.
        assert!(is_office_field(Table::F6.spec("candidate_office").unwrap()));
        assert!(!is_office_field(
            Table::SchA.spec("contributor_state").unwrap()
        ));
    }

    #[test]
    fn phone_numbers_are_ten_digits() {
        let f1 = |phone: &str| {
            let mut filing = parse(&clean_text());
            filing.lines.clear();
            filing.summary = ParsedLine::from_pairs(
                Table::F1,
                v85(),
                2,
                [
                    ("form_type", "F1N"),
                    ("filer_committee_id_number", "C00123456"),
                    ("treasurer_telephone", phone),
                ],
            )
            .unwrap();
            filing.validate()
        };
        for (value, ok) in [
            ("2025551234", true),
            ("5551234", false),
            ("202-555-1234", false),
            ("(202) 555-1234", false),
        ] {
            let v = f1(value);
            assert_eq!(!has(&v, Rule::InvalidPhoneNumber), ok, "{value}: {v}");
            // Reported once, not also as NonNumeric.
            assert!(!has(&v, Rule::NonNumeric), "{value}: {v}");
        }
        let f = f1("5551234");
        let f = f
            .findings
            .iter()
            .find(|f| f.rule == Rule::InvalidPhoneNumber)
            .unwrap();
        assert_eq!(f.severity, Severity::Warning);
        assert!(
            f.message
                .contains("Invalid Area Code/Phone Number: 5551234"),
            "{f}"
        );
    }

    #[test]
    fn election_codes_are_a_letter_and_a_year() {
        for (value, ok) in [
            ("P2026", true),
            ("G2026", true),
            ("o2026", true),
            ("P", false),
            ("2026", false),
            ("X2026", false),
            ("P26", false),
        ] {
            assert_eq!(is_election_code(value), ok, "{value}");
            let v = with_lines(vec![sched_a(&[("election_code", value)])]).validate();
            assert_eq!(!has(&v, Rule::InvalidElectionCode), ok, "{value}: {v}");
        }
        let v = with_lines(vec![sched_a(&[("election_code", "P")])]).validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::InvalidElectionCode)
            .unwrap();
        assert_eq!(f.severity, Severity::Warning);
        assert!(f.message.contains("Election Code invalid: P"), "{f}");
        assert!(is_election_code_field(
            Table::F3X.spec("election_code").unwrap()
        ));
        assert!(!is_election_code_field(
            Table::F3X.spec("report_code").unwrap()
        ));
    }

    #[test]
    fn checkboxes_hold_x() {
        for (value, ok) in [("X", true), ("x", true), ("Y", false), ("1", false)] {
            let mut filing = parse(&clean_text());
            filing.summary.set("change_of_address", value).unwrap();
            let v = filing.validate();
            assert_eq!(!has(&v, Rule::InvalidCheckbox), ok, "{value}: {v}");
        }
        let mut filing = parse(&clean_text());
        filing.summary.set("qualified_committee", "Y").unwrap();
        let v = filing.validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::InvalidCheckbox)
            .unwrap();
        assert_eq!(f.severity, Severity::Warning);
        assert!(
            f.message
                .contains("Value \"Y\" is Invalid for \"Checkbox=X\" field"),
            "{f}"
        );
        // Form 1M's committee type has `X=State Pty; N=Other`: a code list,
        // not a check-box.
        assert!(!is_checkbox_field(
            Table::F1M.spec("committee_type").unwrap()
        ));
        assert!(is_checkbox_field(
            Table::F3X.spec("change_of_address").unwrap()
        ));
    }

    #[test]
    fn h3_event_types_come_from_the_workbook_and_are_demoted_on_old_formats() {
        let h3 = |event: &str| {
            ParsedLine::from_pairs(
                Table::H3,
                v85(),
                3,
                [
                    ("form_type", "H3"),
                    ("filer_committee_id_number", "C00123456"),
                    ("transaction_id", "H3.1"),
                    ("back_reference_tran_id", "H3.1"),
                    ("account_name", "STATE CHECKING"),
                    ("event_type", event),
                    ("receipt_date", "20260504"),
                    ("total_amount_transferred", "100.00"),
                    ("transferred_amount", "100.00"),
                ],
            )
            .unwrap()
        };
        assert_eq!(
            enumerated_codes(
                Table::H3
                    .spec("event_type")
                    .unwrap()
                    .value_reference
                    .unwrap()
            ),
            ["AD", "GV", "DF", "DC", "EA", "PC"]
        );
        assert!(enumerated_codes("01, ..., 99").is_empty());
        for (value, ok) in [
            ("AD", true),
            ("df", true),
            ("PC", true),
            ("A", false),
            ("XX", false),
        ] {
            let v = with_lines(vec![h3(value)]).validate();
            assert_eq!(!has(&v, Rule::InvalidEventType), ok, "{value}: {v}");
        }
        let v = with_lines(vec![h3("A")]).validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::InvalidEventType)
            .unwrap();
        assert_eq!(f.severity, Severity::Error);
        assert!(
            f.message
                .contains("Event Type {A} Invalid - OK Vals: [AD|GV|DF|DC|EA|PC] (H3)"),
            "{f}"
        );
        // Pre-BCRA filings used one-letter codes; the FEC accepted them.
        let mut filing = parse(&clean_text().replacen("\u{1c}8.5\u{1c}", "\u{1c}8.4\u{1c}", 1));
        filing.lines = vec![h3("A")];
        let v = filing.validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::InvalidEventType)
            .unwrap();
        assert_eq!(f.severity, Severity::Warning);
    }

    #[test]
    fn address_in_second_line_only() {
        let v = with_lines(vec![sched_a(&[
            ("contributor_street_1", ""),
            ("contributor_street_2", "1 ELM ST"),
        ])])
        .validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::AddressInSecondLine)
            .unwrap_or_else(|| panic!("{v}"));
        assert_eq!(f.severity, Severity::Warning);
        assert_eq!(f.field, Some("contributor_street_2"));
        assert!(
            f.message
                .contains("Single-line Address NOT in 1st delimited field"),
            "{f}"
        );
        // Both lines filled, or only the first: fine.
        let v = with_lines(vec![sched_a(&[("contributor_street_2", "APT 4")])]).validate();
        assert!(!has(&v, Rule::AddressInSecondLine), "{v}");
        let v = with_lines(vec![sched_a(&[])]).validate();
        assert!(!has(&v, Rule::AddressInSecondLine), "{v}");
        assert_eq!(
            street_1_sibling("payee_street_2").as_deref(),
            Some("payee_street_1")
        );
        assert_eq!(street_1_sibling("payee_street_1"), None);
    }

    #[test]
    fn wrong_report_code_is_an_error_where_the_workbook_says_so() {
        // F24: "Error if Code is missing; Error if Coded incorrectly."
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
            .unwrap();
        assert_eq!(f.severity, Severity::Error);
        assert_eq!(Rule::InvalidAllowedValue.severity(), Severity::Warning);
        assert!(!v.is_acceptable());
        assert!(wrong_code_is_error(Table::F24.spec("report_type").unwrap()));
        assert!(!wrong_code_is_error(
            Table::F1M.spec("committee_type").unwrap()
        ));
    }

    #[test]
    fn illegal_characters_are_demoted_on_old_formats() {
        let mut filing = parse(&clean_text().replacen("\u{1c}8.5\u{1c}", "\u{1c}8.4\u{1c}", 1));
        filing.lines = vec![sched_a(&[("contributor_last_name", "GARC\u{cd}A")])];
        let v = filing.validate();
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::IllegalCharacter)
            .unwrap();
        assert_eq!(f.severity, Severity::Warning);
        assert!(v.is_acceptable(), "{v}");
        let v = with_lines(vec![sched_a(&[("contributor_last_name", "GARC\u{cd}A")])]).validate();
        assert_eq!(
            v.findings
                .iter()
                .find(|f| f.rule == Rule::IllegalCharacter)
                .unwrap()
                .severity,
            Severity::Error
        );
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
        let sched_e = |code: &str| {
            ParsedLine::from_pairs(
                Table::SchE,
                v85(),
                3,
                [
                    ("form_type", "SE"),
                    ("filer_committee_id_number", "C00123456"),
                    ("transaction_id", "E1"),
                    ("support_oppose_code", code),
                ],
            )
            .unwrap()
        };
        let v = with_lines(vec![sched_e("X")]).validate();
        assert!(has(&v, Rule::InvalidSupportOpposeCode), "{v}");
        let f = v
            .findings
            .iter()
            .find(|f| f.rule == Rule::InvalidSupportOpposeCode)
            .unwrap();
        assert_eq!(f.severity, Severity::Warning);
        assert!(f.message.contains("Sup/Opp Code \"X\" Invalid"));
        for ok in ["S", "O", "s"] {
            let v = with_lines(vec![sched_e(ok)]).validate();
            assert!(!has(&v, Rule::InvalidSupportOpposeCode), "{ok}: {v}");
        }
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
                ("transaction_id", "ABC"),
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
        assert_eq!(dups[1].field, Some("transaction_id"));
        assert!(dups[0].message.contains("first used on line 3"));
    }

    #[test]
    fn back_reference_must_resolve() {
        let mut memo = sched_a(&[
            ("transaction_id", "M1"),
            ("back_reference_tran_id", "T1"),
            ("back_reference_sched_name", "SA11AI"),
            ("memo_code", "X"),
        ]);
        memo.line_no = 4;
        let v = with_lines(vec![sched_a(&[]), memo.clone()]).validate();
        assert!(!has(&v, Rule::BackReferenceNotFound), "{v}");

        memo.set("back_reference_tran_id", "NOPE").unwrap();
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
