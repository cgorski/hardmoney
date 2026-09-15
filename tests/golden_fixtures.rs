//! The golden fixture pack in `tests/fixtures/golden/`.
//!
//! Two properties are checked. **Determinism**: regenerating the pack with
//! `examples/golden_fixtures.rs` (included here as a module) produces
//! byte-identical files, so the committed pack is exactly what the
//! generator says it is and nothing was edited by hand. **Expected
//! outputs**: every committed `.fec` re-validates and re-reconciles to what
//! its sidecars and `MANIFEST.json` record, the Form 3X's Column A matches
//! the values FECfile+'s own `test_calculate_summary_column_a` asserts on
//! the lines that calculator computes, and each broken variant produces
//! exactly its intended finding.
//!
//! The live WebCheck check is `#[ignore]` and self-skips unless
//! `HARDMONEY_NETWORK_TESTS=1`:
//!
//! ```text
//! HARDMONEY_NETWORK_TESTS=1 cargo test --all-features --test golden_fixtures -- --ignored --nocapture
//! ```

#![cfg(feature = "serde")]

#[allow(dead_code)]
#[path = "../examples/golden_fixtures.rs"]
mod golden;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use golden::pack;
use hardmoney::parser::reconcile::Column;
use hardmoney::parser::{Rule, Severity};
use hardmoney::{Filing, ParseOptions};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde_json::Value;

fn pack_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(pack::DEFAULT_DIR)
}

