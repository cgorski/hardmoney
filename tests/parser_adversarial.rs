//! Adversarial parsing: hostile input never panics, and valid input
//! survives every round trip.
//!
//! Real FEC data is hostile -- 135 MB files, fields in the wrong column,
//! Windows-1252 bytes, versions like `180.5`, a stray `"` at the end of
//! every record. `CONTRIBUTING.md` rule 1: a malformed filing must produce
//! an `Err` that names the line, never a crash. These property tests throw
//! arbitrary bytes, truncations, and injected structure at every public
//! entry point of the parser and check exactly that; they also build random
//! *valid* filings over random tables, versions, and field values and check
//! that the writer and the streaming reader reproduce them.

use std::io::Cursor;
use std::str::FromStr;

use hardmoney::parser::filing::{decode, split_new_delimited, strip_ant_suffix};
use hardmoney::parser::form::table_for_form_type;
use hardmoney::parser::stream::FilingReader;
use hardmoney::parser::utils::{is_memo_code, normalize_field, normalize_form_type};
use hardmoney::parser::{FecError, Header, Layout, ParseOptions, ParsedLine, SpecVersion};
use hardmoney::{Filing, Table, parse_fec_date, parse_money};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Everything a caller can do with a filing's bytes, none of which may panic.
fn exercise_bytes(bytes: &[u8]) {
    let _ = Filing::parse_bytes(bytes);
    if let Ok(lenient) = Filing::parse_bytes_with(bytes, &ParseOptions::LENIENT) {
        let (filing, skipped) = lenient.into_parts();
        let _ = skipped.len();
        // Everything a consumer might do with a successfully parsed filing.
        let _ = filing.to_fec();
        let _ = filing.is_allowed();
        for line in std::iter::once(&filing.summary).chain(&filing.lines) {
            let _ = line.is_memo();
            let _ = line.to_cells();
            let _ = format!("{line:?}");
            for (name, value) in line.iter() {
                let _ = line.get(name);
                let _ = parse_money(value);
                let _ = parse_fec_date(value);
            }
        }
        let _ = filing.header.original_filing_id();
        let _ = filing.header.amendment_number();
    }
    match FilingReader::with_options(Cursor::new(bytes), ParseOptions::LENIENT) {
        Ok(mut reader) => {
            let _ = reader.preamble().is_allowed();
            for line in reader.by_ref().flatten() {
                let _ = line.to_cells();
            }
            let _ = reader.lines_read();
            let _ = reader.skipped().len();
        }
        Err(e) => {
            let _ = e.line_no();
            let _ = e.to_string();
        }
    }
    if let Ok(mut reader) = FilingReader::new(Cursor::new(bytes)) {
        for item in reader.by_ref() {
            let _ = item.map(|l| l.line_no);
        }
    }
    let _ = decode(bytes);
}

/// Every table that has a layout at `version`.
fn tables_at(version: SpecVersion) -> Vec<Table> {
    Table::ALL
        .iter()
        .copied()
        .filter(|t| *t != Table::Hdr && t.supports_version(version))
        .collect()
}

/// A wire token that dispatches to `table` (from `Table::ALL` names the
/// dispatcher does not accept verbatim, e.g. `SchA`, `TEXT`, `F82`).
fn token_for(table: Table) -> &'static str {
    match table {
        Table::SchA => "SA11AI",
        Table::SchA3L => "SA3L",
        Table::SchB => "SB21B",
        Table::SchC => "SC/10",
        Table::SchC1 => "SC1/10",
        Table::SchC2 => "SC2/10",
        Table::SchD => "SD10",
        Table::SchE => "SE",
        Table::SchF => "SF",
        Table::SchI => "SI",
        Table::SchL => "SL",
        Table::Text => "TEXT",
        Table::F82 => "F8II",
        Table::F83 => "F8III",
        Table::F3 => "F3N",
        Table::F3X => "F3XN",
        Table::F3P => "F3PN",
        Table::F3L => "F3LN",
        Table::F4 => "F4N",
        Table::F5 => "F5N",
        Table::F7 => "F7N",
        Table::F13 => "F13N",
        Table::F1 => "F1N",
        Table::F24 => "F24N",
        Table::F9 => "F9N",
        other => other.as_str(),
    }
}

