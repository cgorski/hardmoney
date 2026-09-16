//! `Filing::review`: the checks a Reports Analysis Division analyst makes
//! before sending a Request for Additional Information (RFAI).
//!
//! The FEC accepts a filing once it passes the format rules that
//! [`Filing::validate`] reproduces. Review comes after acceptance: an
//! analyst reads the report against "an internal, Commission-approved
//! review policy and its thresholds" and, where the report has an error,
//! an omission, or possible prohibited activity, sends the committee an
//! RFAI, a public letter filed as form type `FRQ` with a `request_type`
//! code (1 Statement of Organization, 2 report of receipts and
//! expenditures, 3 second notice, 5 informational, 7 failure to file, ...;
//! openFEC `docs.py`). The committee has 35 days to respond
//! (<https://www.fec.gov/help-candidates-and-committees/request-additional-information/>).
//!
//! This module implements the part of that review a file can support: the
//! deterministic checks whose inputs are all in the report (and, with a
//! [`ReportChain`], in the committee's earlier reports). It cannot see what
//! the analyst also sees: the committee's registration, its other filings,
//! bank records, prior RFAIs, or a contributor's history with other
//! committees. A [`Review`] is therefore advisory. An empty review does not
//! mean no RFAI will come, and an observation is a question to ask, not a
//! violation found. The measured agreement with the FEC's own RFAI letters
//! is in the book chapter "Reviewing a filing".
//!
//! # The concerns
//!
//! Each [`Concern`] cites the regulation or FEC instruction it rests on.
//! In brief:
//!
//! | Concern | What it asks | Basis |
//! |---|---|---|
//! | [`EmployerOccupationMissing`](Concern::EmployerOccupationMissing) | An individual whose contributions aggregate over $200 has a blank employer or occupation. | 52 U.S.C. 30104(b)(3)(A); 11 CFR 104.3(a)(4)(i), 100.12; spec Schedule A rows 24-25 "Req if Donor aggregate >$200" |
//! | [`BestEffortsClaimed`](Concern::BestEffortsClaimed) | The same, but the field says "Information Requested": the filer is claiming best efforts. Informational. | 11 CFR 104.7(b) |
//! | [`OverLimitAggregate`](Concern::OverLimitAggregate) | An individual's contributions to this committee exceed the limit for the recipient and cycle. | 52 U.S.C. 30116(a); 11 CFR 110.1(b)-(d), 110.17; [`CONTRIBUTION_LIMITS`] |
//! | [`DateOutsideCoverage`](Concern::DateOutsideCoverage) | A receipt or disbursement is dated after the report's coverage period ends. | 11 CFR 104.3(a)(4), (b)(4); Form 3X instructions (date of receipt) |
//! | [`TreasurerSignatureMissing`](Concern::TreasurerSignatureMissing) | The cover carries no treasurer name or no `date_signed`. | 11 CFR 104.14(a), 104.18(g) |
//! | [`MemoDoubleCount`](Concern::MemoDoubleCount) | A line that back-references a counted line is itself counted (an earmark or itemization reported twice). | 11 CFR 110.6(c); Form 3X instructions on memo entries |
//! | [`MemoWithoutParent`](Concern::MemoWithoutParent) | A memo entry references a transaction that is not in the file, or an individual's `SA11AI` memo gives no parent and no reason. | spec Schedule A row 4 "Reference to the Tran ID of a Related Record" |
//! | [`CoverNotSupported`](Concern::CoverNotSupported) | A cover-page line disagrees with its schedules or with the other cover lines. | FEC validation messages 3-4; [`Filing::reconcile`] |
//! | [`NegativeItemization`](Concern::NegativeItemization) | A counted Schedule A or B line is negative with no stated reason. Refunds belong on Schedule B; only reattributions, redesignations, and returned checks are negative Schedule A entries. | 11 CFR 103.3(b), 110.1(b)(5); Forms 3X/3/3P instructions for lines 28/20 |
//! | [`UnitemizedInconsistent`](Concern::UnitemizedInconsistent) | Unitemized individual receipts are negative, or large with nothing itemized (heuristic). | 11 CFR 104.3(a)(3)(i) |
//! | [`DuplicateTransactionId`](Concern::DuplicateTransactionId) | Two lines share a transaction id. | spec Schedule A row 3; FEC validation message 40 |
//! | [`DuplicateTransaction`](Concern::DuplicateTransaction) | Two counted Schedule A lines of $200 or more share contributor, date, and amount (heuristic). | Form instructions: each contribution once |
//! | [`ChainCashMismatch`](Concern::ChainCashMismatch) | Cash on hand at the beginning of the period differs from the prior report's close. Chain only. | Form 3X instructions line 6(b); [`ReportChain::carry_forward_checks`] |
//!
//! Concerns marked heuristic ([`Concern::is_heuristic`]) fire on accepted
//! filings often enough that they are reported for the analyst to weigh,
//! not as a prediction.
//!
//! # Contribution limits
//!
//! The over-limit check needs to know what kind of committee received the
//! money. A Form 3 or 3P is a candidate's committee. A Form 3X may be a
//! PAC ($5,000 a year), a state party ($10,000), a national party (four
//! accounts, $443,000 together in 2026), a joint fundraising committee, or
//! an independent-expenditure-only committee that may accept unlimited
//! amounts, and the file does not say which. [`Filing::review`] therefore
//! applies the candidate limit to Forms 3 and 3P and no limit to a Form
//! 3X; [`Filing::review_with`] takes a [`Recipient`] when the caller knows
//! (`hardmoney review --recipient pac`; the `--eval` mode reads openFEC's
//! `committee_type`). Cycles without a row in [`CONTRIBUTION_LIMITS`] are
//! not checked.
//!
//! # Example
//!
//! ```
//! use hardmoney::Filing;
//! use hardmoney::parser::review::Concern;
//!
//! let filing = Filing::open("tests/fixtures/rad/F3XA_2011912.fec")?;
//! let review = filing.review();
//! let cover: Vec<_> = review.by_concern(Concern::CoverNotSupported).collect();
//! assert_eq!(cover.len(), 1);
//! assert!(cover[0].detail.contains("11(c)"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;

use crate::parser::filing::{Filing, ParsedLine};
use crate::parser::reconcile::{Column, Coverage, LineCheck, ReportChain, coverage};
use crate::parser::schema::Typed;
use crate::parser::tables::Table;
use crate::parser::tables::markers::{SchA, SchB};
use crate::parser::tables::{sch_a, sch_b};
use crate::parser::typed::parse_money;
use crate::parser::validate::Rule;

// ---------------------------------------------------------------------------
// Concern, Observation, Review
// ---------------------------------------------------------------------------

/// One class of question a Reports Analysis Division analyst asks of a
/// report. `Display`/`FromStr` use `snake_case` names, which is also the
/// serde representation.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::EnumIter,
)]
#[strum(serialize_all = "snake_case")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum Concern {
    /// A Schedule A line from an individual (`entity_type` `IND`) whose
    /// aggregate exceeds $200 has a blank employer or occupation, and the
    /// other field does not say the person is retired or not employed.
    ///
    /// 52 U.S.C. 30104(b)(3)(A) and 11 CFR 104.3(a)(4)(i) require the
    /// "identification" (11 CFR 100.12: name, address, occupation, and
    /// employer) of each person whose contributions aggregate more than
    /// $200 in the calendar year (Form 3X) or election cycle (Forms 3 and
    /// 3P). The format specification says the same on Schedule A rows
    /// 24-25: "Req if Donor aggregate >$200". Memo entries are not checked.
    /// The aggregate used is the larger of the filer's
    /// `contribution_aggregate` and the sum of that contributor's counted
    /// lines in the file (plus prior reports in a [`ReportChain`]).
    EmployerOccupationMissing,
    /// As [`EmployerOccupationMissing`](Self::EmployerOccupationMissing),
    /// but the employer or occupation field carries the best-efforts
    /// language ("Information Requested", "Information Requested per best
    /// efforts", "Best efforts"). 11 CFR 104.7(b) lets a committee that
    /// has asked the contributor at least once report the missing
    /// information that way. Informational: it is the filer's claim of
    /// compliance, and an analyst may still ask whether the follow-up
    /// requests were made.
    BestEffortsClaimed,
    /// An individual's contributions to this committee, for one election
    /// (candidate committees) or calendar year (other committees), exceed
    /// the limit in [`CONTRIBUTION_LIMITS`] for the cycle.
    ///
    /// 52 U.S.C. 30116(a)(1); 11 CFR 110.1(b) (to candidates, per
    /// election), 110.1(c) (to other political committees, per calendar
    /// year), 110.1(d) (to national party committees), indexed under 11
    /// CFR 110.17. Per-election sums add redesignation and reattribution
    /// memo entries (11 CFR 110.1(b)(5), 110.1(k)), which is how an
    /// excessive primary contribution is lawfully moved to the general.
    /// The amount on the observation is the excess over the limit.
    OverLimitAggregate,
    /// A counted Schedule A or B line is dated after
    /// `coverage_through_date`: a receipt or disbursement that had not
    /// happened when the period closed.
    ///
    /// 11 CFR 104.3(a)(4) and (b)(4) call for the date of receipt and the
    /// date of disbursement; the Form 3X instructions define the date of a
    /// receipt as the date the committee received it, not the date on the
    /// check. Memo entries (a joint fundraising memo carries the date the
    /// joint committee received the money), negative entries, and lines
    /// whose text says refund, reattribution, redesignation, or returned
    /// check are not checked. Dates *before* the period are not flagged:
    /// on the 2024 evaluation sample they were common on reports that drew
    /// no letter (a conduit's batch dated the last days of the prior
    /// month, a new committee's first report reaching back before its
    /// registration) and carried no signal; see the book chapter.
    DateOutsideCoverage,
    /// The cover carries no treasurer name or no `date_signed`.
    ///
    /// 11 CFR 104.14(a): each report "shall be signed by the treasurer";
    /// for electronic filings the treasurer's password is the signature
    /// (11 CFR 104.18(g)) and `date_signed` is the date of it. The FEC's
    /// validator rejects a blank `date_signed` outright (the workbook marks
    /// it an error), so this fires mostly on older or hand-built files.
    TreasurerSignatureMissing,
    /// A counted (non-memo) Schedule A line with a positive amount
    /// back-references another counted Schedule A line. The child of a
    /// counted line -- an earmarked contribution under its conduit's
    /// total, a joint fundraising participant's share under the transfer
    /// -- must be a memo entry (11 CFR 110.6(c)(1); Form 3X instructions:
    /// "memo entries are not included in the totals"). Both counted, the
    /// money is in line 11(a)(i) twice. A counted negative child is a
    /// reversal of its parent and is not flagged here.
    MemoDoubleCount,
    /// A memo entry references a transaction id that is not in the file
    /// (the FEC's validation message 41, reported here only for memos), or
    /// an individual's `SA11AI` memo entry has no back-reference and its
    /// text gives no reason (no joint fundraising, earmark, conduit,
    /// reattribution, redesignation, in-kind, partnership, or best-efforts
    /// language). Heuristic: many accepted filings carry conduit totals
    /// and joint fundraising memos with the parent named in the text
    /// rather than the back-reference field.
    MemoWithoutParent,
    /// A cover-page line fails its rule in [`Filing::reconcile`]: the
    /// "Subtotal ... not supported by Schedule" class of RFAI (FEC
    /// validation messages 3-4, which only warn). One observation per
    /// failing [`LineCheck`], the amount being the violation. From a
    /// [`ReportChain`], Column B sums over the period's reports are
    /// checked too.
    CoverNotSupported,
    /// A counted Schedule A or B line has a negative amount and its text
    /// gives no reason. The form instructions put refunds of contributions
    /// received on Schedule B (Form 3X line 28, Form 3 line 20, Form 3P
    /// line 28) and refunds made to the committee on Schedule A line
    /// 15/14/20; negative Schedule A entries are for reattributions and
    /// redesignations (11 CFR 110.1(b)(5), (k)) and for a contribution
    /// returned unpaid after it was reported (11 CFR 103.3(b)). Lines
    /// whose purpose or memo text says so are not flagged.
    NegativeItemization,
    /// Unitemized individual contributions (Form 3X/3 line 11(a)(ii), Form
    /// 3P line 17(a)(ii)) are negative, or reach
    /// [`UNITEMIZED_WITHOUT_ITEMIZED_FLOOR`] while the report itemizes no
    /// individual contribution at all. 11 CFR 104.3(a)(3)(i) defines the
    /// line as the contributions too small to itemize; a committee that
    /// raised that much from individuals with no one over $200 in
    /// aggregate is unusual. Heuristic.
    UnitemizedInconsistent,
    /// Two lines share a transaction id (spec Schedule A row 3: "must be
    /// unique ... for the life of the report"; FEC validation message 40).
    /// Delegated to [`Filing::validate`].
    DuplicateTransactionId,
    /// Two counted Schedule A lines of $200 or more share the contributor
    /// (name and ZIP), the date, and the amount, or (in a [`ReportChain`])
    /// a counted line repeats one already reported in a prior report.
    /// Heuristic: a contributor who gives the same amount twice on one
    /// day is legitimate and common on large small-dollar reports.
    DuplicateTransaction,
    /// Cash on hand at the beginning of the period (Form 3X line 6(b),
    /// Form 3 line 23, Form 3P line 6) differs from the prior report's
    /// close, or Form 3X line 6(a) differs from the prior year's last
    /// report. The Form 3X instructions: line 6(b) "must equal ... line 8
    /// of the previous report". Only a [`ReportChain`] can raise it.
    ChainCashMismatch,
}

