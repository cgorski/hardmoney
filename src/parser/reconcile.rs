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
//! disbursements, refunds to individuals, independent expenditures,
//! contributions from the candidate, 100%-federal election activity) are
//! checked as *floors*: the cover total must be at least the itemized sum.
//! See [`Relation`] for the line-by-line derivation from the FEC's own
//! instructions.
//!
//! # Allocation schedules (H3-H6)
//!
//! Party committees that keep a nonfederal account report allocated
//! activity on Schedules H3-H6, which feed F3X lines 18(a), 18(b),
//! 21(a)(i), 21(a)(ii), 30(a)(i), and 30(a)(ii). The rules for those lines
//! were checked in September 2026 against 45 FEC-accepted F3X reports from
//! 24 state party committees (Ohio, Florida, Michigan, Wisconsin,
//! Pennsylvania, North Carolina, Arizona, Nevada, Georgia, California,
//! Texas, Minnesota, Virginia, Iowa, Colorado, New Hampshire, Maine; both
//! parties), carrying between 3 and 648 H4 records each and up to 33 H3
//! records. Every one balanced exactly. What the data settled:
//!
//! * **18(a) is the sum of H3 `transferred_amount`, not
//!   `total_amount_transferred`.** One transfer from the nonfederal account
//!   is filed as one `AD` (administrative) H3 record plus zero or more
//!   records for other event types (`DF`, `DC`, `GV`, ...) that
//!   back-reference it. *Every* record in the group repeats the transfer's
//!   total in `total_amount_transferred` and carries its own share in
//!   `transferred_amount`; the shares sum to the total. Summing
//!   `total_amount_transferred` would count a split transfer once per
//!   event type.
//! * **21(a)(i)/(ii) are the sums of H4 `federal_share` /
//!   `nonfederal_share` with memo entries excluded.** 4,065 of the H4
//!   records in the sample were memos (credit-card and payroll breakdowns
//!   back-referencing a parent H4); including them would double every
//!   affected line. No filing carried an `SB21A` record: Schedule H4 is the
//!   only itemization of 21(a).
//! * **30(b) is a floor, not an identity.** 11 CFR 300.36(b)(2)(iv) and the
//!   Form 3X instructions ("Itemize all such disbursements of $200 or more
//!   on Schedule B for Line 30(b)") put federal election activity under the
//!   itemization threshold, and 7 of the 45 reports had a 30(b) total above
//!   their `SB30B` sum by $5 to $336.
//! * H5 (Levin transfers) and H6 (Levin-allocated disbursements) appeared
//!   in none of the reports -- Levin funds have all but vanished since
//!   2002 -- so 18(b), 30(a)(i), and 30(a)(ii) rest on the spec's field
//!   layout alone: H5 is one record per transfer carrying
//!   `total_amount_transferred` plus four category breakdowns (the FEC's
//!   own warning #49 checks they agree), and H6 mirrors H4 with
//!   `federal_share` / `levin_share`.
//!
//! National party committees (RNC, DNC, NRCC, DCCC, DSCC, NRSC; 24 recent
//! reports) file no H schedules at all -- BCRA bars them from nonfederal
//! accounts -- and every one of their reports balanced too, save a 12-cent
//! gap on one NRSC line 12 (see `tests/fixtures/ORACLE_NOTES.md`).
//!
//! # Arithmetic
//!
//! Sums use [`Decimal::saturating_add`] / [`Decimal::saturating_sub`] so
//! the reconciler can never panic, but saturation cannot hide a
//! discrepancy: `Decimal` holds about ±7.9 × 10^28, an `AMT-12` field
//! holds at most 999,999,999,999.99 (≈ 10^12), so it would take more than
//! 7 × 10^16 body lines -- a file of several exabytes -- to reach the limit.
//! Amounts that fail to parse are skipped in schedule sums (a
//! [`Rule::InvalidAmount`](crate::parser::Rule::InvalidAmount) finding
//! from [`Filing::validate`] reports them) and reported as
//! [`LineCheck::reported_unparseable`] on the cover.
//!
//! # Column B across a chain of reports
//!
//! One file cannot check its own Column B schedule lines: they sum every
//! report in the period. [`ReportChain`] takes the current report and the
//! committee's earlier reports and does what the FEC's Form 3X
//! instructions say a filer does ("add the Calendar Year-to-Date total
//! from the previous report to the Total This Period"), except from the
//! schedules: each Column B schedule line is compared with the sum of that
//! line's schedule sums over every report in the period, and cash on hand
//! is carried forward from the prior report's close to the current one's
//! beginning. The period is the calendar year for Form 3X and the election
//! cycle for Forms 3 and 3P ([`PeriodBasis`]). FECfile+'s
//! `calculate_summary_column_b` and `calculate_cash_on_hand_fields` are
//! the FEC's own implementation of the same arithmetic; this one differs
//! only in reading filed `.fec` files rather than a transaction database.
//!
//! # Tolerance
//!
//! Comparisons are exact by default. [`Reconciliation::mismatches_over`]
//! applies a tolerance (e.g. `0.01` for a filer who rounds each line).

use std::fmt;

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;

use crate::parser::filing::{Filing, ParsedLine};
use crate::parser::tables::Table;
use crate::parser::typed::{parse_fec_date, parse_money};

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
/// 104.3(a)(4), (b)(3), (b)(4)); anything smaller is reported in the cover
/// total but need not appear on the schedule. So for those lines the
/// schedule sum is a **floor**, not an identity -- a cover total below its
/// itemized sum is a discrepancy, a cover total above it is normal. Lines
/// that must be itemized regardless of size must match **exactly**.
///
/// Which is which comes from the FEC's own form instructions (Forms 3X,
/// 3, and 3P, revised 05/2016), whose line-by-line text says either
/// "must be itemized ... regardless of the amount" or "aggregating in
/// excess of $200":
///
/// | Itemized regardless of amount ([`Equal`](Relation::Equal)) | $200 threshold ([`AtLeast`](Relation::AtLeast)) |
/// |---|---|
/// | contributions from party committees and PACs (F3X 11(b), 11(c); F3 11(b), 11(c); F3P 17(b), 17(c)) | contributions from the candidate (F3 11(d), F3P 17(d)) |
/// | transfers in and out (F3X 12, 22; F3 12, 18; F3P 18, 24) | offsets to operating expenditures (F3X 15; F3 14; F3P 20(a)-(c)) |
/// | loans received, made, and repaid (F3X 13, 14, 26, 27; F3 13(a), 13(b), 19(a), 19(b); F3P 19(a), 19(b), 27(a), 27(b)) | other receipts (F3X 17; F3 15; F3P 21) |
/// | refunds of contributions made to committees (F3X 16) | operating expenditures (F3X 21(b); F3 17; F3P 23, 25, 26) |
/// | contributions to candidates and committees (F3X 23) | independent expenditures (F3X 24: Schedule E's paper form has an unitemized "Line (b)" the electronic `SE` layout lacks) |
/// | coordinated party expenditures (F3X 25: "must itemize each expenditure on Schedule F") | refunds to individuals (F3X 28(a); F3 20(a); F3P 28(a): itemized only if the original contribution was) |
/// | refunds to party committees and PACs (F3X 28(b), 28(c); F3 20(b), 20(c); F3P 28(b), 28(c)) | other disbursements (F3X 29; F3 21; F3P 29) |
/// | federal funds (F3P 16), debts (Schedules C and D), and the allocation schedules H3-H6 | 100%-federal election activity (F3X 30(b); 11 CFR 300.36(b)(2)(iv)) |
///
/// Line 11(a)(i) (itemized individuals) is `Equal` by definition: it *is*
/// the itemized total, and the rest goes on 11(a)(ii).
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
    /// How many filings were read to compute `expected`: 1 for every check
    /// [`Filing::reconcile`] produces and for a cash-on-hand carry-forward
    /// (which reads one prior report), the number of reports in the period
    /// for a Column B sum from a [`ReportChain`].
    pub reports_summed: usize,
    /// True when the cover field was non-blank but not a valid amount
    /// (`reported` is then `None` and counts as 0). A *blank* field gives
    /// `reported == None` with this `false`: blank is a legitimate way to
    /// write zero, garbage is not.
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
        Source::Schedules(_, sums) => schedule_sum(body, sums),
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
        reports_summed: 1,
        reported_unparseable: unparseable,
    }
}