/// Values that survive the parser's normalisation unchanged: printable
/// FEC-legal characters, no structural characters (delimiter, CR, LF), no
/// surrounding whitespace, not quote-wrapped.
fn wire_safe_value() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[ -~\u{a0}-\u{a8}\u{ad}]{0,30}")
        .expect("valid regex")
        .prop_map(|s| s.trim().to_string())
        .prop_filter("not quote-wrapped", |s| {
            !(s.len() >= 2 && s.starts_with('"') && s.ends_with('"'))
        })
}

/// Electronic versions the bundled tables cover.
fn any_version() -> impl Strategy<Value = SpecVersion> {
    prop_oneof![
        Just(SpecVersion::electronic(3, 0)),
        Just(SpecVersion::electronic(5, 0)),
        Just(SpecVersion::electronic(5, 1)),
        Just(SpecVersion::electronic(5, 2)),
        Just(SpecVersion::electronic(5, 3)),
        Just(SpecVersion::electronic(6, 1)),
        Just(SpecVersion::electronic(6, 2)),
        Just(SpecVersion::electronic(6, 3)),
        Just(SpecVersion::electronic(6, 4)),
        Just(SpecVersion::electronic(7, 0)),
        Just(SpecVersion::electronic(8, 0)),
        Just(SpecVersion::electronic(8, 1)),
        Just(SpecVersion::electronic(8, 2)),
        Just(SpecVersion::electronic(8, 3)),
        Just(SpecVersion::electronic(8, 4)),
        Just(SpecVersion::electronic(8, 5)),
    ]
}

/// The cover-line table for a version: F3X exists in every bucket.
fn cover_for(version: SpecVersion) -> ParsedLine {
    ParsedLine::from_pairs(
        Table::F3X,
        version,
        2,
        [
            ("form_type", "F3XN"),
            ("filer_committee_id_number", "C00123456"),
        ],
    )
    .expect("F3X has a layout at every bundled version")
}

/// A random body line of a random table at `version`, with random values
/// in a random subset of its fields.
fn random_line(version: SpecVersion) -> impl Strategy<Value = ParsedLine> {
    let tables = tables_at(version);
    proptest::sample::select(tables).prop_flat_map(move |table| {
        let layout: &'static Layout = table.layout(version).expect("selected from tables_at");
        let names: Vec<&'static str> = layout
            .fields
            .iter()
            .map(|f| f.name)
            // `form_type`/`rec_type` carry the dispatch token; TEXT's 3.x-5.x
            // `form_type` is the *referenced* form, so leave both alone.
            .filter(|n| !matches!(*n, "form_type" | "rec_type"))
            .collect();
        proptest::collection::vec((proptest::sample::select(names), wire_safe_value()), 0..8)
            .prop_map(move |pairs| {
                let mut line = ParsedLine::from_pairs(table, version, 0, [] as [(&str, &str); 0])
                    .expect("layout exists");
                let token_field = if table == Table::Text {
                    "rec_type"
                } else {
                    "form_type"
                };
                line.set(token_field, token_for(table))
                    .expect("token field exists");
                if table == Table::Text && layout.field("form_type").is_some() {
                    // Old TEXT records name the form they annotate in col 2.
                    line.set("form_type", "F3XN").expect("exists");
                }
                for (name, value) in pairs {
                    line.set(name, &value).expect("name from layout");
                }
                line
            })
    })
}

fn same_records(a: &ParsedLine, b: &ParsedLine) -> bool {
    a.raw_form_type == b.raw_form_type && a.table() == b.table() && a.iter().eq(b.iter())
}

fn header_for(version: SpecVersion) -> Header {
    let raw = version.to_string();
    let fields: Vec<&str> = if version.has_name_delim_header() {
        vec!["HDR", "FEC", &raw, "SoftCo", "1.0", "^", "", "", ""]
    } else {
        vec!["HDR", "FEC", &raw, "SoftCo", "1.0", "", "", ""]
    };
    Header::from_fields(&fields).expect("valid header")
}

