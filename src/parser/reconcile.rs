//! `Filing::reconcile`: does the cover page match the schedules?
//!
//! Every periodic report's cover page states totals -- "Line 11(a)(i):
//! itemized individual contributions = $123,456.78" -- and each such line
//! is supposed to equal either the sum of the matching schedule lines in
//! the same file, or an arithmetic combination of other cover lines. This
//! module recomputes every line with exact [`Decimal`] arithmetic and
//! reports `reported`, `expected`, and `delta` per line. It is the first
//! pass a Reports Analysis Division analyst makes on every report; the
//! FEC's own validator only *warns* about it ("Subtotal … not supported by
//! Schedule"), and its new filing tool computes several lines as zero.
//!
//! # The rules
//!
//! A [`LineRule`] ties one cover-page line (its FEC label, e.g. `11(a)(i)`,
//! and its canonical field, e.g. `col_a_individuals_itemized`) to a
//! [`Source`]:
//!
//! * [`Source::Schedules`] -- the line is the sum of one amount field over
//!   the body lines whose form-type token is one of the listed tokens
//!   (e.g. `SA11AI`), **excluding memo entries**. Forgetting the memo
//!   exclusion is the single most common reason recomputed totals do not
//!   match a filing. Line 9/10 (debts) sum a Schedule C balance *and* a
//!   Schedule D balance, so a source may name several schedules.
//! * [`Source::Formula`] -- the line is an arithmetic combination of other
//!   cover lines, evaluated over the *reported* values of those lines. That
//!   isolates which line is wrong: if 11(a)(i) disagrees with Schedule A
//!   but 11(a)(iii) = 11(a)(i) + 11(a)(ii) holds, the cover page is
//!   internally consistent and the discrepancy is between the cover and
//!   the schedule, not inside the cover.
//!
//! The rule text comes from the FEC's format specification (the
//! `RULE REFERENCE` column of each form's sheet, reproduced in
//! [`FieldSpec::rule`](crate::parser::FieldSpec::rule)) and from the
//! FEC's own reference implementation in FECfile+; the tables here cover
//! Form 3X (PAC/party), Form 3 (House/Senate), and Form 3P (presidential),
//! Column A (this period) with schedule sums and formulas, and Column B
//! (year/cycle to date) with formulas only -- Column B sums span prior
//! reports and cannot be recomputed from one file.
//!
//! Unitemized lines (`11(a)(ii)`, `17(a)(ii)`) have no schedule: by
//! definition they are the contributions too small to itemize, so they are
//! checked only through the formulas that include them. Likewise, lines
//! whose transactions only need itemizing above the $200 aggregate
//! threshold (operating expenditures, offsets, other receipts and
//! disbursements, refunds to individuals) are checked as *floors*: the
//! cover total must be at least the itemized sum. See [`Relation`].
//!
//! # Tolerance
//!
//! Comparisons are exact by default. [`Reconciliation::mismatches_over`]
//! applies a tolerance (e.g. `0.01` for a filer who rounds each line).

use std::fmt;

use rust_decimal::Decimal;

use crate::parser::filing::{Filing, ParsedLine};
use crate::parser::tables::Table;
use crate::parser::typed::parse_money;

/// Which cover-page column a check applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, strum::Display)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Column {
    /// This reporting period.
    A,
    /// Year to date (F3X) / election cycle to date (F3, F3P).
    B,
}

/// One schedule contribution to a cover line: sum `field` over body lines
/// of `table` whose (upper-cased) form-type token is in `tokens`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleSum {
    pub table: Table,
    pub tokens: &'static [&'static str],
    pub field: &'static str,
}

/// A term of a [`Source::Formula`]: `+line` or `-line`, by FEC label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Term {
    pub line: &'static str,
    pub negative: bool,
}

/// How a cover line must relate to its schedule sum.
///
/// Federal law only requires *itemizing* a receipt or disbursement once
/// the payee/contributor aggregate exceeds $200 in the cycle (11 CFR
/// 104.3); anything smaller is reported in the cover total but need not
/// appear on the schedule. So for those lines the schedule sum is a
/// **floor**, not an identity -- a cover total below its itemized sum is a
/// discrepancy, a cover total above it is normal. Lines that must be fully
/// itemized regardless of size (contributions from committees, loans,
/// transfers, independent expenditures, debts) must match **exactly**.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[strum(serialize_all = "snake_case")]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Relation {
    /// `reported == expected`.
    Equal,
    /// `reported >= expected`: sub-threshold items may be unitemized.
    AtLeast,
}

/// How a cover-page line's expected value is derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Source {
    /// Sum of schedule amounts (memo entries excluded), related to the
    /// cover line as `relation` says.
    Schedules(Relation, &'static [ScheduleSum]),
    /// Signed sum of other cover lines' *reported* values.
    Formula(&'static [Term]),
    /// No rule: an input to other lines' formulas (cash on hand brought
    /// forward; a Column B total whose sum spans prior reports). No
    /// [`LineCheck`] is produced for it.
    Input,
}

/// One cover-page line and the rule that determines its expected value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRule {
    /// The FEC's line label, e.g. `"11(a)(i)"`, `"6(c)"`, `"31"`.
    pub line: &'static str,
    /// The canonical cover-page field holding the reported value.
    pub field: &'static str,
    pub column: Column,
    pub source: Source,
}

impl LineRule {
    /// The rule as the FEC writes it, e.g. `= 11ai + 11aii` or
    /// `= Total on Sch A (SA11AI)`.
    #[must_use]
    pub fn formula_text(&self) -> String {
        match self.source {
            Source::Formula(terms) => {
                let mut s = String::from("=");
                for (i, t) in terms.iter().enumerate() {
                    if t.negative {
                        s.push_str(" - ");
                    } else if i == 0 {
                        s.push(' ');
                    } else {
                        s.push_str(" + ");
                    }
                    s.push_str(t.line);
                }
                s
            }
            Source::Input => "(input)".to_string(),
            Source::Schedules(relation, sums) => {
                let parts: Vec<String> = sums
                    .iter()
                    .map(|s| format!("{}.{} on {}", s.table, s.field, s.tokens.join("/")))
                    .collect();
                let op = match relation {
                    Relation::Equal => "=",
                    Relation::AtLeast => ">=",
                };
                format!("{op} sum of {}", parts.join(" + "))
            }
        }
    }
}