/// Sums `sums` over the non-memo body lines: the total and how many lines
/// contributed. Amounts that do not parse are skipped.
fn schedule_sum(body: &[ParsedLine], sums: &[ScheduleSum]) -> (Decimal, usize) {
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

// ---------------------------------------------------------------------------
// Chains of reports: Column B and cash on hand
// ---------------------------------------------------------------------------

/// The period a report's Column B accumulates over.
///
/// The Form 3X instructions call Column B "Calendar Year-to-Date"; the
/// Form 3 and 3P instructions call it "Election Cycle-to-Date" ("the
/// election cycle for disclosure purposes begins the day after the
/// previous general election for a seat or office, and ends on the day of
/// the next general election"). A [`ReportChain`] uses the basis to decide
/// which prior reports feed the Column B sums: for a year-to-date form,
/// only reports whose coverage ends in the same calendar year the current
/// report begins in (the rule FECfile+ applies when it sums by the
/// transaction's calendar year); for a cycle-to-date form, every prior
/// report given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum PeriodBasis {
    /// Column B is the calendar year to date (Form 3X).
    YearToDate,
    /// Column B is the election cycle to date (Forms 3 and 3P).
    CycleToDate,
}

impl PeriodBasis {
    /// The basis a cover form's Column B uses; `None` for forms without
    /// reconciliation rules.
    #[must_use]
    pub fn for_form(form: Table) -> Option<PeriodBasis> {
        match form {
            Table::F3X => Some(PeriodBasis::YearToDate),
            Table::F3 | Table::F3P => Some(PeriodBasis::CycleToDate),
            _ => None,
        }
    }
}

impl fmt::Display for PeriodBasis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PeriodBasis::YearToDate => "year to date",
            PeriodBasis::CycleToDate => "cycle to date",
        })
    }
}

/// A report's coverage period, from the cover line's `coverage_from_date`
/// and `coverage_through_date`. Both ends are inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Coverage {
    pub from: NaiveDate,
    pub through: NaiveDate,
}

impl Coverage {
    /// True when the two periods share at least one day.
    #[must_use]
    pub fn overlaps(&self, other: &Coverage) -> bool {
        self.from <= other.through && other.from <= self.through
    }
}

impl fmt::Display for Coverage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.from, self.through)
    }
}

/// The coverage period on a report's cover line, or `None` if either date
/// is blank, not `YYYYMMDD`, or the period ends before it begins.
#[must_use]
pub fn coverage(filing: &Filing) -> Option<Coverage> {
    let from = filing
        .summary
        .get_non_empty("coverage_from_date")
        .and_then(parse_fec_date)?;
    let through = filing
        .summary
        .get_non_empty("coverage_through_date")
        .and_then(parse_fec_date)?;
    (from <= through).then_some(Coverage { from, through })
}

/// Why a set of reports cannot be chained. `index` is the position of the
/// offending report in the `prior` iterator given to [`ReportChain::new`],
/// counting from 0.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ChainError {
    /// The current report's cover form has no reconciliation rules.
    #[error(transparent)]
    UnsupportedForm(#[from] ReconcileError),
    /// The current report has no usable coverage dates.
    #[error(
        "the current report has no usable coverage period (coverage_from_date and \
         coverage_through_date must both be YYYYMMDD dates, from on or before through)"
    )]
    CurrentCoverageMissing,
    /// A prior report has no usable coverage dates.
    #[error(
        "prior report {index} has no usable coverage period (coverage_from_date and \
         coverage_through_date must both be YYYYMMDD dates, from on or before through)"
    )]
    PriorCoverageMissing { index: usize },
    /// A prior report is on a different cover form.
    #[error(
        "prior report {index} is a {found} but the current report is a {expected}; a chain \
         is one committee's reports on one form"
    )]
    FormMismatch {
        index: usize,
        expected: Table,
        found: Table,
    },
    /// A prior report was filed by a different committee.
    #[error(
        "prior report {index} was filed by {found} but the current report by {expected}; a \
         chain is one committee's reports"
    )]
    FilerMismatch {
        index: usize,
        expected: String,
        found: String,
    },
    /// A prior report does not end before the current report begins.
    #[error(
        "prior report {index} covers {coverage}, which does not end before the current \
         report's period {current} begins; only earlier reports belong in the chain"
    )]
    NotBeforeCurrent {
        index: usize,
        coverage: Coverage,
        current: Coverage,
    },
    /// Two prior reports cover overlapping periods (an original and its
    /// amendment, for example).
    #[error(
        "prior reports {a} ({a_coverage}) and {b} ({b_coverage}) overlap; a chain holds one \
         version of each report -- keep the most recent amendment and drop the rest"
    )]
    Overlap {
        a: usize,
        a_coverage: Coverage,
        b: usize,
        b_coverage: Coverage,
    },
}

/// The cash-on-hand lines a chain carries from one report to the next,
/// by FEC line label.
struct CashLines {
    /// Column A: cash on hand at the beginning of the period.
    beginning: &'static str,
    /// Column A: cash on hand at the close of the period.
    close: &'static str,
    /// Column B: cash on hand on January 1 (Form 3X only).
    year_start: Option<&'static str>,
}

fn cash_lines(form: Table) -> Option<CashLines> {
    match form {
        Table::F3X => Some(CashLines {
            beginning: "6(b)",
            close: "8",
            year_start: Some("6(a)"),
        }),
        Table::F3 => Some(CashLines {
            beginning: "23",
            close: "27",
            year_start: None,
        }),
        Table::F3P => Some(CashLines {
            beginning: "6",
            close: "10",
            year_start: None,
        }),
        _ => None,
    }
}

/// The cover field a rule table maps a `(column, line)` to.
fn field_of(rules: &[LineRule], column: Column, line: &str) -> Option<&'static str> {
    rules
        .iter()
        .find(|r| r.column == column && r.line == line)
        .map(|r| r.field)
}

fn filer_id(filing: &Filing) -> &str {
    filing
        .summary
        .get_non_empty("filer_committee_id_number")
        .unwrap_or("")
        .trim()
}