// ---------------------------------------------------------------------------
// Never panic
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        exercise_bytes(&bytes);
    }

    /// Bytes drawn from the alphabet a filing is made of, so the parser
    /// gets past the header far more often than with uniform bytes.
    #[test]
    fn filing_shaped_bytes_never_panic(
        text in proptest::string::string_regex(
            "(HDR|F3XN|F3N|SA11AI|SB21B|TEXT|F99|H4|SC/10|ZZZ|\\[BEGINTEXT\\]|\\[ENDTEXT\\]|/\\*|FEC|8\\.5|5\\.3|3\\.00|180\\.5|P3\\.4|\u{1c}|,|\"|\r|\n|\0|[ -~]| |\u{e9}|\u{2019}){0,120}"
        ).expect("valid regex"),
        cp1252 in proptest::bool::ANY,
    ) {
        let bytes: Vec<u8> = if cp1252 {
            encoding_rs::WINDOWS_1252.encode(&text).0.into_owned()
        } else {
            text.into_bytes()
        };
        exercise_bytes(&bytes);
    }

    #[test]
    fn field_level_functions_never_panic(s in "\\PC{0,64}", bytes in proptest::collection::vec(any::<u8>(), 0..32)) {
        let _ = Header::from_fields(&[&s, &s, &s, &s, &s, &s, &s, &s, &s]);
        let _ = Header::from_fields(&[]);
        let _ = SpecVersion::from_str(&s);
        let _ = parse_money(&s);
        let _ = parse_fec_date(&s);
        let _ = normalize_field(&s);
        let _ = normalize_form_type(&s);
        let _ = is_memo_code(&s);
        let _ = strip_ant_suffix(&s);
        let _ = table_for_form_type(&s);
        let _ = split_new_delimited(&s);
        let lossy = String::from_utf8_lossy(&bytes);
        let _ = SpecVersion::from_str(&lossy);
        let _ = parse_money(&lossy);
        let _ = parse_fec_date(&lossy);
        let _ = normalize_field(&lossy);
        let _ = strip_ant_suffix(&lossy);
        let _ = table_for_form_type(&lossy);
        let _ = decode(&bytes);
    }

    /// `normalize_field` and friends return borrowed slices, so a
    /// non-ASCII character adjacent to a quote or a space must not be
    /// sliced mid-codepoint.
    #[test]
    fn normalisation_respects_char_boundaries(
        prefix in "[ \t\"]{0,3}", body in "\\PC{0,20}", suffix in "[ \t\"]{0,3}"
    ) {
        let s = format!("{prefix}{body}{suffix}");
        let n = normalize_field(&s);
        prop_assert!(s.contains(n));
        prop_assert!(!n.starts_with(|c: char| c.is_ascii_whitespace()));
        prop_assert!(!n.ends_with(|c: char| c.is_ascii_whitespace()));
        let _ = normalize_form_type(&s);
        let _ = strip_ant_suffix(&s);
    }

    #[test]
    fn spec_version_roundtrips_its_display(major in 0u8..=255, minor in 0u8..=9, paper in proptest::bool::ANY) {
        let v = if paper { SpecVersion::paper(major, minor) } else { SpecVersion::electronic(major, minor) };
        prop_assert_eq!(v.to_string().parse::<SpecVersion>(), Ok(v));
        prop_assert_eq!(SpecVersion::from_str(&format!("{v}.0.1")), Ok(v));
    }

    #[test]
    fn parse_money_agrees_with_decimal_on_well_formed_input(
        cents in -999_999_999_999i64..=999_999_999_999i64, plus in proptest::bool::ANY
    ) {
        let s = format!("{}{}.{:02}", if plus && cents >= 0 { "+" } else { "" }, cents / 100, (cents % 100).abs());
        let expected = rust_decimal::Decimal::new(cents, 2);
        prop_assert_eq!(parse_money(&s), Some(expected), "{}", s);
        prop_assert_eq!(parse_money(&format!(" {s} ")), Some(expected));
    }

    #[test]
    fn parse_money_rejects_anything_with_more_than_two_decimals_or_junk(s in "[0-9]{1,6}\\.[0-9]{3,5}|[0-9]*[a-zA-Z$,][0-9]*|--[0-9]+|[0-9]+-") {
        prop_assert_eq!(parse_money(&s), None, "{}", s);
    }

    #[test]
    fn parse_fec_date_accepts_exactly_valid_yyyymmdd(y in 1i32..=9999, m in 1u32..=12, d in 1u32..=31) {
        let s = format!("{y:04}{m:02}{d:02}");
        let expected = chrono::NaiveDate::from_ymd_opt(y, m, d);
        prop_assert_eq!(parse_fec_date(&s), expected, "{}", s);
    }
}

