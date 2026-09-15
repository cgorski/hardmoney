//! `Filing::validate` against real filings.
//!
//! Every file in `tests/fixtures/` was accepted by the FEC, so none may
//! produce an error-severity finding -- an error there is a bug in our rule,
//! not in the filing. Warnings are allowed (the FEC accepts filings with
//! warnings, and several of these carry real ones) but are pinned per file
//! so a change in either direction is visible.
//!
//! `tests/fixtures/invalid/` holds copies of those filings with deliberate
//! defects (see the README there); each must produce exactly the expected
//! rules.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hardmoney::parser::{Rule, Severity, Validation};
use hardmoney::{Filing, ParseOptions};

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

fn validate(path: &Path) -> Validation {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .validate()
}

fn rules_by_line(v: &Validation) -> Vec<(u64, Rule)> {
    v.iter().map(|f| (f.line_no, f.rule)).collect()
}

/// Every real, FEC-accepted fixture validates with zero errors.
#[test]
fn accepted_filings_have_no_errors() {
    let dir = fixtures_dir();
    let names = fixture_names(&dir);
    assert!(
        names.len() >= 31,
        "expected the 31 real fixtures, found {}",
        names.len()
    );
    for name in &names {
        let v = validate(&dir.join(name));
        let errors: Vec<String> = v.errors().map(ToString::to_string).collect();
        assert!(
            errors.is_empty(),
            "{name}: {} error finding(s) on an FEC-accepted filing:\n{}",
            errors.len(),
            errors.join("\n")
        );
        assert!(v.is_acceptable());
        for f in &v {
            assert_eq!(f.severity, Severity::Warning, "{name}: {f}");
            assert!(f.line_no >= 1, "{name}: {f}");
            assert!(!f.form_type.is_empty(), "{name}: {f}");
            assert!(!f.message.is_empty(), "{name}: {f}");
        }
    }
}

/// The warnings the accepted fixtures carry, pinned exactly. Every current
/// (8.5) fixture except the ones listed is completely clean.
#[test]
fn accepted_filings_warnings_are_pinned() {
    let expected: BTreeMap<&str, Vec<(Rule, usize)>> = BTreeMap::from([
        // 2002, spec 3.00: superseded format; 63 one-digit candidate
        // districts (FEC failing #39 today, demoted on the old format); six
        // Schedule B lines with a blank entity type, which 3.x allowed.
        (
            "F3XA_27789_v3.fec",
            vec![
                (Rule::CurrentFormat, 1),
                (Rule::InvalidDistrict, 63),
                (Rule::RequiredFieldEmpty, 6),
            ],
        ),
        // 2026 House amendment with Schedule C loans and Schedule D debts:
        // one payee with a blank street and a four-digit ZIP (`1016`).
        (
            "F3A_2004471.fec",
            vec![(Rule::RecommendedFieldEmpty, 1), (Rule::InvalidZipCode, 1)],
        ),
        // 2026 presidential amendment (spec 8.5): sixteen blank payee
        // address parts and one four-digit ZIP.
        (
            "F3PA_1993032.fec",
            vec![(Rule::RecommendedFieldEmpty, 16), (Rule::InvalidZipCode, 1)],
        ),
        ("F3XN_210000_v5.3.fec", vec![(Rule::CurrentFormat, 1)]),
        // Spec 6.1 Schedule B lines missing payee street/city/state/zip.
        (
            "F3XN_320000_v6.1.fec",
            vec![(Rule::CurrentFormat, 1), (Rule::RecommendedFieldEmpty, 15)],
        ),
        ("F6N_150000_v5.1.fec", vec![(Rule::CurrentFormat, 1)]),
        // Spec 8.0 F3 amendment with 14 blank contributor zips/streets/state
        // and a PAC contribution (`SA11C`) with no donor committee id.
        (
            "F3A_767339_v8.0.fec",
            vec![
                (Rule::CurrentFormat, 1),
                (Rule::RecommendedFieldEmpty, 14),
                (Rule::ConditionallyRequiredFieldEmpty, 1),
            ],
        ),
        // Georgia Republican Party, spec 8.5, Schedules H2-H4: a party
        // committee's contribution without its FEC id and a candidate
        // committee's without the candidate's id, name, or office
        // ("Used if CCM, PAC or PTY" / "Used if CAN or CCM"). WebCheck
        // reports the first three of the four.
        (
            "F3XA_2011814.fec",
            vec![(Rule::ConditionallyRequiredFieldEmpty, 4)],
        ),
    ]);

    let dir = fixtures_dir();
    let mut total_warnings = 0;
    for name in fixture_names(&dir) {
        let v = validate(&dir.join(&name));
        total_warnings += v.warning_count();
        let mut counts: Vec<(Rule, usize)> = v.counts_by_rule().into_iter().collect();
        counts.sort_by_key(|(r, _)| r.to_string());
        let mut want = expected.get(name.as_str()).cloned().unwrap_or_default();
        want.sort_by_key(|(r, _)| r.to_string());
        assert_eq!(counts, want, "{name}:\n{v}");
    }
    assert_eq!(total_warnings, 127);
}

