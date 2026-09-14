//! Regression tests guarding the fec-csv-sources format tables against the
//! class of bug found and fixed in this project: a format table CSV
//! assigning the same canonical field name to two different column
//! positions within the same FEC spec version bucket. Before the fix,
//! `IndexMap::insert` in `src/parser/line.rs` silently kept only the later
//! position, so real filings quietly lost data for the earlier field (this
//! was confirmed against real F3XN filings: `col_b` "line 19 Total
//! Receipts" and "line 6(c) Total Receipts" legitimately differ on some
//! real filings, e.g. a committee that reports a coarse period total but
//! hasn't itemized any of the categories that roll up into the line-19
//! recap).
//!
//! Six format tables were affected: F2 (candidate_state), F3P, F3X, F4
//! (col_a/col_b total_receipts & total_disbursements recap lines), SchC1
//! (description, date_signed), and SchL (a column mislabeled col_b when it
//! belongs to col_a's own per-period recap). All were fixed by giving the
//! colliding field its own distinct, form-accurate canonical name.
//!
//! `Line::from_csv_str` now returns `FecError::DuplicateCanonicalField`
//! instead of silently overwriting, so any *new* collision -- whether from
//! a future upstream fech-sources update or a local edit mistake -- fails
//! loudly here instead of silently dropping data in production.

use hardmoney::parser::format_data::FORM_CSV_DATA;
use hardmoney::parser::line::Line;

#[test]
fn every_bundled_format_table_parses_without_canonical_field_collisions() {
    let mut failures = Vec::new();
    for (form, csv) in FORM_CSV_DATA {
        if let Err(e) = Line::from_csv_str(form, csv) {
            failures.push(format!("{form}: {e}"));
        }
    }
    assert!(
        failures.is_empty(),
        "found canonical field collisions in bundled format tables:\n{}",
        failures.join("\n")
    );
}

#[test]
fn detects_a_synthetic_collision() {
    // Same canonical name ("dup") assigned to two different positions (2
    // and 3) within the single "^8" version bucket -- must be rejected,
    // not silently resolved to whichever row came last.
    let csv = "canonical,^8\ndup,2,FIRST\ndup,3,SECOND\n";
    match Line::from_csv_str("SYNTH", csv) {
        Ok(_) => panic!("expected a DuplicateCanonicalField error, but parsing succeeded"),
        Err(e) => assert!(
            matches!(e, hardmoney::FecError::DuplicateCanonicalField { .. }),
            "expected DuplicateCanonicalField, got: {e}"
        ),
    }
}

#[test]
fn allows_the_same_canonical_name_repeated_with_an_identical_position() {
    // Not every repeat is a bug: a handful of rows in the real tables
    // legitimately restate the same canonical name at the same position
    // across cosmetic label variants. That must keep working.
    let csv = "canonical,^8\nsame,2,LABEL A\nsame,2,LABEL A (cosmetic variant)\n";
    let line = Line::from_csv_str("SYNTH", csv).expect("identical-position repeat must not error");
    let cols = line.column_locations("8").unwrap();
    assert_eq!(cols.get("same"), Some(&1));
}

/// Every previously-colliding canonical name must now resolve to exactly
/// the distinct, form-accurate name chosen when the bug was fixed, so a
/// careless future edit that reverts one of these renames is caught here
/// even if it doesn't happen to reintroduce a same-bucket collision.
#[test]
fn renamed_fields_use_their_new_distinct_canonical_names() {
    let expectations: &[(&str, &str, &[&str])] = &[
        (
            "F3X",
            "8.5",
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
        ("F3P", "8.5", &["col_a_total_receipts", "col_a_total_receipts_recap"]),
        ("F4", "8.5", &["col_a_total_receipts", "col_a_total_receipts_recap"]),
        ("F2", "8.5", &["candidate_state", "candidate_office_state"]),
        (
            "SchC1",
            "8.5",
            &[
                "collateral_description",
                "future_income_description",
                "treasurer_date_signed",
                "authorized_date_signed",
            ],
        ),
        (
            "SchL",
            "8.5",
            &[
                "col_a_disbursements_period",
                "col_a_cash_on_hand_close_of_period",
                "col_b_disbursements_period",
                "col_b_cash_on_hand_close_of_period",
            ],
        ),
    ];

    for (form, version, names) in expectations {
        let csv = FORM_CSV_DATA
            .iter()
            .find(|(name, _)| name == form)
            .unwrap_or_else(|| panic!("no bundled format table named '{form}'"))
            .1;
        let line = Line::from_csv_str(form, csv).unwrap_or_else(|e| panic!("{form} failed to parse: {e}"));
        let cols = line
            .column_locations(version)
            .unwrap_or_else(|| panic!("no {version} bucket for {form}"));
        for name in *names {
            assert!(
                cols.contains_key(*name),
                "{form} version {version} is missing expected canonical field '{name}'"
            );
        }
    }
}
