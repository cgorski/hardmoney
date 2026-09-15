//! Integrity checks over the build-time-generated format tables.
//!
//! The two collision classes that once silently dropped data -- one
//! canonical name at two columns, or two canonical names at one column,
//! within a single version bucket -- are now rejected by `build.rs` itself
//! (the crate does not compile if a bundled CSV has either), so this file
//! checks the *shape* of what the generator produced: every table has
//! layouts, every layout is internally consistent, every field constant
//! names a real field, and the fields renamed to fix upstream collisions
//! (see `NOTICE`) exist under their new names.

use hardmoney::parser::tables::{f3x, sch_a, sch_c1};
use hardmoney::{SpecVersion, Table};
use strum::IntoEnumIterator;

#[test]
fn every_table_is_in_all_and_round_trips_its_name() {
    let iter: Vec<Table> = Table::iter().collect();
    assert_eq!(iter, Table::ALL.to_vec());
    for &t in Table::ALL {
        assert_eq!(t.as_str().parse::<Table>().ok(), Some(t), "{t}");
        assert_eq!(t.to_string(), t.as_str());
        assert_eq!(
            t.as_str().to_lowercase().parse::<Table>().ok(),
            Some(t),
            "case-insensitive {t}"
        );
    }
    assert!("ZZZ".parse::<Table>().is_err());
}

#[test]
fn layouts_are_consistent_and_version_sets_do_not_overlap() {
    for &t in Table::ALL {
        let layouts = t.layouts();
        assert!(!layouts.is_empty(), "{t} has no layouts");
        for (i, l) in layouts.iter().enumerate() {
            assert_eq!(l.table, t);
            assert!(!l.fields.is_empty(), "{t} layout {i} has no fields");
            // Distinct columns, distinct names, all below width.
            let mut cols: Vec<u16> = l.fields.iter().map(|f| f.column).collect();
            cols.sort_unstable();
            cols.dedup();
            assert_eq!(
                cols.len(),
                l.fields.len(),
                "{t} layout {i}: column collision"
            );
            assert!(
                cols.iter().all(|&c| c < l.width),
                "{t} layout {i}: column >= width"
            );
            for f in l.fields {
                assert_eq!(
                    l.field(f.name).map(|d| d.column),
                    Some(f.column),
                    "{t}.{}",
                    f.name
                );
            }
            // A version is claimed by at most one layout of a table.
            for other in &layouts[i + 1..] {
                for v in l.versions {
                    assert!(!other.supports(*v), "{t}: version {v} in two layouts");
                }
            }
        }
        for name in t.field_names() {
            assert!(
                layouts.iter().any(|l| l.field(name).is_some()),
                "{t}.{name} is in FIELD_NAMES but in no layout"
            );
        }
    }
}

#[test]
fn field_constants_are_real_fields_of_their_table() {
    let v85 = SpecVersion::electronic(8, 5);
    let f3x = Table::F3X.layout(v85).expect("F3X 8.5 layout");
    assert!(f3x.field(f3x::COL_A_TOTAL_RECEIPTS.name()).is_some());
    assert!(f3x.field(f3x::COL_B_CASH_ON_HAND_JAN_1.name()).is_some());
    let sch_a = Table::SchA.layout(v85).expect("SchA 8.5 layout");
    assert!(sch_a.field(sch_a::CONTRIBUTION_AGGREGATE.name()).is_some());
    assert_eq!(sch_a::CONTRIBUTION_AGGREGATE.table(), Table::SchA);
}

#[test]
fn spec_rows_agree_with_current_layouts() {
    let v85 = SpecVersion::electronic(8, 5);
    let mut tables_with_specs = 0;
    for &t in Table::ALL {
        let specs = t.specs();
        if specs.is_empty() {
            continue;
        }
        tables_with_specs += 1;
        // F3PZ1/F3PZ2 are still documented in the FEC's workbook but were
        // dropped from the 8.5 format ("no longer needed"), so they have
        // spec rows and no current layout.
        let Some(layout) = t.layout(v85) else {
            assert!(t.specs().iter().all(|s| s.canonical.is_none()), "{t}");
            continue;
        };
        for s in specs {
            if let Some(name) = s.canonical {
                let def = layout.field(name).unwrap_or_else(|| panic!("{t}.{name}"));
                assert_eq!(
                    def.column, s.column,
                    "{t}.{name} spec/layout column disagree"
                );
            }
        }
        // Nearly every current-layout field should have a spec row.
        let unspecified: Vec<&str> = layout
            .fields
            .iter()
            .filter(|f| t.spec(f.name).is_none())
            .map(|f| f.name)
            .collect();
        assert!(
            unspecified.len() * 10 <= layout.fields.len() + 9,
            "{t}: {} of {} 8.5 fields have no spec row: {unspecified:?}",
            unspecified.len(),
            layout.fields.len()
        );
    }
    assert!(
        tables_with_specs >= 45,
        "only {tables_with_specs} tables carry spec rows"
    );
}

