//! Ergonomic typed views over the highest-value schedules and forms, built
//! on top of the stringly-typed [`ParsedLine`] output.
//!
//! The raw layer hands back `IndexMap<String, String>`, which is faithful to
//! the wire format but tedious and error-prone to consume -- every caller
//! has to remember field names, re-parse dates and money by hand, and
//! re-derive things like "is this an individual or a committee contributor"
//! from raw entity-type codes. The [`TypedView`] trait and its
//! implementations here do that once, correctly.
//!
//! # Table safety
//!
//! Every typed view declares which [`Table`] it reads. [`TypedView::from_line`]
//! (reached via [`ParsedLine::view`] and [`Filing::views`](crate::Filing::views))
//! refuses a line from any other table with [`TypedViewError::WrongTable`].
//! Without that check, a table-blind `ScheduleE` conversion would happily
//! "succeed" on a Schedule A line --
//! every schedule carries `filer_committee_id_number` -- and produce an
//! all-`None` phantom expenditure. That misfiling was observed on 587 of
//! 587 non-Schedule-E lines of a real F3A before this guard existed.
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
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::parser::filing::ParsedLine;
use crate::parser::format_data::Table;

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
pub fn parse_money(raw: &str) -> Option<Decimal> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let negative = trimmed.starts_with('-');
    let unsigned = trimmed.trim_start_matches(['+', '-']);

    let (whole, frac) = match unsigned.split_once('.') {
        Some((w, f)) => (w, f),
        None => (unsigned, ""),
    };
    if frac.len() > 2
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !frac.chars().all(|c| c.is_ascii_digit())
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

/// A strongly-typed view over the fields of one [`Table`].
///
/// Implementors declare their table and how to build themselves from a
/// field map; [`from_line`](Self::from_line) adds the table check and
/// line-number context. Use [`ParsedLine::view`] or [`Filing::views`](crate::Filing::views)
/// rather than calling these directly.
pub trait TypedView: Sized {
    /// The format table this view reads.
    const TABLE: Table;

    /// Builds the view from a field map assumed to come from
    /// [`Self::TABLE`]. Missing optional fields become `None`; only a
    /// genuinely required field (the filer id) is an error.
    fn from_fields(fields: &IndexMap<String, String>) -> Result<Self, TypedViewError>;

    /// Builds the view from a parsed line, refusing lines from other tables.
    fn from_line(line: &ParsedLine) -> Result<Self, TypedViewError> {
        if line.table != Self::TABLE {
            return Err(TypedViewError::WrongTable {
                expected: Self::TABLE,
                found: line.table,
                line_no: line.line_no,
            });
        }
        Self::from_fields(&line.fields).map_err(|e| e.with_line(line.line_no))
    }
}

