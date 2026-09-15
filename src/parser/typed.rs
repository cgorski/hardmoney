//! Domain views over the highest-value schedules and forms, built on the
//! compile-time-checked [`Typed`] layer.
//!
//! The raw layer hands back trimmed strings by field name, which is
//! faithful to the wire format but tedious to consume -- every caller has
//! to re-parse dates and money and re-derive things like "is this an
//! individual or a committee contributor" from raw entity-type codes. The
//! [`TypedView`] trait and its implementations here do that once, correctly.
//!
//! # Table safety
//!
//! Every view declares its table through an associated [`TableMarker`].
//! [`TypedView::from_line`] (reached via [`ParsedLine::view`] and
//! [`Filing::views`](crate::Filing::views)) refuses a line from any other
//! table with [`TypedViewError::WrongTable`], and *inside* a view the field
//! constants are checked against that table by the compiler. Without the
//! runtime check, a table-blind `ScheduleE` conversion would happily
//! "succeed" on a Schedule A line -- every schedule carries
//! `filer_committee_id_number` -- and produce an all-`None` phantom
//! expenditure. That misfiling was observed on 587 of 587 non-Schedule-E
//! lines of a real F3A before this guard existed.
//!
//! # Money
//!
//! Money is parsed with [`parse_money`] into a [`Decimal`] with scale 2
//! (exact cents), never `f64`: FEC amounts are decimal currency values and
//! binary floating point cannot represent every decimal fraction exactly.
//! The same `Decimal` type flows through to Postgres `NUMERIC` columns and
//! the REST API's JSON, so an amount is never round-tripped through `f64`
//! at any layer.

use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::parser::filing::ParsedLine;
use crate::parser::schema::{Field, TableMarker, Typed};
use crate::parser::tables::markers::{F3X, SchA, SchB, SchE};
use crate::parser::tables::{Table, f3x, sch_a, sch_b, sch_e};

/// Parses an FEC amount field (e.g. `"1000"`, `"250.50"`, `"-75"`) into an
/// exact [`Decimal`] with scale 2. Returns `None` for blank or unparseable
/// input.
///
/// FEC amounts always have at most two decimal places in practice, but this
/// tolerates fewer (or a bare integer) and rejects more than two -- a
/// third decimal place in a supposedly-cents field is far more likely to
/// indicate a column-alignment bug upstream than a genuine sub-cent value,
/// so this fails closed rather than silently accepting it.
///
/// The parse itself never goes through `f64`: the validated digits are
/// assembled directly into a scaled integer and handed to
/// [`Decimal::new`], which represents `value * 10^-scale` exactly.
#[must_use]
pub fn parse_money(raw: &str) -> Option<Decimal> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let negative = trimmed.starts_with('-');
    let unsigned = trimmed.trim_start_matches(['+', '-']);
    if unsigned.len() + 1 < trimmed.len() {
        // More than one sign character.
        return None;
    }

    let (whole, frac) = match unsigned.split_once('.') {
        Some((w, f)) => (w, f),
        None => (unsigned, ""),
    };
    if frac.len() > 2
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || !frac.bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    if whole.is_empty() && frac.is_empty() {
        return None;
    }

    let whole_cents: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let frac_cents: i64 = match frac.len() {
        0 => 0,
        1 => frac.parse::<i64>().ok()?.checked_mul(10)?,
        _ => frac.parse().ok()?,
    };

    let total_cents = whole_cents.checked_mul(100)?.checked_add(frac_cents)?;
    let total_cents = if negative {
        total_cents.checked_neg()?
    } else {
        total_cents
    };
    Some(Decimal::new(total_cents, 2))
}

/// Parses an FEC date field (`YYYYMMDD`, exactly eight digits) into a
/// [`NaiveDate`]. Returns `None` for blank, all-zero, or unparseable input
/// -- FEC filings routinely leave optional dates blank or zero-filled
/// rather than omitting the column.
#[must_use]
pub fn parse_fec_date(raw: &str) -> Option<NaiveDate> {
    let trimmed = raw.trim();
    if trimmed.len() != 8
        || !trimmed.bytes().all(|b| b.is_ascii_digit())
        || trimmed.bytes().all(|b| b == b'0')
    {
        return None;
    }
    NaiveDate::parse_from_str(trimmed, "%Y%m%d").ok()
}