impl Concern {
    /// The openFEC `request_type` code of the RFAI letter this concern
    /// most resembles: `2` ("Report of Receipts and Expenditures (Form 3
    /// and 3X)") for every concern about a report, `None` for
    /// [`BestEffortsClaimed`](Self::BestEffortsClaimed), which is not a
    /// request. The codes are documented in openFEC's `docs.py`
    /// (`REQUEST_TYPE`); they classify the document reviewed, not the
    /// problem found, so this is as specific as the FEC's data allows.
    #[must_use]
    pub fn rfai_request_type(self) -> Option<u8> {
        match self {
            Concern::BestEffortsClaimed => None,
            _ => Some(2),
        }
    }

    /// True for the concerns that fire on accepted filings often enough
    /// to be reported for weighing rather than as a prediction:
    /// [`MemoWithoutParent`](Self::MemoWithoutParent),
    /// [`UnitemizedInconsistent`](Self::UnitemizedInconsistent),
    /// [`DuplicateTransaction`](Self::DuplicateTransaction), and
    /// [`BestEffortsClaimed`](Self::BestEffortsClaimed) (informational).
    #[must_use]
    pub fn is_heuristic(self) -> bool {
        matches!(
            self,
            Concern::MemoWithoutParent
                | Concern::UnitemizedInconsistent
                | Concern::DuplicateTransaction
                | Concern::BestEffortsClaimed
        )
    }

    /// One sentence saying what the concern asks, for help text and the
    /// book.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Concern::EmployerOccupationMissing => {
                "individual over $200 in aggregate with a blank employer or occupation"
            }
            Concern::BestEffortsClaimed => {
                "individual over $200 in aggregate whose employer/occupation says information requested"
            }
            Concern::OverLimitAggregate => {
                "individual's contributions exceed the limit for the recipient and cycle"
            }
            Concern::DateOutsideCoverage => {
                "receipt or disbursement dated after the coverage period ends"
            }
            Concern::TreasurerSignatureMissing => {
                "treasurer name or date signed blank on the cover"
            }
            Concern::MemoDoubleCount => {
                "counted line back-references another counted line (earmark or itemization counted twice)"
            }
            Concern::MemoWithoutParent => "memo entry whose parent transaction cannot be found",
            Concern::CoverNotSupported => {
                "cover-page line not supported by the schedules or formulas"
            }
            Concern::NegativeItemization => "negative counted amount with no stated reason",
            Concern::UnitemizedInconsistent => {
                "unitemized individual receipts negative, or large with nothing itemized"
            }
            Concern::DuplicateTransactionId => "transaction id used more than once",
            Concern::DuplicateTransaction => {
                "same contributor, date, and amount reported twice ($200 and over)"
            }
            Concern::ChainCashMismatch => {
                "cash on hand does not carry forward from the prior report"
            }
        }
    }
}

/// One thing an analyst would ask about.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Observation {
    pub concern: Concern,
    /// 1-based physical line in the `.fec` file (2 = cover); `None` for an
    /// observation about the report as a whole.
    pub line_no: Option<u64>,
    /// The line's `transaction_id`, when it has one.
    pub transaction_id: Option<String>,
    /// The dollar amount at issue: the contribution, the excess over a
    /// limit, the violation of a cover rule. `None` when no single amount
    /// applies.
    pub amount: Option<Decimal>,
    /// A complete sentence a treasurer or analyst could act on.
    pub detail: String,
    /// [`Concern::rfai_request_type`] of `concern`.
    pub rfai_request_type: Option<u8>,
}

impl Observation {
    /// Builds an observation; `transaction_id` and `line_no` come from
    /// `line` when one is given.
    #[must_use]
    pub fn new(
        concern: Concern,
        line: Option<&ParsedLine>,
        amount: Option<Decimal>,
        detail: impl Into<String>,
    ) -> Self {
        Observation {
            concern,
            line_no: line.map(|l| l.line_no),
            transaction_id: line
                .and_then(|l| l.get_non_empty("transaction_id"))
                .map(str::to_string),
            amount,
            detail: detail.into(),
            rfai_request_type: concern.rfai_request_type(),
        }
    }
}

impl fmt::Display for Observation {
    /// `employer_occupation_missing line 45 SA11AI.123 250.00: <detail>`
    /// -- one line, like a [`Finding`](crate::parser::Finding).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.concern)?;
        match self.line_no {
            Some(n) => write!(f, " line {n}")?,
            None => write!(f, " report")?,
        }
        if let Some(id) = &self.transaction_id {
            write!(f, " {id}")?;
        }
        if let Some(amount) = self.amount {
            write!(f, " {amount}")?;
        }
        write!(f, ": {}", self.detail)
    }
}

/// Count and total amount at issue for one concern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ConcernTotal {
    pub count: usize,
    /// Sum of the observations' `amount`s (absolute values); zero when
    /// none carries one.
    pub amount: Decimal,
}

/// Per-concern counts and the total amount at issue.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ReviewSummary {
    pub observations: usize,
    /// Sum over every observation's amount (absolute values).
    pub amount_at_issue: Decimal,
    pub by_concern: BTreeMap<Concern, ConcernTotal>,
}

impl ReviewSummary {
    fn of(observations: &[Observation]) -> Self {
        let mut s = ReviewSummary {
            observations: observations.len(),
            ..ReviewSummary::default()
        };
        for o in observations {
            let entry = s.by_concern.entry(o.concern).or_default();
            entry.count = entry.count.saturating_add(1);
            if let Some(a) = o.amount {
                entry.amount = entry.amount.saturating_add(a.abs());
                s.amount_at_issue = s.amount_at_issue.saturating_add(a.abs());
            }
        }
        s
    }
}

/// The result of [`Filing::review`] or [`ReportChain::review`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[must_use]
#[non_exhaustive]
pub struct Review {
    /// Every observation, in line order (report-level ones last).
    pub observations: Vec<Observation>,
    pub summary: ReviewSummary,
}

impl Review {
    fn from_observations(mut observations: Vec<Observation>) -> Self {
        observations.sort_by_key(|o| (o.line_no.is_none(), o.line_no, o.concern));
        let summary = ReviewSummary::of(&observations);
        Review {
            observations,
            summary,
        }
    }

    /// True when nothing was observed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.observations.is_empty()
    }

    /// Number of observations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.observations.len()
    }

    /// Every observation, in line order.
    pub fn iter(&self) -> std::slice::Iter<'_, Observation> {
        self.observations.iter()
    }

    /// The observations for one concern.
    pub fn by_concern(&self, concern: Concern) -> impl Iterator<Item = &Observation> + '_ {
        self.observations
            .iter()
            .filter(move |o| o.concern == concern)
    }

    /// True when at least one observation has `concern`.
    #[must_use]
    pub fn has(&self, concern: Concern) -> bool {
        self.by_concern(concern).next().is_some()
    }

    /// The concerns observed, in enum order.
    #[must_use]
    pub fn concerns(&self) -> Vec<Concern> {
        self.summary.by_concern.keys().copied().collect()
    }

    /// The observations whose concern is not
    /// [heuristic](Concern::is_heuristic).
    pub fn strict(&self) -> impl Iterator<Item = &Observation> + '_ {
        self.observations
            .iter()
            .filter(|o| !o.concern.is_heuristic())
    }
}

impl fmt::Display for Review {
    /// One observation per line, then a summary line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for o in &self.observations {
            writeln!(f, "{o}")?;
        }
        if self.observations.is_empty() {
            write!(f, "no observations")
        } else {
            write!(
                f,
                "{} observation(s) in {} concern(s); amount at issue {}",
                self.summary.observations,
                self.summary.by_concern.len(),
                self.summary.amount_at_issue
            )
        }
    }
}

impl IntoIterator for Review {
    type Item = Observation;
    type IntoIter = std::vec::IntoIter<Observation>;
    fn into_iter(self) -> Self::IntoIter {
        self.observations.into_iter()
    }
}

impl<'a> IntoIterator for &'a Review {
    type Item = &'a Observation;
    type IntoIter = std::slice::Iter<'a, Observation>;
    fn into_iter(self) -> Self::IntoIter {
        self.observations.iter()
    }
}

// ---------------------------------------------------------------------------
// Contribution limits
// ---------------------------------------------------------------------------