// ---------------------------------------------------------------------------
// Structural mutation of a valid filing
// ---------------------------------------------------------------------------

fn sample_filing_text() -> String {
    [
        "HDR\u{1c}FEC\u{1c}8.5\u{1c}FECfile\u{1c}8.5.1.0(f34)\u{1c}FEC-1234567\u{1c}1\u{1c}",
        "F99\u{1c}C00944124\u{1c}REVIVE OREGON\u{1c}PO BOX 26141\u{1c}\u{1c}ALEXANDRIA\u{1c}VA\u{1c}22313\u{1c}MARSTON\u{1c}CHRIS\u{1c}\u{1c}\u{1c}\u{1c}20260914\u{1c}MST\u{1c}\u{1c}",
        "[BEGINTEXT]",
        "Free text, with a \"quote\" and a comma, spanning",
        "two lines.",
        "[ENDTEXT]",
        "SA11AI\u{1c}C00944124\u{1c}A1\u{1c}\u{1c}\u{1c}IND\u{1c}\u{1c}Smith\u{1c}Jane\u{1c}\u{1c}\u{1c}\u{1c}1 Main St\u{1c}\u{1c}Town\u{1c}VA\u{1c}22313\u{1c}P2026\u{1c}\u{1c}20260101\u{1c}250.00\u{1c}250.00",
        "TEXT\u{1c}C00944124\u{1c}T1\u{1c}A1\u{1c}SA11AI\u{1c}memo about A1",
        "SB21B\u{1c}C00944124\u{1c}B1\u{1c}\u{1c}\u{1c}ORG\u{1c}Vendor, Inc.",
    ]
    .join("\r\n")
}

fn sample_csv_filing_text() -> String {
    [
        "HDR,FEC,5.3,SoftCo,1.0,^,FEC-24088,1",
        "\"F3XA\",\"C00123456\",\"Merck PAC, The Political Action Committee\"",
        "\"SA11AI\",\"C00123456\",\"IND\",\"O'Brien \"\"Bob\"\", Jr.\",\"1 Main St\"",
        "SB21B,C00123456,ORG,\"Vendor, Inc.\",,,Town,VA,22313",
    ]
    .join("\r\n")
}

