//! Integration tests against real, live-downloaded FEC electronic filings
//! (see `tests/fixtures/`, all spec 8.5, sourced from the FEC's own RSS
//! feed and `docquery.fec.gov` -- see the crate README for provenance).
//!
//! These are not synthetic strings: every assertion here was checked by
//! hand against the raw bytes of the fixture file (`cat -A` / manual
//! delimiter-splitting) before being written down, so a regression here
//! means the parser disagrees with an actual filing the FEC accepted.

use std::collections::HashSet;
use std::path::Path;

use hardmoney::{Filing, Table};

fn fixture(name: &str) -> Filing {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("reading fixture {name}: {e}"));
    Filing::parse_bytes(&bytes).unwrap_or_else(|e| panic!("parsing fixture {name}: {e}"))
}

fn fixture_names() -> Vec<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".fec"))
        .collect();
    names.sort();
    names
}

/// Every fixture must parse without error, resolve to a known top-level
/// form, and (if it has any body lines) dispatch every single one of them
/// to a real format table -- i.e. zero `ParserMissing` gaps across this
/// real-world sample.
#[test]
fn every_fixture_parses_and_dispatches_cleanly() {
    let names = fixture_names();
    assert!(
        names.len() >= 17,
        "expected at least 17 real-filing fixtures, found {}",
        names.len()
    );

    for name in names {
        let filing = fixture(&name);
        assert!(!filing.version.is_empty(), "{name}: missing version");
        assert!(
            !filing.raw_form_type.is_empty(),
            "{name}: missing form type"
        );
        assert!(
            filing.is_allowed(),
            "{name}: base form type '{}' is not in the allowed top-level forms",
            filing.base_form_type
        );
        for line in &filing.lines {
            assert!(
                !line.fields.is_empty(),
                "{name}: line '{}' (table {}) parsed to zero fields",
                line.raw_form_type,
                line.table
            );
        }
    }
}

/// The original 17 real fixtures were downloaded live and are spec 8.5 --
/// pins the version-bucket regexes actually used in practice, not just
/// what the format-table CSVs theoretically support. A handful of
/// additional fixtures (named `*_v<version>.fec`) were added later,
/// sourced from `docquery.fec.gov`'s low-numbered filing IDs and from
/// `esonderegger/fecfile`'s bundled test data, specifically to cover
/// older spec versions (3.x, 5.x, 6.x) and are excluded from this
/// assertion -- see `older_spec_version_fixtures_use_the_correct_bucket`.
#[test]
fn all_fixtures_are_spec_8_5() {
    for name in fixture_names() {
        if name.contains("_v3")
            || name.contains("_v5")
            || name.contains("_v6")
            || name.contains("_v8.0")
            || name.contains("27789")
        {
            continue;
        }
        let filing = fixture(&name);
        assert_eq!(filing.version, "8.5", "{name}: expected spec 8.5");
    }
}

/// Real filings from FEC spec eras before 8.x, covering all three
/// historical delimiter/version regimes: version 3.00 (comma-delimited,
/// double-quoted CSV, predates the ASCII-28 format entirely -- sourced
/// from `esonderegger/fecfile`'s bundled `27789.fec`, a real 2001 MERCK
/// PAC amendment), version 5.1/5.3 (comma-delimited, no quoting, from
/// `docquery.fec.gov` filing IDs 150000/210000), and version 6.1
/// (ASCII-28 delimited "new" format, from `docquery.fec.gov` filing ID
/// 320000). Confirms `header::parse`'s old/new electronic-header regexes
/// and each format table's version-bucket regex (`^8...`/`^5...`/`^3`)
/// select the correct bucket -- and therefore the correct field
/// positions -- for eras far outside the 8.5-only fixtures above.
#[test]
fn older_spec_version_fixtures_use_the_correct_bucket() {
    let v3 = fixture("F3XA_27789_v3.fec");
    assert_eq!(v3.version, "3.00");
    assert_eq!(v3.base_form_type, "F3X");
    assert!(v3.is_amendment);
    assert_eq!(v3.summary.get("col_a_total_receipts"), Some("180046.52"));
    assert_eq!(
        v3.summary.get("col_a_total_contributions_refunds"),
        Some("538.36")
    );
    assert!(
        !v3.lines.is_empty(),
        "v3 filing should have real Schedule A/B body lines"
    );
    assert!(v3.lines.iter().any(|l| l.table == Table::SchA));
    assert!(v3.lines.iter().any(|l| l.table == Table::SchB));

    let v5_1 = fixture("F6N_150000_v5.1.fec");
    assert_eq!(v5_1.version, "5.1");
    assert_eq!(v5_1.base_form_type, "F6");

    let v5_3 = fixture("F3XN_210000_v5.3.fec");
    assert_eq!(v5_3.version, "5.3");
    assert_eq!(v5_3.base_form_type, "F3X");
    assert!(v5_3.summary.fields.contains_key("col_a_total_receipts"));

    let v6_1 = fixture("F3XN_320000_v6.1.fec");
    assert_eq!(v6_1.version, "6.1");
    assert_eq!(v6_1.base_form_type, "F3X");
    assert!(v6_1.summary.fields.contains_key("col_a_total_receipts"));
    assert!(
        v6_1.summary
            .fields
            .contains_key("col_a_federal_election_activity_total")
    );
}