/// The party-committee fixtures carry Schedules H2-H4 and 100%-federal
/// election activity. Their `NUM-5` allocation percentages on H2 (`0.49`)
/// are numeric, not `non_numeric` (a false positive an earlier rule
/// produced on every accepted state-party report), and two of the three
/// are completely clean -- as WebCheck also says.
#[test]
fn party_fixtures_with_allocation_schedules_are_clean() {
    for name in ["F3XN_1998773.fec", "F3XA_2008083.fec"] {
        let v = validate(&fixtures_dir().join(name));
        assert!(v.is_empty(), "{name}:\n{v}");
    }
    let v = validate(&fixtures_dir().join("F3XA_2011814.fec"));
    assert!(v.is_acceptable(), "{v}");
    assert!(
        v.iter()
            .all(|f| f.rule == Rule::ConditionallyRequiredFieldEmpty),
        "{v}"
    );
}

/// A superseded-format filing's blank required fields are warnings, not
/// errors (the FEC accepted them), and carry the demoted severity on the
/// finding itself.
#[test]
fn superseded_format_required_fields_are_demoted() {
    let v = validate(&fixtures_dir().join("F3XA_27789_v3.fec"));
    let demoted: Vec<_> = v
        .iter()
        .filter(|f| f.rule == Rule::RequiredFieldEmpty)
        .collect();
    assert_eq!(demoted.len(), 6);
    for f in demoted {
        assert_eq!(f.severity, Severity::Warning, "{f}");
        assert_eq!(f.rule.severity(), Severity::Error);
        assert_eq!(f.field, Some("entity_type"));
        assert_eq!(f.form_type, "SB21B");
    }
}

// ---------------------------------------------------------------------------
// Deliberately broken copies
// ---------------------------------------------------------------------------

fn invalid(name: &str) -> Validation {
    validate(&fixtures_dir().join("invalid").join(name))
}

#[test]
fn duplicate_transaction_id() {
    let v = invalid("duplicate_tran_id.fec");
    assert_eq!(
        rules_by_line(&v),
        [(5, Rule::DuplicateTransactionId)],
        "{v}"
    );
    let f = &v.findings[0];
    assert_eq!(f.form_type, "SB21B");
    assert_eq!(f.field, Some("transaction_id"));
    assert!(
        f.message.contains("SB21B.4120") && f.message.contains("line 4"),
        "{f}"
    );
    assert!(!v.is_acceptable());
}

#[test]
fn bad_filer_id() {
    let v = invalid("bad_filer_id.fec");
    assert_eq!(rules_by_line(&v), [(2, Rule::FilerIdFormat)], "{v}");
    assert_eq!(v.findings[0].field, Some("filer_committee_id_number"));
    assert!(v.findings[0].message.contains("C0094412"));
}

#[test]
fn filer_id_mismatch() {
    let v = invalid("filer_id_mismatch.fec");
    assert_eq!(rules_by_line(&v), [(6, Rule::FilerIdMismatch)], "{v}");
    assert!(
        v.findings[0].message.contains("C00944125") && v.findings[0].message.contains("C00944124"),
        "{}",
        v.findings[0]
    );
}