/// What kind of committee received the contributions, which decides the
/// limit an individual's contributions are checked against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display, strum::EnumString)]
#[strum(serialize_all = "kebab-case")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
#[non_exhaustive]
pub enum Recipient {
    /// A candidate's authorized committee (Forms 3 and 3P): the limit is
    /// per election (11 CFR 110.1(b)).
    Candidate,
    /// A PAC or other political committee that is not a party committee
    /// and not independent-expenditure-only: $5,000 per calendar year (11
    /// CFR 110.1(d)).
    Pac,
    /// A national party committee (11 CFR 110.1(c)(2)). Its Form 3X mixes
    /// receipts to the main account and to the three additional accounts
    /// of 52 U.S.C. 30116(a)(9) (headquarters, convention, recounts and
    /// legal), each with its own limit, so the ceiling checked is the sum
    /// of the four.
    NationalParty,
    /// A state, district, or local party committee (11 CFR 110.1(c)(5)):
    /// $10,000 per calendar year, combined.
    StateParty,
    /// An independent-expenditure-only or other committee that may accept
    /// unlimited contributions: no check.
    Unlimited,
}

impl Recipient {
    /// The recipient a form implies when nothing else is known: Forms 3
    /// and 3P are candidate committees. A Form 3X could be a PAC, a party,
    /// a joint fundraising committee, or an unlimited committee, so `None`:
    /// no limit is checked until the caller says which.
    #[must_use]
    pub fn implied_by_form(form: Table) -> Option<Recipient> {
        match form {
            Table::F3 | Table::F3P => Some(Recipient::Candidate),
            _ => None,
        }
    }

    /// The recipient for an openFEC `committee_type` code: `H`, `S`, `P`
    /// (House, Senate, presidential) are candidates; `N`, `Q` (PAC
    /// nonqualified/qualified) are PACs; `X`, `Y` (party
    /// nonqualified/qualified) are treated as state party committees,
    /// which is what all but a dozen of them are (the national committees
    /// must be stated as [`NationalParty`](Self::NationalParty) by the
    /// caller); `O` (independent expenditure-only), `V`, `W` (hybrid PACs,
    /// whose limited account the file cannot separate), and `Z` (national
    /// party nonfederal account) are unlimited. `None` for codes whose
    /// committees do not file Forms 3, 3P, or 3X (`C`, `E`, `I`, `U`,
    /// `D`) and for unknown codes.
    #[must_use]
    pub fn from_openfec_committee_type(code: &str) -> Option<Recipient> {
        match code.trim().to_ascii_uppercase().as_str() {
            "H" | "S" | "P" => Some(Recipient::Candidate),
            "N" | "Q" => Some(Recipient::Pac),
            "X" | "Y" => Some(Recipient::StateParty),
            "O" | "V" | "W" | "Z" => Some(Recipient::Unlimited),
            _ => None,
        }
    }

    /// `"per election"` for a candidate, `"per calendar year"` otherwise.
    #[must_use]
    pub fn basis(self) -> &'static str {
        match self {
            Recipient::Candidate => "per election",
            _ => "per calendar year",
        }
    }
}

/// The individual contribution limits for one two-year cycle, in whole
/// dollars.
///
/// Source: the FEC's contribution limits page
/// (<https://www.fec.gov/help-candidates-and-committees/candidate-taking-receipts/contribution-limits/>)
/// and its archived tables for 2019-2020, 2021-2022, and 2023-2024. The
/// per-election and national-party limits are indexed for inflation every
/// odd year under 11 CFR 110.17; the PAC and state-party limits are fixed
/// by statute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ContributionLimits {
    /// The even year of the cycle (2020 covers 2019-2020).
    pub cycle: u16,
    /// Individual to a candidate committee, per election.
    pub candidate_per_election: u32,
    /// Individual to a PAC (other political committee), per calendar year.
    pub pac_per_year: u32,
    /// Individual to a national party committee's main account, per
    /// calendar year.
    pub national_party_per_year: u32,
    /// Individual to each of a national party committee's three
    /// additional accounts (headquarters, convention, recounts and legal
    /// proceedings; 52 U.S.C. 30116(a)(9)), per account per calendar year.
    pub national_party_additional_account_per_year: u32,
    /// Individual to a state, district, or local party committee, per
    /// calendar year (combined).
    pub state_party_per_year: u32,
}

impl ContributionLimits {
    /// The limit for `recipient` as a [`Decimal`]; `None` for
    /// [`Recipient::Unlimited`]. For [`Recipient::NationalParty`] it is
    /// the four accounts together (main plus three additional), because a
    /// Form 3X does not say which account a receipt went to.
    #[must_use]
    pub fn for_recipient(&self, recipient: Recipient) -> Option<Decimal> {
        let dollars = match recipient {
            Recipient::Candidate => self.candidate_per_election,
            Recipient::Pac => self.pac_per_year,
            Recipient::NationalParty => self.national_party_per_year.saturating_add(
                self.national_party_additional_account_per_year
                    .saturating_mul(3),
            ),
            Recipient::StateParty => self.state_party_per_year,
            Recipient::Unlimited => return None,
        };
        Some(Decimal::from(dollars))
    }
}

/// Individual contribution limits by cycle, 2019-2020 through 2025-2026.
/// See [`ContributionLimits`] for the source.
pub static CONTRIBUTION_LIMITS: &[ContributionLimits] = &[
    ContributionLimits {
        cycle: 2020,
        candidate_per_election: 2_800,
        pac_per_year: 5_000,
        national_party_per_year: 35_500,
        national_party_additional_account_per_year: 106_500,
        state_party_per_year: 10_000,
    },
    ContributionLimits {
        cycle: 2022,
        candidate_per_election: 2_900,
        pac_per_year: 5_000,
        national_party_per_year: 36_500,
        national_party_additional_account_per_year: 109_500,
        state_party_per_year: 10_000,
    },
    ContributionLimits {
        cycle: 2024,
        candidate_per_election: 3_300,
        pac_per_year: 5_000,
        national_party_per_year: 41_300,
        national_party_additional_account_per_year: 123_900,
        state_party_per_year: 10_000,
    },
    ContributionLimits {
        cycle: 2026,
        candidate_per_election: 3_500,
        pac_per_year: 5_000,
        national_party_per_year: 44_300,
        national_party_additional_account_per_year: 132_900,
        state_party_per_year: 10_000,
    },
];

/// The limits for the cycle containing `year` (an odd year belongs to the
/// following even year's cycle); `None` for a cycle not in
/// [`CONTRIBUTION_LIMITS`].
#[must_use]
pub fn contribution_limits(year: i32) -> Option<&'static ContributionLimits> {
    let cycle = if year % 2 == 0 {
        year
    } else {
        year.checked_add(1)?
    };
    CONTRIBUTION_LIMITS
        .iter()
        .find(|l| i32::from(l.cycle) == cycle)
}

/// The itemization threshold: contributions aggregating more than this
/// need the contributor's employer and occupation (11 CFR 104.3(a)(4)).
pub const ITEMIZATION_THRESHOLD: u32 = 200;

/// [`Concern::UnitemizedInconsistent`] fires when unitemized individual
/// receipts reach this many dollars and no individual contribution is
/// itemized. A heuristic threshold; see the concern's docs.
pub const UNITEMIZED_WITHOUT_ITEMIZED_FLOOR: u32 = 25_000;

// ---------------------------------------------------------------------------
// Options and entry points
// ---------------------------------------------------------------------------

/// What the reviewer knows that the file does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ReviewOptions {
    /// The committee's kind, for the contribution limit; `None` means
    /// [`Recipient::implied_by_form`].
    pub recipient: Option<Recipient>,
}

impl ReviewOptions {
    /// Defaults: everything inferred from the file.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// States the recipient kind.
    #[must_use]
    pub fn recipient(mut self, recipient: Recipient) -> Self {
        self.recipient = Some(recipient);
        self
    }
}

impl Filing {
    /// Runs every check in the [module docs](self) that one file supports,
    /// with the contribution limit implied by the form
    /// ([`Recipient::implied_by_form`]). Never fails: a filing that parsed
    /// can always be reviewed. A form without cover rules (a Form 24, a
    /// Form 5) gets the schedule-level checks only.
    pub fn review(&self) -> Review {
        self.review_with(&ReviewOptions::default())
    }

    /// [`review`](Self::review) with the caller's knowledge of the
    /// committee.
    pub fn review_with(&self, options: &ReviewOptions) -> Review {
        Reviewer::new(self, options).run()
    }
}

impl ReportChain<'_> {
    /// [`Filing::review`] of the current report, plus what the chain
    /// supports: [`ChainCashMismatch`](Concern::ChainCashMismatch) from the
    /// carry-forward checks, [`CoverNotSupported`](Concern::CoverNotSupported)
    /// for Column B lines against the period's schedule sums, and the
    /// employer/occupation, over-limit, and duplicate checks with the
    /// prior reports' itemizations behind them (a contributor whose
    /// aggregate crosses $200 in this report because of earlier ones; a
    /// contribution repeated from an earlier report).
    pub fn review(&self) -> Review {
        self.review_with(&ReviewOptions::default())
    }

    /// [`review`](Self::review) with the caller's knowledge of the
    /// committee.
    pub fn review_with(&self, options: &ReviewOptions) -> Review {
        let mut reviewer = Reviewer::new(self.current(), options);
        let prior: Vec<&Filing> = self
            .period_reports()
            .into_iter()
            .filter(|f| !std::ptr::eq(*f, self.current()))
            .collect();
        reviewer.with_prior(&prior);
        let mut review = reviewer.run();
        let mut extra = Vec::new();
        for check in self.carry_forward_checks() {
            if !check.matches() {
                extra.push(cover_observation(
                    Concern::ChainCashMismatch,
                    self.current(),
                    &check,
                ));
            }
        }
        for check in self.column_b_checks() {
            if !check.matches() {
                extra.push(cover_observation(
                    Concern::CoverNotSupported,
                    self.current(),
                    &check,
                ));
            }
        }
        if !extra.is_empty() {
            review.observations.extend(extra);
            review = Review::from_observations(review.observations);
        }
        review
    }
}

fn cover_observation(concern: Concern, filing: &Filing, check: &LineCheck) -> Observation {
    let reported = check
        .reported
        .map_or_else(|| "blank".to_string(), |d| d.to_string());
    let over = match check.column {
        Column::A => String::new(),
        Column::B => format!(" over {} report(s)", check.reports_summed),
    };
    Observation::new(
        concern,
        Some(&filing.summary),
        Some(check.violation()),
        format!(
            "column {} line {} reports {reported} but the rule {} gives {}{over}; delta {}",
            check.column, check.line, check.rule, check.expected, check.delta
        ),
    )
}

// ---------------------------------------------------------------------------
// The reviewer
// ---------------------------------------------------------------------------

/// Words in a purpose or memo text that explain a negative or
/// out-of-period entry (reattribution, redesignation, refund, returned
/// check, correction).
const ADJUSTMENT_WORDS: &[&str] = &[
    "REATTRIBUT",
    "REDESIGNAT",
    "REFUND",
    "RETURN",
    "NSF",
    "VOID",
    "CHARGEBACK",
    "CHARGE BACK",
    "REVERS",
    "CANCEL",
    "BOUNCE",
    "DISPUT",
    "CORRECT",
    "OFFSET",
    "ADJUST",
    "INSUFFICIENT",
    "STOP PAYMENT",
    "REISSUE",
];

/// Words that mark a redesignation or reattribution memo, whose signed
/// amount moves money between elections or contributors.
const REDESIGNATION_WORDS: &[&str] = &["REDESIGNAT", "REATTRIBUT"];

/// The best-efforts language of 11 CFR 104.7(b).
const BEST_EFFORTS_WORDS: &[&str] = &["REQUESTED", "BEST EFFORT"];

