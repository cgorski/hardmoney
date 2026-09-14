//! Ergonomic typed views over the highest-value schedules and forms, built
//! on top of the stringly-typed [`crate::parser::filing::ParsedLine`] output.
//!
//! This is the crate's answer to "not just a straight port": pyfec (and the
//! raw layer below this module) hands back `IndexMap<String, String>`,
//! which is faithful to the wire format but tedious and error-prone to
//! consume -- every caller has to remember field names, re-parse dates and
//! money by hand, and re-derive things like "is this an individual or a
//! committee contributor" from raw entity-type codes.
//!
//! Money is parsed with [`parse_money`] into a [`Decimal`] rather than
//! `f64`, because FEC amounts are decimal currency values and binary
//! floating point cannot represent every decimal fraction exactly (`f64`
//! would silently corrupt sums across millions of transactions, and would
//! silently corrupt individual values once amounts get large enough --
//! `Decimal` never does either, no matter the value). This exactness is
//! carried all the way through the crate: the same `Decimal` type is what
//! gets written to and read back from Postgres's `NUMERIC` columns (via
//! `sqlx`'s native `rust_decimal` support) and what the REST API returns
//! in JSON, so money is never round-tripped through `f64` at any layer.

use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::parser::filing::ParsedLine;

/// Parses an FEC amount field (e.g. `"1000"`, `"250.50"`, `"-75"`) into an
/// exact [`Decimal`]. Returns `None` for blank or unparseable input.
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
        1 => frac.parse::<i64>().ok()? * 10,
        2 => frac.parse().ok()?,
        _ => unreachable!("checked above"),
    };

    let total_cents = whole_cents.checked_mul(100)?.checked_add(frac_cents)?;
    let total_cents = if negative { -total_cents } else { total_cents };
    // `Decimal::new(value, scale)` represents `value * 10^-scale` exactly,
    // with no intermediate float -- this is the entire reason to build the
    // amount from validated cents rather than parsing the string directly
    // with `Decimal::from_str`.
    Some(Decimal::new(total_cents, 2))
}

/// Parses an FEC date field (`YYYYMMDD`) into a [`NaiveDate`]. Returns
/// `None` for blank, all-zero, or unparseable input -- FEC filings
/// routinely leave optional dates blank or zero-filled rather than
/// omitting the column.
pub fn parse_fec_date(raw: &str) -> Option<NaiveDate> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().all(|c| c == '0') {
        return None;
    }
    NaiveDate::parse_from_str(trimmed, "%Y%m%d").ok()
}

/// Error returned when a [`ParsedLine`] doesn't carry the fields a typed
/// view requires -- e.g. calling [`ScheduleA::try_from`] on a line that
/// dispatched to a different table.
#[derive(Debug, thiserror::Error)]
#[error("line (table={table}) is missing required field '{field}' for this typed view")]
pub struct TypedViewError {
    pub table: &'static str,
    pub field: &'static str,
}

fn field<'a>(line: &'a ParsedLine, name: &'static str) -> Result<&'a str, TypedViewError> {
    line.get(name).ok_or(TypedViewError {
        table: line.table,
        field: name,
    })
}