#[test]
fn field_too_long() {
    let v = invalid("field_too_long.fec");
    assert_eq!(
        rules_by_line(&v),
        [(3, Rule::FieldTooLong), (4, Rule::FieldTooLong)],
        "{v}"
    );
    assert_eq!(v.findings[0].field, Some("contributor_last_name"));
    assert!(
        v.findings[0]
            .message
            .contains("maximum length of 30 (31 characters)")
    );
    assert_eq!(v.findings[1].field, Some("payee_organization_name"));
    assert!(
        v.findings[1]
            .message
            .contains("maximum length of 200 (201 characters)")
    );
}

#[test]
fn dangling_back_reference() {
    let v = invalid("dangling_back_reference.fec");
    assert_eq!(rules_by_line(&v), [(5, Rule::BackReferenceNotFound)], "{v}");
    assert_eq!(v.findings[0].field, Some("back_reference_tran_id"));
    assert!(v.findings[0].message.contains("SA11AI.9999"));
}

#[test]
fn illegal_character() {
    let v = invalid("illegal_character.fec");
    assert_eq!(
        rules_by_line(&v),
        [(4, Rule::IllegalCharacter), (7, Rule::IllegalCharacter)],
        "{v}"
    );
    assert!(
        v.findings[0].message.contains("U+007F"),
        "{}",
        v.findings[0]
    );
    assert!(
        v.findings[1].message.contains("U+017E"),
        "{}",
        v.findings[1]
    );
}

#[test]
fn amendment_missing_ids_and_embedded_quote() {
    let v = invalid("amendment_missing_ids.fec");
    assert_eq!(
        rules_by_line(&v),
        [
            (1, Rule::AmendmentNeedsOriginalId),
            (1, Rule::AmendmentNeedsNumber),
            (4, Rule::EmbeddedDoubleQuote),
        ],
        "{v}"
    );
    assert_eq!(v.findings[0].form_type, "HDR");
    assert_eq!(v.findings[2].field, Some("payee_organization_name"));
}

#[test]
fn bad_dates_and_amounts() {
    let v = invalid("bad_dates_and_amounts.fec");
    assert_eq!(
        rules_by_line(&v),
        [
            (2, Rule::NotARealDate),
            (4, Rule::NotARealDate),
            (5, Rule::BadDateFormat),
            (6, Rule::InvalidAmount),
            (7, Rule::InvalidAmount),
        ],
        "{v}"
    );
    assert_eq!(v.findings[0].field, Some("date_signed"));
    assert!(
        v.findings[0]
            .message
            .contains("20261301 is not a Real Date")
    );
    assert!(v.findings[2].message.contains("2026-06-22"));
    assert!(v.findings[3].message.contains("$5,500.00"));
    assert!(v.findings[4].message.contains("1500.005"));
}

#[test]
fn multi_form() {
    let v = invalid("multi_form.fec");
    assert_eq!(
        rules_by_line(&v),
        [(9, Rule::MultipleForms), (9, Rule::RequiredFieldEmpty)],
        "{v}"
    );
    assert_eq!(v.findings[0].form_type, "F3XN");
    assert_eq!(v.findings[1].field, Some("treasurer_last_name"));
    assert!(
        v.findings[1]
            .message
            .contains("TREASURER LAST NAME is Required, but field is Empty")
    );
}

#[test]
fn wrong_schedule_for_form() {
    let v = invalid("wrong_schedule_for_form.fec");
    assert_eq!(
        rules_by_line(&v),
        [(5, Rule::ScheduleNotAllowedWithForm)],
        "{v}"
    );
    assert!(
        v.findings[0].message.contains("Form F24"),
        "{}",
        v.findings[0]
    );
}

/// Every invalid fixture is, as intended, not acceptable, and its
/// `Display` output has one line per finding in the WebCheck style.
#[test]
fn every_invalid_fixture_is_rejected() {
    let dir = fixtures_dir().join("invalid");
    let names = fixture_names(&dir);
    assert_eq!(names.len(), 10, "{names:?}");
    for name in names {
        let v = validate(&dir.join(&name));
        assert!(!v.is_acceptable(), "{name} should have errors");
        let text = v.to_string();
        assert_eq!(text.lines().count(), v.len(), "{name}");
        for (line, f) in text.lines().zip(v.iter()) {
            let prefix = if f.severity == Severity::Error {
                "ERROR line "
            } else {
                "WARN  line "
            };
            assert!(line.starts_with(prefix), "{name}: {line}");
        }
    }
}