/// `F3XN_2011835.fec`'s header has a real stray trailing space on the
/// version field ("8.5 "). Confirms `clean_entry`'s trim survives the
/// round trip from raw bytes through `Filing::parse_bytes`, matching the
/// synthetic case already covered in `header.rs`'s unit tests.
#[test]
fn trailing_space_in_real_header_version_is_trimmed() {
    let filing = fixture("F3XN_2011835.fec");
    assert_eq!(filing.version, "8.5");
}

#[test]
fn f24n_dispatches_to_schedule_e_with_correct_fields() {
    let filing = fixture("F24N_2011823.fec");
    assert_eq!(filing.raw_form_type, "F24N");
    assert_eq!(filing.base_form_type, "F24");
    assert!(!filing.is_amendment);
    assert_eq!(filing.summary.get("committee_name"), Some("OHIO FLYER PAC"));

    assert_eq!(filing.lines.len(), 1);
    let se = &filing.lines[0];
    assert_eq!(se.table, Table::SchE);
    assert_eq!(
        se.get("payee_organization_name"),
        Some("STRATEGIC MEDIA PLACEMENT INC.")
    );
    assert_eq!(se.get("expenditure_amount"), Some("1074900.00"));
    assert_eq!(se.get("candidate_last_name"), Some("BROWN"));
    assert_eq!(se.get("candidate_first_name"), Some("SHERROD"));
    assert_eq!(se.get("candidate_state"), Some("OH"));
}

#[test]
fn f24n_second_sample_has_two_schedule_e_lines() {
    let filing = fixture("F24N_2011832.fec");
    assert_eq!(filing.lines.len(), 2);
    assert!(filing.lines.iter().all(|l| l.table == Table::SchE));
    assert_eq!(
        filing.lines[0].get("payee_organization_name"),
        Some("DECLARATION MEDIA LLC")
    );
    assert_eq!(
        filing.lines[1].get("payee_organization_name"),
        Some("MVAR MEDIA, LLC")
    );
}

/// Structurally the richest fixture: a 132KB real Form 3A amendment with
/// Schedule A (two sub-line variants), Schedule B, and a v6.4-legacy
/// `SC/10` debt line that must still dispatch to `SchC` (the exact case
/// pyfec's own `get_line_parser` docstring calls out).
#[test]
fn f3a_large_filing_covers_diverse_schedules() {
    let filing = fixture("F3A_2011812.fec");
    assert_eq!(filing.base_form_type, "F3");
    assert!(filing.is_amendment);
    assert_eq!(filing.amends_filing.as_deref(), Some("1997089"));

    let tables: HashSet<Table> = filing.lines.iter().map(|l| l.table).collect();
    assert!(
        tables.contains(&Table::SchA),
        "expected at least one Schedule A line"
    );
    assert!(
        tables.contains(&Table::SchB),
        "expected at least one Schedule B line"
    );
    assert!(
        tables.contains(&Table::SchC),
        "expected the legacy SC/10 line to dispatch to SchC"
    );

    let sc10 = filing
        .lines
        .iter()
        .find(|l| l.raw_form_type == "SC/10")
        .expect("SC/10 line present");
    assert_eq!(sc10.table, Table::SchC);

    // Real SA11AI and SA11D lines are both present and both dispatch to
    // the same SchA table (they're sub-line-number variants, not distinct
    // schedules).
    assert!(filing.lines.iter().any(|l| l.raw_form_type == "SA11AI"));
    assert!(filing.lines.iter().any(|l| l.raw_form_type == "SA11D"));
    assert!(
        filing
            .lines
            .iter()
            .filter(|l| l.raw_form_type == "SA11AI")
            .all(|l| l.table == Table::SchA)
    );
    assert!(
        filing
            .lines
            .iter()
            .filter(|l| l.raw_form_type == "SA11D")
            .all(|l| l.table == Table::SchA)
    );
}

