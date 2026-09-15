//! `Filing::reconcile` against real filings.
//!
//! Every Form 3X / 3 / 3P fixture in `tests/fixtures/` was accepted by the
//! FEC; the cover pages of all of them satisfy every Column A rule (the
//! itemization-threshold lines as floors, everything else exactly). Across
//! the wider local corpus of 124 real periodic reports, 102 satisfy every
//! rule; of the rest, 17 are truncated third-party samples (a filing cut
//! off mid-schedule cannot balance) and 5 carry genuine filer
//! discrepancies -- e.g. a committee whose reported 11(c) exceeds its
//! itemized SA11C lines by exactly one $200 contribution -- which is what
//! a Reports Analysis Division analyst would flag. See
//! `tests/fixtures/ORACLE_NOTES.md` for each.
//!
//! Three fixtures are state-party reports with Schedules H2-H4 (Georgia
//! Republican Party, Republican Party of Virginia, New Hampshire
//! Democratic Party), added so the allocation lines 18(a), 21(a)(i),
//! 21(a)(ii), and 30(b) are tested against real data rather than the spec
//! text alone.

use std::path::Path;

use hardmoney::parser::reconcile::{ChainError, Column, PeriodBasis, Relation, ReportChain};
use hardmoney::{Filing, Table};

fn fixtures() -> Vec<(String, Filing)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut out = Vec::new();
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".fec"))
        .collect();
    names.sort();
    for name in names {
        let bytes = std::fs::read(dir.join(&name)).unwrap();
        let filing = Filing::parse_bytes(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        out.push((name, filing));
    }
    out
}

