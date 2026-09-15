//! `Filing::reconcile` against real filings.
//!
//! Every Form 3X / 3 / 3P fixture in `tests/fixtures/` was accepted by the
//! FEC; the cover pages of all of them satisfy every Column A rule (the
//! itemization-threshold lines as floors, everything else exactly). Across
//! the wider local corpus of 109 real reports, 95 satisfy every rule and
//! the rest carry genuine filer discrepancies -- e.g. a committee whose
//! reported 11(c) exceeds its itemized SA11C lines by exactly one $200
//! contribution -- which is what a Reports Analysis Division analyst would
//! flag.

use std::path::Path;

use hardmoney::parser::reconcile::Column;
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
    assert!(checked >= 12, "only {checked} periodic-report fixtures");
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
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let filing = Filing::parse_bytes(&std::fs::read(path).unwrap()).unwrap();
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