/// The outcome of checking one cover-page line.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LineCheck {
    pub line: &'static str,
    pub field: &'static str,
    pub column: Column,
    /// The rule in the FEC's notation.
    pub rule: String,
    /// The value on the cover page; `None` if blank (treated as 0 in `delta`).
    pub reported: Option<Decimal>,
    /// The value the schedules/formula imply.
    pub expected: Decimal,
    /// `reported - expected` (blank reported counts as 0).
    pub delta: Decimal,
    /// How `reported` must relate to `expected` (formulas are always
    /// [`Relation::Equal`]).
    pub relation: Relation,
    /// For schedule sums: how many body lines contributed.
    pub lines_summed: usize,
    /// True when the reported value was blank or not a valid amount.
    pub reported_unparseable: bool,
}

impl LineCheck {
    /// True when the cover page satisfies the rule: `delta == 0`, or
    /// `delta >= 0` for a [`Relation::AtLeast`] line.
    #[must_use]
    pub fn matches(&self) -> bool {
        self.violation().is_zero()
    }

    /// The amount by which the rule is violated: `|delta|` for an equality,
    /// the shortfall (`max(0, -delta)`) for a floor. Zero when it matches.
    #[must_use]
    pub fn violation(&self) -> Decimal {
        match self.relation {
            Relation::Equal => self.delta.abs(),
            Relation::AtLeast => {
                if self.delta.is_sign_negative() {
                    self.delta.abs()
                } else {
                    Decimal::ZERO
                }
            }
        }
    }
}

impl fmt::Display for LineCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = if self.matches() { "ok  " } else { "DIFF" };
        let reported = self
            .reported
            .map_or_else(|| "(blank)".to_string(), |d| d.to_string());
        write!(
            f,
            "{status} col {} line {:<10} reported {:>15} expected {:>15} delta {:>12}  {}",
            self.column, self.line, reported, self.expected, self.delta, self.rule
        )
    }
}

/// Every line check for one filing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[must_use]
#[non_exhaustive]
pub struct Reconciliation {
    pub form: Table,
    pub checks: Vec<LineCheck>,
}

impl Reconciliation {
    /// Checks whose delta is non-zero.
    pub fn mismatches(&self) -> impl Iterator<Item = &LineCheck> + '_ {
        self.checks.iter().filter(|c| !c.matches())
    }

    /// Checks whose [`violation`](LineCheck::violation) exceeds `tolerance`.
    pub fn mismatches_over(&self, tolerance: Decimal) -> impl Iterator<Item = &LineCheck> + '_ {
        self.checks
            .iter()
            .filter(move |c| c.violation() > tolerance)
    }

    /// True when every line agrees exactly.
    #[must_use]
    pub fn balances(&self) -> bool {
        self.checks.iter().all(LineCheck::matches)
    }

    /// The checks for one column.
    pub fn column(&self, column: Column) -> impl Iterator<Item = &LineCheck> + '_ {
        self.checks.iter().filter(move |c| c.column == column)
    }

    /// The check for a line label in a column, if the form has that rule.
    #[must_use]
    pub fn line(&self, column: Column, line: &str) -> Option<&LineCheck> {
        self.checks
            .iter()
            .find(|c| c.column == column && c.line == line)
    }
}

impl fmt::Display for Reconciliation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in &self.checks {
            writeln!(f, "{c}")?;
        }
        let n = self.mismatches().count();
        if n == 0 {
            write!(f, "{}: every line agrees with its rule", self.form)
        } else {
            write!(
                f,
                "{}: {n} of {} line(s) disagree",
                self.form,
                self.checks.len()
            )
        }
    }
}

/// Why a filing cannot be reconciled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ReconcileError {
    /// The cover form has no rule table (only F3X, F3, and F3P do).
    #[error("no reconciliation rules for form {0}; supported: F3X, F3, F3P")]
    UnsupportedForm(Table),
}

impl Filing {
    /// Recomputes every cover-page line from the schedules and the other
    /// cover lines and compares it with what was reported.
    ///
    /// Supported cover forms: F3X, F3, F3P. Column A lines are checked
    /// against schedule sums and formulas; Column B lines against formulas.
    /// Lines the filing's spec version does not carry are omitted.
    pub fn reconcile(&self) -> Result<Reconciliation, ReconcileError> {
        let form = self.summary.table();
        let rules = rules_for(form).ok_or(ReconcileError::UnsupportedForm(form))?;
        Ok(Reconciliation {
            form,
            checks: rules
                .iter()
                .filter(|r| !matches!(r.source, Source::Input))
                // A line the filing's spec version does not have (e.g.
                // 17(a)(i) before 8.0) cannot be checked.
                .filter(|r| self.summary.get(r.field).is_some())
                .map(|r| check_line(&self.summary, &self.lines, rules, r))
                .collect(),
        })
    }
}

/// The rule table for a cover form.
#[must_use]
pub fn rules_for(form: Table) -> Option<&'static [LineRule]> {
    match form {
        Table::F3X => Some(F3X_RULES),
        Table::F3 => Some(F3_RULES),
        Table::F3P => Some(F3P_RULES),
        _ => None,
    }
}

fn reported_amount(cover: &ParsedLine, field: &str) -> (Option<Decimal>, bool) {
    match cover.get_non_empty(field) {
        None => (None, false),
        Some(raw) => match parse_money(raw) {
            Some(d) => (Some(d), false),
            None => (None, true),
        },
    }
}

fn check_line(
    cover: &ParsedLine,
    body: &[ParsedLine],
    rules: &[LineRule],
    rule: &LineRule,
) -> LineCheck {
    let (reported, unparseable) = reported_amount(cover, rule.field);
    let relation = match rule.source {
        Source::Schedules(relation, _) => relation,
        Source::Formula(_) | Source::Input => Relation::Equal,
    };
    let (expected, lines_summed) = match rule.source {
        Source::Schedules(_, sums) => {
            let mut total = Decimal::ZERO;
            let mut n = 0usize;
            for line in body.iter().filter(|l| !l.is_memo()) {
                for s in sums {
                    if line.table() == s.table
                        && s.tokens
                            .iter()
                            .any(|t| t.eq_ignore_ascii_case(&line.raw_form_type))
                        && let Some(amount) = line.get_non_empty(s.field).and_then(parse_money)
                    {
                        total = total.saturating_add(amount);
                        n = n.saturating_add(1);
                    }
                }
            }
            (total, n)
        }
        Source::Input => (reported.unwrap_or(Decimal::ZERO), 0),
        Source::Formula(terms) => {
            let mut total = Decimal::ZERO;
            for t in terms {
                let value = rules
                    .iter()
                    .find(|r| r.column == rule.column && r.line == t.line)
                    .and_then(|r| reported_amount(cover, r.field).0)
                    .unwrap_or(Decimal::ZERO);
                total = if t.negative {
                    total.saturating_sub(value)
                } else {
                    total.saturating_add(value)
                };
            }
            (total, 0)
        }
    };
    let delta = reported.unwrap_or(Decimal::ZERO).saturating_sub(expected);
    LineCheck {
        line: rule.line,
        field: rule.field,
        column: rule.column,
        rule: rule.formula_text(),
        reported,
        expected,
        delta,
        relation,
        lines_summed,
        reported_unparseable: unparseable,
    }
}