fn fixture(name: &str) -> Filing {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    Filing::parse_bytes(&std::fs::read(&path).unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn every_accepted_periodic_report_fixture_reconciles_in_column_a() {
    let mut checked = 0;
    for (name, filing) in fixtures() {
        let Ok(r) = filing.reconcile() else {
            assert!(
                !matches!(filing.summary.table(), Table::F3X | Table::F3 | Table::F3P),
                "{name}: reconcile refused a supported form"
            );
            continue;
        };
        checked += 1;
        let bad: Vec<String> = r
            .column(Column::A)
            .filter(|c| !c.matches())
            .map(ToString::to_string)
            .collect();
        assert!(bad.is_empty(), "{name}:\n{}", bad.join("\n"));
        assert!(r.column(Column::A).count() >= 30, "{name}: too few checks");
    }
    assert!(checked >= 18, "only {checked} periodic-report fixtures");
}

#[test]
fn column_b_formulas_hold_on_accepted_fixtures() {
    for (name, filing) in fixtures() {
        let Ok(r) = filing.reconcile() else { continue };
        let bad: Vec<String> = r
            .column(Column::B)
            .filter(|c| !c.matches())
            .map(ToString::to_string)
            .collect();
        assert!(bad.is_empty(), "{name}:\n{}", bad.join("\n"));
    }
}

#[test]
fn old_spec_versions_reconcile_with_the_lines_they_have() {
    // 3.00 and 5.3 filings spell itemized individuals "SA11A1" and lack the
    // 8.x-only lines; both must still balance.
    for name in [
        "F3XA_27789_v3.fec",
        "F3XN_210000_v5.3.fec",
        "F3XN_320000_v6.1.fec",
    ] {
        let filing = fixture(name);
        let r = filing.reconcile().unwrap();
        let itemized = r
            .line(Column::A, "11(a)(i)")
            .unwrap_or_else(|| panic!("{name}"));
        assert!(
            itemized.lines_summed > 0 || itemized.expected.is_zero(),
            "{name}: {itemized}"
        );
        assert!(itemized.matches(), "{name}: {itemized}");
    }
}

/// The allocation lines are exercised by real H3/H4 records, not just
/// satisfied vacuously by zeros: every party fixture has non-zero 18(a)
/// from Schedule H3 and non-zero 21(a)(i)/(ii) from Schedule H4, summed
/// over the expected number of records, and each balances to the cent.
#[test]
fn party_fixtures_exercise_the_h_schedule_lines() {
    // (fixture, H3 records summed, H4 non-memo records summed)
    for (name, h3_lines, h4_lines) in [
        ("F3XA_2011814.fec", 3, 37),
        ("F3XN_1998773.fec", 1, 31),
        ("F3XA_2008083.fec", 13, 161),
    ] {
        let filing = fixture(name);
        let r = filing.reconcile().unwrap();
        let line = |label: &str| {
            r.line(Column::A, label)
                .unwrap_or_else(|| panic!("{name}: no line {label}"))
        };
        let a18 = line("18(a)");
        assert_eq!(a18.lines_summed, h3_lines, "{name}: {a18}");
        assert!(
            a18.expected.is_sign_positive() && !a18.expected.is_zero(),
            "{name}: {a18}"
        );
        assert!(a18.matches(), "{name}: {a18}");
        for label in ["21(a)(i)", "21(a)(ii)"] {
            let c = line(label);
            assert_eq!(c.lines_summed, h4_lines, "{name}: {c}");
            assert!(!c.expected.is_zero(), "{name}: {c}");
            assert!(c.matches(), "{name}: {c}");
        }
        // The formulas that consume them hold too.
        for label in ["18(c)", "20", "21(c)", "32", "36"] {
            assert!(line(label).matches(), "{name}: {}", line(label));
        }
    }
}

/// Schedule H3 groups repeat the transfer total on every record: summing
/// `total_amount_transferred` would overstate 18(a). The Georgia fixture's
/// three H3 records (`AD`, `DF`, `DC` shares of one and two transfers) sum
/// to exactly the cover's 18(a) on `transferred_amount`.
#[test]
fn h3_transferred_amount_not_total_is_what_balances() {
    use rust_decimal::Decimal;
    let filing = fixture("F3XA_2011814.fec");
    let h3: Vec<_> = filing.lines_for(Table::H3).collect();
    assert_eq!(h3.len(), 3);
    let sum = |field: &str| -> Decimal {
        h3.iter()
            .filter_map(|l| l.get_non_empty(field))
            .map(|v| v.parse::<Decimal>().unwrap())
            .sum()
    };
    let reported: Decimal = filing
        .summary
        .get_non_empty("col_a_transfers_from_nonfederal_h3")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(sum("transferred_amount"), reported);
    assert!(sum("total_amount_transferred") > reported);
}

/// 30(b), 100%-federal election activity, is itemized on `SB30B` only at
/// $200 and up (11 CFR 300.36(b)(2)(iv)); the Virginia fixture's cover
/// total exceeds its itemized sum by $174.19 and is an FEC-accepted report,
/// so the line is a floor and the check passes.
#[test]
fn line_30b_is_a_floor_on_real_party_data() {
    use rust_decimal_macros::dec;
    let filing = fixture("F3XN_1998773.fec");
    let r = filing.reconcile().unwrap();
    let c = r.line(Column::A, "30(b)").unwrap();
    assert_eq!(c.relation, Relation::AtLeast);
    assert_eq!(c.reported, Some(dec!(79762.86)));
    assert_eq!(c.expected, dec!(79588.67));
    assert_eq!(c.delta, dec!(174.19));
    assert_eq!(c.lines_summed, 49); // 69 SB30B records, 20 of them memos
    assert!(c.matches(), "{c}");
    assert!(r.line(Column::A, "30(c)").unwrap().matches());
}

/// The other requested shapes: an F3 with Schedule C loans and Schedule D
/// debts (lines 9 and 10 from two schedules), a joint fundraising
/// committee's F3X with `SA12` transfers in and `SB22` transfers out, and
/// an F3P at spec 8.5 with Schedule D debts, offsets, and refunds -- all
/// exact.
#[test]
fn debts_transfers_and_presidential_fixtures_balance() {
    let f3 = fixture("F3A_2004471.fec").reconcile().unwrap();
    let ten = f3.line(Column::A, "10").unwrap();
    assert_eq!(ten.lines_summed, 61, "{ten}"); // 58 SC/10 + 3 SD10
    assert!(!ten.expected.is_zero() && ten.matches(), "{ten}");

    let jfc = fixture("F3XN_1965568.fec").reconcile().unwrap();
    for (label, n) in [("12", 5), ("22", 106)] {
        let c = jfc.line(Column::A, label).unwrap();
        assert_eq!(c.lines_summed, n, "{c}");
        assert!(!c.expected.is_zero() && c.matches(), "{c}");
    }

    let f3p = fixture("F3PA_1993032.fec").reconcile().unwrap();
    assert!(f3p.balances(), "{f3p}");
    // 173 SB23 records, 31 of them memos; 17(a)(i) is 980 itemized donors.
    for (label, n) in [
        ("12", 8),
        ("17(a)(i)", 980),
        ("20(a)", 2),
        ("23", 142),
        ("28(a)", 2),
    ] {
        let c = f3p.line(Column::A, label).unwrap();
        assert_eq!(c.lines_summed, n, "{c}");
        assert!(!c.expected.is_zero(), "{c}");
    }
}

// ---------------------------------------------------------------------
// Chains: Column B and cash on hand across a committee's reports
// ---------------------------------------------------------------------

fn chain_fixture(name: &str) -> Filing {
    fixture(&format!("chain/{name}"))
}

/// The Republican Party of Minnesota's first three monthly reports of
/// 2026 (`tests/fixtures/chain/README.md`): the March report's Column B
/// equals the January + February + March schedule sums on all 27 lines,
/// and cash on hand carries forward from each report to the next.
#[test]
fn real_chain_column_b_balances_over_three_monthly_reports() {
    use rust_decimal_macros::dec;
    let jan = chain_fixture("F3XN_1948502.fec");
    let feb = chain_fixture("F3XA_2011895.fec");
    let mar = chain_fixture("F3XA_2011898.fec");
    for f in [&jan, &feb, &mar] {
        let r = f.reconcile().unwrap();
        assert!(r.balances(), "{r}");
    }

    let chain = ReportChain::new(&mar, [&feb, &jan]).unwrap();
    assert_eq!(chain.basis(), PeriodBasis::YearToDate);
    assert!(chain.gaps().is_empty(), "{:?}", chain.gaps());
    assert_eq!(chain.period_reports().len(), 3);
    let r = chain.reconcile();
    assert!(r.balances(), "{r}");
    assert_eq!(chain.column_b_checks().len(), 27);
    assert_eq!(chain.carry_forward_checks().len(), 1);

    // Exercised by real records, not satisfied at zero: itemized
    // individuals, transfers from the nonfederal account (H3), and the
    // allocated shares (H4) all sum across the three files.
    let b = |label: &str| r.line(Column::B, label).unwrap();
    let itemized = b("11(a)(i)");
    assert_eq!(itemized.reports_summed, 3);
    assert!(itemized.expected > dec!(100000), "{itemized}");
    assert!(itemized.lines_summed > 100, "{itemized}");
    for label in ["18(a)", "21(a)(i)", "21(a)(ii)", "21(b)", "12"] {
        let c = b(label);
        assert!(!c.expected.is_zero(), "{c}");
        assert!(c.matches(), "{c}");
    }
    // Cash: March's 6(b) is February's 8.
    let carry = r.line(Column::A, "6(b)").unwrap();
    assert_eq!(carry.reported, Some(dec!(63033.71)));
    assert_eq!(carry.expected, dec!(63033.71));

    // The first report of the year: Column B is its own Column A.
    let first = ReportChain::new(&jan, []).unwrap();
    let r = first.reconcile();
    assert!(r.balances(), "{r}");
    assert!(r.checks.iter().all(|c| c.reports_summed == 1));
}

/// Adding the 2025 year-end report to the chain crosses the year boundary:
/// it stays out of the 2026 Column B sums but supplies 6(a) (cash on hand
/// January 1) and the January report's 6(b).
#[test]
fn real_chain_crossing_the_year_boundary_carries_cash_only() {
    use rust_decimal_macros::dec;
    let ye_2025 = chain_fixture("F3XN_1943038.fec");
    let jan = chain_fixture("F3XN_1948502.fec");
    let feb = chain_fixture("F3XA_2011895.fec");
    let mar = chain_fixture("F3XA_2011898.fec");

    let chain = ReportChain::new(&mar, [&ye_2025, &jan, &feb]).unwrap();
    assert_eq!(chain.prior().len(), 3);
    assert_eq!(chain.period_reports().len(), 3, "2025 is not in 2026");
    assert!(chain.gaps().is_empty());
    let r = chain.reconcile();
    assert!(r.balances(), "{r}");
    assert_eq!(chain.carry_forward_checks().len(), 2);
    let jan_1 = r.line(Column::B, "6(a)").unwrap();
    assert_eq!(jan_1.reported, Some(dec!(97188.87)));
    assert_eq!(jan_1.expected, dec!(97188.87));
    // The 2026 sums are the same with or without the 2025 report.
    let without = ReportChain::new(&mar, [&jan, &feb]).unwrap().reconcile();
    assert_eq!(
        r.line(Column::B, "11(a)(i)").unwrap().expected,
        without.line(Column::B, "11(a)(i)").unwrap().expected
    );

    // January behind the year-end: both cash lines point at the same close.
    let chain = ReportChain::new(&jan, [&ye_2025]).unwrap();
    let r = chain.reconcile();
    assert!(r.balances(), "{r}");
    assert_eq!(r.line(Column::A, "6(b)").unwrap().expected, dec!(97188.87));
    assert_eq!(r.line(Column::B, "6(a)").unwrap().expected, dec!(97188.87));

    // The chain refuses an original alongside its amendment, and a report
    // from another committee.
    let jfc = fixture("F3XN_1965568.fec");
    assert!(matches!(
        ReportChain::new(&mar, [&jan, &jfc]).unwrap_err(),
        ChainError::FilerMismatch { index: 1, .. }
    ));
    let dup = chain_fixture("F3XA_2011895.fec");
    assert!(matches!(
        ReportChain::new(&mar, [&feb, &dup]).unwrap_err(),
        ChainError::Overlap { .. }
    ));
}

/// The same committee's May 2026 amendment (`tests/fixtures/rad/`) carries
/// a $200 Column A discrepancy on line 11(c). Chained behind January
/// through April it shows up again in Column B, by the same $200, and
/// every other Column B line agrees: the chain isolates the report that
/// introduced it. Without April the chain has a gap and says so.
#[test]
fn real_chain_propagates_a_column_a_discrepancy_into_column_b() {
    use rust_decimal_macros::dec;
    let jan = chain_fixture("F3XN_1948502.fec");
    let feb = chain_fixture("F3XA_2011895.fec");
    let mar = chain_fixture("F3XA_2011898.fec");
    let apr = chain_fixture("F3XA_2011901.fec");
    let may = fixture("rad/F3XA_2011912.fec");
    assert!(apr.reconcile().unwrap().balances());

    // April on its own chain balances too.
    let chain = ReportChain::new(&apr, [&jan, &feb, &mar]).unwrap();
    assert!(chain.gaps().is_empty());
    let r = chain.reconcile();
    assert!(r.balances(), "{r}");

    // May's own Column A is $200 over its schedule on 11(c) ...
    let own = may.reconcile().unwrap();
    assert_eq!(own.line(Column::A, "11(c)").unwrap().delta, dec!(200.00));
    // ... and so is its Column B, on that line only.
    let chain = ReportChain::new(&may, [&jan, &feb, &mar, &apr]).unwrap();
    assert!(chain.gaps().is_empty());
    let r = chain.reconcile();
    let off: Vec<String> = r.mismatches().map(ToString::to_string).collect();
    assert_eq!(off.len(), 1, "{}", off.join("\n"));
    let c = r.line(Column::B, "11(c)").unwrap();
    assert_eq!(c.delta, dec!(200.00));
    assert_eq!(c.reports_summed, 5);
    assert!(r.line(Column::A, "6(b)").unwrap().matches());

    // Leave April out and the chain reports the hole rather than blaming
    // the filer for it.
    let chain = ReportChain::new(&may, [&jan, &feb, &mar]).unwrap();
    let gaps = chain.gaps();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].to_string(), "2026-04-01..2026-04-30");
    let r = chain.reconcile();
    let carry = r.line(Column::A, "6(b)").unwrap();
    assert!(
        carry.rule.contains("2026-03-01..2026-03-31"),
        "{}",
        carry.rule
    );
    assert!(!carry.matches(), "May's 6(b) is April's close, not March's");
}