#[test]
fn f3a_second_sample_has_schedule_d_debt_lines() {
    let filing = fixture("F3A_2011822.fec");
    let tables: HashSet<Table> = filing.lines.iter().map(|l| l.table).collect();
    assert!(tables.contains(&Table::SchA));
    assert!(tables.contains(&Table::SchB));
    assert!(
        tables.contains(&Table::SchC),
        "SC/10 line should dispatch to SchC"
    );
    assert!(
        tables.contains(&Table::SchD),
        "SD10 lines should dispatch to SchD"
    );
}

/// `F3XA_2011821.fec` is the other structurally rich fixture: Schedule A,
/// Schedule B (memo variant `SB21B`), Schedule D debts, Schedule E
/// independent expenditures, and three delimited `TEXT` records with
/// `back_reference_tran_id_number` pointers -- distinct from (and not to be
/// confused with) the bracketed `[BEGINTEXT]` block format used by F99.
#[test]
fn f3xa_covers_schedule_e_and_delimited_text_records() {
    let filing = fixture("F3XA_2011821.fec");
    assert_eq!(filing.base_form_type, "F3X");
    assert!(filing.is_amendment);
    assert_eq!(filing.amends_filing.as_deref(), Some("1996410"));

    let tables: HashSet<Table> = filing.lines.iter().map(|l| l.table).collect();
    for expected in [
        Table::SchA,
        Table::SchB,
        Table::SchD,
        Table::SchE,
        Table::Text,
    ] {
        assert!(
            tables.contains(&expected),
            "expected table '{expected}' among: {tables:?}"
        );
    }

    let text_lines: Vec<_> = filing
        .lines
        .iter()
        .filter(|l| l.table == Table::Text)
        .collect();
    assert_eq!(text_lines.len(), 3);
    assert!(
        text_lines[0]
            .get("back_reference_tran_id_number")
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    );
    assert!(text_lines[0].get("text").unwrap().contains("EARMARKED"));
}

#[test]
fn f3xa_second_sample_is_amendment_with_schedule_e() {
    let filing = fixture("F3XA_2011827.fec");
    assert!(filing.is_amendment);
    assert_eq!(filing.amends_filing.as_deref(), Some("1991972"));
    assert!(filing.lines.iter().any(|l| l.table == Table::SchE));
}

#[test]
fn f3xn_large_filing_is_not_an_amendment() {
    let filing = fixture("F3XN_2011831.fec");
    assert!(!filing.is_amendment);
    assert!(filing.amends_filing.is_none());
    // The raw bytes contain a lowercase "SB21b" line-type token; clean_entry
    // uppercases every field (including the form-type column) before
    // dispatch, so by the time it's a ParsedLine its raw_form_type reads
    // "SB21B" -- but the *dispatch itself* must still have matched
    // case-insensitively against the original lowercase bytes, which this
    // assertion (indirectly, via the line surviving to become a ParsedLine
    // at all) confirms.
    let line = filing
        .lines
        .iter()
        .find(|l| l.raw_form_type == "SB21B")
        .expect("SB21B line present");
    assert_eq!(line.table, Table::SchB);
}

#[test]
fn f5n_dispatches_f57_subform_lines() {
    let filing = fixture("F5N_2011649.fec");
    assert_eq!(filing.base_form_type, "F5");
    assert!(!filing.lines.is_empty());
    assert!(filing.lines.iter().all(|l| l.table == Table::F57));
}

#[test]
fn f6n_dispatches_f65_subform_line() {
    for name in ["F6N_2011694.fec", "F6N_2011723.fec"] {
        let filing = fixture(name);
        assert_eq!(filing.base_form_type, "F6");
        assert_eq!(filing.lines.len(), 1);
        assert_eq!(filing.lines[0].table, Table::F65);
    }
}

#[test]
fn f3n_dispatches_delimited_text_record() {
    let filing = fixture("F3N_2011557.fec");
    assert_eq!(filing.base_form_type, "F3");
    assert_eq!(filing.lines.len(), 1);
    assert_eq!(filing.lines[0].table, Table::Text);
}