/// Error converting a [`ParsedLine`] into a typed view.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TypedViewError {
    /// The line belongs to a different table than the view reads.
    #[error("line {line_no} is a {found} line, not {expected}")]
    WrongTable {
        expected: Table,
        found: Table,
        line_no: u64,
    },
    /// A field the view genuinely requires is absent or blank.
    #[error("{table} line{} is missing required field '{field}'", fmt_line(*line_no))]
    MissingField {
        table: Table,
        field: &'static str,
        line_no: Option<u64>,
    },
}

fn fmt_line(line_no: Option<u64>) -> String {
    match line_no {
        Some(n) => format!(" {n}"),
        None => String::new(),
    }
}

impl TypedViewError {
    fn with_line(mut self, n: u64) -> Self {
        if let TypedViewError::MissingField { line_no, .. } = &mut self {
            *line_no = Some(n);
        }
        self
    }
}

/// A domain view over the fields of one [`Table`].
///
/// Implementors name their table via [`TypedView::Marker`] and build
/// themselves from a [`Typed`] view whose field access is checked against
/// that table at compile time. Use [`ParsedLine::view`] or
/// [`Filing::views`](crate::Filing::views) rather than calling these directly.
pub trait TypedView: Sized {
    /// The marker type of the table this view reads.
    type Marker: TableMarker;

    /// The table this view reads.
    const TABLE: Table = <Self::Marker as TableMarker>::TABLE;

    /// Builds the view from a table-checked line. Missing optional fields
    /// become `None`; only a genuinely required field (the filer id) is an
    /// error.
    fn from_typed(line: Typed<'_, Self::Marker>) -> Result<Self, TypedViewError>;

    /// Builds the view from a parsed line, refusing lines from other tables.
    fn from_line(line: &ParsedLine) -> Result<Self, TypedViewError> {
        let typed = line.typed::<Self::Marker>()?;
        Self::from_typed(typed).map_err(|e| e.with_line(line.line_no))
    }
}

/// Required non-empty string field.
fn req<T: TableMarker>(line: Typed<'_, T>, field: Field<T>) -> Result<String, TypedViewError> {
    line.string(field).ok_or(TypedViewError::MissingField {
        table: T::TABLE,
        field: field.name(),
        line_no: None,
    })
}

/// The name-shaped columns a schedule may use -- see [`combined_name`] for
/// why both shapes need to be checked. `organization` is `None` for
/// schedules with no such column (e.g. a Schedule E candidate, who is
/// always an individual).
struct NameFields<T: TableMarker> {
    single: Field<T>,
    organization: Option<Field<T>>,
    prefix: Field<T>,
    first: Field<T>,
    middle: Field<T>,
    last: Field<T>,
    suffix: Field<T>,
}

/// Resolves a display name from the fields FEC uses across spec versions.
///
/// Spec 5.x and older report a single pre-joined name field (e.g.
/// `contributor_name`, `payee_name`, `candidate_name`). From 6.1 the FEC
/// dropped that combined field and split the name into
/// `*_organization_name` (for a committee/business payee or contributor)
/// or `*_prefix`/`*_first_name`/`*_middle_name`/`*_last_name`/`*_suffix`
/// (for a person). Without this fallback, every typed-view name field would
/// be `None` for virtually all present-day filings even though the name is
/// right there in the raw line.
fn combined_name<T: TableMarker>(line: Typed<'_, T>, f: &NameFields<T>) -> Option<String> {
    if let Some(name) = line.string(f.single) {
        return Some(name);
    }
    if let Some(org) = f.organization.and_then(|o| line.string(o)) {
        return Some(org);
    }
    let parts = [
        line.get(f.prefix),
        line.get(f.first),
        line.get(f.middle),
        line.get(f.last),
        line.get(f.suffix),
    ];
    let joined = parts.into_iter().flatten().collect::<Vec<_>>().join(" ");
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

// ---------------------------------------------------------------------------
// Code enums
// ---------------------------------------------------------------------------

/// FEC entity type code (`ENTITY TYPE` column on itemized schedules).
///
/// Real filings contain codes outside the documented set (typos, vendor
/// extensions, blank-but-not-empty). Rather than reject the line, unknown
/// codes are preserved verbatim in [`EntityType::Other`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum EntityType {
    /// `IND`
    Individual,
    /// `ORG` -- organization (not a committee)
    Organization,
    /// `CAN` -- candidate
    Candidate,
    /// `CCM` -- candidate committee
    CandidateCommittee,
    /// `COM` -- committee
    Committee,
    /// `PAC` -- political action committee
    Pac,
    /// `PTY` -- party organization
    Party,
    /// Any code not in the documented set, preserved as filed.
    Other(String),
}

impl EntityType {
    /// Parses an FEC entity-type code (case-insensitive, trimmed).
    #[must_use]
    pub fn from_code(code: &str) -> Self {
        match code.trim().to_ascii_uppercase().as_str() {
            "IND" => Self::Individual,
            "ORG" => Self::Organization,
            "CAN" => Self::Candidate,
            "CCM" => Self::CandidateCommittee,
            "COM" => Self::Committee,
            "PAC" => Self::Pac,
            "PTY" => Self::Party,
            other => Self::Other(other.to_string()),
        }
    }