fn read(name: &str) -> Vec<u8> {
    let path = pack_dir().join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn json(name: &str) -> Value {
    serde_json::from_slice(&read(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn manifest_fixtures() -> Vec<Value> {
    json("MANIFEST.json")["fixtures"]
        .as_array()
        .expect("fixtures array")
        .clone()
}

fn parse(name: &str) -> Filing {
    Filing::parse_bytes_with(&read(name), &ParseOptions::LENIENT)
        .unwrap_or_else(|e| panic!("{name}: {e}"))
        .into_parts()
        .0
}

/// Files the generator owns: everything in the directory except the
/// hand-written README.
fn committed_files() -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(pack_dir()).expect("tests/fixtures/golden exists") {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "README.md" {
            continue;
        }
        out.insert(name, std::fs::read(entry.path()).unwrap());
    }
    out
}

// ---------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------

#[test]
fn committed_pack_is_byte_identical_to_a_fresh_regeneration() {
    let fresh = pack::generate().unwrap_or_else(|e| panic!("generate: {e}"));
    let committed = committed_files();
    let regenerate = format!(
        "regenerate with: cargo run --example golden_fixtures -- {}",
        pack::DEFAULT_DIR
    );
    let fresh_names: Vec<&String> = fresh.keys().collect();
    let committed_names: Vec<&String> = committed.keys().collect();
    assert_eq!(
        fresh_names, committed_names,
        "file set differs; {regenerate}"
    );
    for (name, bytes) in &fresh {
        let on_disk = &committed[name];
        if bytes != on_disk {
            let (a, b) = (
                String::from_utf8_lossy(bytes),
                String::from_utf8_lossy(on_disk),
            );
            let first_diff = a
                .lines()
                .zip(b.lines())
                .enumerate()
                .find(|(_, (x, y))| x != y)
                .map(|(i, (x, y))| format!("line {}:\n  fresh:     {x}\n  committed: {y}", i + 1))
                .unwrap_or_else(|| "lengths differ".to_string());
            panic!("{name} differs from a fresh regeneration ({first_diff}); {regenerate}");
        }
    }
}

#[test]
fn generating_twice_gives_the_same_bytes() {
    let a = pack::generate().unwrap();
    let b = pack::generate().unwrap();
    assert_eq!(a, b);
}

/// The same check through the file system: write the pack to a fresh
/// temporary directory and compare each file with its committed twin.
#[test]
fn pack_written_to_a_temp_dir_matches_the_committed_files() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!("hardmoney-golden-{}-{nanos}", std::process::id()));
    let written = pack::write_to(&dir).unwrap_or_else(|e| panic!("write_to: {e}"));
    let committed = committed_files();
    assert_eq!(written.len(), committed.len());
    for (name, on_disk) in &committed {
        let fresh = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(&fresh, on_disk, "{name}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn manifest_hashes_match_the_files() {
    for fx in manifest_fixtures() {
        let file = fx["file"].as_str().unwrap();
        let bytes = read(file);
        assert_eq!(
            pack::sha256_hex(&bytes),
            fx["sha256"].as_str().unwrap(),
            "{file}"
        );
        assert_eq!(bytes.len() as u64, fx["bytes"].as_u64().unwrap(), "{file}");
    }
}

/// FIPS 180-4 test vectors, so the manifest hashes mean what they say.
#[test]
fn sha256_matches_the_standard_vectors() {
    assert_eq!(
        pack::sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        pack::sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        pack::sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    // 1,000,000 x 'a' crosses many blocks.
    assert_eq!(
        pack::sha256_hex(&vec![b'a'; 1_000_000]),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

// ---------------------------------------------------------------------
// Expected outputs hold on the committed files
// ---------------------------------------------------------------------

#[test]
fn every_fixture_is_spec_85_from_a_registry_committee() {
    let fixtures = manifest_fixtures();
    assert_eq!(fixtures.len(), 9);
    let names: Vec<&str> = fixtures
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "f3x",
            "f3",
            "f3p",
            "f24",
            "f1m",
            "f99",
            "f3x_cover_off_by_one_cent",
            "f3x_duplicate_transaction_id",
            "f3x_missing_required_field",
        ]
    );
    let registry: Vec<&str> = pack::COMMITTEE_IDS.iter().map(|(_, id, _)| *id).collect();
    for fx in &fixtures {
        let filing = parse(fx["file"].as_str().unwrap());
        assert_eq!(filing.version.to_string(), "8.5");
        assert_eq!(filing.header.soft_name, pack::SOFT_NAME);
        assert_eq!(filing.header.soft_ver, pack::PACK_VERSION);
        let filer = filing.summary.get("filer_committee_id_number").unwrap();
        let want = match filing.base_form_type.as_str() {
            "F3" => pack::HOUSE_COMMITTEE,
            "F3P" => pack::PRESIDENTIAL_COMMITTEE,
            _ => pack::PAC,
        };
        assert_eq!(filer, want, "{}", fx["name"]);
        // Every body line is filed under the same id, and every committee
        // id anywhere in the file is one the manifest names.
        for line in &filing.lines {
            assert_eq!(line.get("filer_committee_id_number"), Some(filer));
            for (field, value) in line.iter() {
                if field.ends_with("committee_fec_id") || field.ends_with("committee_id_number") {
                    assert!(
                        value.is_empty() || registry.contains(&value),
                        "{}: {field} = {value}",
                        fx["name"]
                    );
                }
            }
        }
        assert_eq!(filing.base_form_type, fx["form"].as_str().unwrap());
        assert_eq!(filing.raw_form_type, fx["form_type"].as_str().unwrap());
        assert_eq!(
            filing.lines.len() as u64,
            fx["body_lines"].as_u64().unwrap()
        );
    }
}

#[test]
fn every_fixture_validates_to_its_manifest_expectation() {
    for fx in manifest_fixtures() {
        let name = fx["name"].as_str().unwrap();
        let filing = parse(fx["file"].as_str().unwrap());
        let v = filing.validate();
        let expected = &fx["expected"];
        assert_eq!(
            v.is_acceptable(),
            expected["acceptable"].as_bool().unwrap(),
            "{name}"
        );
        assert_eq!(
            v.error_count() as u64,
            expected["errors"].as_u64().unwrap(),
            "{name}"
        );
        assert_eq!(
            v.warning_count() as u64,
            expected["warnings"].as_u64().unwrap(),
            "{name}"
        );
        let got: Vec<(String, String, u64, String, Option<String>)> = v
            .iter()
            .map(|f| {
                (
                    f.rule.to_string(),
                    f.severity.to_string(),
                    f.line_no,
                    f.form_type.clone(),
                    f.field.map(str::to_string),
                )
            })
            .collect();
        let want: Vec<(String, String, u64, String, Option<String>)> = expected["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    f["rule"].as_str().unwrap().to_string(),
                    f["severity"].as_str().unwrap().to_string(),
                    f["line_no"].as_u64().unwrap(),
                    f["form_type"].as_str().unwrap().to_string(),
                    f["field"].as_str().map(str::to_string),
                )
            })
            .collect();
        assert_eq!(got, want, "{name}");

        // The sidecar carries the same findings.
        let sidecar = json(&format!("{name}.validation.json"));
        assert_eq!(sidecar["errors"].as_u64().unwrap(), v.error_count() as u64);
        assert_eq!(
            sidecar["warnings"].as_u64().unwrap(),
            v.warning_count() as u64
        );
        assert_eq!(sidecar["findings"].as_array().unwrap().len(), v.len());
    }
}

#[test]
fn every_periodic_report_reconciles_to_its_manifest_expectation() {
    let mut reconciled = 0;
    for fx in manifest_fixtures() {
        let name = fx["name"].as_str().unwrap();
        let filing = parse(fx["file"].as_str().unwrap());
        let expected = &fx["expected"]["reconciliation"];
        match filing.reconcile() {
            Err(_) => {
                assert!(
                    expected.is_null(),
                    "{name}: manifest expects a reconciliation"
                );
                assert!(
                    matches!(filing.base_form_type.as_str(), "F24" | "F1M" | "F99"),
                    "{name}"
                );
            }
            Ok(r) => {
                reconciled += 1;
                assert_eq!(
                    r.checks.len() as u64,
                    expected["checks"].as_u64().unwrap(),
                    "{name}"
                );
                assert_eq!(
                    r.balances(),
                    expected["balances"].as_bool().unwrap(),
                    "{name}"
                );
                let got: Vec<(String, String, String)> = r
                    .mismatches()
                    .map(|c| {
                        (
                            c.column.to_string(),
                            c.line.to_string(),
                            c.delta.to_string(),
                        )
                    })
                    .collect();
                let want: Vec<(String, String, String)> = expected["lines"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|l| {
                        (
                            l["column"].as_str().unwrap().to_string(),
                            l["line"].as_str().unwrap().to_string(),
                            l["delta"].as_str().unwrap().to_string(),
                        )
                    })
                    .collect();
                assert_eq!(got, want, "{name}");
                let sidecar = json(&format!("{name}.reconciliation.json"));
                assert_eq!(sidecar["checks"].as_u64().unwrap(), r.checks.len() as u64);
                assert_eq!(
                    sidecar["disagreeing"].as_u64().unwrap(),
                    r.mismatches().count() as u64
                );
            }
        }
    }
    assert_eq!(reconciled, 6, "f3x, f3, f3p and the three f3x variants");
}

#[test]
fn clean_fixtures_have_no_findings_and_balance() {
    for name in ["f3x", "f3", "f3p", "f24", "f1m", "f99"] {
        let filing = parse(&format!("{name}.fec"));
        let v = filing.validate();
        assert!(v.is_empty(), "{name}:\n{v}");
        if let Ok(r) = filing.reconcile() {
            assert!(r.balances(), "{name}:\n{r}");
            // Both columns were filled: nothing on the cover is blank.
            for (field, value) in filing.summary.iter() {
                if field.starts_with("col_") {
                    assert!(!value.is_empty(), "{name}: {field} blank");
                }
            }
        }
    }
}

#[test]
fn every_fixture_round_trips_through_the_writer() {
    for fx in manifest_fixtures() {
        let file = fx["file"].as_str().unwrap();
        let bytes = read(file);
        let filing = Filing::parse_bytes(&bytes).unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(filing.to_fec(), bytes, "{file}: writer output is canonical");
    }
}

#[test]
fn f99_carries_a_text_block() {
    let bytes = read("f99.fec");
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("\r\n[BEGINTEXT]\r\n"));
    assert!(text.contains("\r\n[ENDTEXT]\r\n"));
    let filing = parse("f99.fec");
    let body = filing.summary.get("text").unwrap();
    assert!(body.starts_with("This filing is a synthetic fixture"));
    assert!(body.contains("\n\n"), "two paragraphs");
}

// ---------------------------------------------------------------------
// The Form 3X and FECfile+'s expected Column A
// ---------------------------------------------------------------------

fn column_a() -> BTreeMap<String, Decimal> {
    json("f3x.expected_column_a.json")
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap().parse().unwrap()))
        .collect()
}