/// The headline real-data discovery this session: F99's free text is
/// carried in a `[BEGINTEXT]`/`[ENDTEXT]` block, not in the delimited
/// `text` column (which is left blank) -- a convention pyfec never
/// implemented at all. Confirms both single-line and real multi-paragraph
/// (with blank lines) letters round-trip with case preserved.
#[test]
fn f99_single_paragraph_begintext_block_is_recovered() {
    let filing = fixture("F99_2011828.fec");
    assert_eq!(filing.base_form_type, "F99");
    assert!(
        filing.lines.is_empty(),
        "F99's only content is the summary line + free text block"
    );

    // The delimited `text` column (position 18 in the 8.5 bucket) is blank
    // in the raw bytes; Filing::parse must splice the BEGINTEXT block's
    // content into it rather than leaving it empty.
    let text = filing
        .summary
        .get("text")
        .expect("text field recovered from BEGINTEXT block");
    assert!(text.starts_with("The independent expenditure was timely and correctly filed"));
    assert!(text.ends_with("This amended report corrects that error."));

    assert_eq!(filing.summary.get("committee_name"), Some("REVIVE OREGON"));
    assert_eq!(filing.summary.get("treasurer_last_name"), Some("MARSTON"));
}

#[test]
fn f99_multi_paragraph_begintext_block_preserves_line_structure_and_case() {
    let filing = fixture("F99_2011833.fec");
    let text = filing
        .summary
        .get("text")
        .expect("text field recovered from BEGINTEXT block");

    // Real multi-paragraph letter: must preserve blank lines between
    // paragraphs and must NOT be uppercased like ordinary delimited fields
    // (clean_entry uppercases; free text must bypass that).
    assert!(
        text.contains("Michael Dobi"),
        "mixed case must survive: {text}"
    );
    assert!(
        text.contains("\n\n"),
        "blank line between paragraphs must survive: {text}"
    );
    assert!(text.starts_with("September 14, 2026"));
    assert!(text.trim_end().ends_with("C00466482"));

    // Ordinary delimited fields (unlike the free-text block) go through
    // clean_entry, which uppercases -- the real filing's own casing was
    // "Families for James Lankford".
    assert_eq!(
        filing.summary.get("committee_name"),
        Some("FAMILIES FOR JAMES LANKFORD")
    );
}

#[test]
fn f3t_termination_report_is_not_flagged_as_amendment() {
    let filing = fixture("F3T_2011720.fec");
    assert_eq!(filing.base_form_type, "F3");
    assert!(!filing.is_amendment);
    assert!(!filing.lines.is_empty());
}

#[test]
fn f3xt_termination_report_dispatches_schedule_b() {
    let filing = fixture("F3XT_2011632.fec");
    assert_eq!(filing.base_form_type, "F3X");
    assert!(!filing.is_amendment);
    assert!(filing.lines.iter().all(|l| l.table == Table::SchB));
}

// ---------------------------------------------------------------------------
// Form families that were unparseable before the dispatch table covered
// them. Before these entries existed, every Form 1/1M/2 filing on the FEC's
// live feed (~26% of daily volume) failed with `ParserMissing`, and an F3
// containing a single F3Z line lost all of its body lines.
// ---------------------------------------------------------------------------

#[test]
fn form_1_registration_with_f1s_continuation_parses() {
    let filing = fixture("F1A_2011905.fec");
    assert_eq!(filing.version, "8.5");
    assert_eq!(filing.raw_form_type, "F1A");
    assert_eq!(filing.base_form_type, "F1");
    assert!(filing.is_amendment);
    assert!(filing.is_allowed());
    assert_eq!(filing.summary.table, Table::F1);
    assert_eq!(
        filing.summary.get("committee_name"),
        Some("INDEPENDENCE BLUE CROSS LLC PAC")
    );
    // The F1S continuation line (additional joint fundraising participants
    // / affiliated committees) dispatches to its own table.
    assert_eq!(filing.lines.len(), 1);
    assert_eq!(filing.lines[0].table, Table::F1S);
    assert_eq!(filing.lines[0].line_no, 3);
}