    /// The FEC code, e.g. `"IND"`.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::Individual => "IND",
            Self::Organization => "ORG",
            Self::Candidate => "CAN",
            Self::CandidateCommittee => "CCM",
            Self::Committee => "COM",
            Self::Pac => "PAC",
            Self::Party => "PTY",
            Self::Other(s) => s,
        }
    }

    /// True for the documented codes, false for [`EntityType::Other`].
    #[must_use]
    pub fn is_known(&self) -> bool {
        !matches!(self, Self::Other(_))
    }
}

impl std::fmt::Display for EntityType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

/// Schedule E support/oppose indicator.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SupportOppose {
    /// `S`
    Support,
    /// `O`
    Oppose,
    /// Any other value, preserved as filed.
    Other(String),
}

impl SupportOppose {
    #[must_use]
    pub fn from_code(code: &str) -> Self {
        match code.trim().to_ascii_uppercase().as_str() {
            "S" => Self::Support,
            "O" => Self::Oppose,
            other => Self::Other(other.to_string()),
        }
    }

    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::Support => "S",
            Self::Oppose => "O",
            Self::Other(s) => s,
        }
    }

    #[must_use]
    pub fn is_known(&self) -> bool {
        !matches!(self, Self::Other(_))
    }
}

impl std::fmt::Display for SupportOppose {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

/// Ergonomic view over a Schedule A line (itemized receipts/contributions).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ScheduleA {
    pub filer_committee_id: String,
    pub transaction_id: Option<String>,
    pub entity_type: Option<EntityType>,
    pub contributor_name: Option<String>,
    pub contributor_city: Option<String>,
    pub contributor_state: Option<String>,
    pub contributor_zip_code: Option<String>,
    pub contributor_employer: Option<String>,
    pub contributor_occupation: Option<String>,
    pub contribution_date: Option<NaiveDate>,
    /// Exact contribution amount in dollars (see [`parse_money`]).
    pub contribution_amount: Option<Decimal>,
    /// Year-to-date (F3X) / cycle-to-date (F3, F3P) aggregate for this
    /// contributor, as reported by the filer.
    pub contribution_aggregate: Option<Decimal>,
    pub contribution_purpose_descrip: Option<String>,
    pub memo_code: Option<String>,
    pub memo_text_description: Option<String>,
}

impl TypedView for ScheduleA {
    type Marker = SchA;