/// A periodic report together with the committee's earlier reports, for
/// the checks one file cannot make on its own: Column B schedule sums and
/// cash-on-hand carry-forward.
///
/// Build one with [`ReportChain::new`], which checks that every prior
/// report is on the same form, from the same filer, and covers a period
/// that ends before the current report begins and overlaps no other. Then
/// [`ReportChain::reconcile`] (or [`column_b_checks`](Self::column_b_checks)
/// and [`carry_forward_checks`](Self::carry_forward_checks) separately)
/// produces [`LineCheck`]s. The current report's own Column A and Column B
/// formula checks come from [`Filing::reconcile`]; the two sets do not
/// overlap.
///
/// The chain need not be complete. Column B sums are only meaningful when
/// every report of the period is present, so [`gaps`](Self::gaps) reports
/// the days between the start of the period (January 1 for a year-to-date
/// form) and the current report that no report in the chain covers.
///
/// ```
/// use hardmoney::Filing;
/// use hardmoney::parser::reconcile::{Column, ReportChain};
///
/// let dir = "tests/fixtures/chain/";
/// let jan = Filing::open(format!("{dir}F3XN_1948502.fec"))?;
/// let feb = Filing::open(format!("{dir}F3XA_2011895.fec"))?;
/// let mar = Filing::open(format!("{dir}F3XA_2011898.fec"))?;
/// let chain = ReportChain::new(&mar, [&jan, &feb])?;
/// assert!(chain.gaps().is_empty());
/// let r = chain.reconcile();
/// assert!(r.balances(), "{r}");
/// let itemized = r.line(Column::B, "11(a)(i)").unwrap();
/// assert_eq!(itemized.reports_summed, 3);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct ReportChain<'a> {
    current: &'a Filing,
    /// Oldest first.
    prior: Vec<&'a Filing>,
    form: Table,
    basis: PeriodBasis,
    current_coverage: Coverage,
}

impl<'a> ReportChain<'a> {
    /// Chains `prior` (in any order) behind `current`.
    ///
    /// Fails with [`ChainError::UnsupportedForm`] if `current` is not an
    /// F3X, F3, or F3P; [`ChainError::CurrentCoverageMissing`] or
    /// [`ChainError::PriorCoverageMissing`] if a cover line lacks parseable
    /// coverage dates; [`ChainError::FormMismatch`] or
    /// [`ChainError::FilerMismatch`] if a prior report is on another form
    /// or from another committee; [`ChainError::NotBeforeCurrent`] if a
    /// prior report does not end before `current` begins; and
    /// [`ChainError::Overlap`] if two prior reports share a day (an
    /// original and its amendment, for instance). Prior reports from an
    /// earlier year are accepted on a year-to-date form: they are left out
    /// of the Column B sums but supply the cash-on-hand carry-forward.
    pub fn new<I>(current: &'a Filing, prior: I) -> Result<Self, ChainError>
    where
        I: IntoIterator<Item = &'a Filing>,
    {
        let form = current.summary.table();
        let basis = PeriodBasis::for_form(form).ok_or(ChainError::UnsupportedForm(
            ReconcileError::UnsupportedForm(form),
        ))?;
        let current_coverage = coverage(current).ok_or(ChainError::CurrentCoverageMissing)?;
        let filer = filer_id(current);

        let mut sorted: Vec<(usize, Coverage, &'a Filing)> = Vec::new();
        for (index, report) in prior.into_iter().enumerate() {
            let found = report.summary.table();
            if found != form {
                return Err(ChainError::FormMismatch {
                    index,
                    expected: form,
                    found,
                });
            }
            let report_filer = filer_id(report);
            if !report_filer.eq_ignore_ascii_case(filer) {
                return Err(ChainError::FilerMismatch {
                    index,
                    expected: filer.to_string(),
                    found: report_filer.to_string(),
                });
            }
            let cov = coverage(report).ok_or(ChainError::PriorCoverageMissing { index })?;
            if cov.through >= current_coverage.from {
                return Err(ChainError::NotBeforeCurrent {
                    index,
                    coverage: cov,
                    current: current_coverage,
                });
            }
            for (other_index, other, _) in &sorted {
                if other.overlaps(&cov) {
                    return Err(ChainError::Overlap {
                        a: *other_index,
                        a_coverage: *other,
                        b: index,
                        b_coverage: cov,
                    });
                }
            }
            sorted.push((index, cov, report));
        }
        sorted.sort_by_key(|(_, cov, _)| *cov);
        Ok(ReportChain {
            current,
            prior: sorted.into_iter().map(|(_, _, f)| f).collect(),
            form,
            basis,
            current_coverage,
        })
    }

    /// The report whose Column B is checked.
    #[must_use]
    pub fn current(&self) -> &'a Filing {
        self.current
    }

    /// Every prior report, oldest first.
    #[must_use]
    pub fn prior(&self) -> &[&'a Filing] {
        &self.prior
    }

    /// The cover form every report in the chain is on.
    #[must_use]
    pub fn form(&self) -> Table {
        self.form
    }

    /// What Column B accumulates over on this form.
    #[must_use]
    pub fn basis(&self) -> PeriodBasis {
        self.basis
    }

    /// The current report's coverage period.
    #[must_use]
    pub fn current_coverage(&self) -> Coverage {
        self.current_coverage
    }

    /// The reports whose schedules feed the current report's Column B,
    /// oldest first and ending with the current report: every prior report
    /// on a cycle-to-date form, and on a year-to-date form those whose
    /// coverage ends in the calendar year the current report begins in.
    #[must_use]
    pub fn period_reports(&self) -> Vec<&'a Filing> {
        let year = self.current_coverage.from.year();
        let mut reports: Vec<&'a Filing> = self
            .prior
            .iter()
            .copied()
            .filter(|f| match self.basis {
                PeriodBasis::CycleToDate => true,
                PeriodBasis::YearToDate => coverage(f).is_some_and(|c| c.through.year() == year),
            })
            .collect();
        reports.push(self.current);
        reports
    }