fn non_empty(line: &ParsedLine, name: &str) -> Option<String> {
    line.get(name).filter(|s| !s.is_empty()).map(str::to_string)
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
/// the name is right there in the raw line -- so this checks both shapes
/// to give a usable name across spec versions.
fn combined_name(line: &ParsedLine, f: NameFields) -> Option<String> {
    if let Some(name) = non_empty(line, f.single) {
        return Some(name);
    }
    if let Some(org) = non_empty(line, f.organization) {
        return Some(org);
    }
    let parts = [
        non_empty(line, f.prefix),
        non_empty(line, f.first),
        non_empty(line, f.middle),
        non_empty(line, f.last),
        non_empty(line, f.suffix),
    ];
    let joined = parts.into_iter().flatten().collect::<Vec<_>>().join(" ");
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

/// Ergonomic view over a Schedule A line (itemized receipts/contributions).
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleA {
    pub filer_committee_id: String,
    pub transaction_id: Option<String>,
    pub entity_type: Option<String>,
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

impl TryFrom<&ParsedLine> for ScheduleA {
    type Error = TypedViewError;

    fn try_from(line: &ParsedLine) -> Result<Self, Self::Error> {
        Ok(ScheduleA {
            filer_committee_id: field(line, "filer_committee_id_number")?.to_string(),
            transaction_id: line
                .get("transaction_id")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            entity_type: line
                .get("entity_type")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            contributor_name: combined_name(
                line,
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
            contributor_city: line
                .get("contributor_city")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            contributor_state: line
                .get("contributor_state")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            contributor_zip_code: line
                .get("contributor_zip_code")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            contributor_employer: line
                .get("contributor_employer")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            contributor_occupation: line
                .get("contributor_occupation")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            contribution_date: line.get("contribution_date").and_then(parse_fec_date),
            contribution_amount: line.get("contribution_amount").and_then(parse_money),
            contribution_purpose_descrip: line
                .get("contribution_purpose_descrip")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            memo_code: line
                .get("memo_code")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            memo_text_description: line
                .get("memo_text_description")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        })
    }
}

/// Ergonomic view over a Schedule B line (itemized disbursements).
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleB {
    pub filer_committee_id: String,
    pub transaction_id: Option<String>,
    pub entity_type: Option<String>,
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

impl TryFrom<&ParsedLine> for ScheduleB {
    type Error = TypedViewError;

    fn try_from(line: &ParsedLine) -> Result<Self, Self::Error> {
        Ok(ScheduleB {
            filer_committee_id: field(line, "filer_committee_id_number")?.to_string(),
            transaction_id: line
                .get("transaction_id_number")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            entity_type: line
                .get("entity_type")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            payee_name: combined_name(
                line,
                NameFields {
                    single: "payee_name",
                    organization: "payee_organization_name",
                    prefix: "payee_prefix",
                    first: "payee_first_name",
                    middle: "payee_middle_name",
                    last: "payee_last_name",
                    suffix: "payee_suffix",
                },
            ),
            payee_city: line
                .get("payee_city")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            payee_state: line
                .get("payee_state")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            payee_zip_code: line
                .get("payee_zip_code")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            expenditure_date: line.get("expenditure_date").and_then(parse_fec_date),
            expenditure_amount: line.get("expenditure_amount").and_then(parse_money),
            expenditure_purpose_descrip: line
                .get("expenditure_purpose_descrip")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            category_code: line
                .get("category_code")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            memo_code: line
                .get("memo_code")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            memo_text_description: line
                .get("memo_text_description")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        })
    }
}

/// Ergonomic view over a Schedule E line (independent expenditures).
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleE {
    pub filer_committee_id: String,
    pub transaction_id: Option<String>,
    pub payee_name: Option<String>,
    pub support_oppose_code: Option<String>,
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

impl TryFrom<&ParsedLine> for ScheduleE {
    type Error = TypedViewError;

    fn try_from(line: &ParsedLine) -> Result<Self, Self::Error> {
        Ok(ScheduleE {
            filer_committee_id: field(line, "filer_committee_id_number")?.to_string(),
            transaction_id: line
                .get("transaction_id_number")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            payee_name: combined_name(
                line,
                NameFields {
                    single: "payee_name",
                    organization: "payee_organization_name",
                    prefix: "payee_prefix",
                    first: "payee_first_name",
                    middle: "payee_middle_name",
                    last: "payee_last_name",
                    suffix: "payee_suffix",
                },
            ),
            support_oppose_code: line
                .get("support_oppose_code")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            candidate_id_number: line
                .get("candidate_id_number")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            // Schedule E has no `candidate_organization_name` (candidates
            // are always individuals), so the organization slot is an
            // empty field name that never matches anything.
            candidate_name: combined_name(
                line,
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
            candidate_office: line
                .get("candidate_office")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            candidate_state: line
                .get("candidate_state")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            candidate_district: line
                .get("candidate_district")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            dissemination_date: line.get("dissemination_date").and_then(parse_fec_date),
            disbursement_date: line.get("disbursement_date").and_then(parse_fec_date),
            expenditure_amount: line.get("expenditure_amount").and_then(parse_money),
            expenditure_purpose_descrip: line
                .get("expenditure_purpose_descrip")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            memo_code: line
                .get("memo_code")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            memo_text_description: line
                .get("memo_text_description")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        })
    }
}

/// Ergonomic view over a Form 3X summary/cover line (periodic report of a
/// PAC/party committee) -- the most commonly filed top-level form.
#[derive(Debug, Clone, PartialEq)]
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

impl TryFrom<&indexmap::IndexMap<String, String>> for Form3XSummary {
    type Error = TypedViewError;

    fn try_from(fields: &indexmap::IndexMap<String, String>) -> Result<Self, Self::Error> {
        let get = |name: &str| fields.get(name).map(|s| s.as_str());
        let filer_committee_id = get("filer_committee_id_number")
            .filter(|s| !s.is_empty())
            .ok_or(TypedViewError {
                table: "F3X",
                field: "filer_committee_id_number",
            })?
            .to_string();

        Ok(Form3XSummary {
            filer_committee_id,
            committee_name: get("committee_name")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            report_code: get("report_code")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            coverage_from_date: get("coverage_from_date").and_then(parse_fec_date),
            coverage_through_date: get("coverage_through_date").and_then(parse_fec_date),
            cash_on_hand_close_of_period: get("col_a_cash_on_hand_close_of_period")
                .and_then(parse_money),
            total_receipts: get("col_a_total_receipts").and_then(parse_money),
            total_disbursements: get("col_a_total_disbursements").and_then(parse_money),
            cycle_total_receipts: get("col_b_total_receipts").and_then(parse_money),
            cycle_total_disbursements: get("col_b_total_disbursements").and_then(parse_money),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use rust_decimal_macros::dec;

    #[test]
    fn parses_whole_dollar_amounts() {
        assert_eq!(parse_money("1000"), Some(dec!(1000.00)));
        assert_eq!(parse_money("0"), Some(dec!(0.00)));
    }

    #[test]
    fn parses_cents_exactly_without_float_error() {
        // These are exact `Decimal` equality checks, not `f64` ones -- if
        // `parse_money` ever regressed to building the amount via
        // `whole as f64 + frac as f64 / 100.0` instead of assembling exact
        // scaled integers, values like `19.99` (which has no exact binary
        // floating-point representation) would fail this comparison.
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
        // The property that actually matters for real filings: summing a
        // long run of parsed amounts must reproduce the exact integer-cent
        // total, with no accumulated rounding drift. `f64` is not
        // guaranteed to hold this property in general (individual amounts
        // like `19.99` are already inexact in binary before any summing
        // happens); `Decimal` holds it by construction, since each value
        // is an exact multiple of one cent.
        let raw = [
            "10.99", "25.49", "3.75", "88.01", "123.99", "0.45", "67.33", "9.99", "150.00", "2.33",
        ];
        let sum: Decimal = raw.iter().filter_map(|s| parse_money(s)).sum();
        assert_eq!(sum, dec!(482.33));
    }

    #[test]
    fn rejects_malformed_amounts() {
        assert_eq!(parse_money(""), None);
        assert_eq!(parse_money("abc"), None);
        assert_eq!(parse_money("1.234"), None);
        assert_eq!(parse_money("1,000"), None);
    }

    #[test]
    fn parses_fec_dates() {
        assert_eq!(
            parse_fec_date("20260914"),
            NaiveDate::from_ymd_opt(2026, 9, 14)
        );
        assert_eq!(parse_fec_date(""), None);
        assert_eq!(parse_fec_date("00000000"), None);
        assert_eq!(parse_fec_date("garbage"), None);
    }

    #[test]
    fn schedule_a_try_from_maps_fields() {
        let mut fields = IndexMap::new();
        fields.insert(
            "filer_committee_id_number".to_string(),
            "C00123456".to_string(),
        );
        fields.insert("contributor_name".to_string(), "SMITH, JANE".to_string());
        fields.insert("contribution_date".to_string(), "20260101".to_string());
        fields.insert("contribution_amount".to_string(), "500.00".to_string());
        let line = ParsedLine {
            raw_form_type: "SA11AI".to_string(),
            table: "SchA",
            fields,
        };

        let sa = ScheduleA::try_from(&line).unwrap();
        assert_eq!(sa.filer_committee_id, "C00123456");
        assert_eq!(sa.contributor_name.as_deref(), Some("SMITH, JANE"));
        assert_eq!(sa.contribution_amount, Some(dec!(500.00)));
        assert_eq!(sa.contribution_date, NaiveDate::from_ymd_opt(2026, 1, 1));
    }

    #[test]
    fn schedule_a_try_from_errors_on_missing_required_field() {
        let line = ParsedLine {
            raw_form_type: "SA11AI".to_string(),
            table: "SchA",
            fields: IndexMap::new(),
        };
        assert!(ScheduleA::try_from(&line).is_err());
    }

    // Current spec versions (8.0+, what real-world filings use today) never
    // populate the single combined `contributor_name`/`payee_name`/
    // `candidate_name` fields -- they only ever populate the split
    // organization/last/first/middle fields. Without `combined_name`
    // falling back to those, every typed-view name would silently be
    // `None` for present-day filings. These tests cover both shapes that
    // split form can take: an organization, and an individual.
    #[test]
    fn schedule_a_falls_back_to_organization_name_for_modern_spec_versions() {
        let mut fields = IndexMap::new();
        fields.insert(
            "filer_committee_id_number".to_string(),
            "C00123456".to_string(),
        );
        fields.insert(
            "contributor_organization_name".to_string(),
            "ACME WIDGETS PAC".to_string(),
        );
        let line = ParsedLine {
            raw_form_type: "SA11AI".to_string(),
            table: "SchA",
            fields,
        };

        let sa = ScheduleA::try_from(&line).unwrap();
        assert_eq!(sa.contributor_name.as_deref(), Some("ACME WIDGETS PAC"));
    }

    #[test]
    fn schedule_a_falls_back_to_split_individual_name_for_modern_spec_versions() {
        let mut fields = IndexMap::new();
        fields.insert(
            "filer_committee_id_number".to_string(),
            "C00123456".to_string(),
        );
        fields.insert("contributor_first_name".to_string(), "JANE".to_string());
        fields.insert("contributor_last_name".to_string(), "SMITH".to_string());
        let line = ParsedLine {
            raw_form_type: "SA11AI".to_string(),
            table: "SchA",
            fields,
        };

        let sa = ScheduleA::try_from(&line).unwrap();
        assert_eq!(sa.contributor_name.as_deref(), Some("JANE SMITH"));
    }

    #[test]
    fn schedule_e_candidate_name_falls_back_to_split_fields() {
        let mut fields = IndexMap::new();
        fields.insert(
            "filer_committee_id_number".to_string(),
            "C00123456".to_string(),
        );
        fields.insert("candidate_first_name".to_string(), "BERT".to_string());
        fields.insert("candidate_last_name".to_string(), "MIZUSAWA".to_string());
        let line = ParsedLine {
            raw_form_type: "SE".to_string(),
            table: "SchE",
            fields,
        };

        let se = ScheduleE::try_from(&line).unwrap();
        assert_eq!(se.candidate_name.as_deref(), Some("BERT MIZUSAWA"));
    }
}