    fn from_typed(l: Typed<'_, SchA>) -> Result<Self, TypedViewError> {
        Ok(ScheduleA {
            filer_committee_id: req(l, sch_a::FILER_COMMITTEE_ID_NUMBER)?,
            transaction_id: l.string(sch_a::TRANSACTION_ID),
            entity_type: l.get(sch_a::ENTITY_TYPE).map(EntityType::from_code),
            contributor_name: combined_name(
                l,
                &NameFields {
                    single: sch_a::CONTRIBUTOR_NAME,
                    organization: Some(sch_a::CONTRIBUTOR_ORGANIZATION_NAME),
                    prefix: sch_a::CONTRIBUTOR_PREFIX,
                    first: sch_a::CONTRIBUTOR_FIRST_NAME,
                    middle: sch_a::CONTRIBUTOR_MIDDLE_NAME,
                    last: sch_a::CONTRIBUTOR_LAST_NAME,
                    suffix: sch_a::CONTRIBUTOR_SUFFIX,
                },
            ),
            contributor_city: l.string(sch_a::CONTRIBUTOR_CITY),
            contributor_state: l.string(sch_a::CONTRIBUTOR_STATE),
            contributor_zip_code: l.string(sch_a::CONTRIBUTOR_ZIP_CODE),
            contributor_employer: l.string(sch_a::CONTRIBUTOR_EMPLOYER),
            contributor_occupation: l.string(sch_a::CONTRIBUTOR_OCCUPATION),
            contribution_date: l.date(sch_a::CONTRIBUTION_DATE),
            contribution_amount: l.money(sch_a::CONTRIBUTION_AMOUNT),
            contribution_aggregate: l.money(sch_a::CONTRIBUTION_AGGREGATE),
            contribution_purpose_descrip: l.string(sch_a::CONTRIBUTION_PURPOSE_DESCRIP),
            memo_code: l.string(sch_a::MEMO_CODE),
            memo_text_description: l.string(sch_a::MEMO_TEXT_DESCRIPTION),
        })
    }
}

/// Ergonomic view over a Schedule B line (itemized disbursements).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ScheduleB {
    pub filer_committee_id: String,
    pub transaction_id: Option<String>,
    pub entity_type: Option<EntityType>,
    pub payee_name: Option<String>,
    pub payee_city: Option<String>,
    pub payee_state: Option<String>,
    pub payee_zip_code: Option<String>,
    pub expenditure_date: Option<NaiveDate>,
    /// Exact expenditure amount in dollars (see [`parse_money`]).
    pub expenditure_amount: Option<Decimal>,
    pub expenditure_purpose_descrip: Option<String>,
    pub category_code: Option<String>,
    pub memo_code: Option<String>,
    pub memo_text_description: Option<String>,
}

impl TypedView for ScheduleB {
    type Marker = SchB;

    fn from_typed(l: Typed<'_, SchB>) -> Result<Self, TypedViewError> {
        Ok(ScheduleB {
            filer_committee_id: req(l, sch_b::FILER_COMMITTEE_ID_NUMBER)?,
            transaction_id: l.string(sch_b::TRANSACTION_ID_NUMBER),
            entity_type: l.get(sch_b::ENTITY_TYPE).map(EntityType::from_code),
            payee_name: combined_name(
                l,
                &NameFields {
                    single: sch_b::PAYEE_NAME,
                    organization: Some(sch_b::PAYEE_ORGANIZATION_NAME),
                    prefix: sch_b::PAYEE_PREFIX,
                    first: sch_b::PAYEE_FIRST_NAME,
                    middle: sch_b::PAYEE_MIDDLE_NAME,
                    last: sch_b::PAYEE_LAST_NAME,
                    suffix: sch_b::PAYEE_SUFFIX,
                },
            ),
            payee_city: l.string(sch_b::PAYEE_CITY),
            payee_state: l.string(sch_b::PAYEE_STATE),
            payee_zip_code: l.string(sch_b::PAYEE_ZIP_CODE),
            expenditure_date: l.date(sch_b::EXPENDITURE_DATE),
            expenditure_amount: l.money(sch_b::EXPENDITURE_AMOUNT),
            expenditure_purpose_descrip: l.string(sch_b::EXPENDITURE_PURPOSE_DESCRIP),
            category_code: l.string(sch_b::CATEGORY_CODE),
            memo_code: l.string(sch_b::MEMO_CODE),
            memo_text_description: l.string(sch_b::MEMO_TEXT_DESCRIPTION),
        })
    }
}

