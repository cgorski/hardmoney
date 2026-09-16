//! `Filing::review` and `ReportChain::review` against real filings.
//!
//! Every file in `tests/fixtures/` was accepted by the FEC, and a review
//! is not an acceptance check: an observation on an accepted filing is
//! the point (RAD reviews accepted reports). So this suite pins what fires
//! per fixture, the way `validate_fixtures.rs` pins warnings, so that a
//! rule change in either direction is visible. Two things are asserted
//! outright: no `cover_not_supported` on a fixture whose cover reconciles,
//! and the accepted-but-unbalanced filing in `tests/fixtures/rad/` gets
//! exactly its $200 on line 11(c).
//!
//! The golden pack (`tests/fixtures/golden/`) is built to be clean. Its
//! Form 3X files carry one deliberate negative `SA17` line of -1.00 from
//! FECfile+'s own test transaction set (`line_17` = 200.50 - 1.00 +
//! 800.50), which the negative-amount rule reports; the sidecar-documented
//! defects (`cover_off_by_one_cent`, `duplicate_transaction_id`) produce
//! exactly their concern. Nothing else may fire.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hardmoney::Filing;
use hardmoney::parser::reconcile::ReportChain;
use hardmoney::parser::review::{Concern, Recipient, Review, ReviewOptions};
use rust_decimal_macros::dec;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fixture_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".fec"))
        .collect();
    names.sort();
    names
}