// ---------------------------------------------------------------------------
// Rule tables
// ---------------------------------------------------------------------------

/// Declares one form's rule table in the FEC's own notation.
///
/// ```text
/// rules! { A:
///     "6(b)"       col_a_cash_on_hand_beginning_period { input };
///     "11(a)(i)"   col_a_individuals_itemized          { <- SchA["SA11AI"].contribution_amount };
///     "21(b)"      col_a_other_federal_operating_expenditures { >= SchB["SB21B"].expenditure_amount };
///     "11(a)(iii)" col_a_individual_contribution_total { = "11(a)(i)" + "11(a)(ii)" };
/// }
/// ```
///
/// `<-` means the cover line must equal the schedule sum; `>=` means it
/// must be at least the schedule sum (sub-threshold items may be left
/// unitemized -- see [`Relation`]). `{ input }` declares a line that has
/// no rule of its own (cash on hand brought forward; Column B amounts
/// whose sums span prior reports) but may appear as a term in other lines'
/// formulas.
macro_rules! rules {
    (@src { input }) => { Source::Input };
    (@src { <- $($table:ident [$($tok:literal),+] . $field:ident)+ }) => {
        Source::Schedules(Relation::Equal, &[$(ScheduleSum {
            table: Table::$table,
            tokens: &[$($tok),+],
            field: stringify!($field),
        }),+])
    };
    (@src { >= $($table:ident [$($tok:literal),+] . $field:ident)+ }) => {
        Source::Schedules(Relation::AtLeast, &[$(ScheduleSum {
            table: Table::$table,
            tokens: &[$($tok),+],
            field: stringify!($field),
        }),+])
    };
    (@src { = $first:literal $(+ $plus:literal)* $(- $minus:literal)* }) => {
        Source::Formula(&[
            Term { line: $first, negative: false },
            $(Term { line: $plus, negative: false },)*
            $(Term { line: $minus, negative: true },)*
        ])
    };
    ($( $col:ident : $( $line:literal $field:ident $src:tt ; )+ )+) => {
        &[ $( $( LineRule {
            line: $line,
            field: stringify!($field),
            column: Column::$col,
            source: rules!(@src $src),
        }, )+ )+ ]
    };
}

