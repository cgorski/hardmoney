//! Oracle tests for the parser: hardmoney against the FEC's own artefacts
//! and against an independently maintained parser.
//!
//! * The pure-Rust tests here run always. They check the bundled 8.5
//!   layouts against the FEC's v8.5 specification workbook (distilled into
//!   `FieldSpec`), check that every version literal in a format table's
//!   header lands in the bucket that names it, and round-trip every real
//!   fixture through the writer and the streaming reader.
//! * `fecfile_oracle_has_no_unexplained_disagreements` compares hardmoney's
//!   parse of every corpus filing, field by field, with the `fecfile`
//!   Python library (`tests/oracle_fecfile.py`). It needs the Python venv
//!   with both libraries installed, so it runs only when
//!   `HARDMONEY_ORACLE_TESTS=1` (set `HARDMONEY_ORACLE_PYTHON` to point at
//!   an interpreter other than `tmp/venv/bin/python`).

use std::path::{Path, PathBuf};
use std::process::Command;

use hardmoney::parser::stream::FilingReader;
use hardmoney::parser::{ParsedLine, SpecVersion};
use hardmoney::{Filing, Table};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture_paths(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "fec"))
        .collect();
    paths.sort();
    paths
}

/// `tests/fixtures/*.fec` and `tests/fixtures/quirks/*.fec` always; the
/// larger audit corpus under `tmp/` when it is present (it is not shipped)
/// and the file is under 20 MB.
fn corpus() -> Vec<PathBuf> {
    let root = root();
    let mut paths = fixture_paths(&root.join("tests/fixtures"));
    paths.extend(fixture_paths(&root.join("tests/fixtures/quirks")));
    for extra in [
        "tmp/agent-misc/f3scan",
        "tmp/agent-misc/samples",
        "tmp/agent-misc/filings",
    ] {
        paths.extend(fixture_paths(&root.join(extra)));
    }
    if let Ok(entries) = std::fs::read_dir(root.join("tmp/competitors")) {
        for comp in entries.filter_map(|e| e.ok().map(|e| e.path())) {
            for sub in [
                "test",
                "tests",
                "test-data",
                "test/data",
                "test/fecs",
                "tests/fixtures",
                "python/tests/fixtures",
            ] {
                paths.extend(fixture_paths(&comp.join(sub)));
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths.retain(|p| std::fs::metadata(p).is_ok_and(|m| m.len() <= 20 * 1024 * 1024));
    paths
}

// ---------------------------------------------------------------------------
// 8.5 layouts vs. the FEC workbook
// ---------------------------------------------------------------------------

/// Every column the FEC's v8.5 workbook lists for a table must be named by
/// hardmoney's 8.5 layout for that table, at the same column. A spec row
/// with no canonical field is a column we silently cannot read or write;
/// the audit found exactly one (F3L column 14, STATE OF ELECTION) and it
/// has been added, so the expected list is empty.
#[test]
fn every_8_5_spec_column_has_a_canonical_field_at_the_same_column() {
    let v85 = SpecVersion::electronic(8, 5);
    let mut unnamed: Vec<String> = Vec::new();
    let mut misplaced: Vec<String> = Vec::new();
    let mut compared = 0usize;
    for &t in Table::ALL {
        let specs = t.specs();
        if specs.is_empty() {
            continue;
        }
        let Some(layout) = t.layout(v85) else {
            // F3Z1/F3Z2/F3PZ1/F3PZ2 keep a workbook sheet but were dropped
            // from the 8.5 format ("no longer needed" -- SUMMARY OF CHANGES).
            assert!(
                matches!(t, Table::F3Z1 | Table::F3Z2 | Table::F3PZ1 | Table::F3PZ2),
                "{t} has spec rows but no 8.5 layout"
            );
            continue;
        };
        for s in specs {
            compared += 1;
            match s.canonical {
                None => unnamed.push(format!("{t} col {} {:?}", s.column + 1, s.description)),
                Some(name) => match layout.field(name) {
                    Some(def) if def.column == s.column => {}
                    Some(def) => misplaced.push(format!(
                        "{t}.{name}: layout col {} vs spec col {}",
                        def.column + 1,
                        s.column + 1
                    )),
                    None => {
                        misplaced.push(format!("{t}.{name}: named by spec, absent from layout"))
                    }
                },
            }
        }
    }
    assert!(compared > 1500, "only {compared} spec rows compared");
    assert_eq!(
        unnamed,
        Vec::<String>::new(),
        "8.5 spec columns with no canonical field"
    );
    assert_eq!(
        misplaced,
        Vec::<String>::new(),
        "8.5 spec/layout column disagreements"
    );
}

/// The other direction: every 8.5 layout field should be a spec column,
/// apart from the columns the FEC documents outside the workbook (the
/// F99 `text` column, which the FEC's own 8.5 listing carries as column 18
/// while the workbook leaves it to the `[BEGINTEXT]` block).
#[test]
fn every_8_5_layout_field_is_a_spec_column_except_the_documented_ones() {
    let v85 = SpecVersion::electronic(8, 5);
    let mut extra: Vec<String> = Vec::new();
    for &t in Table::ALL {
        if t.specs().is_empty() {
            continue;
        }
        let Some(layout) = t.layout(v85) else {
            continue;
        };
        for f in layout.fields {
            if t.spec(f.name).is_none() {
                extra.push(format!("{t}.{} (col {})", f.name, f.column + 1));
            }
        }
    }
    assert_eq!(extra, vec!["F99.text (col 18)".to_string()]);
}

// ---------------------------------------------------------------------------
// Version-bucket assignment vs. the CSV header text
// ---------------------------------------------------------------------------

/// The version literals a bucket header names, e.g. `^8.5|8.4` -> [8.5,
/// 8.4], `^(P2.6|P3.0)` -> [P2.6, P3.0], `^3|^2` -> [3.0, 2.0] (a bare
/// major stands for `M.0`). Character classes (`^[6-8]`, HDR only) yield
/// nothing and are checked separately.
fn header_literals(pattern: &str) -> Vec<SpecVersion> {
    pattern
        .split('|')
        .map(|alt| alt.trim_matches(|c| matches!(c, '^' | '(' | ')' | ' ')))
        .filter(|alt| !alt.is_empty() && !alt.contains('['))
        .filter_map(|alt| {
            // `8.5.0.1` is a build of 8.5; `SpecVersion` keeps one minor digit.
            alt.parse::<SpecVersion>().ok()
        })
        .collect()
}

fn csv_bucket_patterns(table: Table) -> Vec<String> {
    let path = root().join(format!("data/fec-csv-sources/{}.csv", table.as_str()));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let header = text.lines().next().unwrap_or("");
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(header.as_bytes());
    let record = reader
        .records()
        .next()
        .unwrap_or_else(|| panic!("{}: empty", path.display()))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    record
        .iter()
        .skip(1)
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string)
        .collect()
}

/// `build.rs` assigns each version to the *first* bucket whose header
/// pattern matches it. For every table, every version literally named in
/// a bucket's header must resolve to that bucket -- unless an earlier
/// bucket also names it, in which case the earlier one wins and the
/// literal is redundant. Buckets are generated in header order, so the
/// bucket index is the layout index.
#[test]
fn every_version_literal_in_a_csv_header_lands_in_the_bucket_that_names_it() {
    let mut checked = 0usize;
    for &t in Table::ALL {
        let patterns = csv_bucket_patterns(t);
        let layouts = t.layouts();
        assert_eq!(
            patterns.len(),
            layouts.len(),
            "{t}: header buckets vs generated layouts"
        );
        let mut claimed: Vec<SpecVersion> = Vec::new();
        for (i, pattern) in patterns.iter().enumerate() {
            let literals = header_literals(pattern);
            for v in literals {
                let expected = if claimed.contains(&v) {
                    claimed.iter().position(|c| *c == v).map(|_| {
                        layouts
                            .iter()
                            .position(|l| l.supports(v))
                            .expect("claimed version has a layout")
                    })
                } else {
                    Some(i)
                };
                let actual = layouts.iter().position(|l| l.supports(v));
                assert_eq!(
                    actual, expected,
                    "{t}: version {v} named by bucket {i} ({pattern:?}) resolved to layout {actual:?}"
                );
                assert!(
                    t.layout(v)
                        .is_some_and(|l| std::ptr::eq(l, &layouts[actual.expect("checked")])),
                    "{t}: Table::layout({v}) does not return the bucket's layout"
                );
                claimed.push(v);
                checked += 1;
            }
        }
    }
    assert!(checked > 300, "only {checked} literals checked");

    // Spot checks a reader can verify against the CSV headers by eye.
    let bucket_of = |t: Table, v: SpecVersion| {
        t.layouts()
            .iter()
            .position(|l| l.supports(v))
            .unwrap_or_else(|| panic!("{t} {v}"))
    };
    let e = SpecVersion::electronic;
    assert_eq!(
        bucket_of(Table::SchA, e(8, 0)),
        0,
        "SchA 8.0 is in ^8.5|8.4|8.3|8.2|8.1|8.0"
    );
    assert_eq!(
        bucket_of(Table::SchA, e(7, 0)),
        1,
        "SchA 7.0 is in ^7.0|6.4"
    );
    assert_eq!(bucket_of(Table::SchA, e(3, 0)), 8, "SchA 3.0 is in ^3|^2");
    assert_eq!(
        bucket_of(Table::F5, e(8, 1)),
        0,
        "F5 8.1 is in ^8.5|8.4|8.3|8.2|8.1"
    );
    assert_eq!(
        bucket_of(Table::F5, e(8, 0)),
        1,
        "F5 8.0 is in ^8.0|7.0|6.4|6.3|6.2|6.1"
    );
    assert_eq!(
        bucket_of(Table::F5, e(6, 1)),
        1,
        "F5 6.1 (added: headers/6.1.csv == 6.2)"
    );
    assert_eq!(
        bucket_of(Table::F2, e(8, 1)),
        1,
        "F2 8.1 is in ^8.1|8.0|7.0|6.4"
    );
    assert_eq!(
        bucket_of(Table::F2, e(8, 2)),
        0,
        "F2 8.2 is in ^8.5|8.4|8.3|8.2"
    );
    assert_eq!(
        bucket_of(Table::F9, e(6, 1)),
        2,
        "F9 6.1 has its own bucket"
    );
    assert_eq!(
        bucket_of(Table::SchC, e(6, 1)),
        1,
        "SchC 6.1 has its own bucket"
    );
    assert_eq!(
        bucket_of(Table::F99, e(8, 5)),
        0,
        "F99 8.5 matches ^8.5.0.1|8.5"
    );
    assert_eq!(bucket_of(Table::F3Z, e(3, 0)), 0, "F3Z covers 3.x (added)");
    assert_eq!(
        bucket_of(Table::SchC2, e(5, 3)),
        2,
        "SchC2 5.3 has its own bucket"
    );
    // HDR's `^[6-8]` / `^[3-5]` classes.
    assert_eq!(bucket_of(Table::Hdr, e(8, 5)), 0);
    assert_eq!(bucket_of(Table::Hdr, e(6, 0)), 0);
    assert_eq!(bucket_of(Table::Hdr, e(5, 3)), 1);
    assert_eq!(bucket_of(Table::Hdr, e(3, 0)), 1);
    assert!(Table::Hdr.layout(e(2, 0)).is_none());
}

/// The comma-vs-ASCII-28 boundary the header's version decides must agree
/// with the buckets: a 5.x layout is comma-delimited, a 6.x one is not.
#[test]
fn delimiter_choice_matches_version_major() {
    for major in 3..=5u8 {
        assert!(!SpecVersion::electronic(major, 0).uses_fs_delimiter());
        assert!(SpecVersion::electronic(major, 0).has_name_delim_header());
    }
    for major in 6..=8u8 {
        assert!(SpecVersion::electronic(major, 5).uses_fs_delimiter());
        assert!(!SpecVersion::electronic(major, 5).has_name_delim_header());
    }
    assert!(!SpecVersion::paper(3, 4).uses_fs_delimiter());
    assert!(!SpecVersion::paper(3, 4).has_name_delim_header());
}

// ---------------------------------------------------------------------------
// Round trips over the corpus
// ---------------------------------------------------------------------------

fn same_records(a: &ParsedLine, b: &ParsedLine) -> bool {
    a.raw_form_type == b.raw_form_type && a.table() == b.table() && a.iter().eq(b.iter())
}

/// `parse(write(parse(f)))` reproduces every header field, the cover, and
/// every body line of every corpus filing hardmoney accepts, including the
/// ones with non-ASCII bytes (Windows-1252 and UTF-8 alike), and the
/// streaming reader yields exactly what the eager parser does.
#[test]
fn corpus_round_trips_through_the_writer_and_the_streaming_reader() {
    let mut parsed = 0usize;
    let mut non_ascii = 0usize;
    for path in corpus() {
        let name = path.display().to_string();
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let Ok(filing) = Filing::parse_bytes(&bytes) else {
            // Paper versions, `/*` headers, unknown tokens under STRICT:
            // not round-trip material. The lenient parse must still not
            // panic.
            let _ = Filing::parse_bytes_with(&bytes, &hardmoney::parser::ParseOptions::LENIENT);
            continue;
        };
        parsed += 1;
        if !bytes.is_ascii() {
            non_ascii += 1;
        }

        let written = filing.to_fec();
        let again =
            Filing::parse_bytes(&written).unwrap_or_else(|e| panic!("{name}: re-parse: {e}"));
        assert_eq!(filing.header, again.header, "{name}: header");
        assert_eq!(filing.version, again.version, "{name}");
        assert_eq!(filing.raw_form_type, again.raw_form_type, "{name}");
        assert!(
            same_records(&filing.summary, &again.summary),
            "{name}: cover"
        );
        assert_eq!(filing.lines.len(), again.lines.len(), "{name}: line count");
        for (a, b) in filing.lines.iter().zip(&again.lines) {
            assert!(
                same_records(a, b),
                "{name}: line {} differs:\n{a:?}\n{b:?}",
                a.line_no
            );
        }
        // Writing is idempotent: the canonical form re-parses to itself.
        assert_eq!(again.to_fec(), written, "{name}: writer is not idempotent");

        let reader = FilingReader::new(std::io::Cursor::new(&bytes))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            reader.preamble().summary,
            filing.summary,
            "{name}: streamed cover"
        );
        let streamed: Vec<ParsedLine> = reader
            .collect::<Result<_, _>>()
            .unwrap_or_else(|e| panic!("{name}: stream: {e}"));
        assert_eq!(streamed, filing.lines, "{name}: streamed lines");
    }
    assert!(parsed >= 20, "only {parsed} corpus filings parsed");
    assert!(
        non_ascii >= 1,
        "the corpus should include at least one non-ASCII filing"
    );
}

// ---------------------------------------------------------------------------
// Real comma-delimited quirks (spec 3.00, Aristotle CM4 / Navision AVF)
// ---------------------------------------------------------------------------

/// A real 2001 F3X from Aristotle CM4 ends every Schedule H4 record with an
/// unbalanced `"`. Read as one CSV stream (hardmoney before 2.3) that quote
/// opened a field which swallowed the remaining 113 records into one, with
/// no error. Every other parser of the era reads one record per line; so
/// does hardmoney now. 212 physical lines: header, cover, 210 records.
#[test]
fn aristotle_trailing_quote_filing_keeps_every_record() {
    let path = root().join("tests/fixtures/quirks/F3XN_3.00_trailing_quote.fec");
    let bytes = std::fs::read(&path).expect("fixture");
    let physical = bytes.iter().filter(|&&b| b == b'\n').count();
    let filing = Filing::parse_bytes(&bytes).expect("parses");
    assert_eq!(filing.version, SpecVersion::electronic(3, 0));
    assert_eq!(filing.summary.line_no, 2);
    assert_eq!(
        filing.lines.len(),
        physical - 2,
        "one record per physical line"
    );
    assert_eq!(
        filing.lines.last().map(|l| l.line_no),
        Some(physical as u64)
    );
    let h4: Vec<&ParsedLine> = filing.lines_for(Table::H4).collect();
    assert_eq!(h4.len(), 114);
    for line in &h4 {
        // The stray quote is an empty trailing field, not data in a field.
        assert!(line.iter().all(|(_, v)| !v.contains('"')), "{line:?}");
        assert!(
            line.get("payee_name").is_some_and(|v| !v.is_empty()),
            "{line:?}"
        );
    }
    assert_eq!(h4[0].get("payee_name"), Some("Public Housing Agenc^"));
    assert_eq!(h4[0].get("transaction_id"), Some("H40212200123E6995"));
    // The H1 line carries a real AMENDED CD value, readable since the
    // column was named.
    let h1 = filing.lines_for(Table::H1).next().expect("one H1");
    assert_eq!(h1.get("amended_cd"), Some("A"));
    assert_eq!(h1.get("transaction_id"), Some("H10212200143J0"));

    // Streaming agrees, and the writer's output re-parses to the same records.
    let streamed: Vec<ParsedLine> = FilingReader::new(std::io::Cursor::new(&bytes))
        .expect("streams")
        .collect::<Result<_, _>>()
        .expect("streams");
    assert_eq!(streamed, filing.lines);
    let again = Filing::parse_bytes(&filing.to_fec()).expect("re-parses");
    assert_eq!(again.lines.len(), filing.lines.len());
}

/// A real 2001 F3X from Navision AVF with blank lines between records
/// (and between header and cover). Line numbers are the physical lines,
/// not the `csv` crate's "position before the skipped blanks".
#[test]
fn navision_blank_lines_filing_has_physical_line_numbers() {
    let path = root().join("tests/fixtures/quirks/F3XN_3.00_blank_lines.fec");
    let bytes = std::fs::read(&path).expect("fixture");
    let text = String::from_utf8_lossy(&bytes);
    let filing = Filing::parse_bytes(&bytes).expect("parses");
    assert_eq!(filing.summary.line_no, 3, "cover is on physical line 3");
    for line in std::iter::once(&filing.summary).chain(&filing.lines) {
        let physical = text
            .lines()
            .nth(usize::try_from(line.line_no).expect("small") - 1)
            .unwrap_or("");
        assert!(
            physical
                .trim_start_matches('"')
                .starts_with(&line.raw_form_type),
            "line {} is {physical:?}, not a {} record",
            line.line_no,
            line.raw_form_type
        );
    }
    assert_eq!(
        filing.lines.iter().map(|l| l.line_no).collect::<Vec<_>>(),
        [5, 6, 10, 12, 13, 15, 16, 18, 22]
    );
    let streamed: Vec<ParsedLine> = FilingReader::new(std::io::Cursor::new(&bytes))
        .expect("streams")
        .collect::<Result<_, _>>()
        .expect("streams");
    assert_eq!(streamed, filing.lines);
}

// ---------------------------------------------------------------------------
// The fecfile oracle (needs the Python venv)
// ---------------------------------------------------------------------------

#[test]
fn fecfile_oracle_has_no_unexplained_disagreements() {
    if std::env::var("HARDMONEY_ORACLE_TESTS").ok().as_deref() != Some("1") {
        eprintln!(
            "skipping: set HARDMONEY_ORACLE_TESTS=1 (needs tmp/venv with hardmoney and fecfile)"
        );
        return;
    }
    let python = std::env::var("HARDMONEY_ORACLE_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root().join("tmp/venv/bin/python"));
    let output = Command::new(&python)
        .arg(root().join("tests/oracle_fecfile.py"))
        .arg("--show")
        .arg("3")
        .current_dir(root())
        .output()
        .unwrap_or_else(|e| panic!("running {}: {e}", python.display()));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    println!("{stdout}");
    assert!(
        output.status.success(),
        "oracle reported unexplained disagreements (exit {:?})\n{stderr}",
        output.status.code()
    );
    assert!(stdout.contains("no unexplained disagreements"), "{stdout}");
}