fn open(path: &Path) -> Filing {
    Filing::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn counts(review: &Review) -> BTreeMap<Concern, usize> {
    review
        .summary
        .by_concern
        .iter()
        .map(|(c, t)| (*c, t.count))
        .collect()
}

/// What fires on the accepted fixtures, pinned exactly. Every fixture not
/// listed produces no observation.
#[test]
fn accepted_fixtures_observations_are_pinned() {
    use Concern::*;
    let expected: BTreeMap<&str, Vec<(Concern, usize)>> = BTreeMap::from([
        // Senate committee, 2026 Q2: one best-efforts donor; three
        // same-day, same-amount repeats at $200 and over.
        (
            "F3A_2011812.fec",
            vec![(BestEffortsClaimed, 1), (DuplicateTransaction, 3)],
        ),
        // Spec 8.0 House amendment from 2016: 22 individuals over $200 with
        // blank employer and occupation and no best-efforts language.
        ("F3A_767339_v8.0.fec", vec![(EmployerOccupationMissing, 22)]),
        // 2024 presidential amendment: one blank employer/occupation, two
        // contributors over $3,300 for the primary in one report (300.00
        // and 9.90 over), one $400 repeat.
        (
            "F3PA_1993032.fec",
            vec![
                (EmployerOccupationMissing, 1),
                (OverLimitAggregate, 2),
                (DuplicateTransaction, 1),
            ],
        ),
        ("F3XA_2011814.fec", vec![(BestEffortsClaimed, 2)]),
        // 2002, spec 3.00: six blank employer/occupation over $200; two
        // negative Schedule A entries with no reason given.
        (
            "F3XA_27789_v3.fec",
            vec![(EmployerOccupationMissing, 6), (NegativeItemization, 2)],
        ),
        // A joint fundraising / unlimited committee (no limit is checked on
        // a Form 3X without a stated recipient).
        (
            "F3XN_1965568.fec",
            vec![(BestEffortsClaimed, 4), (DuplicateTransaction, 1)],
        ),
        ("F3XN_771694_v8.0_cp1252.fec", vec![(BestEffortsClaimed, 1)]),
    ]);
    let dir = fixtures_dir();
    let names = fixture_names(&dir);
    assert!(names.len() >= 31, "found {} fixtures", names.len());
    let mut checked = 0;
    for name in &names {
        let filing = open(&dir.join(name));
        let review = filing.review();
        let got: Vec<(Concern, usize)> = counts(&review).into_iter().collect();
        let want = expected.get(name.as_str()).cloned().unwrap_or_default();
        assert_eq!(got, want, "{name}:\n{review}");
        // Accepted filings whose cover reconciles never get the cover
        // concern; the validator-delegated id concern never fires either.
        if filing.reconcile().is_ok_and(|r| r.balances()) {
            assert!(!review.has(Concern::CoverNotSupported), "{name}");
        }
        assert!(!review.has(Concern::DuplicateTransactionId), "{name}");
        for o in &review {
            assert!(!o.detail.is_empty(), "{name}: {o:?}");
            assert_eq!(o.rfai_request_type, o.concern.rfai_request_type());
            if let Some(a) = o.amount {
                assert!(a.abs() > dec!(0), "{name}: {o}");
            }
        }
        checked += 1;
    }
    assert_eq!(checked, names.len());
}

/// The accepted-but-unbalanced May 2026 report: exactly one cover concern,
/// on line 11(c), for the $200 the cover carries and Schedule A does not.
#[test]
fn rad_fixture_reports_the_200_dollar_cover_gap() {
    let filing = open(&fixtures_dir().join("rad/F3XA_2011912.fec"));
    let review = filing.review();
    let cover: Vec<_> = review.by_concern(Concern::CoverNotSupported).collect();
    assert_eq!(cover.len(), 1, "{review}");
    assert_eq!(cover[0].amount, Some(dec!(200.00)));
    assert_eq!(cover[0].line_no, Some(2));
    assert!(
        cover[0].detail.contains("line 11(c)"),
        "{}",
        cover[0].detail
    );
    assert!(cover[0].detail.contains("2045.00"), "{}", cover[0].detail);
    assert!(cover[0].detail.contains("1845.00"), "{}", cover[0].detail);
    assert_eq!(
        counts(&review),
        BTreeMap::from([
            (Concern::BestEffortsClaimed, 16),
            (Concern::CoverNotSupported, 1),
            (Concern::NegativeItemization, 5),
            (Concern::DuplicateTransaction, 3),
        ]),
        "{review}"
    );
    assert_eq!(
        review.summary.by_concern[&Concern::CoverNotSupported].amount,
        dec!(200.00)
    );
    // The five negatives are the unexplained reversals on lines 139-141
    // and 258-259; the eleven other negative lines in the file carry a
    // reason in their text.
    let negatives: Vec<u64> = review
        .by_concern(Concern::NegativeItemization)
        .filter_map(|o| o.line_no)
        .collect();
    assert_eq!(negatives, vec![139, 140, 141, 258, 259]);
    // Every observation serializes with the snake_case concern name.
    #[cfg(feature = "serde")]
    {
        let json = serde_json::to_value(&review).unwrap();
        assert_eq!(json["summary"]["observations"], 25);
        assert!(
            json["observations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["concern"] == "cover_not_supported")
        );
    }
}

/// The golden pack: clean except the documented -1.00 `SA17` line and the
/// two deliberate defects.
#[test]
fn golden_pack_has_only_the_documented_observations() {
    use Concern::*;
    let expected: BTreeMap<&str, Vec<(Concern, usize)>> = BTreeMap::from([
        ("f3x.fec", vec![(NegativeItemization, 1)]),
        (
            "f3x_cover_off_by_one_cent.fec",
            vec![(CoverNotSupported, 2), (NegativeItemization, 1)],
        ),
        (
            "f3x_duplicate_transaction_id.fec",
            vec![(NegativeItemization, 1), (DuplicateTransactionId, 1)],
        ),
        (
            "f3x_missing_required_field.fec",
            vec![(NegativeItemization, 1)],
        ),
    ]);
    let dir = fixtures_dir().join("golden");
    let names = fixture_names(&dir);
    assert!(names.len() >= 9, "found {} golden fixtures", names.len());
    for name in &names {
        let filing = open(&dir.join(name));
        let review = filing.review();
        let got: Vec<(Concern, usize)> = counts(&review).into_iter().collect();
        let want = expected.get(name.as_str()).cloned().unwrap_or_default();
        assert_eq!(got, want, "{name}:\n{review}");
        if let Some(o) = review.by_concern(NegativeItemization).next() {
            assert_eq!(o.transaction_id.as_deref(), Some("SA17.12"), "{name}");
            assert_eq!(o.amount, Some(dec!(-1.00)), "{name}");
        }
    }
    // The off-by-one-cent file: 11(a)(i) and 11(a)(iii), a cent each.
    let review = open(&dir.join("f3x_cover_off_by_one_cent.fec")).review();
    let cover: Vec<_> = review.by_concern(CoverNotSupported).collect();
    assert!(
        cover.iter().all(|o| o.amount == Some(dec!(0.01))),
        "{review}"
    );
    assert_eq!(
        review.summary.by_concern[&CoverNotSupported].amount,
        dec!(0.02)
    );
    // The golden F3 and F3P are candidate committees at the limit, not over.
    for name in ["f3.fec", "f3p.fec"] {
        let filing = open(&dir.join(name));
        assert_eq!(
            Recipient::implied_by_form(filing.summary.table()),
            Some(Recipient::Candidate)
        );
        assert!(filing.review().is_empty(), "{name}");
    }
}

/// The Republican Party of Minnesota chain: no cash-on-hand mismatch at any
/// link, no Column B concern on the March report with January and
/// February behind it, and the May report's $200 shows up in Column B too.
#[test]
fn chain_fixtures_have_no_cash_mismatch() {
    let dir = fixtures_dir().join("chain");
    let ye = open(&dir.join("F3XN_1943038.fec"));
    let jan = open(&dir.join("F3XN_1948502.fec"));
    let feb = open(&dir.join("F3XA_2011895.fec"));
    let mar = open(&dir.join("F3XA_2011898.fec"));
    let apr = open(&dir.join("F3XA_2011901.fec"));
    let may = open(&fixtures_dir().join("rad/F3XA_2011912.fec"));

    let chain = ReportChain::new(&mar, [&ye, &jan, &feb]).unwrap();
    assert!(chain.gaps().is_empty());
    let review = chain.review();
    assert!(!review.has(Concern::ChainCashMismatch), "{review}");
    assert!(!review.has(Concern::CoverNotSupported), "{review}");
    // The chain adds nothing the March file did not already say.
    assert_eq!(counts(&review), counts(&mar.review()), "{review}");

    let chain = ReportChain::new(&may, [&ye, &jan, &feb, &mar, &apr]).unwrap();
    assert!(chain.gaps().is_empty());
    let review = chain.review_with(&ReviewOptions::new().recipient(Recipient::StateParty));
    assert!(!review.has(Concern::ChainCashMismatch), "{review}");
    let cover: Vec<_> = review.by_concern(Concern::CoverNotSupported).collect();
    assert_eq!(cover.len(), 2, "{review}");
    assert!(cover.iter().all(|o| o.amount == Some(dec!(200.00))));
    assert!(
        cover
            .iter()
            .any(|o| o.detail.contains("column B line 11(c)"))
    );
    assert!(cover.iter().any(|o| o.detail.contains("over 5 report(s)")));
    // As a state party, no individual is over $10,000 for the year.
    assert!(!review.has(Concern::OverLimitAggregate), "{review}");

    // Every link carries cash forward.
    for (current, prior) in [
        (&jan, vec![&ye]),
        (&feb, vec![&ye, &jan]),
        (&apr, vec![&ye, &jan, &feb, &mar]),
    ] {
        let chain = ReportChain::new(current, prior).unwrap();
        assert!(
            !chain.review().has(Concern::ChainCashMismatch),
            "{}",
            chain.current().raw_form_type
        );
    }
}

/// A chain built from a doctored prior report shows the cash mismatch.
#[test]
fn chain_review_reports_a_broken_carry_forward() {
    let dir = fixtures_dir().join("chain");
    let mut jan = open(&dir.join("F3XN_1948502.fec"));
    let feb = open(&dir.join("F3XA_2011895.fec"));
    jan.summary
        .set("col_a_cash_on_hand_close_of_period", "1.00")
        .unwrap();
    let chain = ReportChain::new(&feb, [&jan]).unwrap();
    let review = chain.review();
    let cash: Vec<_> = review.by_concern(Concern::ChainCashMismatch).collect();
    assert_eq!(cash.len(), 1, "{review}");
    assert!(cash[0].detail.contains("line 6(b)"), "{}", cash[0].detail);
    assert_eq!(cash[0].line_no, Some(2));
}