#[test]
fn truncating_a_valid_filing_at_every_byte_never_panics() {
    for text in [sample_filing_text(), sample_csv_filing_text()] {
        let bytes = text.as_bytes();
        for cut in 0..=bytes.len() {
            exercise_bytes(&bytes[..cut]);
        }
    }
    // The same for a real fixture with Windows-1252 bytes, every 7th byte
    // (the fixture is ~2.6 KB).
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/F99_2011828.fec"
    );
    let fixture = std::fs::read(path).expect("fixture");
    for cut in (0..=fixture.len()).step_by(7) {
        exercise_bytes(&fixture[..cut]);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn injecting_structure_at_random_positions_never_panics(
        which in 0usize..2,
        edits in proptest::collection::vec((0usize..1200, 0usize..9), 1..6),
    ) {
        let mut bytes = if which == 0 { sample_filing_text() } else { sample_csv_filing_text() }.into_bytes();
        for (pos, kind) in edits {
            let insert: &[u8] = match kind {
                0 => b"\x1c",
                1 => b"\"",
                2 => b"\r",
                3 => b"\n",
                4 => b"\0",
                5 => b"\xff\xfe",          // invalid UTF-8
                6 => b"\x92",              // cp1252 curly quote, invalid UTF-8 alone
                7 => b"[BEGINTEXT]\r\n",
                _ => b"[ENDTEXT]\r\n",
            };
            let at = pos.min(bytes.len());
            bytes.splice(at..at, insert.iter().copied());
        }
        exercise_bytes(&bytes);
    }

    /// Whatever the mutation, a successful parse re-parses to itself.
    #[test]
    fn mutated_filings_that_parse_still_round_trip(
        edits in proptest::collection::vec((0usize..600, 0usize..6), 0..4),
    ) {
        let mut bytes = sample_filing_text().into_bytes();
        for (pos, kind) in edits {
            let insert: &[u8] = match kind {
                0 => b"\x1c",
                1 => b"\"",
                2 => b"\r\n",
                3 => b" ",
                4 => b"\xe9",
                _ => b"\n\nSB21B\x1cC00944124\x1cB9\n",
            };
            let at = pos.min(bytes.len());
            bytes.splice(at..at, insert.iter().copied());
        }
        if let Ok(filing) = Filing::parse_bytes(&bytes) {
            let again = Filing::parse_bytes(&filing.to_fec()).expect("canonical output parses");
            prop_assert_eq!(&filing.header, &again.header);
            prop_assert!(same_records(&filing.summary, &again.summary));
            prop_assert_eq!(filing.lines.len(), again.lines.len());
            for (a, b) in filing.lines.iter().zip(&again.lines) {
                prop_assert!(same_records(a, b), "{:?}\n{:?}", a, b);
            }
        }
    }
}

#[test]
fn a_five_megabyte_single_line_does_not_blow_the_stack() {
    let mut text = String::from("HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF3XN\u{1c}C00123456\u{1c}");
    text.push_str(&"N".repeat(5 * 1024 * 1024));
    text.push('\n');
    let filing = Filing::parse(&text).expect("long field is data");
    assert_eq!(
        filing.summary.get("committee_name").map(str::len),
        Some(5 * 1024 * 1024)
    );
    let _ = filing.to_fec();
    // One 5 MB token with no delimiter at all, on both delimiter paths.
    let mut huge_token = String::from("HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\n");
    huge_token.push_str(&"Z".repeat(5 * 1024 * 1024));
    assert!(matches!(
        Filing::parse(&huge_token),
        Err(FecError::ParserMissing { .. })
    ));
    let mut huge_csv = String::from("HDR,FEC,5.3,X,1\nF3XN,C00123456,\"");
    huge_csv.push_str(&"a,".repeat(2 * 1024 * 1024));
    huge_csv.push('\n');
    let filing = Filing::parse(&huge_csv).expect("an unterminated quote runs to end of line");
    assert!(
        filing
            .summary
            .get("committee_name")
            .is_some_and(|v| v.len() > 4_000_000)
    );
    let streamed: Vec<_> = FilingReader::new(Cursor::new(huge_csv.as_bytes()))
        .expect("streams")
        .collect();
    assert!(streamed.is_empty());
}

// ---------------------------------------------------------------------------
// Random valid filings round-trip
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn random_valid_filings_round_trip_through_writer_and_reader(
        (version, lines) in any_version().prop_flat_map(|v| {
            (Just(v), proptest::collection::vec(random_line(v), 0..12))
        })
    ) {
        let header = header_for(version);
        let mut cover = cover_for(version);
        cover.set("committee_name", "Friends of Round Trips").expect("field exists");
        let lines: Vec<ParsedLine> = lines
            .into_iter()
            .enumerate()
            .map(|(i, mut l)| {
                l.line_no = 3 + i as u64;
                l
            })
            .collect();
        let filing = Filing::from_parts(header, cover, lines).expect("consistent parts");

        let bytes = filing.to_fec();
        let again = Filing::parse_bytes(&bytes).expect("canonical output parses");
        prop_assert_eq!(&filing.header, &again.header);
        prop_assert_eq!(filing.version, again.version);
        prop_assert!(same_records(&filing.summary, &again.summary), "cover");
        prop_assert_eq!(filing.lines.len(), again.lines.len());
        for (a, b) in filing.lines.iter().zip(&again.lines) {
            prop_assert!(same_records(a, b), "{:?}\n{:?}", a, b);
        }
        // Body lines are re-numbered by physical position; every record is
        // exactly one line in canonical output.
        for (i, b) in again.lines.iter().enumerate() {
            prop_assert_eq!(b.line_no, 3 + i as u64);
        }

        let streamed = FilingReader::new(Cursor::new(&bytes)).expect("streams");
        prop_assert_eq!(&streamed.preamble().summary, &again.summary);
        let streamed: Vec<ParsedLine> = streamed.collect::<Result<_, _>>().expect("streams");
        prop_assert_eq!(&streamed, &again.lines);

        // Idempotent.
        prop_assert_eq!(again.to_fec(), bytes);
    }
}