/// The values `fecfiler/reports/form_3x/tests/test_summary.py::
/// test_calculate_summary_column_a` asserts, on the lines FECfile+ derives
/// from Schedules A-F (it stubs the H-schedule lines to zero, so those and
/// the totals that include them are checked separately below).
#[test]
fn f3x_column_a_matches_fecfile_plus_on_the_lines_it_computes() {
    let a = column_a();
    let expect = |key: &str, want: Decimal| {
        assert_eq!(a.get(key).copied(), Some(want), "{key}");
    };
    expect("line_6b", dec!(61.00));
    expect("line_9", dec!(250.00));
    expect("line_10", dec!(250.00));
    expect("line_11ai", dec!(10000.23));
    expect("line_11aii", dec!(3.77));
    expect("line_11aiii", dec!(10004.00));
    expect("line_11b", dec!(444.44));
    expect("line_11c", dec!(555.55));
    expect("line_11d", dec!(11003.99));
    expect("line_12", dec!(1212.12));
    expect("line_13", dec!(1313.13));
    expect("line_14", dec!(1414.14));
    expect("line_15", dec!(2125.79));
    expect("line_16", dec!(16.00));
    expect("line_17", dec!(1000.00));
    expect("line_20", dec!(18085.17)); // = 19 - 18(c): the H3 transfer cancels out
    expect("line_21b", dec!(150.00));
    expect("line_22", dec!(22.00));
    expect("line_23", dec!(14.00));
    expect("line_24", dec!(151.00));
    expect("line_25", dec!(133.00));
    expect("line_26", dec!(44.00));
    expect("line_27", dec!(31.00));
    expect("line_28a", dec!(101.50));
    expect("line_28b", dec!(201.50));
    expect("line_28c", dec!(301.50));
    expect("line_28d", dec!(604.50));
    expect("line_29", dec!(201.50));
    expect("line_30b", dec!(102.25));
    expect("line_30c", dec!(102.25));
    expect("line_33", dec!(11003.99));
    expect("line_34", dec!(604.50));
    expect("line_35", dec!(10399.49));
    expect("line_37", dec!(2125.79));
}

