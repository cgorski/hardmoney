//! `hardmoney spec export` as a contract.
//!
//! The JSON Schema rendering of the FEC format
//! (`hardmoney::parser::jsonschema`) must never reject data the FEC
//! accepted: the same false-positive discipline `tests/validate_fixtures.rs`
//! holds the validator to. Every record of every real filing in
//! `tests/fixtures/*.fec` (spec 3.00 through 8.5) is checked against the
//! schema for its table at its version, with a small in-crate reading of the
//! keywords the schema uses (`type`, `maxLength`, `enum`, `pattern`,
//! `required`).
//!
//! `python_jsonschema_accepts_the_schemas_and_the_fixture_records` repeats
//! the check with the `jsonschema` Python package (draft 2020-12 metaschema
//! plus record validation) so the in-crate reading cannot drift from the
//! real thing. It needs `tmp/venv` with `jsonschema` installed
//! (`tmp/venv/bin/pip install jsonschema`) and runs only when
//! `HARDMONEY_ORACLE_TESTS=1`.
//!
//! The CLI tests run the built binary and need the `cli` feature.

#![cfg(feature = "serde")]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use hardmoney::parser::jsonschema::{JSON_SCHEMA_DRAFT, bundle, table_schema};
use hardmoney::parser::{ParsedLine, SpecVersion};
use hardmoney::{Filing, ParseOptions, Table};
use regex::Regex;
use serde_json::{Value, json};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture_paths() -> Vec<PathBuf> {
    let dir = root().join("tests/fixtures");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "fec"))
        .collect();
    paths.sort();
    assert!(
        paths.len() >= 31,
        "expected the real fixtures, found {}",
        paths.len()
    );
    paths
}

fn parse(path: &Path) -> Filing {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .into_parts()
        .0
}

/// Every record of a filing: the cover line, then the body.
fn records(filing: &Filing) -> impl Iterator<Item = &ParsedLine> {
    std::iter::once(&filing.summary).chain(filing.lines.iter())
}

/// Every spec version some table has a layout for.
fn known_versions() -> BTreeSet<SpecVersion> {
    Table::ALL
        .iter()
        .flat_map(|t| t.layouts())
        .flat_map(|l| l.versions.iter().copied())
        .collect()
}

// ---------------------------------------------------------------------------
// A minimal reading of the keywords the schema uses
// ---------------------------------------------------------------------------

/// Compiled `pattern`s, shared across records.
#[derive(Default)]
struct Patterns(HashMap<String, Regex>);

impl Patterns {
    fn get(&mut self, pattern: &str) -> &Regex {
        self.0
            .entry(pattern.to_string())
            .or_insert_with(|| Regex::new(pattern).unwrap_or_else(|e| panic!("{pattern}: {e}")))
    }
}