/// Ergonomic view over a Schedule E line (independent expenditures).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ScheduleE {
    pub filer_committee_id: String,
    pub transaction_id: Option<String>,
    pub payee_name: Option<String>,
    pub support_oppose: Option<SupportOppose>,
    pub candidate_id_number: Option<String>,
    pub candidate_name: Option<String>,
    pub candidate_office: Option<String>,
    pub candidate_state: Option<String>,
    pub candidate_district: Option<String>,
    pub dissemination_date: Option<NaiveDate>,
    pub disbursement_date: Option<NaiveDate>,
    /// Exact expenditure amount in dollars (see [`parse_money`]).
    pub expenditure_amount: Option<Decimal>,
    pub expenditure_purpose_descrip: Option<String>,
    pub memo_code: Option<String>,
    pub memo_text_description: Option<String>,
}

impl ScheduleE {
    /// The support/oppose indicator as its raw code (`"S"`, `"O"`, ...).
    #[must_use]
    pub fn support_oppose_code(&self) -> Option<&str> {
        self.support_oppose.as_ref().map(SupportOppose::code)
    }
}

impl TypedView for ScheduleE {
    type Marker = SchE;

    fn from_typed(l: Typed<'_, SchE>) -> Result<Self, TypedViewError> {
        Ok(ScheduleE {
            filer_committee_id: req(l, sch_e::FILER_COMMITTEE_ID_NUMBER)?,
            transaction_id: l.string(sch_e::TRANSACTION_ID_NUMBER),
            payee_name: combined_name(
                l,
                &NameFields {
                    single: sch_e::PAYEE_NAME,
                    organization: Some(sch_e::PAYEE_ORGANIZATION_NAME),
                    prefix: sch_e::PAYEE_PREFIX,
                    first: sch_e::PAYEE_FIRST_NAME,
                    middle: sch_e::PAYEE_MIDDLE_NAME,
                    last: sch_e::PAYEE_LAST_NAME,
                    suffix: sch_e::PAYEE_SUFFIX,
                },
            ),
            support_oppose: l
                .get(sch_e::SUPPORT_OPPOSE_CODE)
                .map(SupportOppose::from_code),
            candidate_id_number: l.string(sch_e::CANDIDATE_ID_NUMBER),
            // Schedule E has no `candidate_organization_name` (candidates
            // are always individuals).
            candidate_name: combined_name(
                l,
                &NameFields {
                    single: sch_e::CANDIDATE_NAME,
                    organization: None,
                    prefix: sch_e::CANDIDATE_PREFIX,
                    first: sch_e::CANDIDATE_FIRST_NAME,
                    middle: sch_e::CANDIDATE_MIDDLE_NAME,
                    last: sch_e::CANDIDATE_LAST_NAME,
                    suffix: sch_e::CANDIDATE_SUFFIX,
                },
            ),
            candidate_office: l.string(sch_e::CANDIDATE_OFFICE),
            candidate_state: l.string(sch_e::CANDIDATE_STATE),
            candidate_district: l.string(sch_e::CANDIDATE_DISTRICT),
            dissemination_date: l.date(sch_e::DISSEMINATION_DATE),
            disbursement_date: l.date(sch_e::DISBURSEMENT_DATE),
            expenditure_amount: l.money(sch_e::EXPENDITURE_AMOUNT),
            expenditure_purpose_descrip: l.string(sch_e::EXPENDITURE_PURPOSE_DESCRIP),
            memo_code: l.string(sch_e::MEMO_CODE),
            memo_text_description: l.string(sch_e::MEMO_TEXT_DESCRIPTION),
        })
    }
}