/// The five lines FECfile+ sets to `Decimal(0)` ("Stubbed out until a
/// future ticket"), with the values the H3/H4 records imply, and the
/// totals that change because of them. 18(a) sums `transferred_amount`
/// (750 + 250), not `total_amount_transferred` (1000 + 1000); 21(a)(i)/(ii)
/// skip the memo H4 breakdown.
#[test]
fn f3x_column_a_derives_the_h_schedule_lines_fecfile_plus_stubs() {
    let a = column_a();
    let expect = |key: &str, want: Decimal| {
        assert_eq!(a.get(key).copied(), Some(want), "{key}");
    };
    expect("line_18a", dec!(1000.00));
    expect("line_18b", dec!(0.00));
    expect("line_18c", dec!(1000.00));
    expect("line_21ai", dec!(330.00));
    expect("line_21aii", dec!(670.00));
    expect("line_30ai", dec!(0.00));
    expect("line_30aii", dec!(0.00));
    // Totals FECfile+ computes with the stubs at zero: 18085.17, 150.00,
    // 1453.25, 1453.25, 150.00, -1975.79, 18146.17, 16631.92.
    expect("line_19", dec!(19085.17));
    expect("line_6c", dec!(19085.17));
    expect("line_21c", dec!(1150.00));
    expect("line_31", dec!(2453.25));
    expect("line_7", dec!(2453.25));
    expect("line_32", dec!(1783.25));
    expect("line_36", dec!(480.00));
    expect("line_38", dec!(-1645.79));
    expect("line_6d", dec!(19146.17));
    expect("line_8", dec!(16692.92));
}