#[test]
fn renamed_fields_use_their_new_distinct_canonical_names() {
    let v85 = SpecVersion::electronic(8, 5);
    let expectations: &[(Table, &[&str])] = &[
        (
            Table::F3X,
            &[
                "col_a_total_receipts",
                "col_a_total_receipts_recap",
                "col_a_total_disbursements",
                "col_a_total_disbursements_recap",
                "col_a_total_contributions",
                "col_a_total_contributions_recap",
                "col_a_refunds_of_federal_contributions",
                "col_a_total_contributions_refunds",
                "col_b_total_receipts",
                "col_b_total_receipts_recap",
            ],
        ),
        (
            Table::F3P,
            &["col_a_total_receipts", "col_a_total_receipts_recap"],
        ),
        (
            Table::F4,
            &["col_a_total_receipts", "col_a_total_receipts_recap"],
        ),
        (Table::F2, &["candidate_state", "candidate_office_state"]),
        (
            Table::SchC1,
            &[
                sch_c1::COLLATERAL_DESCRIPTION.name(),
                sch_c1::FUTURE_INCOME_DESCRIPTION.name(),
                sch_c1::TREASURER_DATE_SIGNED.name(),
                sch_c1::AUTHORIZED_DATE_SIGNED.name(),
            ],
        ),
        (
            Table::SchL,
            &[
                "col_a_disbursements_period",
                "col_a_cash_on_hand_close_of_period",
                "col_b_disbursements_period",
                "col_b_cash_on_hand_close_of_period",
            ],
        ),
    ];

    for (table, names) in expectations {
        let layout = table
            .layout(v85)
            .unwrap_or_else(|| panic!("no 8.5 layout for {table}"));
        for name in *names {
            assert!(
                layout.field(name).is_some(),
                "{table} 8.5 is missing field '{name}'"
            );
        }
    }
}

/// The upstream data defects fixed in 2.0 (see `NOTICE`): fields that were
/// unreadable in the affected versions must now resolve to the FEC's
/// documented columns.
#[test]
fn upstream_position_fixes_are_in_effect() {
    let col = |t: Table, v: SpecVersion, f: &str| {
        t.layout(v)
            .and_then(|l| l.field(f))
            .map(|d| d.column + 1)
            .unwrap_or_else(|| panic!("{t} {v} {f}"))
    };
    // F3 6.1-6.3: election_date collided with report_code at 12.
    assert_eq!(
        col(Table::F3, SpecVersion::electronic(6, 2), "election_date"),
        14
    );
    assert_eq!(
        col(Table::F3, SpecVersion::electronic(6, 2), "report_code"),
        12
    );
    // F3X 5.x: an unquoted comma in the 6(a) label shifted the cell.
    assert_eq!(
        col(
            Table::F3X,
            SpecVersion::electronic(5, 3),
            "col_b_cash_on_hand_jan_1"
        ),
        62
    );
    assert_eq!(
        col(Table::F3X, SpecVersion::electronic(5, 3), "col_b_year"),
        63
    );
    // F3S 5.x: three fields had label text where their positions should be.
    assert_eq!(
        col(
            Table::F3S,
            SpecVersion::electronic(5, 3),
            "19_b_loan_repayments_all_other_loans"
        ),
        26
    );
    assert_eq!(
        col(
            Table::F3S,
            SpecVersion::electronic(5, 3),
            "20_b_refund_political_party_committees"
        ),
        29
    );
    assert_eq!(
        col(
            Table::F3S,
            SpecVersion::electronic(5, 3),
            "20_c_refund_other_political_committees"
        ),
        30
    );
    // F5 5.3: occupation collided with coverage_through_date at 18.
    assert_eq!(
        col(
            Table::F5,
            SpecVersion::electronic(5, 3),
            "individual_occupation"
        ),
        12
    );
    // F57 3.x: street 2 collided with street 1 at 5.
    assert_eq!(
        col(Table::F57, SpecVersion::electronic(3, 0), "payee_street_2"),
        6
    );
}