/// Ergonomic view over a Form 3X cover line (periodic report of a
/// PAC/party committee) -- the most commonly filed top-level form.
///
/// For every cover-page line (Column A and B), use
/// [`Filing::summary_as::<F3X>`](crate::Filing::summary_as) with the
/// constants in [`crate::parser::tables::f3x`]; this view carries the
/// headline figures.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Form3XSummary {
    pub filer_committee_id: String,
    pub committee_name: Option<String>,
    /// e.g. `"Q1"`, `"12G"`, `"YE"`.
    pub report_code: Option<String>,
    pub coverage_from_date: Option<NaiveDate>,
    pub coverage_through_date: Option<NaiveDate>,
    pub date_signed: Option<NaiveDate>,
    /// Column A line 6(b).
    pub cash_on_hand_beginning_period: Option<Decimal>,
    /// Column A line 8.
    pub cash_on_hand_close_of_period: Option<Decimal>,
    /// Column A line 6(c) ("this period") total receipts.
    pub total_receipts: Option<Decimal>,
    /// Column A line 7 total disbursements.
    pub total_disbursements: Option<Decimal>,
    /// Column A line 9.
    pub debts_owed_to_committee: Option<Decimal>,
    /// Column A line 10.
    pub debts_owed_by_committee: Option<Decimal>,
    /// Column B (year-to-date) total receipts.
    pub cycle_total_receipts: Option<Decimal>,
    /// Column B (year-to-date) total disbursements.
    pub cycle_total_disbursements: Option<Decimal>,
}

impl TypedView for Form3XSummary {
    type Marker = F3X;