// ---------------------------------------------------------------------------
// Every FecError variant can be provoked
// ---------------------------------------------------------------------------

#[test]
fn every_error_variant_is_reachable() {
    let hdr = "HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1";

    assert!(matches!(
        Filing::parse("/* FEC_Ver_# = 2.02"),
        Err(FecError::DeprecatedHeaderFormat)
    ));
    assert!(matches!(
        Filing::parse("HDR\u{1c}FEC\u{1c}180.5\u{1c}X\nF3XN\u{1c}C1"),
        Err(FecError::UnknownElectronicHeaderVersion(v)) if v == "180.5"
    ));
    assert!(matches!(
        Filing::parse("HDR\u{1c}FEC\u{1c}P3.4\u{1c}X\nF3XN\u{1c}C1"),
        Err(FecError::UnknownElectronicHeaderVersion(_))
    ));
    assert!(matches!(Filing::parse(""), Err(FecError::MissingFormLine)));
    assert!(matches!(Filing::parse(hdr), Err(FecError::MissingFormLine)));
    assert!(matches!(
        Filing::parse(&format!("{hdr}\n\u{1c}C1")),
        Err(FecError::MissingFormLine)
    ));
    assert!(matches!(
        Filing::parse(&format!("{hdr}\nZZZ\u{1c}C1")),
        Err(FecError::ParserMissing {
            line_no: Some(2),
            ..
        })
    ));
    assert!(matches!(
        Filing::parse(&format!("{hdr}\nF3XN\u{1c}C1\nZZZ\u{1c}C1")),
        Err(FecError::ParserMissing {
            line_no: Some(3),
            ..
        })
    ));
    // Schedule I was dropped in 8.5: table exists, no layout at this version.
    assert!(matches!(
        Filing::parse(&format!("{hdr}\nF3XN\u{1c}C1\nSI\u{1c}C1")),
        Err(FecError::NoMatchingVersionBucket {
            table: Table::SchI,
            line_no: Some(3),
            ..
        })
    ));
    assert!(matches!(
        Filing::parse(&format!("{hdr}\nF99\u{1c}C1\n[BEGINTEXT]\nnever closed")),
        Err(FecError::UnterminatedTextBlock { line_no: 3 })
    ));
    let mut line =
        ParsedLine::from_pairs(Table::SchA, SpecVersion::electronic(8, 5), 3, []).unwrap();
    assert!(matches!(
        line.set("no_such_field", "x"),
        Err(FecError::UnknownField {
            table: Table::SchA,
            ..
        })
    ));
    let lenient = Filing::parse_with(
        &format!("{hdr}\nF3XN\u{1c}C1\nZZZ\u{1c}C1\nSI\u{1c}C1"),
        &ParseOptions::LENIENT,
    )
    .unwrap();
    assert!(matches!(
        lenient.into_strict(),
        Err(FecError::LinesSkipped { count: 2, .. })
    ));
    assert!(matches!(
        Filing::open("/definitely/not/here.fec"),
        Err(FecError::Io(_))
    ));
    // `Csv` is the `From<csv::Error>` conversion. Since every comma-
    // delimited line is split on its own from an in-memory UTF-8 string
    // the parser can no longer trigger it; the variant remains for callers
    // that match on it and for the conversion itself.
    let csv_err: FecError = csv::Error::from(std::io::Error::other("disk on fire")).into();
    assert!(matches!(csv_err, FecError::Csv(_)));
    assert_eq!(csv_err.line_no(), None);
    assert!(csv_err.to_string().contains("csv error"));
}