/// Checks one record against the schema for its table, the way a JSON
/// Schema validator would read the keywords hardmoney emits: `type`
/// (string), `maxLength` (code points), `enum` (exact match), `pattern`
/// (unanchored search), `required` (key present). Returns the violations.
fn violations(schema: &Value, line: &ParsedLine, patterns: &mut Patterns) -> Vec<String> {
    let mut out = Vec::new();
    let props = schema["properties"]
        .as_object()
        .unwrap_or_else(|| panic!("{}: properties is not an object", line.table()));
    for (name, value) in line.iter() {
        let Some(prop) = props.get(name) else {
            out.push(format!("{name}: no property in the schema"));
            continue;
        };
        if prop["type"] != "string" {
            out.push(format!("{name}: type is {}", prop["type"]));
        }
        if let Some(max) = prop.get("maxLength").and_then(Value::as_u64) {
            let len = value.chars().count() as u64;
            if len > max {
                out.push(format!(
                    "{name}: {value:?} has {len} chars, maxLength {max}"
                ));
            }
        }
        if let Some(allowed) = prop.get("enum").and_then(Value::as_array)
            && !allowed.iter().any(|a| a == value)
        {
            out.push(format!("{name}: {value:?} not in enum {allowed:?}"));
        }
        if let Some(pattern) = prop.get("pattern").and_then(Value::as_str)
            && !patterns.get(pattern).is_match(value)
        {
            out.push(format!("{name}: {value:?} does not match {pattern:?}"));
        }
    }
    for required in schema["required"].as_array().into_iter().flatten() {
        let name = required.as_str().unwrap_or_default();
        if line.get(name).is_none() {
            out.push(format!("{name}: required but absent from the record"));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The contract: the schema never rejects FEC-accepted data
// ---------------------------------------------------------------------------

#[test]
fn every_fixture_record_conforms_to_its_table_schema() {
    let mut schemas: HashMap<(Table, SpecVersion), Value> = HashMap::new();
    let mut patterns = Patterns::default();
    let mut records_checked = 0usize;
    let mut values_checked = 0usize;
    let mut versions_seen = BTreeSet::new();
    let mut failures = Vec::new();
    for path in fixture_paths() {
        let filing = parse(&path);
        let version = filing.version;
        versions_seen.insert(version);
        for line in records(&filing) {
            let schema = schemas.entry((line.table(), version)).or_insert_with(|| {
                table_schema(line.table(), version).unwrap_or_else(|| {
                    panic!(
                        "{}: line {} parsed as {} at {version} but there is no schema",
                        path.display(),
                        line.line_no,
                        line.table()
                    )
                })
            });
            records_checked += 1;
            values_checked += line.iter().filter(|(_, v)| !v.is_empty()).count();
            for v in violations(schema, line, &mut patterns) {
                failures.push(format!(
                    "{} line {} ({}): {v}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    line.line_no,
                    line.raw_form_type
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} schema violation(s) on FEC-accepted filings:\n{}",
        failures.len(),
        failures.join("\n")
    );
    // The check must have covered real data, across versions.
    assert!(
        records_checked > 5_000,
        "only {records_checked} records checked"
    );
    assert!(
        values_checked > 50_000,
        "only {values_checked} values checked"
    );
    assert!(versions_seen.contains(&SpecVersion::electronic(8, 5)));
    assert!(versions_seen.contains(&SpecVersion::electronic(3, 0)));
    assert!(versions_seen.len() >= 5, "{versions_seen:?}");
}

/// The keywords that reject data (`enum`, `pattern`) are really present
/// at 8.5, so the test above is not passing vacuously.
#[test]
fn constraining_keywords_are_present_at_the_bundled_version() {
    let v85 = SpecVersion::electronic(8, 5);
    let b = bundle(v85);
    let mut enums = 0;
    let mut patterns = 0;
    let mut max_lengths = 0;
    for def in b["$defs"].as_object().unwrap().values() {
        for prop in def["properties"].as_object().unwrap().values() {
            enums += usize::from(prop.get("enum").is_some());
            patterns += usize::from(prop.get("pattern").is_some());
            max_lengths += usize::from(prop.get("maxLength").is_some());
        }
    }
    assert!(enums >= 10, "{enums} enums");
    assert!(patterns >= 30, "{patterns} patterns");
    assert!(max_lengths >= 1_500, "{max_lengths} maxLengths");
}

// ---------------------------------------------------------------------------
// Structure, at every table and version
// ---------------------------------------------------------------------------

#[test]
fn every_table_at_every_version_renders_a_sound_schema() {
    let mut patterns = Patterns::default();
    let mut rendered = 0;
    for &table in Table::ALL {
        for version in known_versions() {
            let schema = table_schema(table, version);
            let layout = table.layout(version);
            assert_eq!(schema.is_some(), layout.is_some(), "{table} at {version}");
            let (Some(schema), Some(layout)) = (schema, layout) else {
                continue;
            };
            rendered += 1;
            assert_eq!(schema["$schema"], JSON_SCHEMA_DRAFT);
            assert_eq!(schema["type"], "object");
            let props = schema["properties"].as_object().unwrap();
            assert_eq!(props.len(), layout.fields.len(), "{table} at {version}");
            let mut columns = BTreeSet::new();
            for (name, prop) in props {
                assert!(layout.field(name).is_some(), "{table} at {version}: {name}");
                assert_eq!(prop["type"], "string", "{table}.{name}");
                let column = prop["x-fec"]["column"].as_u64().unwrap();
                assert!(
                    (1..=u64::from(layout.width)).contains(&column),
                    "{table}.{name} column {column} outside 1..={}",
                    layout.width
                );
                assert!(columns.insert(column), "{table}.{name}: duplicate column");
                if let Some(p) = prop.get("pattern").and_then(Value::as_str) {
                    assert!(p.starts_with("^$|"), "{table}.{name}: {p}");
                    let _ = patterns.get(p);
                    assert!(
                        patterns.get(p).is_match(""),
                        "{table}.{name}: {p} rejects blank"
                    );
                }
                if let Some(e) = prop.get("enum").and_then(Value::as_array) {
                    assert!(
                        e.iter().any(|v| v == ""),
                        "{table}.{name}: enum without blank"
                    );
                    assert!(e.len() >= 2, "{table}.{name}: {e:?}");
                }
                if let Some(n) = prop.get("maxLength").and_then(Value::as_u64) {
                    assert!(n >= 1, "{table}.{name}");
                    let max_len = prop["x-fec"]["max_len"].as_u64().unwrap();
                    if prop["x-fec"]["kind"] == "amount" {
                        assert_eq!(n, max_len + 2, "{table}.{name}");
                    } else {
                        assert_eq!(n, max_len, "{table}.{name}");
                    }
                }
            }
            for r in schema["required"].as_array().unwrap() {
                let name = r.as_str().unwrap();
                assert!(
                    props.contains_key(name),
                    "{table} at {version}: required {name}"
                );
                assert_eq!(props[name]["x-fec"]["required_level"], "error");
            }
            let hm = &schema["x-hardmoney"];
            assert_eq!(hm["spec_version"], version.to_string());
            assert_eq!(hm["table"], table.as_str());
            assert_eq!(hm["column_base"], 1);
            assert_eq!(hm["width"], u64::from(layout.width));
            let layout_versions: Vec<SpecVersion> = hm["layout_versions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().parse().unwrap())
                .collect();
            assert!(layout_versions.contains(&version));
            assert_eq!(layout_versions.len(), layout.versions.len());
        }
    }
    assert!(rendered > 1_000, "{rendered} schemas rendered");
}

#[test]
fn bundles_agree_with_standalone_documents_at_every_version() {
    for version in known_versions() {
        let b = bundle(version);
        assert_eq!(b["$schema"], JSON_SCHEMA_DRAFT);
        let defs = b["$defs"].as_object().unwrap();
        let with_layout: Vec<&str> = Table::ALL
            .iter()
            .filter(|t| t.layout(version).is_some())
            .map(|t| t.as_str())
            .collect();
        let names: Vec<&str> = defs.keys().map(String::as_str).collect();
        assert_eq!(names, with_layout, "{version}");
        assert_eq!(b["x-hardmoney"]["tables"], json!(with_layout));
        for (name, def) in defs {
            let table: Table = name.parse().unwrap();
            let mut standalone = table_schema(table, version).unwrap();
            standalone.as_object_mut().unwrap().remove("$schema");
            assert_eq!(&standalone, def, "{name} at {version}");
        }
    }
}

// ---------------------------------------------------------------------------
// The command line
// ---------------------------------------------------------------------------

#[cfg(feature = "cli")]
mod cli {
    use super::*;
    use std::process::Command;

    fn run(args: &[&str]) -> (bool, String, String) {
        let bin = env!("CARGO_BIN_EXE_hardmoney");
        let out = Command::new(bin)
            .args(args)
            .output()
            .unwrap_or_else(|e| panic!("running {bin}: {e}"));
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    #[test]
    fn json_schema_export_matches_the_library() {
        let v85 = SpecVersion::electronic(8, 5);
        let (ok, stdout, stderr) = run(&["spec", "export", "--format", "json-schema"]);
        assert!(ok, "{stderr}");
        let doc: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(doc, bundle(v85));

        let (ok, stdout, _) = run(&[
            "spec",
            "export",
            "--format",
            "json-schema",
            "--table",
            "scha",
        ]);
        assert!(ok);
        let doc: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(doc, table_schema(Table::SchA, v85).unwrap());

        let (ok, stdout, _) = run(&[
            "spec",
            "export",
            "--format",
            "json-schema",
            "--table",
            "SchA",
            "--version",
            "7.0",
        ]);
        assert!(ok);
        let doc: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(doc["x-hardmoney"]["spec_version"], "7.0");
        assert!(doc["properties"]["contribution_purpose_code"].is_object());

        // A table without a layout at the version is exit 1 naming what it has.
        let (ok, _, stderr) = run(&[
            "spec",
            "export",
            "--format",
            "json-schema",
            "--table",
            "SchI",
            "--version",
            "8.5",
        ]);
        assert!(!ok);
        assert!(
            stderr.contains("SchI has no column layout for spec version 8.5"),
            "{stderr}"
        );
        // A version nobody has is exit 1 too, not an empty bundle.
        let (ok, _, stderr) = run(&[
            "spec",
            "export",
            "--format",
            "json-schema",
            "--version",
            "9.9",
        ]);
        assert!(!ok);
        assert!(
            stderr.contains("no table has a column layout for spec version 9.9"),
            "{stderr}"
        );
    }

    #[test]
    fn csv_export_has_one_row_per_table_bucket_and_field() {
        let (ok, stdout, stderr) = run(&["spec", "export", "--format", "csv"]);
        assert!(ok, "{stderr}");
        let mut reader = csv::Reader::from_reader(stdout.as_bytes());
        let headers: Vec<String> = reader
            .headers()
            .unwrap()
            .iter()
            .map(str::to_string)
            .collect();
        assert_eq!(
            headers,
            [
                "table",
                "versions",
                "first_version",
                "last_version",
                "width",
                "column",
                "field",
                "rules_version",
                "description",
                "type",
                "kind",
                "max_len",
                "required_level",
                "required_condition",
                "sample",
                "value_reference",
                "rule",
                "forms",
                "allowed_values",
                "pattern",
            ]
        );
        let rows: Vec<BTreeMap<String, String>> =
            reader.deserialize().map(|r| r.unwrap()).collect();
        let expected: usize = Table::ALL
            .iter()
            .flat_map(|t| t.layouts())
            .map(|l| l.fields.len())
            .sum();
        assert_eq!(rows.len(), expected);
        // (table, versions, column) is a key.
        let keys: BTreeSet<(String, String, String)> = rows
            .iter()
            .map(|r| {
                (
                    r["table"].clone(),
                    r["versions"].clone(),
                    r["column"].clone(),
                )
            })
            .collect();
        assert_eq!(keys.len(), rows.len());
        let amount = rows
            .iter()
            .find(|r| {
                r["table"] == "SchA"
                    && r["field"] == "contribution_amount"
                    && r["last_version"] == "8.5"
            })
            .unwrap();
        assert_eq!(amount["column"], "21");
        assert_eq!(amount["type"], "AMT-12");
        assert_eq!(amount["forms"], "F3|F3X|F3P|F3L");
        // No cell carries a line break: one record per line for spreadsheets.
        assert!(rows.iter().all(|r| r.values().all(|v| !v.contains('\n'))));

        let (ok, stdout, _) = run(&[
            "spec",
            "export",
            "--format",
            "csv",
            "--table",
            "F3X",
            "--version",
            "8.5",
        ]);
        assert!(ok);
        let n = csv::Reader::from_reader(stdout.as_bytes())
            .records()
            .count();
        assert_eq!(
            n,
            Table::F3X
                .layout(SpecVersion::electronic(8, 5))
                .unwrap()
                .fields
                .len()
        );
    }

    #[test]
    fn json_export_is_unchanged_by_default_and_filters_with_table_and_version() {
        let (ok, stdout, _) = run(&["spec", "export"]);
        assert!(ok);
        let doc: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(doc["column_base"], 0);
        assert_eq!(doc["tables"].as_array().unwrap().len(), Table::ALL.len());
        let (_, legacy, _) = run(&["spec", "export", "--json"]);
        assert_eq!(legacy, stdout);

        let (ok, stdout, _) = run(&["spec", "export", "--table", "SchA", "--version", "7.0"]);
        assert!(ok);
        let doc: Value = serde_json::from_str(&stdout).unwrap();
        let tables = doc["tables"].as_array().unwrap();
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0]["table"], "SchA");
        let layouts = tables[0]["layouts"].as_array().unwrap();
        assert_eq!(layouts.len(), 1);
        assert!(
            layouts[0]["versions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "7.0")
        );

        // --json and --format are two spellings of one thing; both is a usage error.
        let (ok, _, stderr) = run(&["spec", "export", "--json", "--format", "csv"]);
        assert!(!ok);
        assert!(stderr.contains("cannot be used with"), "{stderr}");
    }
}

// ---------------------------------------------------------------------------
// The Python `jsonschema` package as oracle
// ---------------------------------------------------------------------------

/// Checks every bundle against the draft 2020-12 metaschema and every
/// fixture record against its table's schema with the reference
/// implementation. Reads `bundle-<version>.json` files and `records.jsonl`
/// (`{"version", "table", "file", "line_no", "record"}`) from the directory
/// in `argv[1]`.
const PYTHON_ORACLE: &str = r#"
import glob, json, os, sys
from jsonschema import Draft202012Validator

d = sys.argv[1]
validators = {}
schemas = 0
for path in sorted(glob.glob(os.path.join(d, "bundle-*.json"))):
    bundle = json.load(open(path))
    Draft202012Validator.check_schema(bundle)
    version = bundle["x-hardmoney"]["spec_version"]
    for table, schema in bundle["$defs"].items():
        Draft202012Validator.check_schema(schema)
        validators[(version, table)] = Draft202012Validator(schema)
        schemas += 1
records = 0
bad = []
for line in open(os.path.join(d, "records.jsonl")):
    r = json.loads(line)
    v = validators[(r["version"], r["table"])]
    records += 1
    for e in v.iter_errors(r["record"]):
        bad.append(f'{r["file"]} line {r["line_no"]} ({r["table"]} at {r["version"]}): {"/".join(map(str, e.path))}: {e.message}')
for b in bad[:50]:
    print(b)
print(f"{schemas} schemas valid, {records} records checked, {len(bad)} violations")
sys.exit(1 if bad else 0)
"#;

#[test]
fn python_jsonschema_accepts_the_schemas_and_the_fixture_records() {
    if std::env::var("HARDMONEY_ORACLE_TESTS").ok().as_deref() != Some("1") {
        eprintln!("skipping: set HARDMONEY_ORACLE_TESTS=1 (needs tmp/venv with jsonschema)");
        return;
    }
    let python = std::env::var("HARDMONEY_ORACLE_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root().join("tmp/venv/bin/python"));
    let dir = std::env::temp_dir().join(format!("hardmoney-spec-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let mut versions = BTreeSet::new();
    let mut records_out = String::new();
    for path in fixture_paths() {
        let filing = parse(&path);
        versions.insert(filing.version);
        let file = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        for line in records(&filing) {
            let record: serde_json::Map<String, Value> = line
                .iter()
                .map(|(k, v)| (k.to_string(), Value::from(v)))
                .collect();
            let row = json!({
                "version": filing.version.to_string(),
                "table": line.table().as_str(),
                "file": file,
                "line_no": line.line_no,
                "record": record,
            });
            records_out.push_str(&serde_json::to_string(&row).unwrap());
            records_out.push('\n');
        }
    }
    std::fs::write(dir.join("records.jsonl"), records_out).unwrap();
    for version in &versions {
        std::fs::write(
            dir.join(format!("bundle-{version}.json")),
            serde_json::to_string(&bundle(*version)).unwrap(),
        )
        .unwrap();
    }

    let output = std::process::Command::new(&python)
        .arg("-c")
        .arg(PYTHON_ORACLE)
        .arg(&dir)
        .output()
        .unwrap_or_else(|e| panic!("running {}: {e}", python.display()));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    println!("{stdout}");
    assert!(
        output.status.success(),
        "jsonschema rejected a schema or an FEC-accepted record (exit {:?})\n{stdout}\n{stderr}",
        output.status.code()
    );
    assert!(stdout.contains(" 0 violations"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}