    fn from_typed(l: Typed<'_, F3X>) -> Result<Self, TypedViewError> {
        Ok(Form3XSummary {
            filer_committee_id: req(l, f3x::FILER_COMMITTEE_ID_NUMBER)?,
            committee_name: l.string(f3x::COMMITTEE_NAME),
            report_code: l.string(f3x::REPORT_CODE),
            coverage_from_date: l.date(f3x::COVERAGE_FROM_DATE),
            coverage_through_date: l.date(f3x::COVERAGE_THROUGH_DATE),
            date_signed: l.date(f3x::DATE_SIGNED),
            cash_on_hand_beginning_period: l.money(f3x::COL_A_CASH_ON_HAND_BEGINNING_PERIOD),
            cash_on_hand_close_of_period: l.money(f3x::COL_A_CASH_ON_HAND_CLOSE_OF_PERIOD),
            total_receipts: l.money(f3x::COL_A_TOTAL_RECEIPTS),
            total_disbursements: l.money(f3x::COL_A_TOTAL_DISBURSEMENTS),
            debts_owed_to_committee: l.money(f3x::COL_A_DEBTS_TO),
            debts_owed_by_committee: l.money(f3x::COL_A_DEBTS_BY),
            cycle_total_receipts: l.money(f3x::COL_B_TOTAL_RECEIPTS),
            cycle_total_disbursements: l.money(f3x::COL_B_TOTAL_DISBURSEMENTS),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::schema::SpecVersion;
    use rust_decimal_macros::dec;

    fn line(table: Table, fields: &[(&str, &str)]) -> ParsedLine {
        ParsedLine::from_pairs(
            table,
            SpecVersion::electronic(8, 5),
            7,
            fields.iter().copied(),
        )
        .unwrap()
    }

    /// Spec 5.x layouts carry the combined `*_name` columns.
    fn line_v53(table: Table, fields: &[(&str, &str)]) -> ParsedLine {
        ParsedLine::from_pairs(
            table,
            SpecVersion::electronic(5, 3),
            7,
            fields.iter().copied(),
        )
        .unwrap()
    }

    #[test]
    fn parses_whole_dollar_amounts() {
        assert_eq!(parse_money("1000"), Some(dec!(1000.00)));
        assert_eq!(parse_money("0"), Some(dec!(0.00)));
        assert_eq!(parse_money("+5"), Some(dec!(5.00)));
    }

    #[test]
    fn parses_cents_exactly_without_float_error() {
        assert_eq!(parse_money("250.50"), Some(dec!(250.50)));
        assert_eq!(parse_money("0.01"), Some(dec!(0.01)));
        assert_eq!(parse_money("19.99"), Some(dec!(19.99)));
        assert_eq!(parse_money("100.1"), Some(dec!(100.10)));
        assert_eq!(parse_money(".5"), Some(dec!(0.50)));
        assert_eq!(parse_money("7."), Some(dec!(7.00)));
    }

    #[test]
    fn parses_negative_amounts() {
        assert_eq!(parse_money("-75"), Some(dec!(-75.00)));
        assert_eq!(parse_money("-75.25"), Some(dec!(-75.25)));
    }

    #[test]
    fn sums_many_amounts_with_zero_drift() {
        let raw = [
            "10.99", "25.49", "3.75", "88.01", "123.99", "0.45", "67.33", "9.99", "150.00", "2.33",
        ];
        let sum: Decimal = raw.iter().filter_map(|s| parse_money(s)).sum();
        assert_eq!(sum, dec!(482.33));
    }

    #[test]
    fn rejects_malformed_and_overflowing_amounts() {
        assert_eq!(parse_money(""), None);
        assert_eq!(parse_money("abc"), None);
        assert_eq!(parse_money("1.234"), None);
        assert_eq!(parse_money("1,000"), None);
        assert_eq!(parse_money("--5"), None);
        assert_eq!(parse_money("-"), None);
        assert_eq!(parse_money("."), None);
        // i64::MAX dollars * 100 overflows; must be None, not a panic.
        assert_eq!(parse_money("9223372036854775807"), None);
        assert_eq!(parse_money("-9223372036854775808.99"), None);
    }

    #[test]
    fn parses_fec_dates_and_rejects_short_ones() {
        assert_eq!(
            parse_fec_date("20260914"),
            NaiveDate::from_ymd_opt(2026, 9, 14)
        );
        assert_eq!(
            parse_fec_date(" 20260914 "),
            NaiveDate::from_ymd_opt(2026, 9, 14)
        );
        assert_eq!(parse_fec_date(""), None);
        assert_eq!(parse_fec_date("00000000"), None);
        assert_eq!(parse_fec_date("garbage"), None);
        assert_eq!(parse_fec_date("2026091"), None);
        assert_eq!(parse_fec_date("202609141"), None);
        assert_eq!(parse_fec_date("20261301"), None);
    }

    #[test]
    fn schedule_a_maps_fields_and_codes() {
        let l = line(
            Table::SchA,
            &[
                ("filer_committee_id_number", "C00123456"),
                ("entity_type", "ind"),
                ("contributor_last_name", "Smith"),
                ("contributor_first_name", "Jane"),
                ("contribution_date", "20260101"),
                ("contribution_amount", "500.00"),
                ("contribution_aggregate", "1250"),
            ],
        );
        let sa: ScheduleA = l.view().unwrap();
        assert_eq!(sa.filer_committee_id, "C00123456");
        assert_eq!(sa.entity_type, Some(EntityType::Individual));
        assert_eq!(sa.contributor_name.as_deref(), Some("Jane Smith"));
        assert_eq!(sa.contribution_amount, Some(dec!(500.00)));
        assert_eq!(sa.contribution_aggregate, Some(dec!(1250.00)));
        assert_eq!(sa.contribution_date, NaiveDate::from_ymd_opt(2026, 1, 1));
    }

    #[test]
    fn unknown_codes_are_preserved_not_rejected() {
        assert_eq!(EntityType::from_code("ind"), EntityType::Individual);
        assert_eq!(
            EntityType::from_code("XYZ"),
            EntityType::Other("XYZ".to_string())
        );
        assert!(!EntityType::from_code("XYZ").is_known());
        assert_eq!(EntityType::from_code("XYZ").code(), "XYZ");
        assert_eq!(SupportOppose::from_code("s"), SupportOppose::Support);
        assert_eq!(SupportOppose::from_code("O"), SupportOppose::Oppose);
        assert_eq!(
            SupportOppose::from_code("?"),
            SupportOppose::Other("?".to_string())
        );
    }

    #[test]
    fn missing_or_blank_required_field_errors_with_line_number() {
        let l = line(Table::SchA, &[("contributor_last_name", "X")]);
        match l.view::<ScheduleA>() {
            Err(TypedViewError::MissingField {
                table,
                field,
                line_no,
            }) => {
                assert_eq!(table, Table::SchA);
                assert_eq!(field, "filer_committee_id_number");
                assert_eq!(line_no, Some(7));
            }
            other => panic!("{other:?}"),
        }
        // Present but blank is also missing.
        let l = line(Table::SchA, &[("filer_committee_id_number", "")]);
        assert!(matches!(
            l.view::<ScheduleA>(),
            Err(TypedViewError::MissingField { .. })
        ));
    }

    #[test]
    fn wrong_table_is_refused() {
        // The exact misfiling this guards against: a Schedule A line has
        // a filer id, so a table-blind ScheduleE would "succeed".
        let l = line(Table::SchA, &[("filer_committee_id_number", "C00123456")]);
        match l.view::<ScheduleE>() {
            Err(TypedViewError::WrongTable {
                expected,
                found,
                line_no,
            }) => {
                assert_eq!(expected, Table::SchE);
                assert_eq!(found, Table::SchA);
                assert_eq!(line_no, 7);
            }
            other => panic!("{other:?}"),
        }
        assert!(l.view::<ScheduleA>().is_ok());
        assert_eq!(ScheduleE::TABLE, Table::SchE);
    }

    #[test]
    fn schedule_a_prefers_organization_name_when_no_combined_name() {
        let l = line(
            Table::SchA,
            &[
                ("filer_committee_id_number", "C00123456"),
                ("contributor_organization_name", "ACME WIDGETS PAC"),
                ("contributor_first_name", "IGNORED"),
            ],
        );
        let sa: ScheduleA = l.view().unwrap();
        assert_eq!(sa.contributor_name.as_deref(), Some("ACME WIDGETS PAC"));
    }

    #[test]
    fn schedule_a_uses_combined_name_on_older_versions() {
        let l = line_v53(
            Table::SchA,
            &[
                ("filer_committee_id_number", "C00123456"),
                ("contributor_name", "SMITH, JANE"),
            ],
        );
        let sa: ScheduleA = l.view().unwrap();
        assert_eq!(sa.contributor_name.as_deref(), Some("SMITH, JANE"));
    }

    #[test]
    fn schedule_e_candidate_name_falls_back_to_split_fields() {
        let l = line(
            Table::SchE,
            &[
                ("filer_committee_id_number", "C00123456"),
                ("candidate_first_name", "BERT"),
                ("candidate_last_name", "MIZUSAWA"),
                ("support_oppose_code", "S"),
            ],
        );
        let se: ScheduleE = l.view().unwrap();
        assert_eq!(se.candidate_name.as_deref(), Some("BERT MIZUSAWA"));
        assert_eq!(se.support_oppose, Some(SupportOppose::Support));
        assert_eq!(se.support_oppose_code(), Some("S"));
    }

    #[test]
    fn form3x_summary_from_summary_line() {
        let l = line(
            Table::F3X,
            &[
                ("filer_committee_id_number", "C00123456"),
                ("committee_name", "PAC"),
                ("col_a_total_receipts", "1000.50"),
                ("col_a_debts_by", "12"),
            ],
        );
        let s: Form3XSummary = l.view().unwrap();
        assert_eq!(s.total_receipts, Some(dec!(1000.50)));
        assert_eq!(s.debts_owed_by_committee, Some(dec!(12.00)));
        assert_eq!(s.cycle_total_receipts, None);
    }
}
