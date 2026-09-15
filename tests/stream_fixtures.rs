//! The streaming parser (`FilingReader` / `Filing::open`) against real
//! filings: on every fixture it must produce exactly what the eager
//! parser produces from the same bytes -- same preamble, same number of
//! lines, every line equal -- and it must do so under the same options,
//! with the same skips, and with `filter_tables` agreeing with
//! `Filing::lines_for`.
//!
//! The 135 MB presidential F3P in `tmp/agent-misc/filings/2010101.fec`
//! (700k lines) is used when present and skipped otherwise; it is not a
//! committed fixture.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::Instant;

use hardmoney::parser::stream::FilingReader;
use hardmoney::parser::{FecError, SkippedLine};
use hardmoney::{Filing, ParseOptions, Table};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fixture_paths(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "fec"))
        .collect();
    paths.sort();
    assert!(
        paths.len() >= 17,
        "expected at least 17 fixtures in {}, found {}",
        dir.display(),
        paths.len()
    );
    paths
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn open_reader(path: &Path, options: ParseOptions) -> FilingReader<BufReader<File>> {
    let file = File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    FilingReader::with_options(BufReader::new(file), options)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Field-for-field equality of two filings (`Filing` has no `PartialEq`;
/// `ParsedLine` does).
fn assert_filings_equal(streamed: &Filing, eager: &Filing, what: &str) {
    assert_eq!(streamed.header, eager.header, "{what}: header");
    assert_eq!(streamed.version, eager.version, "{what}: version");
    assert_eq!(
        streamed.raw_form_type, eager.raw_form_type,
        "{what}: raw_form_type"
    );
    assert_eq!(
        streamed.base_form_type, eager.base_form_type,
        "{what}: base_form_type"
    );
    assert_eq!(
        streamed.is_amendment, eager.is_amendment,
        "{what}: is_amendment"
    );
    assert_eq!(
        streamed.amends_filing, eager.amends_filing,
        "{what}: amends_filing"
    );
    assert_eq!(streamed.summary, eager.summary, "{what}: summary");
    assert_eq!(
        streamed.lines.len(),
        eager.lines.len(),
        "{what}: line count"
    );
    for (i, (s, e)) in streamed.lines.iter().zip(&eager.lines).enumerate() {
        assert_eq!(s, e, "{what}: body line #{i} (physical line {})", e.line_no);
    }
}

/// `Filing::open` (streaming from disk) == `Filing::parse_bytes` (eager
/// from memory) on every fixture, strictly.
#[test]
fn open_equals_parse_bytes_on_every_fixture() {
    for path in fixture_paths(&fixtures_dir()) {
        let name = path.display().to_string();
        let eager = Filing::parse_bytes(&read(&path)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let streamed = Filing::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_filings_equal(&streamed, &eager, &name);
    }
}

/// The preamble is complete before iteration: for every fixture it equals
/// the eager filing's header/summary (including the F99 text block).
#[test]
fn preamble_is_complete_before_the_first_body_line() {
    for path in fixture_paths(&fixtures_dir()) {
        let name = path.display().to_string();
        let eager = Filing::parse_bytes(&read(&path)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let reader = open_reader(&path, ParseOptions::STRICT);
        let p = reader.preamble();
        assert_eq!(p.header, eager.header, "{name}");
        assert_eq!(p.summary, eager.summary, "{name}");
        assert_eq!(p.raw_form_type, eager.raw_form_type, "{name}");
        assert_eq!(p.is_allowed(), eager.is_allowed(), "{name}");
        assert!(reader.lines_read() >= 2, "{name}");
    }
}

/// The two F99 fixtures carry `[BEGINTEXT]` blocks; the streaming reader
/// must have spliced them into the cover line.
#[test]
fn f99_text_blocks_are_spliced_into_the_cover_line() {
    let mut seen = 0;
    for path in fixture_paths(&fixtures_dir()) {
        if !path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("F99"))
        {
            continue;
        }
        let reader = open_reader(&path, ParseOptions::STRICT);
        let text = reader.preamble().summary.get("text").unwrap_or("");
        assert!(!text.is_empty(), "{}: empty F99 text", path.display());
        seen += 1;
    }
    assert_eq!(seen, 2, "expected two F99 fixtures");
}

/// Lenient streaming agrees with lenient eager parsing: same lines, same
/// skipped list (which is empty for every fixture -- they all dispatch).
#[test]
fn lenient_open_matches_lenient_parse_on_every_fixture() {
    for path in fixture_paths(&fixtures_dir()) {
        let name = path.display().to_string();
        let (eager, eager_skipped) = Filing::parse_bytes_with(&read(&path), &ParseOptions::LENIENT)
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .into_parts();
        let (streamed, skipped) = Filing::open_with(&path, ParseOptions::LENIENT)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_filings_equal(&streamed, &eager, &name);
        assert_eq!(skipped, eager_skipped, "{name}: skipped lines");
    }
}

/// `filter_tables` yields exactly `Filing::lines_for(table)`, in order,
/// for every table that occurs in each fixture.
#[test]
fn filter_tables_matches_lines_for() {
    for path in fixture_paths(&fixtures_dir()) {
        let name = path.display().to_string();
        let eager = Filing::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));

        let mut tables: Vec<Table> = eager.lines.iter().map(|l| l.table()).collect();
        tables.sort_unstable();
        tables.dedup();

        for table in tables {
            let expected: Vec<_> = eager.lines_for(table).collect();
            let got = open_reader(&path, ParseOptions::STRICT)
                .filter_tables([table])
                .collect::<Result<Vec<_>, _>>()
                .unwrap_or_else(|e| panic!("{name}/{table}: {e}"));
            assert_eq!(got.len(), expected.len(), "{name}: {table} count");
            for (g, e) in got.iter().zip(expected) {
                assert_eq!(g, e, "{name}: {table} line {}", e.line_no);
            }
        }

        // Several tables at once are yielded in file order, not grouped.
        let got = open_reader(&path, ParseOptions::STRICT)
            .filter_tables([Table::SchA, Table::SchB])
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected: Vec<_> = eager
            .lines
            .iter()
            .filter(|l| matches!(l.table(), Table::SchA | Table::SchB))
            .collect();
        assert_eq!(
            got.iter().collect::<Vec<_>>(),
            expected,
            "{name}: SchA+SchB"
        );
    }
}

/// `lines_read` ends at the file's physical line count for every ASCII-28
/// fixture (for comma-delimited ones it is at least the last record's
/// line).
#[test]
fn lines_read_reaches_the_end_of_the_file() {
    for path in fixture_paths(&fixtures_dir()) {
        let name = path.display().to_string();
        let bytes = read(&path);
        let physical = bytes.iter().filter(|&&b| b == b'\n').count() as u64
            + u64::from(!bytes.ends_with(b"\n"));
        let mut reader = open_reader(&path, ParseOptions::STRICT);
        let last_line_no = reader
            .by_ref()
            .map(|l| l.unwrap_or_else(|e| panic!("{name}: {e}")).line_no)
            .max()
            .unwrap_or(2);
        let read = reader.lines_read();
        assert!(
            read >= last_line_no,
            "{name}: read {read} < last {last_line_no}"
        );
        if reader.preamble().version.uses_fs_delimiter() {
            assert_eq!(read, physical, "{name}");
        } else {
            assert!(
                read <= physical,
                "{name}: read {read} > physical {physical}"
            );
        }
    }
}

/// An unterminated `[BEGINTEXT]` block is an error with the block's
/// starting line, whether it follows the cover (construction fails) or a
/// body line (the iterator fails after releasing that line).
#[test]
fn unterminated_text_block_is_an_error_with_its_line() {
    let f99 = fixtures_dir().join("F99_2011828.fec");
    let text = String::from_utf8(read(&f99)).expect("fixture is UTF-8");
    let begin = text
        .lines()
        .position(|l| l.trim().eq_ignore_ascii_case("[BEGINTEXT]"))
        .expect("F99 fixture has a text block");
    let truncated: String = text.lines().take(begin + 2).collect::<Vec<_>>().join("\n");
    let begin_line_no = begin as u64 + 1;

    match FilingReader::new(truncated.as_bytes()) {
        Err(FecError::UnterminatedTextBlock { line_no }) => assert_eq!(line_no, begin_line_no),
        other => panic!(
            "expected UnterminatedTextBlock, got {:?}",
            other.map(|_| ())
        ),
    }
    assert!(matches!(
        Filing::parse(&truncated),
        Err(FecError::UnterminatedTextBlock { line_no }) if line_no == begin_line_no
    ));

    // Same block after a body line: the line comes out, then the error.
    let mut with_body = text.lines().take(begin).collect::<Vec<_>>().join("\n");
    with_body.push_str("\nTEXT\u{1c}C00944124\u{1c}T1\n[BEGINTEXT]\nnever closed\n");
    let mut reader = FilingReader::new(with_body.as_bytes()).expect("opens");
    assert!(matches!(reader.next(), Some(Ok(l)) if l.table() == Table::Text));
    assert!(matches!(
        reader.next(),
        Some(Err(FecError::UnterminatedTextBlock { line_no })) if line_no == begin_line_no + 1
    ));
    assert!(reader.next().is_none());
}

/// Lenient skipping on a real filing with junk injected: the unknown line
/// is recorded with its physical line number and everything else parses
/// exactly as before.
#[test]
fn lenient_skipping_on_a_real_filing() {
    let path = fixtures_dir().join("F3XN_2011831.fec");
    let text = String::from_utf8(read(&path)).expect("fixture is UTF-8");
    let mut lines: Vec<&str> = text.lines().collect();
    lines.insert(10, "ZZZ\u{1c}not a real record");
    lines.insert(20, "SI\u{1c}C00123456\u{1c}no 8.5 layout for Schedule I");
    let mutated = lines.join("\n");

    let mut strict = FilingReader::new(mutated.as_bytes()).expect("opens");
    let strict_err = strict.by_ref().find_map(Result::err).expect("strict fails");
    assert!(
        matches!(
            strict_err,
            FecError::ParserMissing {
                line_no: Some(11),
                ..
            }
        ),
        "{strict_err}"
    );
    assert!(strict.next().is_none(), "fused after the error");

    let mut reader =
        FilingReader::with_options(mutated.as_bytes(), ParseOptions::LENIENT).expect("opens");
    let parsed = reader
        .by_ref()
        .collect::<Result<Vec<_>, _>>()
        .expect("lenient never yields Err for a bad line");
    let skipped: Vec<SkippedLine> = reader.skipped().to_vec();
    assert_eq!(skipped.len(), 2, "{skipped:?}");
    assert_eq!(skipped[0].line_no, 11);
    assert_eq!(skipped[0].raw_form_type, "ZZZ");
    assert_eq!(skipped[1].line_no, 21);
    assert_eq!(skipped[1].raw_form_type, "SI");

    let (eager, eager_skipped) = Filing::parse_with(&mutated, &ParseOptions::LENIENT)
        .expect("eager lenient parses")
        .into_parts();
    assert_eq!(parsed, eager.lines);
    assert_eq!(skipped, eager_skipped);
}

/// The 135 MB, 700k-line F3P: streaming must agree with eager parsing on
/// every line, and its filtered pass must agree with `lines_for`. Skipped
/// when the file is not present (it is not committed).
#[test]
fn large_presidential_filing_if_present() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tmp/agent-misc/filings/2010101.fec");
    if !path.is_file() {
        eprintln!("skipping: {} not present", path.display());
        return;
    }

    let t = Instant::now();
    let bytes = read(&path);
    let eager = Filing::parse_bytes(&bytes).unwrap_or_else(|e| panic!("eager: {e}"));
    let eager_time = t.elapsed();
    drop(bytes);

    let t = Instant::now();
    let mut reader = open_reader(&path, ParseOptions::STRICT);
    assert_eq!(reader.preamble().summary, eager.summary);
    let mut n = 0usize;
    for (i, line) in reader.by_ref().enumerate() {
        let line = line.unwrap_or_else(|e| panic!("stream: {e}"));
        let expected = eager
            .lines
            .get(i)
            .unwrap_or_else(|| panic!("stream yielded more than {} lines", eager.lines.len()));
        assert_eq!(&line, expected, "line #{i} (physical {})", expected.line_no);
        n += 1;
    }
    let stream_time = t.elapsed();
    assert_eq!(n, eager.lines.len());
    assert!(reader.skipped().is_empty());

    let t = Instant::now();
    let sch_a = open_reader(&path, ParseOptions::STRICT)
        .filter_tables([Table::SchA])
        .count();
    let filtered_time = t.elapsed();
    assert_eq!(sch_a, eager.lines_for(Table::SchA).count());

    eprintln!(
        "2010101.fec: {} lines; eager {:.2?}, streaming {:.2?}, streaming SchA-only {:.2?}",
        n, eager_time, stream_time, filtered_time
    );
}