/// Form 3X (PAC / party committee periodic report).
///
/// Sources: FEC format spec sheet `F3X`, `RULE REFERENCE` column;
/// FECfile+ `reports/form_3x/summary.py` (which stubs 18(c), 21(a)(i),
/// 21(a)(ii), 30(a)(i), 30(a)(ii) to zero -- implemented here from
/// Schedules H3, H4, H5, and H6 per the spec's own rule text).
pub static F3X_RULES: &[LineRule] = rules! {
    A:
        "9" col_a_debts_to { <- SchC["SC/9"].loan_balance  SchD["SD9"].balance_at_close_this_period };
        "10" col_a_debts_by { <- SchC["SC/10"].loan_balance SchD["SD10"].balance_at_close_this_period };
        "11(a)(i)" col_a_individuals_itemized { <- SchA["SA11AI", "SA11A1"].contribution_amount };
        "11(a)(ii)" col_a_individuals_unitemized { input };
        "11(a)(iii)" col_a_individual_contribution_total { = "11(a)(i)" + "11(a)(ii)" };
        "11(b)" col_a_political_party_committees { <- SchA["SA11B"].contribution_amount };
        "11(c)" col_a_other_political_committees_pacs { <- SchA["SA11C"].contribution_amount };
        "11(d)" col_a_total_contributions { = "11(a)(iii)" + "11(b)" + "11(c)" };
        "12" col_a_transfers_from_aff_other_party_cmttees { <- SchA["SA12"].contribution_amount };
        "13" col_a_total_loans { <- SchA["SA13"].contribution_amount };
        "14" col_a_total_loan_repayments_received { <- SchA["SA14"].contribution_amount };
        "15" col_a_offsets_to_expenditures { >= SchA["SA15"].contribution_amount };
        "16" col_a_refunds_of_federal_contributions { <- SchA["SA16"].contribution_amount };
        "17" col_a_other_federal_receipts { >= SchA["SA17"].contribution_amount };
        "18(a)" col_a_transfers_from_nonfederal_h3 { <- H3["H3"].transferred_amount };
        "18(b)" col_a_levin_funds { <- H5["H5"].total_amount_transferred };
        "18(c)" col_a_total_nonfederal_transfers { = "18(a)" + "18(b)" };
        "19" col_a_total_receipts_recap { = "11(d)" + "12" + "13" + "14" + "15" + "16" + "17" + "18(c)" };
        "20" col_a_total_federal_receipts { = "19" - "18(c)" };
        "21(a)(i)" col_a_shared_operating_expenditures_federal { <- H4["H4"].federal_share };
        "21(a)(ii)" col_a_shared_operating_expenditures_nonfederal { <- H4["H4"].nonfederal_share };
        "21(b)" col_a_other_federal_operating_expenditures { >= SchB["SB21B"].expenditure_amount };
        "21(c)" col_a_total_operating_expenditures { = "21(a)(i)" + "21(a)(ii)" + "21(b)" };
        "22" col_a_transfers_to_affiliated { <- SchB["SB22"].expenditure_amount };
        "23" col_a_contributions_to_candidates { <- SchB["SB23"].expenditure_amount };
        "24" col_a_independent_expenditures { <- SchE["SE"].expenditure_amount };
        "25" col_a_coordinated_expenditures_by_party_committees { <- SchF["SF"].expenditure_amount };
        "26" col_a_total_loan_repayments_made { <- SchB["SB26"].expenditure_amount };
        "27" col_a_loans_made { <- SchB["SB27"].expenditure_amount };
        "28(a)" col_a_refunds_to_individuals { >= SchB["SB28A"].expenditure_amount };
        "28(b)" col_a_refunds_to_party_committees { <- SchB["SB28B"].expenditure_amount };
        "28(c)" col_a_refunds_to_other_committees { <- SchB["SB28C"].expenditure_amount };
        "28(d)" col_a_total_refunds { = "28(a)" + "28(b)" + "28(c)" };
        "29" col_a_other_disbursements { >= SchB["SB29"].expenditure_amount };
        "30(a)(i)" col_a_federal_election_activity_federal_share { <- H6["H6"].federal_share };
        "30(a)(ii)" col_a_federal_election_activity_levin_share { <- H6["H6"].levin_share };
        "30(b)" col_a_federal_election_activity_all_federal { <- SchB["SB30B"].expenditure_amount };
        "30(c)" col_a_federal_election_activity_total { = "30(a)(i)" + "30(a)(ii)" + "30(b)" };
        "31" col_a_total_disbursements_recap { = "21(c)" + "22" + "23" + "24" + "25" + "26" + "27" + "28(d)" + "29" + "30(c)" };
        "32" col_a_total_federal_disbursements { = "31" - "21(a)(ii)" - "30(a)(ii)" };
        "33" col_a_total_contributions_recap { = "11(d)" };
        "34" col_a_total_contributions_refunds { = "28(d)" };
        "35" col_a_net_contributions { = "33" - "34" };
        "36" col_a_total_federal_operating_expenditures { = "21(a)(i)" + "21(b)" };
        "37" col_a_total_offsets_to_expenditures { = "15" };
        "38" col_a_net_operating_expenditures { = "36" - "37" };
        "6(b)" col_a_cash_on_hand_beginning_period { input };
        "6(c)" col_a_total_receipts { = "19" };
        "6(d)" col_a_subtotal { = "6(b)" + "6(c)" };
        "7" col_a_total_disbursements { = "31" };
        "8" col_a_cash_on_hand_close_of_period { = "6(d)" - "7" };
    B:
        "11(a)(i)" col_b_individuals_itemized { input };
        "11(a)(ii)" col_b_individuals_unitemized { input };
        "11(a)(iii)" col_b_individual_contribution_total { = "11(a)(i)" + "11(a)(ii)" };
        "11(b)" col_b_political_party_committees { input };
        "11(c)" col_b_other_political_committees_pacs { input };
        "11(d)" col_b_total_contributions { = "11(a)(iii)" + "11(b)" + "11(c)" };
        "12" col_b_transfers_from_aff_other_party_cmttees { input };
        "13" col_b_total_loans { input };
        "14" col_b_total_loan_repayments_received { input };
        "15" col_b_offsets_to_expenditures { input };
        "16" col_b_refunds_of_federal_contributions { input };
        "17" col_b_other_federal_receipts { input };
        "18(a)" col_b_transfers_from_nonfederal_h3 { input };
        "18(b)" col_b_levin_funds { input };
        "18(c)" col_b_total_nonfederal_transfers { = "18(a)" + "18(b)" };
        "19" col_b_total_receipts_recap { = "11(d)" + "12" + "13" + "14" + "15" + "16" + "17" + "18(c)" };
        "20" col_b_total_federal_receipts { = "19" - "18(c)" };
        "21(a)(i)" col_b_shared_operating_expenditures_federal { input };
        "21(a)(ii)" col_b_shared_operating_expenditures_nonfederal { input };
        "21(b)" col_b_other_federal_operating_expenditures { input };
        "21(c)" col_b_total_operating_expenditures { = "21(a)(i)" + "21(a)(ii)" + "21(b)" };
        "22" col_b_transfers_to_affiliated { input };
        "23" col_b_contributions_to_candidates { input };
        "24" col_b_independent_expenditures { input };
        "25" col_b_coordinated_expenditures_by_party_committees { input };
        "26" col_b_total_loan_repayments_made { input };
        "27" col_b_loans_made { input };
        "28(a)" col_b_refunds_to_individuals { input };
        "28(b)" col_b_refunds_to_party_committees { input };
        "28(c)" col_b_refunds_to_other_committees { input };
        "28(d)" col_b_total_refunds { = "28(a)" + "28(b)" + "28(c)" };
        "29" col_b_other_disbursements { input };
        "30(a)(i)" col_b_federal_election_activity_federal_share { input };
        "30(a)(ii)" col_b_federal_election_activity_levin_share { input };
        "30(b)" col_b_federal_election_activity_all_federal { input };
        "30(c)" col_b_federal_election_activity_total { = "30(a)(i)" + "30(a)(ii)" + "30(b)" };
        "31" col_b_total_disbursements_recap { = "21(c)" + "22" + "23" + "24" + "25" + "26" + "27" + "28(d)" + "29" + "30(c)" };
        "32" col_b_total_federal_disbursements { = "31" - "21(a)(ii)" - "30(a)(ii)" };
        "33" col_b_total_contributions_recap { = "11(d)" };
        "34" col_b_total_contributions_refunds { = "28(d)" };
        "35" col_b_net_contributions { = "33" - "34" };
        "36" col_b_total_federal_operating_expenditures { = "21(a)(i)" + "21(b)" };
        "37" col_b_total_offsets_to_expenditures { = "15" };
        "38" col_b_net_operating_expenditures { = "36" - "37" };
        "6(a)" col_b_cash_on_hand_jan_1 { input };
        "6(c)" col_b_total_receipts { = "19" };
        "6(d)" col_b_subtotal { = "6(a)" + "6(c)" };
        "7" col_b_total_disbursements { = "31" };
        "8" col_b_cash_on_hand_close_of_period { = "6(d)" - "7" };
};