#[test]
fn f3x_column_a_keys_are_fecfile_plus_keys_and_cover_every_rule() {
    let a = column_a();
    let filing = parse("f3x.fec");
    let r = filing.reconcile().unwrap();
    for c in r.column(Column::A) {
        let key = pack::line_key(c.line);
        assert!(key.starts_with("line_") && !key.contains('('), "{key}");
        assert_eq!(a.get(&key).copied(), Some(c.expected), "{key}");
        // The cover agrees with every one of them.
        assert_eq!(c.reported, Some(c.expected), "{}", c.line);
    }
    assert_eq!(pack::line_key("11(a)(i)"), "line_11ai");
    assert_eq!(pack::line_key("30(a)(ii)"), "line_30aii");
    assert_eq!(pack::line_key("6(b)"), "line_6b");
    assert_eq!(pack::line_key("38"), "line_38");
    // The two inputs the reconciler does not check are present too.
    assert!(a.contains_key("line_11aii") && a.contains_key("line_6b"));
    assert_eq!(a.len(), r.column(Column::A).count() + 2);
}

#[test]
fn f3x_body_has_every_schedule_the_pack_promises() {
    let filing = parse("f3x.fec");
    let tokens: Vec<&str> = filing
        .lines
        .iter()
        .map(|l| l.raw_form_type.as_str())
        .collect();
    for token in [
        "SA11AI", "SA11B", "SA11C", "SA12", "SA13", "SA14", "SA15", "SA16", "SA17", "SB21B",
        "SB22", "SB23", "SB26", "SB27", "SB28A", "SB28B", "SB28C", "SB29", "SB30B", "SC/9",
        "SC/10", "SD9", "SD10", "SE", "SF", "H3", "H4",
    ] {
        assert!(tokens.contains(&token), "{token} missing");
    }
    assert_eq!(filing.lines.iter().filter(|l| l.is_memo()).count(), 3);
    // Column B equals Column A on a first-quarter report.
    let r = filing.reconcile().unwrap();
    for b in r.column(Column::B) {
        if let Some(a) = r.line(Column::A, b.line) {
            assert_eq!(a.reported, b.reported, "line {}", b.line);
        }
    }
    assert_eq!(filing.summary.get("col_b_year"), Some("2026"));
}

// ---------------------------------------------------------------------
// The broken variants produce exactly their intended finding
// ---------------------------------------------------------------------

#[test]
fn off_by_one_cent_validates_clean_and_reconcile_flags_two_lines() {
    let filing = parse("f3x_cover_off_by_one_cent.fec");
    assert!(filing.validate().is_empty());
    let r = filing.reconcile().unwrap();
    let bad: Vec<(&str, Decimal)> = r.mismatches().map(|c| (c.line, c.delta)).collect();
    assert_eq!(bad, [("11(a)(i)", dec!(0.01)), ("11(a)(iii)", dec!(-0.01))]);
    assert!(r.mismatches_over(dec!(0.01)).next().is_none());
}