/// Non-empty string field, or `None`.
fn opt(fields: &IndexMap<String, String>, name: &str) -> Option<String> {
    fields
        .get(name)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Required non-empty string field.
fn req(
    fields: &IndexMap<String, String>,
    table: Table,
    name: &'static str,
) -> Result<String, TypedViewError> {
    opt(fields, name).ok_or(TypedViewError::MissingField {
        table,
        field: name,
        line_no: None,
    })
}

fn date(fields: &IndexMap<String, String>, name: &str) -> Option<NaiveDate> {
    fields.get(name).and_then(|s| parse_fec_date(s))
}

fn money(fields: &IndexMap<String, String>, name: &str) -> Option<Decimal> {
    fields.get(name).and_then(|s| parse_money(s))
}

/// Field names for the name-shaped columns a schedule/form may use --
/// see [`combined_name`] for why both shapes need to be checked. Pass
/// `""` for `organization` on schedules that have no such column (e.g. a
/// Schedule E candidate, who is always an individual).
struct NameFields {
    single: &'static str,
    organization: &'static str,
    prefix: &'static str,
    first: &'static str,
    middle: &'static str,
    last: &'static str,
    suffix: &'static str,
}

/// Resolves a display name from the fields FEC uses across spec versions.
///
/// Older spec versions (roughly pre-8.0) report a single pre-joined name
/// field (e.g. `contributor_name`, `payee_name`, `candidate_name`).
/// Current spec versions (8.0 and later, which is what real-world filings
/// use today) drop that combined field entirely and instead split the name
/// into `*_organization_name` (for a committee/business payee or
/// contributor) or `*_prefix`/`*_first_name`/`*_middle_name`/`*_last_name`/
/// `*_suffix` (for a person). Without this fallback, every typed-view name
/// field would be `None` for virtually all present-day filings even though
/// the name is right there in the raw line.
fn combined_name(fields: &IndexMap<String, String>, f: NameFields) -> Option<String> {
    if let Some(name) = opt(fields, f.single) {
        return Some(name);
    }
    if !f.organization.is_empty()
        && let Some(org) = opt(fields, f.organization)
    {
        return Some(org);
    }
    let parts = [
        opt(fields, f.prefix),
        opt(fields, f.first),
        opt(fields, f.middle),
        opt(fields, f.last),
        opt(fields, f.suffix),
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
    pub fn from_code(code: &str) -> Self {
        match code.trim().to_ascii_uppercase().as_str() {
            "S" => Self::Support,
            "O" => Self::Oppose,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn code(&self) -> &str {
        match self {
            Self::Support => "S",
            Self::Oppose => "O",
            Self::Other(s) => s,
        }
    }

    pub fn is_known(&self) -> bool {
        !matches!(self, Self::Other(_))
    }
}

impl std::fmt::Display for SupportOppose {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

fn entity_type(fields: &IndexMap<String, String>) -> Option<EntityType> {
    opt(fields, "entity_type").map(|c| EntityType::from_code(&c))
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
    pub contribution_purpose_descrip: Option<String>,
    pub memo_code: Option<String>,
    pub memo_text_description: Option<String>,
}

impl TypedView for ScheduleA {
    const TABLE: Table = Table::SchA;

    fn from_fields(fields: &IndexMap<String, String>) -> Result<Self, TypedViewError> {
        Ok(ScheduleA {
            filer_committee_id: req(fields, Self::TABLE, "filer_committee_id_number")?,
            transaction_id: opt(fields, "transaction_id"),
            entity_type: entity_type(fields),
            contributor_name: combined_name(
                fields,
                NameFields {
                    single: "contributor_name",
                    organization: "contributor_organization_name",
                    prefix: "contributor_prefix",
                    first: "contributor_first_name",
                    middle: "contributor_middle_name",
                    last: "contributor_last_name",
                    suffix: "contributor_suffix",
                },
            ),
            contributor_city: opt(fields, "contributor_city"),
            contributor_state: opt(fields, "contributor_state"),
            contributor_zip_code: opt(fields, "contributor_zip_code"),
            contributor_employer: opt(fields, "contributor_employer"),
            contributor_occupation: opt(fields, "contributor_occupation"),
            contribution_date: date(fields, "contribution_date"),
            contribution_amount: money(fields, "contribution_amount"),
            contribution_purpose_descrip: opt(fields, "contribution_purpose_descrip"),
            memo_code: opt(fields, "memo_code"),
            memo_text_description: opt(fields, "memo_text_description"),
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
    const TABLE: Table = Table::SchB;

    fn from_fields(fields: &IndexMap<String, String>) -> Result<Self, TypedViewError> {
        Ok(ScheduleB {
            filer_committee_id: req(fields, Self::TABLE, "filer_committee_id_number")?,
            transaction_id: opt(fields, "transaction_id_number"),
            entity_type: entity_type(fields),
            payee_name: combined_name(fields, PAYEE_NAME),
            payee_city: opt(fields, "payee_city"),
            payee_state: opt(fields, "payee_state"),
            payee_zip_code: opt(fields, "payee_zip_code"),
            expenditure_date: date(fields, "expenditure_date"),
            expenditure_amount: money(fields, "expenditure_amount"),
            expenditure_purpose_descrip: opt(fields, "expenditure_purpose_descrip"),
            category_code: opt(fields, "category_code"),
            memo_code: opt(fields, "memo_code"),
            memo_text_description: opt(fields, "memo_text_description"),
        })
    }
}

const PAYEE_NAME: NameFields = NameFields {
    single: "payee_name",
    organization: "payee_organization_name",
    prefix: "payee_prefix",
    first: "payee_first_name",
    middle: "payee_middle_name",
    last: "payee_last_name",
    suffix: "payee_suffix",
};

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
    pub fn support_oppose_code(&self) -> Option<&str> {
        self.support_oppose.as_ref().map(SupportOppose::code)
    }
}

impl TypedView for ScheduleE {
    const TABLE: Table = Table::SchE;

    fn from_fields(fields: &IndexMap<String, String>) -> Result<Self, TypedViewError> {
        Ok(ScheduleE {
            filer_committee_id: req(fields, Self::TABLE, "filer_committee_id_number")?,
            transaction_id: opt(fields, "transaction_id_number"),
            payee_name: combined_name(fields, PAYEE_NAME),
            support_oppose: opt(fields, "support_oppose_code")
                .map(|c| SupportOppose::from_code(&c)),
            candidate_id_number: opt(fields, "candidate_id_number"),
            // Schedule E has no `candidate_organization_name` (candidates
            // are always individuals), so the organization slot is empty.
            candidate_name: combined_name(
                fields,
                NameFields {
                    single: "candidate_name",
                    organization: "",
                    prefix: "candidate_prefix",
                    first: "candidate_first_name",
                    middle: "candidate_middle_name",
                    last: "candidate_last_name",
                    suffix: "candidate_suffix",
                },
            ),
            candidate_office: opt(fields, "candidate_office"),
            candidate_state: opt(fields, "candidate_state"),
            candidate_district: opt(fields, "candidate_district"),
            dissemination_date: date(fields, "dissemination_date"),
            disbursement_date: date(fields, "disbursement_date"),
            expenditure_amount: money(fields, "expenditure_amount"),
            expenditure_purpose_descrip: opt(fields, "expenditure_purpose_descrip"),
            memo_code: opt(fields, "memo_code"),
            memo_text_description: opt(fields, "memo_text_description"),
        })
    }
}

/// Ergonomic view over a Form 3X summary/cover line (periodic report of a
/// PAC/party committee) -- the most commonly filed top-level form.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Form3XSummary {
    pub filer_committee_id: String,
    pub committee_name: Option<String>,
    pub report_code: Option<String>,
    pub coverage_from_date: Option<NaiveDate>,
    pub coverage_through_date: Option<NaiveDate>,
    /// Column A ("this period") cash on hand at close of period.
    pub cash_on_hand_close_of_period: Option<Decimal>,
    /// Column A total receipts this period.
    pub total_receipts: Option<Decimal>,
    /// Column A total disbursements this period.
    pub total_disbursements: Option<Decimal>,
    /// Column B (election-cycle-to-date) total receipts.
    pub cycle_total_receipts: Option<Decimal>,
    /// Column B (election-cycle-to-date) total disbursements.
    pub cycle_total_disbursements: Option<Decimal>,
}

impl TypedView for Form3XSummary {
    const TABLE: Table = Table::F3X;

    fn from_fields(fields: &IndexMap<String, String>) -> Result<Self, TypedViewError> {
        Ok(Form3XSummary {
            filer_committee_id: req(fields, Self::TABLE, "filer_committee_id_number")?,
            committee_name: opt(fields, "committee_name"),
            report_code: opt(fields, "report_code"),
            coverage_from_date: date(fields, "coverage_from_date"),
            coverage_through_date: date(fields, "coverage_through_date"),
            cash_on_hand_close_of_period: money(fields, "col_a_cash_on_hand_close_of_period"),
            total_receipts: money(fields, "col_a_total_receipts"),
            total_disbursements: money(fields, "col_a_total_disbursements"),
            cycle_total_receipts: money(fields, "col_b_total_receipts"),
            cycle_total_disbursements: money(fields, "col_b_total_disbursements"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn line(table: Table, fields: &[(&str, &str)]) -> ParsedLine {
        let map: IndexMap<String, String> = fields
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        ParsedLine::new(table.as_str(), table, 7, map)
    }

    #[test]
    fn parses_whole_dollar_amounts() {
        assert_eq!(parse_money("1000"), Some(dec!(1000.00)));
        assert_eq!(parse_money("0"), Some(dec!(0.00)));
    }

    #[test]
    fn parses_cents_exactly_without_float_error() {
        assert_eq!(parse_money("250.50"), Some(dec!(250.50)));
        assert_eq!(parse_money("0.01"), Some(dec!(0.01)));
        assert_eq!(parse_money("19.99"), Some(dec!(19.99)));
        assert_eq!(parse_money("100.1"), Some(dec!(100.10)));
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
                ("entity_type", "IND"),
                ("contributor_name", "SMITH, JANE"),
                ("contribution_date", "20260101"),
                ("contribution_amount", "500.00"),
            ],
        );
        let sa: ScheduleA = l.view().unwrap();
        assert_eq!(sa.filer_committee_id, "C00123456");
        assert_eq!(sa.entity_type, Some(EntityType::Individual));
        assert_eq!(sa.contributor_name.as_deref(), Some("SMITH, JANE"));
        assert_eq!(sa.contribution_amount, Some(dec!(500.00)));
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
        let l = line(Table::SchA, &[("contributor_name", "X")]);
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
    fn schedule_a_falls_back_to_split_individual_name_for_modern_spec_versions() {
        let l = line(
            Table::SchA,
            &[
                ("filer_committee_id_number", "C00123456"),
                ("contributor_first_name", "JANE"),
                ("contributor_last_name", "SMITH"),
            ],
        );
        let sa: ScheduleA = l.view().unwrap();
        assert_eq!(sa.contributor_name.as_deref(), Some("JANE SMITH"));
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
            ],
        );
        let s: Form3XSummary = l.view().unwrap();
        assert_eq!(s.total_receipts, Some(dec!(1000.50)));
        let s2 = Form3XSummary::from_fields(&l.fields).unwrap();
        assert_eq!(s, s2);
    }
}