/// Employer/occupation values (letters and digits only, upper case) that
/// say the person has no employer; the other field may then be blank.
const NOT_EMPLOYED: &[&str] = &[
    "RETIRED",
    "NOTEMPLOYED",
    "UNEMPLOYED",
    "HOMEMAKER",
    "STUDENT",
    "NONE",
    "NA",
];

/// Words in a memo entry's text that name its parent or its reason, so a
/// blank back-reference is not a concern.
const MEMO_REASON_WORDS: &[&str] = &[
    "JFC",
    "JOINT",
    "TRANSFER",
    "EARMARK",
    "CONDUIT",
    "ATTRIBUT",
    "REDESIGNAT",
    "IN-KIND",
    "INKIND",
    "IN KIND",
    "PARTNERSHIP",
    "LLC",
    "BEST EFFORT",
    "REFUND",
    "PREVIOUSLY",
    "ORIGINAL",
    "VIA",
    "THROUGH",
    "FROM",
    "SEE",
];

fn dollars(n: u32) -> Decimal {
    Decimal::from(n)
}

/// Letters and digits only, upper-cased: the key form of a name or ZIP.
fn squash(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn contains_any(text: &str, words: &[&str]) -> bool {
    words.iter().any(|w| text.contains(w))
}

/// The cycle an election code (`P2026`, `G2024`) belongs to, from its
/// trailing four digits.
fn election_year(code: &str) -> Option<i32> {
    let digits: String = code.chars().filter(char::is_ascii_digit).collect();
    if digits.len() == 4 {
        digits.parse().ok()
    } else {
        None
    }
}

/// One Schedule A line, with the fields the checks read parsed once.
struct Receipt<'a> {
    line: &'a ParsedLine,
    is_individual: bool,
    is_memo: bool,
    amount: Option<Decimal>,
    aggregate: Option<Decimal>,
    date: Option<NaiveDate>,
    /// Upper-cased election code, blank if none.
    election: String,
    /// Contributor key: squashed name plus ZIP5. Blank when the line
    /// names no one.
    key: String,
    /// The name as filed, for messages.
    name: String,
    employer: &'a str,
    occupation: &'a str,
    /// Upper-cased purpose and memo text.
    text: String,
    back_reference: Option<&'a str>,
}

impl<'a> Receipt<'a> {
    fn from_line(line: &'a ParsedLine) -> Option<Self> {
        let t: Typed<'a, SchA> = line.typed::<SchA>().ok()?;
        let last = t.get(sch_a::CONTRIBUTOR_LAST_NAME).unwrap_or("");
        let first = t.get(sch_a::CONTRIBUTOR_FIRST_NAME).unwrap_or("");
        let single = t.get(sch_a::CONTRIBUTOR_NAME).unwrap_or("");
        let org = t.get(sch_a::CONTRIBUTOR_ORGANIZATION_NAME).unwrap_or("");
        let (name, name_key) = if !last.is_empty() || !first.is_empty() {
            (
                format!("{last}, {first}")
                    .trim_matches([',', ' '])
                    .to_string(),
                format!("{}|{}", squash(last), squash(first)),
            )
        } else if !single.is_empty() {
            (single.to_string(), squash(single))
        } else {
            (org.to_string(), squash(org))
        };
        let zip: String = t
            .get(sch_a::CONTRIBUTOR_ZIP_CODE)
            .unwrap_or("")
            .chars()
            .filter(char::is_ascii_digit)
            .take(5)
            .collect();
        let key = if name_key.trim_matches('|').is_empty() {
            String::new()
        } else {
            format!("{name_key}|{zip}")
        };
        let mut text = t
            .get(sch_a::CONTRIBUTION_PURPOSE_DESCRIP)
            .unwrap_or("")
            .to_ascii_uppercase();
        if let Some(memo) = t.get(sch_a::MEMO_TEXT) {
            text.push(' ');
            text.push_str(&memo.to_ascii_uppercase());
        }
        Some(Receipt {
            line,
            is_individual: t
                .get(sch_a::ENTITY_TYPE)
                .is_some_and(|e| e.eq_ignore_ascii_case("IND")),
            is_memo: line.is_memo(),
            amount: t.money(sch_a::CONTRIBUTION_AMOUNT),
            aggregate: t.money(sch_a::CONTRIBUTION_AGGREGATE),
            date: t.date(sch_a::CONTRIBUTION_DATE),
            election: t
                .get(sch_a::ELECTION_CODE)
                .unwrap_or("")
                .to_ascii_uppercase(),
            key,
            name,
            employer: t.get(sch_a::CONTRIBUTOR_EMPLOYER).unwrap_or(""),
            occupation: t.get(sch_a::CONTRIBUTOR_OCCUPATION).unwrap_or(""),
            text,
            back_reference: t.get(sch_a::BACK_REFERENCE_TRAN_ID),
        })
    }

    /// Counted toward the totals: not a memo, with a parseable amount.
    fn counted(&self) -> Option<Decimal> {
        if self.is_memo { None } else { self.amount }
    }

    fn is_redesignation_memo(&self) -> bool {
        self.is_memo && contains_any(&self.text, REDESIGNATION_WORDS)
    }
}

/// Per-contributor and per-election running totals from prior reports.
#[derive(Default)]
struct PriorTotals {
    reports: usize,
    /// Contributor key -> sum of counted individual contributions.
    by_contributor: HashMap<String, Decimal>,
    /// (contributor key, election) -> sum including redesignation memos.
    by_election: HashMap<(String, String), Decimal>,
    /// (token, key, date, amount) of every counted Schedule A line, with
    /// the coverage of the report it was in.
    transactions: HashMap<(String, String, NaiveDate, Decimal), Coverage>,
}

struct Reviewer<'a> {
    filing: &'a Filing,
    recipient: Option<Recipient>,
    coverage: Option<Coverage>,
    prior: PriorTotals,
    out: Vec<Observation>,
}

impl<'a> Reviewer<'a> {
    fn new(filing: &'a Filing, options: &ReviewOptions) -> Self {
        let form = filing.summary.table();
        Reviewer {
            filing,
            recipient: options
                .recipient
                .or_else(|| Recipient::implied_by_form(form)),
            coverage: coverage(filing),
            prior: PriorTotals::default(),
            out: Vec::new(),
        }
    }

    /// Accumulates the prior reports' counted Schedule A lines.
    fn with_prior(&mut self, prior: &[&Filing]) {
        for report in prior {
            let Some(cov) = coverage(report) else {
                continue;
            };
            self.prior.reports = self.prior.reports.saturating_add(1);
            for line in report.lines_for(Table::SchA) {
                let Some(r) = Receipt::from_line(line) else {
                    continue;
                };
                if r.key.is_empty() {
                    continue;
                }
                if let Some(amount) = r.counted() {
                    if r.is_individual {
                        let e = self.prior.by_contributor.entry(r.key.clone()).or_default();
                        *e = e.saturating_add(amount);
                    }
                    if let Some(date) = r.date {
                        self.prior.transactions.insert(
                            (line.raw_form_type.clone(), r.key.clone(), date, amount),
                            cov,
                        );
                    }
                }
                if r.is_individual
                    && let Some(amount) = r.amount
                    && (!r.is_memo || r.is_redesignation_memo())
                {
                    let e = self
                        .prior
                        .by_election
                        .entry((r.key.clone(), r.election.clone()))
                        .or_default();
                    *e = e.saturating_add(amount);
                }
            }
        }
    }

    fn run(mut self) -> Review {
        self.check_cover_signature();
        self.check_schedule_a();
        self.check_schedule_b();
        self.check_cover_rules();
        self.check_unitemized();
        self.check_validation();
        Review::from_observations(self.out)
    }

    fn push(&mut self, o: Observation) {
        self.out.push(o);
    }

    // -- (4) treasurer signature ------------------------------------------

    fn check_cover_signature(&mut self) {
        let filing = self.filing;
        let cover = &filing.summary;
        // Only forms with a treasurer block; `get` is `None` when the layout
        // has no such field.
        let Some(date_signed) = cover.get("date_signed") else {
            return;
        };
        if date_signed.trim().is_empty() {
            self.push(Observation::new(
                Concern::TreasurerSignatureMissing,
                Some(cover),
                None,
                "the cover page has no date signed; 11 CFR 104.14(a) requires the treasurer's \
                 signature and its date on every report",
            ));
        }
        let named = [
            "treasurer_last_name",
            "treasurer_first_name",
            "treasurer_name",
        ]
        .iter()
        .any(|f| cover.get_non_empty(f).is_some());
        let has_field = ["treasurer_last_name", "treasurer_name"]
            .iter()
            .any(|f| cover.get(f).is_some());
        if has_field && !named {
            self.push(Observation::new(
                Concern::TreasurerSignatureMissing,
                Some(cover),
                None,
                "the cover page names no treasurer; 11 CFR 104.14(a) requires the report to be \
                 signed by the treasurer",
            ));
        }
    }

    // -- Schedule A: (1) (2) (3) (5) (7) (9) --------------------------------