/// Form 3 (House / Senate candidate committee report).
///
/// Source: FEC format spec sheet `F3`, `RULE REFERENCE` column. (FECfile+
/// computes this form as all zeros.)
pub static F3_RULES: &[LineRule] = rules! {
    A:
        "9" col_a_debts_to { <- SchC["SC/9"].loan_balance  SchD["SD9"].balance_at_close_this_period };
        "10" col_a_debts_by { <- SchC["SC/10"].loan_balance SchD["SD10"].balance_at_close_this_period };
        "11(a)(i)" col_a_individual_contributions_itemized { <- SchA["SA11AI", "SA11A1"].contribution_amount };
        "11(a)(ii)" col_a_individual_contributions_unitemized { input };
        "11(a)(iii)" col_a_total_individual_contributions { = "11(a)(i)" + "11(a)(ii)" };
        "11(b)" col_a_political_party_contributions { <- SchA["SA11B"].contribution_amount };
        "11(c)" col_a_pac_contributions { <- SchA["SA11C"].contribution_amount };
        "11(d)" col_a_candidate_contributions { <- SchA["SA11D"].contribution_amount };
        "11(e)" col_a_total_contributions { = "11(a)(iii)" + "11(b)" + "11(c)" + "11(d)" };
        "12" col_a_transfers_from_authorized { <- SchA["SA12"].contribution_amount };
        "13(a)" col_a_candidate_loans { <- SchA["SA13A"].contribution_amount };
        "13(b)" col_a_other_loans { <- SchA["SA13B"].contribution_amount };
        "13(c)" col_a_total_loans { = "13(a)" + "13(b)" };
        "14" col_a_offset_to_operating_expenditures { >= SchA["SA14"].contribution_amount };
        "15" col_a_other_receipts { >= SchA["SA15"].contribution_amount };
        "16" col_a_total_receipts { = "11(e)" + "12" + "13(c)" + "14" + "15" };
        "17" col_a_operating_expenditures { >= SchB["SB17"].expenditure_amount };
        "18" col_a_transfers_to_authorized { <- SchB["SB18"].expenditure_amount };
        "19(a)" col_a_candidate_loan_repayments { <- SchB["SB19A"].expenditure_amount };
        "19(b)" col_a_other_loan_repayments { <- SchB["SB19B"].expenditure_amount };
        "19(c)" col_a_total_loan_repayments { = "19(a)" + "19(b)" };
        "20(a)" col_a_refunds_to_individuals { >= SchB["SB20A"].expenditure_amount };
        "20(b)" col_a_refunds_to_party_committees { <- SchB["SB20B"].expenditure_amount };
        "20(c)" col_a_refunds_to_other_committees { <- SchB["SB20C"].expenditure_amount };
        "20(d)" col_a_total_refunds { = "20(a)" + "20(b)" + "20(c)" };
        "21" col_a_other_disbursements { >= SchB["SB21"].expenditure_amount };
        "22" col_a_total_disbursements { = "17" + "18" + "19(c)" + "20(d)" + "21" };
        "6(a)" col_a_total_contributions_no_loans { = "11(e)" };
        "6(b)" col_a_total_contributions_refunds { = "20(d)" };
        "6(c)" col_a_net_contributions { = "6(a)" - "6(b)" };
        "7(a)" col_a_total_operating_expenditures { = "17" };
        "7(b)" col_a_total_offset_to_operating_expenditures { = "14" };
        "7(c)" col_a_net_operating_expenditures { = "7(a)" - "7(b)" };
        "23" col_a_cash_beginning_reporting_period { input };
        "24" col_a_total_receipts_period { = "16" };
        "25" col_a_subtotals { = "23" + "24" };
        "26" col_a_total_disbursements_period { = "22" };
        "27" col_a_cash_on_hand_close { = "25" - "26" };
        "8" col_a_cash_on_hand_close_of_period { = "27" };
    B:
        "11(a)(i)" col_b_individual_contributions_itemized { input };
        "11(a)(ii)" col_b_individual_contributions_unitemized { input };
        "11(a)(iii)" col_b_total_individual_contributions { = "11(a)(i)" + "11(a)(ii)" };
        "11(b)" col_b_political_party_contributions { input };
        "11(c)" col_b_pac_contributions { input };
        "11(d)" col_b_candidate_contributions { input };
        "11(e)" col_b_total_contributions { = "11(a)(iii)" + "11(b)" + "11(c)" + "11(d)" };
        "12" col_b_transfers_from_authorized { input };
        "13(a)" col_b_candidate_loans { input };
        "13(b)" col_b_other_loans { input };
        "13(c)" col_b_total_loans { = "13(a)" + "13(b)" };
        "14" col_b_offset_to_operating_expenditures { input };
        "15" col_b_other_receipts { input };
        "16" col_b_total_receipts { = "11(e)" + "12" + "13(c)" + "14" + "15" };
        "17" col_b_operating_expenditures { input };
        "18" col_b_transfers_to_authorized { input };
        "19(a)" col_b_candidate_loan_repayments { input };
        "19(b)" col_b_other_loan_repayments { input };
        "19(c)" col_b_total_loan_repayments { = "19(a)" + "19(b)" };
        "20(a)" col_b_refunds_to_individuals { input };
        "20(b)" col_b_refunds_to_party_committees { input };
        "20(c)" col_b_refunds_to_other_committees { input };
        "20(d)" col_b_total_refunds { = "20(a)" + "20(b)" + "20(c)" };
        "21" col_b_other_disbursements { input };
        "22" col_b_total_disbursements { = "17" + "18" + "19(c)" + "20(d)" + "21" };
        "6(a)" col_b_total_contributions_no_loans { = "11(e)" };
        "6(b)" col_b_total_contributions_refunds { = "20(d)" };
        "6(c)" col_b_net_contributions { = "6(a)" - "6(b)" };
        "7(a)" col_b_total_operating_expenditures { = "17" };
        "7(b)" col_b_total_offset_to_operating_expenditures { = "14" };
        "7(c)" col_b_net_operating_expenditures { = "7(a)" - "7(b)" };
};