// Peak-memory measurements. Each is `#[ignore]`d so it runs alone, in its
// own process, under an external meter:
//
//   cargo test --release --test stream_fixtures --all-features \
//       measure_eager -- --ignored --exact --nocapture
//   /usr/bin/time -l target/release/deps/stream_fixtures-* measure_eager --ignored --exact
//
// (`/usr/bin/time -l` on macOS prints "maximum resident set size"; on Linux
// use `/usr/bin/time -v`.) Both no-op when the large file is absent.

fn large_filing() -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tmp/agent-misc/filings/2010101.fec");
    path.is_file().then_some(path)
}

/// Peak RSS of `Filing::parse_bytes` on the 135 MB filing.
#[test]
#[ignore = "memory measurement; run alone under /usr/bin/time -l"]
fn measure_eager() {
    let Some(path) = large_filing() else { return };
    let t = Instant::now();
    let filing = Filing::parse_bytes(&read(&path)).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("eager: {} lines in {:.2?}", filing.lines.len(), t.elapsed());
}

/// Peak RSS of one streaming pass that keeps nothing (a running total).
#[test]
#[ignore = "memory measurement; run alone under /usr/bin/time -l"]
fn measure_stream() {
    let Some(path) = large_filing() else { return };
    let t = Instant::now();
    let mut n = 0u64;
    let mut total = rust_decimal::Decimal::ZERO;
    for line in open_reader(&path, ParseOptions::STRICT).filter_tables([Table::SchA]) {
        let line = line.unwrap_or_else(|e| panic!("{e}"));
        if let Ok(a) = line.view::<hardmoney::ScheduleA>() {
            total += a.contribution_amount.unwrap_or_default();
        }
        n += 1;
    }
    eprintln!(
        "stream: {n} Schedule A lines, total {total}, in {:.2?}",
        t.elapsed()
    );
}

/// Peak RSS of `Filing::open` (streaming from disk into an eager `Filing`).
#[test]
#[ignore = "memory measurement; run alone under /usr/bin/time -l"]
fn measure_open() {
    let Some(path) = large_filing() else { return };
    let t = Instant::now();
    let filing = Filing::open(&path).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("open: {} lines in {:.2?}", filing.lines.len(), t.elapsed());
}