#[test]
fn form_1m_multicandidate_notification_parses() {
    let filing = fixture("F1MN_2011755.fec");
    assert_eq!(filing.raw_form_type, "F1MN");
    assert_eq!(filing.base_form_type, "F1M");
    assert_eq!(filing.summary.table, Table::F1M);
    assert!(filing.lines.is_empty());
    assert!(
        filing
            .summary
            .get("committee_name")
            .is_some_and(|n| n.contains("KENTUCKY"))
    );
}

#[test]
fn form_2_candidate_registration_with_f2s_authorized_committees_parses() {
    let filing = fixture("F2A_2011896.fec");
    assert_eq!(filing.raw_form_type, "F2A");
    assert_eq!(filing.base_form_type, "F2");
    assert_eq!(filing.summary.table, Table::F2);
    assert_eq!(filing.summary.get("candidate_id_number"), Some("S6IL00458"));
    // Three F2S lines, each naming an authorized committee. F2S has no
    // fech-sources table; ours is authored locally.
    let f2s: Vec<_> = filing.lines_for(Table::F2S).collect();
    assert_eq!(f2s.len(), 3);
    for line in &f2s {
        assert_eq!(line.get("filer_candidate_id_number"), Some("S6IL00458"));
        assert!(
            line.get("authorized_committee_id_number")
                .is_some_and(|id| id.starts_with('C') && id.len() == 9),
            "{:?}",
            line.get("authorized_committee_id_number")
        );
        assert!(
            line.get("authorized_committee_name")
                .is_some_and(|n| !n.is_empty())
        );
    }
}

#[test]
fn form_3_with_f3z_consolidated_lines_keeps_every_body_line() {
    let filing = fixture("F3A_767339_v8.0.fec");
    assert_eq!(filing.version, "8.0");
    assert_eq!(filing.base_form_type, "F3");
    // 524 SA11AI + 52 SA11C + 58 SB17 + 2 SB20A + 2 SD10 + 2 F3Z + 1 F3ZT
    assert_eq!(filing.lines.len(), 641);
    assert_eq!(filing.lines_for(Table::F3Z).count(), 3);
    assert_eq!(filing.lines_for(Table::SchA).count(), 576);
    assert_eq!(filing.lines_for(Table::SchB).count(), 60);
    assert_eq!(filing.lines_for(Table::SchD).count(), 2);
    let f3zt = filing
        .lines
        .iter()
        .find(|l| l.raw_form_type == "F3ZT")
        .expect("consolidated F3ZT line present");
    assert_eq!(f3zt.table, Table::F3Z);
    assert_eq!(
        f3zt.get("principal_committee_name"),
        Some("DJOU FOR HAWAII")
    );
    assert_eq!(f3zt.get("filer_committee_id_number"), Some("C00441451"));
}

#[test]
fn every_fixture_parses_identically_in_lenient_mode() {
    // For well-formed filings lenient parsing must be a no-op: same lines,
    // nothing skipped. This pins that `ParseOptions::LENIENT` never
    // *changes* what parses, only what happens on failure.
    use hardmoney::ParseOptions;
    for name in fixture_names() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(&name);
        let bytes = std::fs::read(&path).unwrap();
        let strict = Filing::parse_bytes(&bytes).unwrap();
        let (lenient, skipped) = Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT)
            .unwrap()
            .into_parts();
        assert!(skipped.is_empty(), "{name}: {skipped:?}");
        assert_eq!(strict.lines.len(), lenient.lines.len(), "{name}");
        assert_eq!(strict.summary, lenient.summary, "{name}");
    }
}

#[test]
fn typed_views_on_real_filings_respect_tables() {
    use hardmoney::{ScheduleA, ScheduleE};
    // F3A_2011812 has 249 Schedule A lines and zero Schedule E lines. A
    // table-blind ScheduleE conversion used to "succeed" on all 587 lines.
    let filing = fixture("F3A_2011812.fec");
    assert_eq!(filing.lines_for(Table::SchE).count(), 0);
    assert_eq!(filing.views::<ScheduleE>().count(), 0);
    let sched_a = filing.views::<ScheduleA>().count();
    assert_eq!(sched_a, filing.lines_for(Table::SchA).count());
    assert!(sched_a > 200);
    // And the wrong view on a real line is a typed refusal, not a phantom.
    let first_a = filing.lines_for(Table::SchA).next().unwrap();
    assert!(matches!(
        first_a.view::<ScheduleE>(),
        Err(hardmoney::TypedViewError::WrongTable { .. })
    ));
}