    /// On a year-to-date form, the latest prior report that closed before
    /// the current report's calendar year began: the one whose cash on hand
    /// at close is the current year's line 6(a). `None` on a cycle-to-date
    /// form or when the chain has no earlier-year report.
    #[must_use]
    pub fn last_report_of_prior_year(&self) -> Option<&'a Filing> {
        if self.basis != PeriodBasis::YearToDate {
            return None;
        }
        let year = self.current_coverage.from.year();
        self.prior
            .iter()
            .rev()
            .copied()
            .find(|f| coverage(f).is_some_and(|c| c.through.year() < year))
    }

    /// The days between the start of the period and the current report
    /// that no report in the chain covers, oldest first. Empty means the
    /// chain is complete and the Column B sums cover everything they should.
    ///
    /// On a year-to-date form the period starts on January 1 of the current
    /// report's year. On a cycle-to-date form the start of the cycle is not
    /// known from the filings (it is the day after the previous general
    /// election for the seat), so only gaps between the reports given are
    /// reported.
    #[must_use]
    pub fn gaps(&self) -> Vec<Coverage> {
        let mut gaps = Vec::new();
        let reports = self.period_reports();
        let mut cursor: Option<NaiveDate> = match self.basis {
            PeriodBasis::YearToDate => {
                NaiveDate::from_ymd_opt(self.current_coverage.from.year(), 1, 1)
            }
            PeriodBasis::CycleToDate => {
                // Start from the first report given; nothing before it can be
                // called a gap.
                reports.first().and_then(|f| coverage(f)).map(|c| c.from)
            }
        };
        for report in reports {
            let Some(cov) = coverage(report) else {
                continue;
            };
            if let Some(expected_from) = cursor
                && cov.from > expected_from
                && let Some(through) = cov.from.pred_opt()
            {
                gaps.push(Coverage {
                    from: expected_from,
                    through,
                });
            }
            cursor = cov.through.succ_opt();
        }
        gaps
    }

    /// Column B schedule checks: for every Column A line that is a schedule
    /// sum, the current report's Column B value compared (with the same
    /// [`Relation`]) with the sum of that schedule over every report in
    /// [`period_reports`](Self::period_reports). Lines the current report's
    /// spec version lacks are omitted. `reports_summed` on each check is the
    /// number of reports summed.
    #[must_use]
    pub fn column_b_checks(&self) -> Vec<LineCheck> {
        let Some(rules) = rules_for(self.form) else {
            return Vec::new();
        };
        let reports = self.period_reports();
        let cover = &self.current.summary;
        let mut checks = Vec::new();
        for a in rules.iter().filter(|r| r.column == Column::A) {
            let Source::Schedules(relation, sums) = a.source else {
                continue;
            };
            let Some(b) = rules
                .iter()
                .find(|r| r.column == Column::B && r.line == a.line)
            else {
                continue;
            };
            if cover.get(b.field).is_none() {
                continue;
            }
            let (reported, unparseable) = reported_amount(cover, b.field);
            let mut expected = Decimal::ZERO;
            let mut lines_summed = 0usize;
            for report in &reports {
                let (total, n) = schedule_sum(&report.lines, sums);
                expected = expected.saturating_add(total);
                lines_summed = lines_summed.saturating_add(n);
            }
            let delta = reported.unwrap_or(Decimal::ZERO).saturating_sub(expected);
            checks.push(LineCheck {
                line: b.line,
                field: b.field,
                column: Column::B,
                rule: format!(
                    "{} over {} report(s), {}",
                    a.formula_text(),
                    reports.len(),
                    self.basis
                ),
                reported,
                expected,
                delta,
                relation,
                lines_summed,
                reports_summed: reports.len(),
                reported_unparseable: unparseable,
            });
        }
        checks
    }

    /// Cash-on-hand carry-forward checks, each [`Relation::Equal`]:
    ///
    /// * the current report's cash on hand at the beginning of the period
    ///   (F3X 6(b), F3 23, F3P 6; Column A) against the immediately prior
    ///   report's cash on hand at close (F3X 8, F3 27, F3P 10);
    /// * on Form 3X, cash on hand on January 1 (6(a); Column B) against the
    ///   close of [`last_report_of_prior_year`](Self::last_report_of_prior_year),
    ///   when the chain has one.
    ///
    /// Empty when the chain has no prior report.
    #[must_use]
    pub fn carry_forward_checks(&self) -> Vec<LineCheck> {
        let Some(rules) = rules_for(self.form) else {
            return Vec::new();
        };
        let Some(cash) = cash_lines(self.form) else {
            return Vec::new();
        };
        let cover = &self.current.summary;
        let mut checks = Vec::new();
        let carry = |line: &'static str, column: Column, from: &Filing| -> Option<LineCheck> {
            let field = field_of(rules, column, line)?;
            cover.get(field)?;
            let close_field = field_of(rules, Column::A, cash.close)?;
            let (reported, unparseable) = reported_amount(cover, field);
            let expected = reported_amount(&from.summary, close_field)
                .0
                .unwrap_or(Decimal::ZERO);
            let delta = reported.unwrap_or(Decimal::ZERO).saturating_sub(expected);
            let when = coverage(from).map_or_else(String::new, |c| format!(" ({c})"));
            Some(LineCheck {
                line,
                field,
                column,
                rule: format!("= {} of the prior report{when}", cash.close),
                reported,
                expected,
                delta,
                relation: Relation::Equal,
                lines_summed: 0,
                reports_summed: 1,
                reported_unparseable: unparseable,
            })
        };
        if let Some(previous) = self.prior.last()
            && let Some(check) = carry(cash.beginning, Column::A, previous)
        {
            checks.push(check);
        }
        if let Some(line) = cash.year_start
            && let Some(previous) = self.last_report_of_prior_year()
            && let Some(check) = carry(line, Column::B, previous)
        {
            checks.push(check);
        }
        checks
    }

    /// [`column_b_checks`](Self::column_b_checks) followed by
    /// [`carry_forward_checks`](Self::carry_forward_checks), as one
    /// [`Reconciliation`] for the current report's form.
    pub fn reconcile(&self) -> Reconciliation {
        let mut checks = self.column_b_checks();
        checks.extend(self.carry_forward_checks());
        Reconciliation {
            form: self.form,
            checks,
        }
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
/// Schedules H3, H4, H5, and H6 per the spec's rule text and verified
/// against 45 state-party reports; see the [module docs](self)); the Form
/// 3X instructions for the `<-` / `>=` split ([`Relation`]). FECfile+
/// sums Schedule E for line 24 because, as the filing tool, it holds every
/// transaction; a filed `.fec` need not itemize sub-$200 payees, so here 24
/// is a floor.
pub static F3X_RULES: &[LineRule] = rules! {
    A:
        "9" col_a_debts_to { <- SchC["SC/9"].loan_balance  SchD["SD9"].balance_at_close_this_period };
        "10" col_a_debts_by { <- SchC["SC/10"].loan_balance SchD["SD10"].balance_at_close_this_period };
        "11(a)(i)" col_a_individuals_itemized { <- SchA["SA11AI", "SA11A1"].contribution_amount };
        "11(a)(ii)" col_a_individuals_unitemized { input };
        "11(a)(iii)" col_a_individual_contribution_total { = "11(a)(i)" + "11(a)(ii)" };
        "11(b)" col_a_political_party_contributions { <- SchA["SA11B"].contribution_amount };
        "11(c)" col_a_pac_contributions { <- SchA["SA11C"].contribution_amount };
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
        "24" col_a_independent_expenditures { >= SchE["SE"].expenditure_amount };
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
        "30(b)" col_a_federal_election_activity_all_federal { >= SchB["SB30B"].expenditure_amount };
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
        "11(b)" col_b_political_party_contributions { input };
        "11(c)" col_b_pac_contributions { input };
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
/// Sources: FEC format spec sheet `F3`, `RULE REFERENCE` column; the Form
/// 3 instructions for the `<-` / `>=` split (11(d), contributions from the
/// candidate, is itemized only "aggregating in excess of $200", so it is a
/// floor). FECfile+ computes this form as all zeros. Checked against 12
/// recent House and Senate reports carrying Schedule C loans and Schedule
/// D debts (lines 9 and 10); all balanced.
pub static F3_RULES: &[LineRule] = rules! {
    A:
        "9" col_a_debts_to { <- SchC["SC/9"].loan_balance  SchD["SD9"].balance_at_close_this_period };
        "10" col_a_debts_by { <- SchC["SC/10"].loan_balance SchD["SD10"].balance_at_close_this_period };
        "11(a)(i)" col_a_individuals_itemized { <- SchA["SA11AI", "SA11A1"].contribution_amount };
        "11(a)(ii)" col_a_individuals_unitemized { input };
        "11(a)(iii)" col_a_individual_contribution_total { = "11(a)(i)" + "11(a)(ii)" };
        "11(b)" col_a_political_party_contributions { <- SchA["SA11B"].contribution_amount };
        "11(c)" col_a_pac_contributions { <- SchA["SA11C"].contribution_amount };
        "11(d)" col_a_candidate_contributions { >= SchA["SA11D"].contribution_amount };
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
        "23" col_a_cash_on_hand_beginning_period { input };
        "24" col_a_total_receipts_period { = "16" };
        "25" col_a_subtotal { = "23" + "24" };
        "26" col_a_total_disbursements_period { = "22" };
        "27" col_a_cash_on_hand_close { = "25" - "26" };
        "8" col_a_cash_on_hand_close_of_period { = "27" };
    B:
        "11(a)(i)" col_b_individuals_itemized { input };
        "11(a)(ii)" col_b_individuals_unitemized { input };
        "11(a)(iii)" col_b_individual_contribution_total { = "11(a)(i)" + "11(a)(ii)" };
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
/// Sources: FEC format spec sheet `F3P`, `RULE REFERENCE` column; the Form
/// 3P instructions for the `<-` / `>=` split (16, federal funds, "must be
/// itemized ... regardless of the amount"; 17(d), contributions from the
/// candidate, only "in excess of $200"). Line 17(a)(i) sums `SA17A`
/// (itemized individuals); unitemized 17(a)(ii) has no schedule. Lines 14
/// and 15 are Column-B-derived per the spec (`= 17e Col B - 28d Col B`,
/// `= 23 Col B - 20a Col B`). Checked against ten 2024-cycle presidential
/// reports (Harris, Kennedy, Haley, Ramaswamy) at spec 8.4 and four 2026
/// filings at 8.5; see `tests/fixtures/ORACLE_NOTES.md` for the one that
/// does not balance.
pub static F3P_RULES: &[LineRule] = rules! {
    A:
        "11" col_a_debts_to { <- SchC["SC/11"].loan_balance  SchD["SD11"].balance_at_close_this_period };
        "12" col_a_debts_by { <- SchC["SC/12"].loan_balance  SchD["SD12"].balance_at_close_this_period };
        "16" col_a_federal_funds { <- SchA["SA16"].contribution_amount };
        "17(a)(i)" col_a_individuals_itemized { <- SchA["SA17A"].contribution_amount };
        "17(a)(ii)" col_a_individuals_unitemized { input };
        "17(a)(iii)" col_a_individual_contribution_total { = "17(a)(i)" + "17(a)(ii)" };
        "17(b)" col_a_political_party_contributions { <- SchA["SA17B"].contribution_amount };
        "17(c)" col_a_pac_contributions { <- SchA["SA17C"].contribution_amount };
        "17(d)" col_a_candidate_contributions { >= SchA["SA17D"].contribution_amount };
        "17(e)" col_a_total_contributions { = "17(a)(iii)" + "17(b)" + "17(c)" + "17(d)" };
        "18" col_a_transfers_from_aff_other_party_cmttees { <- SchA["SA18"].contribution_amount };
        "19(a)" col_a_candidate_loans { <- SchA["SA19A"].contribution_amount };
        "19(b)" col_a_other_loans { <- SchA["SA19B"].contribution_amount };
        "19(c)" col_a_total_loans { = "19(a)" + "19(b)" };
        "20(a)" col_a_offset_to_operating_expenditures { >= SchA["SA20A"].contribution_amount };
        "20(b)" col_a_offset_to_fundraising_expenditures { >= SchA["SA20B"].contribution_amount };
        "20(c)" col_a_offset_to_legal_expenditures { >= SchA["SA20C"].contribution_amount };
        "20(d)" col_a_total_offsets_to_expenditures { = "20(a)" + "20(b)" + "20(c)" };
        "21" col_a_other_receipts { >= SchA["SA21"].contribution_amount };
        "22" col_a_total_receipts_recap { = "16" + "17(e)" + "18" + "19(c)" + "20(d)" + "21" };
        "23" col_a_operating_expenditures { >= SchB["SB23"].expenditure_amount };
        "24" col_a_transfers_to_authorized { <- SchB["SB24"].expenditure_amount };
        "25" col_a_fundraising_disbursements { >= SchB["SB25"].expenditure_amount };
        "26" col_a_exempt_legal_disbursements { >= SchB["SB26"].expenditure_amount };
        "27(a)" col_a_candidate_loan_repayments { <- SchB["SB27A"].expenditure_amount };
        "27(b)" col_a_other_loan_repayments { <- SchB["SB27B"].expenditure_amount };
        "27(c)" col_a_total_loan_repayments_made { = "27(a)" + "27(b)" };
        "28(a)" col_a_refunds_to_individuals { >= SchB["SB28A"].expenditure_amount };
        "28(b)" col_a_refunds_to_party_committees { <- SchB["SB28B"].expenditure_amount };
        "28(c)" col_a_refunds_to_other_committees { <- SchB["SB28C"].expenditure_amount };
        "28(d)" col_a_total_refunds { = "28(a)" + "28(b)" + "28(c)" };
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
        "17(b)" col_b_political_party_contributions { input };
        "17(c)" col_b_pac_contributions { input };
        "17(d)" col_b_candidate_contributions { input };
        "17(e)" col_b_total_contributions { = "17(a)(iii)" + "17(b)" + "17(c)" + "17(d)" };
        "18" col_b_transfers_from_aff_other_party_cmttees { input };
        "19(a)" col_b_candidate_loans { input };
        "19(b)" col_b_other_loans { input };
        "19(c)" col_b_total_loans { = "19(a)" + "19(b)" };
        "20(a)" col_b_offset_to_operating_expenditures { input };
        "20(b)" col_b_offset_to_fundraising_expenditures { input };
        "20(c)" col_b_offset_to_legal_expenditures { input };
        "20(d)" col_b_total_offsets_to_expenditures { = "20(a)" + "20(b)" + "20(c)" };
        "21" col_b_other_receipts { input };
        "22" col_b_total_receipts { = "16" + "17(e)" + "18" + "19(c)" + "20(d)" + "21" };
        "23" col_b_operating_expenditures { input };
        "24" col_b_transfers_to_authorized { input };
        "25" col_b_fundraising_disbursements { input };
        "26" col_b_exempt_legal_disbursements { input };
        "27(a)" col_b_candidate_loan_repayments { input };
        "27(b)" col_b_other_loan_repayments { input };
        "27(c)" col_b_total_loan_repayments_made { = "27(a)" + "27(b)" };
        "28(a)" col_b_refunds_to_individuals { input };
        "28(b)" col_b_refunds_to_party_committees { input };
        "28(c)" col_b_refunds_to_other_committees { input };
        "28(d)" col_b_total_refunds { = "28(a)" + "28(b)" + "28(c)" };
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
        // Blank is zero, not garbage.
        let filing = f3x_with(&[("col_a_total_receipts", "")], vec![]);
        let r = filing.reconcile().unwrap();
        let c = r.line(Column::A, "6(c)").unwrap();
        assert!(!c.reported_unparseable);
        assert_eq!(c.reported, None);
        assert_eq!(c.delta, Decimal::ZERO);
    }

    /// Schedule H3 files one transfer as an `AD` record plus one record per
    /// other event type, each repeating the transfer's total in
    /// `total_amount_transferred` and carrying its own share in
    /// `transferred_amount` (the shape of every H3 group in the 45
    /// state-party reports the rules were checked against). 18(a) is the
    /// sum of the shares; summing the totals would double-count.
    #[test]
    fn h3_line_18a_sums_transferred_amount_not_the_repeated_total() {
        let h3 = |tid: &str, back: &str, event: &str, total: &str, share: &str| {
            ParsedLine::from_pairs(
                Table::H3,
                SpecVersion::electronic(8, 5),
                3,
                [
                    ("form_type", "H3"),
                    ("filer_committee_id_number", "C00123456"),
                    ("transaction_id", tid),
                    ("back_reference_tran_id", back),
                    ("account_name", "STATE CHECKING"),
                    ("event_type", event),
                    ("total_amount_transferred", total),
                    ("transferred_amount", share),
                ],
            )
            .unwrap()
        };
        // 46102.21 = 44060.51 administrative + 2041.70 direct fundraising
        // (a real MN DFL transfer), plus a 2605.25 transfer that is all AD.
        let filing = f3x_with(
            &[
                ("col_a_transfers_from_nonfederal_h3", "48707.46"),
                ("col_a_levin_funds", "0"),
                ("col_a_total_nonfederal_transfers", "48707.46"),
            ],
            vec![
                h3("4948AD", "4948AD", "AD", "46102.21", "44060.51"),
                h3("11388Q", "4948AD", "DF", "46102.21", "2041.70"),
                h3("4918AD", "4918AD", "AD", "2605.25", "2605.25"),
            ],
        );
        let r = filing.reconcile().unwrap();
        let c = r.line(Column::A, "18(a)").unwrap();
        assert_eq!(c.expected, dec!(48707.46));
        assert_eq!(c.lines_summed, 3);
        assert!(c.matches(), "{c}");
        assert!(r.line(Column::A, "18(c)").unwrap().matches());
    }

    /// 21(a)(i)/(ii) sum H4's federal and nonfederal shares with memo
    /// entries (payroll and credit-card breakdowns back-referencing a
    /// parent H4) excluded, and 36 = 21(a)(i) + 21(b) carries only the
    /// federal share forward.
    #[test]
    fn h4_lines_21a_split_the_shares_and_skip_memos() {
        let h4 = |tid: &str, total: &str, fed: &str, nonfed: &str, memo: bool| {
            let mut pairs = vec![
                ("form_type", "H4"),
                ("filer_committee_id_number", "C00123456"),
                ("transaction_id", tid),
                ("total_amount", total),
                ("federal_share", fed),
                ("nonfederal_share", nonfed),
            ];
            if memo {
                pairs.push(("memo_code", "X"));
            }
            ParsedLine::from_pairs(Table::H4, SpecVersion::electronic(8, 5), 3, pairs).unwrap()
        };
        let filing = f3x_with(
            &[
                ("col_a_shared_operating_expenditures_federal", "330.00"),
                ("col_a_shared_operating_expenditures_nonfederal", "670.00"),
                ("col_a_other_federal_operating_expenditures", "10.00"),
                ("col_a_total_operating_expenditures", "1010.00"),
                ("col_a_total_federal_operating_expenditures", "340.00"),
            ],
            vec![
                h4("H4.1", "1000.00", "330.00", "670.00", false),
                h4("H4.1a", "600.00", "198.00", "402.00", true),
                h4("H4.1b", "400.00", "132.00", "268.00", true),
            ],
        );
        let r = filing.reconcile().unwrap();
        for (line, want) in [("21(a)(i)", dec!(330.00)), ("21(a)(ii)", dec!(670.00))] {
            let c = r.line(Column::A, line).unwrap();
            assert_eq!(c.expected, want, "{c}");
            assert_eq!(c.lines_summed, 1, "{c}");
            assert!(c.matches(), "{c}");
        }
        assert!(r.line(Column::A, "21(c)").unwrap().matches());
        assert!(r.line(Column::A, "36").unwrap().matches());
    }

    /// The $200 itemization threshold makes 24 (independent expenditures)
    /// and 30(b) (100%-federal election activity) floors on Form 3X, and
    /// contributions from the candidate floors on Forms 3 and 3P: a cover
    /// total above the itemized sum is normal, below it is a discrepancy.
    #[test]
    fn threshold_lines_are_floors() {
        let filing = f3x_with(
            &[
                ("col_a_independent_expenditures", "250.00"),
                ("col_a_federal_election_activity_all_federal", "79762.86"),
            ],
            vec![
                body(Table::SchE, "SE", "expenditure_amount", "201.00", false),
                body(
                    Table::SchB,
                    "SB30B",
                    "expenditure_amount",
                    "79588.67",
                    false,
                ),
            ],
        );
        let r = filing.reconcile().unwrap();
        for line in ["24", "30(b)"] {
            let c = r.line(Column::A, line).unwrap();
            assert_eq!(c.relation, Relation::AtLeast, "{c}");
            assert!(c.delta > Decimal::ZERO, "{c}");
            assert!(c.matches(), "{c}");
            assert!(c.rule.starts_with(">= sum of"), "{c}");
        }
        // Below the itemized sum is still a violation.
        let filing = f3x_with(
            &[("col_a_independent_expenditures", "100.00")],
            vec![body(
                Table::SchE,
                "SE",
                "expenditure_amount",
                "201.00",
                false,
            )],
        );
        let c = filing.reconcile().unwrap();
        let c = c.line(Column::A, "24").unwrap();
        assert!(!c.matches());
        assert_eq!(c.violation(), dec!(101.00));

        let source = |rules: &'static [LineRule], line: &str| {
            rules
                .iter()
                .find(|r| r.column == Column::A && r.line == line)
                .map(|r| r.source)
        };
        assert!(matches!(
            source(F3_RULES, "11(d)"),
            Some(Source::Schedules(Relation::AtLeast, _))
        ));
        assert!(matches!(
            source(F3P_RULES, "17(d)"),
            Some(Source::Schedules(Relation::AtLeast, _))
        ));
        // Their committee counterparts stay exact.
        assert!(matches!(
            source(F3_RULES, "11(c)"),
            Some(Source::Schedules(Relation::Equal, _))
        ));
        assert!(matches!(
            source(F3P_RULES, "16"),
            Some(Source::Schedules(Relation::Equal, _))
        ));
    }

    /// Every form has a Column B table, and every Column B rule names a
    /// field the 8.5 layout has (the spec's labels differ between columns,
    /// e.g. F3P 17(e) is "Total contributions (Other than Loans)" in B).
    #[test]
    fn column_b_rules_exist_and_resolve_for_every_form() {
        let v85 = SpecVersion::electronic(8, 5);
        for (form, rules) in [
            (Table::F3X, F3X_RULES),
            (Table::F3, F3_RULES),
            (Table::F3P, F3P_RULES),
        ] {
            let layout = form.layout(v85).unwrap();
            let b: Vec<&LineRule> = rules.iter().filter(|r| r.column == Column::B).collect();
            assert!(b.len() >= 20, "{form}: only {} Column B rules", b.len());
            for r in b {
                assert!(
                    layout.field(r.field).is_some(),
                    "{form} col B line {}: no field {}",
                    r.line,
                    r.field
                );
                assert!(
                    r.field.starts_with("col_b_"),
                    "{form} col B line {}: {}",
                    r.line,
                    r.field
                );
            }
            for r in rules.iter().filter(|r| r.column == Column::A) {
                assert!(
                    r.field.starts_with("col_a_"),
                    "{form} col A line {}: {}",
                    r.line,
                    r.field
                );
            }
        }
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
            ("col_a_political_party_contributions", "444.44"),
            ("col_a_pac_contributions", "555.55"),
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

    // -----------------------------------------------------------------
    // Chains
    // -----------------------------------------------------------------

    /// A synthetic F3X report: coverage, cash lines, itemized individuals
    /// on the cover (Column A and B) and matching `SA11AI` lines.
    #[allow(clippy::too_many_arguments)] // a test fixture, not an API
    fn f3x_report(
        filer: &str,
        from: &str,
        through: &str,
        cash_begin: &str,
        cash_close: &str,
        itemized_a: &str,
        itemized_b: &str,
        jan_1: &str,
        schedule: &[&str],
    ) -> Filing {
        let mut filing = f3x_with(
            &[
                ("coverage_from_date", from),
                ("coverage_through_date", through),
                ("col_a_cash_on_hand_beginning_period", cash_begin),
                ("col_a_cash_on_hand_close_of_period", cash_close),
                ("col_a_individuals_itemized", itemized_a),
                ("col_b_individuals_itemized", itemized_b),
                ("col_b_cash_on_hand_jan_1", jan_1),
            ],
            schedule
                .iter()
                .map(|amt| body(Table::SchA, "SA11AI", "contribution_amount", amt, false))
                .collect(),
        );
        filing
            .summary
            .set("filer_committee_id_number", filer)
            .unwrap();
        filing
    }

    #[test]
    fn period_basis_per_form() {
        assert_eq!(
            PeriodBasis::for_form(Table::F3X),
            Some(PeriodBasis::YearToDate)
        );
        assert_eq!(
            PeriodBasis::for_form(Table::F3),
            Some(PeriodBasis::CycleToDate)
        );
        assert_eq!(
            PeriodBasis::for_form(Table::F3P),
            Some(PeriodBasis::CycleToDate)
        );
        assert_eq!(PeriodBasis::for_form(Table::F24), None);
        assert_eq!(PeriodBasis::YearToDate.to_string(), "year to date");
    }

    #[test]
    fn coverage_reads_the_cover_dates() {
        let f = f3x_report("C1", "20260101", "20260131", "0", "0", "0", "0", "0", &[]);
        let c = coverage(&f).unwrap();
        assert_eq!(c.to_string(), "2026-01-01..2026-01-31");
        // Blank, malformed, or inverted dates give None rather than a panic.
        let blank = f3x_with(&[("coverage_from_date", "20260101")], vec![]);
        assert_eq!(coverage(&blank), None);
        let inverted = f3x_report("C1", "20260201", "20260131", "0", "0", "0", "0", "0", &[]);
        assert_eq!(coverage(&inverted), None);
        let garbage = f3x_with(
            &[
                ("coverage_from_date", "Jan 1"),
                ("coverage_through_date", "20260131"),
            ],
            vec![],
        );
        assert_eq!(coverage(&garbage), None);
        assert!(
            Coverage {
                from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                through: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
            }
            .overlaps(&Coverage {
                from: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
                through: NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
            })
        );
    }

    /// Three monthly reports in one year: Column B on the third is the sum
    /// of the three schedules, and cash on hand carries forward.
    #[test]
    fn column_b_sums_the_year_and_cash_carries_forward() {
        let jan = f3x_report(
            "C1",
            "20260101",
            "20260131",
            "100.00",
            "150.00",
            "50.00",
            "50.00",
            "100.00",
            &["20.00", "30.00"],
        );
        let feb = f3x_report(
            "C1",
            "20260201",
            "20260228",
            "150.00",
            "160.00",
            "10.00",
            "60.00",
            "100.00",
            &["10.00"],
        );
        let mar = f3x_report(
            "C1",
            "20260301",
            "20260331",
            "160.00",
            "200.00",
            "40.00",
            "100.00",
            "100.00",
            &["40.00"],
        );
        // Order of the priors does not matter.
        let chain = ReportChain::new(&mar, [&feb, &jan]).unwrap();
        assert_eq!(chain.basis(), PeriodBasis::YearToDate);
        assert_eq!(chain.form(), Table::F3X);
        assert_eq!(chain.prior().len(), 2);
        assert_eq!(
            coverage(chain.prior()[0]).unwrap().from.to_string(),
            "2026-01-01"
        );
        assert_eq!(chain.period_reports().len(), 3);
        assert!(chain.gaps().is_empty(), "{:?}", chain.gaps());
        assert!(chain.last_report_of_prior_year().is_none());

        let r = chain.reconcile();
        assert_eq!(r.form, Table::F3X);
        let b = r.line(Column::B, "11(a)(i)").unwrap();
        assert_eq!(b.expected, dec!(100.00));
        assert_eq!(b.reported, Some(dec!(100.00)));
        assert_eq!(b.lines_summed, 4);
        assert_eq!(b.reports_summed, 3);
        assert_eq!(b.relation, Relation::Equal);
        assert!(
            b.rule.contains("over 3 report(s), year to date"),
            "{}",
            b.rule
        );
        assert!(b.matches(), "{b}");
        // Every other Column B schedule line is blank on the cover and has
        // no schedule lines, so it agrees at zero.
        assert!(r.column(Column::B).count() >= 25);
        // 6(b) of March = 8 of February.
        let carry = r.line(Column::A, "6(b)").unwrap();
        assert_eq!(carry.expected, dec!(160.00));
        assert_eq!(carry.reports_summed, 1);
        assert!(carry.matches(), "{carry}");
        assert!(
            carry.rule.contains("2026-02-01..2026-02-28"),
            "{}",
            carry.rule
        );
        // No prior-year report, so 6(a) is an input.
        assert!(r.line(Column::B, "6(a)").is_none());
        assert!(r.balances(), "{r}");
        assert_eq!(chain.carry_forward_checks().len(), 1);
        assert_eq!(
            chain.column_b_checks().len() + chain.carry_forward_checks().len(),
            r.checks.len()
        );
    }

    /// The first report of the year has no priors in the year: its Column B
    /// must equal its own Column A, and the chain says so.
    #[test]
    fn a_single_report_chain_compares_column_b_with_its_own_schedules() {
        let jan = f3x_report(
            "C1",
            "20260101",
            "20260131",
            "0",
            "50.00",
            "50.00",
            "55.00",
            "0",
            &["50.00"],
        );
        let chain = ReportChain::new(&jan, []).unwrap();
        let r = chain.reconcile();
        let b = r.line(Column::B, "11(a)(i)").unwrap();
        assert_eq!(b.reports_summed, 1);
        assert_eq!(b.delta, dec!(5.00));
        assert!(!b.matches());
        assert!(chain.carry_forward_checks().is_empty());
    }

    /// A December report in the chain feeds only the carry-forward on a
    /// year-to-date form: 6(a) and January's 6(b) are its close, and its
    /// schedules stay out of the new year's Column B.
    #[test]
    fn a_prior_year_report_feeds_cash_on_hand_but_not_column_b() {
        let dec_2025 = f3x_report(
            "C1",
            "20251201",
            "20251231",
            "900.00",
            "1000.00",
            "500.00",
            "5000.00",
            "0",
            &["500.00"],
        );
        let jan = f3x_report(
            "C1",
            "20260101",
            "20260131",
            "1000.00",
            "1010.00",
            "10.00",
            "10.00",
            "1000.00",
            &["10.00"],
        );
        let feb = f3x_report(
            "C1",
            "20260201",
            "20260228",
            "1010.00",
            "1030.00",
            "20.00",
            "30.00",
            "999.00",
            &["20.00"],
        );
        let chain = ReportChain::new(&feb, [&dec_2025, &jan]).unwrap();
        assert_eq!(chain.prior().len(), 2);
        assert_eq!(chain.period_reports().len(), 2, "December is not in 2026");
        assert!(chain.gaps().is_empty());
        assert!(std::ptr::eq(
            chain.last_report_of_prior_year().unwrap(),
            &dec_2025
        ));
        let r = chain.reconcile();
        let b = r.line(Column::B, "11(a)(i)").unwrap();
        assert_eq!(b.expected, dec!(30.00));
        assert_eq!(b.reports_summed, 2);
        assert!(b.matches(), "{b}");
        let six_b = r.line(Column::A, "6(b)").unwrap();
        assert_eq!(six_b.expected, dec!(1010.00));
        assert!(six_b.matches(), "{six_b}");
        // February says cash on Jan 1 was 999.00; December closed at 1000.00.
        let six_a = r.line(Column::B, "6(a)").unwrap();
        assert_eq!(six_a.expected, dec!(1000.00));
        assert_eq!(six_a.delta, dec!(-1.00));
        assert!(!six_a.matches());
        assert!(
            six_a.rule.contains("2025-12-01..2025-12-31"),
            "{}",
            six_a.rule
        );

        // January alone behind December: 6(b) and 6(a) both point at the
        // December close.
        let chain = ReportChain::new(&jan, [&dec_2025]).unwrap();
        let r = chain.reconcile();
        assert!(r.line(Column::A, "6(b)").unwrap().matches());
        assert!(r.line(Column::B, "6(a)").unwrap().matches());
        assert_eq!(r.line(Column::B, "11(a)(i)").unwrap().reports_summed, 1);
    }

    #[test]
    fn gaps_name_the_uncovered_days() {
        let mar = f3x_report("C1", "20260301", "20260331", "0", "0", "0", "0", "0", &[]);
        let jan = f3x_report("C1", "20260101", "20260115", "0", "0", "0", "0", "0", &[]);
        // No priors: January 1 through the day before March is uncovered.
        let chain = ReportChain::new(&mar, []).unwrap();
        let gaps = chain.gaps();
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].to_string(), "2026-01-01..2026-02-28");
        // A half-January report leaves the rest of January and February.
        let chain = ReportChain::new(&mar, [&jan]).unwrap();
        let gaps = chain.gaps();
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].to_string(), "2026-01-16..2026-02-28");
        // A report that starts on January 1 leaves none.
        let full_jan = f3x_report("C1", "20260101", "20260228", "0", "0", "0", "0", "0", &[]);
        assert!(
            ReportChain::new(&mar, [&full_jan])
                .unwrap()
                .gaps()
                .is_empty()
        );
    }

    #[test]
    fn overlapping_and_out_of_order_priors_are_rejected() {
        let mar = f3x_report("C1", "20260301", "20260331", "0", "0", "0", "0", "0", &[]);
        let feb = f3x_report("C1", "20260201", "20260228", "0", "0", "0", "0", "0", &[]);
        let feb_amended = f3x_report("C1", "20260201", "20260228", "0", "0", "0", "0", "0", &[]);
        // An original and its amendment overlap.
        let err = ReportChain::new(&mar, [&feb, &feb_amended]).unwrap_err();
        assert!(
            matches!(err, ChainError::Overlap { a: 0, b: 1, .. }),
            "{err:?}"
        );
        assert!(err.to_string().contains("overlap"), "{err}");
        // A report that ends on the current report's first day is not prior.
        let straddles = f3x_report("C1", "20260215", "20260301", "0", "0", "0", "0", "0", &[]);
        let err = ReportChain::new(&mar, [&straddles]).unwrap_err();
        assert!(
            matches!(err, ChainError::NotBeforeCurrent { index: 0, .. }),
            "{err:?}"
        );
        // Nor is a later report.
        let apr = f3x_report("C1", "20260401", "20260430", "0", "0", "0", "0", "0", &[]);
        assert!(matches!(
            ReportChain::new(&mar, [&apr]).unwrap_err(),
            ChainError::NotBeforeCurrent { .. }
        ));
    }

    #[test]
    fn wrong_committee_form_and_missing_dates_are_rejected() {
        let mar = f3x_report("C1", "20260301", "20260331", "0", "0", "0", "0", "0", &[]);
        let other = f3x_report("C2", "20260201", "20260228", "0", "0", "0", "0", "0", &[]);
        let err = ReportChain::new(&mar, [&other]).unwrap_err();
        assert_eq!(
            err,
            ChainError::FilerMismatch {
                index: 0,
                expected: "C1".to_string(),
                found: "C2".to_string(),
            }
        );
        assert!(err.to_string().contains("filed by C2"), "{err}");

        // Same committee id, different form.
        let mut f3 = Filing::parse("HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF3N\u{1c}C1").unwrap();
        f3.summary.set("coverage_from_date", "20260101").unwrap();
        f3.summary.set("coverage_through_date", "20260228").unwrap();
        let err = ReportChain::new(&mar, [&f3]).unwrap_err();
        assert_eq!(
            err,
            ChainError::FormMismatch {
                index: 0,
                expected: Table::F3X,
                found: Table::F3,
            }
        );
        // And the reverse: an F3 chain is cycle to date with its own cash
        // lines (23 from the prior 27).
        let mut f3_prior =
            Filing::parse("HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF3N\u{1c}C1").unwrap();
        f3_prior
            .summary
            .set("coverage_from_date", "20251001")
            .unwrap();
        f3_prior
            .summary
            .set("coverage_through_date", "20251231")
            .unwrap();
        f3_prior
            .summary
            .set("col_a_cash_on_hand_close", "77.00")
            .unwrap();
        f3.summary
            .set("col_a_cash_on_hand_beginning_period", "77.00")
            .unwrap();
        let chain = ReportChain::new(&f3, [&f3_prior]).unwrap();
        assert_eq!(chain.basis(), PeriodBasis::CycleToDate);
        // Cycle to date: the 2025 report is in the period.
        assert_eq!(chain.period_reports().len(), 2);
        assert!(chain.last_report_of_prior_year().is_none());
        assert!(chain.gaps().is_empty());
        let r = chain.reconcile();
        let carry = r.line(Column::A, "23").unwrap();
        assert_eq!(carry.expected, dec!(77.00));
        assert!(carry.matches(), "{carry}");
        assert!(
            carry.rule.starts_with("= 27 of the prior report"),
            "{}",
            carry.rule
        );
        assert!(r.line(Column::B, "6(a)").is_none());

        // Missing coverage dates.
        let undated = f3x_with(&[("filer_committee_id_number", "C1")], vec![]);
        assert_eq!(
            ReportChain::new(&undated, []).unwrap_err(),
            ChainError::CurrentCoverageMissing
        );
        assert_eq!(
            ReportChain::new(&mar, [&undated]).unwrap_err(),
            ChainError::PriorCoverageMissing { index: 0 }
        );

        // A form without rules.
        let f24 = Filing::parse("HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF24N\u{1c}C1").unwrap();
        assert_eq!(
            ReportChain::new(&f24, []).unwrap_err(),
            ChainError::UnsupportedForm(ReconcileError::UnsupportedForm(Table::F24))
        );
    }

    /// A Column B floor line stays a floor across the chain: the cover may
    /// exceed the itemized sum, never fall below it.
    #[test]
    fn column_b_floors_stay_floors() {
        let mk = |from: &str, through: &str, b_other: &str, sa17: &str| {
            let mut f = f3x_report("C1", from, through, "0", "0", "0", "0", "0", &[]);
            f.summary
                .set("col_b_other_federal_receipts", b_other)
                .unwrap();
            f.lines.push(body(
                Table::SchA,
                "SA17",
                "contribution_amount",
                sa17,
                false,
            ));
            f
        };
        let jan = mk("20260101", "20260131", "300.00", "250.00");
        let feb_over = mk("20260201", "20260228", "700.00", "300.00");
        let chain = ReportChain::new(&feb_over, [&jan]).unwrap();
        let c = chain.reconcile().line(Column::B, "17").cloned().unwrap();
        assert_eq!(c.relation, Relation::AtLeast);
        assert_eq!(c.expected, dec!(550.00));
        assert_eq!(c.delta, dec!(150.00));
        assert!(c.matches(), "{c}");
        let feb_under = mk("20260201", "20260228", "500.00", "300.00");
        let chain = ReportChain::new(&feb_under, [&jan]).unwrap();
        let c = chain.reconcile().line(Column::B, "17").cloned().unwrap();
        assert_eq!(c.violation(), dec!(50.00));
        assert!(!c.matches(), "{c}");
    }
}