    fn check_schedule_a(&mut self) {
        let filing = self.filing;
        let receipts: Vec<Receipt<'a>> = filing
            .lines_for(Table::SchA)
            .filter_map(Receipt::from_line)
            .collect();
        let threshold = dollars(ITEMIZATION_THRESHOLD);

        // Transaction id (upper-cased) -> (is_memo, amount) for the
        // double-count check.
        let by_id: HashMap<String, (bool, Option<Decimal>, u64)> = receipts
            .iter()
            .filter_map(|r| {
                r.line.get_non_empty("transaction_id").map(|id| {
                    (
                        id.to_ascii_uppercase(),
                        (r.is_memo, r.amount, r.line.line_no),
                    )
                })
            })
            .collect();

        // Running in-file totals per contributor (counted individual
        // lines), seeded from prior reports.
        let mut running: HashMap<&str, Decimal> = HashMap::new();
        // (key, election) -> (sum, name, last line).
        let mut by_election: HashMap<(&str, &str), (Decimal, &str, &'a ParsedLine)> =
            HashMap::new();
        // key -> (sum, max reported aggregate, name, last line) for the
        // per-year limits.
        let mut by_contributor: HashMap<&str, (Decimal, Decimal, &str, &'a ParsedLine)> =
            HashMap::new();
        // (token, key, date, amount) -> first line, for in-file duplicates.
        let mut seen: HashMap<(&str, &str, NaiveDate, Decimal), &'a ParsedLine> = HashMap::new();

        for r in &receipts {
            let line = r.line;
            let counted = r.counted();

            // (7) negative counted amounts without a reason.
            if let Some(amount) = counted
                && amount.is_sign_negative()
                && !contains_any(&r.text, ADJUSTMENT_WORDS)
            {
                self.push(Observation::new(
                    Concern::NegativeItemization,
                    Some(line),
                    Some(amount),
                    format!(
                        "{} carries a negative counted amount {amount} from {} with no reason \
                         in its text; a refund of a contribution belongs on Schedule B, and a \
                         reattribution, redesignation, or returned check should say so",
                        line.raw_form_type,
                        if r.name.is_empty() {
                            "(unnamed)"
                        } else {
                            &r.name
                        }
                    ),
                ));
            }

            // (3) dates after the period: counted, positive, unexplained.
            if let (Some(cov), Some(date), Some(amount)) = (self.coverage, r.date, counted)
                && amount.is_sign_positive()
                && !amount.is_zero()
                && !contains_any(&r.text, ADJUSTMENT_WORDS)
                && date > cov.through
            {
                self.push(Observation::new(
                    Concern::DateOutsideCoverage,
                    Some(line),
                    Some(amount),
                    format!(
                        "{} from {} is dated {date}, after the coverage period {cov} closed; \
                         the date of a receipt is the date the committee received it, and a \
                         receipt after the period belongs on the next report",
                        line.raw_form_type,
                        if r.name.is_empty() {
                            "(unnamed)"
                        } else {
                            &r.name
                        }
                    ),
                ));
            }

            // (5) a counted positive line under a counted parent. A counted
            // *negative* child is a reversal of its parent (a chargeback
            // that nets the total), not a second count.
            if let (false, Some(back)) = (r.is_memo, r.back_reference)
                && r.amount
                    .is_some_and(|a| a.is_sign_positive() && !a.is_zero())
                && let Some((parent_memo, parent_amount, parent_line)) =
                    by_id.get(&back.to_ascii_uppercase())
                && !parent_memo
                && *parent_line != line.line_no
            {
                self.push(Observation::new(
                    Concern::MemoDoubleCount,
                    Some(line),
                    r.amount,
                    format!(
                        "{} ({}) back-references {back} on line {parent_line} ({}), and both are \
                         counted; the child of a counted line must be a memo entry or the amount \
                         is in the totals twice",
                        line.raw_form_type,
                        r.amount
                            .map_or_else(|| "no amount".to_string(), |a| a.to_string()),
                        parent_amount.map_or_else(|| "no amount".to_string(), |a| a.to_string())
                    ),
                ));
            }

            // (5b) an individual's SA11AI memo with no parent and no reason.
            if r.is_memo
                && r.is_individual
                && r.back_reference.is_none()
                && line.raw_form_type.eq_ignore_ascii_case("SA11AI")
                && !contains_any(&r.text, MEMO_REASON_WORDS)
            {
                self.push(Observation::new(
                    Concern::MemoWithoutParent,
                    Some(line),
                    r.amount,
                    format!(
                        "memo entry from {} has no back-reference and its text does not say \
                         what it itemizes; a memo entry should reference the counted \
                         transaction it breaks down",
                        if r.name.is_empty() {
                            "(unnamed)"
                        } else {
                            &r.name
                        }
                    ),
                ));
            }

            if r.key.is_empty() {
                continue;
            }

            // (9) identical contributor, date, amount at $200 and over.
            if let (Some(amount), Some(date)) = (counted, r.date)
                && amount >= threshold
            {
                let k = (line.raw_form_type.as_str(), r.key.as_str(), date, amount);
                match seen.get(&k) {
                    Some(first) => {
                        self.push(Observation::new(
                            Concern::DuplicateTransaction,
                            Some(line),
                            Some(amount),
                            format!(
                                "{} from {} dated {date} for {amount} repeats line {}{}; if this is \
                                 one contribution reported twice, the second entry overstates \
                                 the totals",
                                line.raw_form_type,
                                r.name,
                                first.line_no,
                                first
                                    .get_non_empty("transaction_id")
                                    .map_or_else(String::new, |id| format!(" ({id})"))
                            ),
                        ));
                    }
                    None => {
                        seen.insert(k, line);
                    }
                }
                // (9, chain) the same transaction in an earlier report.
                if let Some(prior_cov) = self
                    .prior
                    .transactions
                    .get(&(line.raw_form_type.clone(), r.key.clone(), date, amount))
                    .copied()
                {
                    self.push(Observation::new(
                        Concern::DuplicateTransaction,
                        Some(line),
                        Some(amount),
                        format!(
                            "{} from {} dated {date} for {amount} was already reported on the \
                             report covering {prior_cov}",
                            line.raw_form_type, r.name
                        ),
                    ));
                }
            }

            if !r.is_individual {
                continue;
            }

            // Per-election sums (counted lines plus redesignation memos).
            if let Some(amount) = r.amount
                && (!r.is_memo || r.is_redesignation_memo())
            {
                let e = by_election
                    .entry((r.key.as_str(), r.election.as_str()))
                    .or_insert((Decimal::ZERO, r.name.as_str(), line));
                e.0 = e.0.saturating_add(amount);
                e.2 = line;
            }

            let Some(amount) = counted else {
                continue;
            };
            let prior_sum = self
                .prior
                .by_contributor
                .get(r.key.as_str())
                .copied()
                .unwrap_or(Decimal::ZERO);
            let run = running.entry(r.key.as_str()).or_insert(prior_sum);
            *run = run.saturating_add(amount);
            let in_file = *run;
            let c = by_contributor.entry(r.key.as_str()).or_insert((
                Decimal::ZERO,
                Decimal::ZERO,
                r.name.as_str(),
                line,
            ));
            c.0 = c.0.saturating_add(amount);
            c.1 = c.1.max(r.aggregate.unwrap_or(Decimal::ZERO));
            c.3 = line;

            // (1) employer and occupation over the threshold.
            let reported = r.aggregate.unwrap_or(Decimal::ZERO);
            let aggregate = reported.max(in_file);
            if aggregate <= threshold {
                continue;
            }
            let employer = r.employer.trim();
            let occupation = r.occupation.trim();
            let employer_up = employer.to_ascii_uppercase();
            let occupation_up = occupation.to_ascii_uppercase();
            if contains_any(&employer_up, BEST_EFFORTS_WORDS)
                || contains_any(&occupation_up, BEST_EFFORTS_WORDS)
            {
                self.push(Observation::new(
                    Concern::BestEffortsClaimed,
                    Some(line),
                    Some(amount),
                    format!(
                        "{} aggregates {aggregate} and the employer/occupation fields read \
                         '{employer}' / '{occupation}': the committee is reporting under best \
                         efforts (11 CFR 104.7(b))",
                        r.name
                    ),
                ));
                continue;
            }
            let missing = match (employer.is_empty(), occupation.is_empty()) {
                (false, false) => None,
                (true, true) => Some("employer and occupation"),
                (true, false) => {
                    (!NOT_EMPLOYED.contains(&squash(occupation).as_str())).then_some("employer")
                }
                (false, true) => {
                    (!NOT_EMPLOYED.contains(&squash(employer).as_str())).then_some("occupation")
                }
            };
            if let Some(missing) = missing {
                let basis = if reported >= in_file {
                    "as reported".to_string()
                } else if self.prior.reports > 0 {
                    format!(
                        "summed over this report and {} prior report(s)",
                        self.prior.reports
                    )
                } else {
                    "summed over this report's lines".to_string()
                };
                self.push(Observation::new(
                    Concern::EmployerOccupationMissing,
                    Some(line),
                    Some(amount),
                    format!(
                        "{} aggregates {aggregate} ({basis}), over the $200 threshold, and the \
                         {missing} field is blank; 11 CFR 104.3(a)(4)(i) requires both once \
                         the aggregate exceeds $200",
                        r.name
                    ),
                ));
            }
        }

        // (2) limits.
        let Some(recipient) = self.recipient else {
            return;
        };
        let report_year = self.coverage.map(|c| c.through.year());
        match recipient {
            Recipient::Unlimited => {}
            Recipient::Candidate => {
                let mut groups: Vec<_> = by_election.into_iter().collect();
                groups.sort_by_key(|(_, (_, _, line))| line.line_no);
                for ((key, election), (in_file, name, line)) in groups {
                    let prior = self
                        .prior
                        .by_election
                        .get(&(key.to_string(), election.to_string()))
                        .copied()
                        .unwrap_or(Decimal::ZERO);
                    let total = in_file.saturating_add(prior);
                    let year = election_year(election).or(report_year);
                    let Some(limits) = year.and_then(contribution_limits) else {
                        continue;
                    };
                    let Some(limit) = limits.for_recipient(recipient) else {
                        continue;
                    };
                    if total > limit {
                        let election_label = if election.is_empty() {
                            "an unstated election".to_string()
                        } else {
                            format!("election {election}")
                        };
                        let span = if prior.is_zero() {
                            "in this report".to_string()
                        } else {
                            format!(
                                "over this report and {} prior report(s)",
                                self.prior.reports
                            )
                        };
                        self.push(Observation::new(
                            Concern::OverLimitAggregate,
                            Some(line),
                            Some(total.saturating_sub(limit)),
                            format!(
                                "{name}'s contributions for {election_label} total {total} {span}, \
                                 {} over the {limit} per-election limit for the {} cycle (11 CFR \
                                 110.1(b)); redesignation and reattribution memo entries are \
                                 already netted",
                                total.saturating_sub(limit),
                                limits.cycle
                            ),
                        ));
                    }
                }
            }
            Recipient::Pac | Recipient::NationalParty | Recipient::StateParty => {
                let Some(limits) = report_year.and_then(contribution_limits) else {
                    return;
                };
                let Some(limit) = limits.for_recipient(recipient) else {
                    return;
                };
                let mut groups: Vec<_> = by_contributor.into_iter().collect();
                groups.sort_by_key(|(_, (_, _, _, line))| line.line_no);
                for (key, (in_file, reported, name, line)) in groups {
                    let prior = self
                        .prior
                        .by_contributor
                        .get(key)
                        .copied()
                        .unwrap_or(Decimal::ZERO);
                    let total = in_file.saturating_add(prior).max(reported);
                    if total > limit {
                        let how = if reported >= in_file.saturating_add(prior) {
                            "by the filer's own year-to-date aggregate".to_string()
                        } else if prior.is_zero() {
                            "summed over this report".to_string()
                        } else {
                            format!(
                                "summed over this report and {} prior report(s)",
                                self.prior.reports
                            )
                        };
                        let kind = match recipient {
                            Recipient::Pac => "a PAC (11 CFR 110.1(d))",
                            Recipient::StateParty => {
                                "a state or local party committee (11 CFR 110.1(c)(5))"
                            }
                            _ => {
                                "a national party committee's four accounts together (11 CFR \
                                 110.1(c)(2); 52 U.S.C. 30116(a)(9))"
                            }
                        };
                        self.push(Observation::new(
                            Concern::OverLimitAggregate,
                            Some(line),
                            Some(total.saturating_sub(limit)),
                            format!(
                                "{name}'s contributions total {total} ({how}), {} over the {limit} \
                                 per-calendar-year limit to {kind} for {}",
                                total.saturating_sub(limit),
                                limits.cycle
                            ),
                        ));
                    }
                }
            }
        }
    }

    // -- Schedule B: (3) (7) ---------------------------------------------

    fn check_schedule_b(&mut self) {
        let filing = self.filing;
        for line in filing.lines_for(Table::SchB) {
            let Ok(t) = line.typed::<SchB>() else {
                continue;
            };
            if line.is_memo() {
                continue;
            }
            let Some(amount) = t.money(sch_b::EXPENDITURE_AMOUNT) else {
                continue;
            };
            let mut text = t
                .get(sch_b::EXPENDITURE_PURPOSE_DESCRIP)
                .unwrap_or("")
                .to_ascii_uppercase();
            if let Some(memo) = t.get(sch_b::MEMO_TEXT) {
                text.push(' ');
                text.push_str(&memo.to_ascii_uppercase());
            }
            let payee = [
                sch_b::PAYEE_ORGANIZATION_NAME,
                sch_b::PAYEE_LAST_NAME,
                sch_b::PAYEE_NAME,
            ]
            .into_iter()
            .find_map(|f| t.get(f))
            .unwrap_or("(unnamed)");
            if amount.is_sign_negative() && !contains_any(&text, ADJUSTMENT_WORDS) {
                self.push(Observation::new(
                    Concern::NegativeItemization,
                    Some(line),
                    Some(amount),
                    format!(
                        "{} carries a negative counted amount {amount} to {payee} with no reason \
                         in its text; money that came back to the committee is a receipt \
                         (Schedule A offsets or refunds), not a negative disbursement",
                        line.raw_form_type
                    ),
                ));
            }
            if let (Some(cov), Some(date)) = (self.coverage, t.date(sch_b::EXPENDITURE_DATE))
                && amount.is_sign_positive()
                && !amount.is_zero()
                && !contains_any(&text, ADJUSTMENT_WORDS)
                && date > cov.through
            {
                self.push(Observation::new(
                    Concern::DateOutsideCoverage,
                    Some(line),
                    Some(amount),
                    format!(
                        "{} to {payee} is dated {date}, after the coverage period {cov} closed; \
                         a disbursement after the period belongs on the next report",
                        line.raw_form_type
                    ),
                ));
            }
        }
    }

    // -- (6) cover rules -------------------------------------------------

    fn check_cover_rules(&mut self) {
        let filing = self.filing;
        let Ok(r) = filing.reconcile() else {
            return;
        };
        for check in r.mismatches() {
            self.push(cover_observation(Concern::CoverNotSupported, filing, check));
        }
    }

    // -- (8) unitemized ---------------------------------------------------

    fn check_unitemized(&mut self) {
        let filing = self.filing;
        let cover = &filing.summary;
        let Some(unitemized) = cover
            .get_non_empty("col_a_individuals_unitemized")
            .and_then(parse_money)
        else {
            return;
        };
        if unitemized.is_sign_negative() {
            self.push(Observation::new(
                Concern::UnitemizedInconsistent,
                Some(cover),
                Some(unitemized),
                format!(
                    "unitemized individual contributions are reported as {unitemized}; the line \
                     is a sum of receipts and cannot be negative"
                ),
            ));
            return;
        }
        let itemized = cover
            .get_non_empty("col_a_individuals_itemized")
            .and_then(parse_money)
            .unwrap_or(Decimal::ZERO);
        let any_itemized = filing.lines_for(Table::SchA).any(|l| {
            !l.is_memo()
                && l.get("entity_type")
                    .is_some_and(|e| e.eq_ignore_ascii_case("IND"))
        });
        if unitemized >= dollars(UNITEMIZED_WITHOUT_ITEMIZED_FLOOR)
            && itemized.is_zero()
            && !any_itemized
        {
            self.push(Observation::new(
                Concern::UnitemizedInconsistent,
                Some(cover),
                Some(unitemized),
                format!(
                    "unitemized individual contributions are {unitemized} while no individual \
                     contribution is itemized; a committee that raised this much from \
                     individuals with none over $200 in aggregate is unusual (heuristic: \
                     threshold ${UNITEMIZED_WITHOUT_ITEMIZED_FLOOR})"
                ),
            ));
        }
    }

    // -- (9) ids and (5) unresolved memo parents, from the validator ------

    fn check_validation(&mut self) {
        let filing = self.filing;
        let by_line: HashMap<u64, &'a ParsedLine> =
            filing.lines.iter().map(|l| (l.line_no, l)).collect();
        let memo_lines: HashSet<u64> = filing
            .lines
            .iter()
            .filter(|l| l.is_memo())
            .map(|l| l.line_no)
            .collect();
        let validation = filing.validate();
        for finding in validation.iter() {
            let line = by_line.get(&finding.line_no).copied();
            let amount = line.and_then(|l| {
                ["contribution_amount", "expenditure_amount"]
                    .iter()
                    .find_map(|f| l.get_non_empty(f))
                    .and_then(parse_money)
            });
            match finding.rule {
                Rule::DuplicateTransactionId => {
                    self.push(Observation::new(
                        Concern::DuplicateTransactionId,
                        line,
                        amount,
                        format!(
                            "{}; a transaction id must be unique for the life of the report, \
                             and a repeated id usually means a transaction was entered twice",
                            finding.message
                        ),
                    ));
                }
                Rule::BackReferenceNotFound if memo_lines.contains(&finding.line_no) => {
                    self.push(Observation::new(
                        Concern::MemoWithoutParent,
                        line,
                        amount,
                        format!(
                            "{}; the memo entry cannot be tied to the counted transaction it \
                             breaks down",
                            finding.message
                        ),
                    ));
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use rust_decimal_macros::dec;
    use strum::IntoEnumIterator;

    use super::*;
    use crate::parser::SpecVersion;

    const V85: SpecVersion = SpecVersion::electronic(8, 5);

    fn f3x(cover: &[(&str, &str)], body: Vec<ParsedLine>) -> Filing {
        let mut filing =
            Filing::parse("HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF3XN\u{1c}C00123456").unwrap();
        let defaults = [
            ("coverage_from_date", "20260401"),
            ("coverage_through_date", "20260630"),
            ("treasurer_last_name", "Doe"),
            ("treasurer_first_name", "Pat"),
            ("date_signed", "20260715"),
        ];
        for (k, v) in defaults.iter().chain(cover) {
            filing.summary.set(k, v).unwrap();
        }
        filing.lines = body;
        filing
    }

    /// Cover values that make an F3X with one itemized individual
    /// contribution of `amount` (and nothing else) reconcile on every
    /// line, in both columns.
    fn balanced_cover(amount: &str) -> Vec<(&'static str, &str)> {
        let mut v = Vec::new();
        for col in ["col_a", "col_b"] {
            for field in [
                "individuals_itemized",
                "individual_contribution_total",
                "total_contributions",
                "total_receipts_recap",
                "total_federal_receipts",
                "total_contributions_recap",
                "net_contributions",
                "total_receipts",
                "subtotal",
                "cash_on_hand_close_of_period",
            ] {
                let name: &'static str = Box::leak(format!("{col}_{field}").into_boxed_str());
                v.push((name, amount));
            }
        }
        v
    }

    fn f3(cover: &[(&str, &str)], body: Vec<ParsedLine>) -> Filing {
        let mut filing =
            Filing::parse("HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF3N\u{1c}C00123456").unwrap();
        let defaults = [
            ("coverage_from_date", "20260401"),
            ("coverage_through_date", "20260630"),
            ("treasurer_last_name", "Doe"),
            ("treasurer_first_name", "Pat"),
            ("date_signed", "20260715"),
        ];
        for (k, v) in defaults.iter().chain(cover) {
            filing.summary.set(k, v).unwrap();
        }
        filing.lines = body;
        filing
    }

    /// A Schedule A line with sensible defaults; `extra` overrides.
    fn sa(line_no: u64, id: &str, amount: &str, extra: &[(&str, &str)]) -> ParsedLine {
        let mut pairs = vec![
            ("form_type", "SA11AI"),
            ("filer_committee_id_number", "C00123456"),
            ("transaction_id", id),
            ("entity_type", "IND"),
            ("contributor_last_name", "SMITH"),
            ("contributor_first_name", "JANE"),
            ("contributor_zip_code", "22150"),
            ("contribution_date", "20260415"),
            ("contribution_amount", amount),
            ("contribution_aggregate", amount),
            ("contributor_employer", "ACME"),
            ("contributor_occupation", "ENGINEER"),
        ];
        pairs.extend_from_slice(extra);
        ParsedLine::from_pairs(Table::SchA, V85, line_no, pairs).unwrap()
    }

    fn sb(line_no: u64, id: &str, amount: &str, extra: &[(&str, &str)]) -> ParsedLine {
        let mut pairs = vec![
            ("form_type", "SB21B"),
            ("filer_committee_id_number", "C00123456"),
            ("transaction_id", id),
            ("entity_type", "ORG"),
            ("payee_organization_name", "PRINTER INC"),
            ("expenditure_date", "20260415"),
            ("expenditure_amount", amount),
        ];
        pairs.extend_from_slice(extra);
        ParsedLine::from_pairs(Table::SchB, V85, line_no, pairs).unwrap()
    }

    #[test]
    fn concern_names_round_trip_and_have_descriptions() {
        for c in Concern::iter() {
            let name = c.to_string();
            assert_eq!(Concern::from_str(&name).unwrap(), c);
            assert!(!c.describe().is_empty());
            if c == Concern::BestEffortsClaimed {
                assert_eq!(c.rfai_request_type(), None);
            } else {
                assert_eq!(c.rfai_request_type(), Some(2));
            }
        }
        assert_eq!(
            Concern::CoverNotSupported.to_string(),
            "cover_not_supported"
        );
    }

    #[test]
    fn limits_table_is_sorted_and_indexed_by_cycle() {
        assert!(
            CONTRIBUTION_LIMITS
                .windows(2)
                .all(|w| w[0].cycle < w[1].cycle)
        );
        assert_eq!(contribution_limits(2023).unwrap().cycle, 2024);
        assert_eq!(contribution_limits(2024).unwrap().cycle, 2024);
        assert_eq!(
            contribution_limits(2025)
                .unwrap()
                .for_recipient(Recipient::Candidate),
            Some(dec!(3500))
        );
        assert_eq!(contribution_limits(2010), None);
        assert_eq!(
            contribution_limits(2020)
                .unwrap()
                .for_recipient(Recipient::Unlimited),
            None
        );
        assert_eq!(
            contribution_limits(2026)
                .unwrap()
                .for_recipient(Recipient::NationalParty),
            Some(dec!(443_000))
        );
        assert_eq!(
            Recipient::from_str("national-party"),
            Ok(Recipient::NationalParty)
        );
        assert_eq!(
            Recipient::from_openfec_committee_type("h"),
            Some(Recipient::Candidate)
        );
        assert_eq!(
            Recipient::from_openfec_committee_type("Y"),
            Some(Recipient::StateParty)
        );
        assert_eq!(
            Recipient::from_openfec_committee_type("O"),
            Some(Recipient::Unlimited)
        );
        assert_eq!(Recipient::from_openfec_committee_type("I"), None);
        assert_eq!(Recipient::implied_by_form(Table::F24), None);
        assert_eq!(Recipient::implied_by_form(Table::F3X), None);
        assert_eq!(
            Recipient::implied_by_form(Table::F3),
            Some(Recipient::Candidate)
        );
    }

    #[test]
    fn a_stub_filing_raises_only_cover_concerns() {
        let filing = f3x(
            &[
                ("col_a_individuals_itemized", "250.00"),
                ("col_a_individuals_unitemized", "0"),
            ],
            vec![sa(3, "A1", "250.00", &[])],
        );
        let review = filing.review();
        // The other cover lines (11(a)(iii), 11(d), 19, 6(c), ...) are blank
        // on this stub, so the cover concern fires; nothing else may.
        assert!(review.has(Concern::CoverNotSupported));
        assert!(
            review
                .iter()
                .all(|o| o.concern == Concern::CoverNotSupported),
            "{review}"
        );
    }

    #[test]
    fn employer_occupation_missing_fires_over_200_and_not_under() {
        // One contributor per line, so the in-file running sums do not
        // cross the threshold on their own.
        let filing = f3x(
            &[],
            vec![
                sa(3, "A1", "250.00", &[("contributor_employer", "")]),
                sa(
                    4,
                    "A2",
                    "150.00",
                    &[("contributor_employer", ""), ("contributor_last_name", "B")],
                ),
                sa(
                    5,
                    "A3",
                    "500.00",
                    &[
                        ("contributor_occupation", ""),
                        ("contributor_last_name", "C"),
                    ],
                ),
                // Retired: employer may be blank.
                sa(
                    6,
                    "A4",
                    "300.00",
                    &[
                        ("contributor_employer", ""),
                        ("contributor_occupation", "Retired"),
                        ("contributor_last_name", "D"),
                    ],
                ),
                // Memo entries are not checked.
                sa(
                    7,
                    "A5",
                    "300.00",
                    &[
                        ("contributor_employer", ""),
                        ("memo_code", "X"),
                        ("contributor_last_name", "E"),
                    ],
                ),
                // Not an individual.
                sa(
                    8,
                    "A6",
                    "300.00",
                    &[
                        ("contributor_employer", ""),
                        ("entity_type", "ORG"),
                        ("contributor_last_name", "F"),
                    ],
                ),
            ],
        );
        let review = filing.review();
        let lines: Vec<u64> = review
            .by_concern(Concern::EmployerOccupationMissing)
            .filter_map(|o| o.line_no)
            .collect();
        assert_eq!(lines, vec![3, 5], "{review}");
        let o = review
            .by_concern(Concern::EmployerOccupationMissing)
            .next()
            .unwrap();
        assert_eq!(o.amount, Some(dec!(250.00)));
        assert_eq!(o.transaction_id.as_deref(), Some("A1"));
        assert!(o.detail.contains("employer field is blank"), "{}", o.detail);
        assert_eq!(o.rfai_request_type, Some(2));
    }

    #[test]
    fn aggregate_is_the_larger_of_reported_and_in_file() {
        // Two $150 gifts with a stale aggregate of 150: the second crosses
        // $200 by the file's own arithmetic.
        let filing = f3x(
            &[],
            vec![
                sa(
                    3,
                    "A1",
                    "150.00",
                    &[
                        ("contributor_employer", ""),
                        ("contribution_aggregate", "150.00"),
                    ],
                ),
                sa(
                    4,
                    "A2",
                    "150.00",
                    &[
                        ("contributor_employer", ""),
                        ("contribution_aggregate", "150.00"),
                    ],
                ),
            ],
        );
        let review = filing.review();
        let lines: Vec<u64> = review
            .by_concern(Concern::EmployerOccupationMissing)
            .filter_map(|o| o.line_no)
            .collect();
        assert_eq!(lines, vec![4], "{review}");
        assert!(
            review
                .by_concern(Concern::EmployerOccupationMissing)
                .next()
                .unwrap()
                .detail
                .contains("summed over this report's lines")
        );
    }

    #[test]
    fn best_efforts_language_is_reported_separately() {
        let filing = f3x(
            &[],
            vec![
                sa(
                    3,
                    "A1",
                    "250.00",
                    &[
                        ("contributor_employer", "INFORMATION REQUESTED"),
                        ("contributor_occupation", "INFORMATION REQUESTED"),
                    ],
                ),
                sa(
                    4,
                    "A2",
                    "250.00",
                    &[
                        (
                            "contributor_employer",
                            "Information Requested Per Best Efforts",
                        ),
                        ("contributor_occupation", ""),
                    ],
                ),
            ],
        );
        let review = filing.review();
        assert_eq!(review.by_concern(Concern::BestEffortsClaimed).count(), 2);
        assert!(!review.has(Concern::EmployerOccupationMissing), "{review}");
        assert_eq!(
            review
                .by_concern(Concern::BestEffortsClaimed)
                .next()
                .unwrap()
                .rfai_request_type,
            None
        );
    }

    #[test]
    fn over_limit_per_election_for_a_candidate_nets_redesignations() {
        let filing = f3(
            &[],
            vec![
                // 7,000 on P2026 with 3,500 redesignated to the general: fine.
                sa(3, "A1", "7000.00", &[("election_code", "P2026")]),
                sa(
                    4,
                    "A2",
                    "-3500.00",
                    &[
                        ("election_code", "P2026"),
                        ("memo_code", "X"),
                        ("back_reference_tran_id", "A1"),
                        ("memo_text", "REDESIGNATION TO GENERAL"),
                    ],
                ),
                sa(
                    5,
                    "A3",
                    "3500.00",
                    &[
                        ("election_code", "G2026"),
                        ("memo_code", "X"),
                        ("back_reference_tran_id", "A1"),
                        ("memo_text", "REDESIGNATION FROM PRIMARY"),
                    ],
                ),
                // 3,600 on G2026 from someone else: 100 over.
                sa(
                    6,
                    "B1",
                    "3600.00",
                    &[
                        ("election_code", "G2026"),
                        ("contributor_last_name", "JONES"),
                        ("contributor_first_name", "ANN"),
                    ],
                ),
                // Two 2,000 gifts for P2026 from a third person: 500 over.
                sa(
                    7,
                    "C1",
                    "2000.00",
                    &[("election_code", "P2026"), ("contributor_last_name", "LEE")],
                ),
                sa(
                    8,
                    "C2",
                    "2000.00",
                    &[
                        ("election_code", "P2026"),
                        ("contributor_last_name", "LEE"),
                        ("contribution_date", "20260416"),
                    ],
                ),
            ],
        );
        let review = filing.review();
        let over: Vec<(u64, Decimal)> = review
            .by_concern(Concern::OverLimitAggregate)
            .map(|o| (o.line_no.unwrap(), o.amount.unwrap()))
            .collect();
        assert_eq!(over, vec![(6, dec!(100.00)), (8, dec!(500.00))], "{review}");
        assert!(
            review
                .by_concern(Concern::OverLimitAggregate)
                .next()
                .unwrap()
                .detail
                .contains("3500 per-election limit for the 2026 cycle")
        );
    }

    #[test]
    fn over_limit_per_year_uses_the_recipient_and_skips_unknown_cycles() {
        let body = vec![sa(
            3,
            "A1",
            "6000.00",
            &[("contribution_aggregate", "6000.00")],
        )];
        // A Form 3X with no stated recipient: not checked.
        let filing = f3x(&[], body.clone());
        assert!(!filing.review().has(Concern::OverLimitAggregate));
        // As a state party: 6,000 is under the 10,000-a-year limit.
        let review = filing.review_with(&ReviewOptions::new().recipient(Recipient::StateParty));
        assert!(!review.has(Concern::OverLimitAggregate));
        // Stated as a PAC: 1,000 over.
        let review = filing.review_with(&ReviewOptions::new().recipient(Recipient::Pac));
        let o = review
            .by_concern(Concern::OverLimitAggregate)
            .next()
            .unwrap();
        assert_eq!(o.amount, Some(dec!(1000.00)));
        assert!(
            o.detail.contains("per-calendar-year limit to a PAC"),
            "{}",
            o.detail
        );
        // Unlimited: nothing.
        let review = filing.review_with(&ReviewOptions::new().recipient(Recipient::Unlimited));
        assert!(!review.has(Concern::OverLimitAggregate));
        // A cycle without a limits row is not checked.
        let old = f3x(
            &[
                ("coverage_from_date", "20100101"),
                ("coverage_through_date", "20100331"),
            ],
            body,
        );
        assert!(
            !old.review_with(&ReviewOptions::new().recipient(Recipient::Pac))
                .has(Concern::OverLimitAggregate)
        );
    }

    #[test]
    fn dates_after_coverage_skip_memos_negatives_refunds_and_early_dates() {
        let filing = f3x(
            &[],
            vec![
                sa(3, "A1", "100.00", &[("contribution_date", "20260315")]),
                sa(4, "A2", "100.00", &[("contribution_date", "20260701")]),
                sa(
                    5,
                    "A3",
                    "100.00",
                    &[("contribution_date", "20260315"), ("memo_code", "X")],
                ),
                sa(
                    6,
                    "A4",
                    "-100.00",
                    &[
                        ("contribution_date", "20260315"),
                        ("memo_text", "RETURNED CHECK"),
                    ],
                ),
                sa(
                    7,
                    "A5",
                    "100.00",
                    &[
                        ("contribution_date", "20260315"),
                        ("contribution_purpose_descrip", "Refund of overpayment"),
                    ],
                ),
                sa(8, "A6", "100.00", &[("contribution_date", "20260401")]),
                sb(9, "B1", "50.00", &[("expenditure_date", "20260701")]),
                sb(10, "B2", "50.00", &[("expenditure_date", "20260630")]),
            ],
        );
        let review = filing.review();
        // Line 3 (dated before the period) is not flagged: only dates after
        // the period are.
        let lines: Vec<u64> = review
            .by_concern(Concern::DateOutsideCoverage)
            .filter_map(|o| o.line_no)
            .collect();
        assert_eq!(lines, vec![4, 9], "{review}");
        let first = review
            .by_concern(Concern::DateOutsideCoverage)
            .next()
            .unwrap();
        assert!(first.detail.contains("after the coverage period"));
        assert!(!review.has(Concern::NegativeItemization), "{review}");
    }

    #[test]
    fn treasurer_signature_missing_fires_on_blank_date_or_name() {
        let filing = f3x(&[("date_signed", "")], vec![]);
        let review = filing.review();
        assert_eq!(
            review
                .by_concern(Concern::TreasurerSignatureMissing)
                .count(),
            1
        );
        let filing = f3x(
            &[("treasurer_last_name", ""), ("treasurer_first_name", "")],
            vec![],
        );
        let review = filing.review();
        let o = review
            .by_concern(Concern::TreasurerSignatureMissing)
            .next()
            .unwrap();
        assert_eq!(o.line_no, Some(2));
        assert!(o.detail.contains("names no treasurer"));
        assert!(
            !f3x(&[], vec![])
                .review()
                .has(Concern::TreasurerSignatureMissing)
        );
    }

    #[test]
    fn memo_double_count_needs_a_counted_child_of_a_counted_parent() {
        let filing = f3x(
            &[],
            vec![
                // Conduit total, counted.
                sa(
                    3,
                    "P1",
                    "500.00",
                    &[
                        ("entity_type", "ORG"),
                        ("contributor_organization_name", "ACTBLUE"),
                    ],
                ),
                // Earmarked child, correctly a memo.
                sa(
                    4,
                    "C1",
                    "250.00",
                    &[("back_reference_tran_id", "P1"), ("memo_code", "X")],
                ),
                // Earmarked child, wrongly counted.
                sa(
                    5,
                    "C2",
                    "250.00",
                    &[
                        ("back_reference_tran_id", "p1"),
                        ("contributor_last_name", "JONES"),
                    ],
                ),
                // Counted child of a memo parent (the WinRed style): fine.
                sa(
                    6,
                    "M1",
                    "100.00",
                    &[
                        ("memo_code", "X"),
                        ("entity_type", "PAC"),
                        ("contributor_organization_name", "WINRED"),
                        ("memo_text", "TOTAL EARMARKED THROUGH CONDUIT"),
                    ],
                ),
                sa(
                    7,
                    "C3",
                    "100.00",
                    &[
                        ("back_reference_tran_id", "M1"),
                        ("contributor_last_name", "LEE"),
                    ],
                ),
                // A counted reversal of a counted parent: a chargeback, not
                // a double count.
                sa(
                    8,
                    "R1",
                    "-250.00",
                    &[
                        ("back_reference_tran_id", "C2"),
                        ("contributor_last_name", "JONES"),
                        ("memo_text", "CHARGEBACK"),
                    ],
                ),
            ],
        );
        let review = filing.review();
        let lines: Vec<u64> = review
            .by_concern(Concern::MemoDoubleCount)
            .filter_map(|o| o.line_no)
            .collect();
        assert_eq!(lines, vec![5], "{review}");
        assert!(!review.has(Concern::MemoWithoutParent), "{review}");
    }

    #[test]
    fn memo_without_parent_covers_unresolved_and_unexplained_memos() {
        let filing = f3x(
            &[],
            vec![
                sa(
                    3,
                    "M1",
                    "250.00",
                    &[("memo_code", "X"), ("back_reference_tran_id", "NOPE")],
                ),
                sa(4, "M2", "250.00", &[("memo_code", "X")]),
                sa(
                    5,
                    "M3",
                    "250.00",
                    &[
                        ("memo_code", "X"),
                        ("memo_text", "JFC TRANSFER: VICTORY FUND"),
                    ],
                ),
                sa(
                    6,
                    "M4",
                    "250.00",
                    &[("memo_code", "X"), ("form_type", "SA12")],
                ),
            ],
        );
        let review = filing.review();
        let lines: Vec<u64> = review
            .by_concern(Concern::MemoWithoutParent)
            .filter_map(|o| o.line_no)
            .collect();
        assert_eq!(lines, vec![3, 4], "{review}");
    }

    #[test]
    fn cover_not_supported_delegates_to_reconcile() {
        let filing = f3x(
            &[
                ("col_a_individuals_itemized", "300.00"),
                ("col_a_individuals_unitemized", "0"),
                ("col_a_individual_contribution_total", "300.00"),
            ],
            vec![sa(3, "A1", "250.00", &[])],
        );
        let review = filing.review();
        let o = review
            .by_concern(Concern::CoverNotSupported)
            .find(|o| o.detail.contains("line 11(a)(i)"))
            .unwrap();
        assert_eq!(o.amount, Some(dec!(50.00)));
        assert_eq!(o.line_no, Some(2));
        assert!(o.detail.contains("reports 300.00"));
        // A form without cover rules produces none.
        let f24 =
            Filing::parse("HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF24N\u{1c}C00123456").unwrap();
        assert!(!f24.review().has(Concern::CoverNotSupported));
    }

    #[test]
    fn negative_itemization_needs_a_counted_unexplained_negative() {
        let filing = f3x(
            &[],
            vec![
                sa(3, "A1", "-600.00", &[]),
                sa(
                    4,
                    "A2",
                    "-600.00",
                    &[("memo_text", "REATTRIBUTION TO SPOUSE"), ("memo_code", "X")],
                ),
                sa(
                    5,
                    "A3",
                    "-600.00",
                    &[("contribution_purpose_descrip", "NSF check")],
                ),
                sb(6, "B1", "-25.00", &[]),
                sb(
                    7,
                    "B2",
                    "-25.00",
                    &[("expenditure_purpose_descrip", "Void")],
                ),
            ],
        );
        let review = filing.review();
        let lines: Vec<u64> = review
            .by_concern(Concern::NegativeItemization)
            .filter_map(|o| o.line_no)
            .collect();
        assert_eq!(lines, vec![3, 6], "{review}");
        assert_eq!(
            review.summary.by_concern[&Concern::NegativeItemization].amount,
            dec!(625.00)
        );
    }

    #[test]
    fn unitemized_inconsistent_is_negative_or_large_and_alone() {
        let filing = f3x(&[("col_a_individuals_unitemized", "-5.00")], vec![]);
        assert_eq!(
            filing
                .review()
                .by_concern(Concern::UnitemizedInconsistent)
                .count(),
            1
        );
        let filing = f3x(&[("col_a_individuals_unitemized", "30000.00")], vec![]);
        assert!(filing.review().has(Concern::UnitemizedInconsistent));
        let filing = f3x(
            &[("col_a_individuals_unitemized", "30000.00")],
            vec![sa(3, "A1", "250.00", &[])],
        );
        assert!(!filing.review().has(Concern::UnitemizedInconsistent));
        let filing = f3x(&[("col_a_individuals_unitemized", "24999.99")], vec![]);
        assert!(!filing.review().has(Concern::UnitemizedInconsistent));
    }

    #[test]
    fn duplicates_by_id_and_by_contributor_date_amount() {
        let filing = f3x(
            &[],
            vec![
                sa(3, "A1", "250.00", &[]),
                sa(4, "A1", "75.00", &[("contributor_last_name", "OTHER")]),
                sa(5, "A3", "250.00", &[]),
                // Under $200: not a pair.
                sa(6, "A4", "50.00", &[("contributor_last_name", "SMALL")]),
                sa(7, "A5", "50.00", &[("contributor_last_name", "SMALL")]),
                // A memo repeat is not a pair.
                sa(8, "A6", "250.00", &[("memo_code", "X")]),
            ],
        );
        let review = filing.review();
        let ids: Vec<u64> = review
            .by_concern(Concern::DuplicateTransactionId)
            .filter_map(|o| o.line_no)
            .collect();
        assert_eq!(ids, vec![4], "{review}");
        let pairs: Vec<u64> = review
            .by_concern(Concern::DuplicateTransaction)
            .filter_map(|o| o.line_no)
            .collect();
        assert_eq!(pairs, vec![5], "{review}");
        assert!(
            review
                .by_concern(Concern::DuplicateTransaction)
                .next()
                .unwrap()
                .detail
                .contains("repeats line 3 (A1)")
        );
        assert!(Concern::DuplicateTransaction.is_heuristic());
        assert!(!Concern::DuplicateTransactionId.is_heuristic());
    }

    #[test]
    fn a_balanced_stub_has_no_observations() {
        let filing = f3x(&balanced_cover("250.00"), vec![sa(3, "A1", "250.00", &[])]);
        assert!(filing.reconcile().unwrap().balances());
        let review = filing.review();
        assert!(review.is_empty(), "{review}");
    }

    #[test]
    fn display_and_summary() {
        let filing = f3x(
            &balanced_cover("250.00"),
            vec![sa(3, "A1", "250.00", &[("contributor_employer", "")])],
        );
        let review = filing.review();
        let text = review.to_string();
        assert!(
            text.contains("employer_occupation_missing line 3 A1 250.00: SMITH, JANE aggregates"),
            "{text}"
        );
        assert!(text.ends_with("amount at issue 250.00"), "{text}");
        assert_eq!(review.len(), 1);
        assert_eq!(review.summary.observations, 1);
        assert_eq!(review.summary.amount_at_issue, dec!(250.00));
        assert_eq!(
            review.summary.by_concern[&Concern::EmployerOccupationMissing].count,
            1
        );
        assert_eq!(
            f3x(&balanced_cover("0"), vec![]).review().to_string(),
            "no observations"
        );
        let mut n = 0;
        for o in &review {
            assert_eq!(o.concern, Concern::EmployerOccupationMissing);
            n += 1;
        }
        assert_eq!(n, 1);
        assert_eq!(review.strict().count(), 1);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn review_serializes_with_snake_case_concerns() {
        let filing = f3x(
            &balanced_cover("250.00"),
            vec![sa(3, "A1", "250.00", &[("contributor_employer", "")])],
        );
        let json = serde_json::to_value(filing.review()).unwrap();
        assert_eq!(
            json["observations"][0]["concern"],
            "employer_occupation_missing"
        );
        assert_eq!(json["observations"][0]["amount"], "250.00");
        assert_eq!(json["observations"][0]["rfai_request_type"], 2);
        assert_eq!(
            json["summary"]["by_concern"]["employer_occupation_missing"]["count"],
            1
        );
    }

    #[test]
    fn chain_review_adds_cash_and_cross_report_concerns() {
        let jan = f3x(
            &[
                ("coverage_from_date", "20260101"),
                ("coverage_through_date", "20260131"),
                ("col_a_cash_on_hand_close_of_period", "1000.00"),
            ],
            vec![
                // 150 in January with employer blank: under $200 then.
                sa(
                    3,
                    "J1",
                    "150.00",
                    &[
                        ("contribution_date", "20260110"),
                        ("contributor_employer", ""),
                        ("contribution_aggregate", "150.00"),
                    ],
                ),
                sa(
                    4,
                    "J2",
                    "250.00",
                    &[
                        ("contribution_date", "20260111"),
                        ("contributor_last_name", "REPEAT"),
                    ],
                ),
            ],
        );
        let feb = f3x(
            &[
                ("coverage_from_date", "20260201"),
                ("coverage_through_date", "20260228"),
                ("col_a_cash_on_hand_beginning_period", "900.00"),
            ],
            vec![
                // Another 100 in February, aggregate under-reported as 100:
                // the chain knows it is 250.
                sa(
                    3,
                    "F1",
                    "100.00",
                    &[
                        ("contribution_date", "20260210"),
                        ("contributor_employer", ""),
                        ("contribution_aggregate", "100.00"),
                    ],
                ),
                // The January 250 reported again in February.
                sa(
                    4,
                    "F2",
                    "250.00",
                    &[
                        ("contribution_date", "20260111"),
                        ("contributor_last_name", "REPEAT"),
                    ],
                ),
            ],
        );
        assert!(!feb.review().has(Concern::EmployerOccupationMissing));
        let chain = ReportChain::new(&feb, [&jan]).unwrap();
        let review = chain.review();
        let cash: Vec<_> = review.by_concern(Concern::ChainCashMismatch).collect();
        assert_eq!(cash.len(), 1, "{review}");
        assert_eq!(cash[0].amount, Some(dec!(100.00)));
        assert!(cash[0].detail.contains("line 6(b)"));
        let emp: Vec<_> = review
            .by_concern(Concern::EmployerOccupationMissing)
            .collect();
        assert_eq!(emp.len(), 1, "{review}");
        assert_eq!(emp[0].line_no, Some(3));
        assert!(
            emp[0].detail.contains("1 prior report(s)"),
            "{}",
            emp[0].detail
        );
        let dup: Vec<_> = review.by_concern(Concern::DuplicateTransaction).collect();
        assert_eq!(dup.len(), 1, "{review}");
        assert!(
            dup[0].detail.contains("already reported"),
            "{}",
            dup[0].detail
        );
        // A January date on the February report is before the period, which
        // is not flagged on its own; the chain is what catches the repeat.
        assert!(!review.has(Concern::DateOutsideCoverage));
    }
}