/// Form 3P (presidential committee report).
///
/// Source: FEC format spec sheet `F3P`, `RULE REFERENCE` column. Line
/// 17(a)(i) sums `SA17A` (itemized individuals); unitemized 17(a)(ii) has
/// no schedule. Lines 14 and 15 are Column-B-derived per the spec
/// (`= 17e Col B - 28d Col B`, `= 23 Col B - 20a Col B`).
pub static F3P_RULES: &[LineRule] = rules! {
    A:
        "11" col_a_debts_to { <- SchC["SC/11"].loan_balance  SchD["SD11"].balance_at_close_this_period };
        "12" col_a_debts_by { <- SchC["SC/12"].loan_balance  SchD["SD12"].balance_at_close_this_period };
        "16" col_a_federal_funds { <- SchA["SA16"].contribution_amount };
        "17(a)(i)" col_a_individuals_itemized { <- SchA["SA17A"].contribution_amount };
        "17(a)(ii)" col_a_individuals_unitemized { input };
        "17(a)(iii)" col_a_individual_contribution_total { = "17(a)(i)" + "17(a)(ii)" };
        "17(b)" col_a_political_party_committees_receipts { <- SchA["SA17B"].contribution_amount };
        "17(c)" col_a_other_political_committees_pacs { <- SchA["SA17C"].contribution_amount };
        "17(d)" col_a_the_candidate { <- SchA["SA17D"].contribution_amount };
        "17(e)" col_a_total_contributions { = "17(a)(iii)" + "17(b)" + "17(c)" + "17(d)" };
        "18" col_a_transfers_from_aff_other_party_cmttees { <- SchA["SA18"].contribution_amount };
        "19(a)" col_a_received_from_or_guaranteed_by_cand { <- SchA["SA19A"].contribution_amount };
        "19(b)" col_a_other_loans { <- SchA["SA19B"].contribution_amount };
        "19(c)" col_a_total_loans { = "19(a)" + "19(b)" };
        "20(a)" col_a_operating { >= SchA["SA20A"].contribution_amount };
        "20(b)" col_a_fundraising { >= SchA["SA20B"].contribution_amount };
        "20(c)" col_a_legal_and_accounting { >= SchA["SA20C"].contribution_amount };
        "20(d)" col_a_total_offsets_to_expenditures { = "20(a)" + "20(b)" + "20(c)" };
        "21" col_a_other_receipts { >= SchA["SA21"].contribution_amount };
        "22" col_a_total_receipts_recap { = "16" + "17(e)" + "18" + "19(c)" + "20(d)" + "21" };
        "23" col_a_operating_expenditures { >= SchB["SB23"].expenditure_amount };
        "24" col_a_transfers_to_other_authorized_committees { <- SchB["SB24"].expenditure_amount };
        "25" col_a_fundraising_disbursements { >= SchB["SB25"].expenditure_amount };
        "26" col_a_exempt_legal_accounting_disbursement { >= SchB["SB26"].expenditure_amount };
        "27(a)" col_a_made_or_guaranteed_by_candidate { <- SchB["SB27A"].expenditure_amount };
        "27(b)" col_a_other_repayments { <- SchB["SB27B"].expenditure_amount };
        "27(c)" col_a_total_loan_repayments_made { = "27(a)" + "27(b)" };
        "28(a)" col_a_individuals { >= SchB["SB28A"].expenditure_amount };
        "28(b)" col_a_political_party_committees_refunds { <- SchB["SB28B"].expenditure_amount };
        "28(c)" col_a_other_political_committees { <- SchB["SB28C"].expenditure_amount };
        "28(d)" col_a_total_contributions_refunds { = "28(a)" + "28(b)" + "28(c)" };
        "29" col_a_other_disbursements { >= SchB["SB29"].expenditure_amount };
        "30" col_a_total_disbursements_recap { = "23" + "24" + "25" + "26" + "27(c)" + "28(d)" + "29" };
        "6" col_a_cash_on_hand_beginning_period { input };
        "7" col_a_total_receipts { = "22" };
        "8" col_a_subtotal { = "6" + "7" };
        "9" col_a_total_disbursements { = "30" };
        "10" col_a_cash_on_hand_close_of_period { = "8" - "9" };
    B:
        "16" col_b_federal_funds { input };
        "17(a)(i)" col_b_individuals_itemized { input };
        "17(a)(ii)" col_b_individuals_unitemized { input };
        "17(a)(iii)" col_b_individual_contribution_total { = "17(a)(i)" + "17(a)(ii)" };
        "17(b)" col_b_political_party_committees_receipts { input };
        "17(c)" col_b_other_political_committees_pacs { input };
        "17(d)" col_b_the_candidate { input };
        "17(e)" col_b_total_contributions_other_than_loans { = "17(a)(iii)" + "17(b)" + "17(c)" + "17(d)" };
        "18" col_b_transfers_from_aff_other_party_cmttees { input };
        "19(a)" col_b_received_from_or_guaranteed_by_cand { input };
        "19(b)" col_b_other_loans { input };
        "19(c)" col_b_total_loans { = "19(a)" + "19(b)" };
        "20(a)" col_b_operating { input };
        "20(b)" col_b_fundraising { input };
        "20(c)" col_b_legal_and_accounting { input };
        "20(d)" col_b_total_offsets_to_operating_expenditures { = "20(a)" + "20(b)" + "20(c)" };
        "21" col_b_other_receipts { input };
        "22" col_b_total_receipts { = "16" + "17(e)" + "18" + "19(c)" + "20(d)" + "21" };
        "23" col_b_operating_expenditures { input };
        "24" col_b_transfers_to_other_authorized_committees { input };
        "25" col_b_fundraising_disbursements { input };
        "26" col_b_exempt_legal_accounting_disbursement { input };
        "27(a)" col_b_made_or_guaranteed_by_the_candidate { input };
        "27(b)" col_b_other_repayments { input };
        "27(c)" col_b_total_loan_repayments_made { = "27(a)" + "27(b)" };
        "28(a)" col_b_individuals { input };
        "28(b)" col_b_political_party_committees_refunds { input };
        "28(c)" col_b_other_political_committees { input };
        "28(d)" col_b_total_contributions_refunds { = "28(a)" + "28(b)" + "28(c)" };
        "29" col_b_other_disbursements { input };
        "30" col_b_total_disbursements { = "23" + "24" + "25" + "26" + "27(c)" + "28(d)" + "29" };
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::SpecVersion;
    use rust_decimal_macros::dec;

    /// Every rule's cover field must exist in the form's current layout,
    /// every schedule field in its table's layout, and every formula term
    /// must name a rule of the same column. A typo here would otherwise be
    /// a silent `None`.
    #[test]
    fn rule_tables_reference_real_fields_and_lines() {
        let v85 = SpecVersion::electronic(8, 5);
        for (form, rules) in [
            (Table::F3X, F3X_RULES),
            (Table::F3, F3_RULES),
            (Table::F3P, F3P_RULES),
        ] {
            let layout = form.layout(v85).unwrap();
            for r in rules {
                assert!(
                    layout.field(r.field).is_some(),
                    "{form}: cover field {}",
                    r.field
                );
                match r.source {
                    Source::Input => {}
                    Source::Schedules(_, sums) => {
                        for s in sums {
                            let l = s.table.layout(v85).unwrap_or_else(|| panic!("{}", s.table));
                            assert!(
                                l.field(s.field).is_some(),
                                "{form} {}: {}.{}",
                                r.line,
                                s.table,
                                s.field
                            );
                        }
                    }
                    Source::Formula(terms) => {
                        for t in terms {
                            assert!(
                                rules
                                    .iter()
                                    .any(|o| o.column == r.column && o.line == t.line),
                                "{form} col {} line {}: term {} names no rule",
                                r.column,
                                r.line,
                                t.line
                            );
                        }
                    }
                }
            }
            // No duplicate (column, line).
            let mut keys: Vec<(Column, &str)> = rules.iter().map(|r| (r.column, r.line)).collect();
            keys.sort();
            keys.dedup();
            assert_eq!(keys.len(), rules.len(), "{form}: duplicate rule");
        }
    }

    #[test]
    fn formula_text_matches_fec_notation() {
        let r = F3X_RULES
            .iter()
            .find(|r| r.column == Column::A && r.line == "32")
            .unwrap();
        assert_eq!(r.formula_text(), "= 31 - 21(a)(ii) - 30(a)(ii)");
        let r = F3X_RULES
            .iter()
            .find(|r| r.column == Column::A && r.line == "9")
            .unwrap();
        assert_eq!(
            r.formula_text(),
            "= sum of SchC.loan_balance on SC/9 + SchD.balance_at_close_this_period on SD9"
        );
    }

    fn f3x_with(cover: &[(&str, &str)], body: Vec<ParsedLine>) -> Filing {
        let mut filing =
            Filing::parse("HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF3XN\u{1c}C00123456").unwrap();
        for (k, v) in cover {
            filing.summary.set(k, v).unwrap();
        }
        filing.lines = body;
        filing
    }

    fn body(table: Table, token: &str, field: &str, amount: &str, memo: bool) -> ParsedLine {
        let mut pairs = vec![
            ("form_type", token),
            ("filer_committee_id_number", "C00123456"),
            (field, amount),
        ];
        if memo {
            pairs.push(("memo_code", "X"));
        }
        ParsedLine::from_pairs(table, SpecVersion::electronic(8, 5), 3, pairs).unwrap()
    }

    #[test]
    fn schedule_sum_excludes_memos_and_matches_case_insensitively() {
        let filing = f3x_with(
            &[("col_a_individuals_itemized", "300.00")],
            vec![
                body(
                    Table::SchA,
                    "SA11AI",
                    "contribution_amount",
                    "100.00",
                    false,
                ),
                body(
                    Table::SchA,
                    "sa11ai",
                    "contribution_amount",
                    "200.00",
                    false,
                ),
                body(Table::SchA, "SA11AI", "contribution_amount", "999.00", true),
                body(Table::SchA, "SA11C", "contribution_amount", "5.00", false),
            ],
        );
        let r = filing.reconcile().unwrap();
        let c = r.line(Column::A, "11(a)(i)").unwrap();
        assert_eq!(c.expected, dec!(300.00));
        assert_eq!(c.reported, Some(dec!(300.00)));
        assert_eq!(c.lines_summed, 2);
        assert!(c.matches());
        let c = r.line(Column::A, "11(c)").unwrap();
        assert_eq!(c.expected, dec!(5.00));
        assert_eq!(c.reported, None);
        assert_eq!(c.delta, dec!(-5.00));
        assert!(!c.matches());
    }

    #[test]
    fn formulas_use_reported_values_so_a_bad_input_is_isolated() {
        // 11(a)(i) is wrong vs the schedule, but 11(a)(iii) = 11(a)(i) + 11(a)(ii)
        // holds on the reported numbers, so only 11(a)(i) is flagged.
        let filing = f3x_with(
            &[
                ("col_a_individuals_itemized", "150.00"),
                ("col_a_individuals_unitemized", "50.00"),
                ("col_a_individual_contribution_total", "200.00"),
            ],
            vec![body(
                Table::SchA,
                "SA11AI",
                "contribution_amount",
                "100.00",
                false,
            )],
        );
        let r = filing.reconcile().unwrap();
        assert_eq!(r.line(Column::A, "11(a)(i)").unwrap().delta, dec!(50.00));
        assert!(r.line(Column::A, "11(a)(iii)").unwrap().matches());
        let mismatched: Vec<&str> = r
            .mismatches()
            .filter(|c| c.column == Column::A)
            .map(|c| c.line)
            .collect();
        // 11(d), 19, 20, 6(c), 33, 35 all propagate the reported 200 and are
        // blank on the cover, so they also differ -- but 11(a)(iii) does not.
        assert!(mismatched.contains(&"11(a)(i)"));
        assert!(!mismatched.contains(&"11(a)(iii)"));
    }

    #[test]
    fn debts_sum_schedule_c_and_d() {
        let filing = f3x_with(
            &[("col_a_debts_by", "250.00"), ("col_a_debts_to", "0")],
            vec![
                body(Table::SchC, "SC/10", "loan_balance", "30.00", false),
                body(
                    Table::SchD,
                    "SD10",
                    "balance_at_close_this_period",
                    "220.00",
                    false,
                ),
                body(Table::SchC, "SC/9", "loan_balance", "1.00", true),
            ],
        );
        let r = filing.reconcile().unwrap();
        assert!(r.line(Column::A, "10").unwrap().matches());
        let nine = r.line(Column::A, "9").unwrap();
        assert_eq!(nine.expected, Decimal::ZERO);
        assert!(nine.matches());
    }

    #[test]
    fn unparseable_reported_amount_is_flagged_not_panicked() {
        let filing = f3x_with(&[("col_a_total_receipts", "$1,000")], vec![]);
        let r = filing.reconcile().unwrap();
        let c = r.line(Column::A, "6(c)").unwrap();
        assert!(c.reported_unparseable);
        assert_eq!(c.reported, None);
    }

    #[test]
    fn unsupported_form_errors() {
        let filing =
            Filing::parse("HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF24N\u{1c}C00123456").unwrap();
        assert_eq!(
            filing.reconcile(),
            Err(ReconcileError::UnsupportedForm(Table::F24))
        );
    }

    #[test]
    fn tolerance_and_display() {
        let filing = f3x_with(
            &[("col_a_individuals_itemized", "100.01")],
            vec![body(
                Table::SchA,
                "SA11AI",
                "contribution_amount",
                "100.00",
                false,
            )],
        );
        let r = filing.reconcile().unwrap();
        assert!(r.mismatches().any(|c| c.line == "11(a)(i)"));
        assert!(!r.mismatches_over(dec!(0.01)).any(|c| c.line == "11(a)(i)"));
        let text = r.to_string();
        assert!(text.contains("DIFF col A line 11(a)(i)"), "{text}");
        assert!(text.contains("line(s) disagree"));
    }

    /// Oracle: the FEC's FECfile+ test suite
    /// (`fecfiler/reports/form_3x/tests/test_summary.py`, public domain)
    /// asserts the Column A totals its summary calculator produces for a
    /// fixed transaction set (`web_services/summary/tests/utils.py`). The
    /// same transactions, filed as a .fec, must reconcile to the same
    /// numbers -- including the memo exclusions (a memo SA15 of 10000.23 and
    /// a memo SE of 57.00 are ignored by both implementations).
    #[test]
    fn oracle_fecfile_plus_f3x_column_a() {
        use Table::*;
        let a =
            |tok: &str, amt: &str, memo: bool| body(SchA, tok, "contribution_amount", amt, memo);
        let b = |tok: &str, amt: &str| body(SchB, tok, "expenditure_amount", amt, false);
        let mut lines = vec![
            a("SA11AI", "10000.23", false),
            a("SA11AII", "3.77", false),
            a("SA11B", "444.44", false),
            a("SA11C", "555.55", false),
            a("SA12", "1212.12", false),
            a("SA13", "1313.13", false),
            a("SA14", "1414.14", false),
            a("SA15", "1234.56", false),
            a("SA15", "891.23", false),
            a("SA15", "10000.23", true),
            a("SA16", "16", false),
            a("SA17", "200.50", false),
            a("SA17", "-1", false),
            a("SA17", "800.50", false),
            b("SB21B", "150"),
            b("SB22", "22"),
            b("SB23", "14"),
            b("SB26", "44"),
            b("SB27", "31"),
            b("SB28A", "101.50"),
            b("SB28B", "201.50"),
            b("SB28C", "301.50"),
            b("SB29", "201.50"),
            b("SB30B", "102.25"),
            body(SchC, "SC/9", "loan_balance", "150", false),
            body(SchC, "SC/10", "loan_balance", "30", false),
            body(SchD, "SD9", "balance_at_close_this_period", "100", false),
            body(SchD, "SD10", "balance_at_close_this_period", "220", false),
        ];
        for (amt, memo) in [("65", false), ("76", false), ("10", false), ("57", true)] {
            lines.push(body(SchE, "SE", "expenditure_amount", amt, memo));
        }
        for amt in ["65", "15", "53"] {
            lines.push(body(SchF, "SF", "expenditure_amount", amt, false));
        }
        let filing = f3x_with(&[], lines);
        let r = filing.reconcile().unwrap();
        let expected = |line: &str| r.line(Column::A, line).unwrap().expected;

        // Values asserted by the FEC's test_calculate_summary_column_a.
        assert_eq!(expected("9"), dec!(250.00));
        assert_eq!(expected("10"), dec!(250.00));
        assert_eq!(expected("11(a)(i)"), dec!(10000.23));
        // 11(a)(ii) (3.77 in the FEC's test) is an input here: FECfile+
        // stores unitemized receipts as internal SA11AII transactions, but
        // a filed .fec never carries them.
        assert!(r.line(Column::A, "11(a)(ii)").is_none());
        assert_eq!(expected("11(b)"), dec!(444.44));
        assert_eq!(expected("11(c)"), dec!(555.55));
        assert_eq!(expected("12"), dec!(1212.12));
        assert_eq!(expected("13"), dec!(1313.13));
        assert_eq!(expected("14"), dec!(1414.14));
        assert_eq!(expected("15"), dec!(2125.79));
        assert_eq!(expected("16"), dec!(16.00));
        assert_eq!(expected("17"), dec!(1000.00));
        assert_eq!(expected("21(b)"), dec!(150.00));
        assert_eq!(expected("22"), dec!(22.00));
        assert_eq!(expected("23"), dec!(14.00));
        assert_eq!(expected("24"), dec!(151.00));
        assert_eq!(expected("25"), dec!(133.00));
        assert_eq!(expected("26"), dec!(44.00));
        assert_eq!(expected("27"), dec!(31.00));
        assert_eq!(expected("28(a)"), dec!(101.50));
        assert_eq!(expected("28(b)"), dec!(201.50));
        assert_eq!(expected("28(c)"), dec!(301.50));
        assert_eq!(expected("29"), dec!(201.50));
        assert_eq!(expected("30(b)"), dec!(102.25));

        // Formulas evaluate over reported values; feed the FEC's computed
        // figures back into the cover and the derived lines must match
        // theirs: 11(a)(iii) 10004.00, 11(d) 11003.99, 6(c)/19 18085.17,
        // 28(d) 604.50, 31/7 1453.25, 8 16631.92, 38 = 150 - 2125.79.
        let mut cover = vec![
            ("col_a_individuals_itemized", "10000.23"),
            ("col_a_individuals_unitemized", "3.77"),
            ("col_a_political_party_committees", "444.44"),
            ("col_a_other_political_committees_pacs", "555.55"),
            ("col_a_individual_contribution_total", "10004.00"),
            ("col_a_total_contributions", "11003.99"),
            ("col_a_transfers_from_aff_other_party_cmttees", "1212.12"),
            ("col_a_total_loans", "1313.13"),
            ("col_a_total_loan_repayments_received", "1414.14"),
            ("col_a_offsets_to_expenditures", "2125.79"),
            ("col_a_refunds_of_federal_contributions", "16.00"),
            ("col_a_other_federal_receipts", "1000.00"),
            ("col_a_total_nonfederal_transfers", "0"),
            ("col_a_total_receipts_recap", "18085.17"),
            ("col_a_total_federal_receipts", "18085.17"),
            ("col_a_total_receipts", "18085.17"),
            ("col_a_other_federal_operating_expenditures", "150.00"),
            ("col_a_total_operating_expenditures", "150.00"),
            ("col_a_transfers_to_affiliated", "22.00"),
            ("col_a_contributions_to_candidates", "14.00"),
            ("col_a_independent_expenditures", "151.00"),
            (
                "col_a_coordinated_expenditures_by_party_committees",
                "133.00",
            ),
            ("col_a_total_loan_repayments_made", "44.00"),
            ("col_a_loans_made", "31.00"),
            ("col_a_refunds_to_individuals", "101.50"),
            ("col_a_refunds_to_party_committees", "201.50"),
            ("col_a_refunds_to_other_committees", "301.50"),
            ("col_a_total_refunds", "604.50"),
            ("col_a_other_disbursements", "201.50"),
            ("col_a_federal_election_activity_all_federal", "102.25"),
            ("col_a_federal_election_activity_total", "102.25"),
            ("col_a_total_disbursements_recap", "1453.25"),
            ("col_a_total_disbursements", "1453.25"),
            ("col_a_total_federal_disbursements", "1453.25"),
            ("col_a_total_contributions_recap", "11003.99"),
            ("col_a_total_contributions_refunds", "604.50"),
            ("col_a_net_contributions", "10399.49"),
            ("col_a_total_federal_operating_expenditures", "150.00"),
            ("col_a_total_offsets_to_expenditures", "2125.79"),
            ("col_a_net_operating_expenditures", "-1975.79"),
            ("col_a_debts_to", "250.00"),
            ("col_a_debts_by", "250.00"),
            ("col_a_cash_on_hand_beginning_period", "0"),
            ("col_a_subtotal", "18085.17"),
            ("col_a_cash_on_hand_close_of_period", "16631.92"),
        ];
        cover.sort();
        let filing = f3x_with(&cover, filing.lines.clone());
        let r = filing.reconcile().unwrap();
        let disagreeing: Vec<String> = r
            .column(Column::A)
            .filter(|c| !c.matches())
            .map(ToString::to_string)
            .collect();
        assert!(disagreeing.is_empty(), "{}", disagreeing.join("\n"));
    }
}