#[test]
fn duplicate_transaction_id_is_one_error_on_line_4() {
    let filing = parse("f3x_duplicate_transaction_id.fec");
    let v = filing.validate();
    assert_eq!(v.len(), 1, "{v}");
    let f = &v.findings[0];
    assert_eq!(f.rule, Rule::DuplicateTransactionId);
    assert_eq!(f.severity, Severity::Error);
    assert_eq!(f.line_no, 4);
    assert_eq!(f.form_type, "SA11B");
    assert_eq!(f.field, Some("transaction_id"));
    assert!(f.message.contains("SA11AI.1"));
    assert!(filing.reconcile().unwrap().balances());
}

#[test]
fn missing_required_field_is_one_error_on_line_3() {
    let filing = parse("f3x_missing_required_field.fec");
    let v = filing.validate();
    assert_eq!(v.len(), 1, "{v}");
    let f = &v.findings[0];
    assert_eq!(f.rule, Rule::RequiredFieldEmpty);
    assert_eq!(f.severity, Severity::Error);
    assert_eq!(f.line_no, 3);
    assert_eq!(f.form_type, "SA11AI");
    assert_eq!(f.field, Some("contributor_last_name"));
    assert!(filing.reconcile().unwrap().balances());
}

// ---------------------------------------------------------------------
// The sidecars are what the CLI prints
// ---------------------------------------------------------------------

#[cfg(feature = "cli")]
#[test]
fn sidecars_equal_the_cli_json_output() {
    let bin = env!("CARGO_BIN_EXE_hardmoney");
    for fx in manifest_fixtures() {
        let name = fx["name"].as_str().unwrap();
        let file = format!("{name}.fec");
        let out = std::process::Command::new(bin)
            .current_dir(pack_dir())
            .args(["validate", &file, "--json"])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&read(&format!("{name}.validation.json"))),
            "{name}: validate --json"
        );
        if fx["expected"]["reconciliation"].is_null() {
            continue;
        }
        let out = std::process::Command::new(bin)
            .current_dir(pack_dir())
            .args(["reconcile", &file, "--json", "--all"])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&read(&format!("{name}.reconciliation.json"))),
            "{name}: reconcile --json --all"
        );
    }
}

// ---------------------------------------------------------------------
// Live oracle: the FEC's WebCheck
// ---------------------------------------------------------------------

/// Submits every fixture to WebCheck and prints the verdicts (recorded in
/// the pack's README). Clean fixtures must be accepted; the duplicate-id
/// and missing-field variants must be rejected; the off-by-one-cent cover
/// is accepted (the FEC only warns about unsupported subtotals).
#[cfg(feature = "fetch")]
#[test]
#[ignore = "network: HARDMONEY_NETWORK_TESTS=1"]
fn webcheck_verdicts() {
    use hardmoney::parser::webcheck::WebCheck;
    if std::env::var("HARDMONEY_NETWORK_TESTS").as_deref() != Ok("1") {
        eprintln!("skipping: set HARDMONEY_NETWORK_TESTS=1 to run network tests");
        return;
    }
    let webcheck = WebCheck::new();
    let mut failures = Vec::new();
    for fx in manifest_fixtures() {
        let name = fx["name"].as_str().unwrap();
        let file = format!("{name}.fec");
        let bytes = read(&file);
        let report = webcheck
            .submit(&file, &bytes, None)
            .unwrap_or_else(|e| panic!("{file}: {e}"));
        println!(
            "{file}: WebCheck {} ({} error(s), {} warning(s))",
            report.result.as_deref().unwrap_or("?"),
            report.error_count(),
            report.warning_count()
        );
        for f in &report.findings {
            println!("    {:?} line {:?}: {}", f.severity, f.line_no, f.message);
        }
        let want_accepted = fx["expected"]["acceptable"].as_bool().unwrap();
        if report.is_acceptable() != want_accepted {
            failures.push(format!(
                "{file}: WebCheck acceptable={} but hardmoney says {want_accepted}",
                report.is_acceptable()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
