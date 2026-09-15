//! `hardmoney::export` against real filings: every format must reproduce
//! exactly the rows the parser yields -- same count per table, same values
//! -- and Parquet's typed columns must hold the exact amounts and dates the
//! filing carries. Read-back goes through the same third-party readers a
//! user would use (`csv`, `serde_json`, `parquet`, `rusqlite`).
//!
//! Outputs go under `std::env::temp_dir()` in a directory unique per test
//! (process id + counter) and are removed on success.

#![cfg(feature = "export")]

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use arrow::array::{Array, AsArray, Date32Array, Decimal128Array, Int64Array};
use arrow::datatypes::{DataType, Date32Type, Decimal128Type, Int64Type};
use hardmoney::export::{
    ExportError, ExportOptions, ExportReport, Format, export_path, export_reader,
    filing_id_from_path, resolve_table,
};
use hardmoney::parser::stream::FilingReader;
use hardmoney::parser::{FecError, FieldKind};
use hardmoney::{Filing, ParseOptions, Table, parse_fec_date, parse_money};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

const FIXTURE: &str = "F3XA_2011821.fec";

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fixture(name: &str) -> PathBuf {
    fixtures_dir().join(name)
}

fn fixture_paths() -> Vec<PathBuf> {
    let dir = fixtures_dir();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "fec"))
        .collect();
    paths.sort();
    assert!(paths.len() >= 17, "expected at least 17 fixtures");
    paths
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A unique scratch directory, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "hardmoney-export-{}-{n}-{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn parse(path: &Path) -> Filing {
    Filing::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn export(path: &Path, out: &Path, opts: &ExportOptions) -> ExportReport {
    export_path(path, out, opts).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Distinct tables in the filing's body plus the cover's table, in order
/// of first appearance (the order the report uses).
fn tables_in(filing: &Filing) -> Vec<Table> {
    let mut tables = vec![filing.summary.table()];
    for line in &filing.lines {
        if !tables.contains(&line.table()) {
            tables.push(line.table());
        }
    }
    tables
}

/// Rows the exporter must write for `table`: the body lines of that table
/// plus one for the cover if it is the cover's table.
fn expected_rows(filing: &Filing, table: Table) -> u64 {
    let body = filing.lines_for(table).count() as u64;
    body + u64::from(filing.summary.table() == table)
}

fn assert_report_matches(report: &ExportReport, filing: &Filing) {
    assert_eq!(report.skipped, 0);
    let expected: Vec<Table> = tables_in(filing);
    let got: Vec<Table> = report.tables.iter().map(|t| t.table).collect();
    assert_eq!(got, expected, "tables in order of first appearance");
    for t in &report.tables {
        assert_eq!(t.rows, expected_rows(filing, t.table), "{}: rows", t.table);
    }
    assert_eq!(report.rows(), filing.lines.len() as u64 + 1);
}

// ---------------------------------------------------------------------------
// CSV
// ---------------------------------------------------------------------------

fn csv_records(path: &Path) -> (Vec<String>, Vec<Vec<String>>) {
    let mut reader =
        csv::Reader::from_path(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let header: Vec<String> = reader
        .headers()
        .unwrap_or_else(|e| panic!("{e}"))
        .iter()
        .map(str::to_owned)
        .collect();
    let rows: Vec<Vec<String>> = reader
        .records()
        .map(|r| {
            r.unwrap_or_else(|e| panic!("{e}"))
                .iter()
                .map(str::to_owned)
                .collect()
        })
        .collect();
    (header, rows)
}

#[test]
fn csv_writes_one_file_per_table_with_the_parsers_rows() {
    let path = fixture(FIXTURE);
    let filing = parse(&path);
    let scratch = Scratch::new("csv");
    let out = scratch.path("out.csv");
    let report = export(&path, &out, &ExportOptions::new(Format::Csv));
    assert_report_matches(&report, &filing);
    assert_eq!(report.tables.len(), 6, "F3X, SchA, SchB, SchE, SchD, TEXT");

    // The cover: its own file, one row, columns = line_no + layout fields.
    let (header, rows) = csv_records(&out.join("F3X.csv"));
    assert_eq!(rows.len(), 1);
    let expected_header: Vec<&str> = std::iter::once("line_no")
        .chain(filing.summary.field_names())
        .collect();
    assert_eq!(header, expected_header);
    assert_eq!(rows[0][0], "2");
    assert_eq!(rows[0][1], "F3XA");
    assert_eq!(rows[0][2], "C00922229");

    // Every body table: row count, line numbers, and every value verbatim.
    for table in filing
        .lines
        .iter()
        .map(|l| l.table())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let file = out.join(format!("{table}.csv"));
        let (header, rows) = csv_records(&file);
        let lines: Vec<_> = filing.lines_for(table).collect();
        assert_eq!(rows.len(), lines.len(), "{table}: row count");
        assert_eq!(
            header.len(),
            lines[0].iter().len() + 1,
            "{table}: header width"
        );
        for (row, line) in rows.iter().zip(&lines) {
            assert_eq!(row[0], line.line_no.to_string(), "{table}: line_no");
            let values: Vec<&str> = line.iter().map(|(_, v)| v).collect();
            assert_eq!(
                &row[1..],
                values.as_slice(),
                "{table}: line {}",
                line.line_no
            );
        }
    }

    // Bytes in the report are the files' sizes.
    for t in &report.tables {
        let size = std::fs::metadata(out.join(format!("{}.csv", t.table)))
            .unwrap()
            .len();
        assert_eq!(t.bytes, Some(size), "{}: bytes", t.table);
    }
    assert_eq!(
        report.bytes,
        report.tables.iter().map(|t| t.bytes.unwrap()).sum::<u64>()
    );
}

#[test]
fn csv_quotes_fields_that_need_it_and_round_trips_them() {
    // The fixture's F3X cover has a comma in `street_1` ("1215 K Street,
    // Suite 2000") and SchB has a payee with an ampersand and a comma.
    let path = fixture(FIXTURE);
    let filing = parse(&path);
    let scratch = Scratch::new("csv-quoting");
    let out = scratch.path("out.csv");
    export(&path, &out, &ExportOptions::new(Format::Csv));

    let raw = std::fs::read_to_string(out.join("F3X.csv")).unwrap();
    assert!(raw.contains("\"1215 K Street, Suite 2000\""), "{raw}");
    let (header, rows) = csv_records(&out.join("F3X.csv"));
    let col = header.iter().position(|h| h == "street_1").unwrap();
    assert_eq!(rows[0][col], filing.summary.get("street_1").unwrap());
    assert_eq!(rows[0][col], "1215 K Street, Suite 2000");
}

#[test]
fn csv_total_rows_equal_the_parsers_lines_on_every_fixture() {
    for path in fixture_paths() {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let filing = parse(&path);
        let scratch = Scratch::new(&format!("all-{name}"));
        let out = scratch.path("csv");
        let report = export(&path, &out, &ExportOptions::new(Format::Csv));
        assert_report_matches(&report, &filing);

        // Count rows in the files themselves, not just the report.
        let mut total = 0usize;
        for t in &report.tables {
            let (_, rows) = csv_records(&out.join(format!("{}.csv", t.table)));
            assert_eq!(rows.len() as u64, t.rows, "{name}/{}", t.table);
            total += rows.len();
        }
        assert_eq!(total, filing.lines.len() + 1, "{name}: body lines + cover");
    }
}

// ---------------------------------------------------------------------------
// JSON Lines
// ---------------------------------------------------------------------------

#[test]
fn jsonl_rows_parse_back_with_the_parsers_values() {
    let path = fixture(FIXTURE);
    let filing = parse(&path);
    let scratch = Scratch::new("jsonl");
    let out = scratch.path("out.jsonl");
    let report = export(&path, &out, &ExportOptions::new(Format::Jsonl));
    assert_report_matches(&report, &filing);

    for table in tables_in(&filing) {
        let text = std::fs::read_to_string(out.join(format!("{table}.jsonl"))).unwrap();
        let objects: Vec<serde_json::Map<String, serde_json::Value>> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{table}: {e}: {l}")))
            .collect();
        let lines: Vec<_> = std::iter::once(&filing.summary)
            .filter(|s| s.table() == table)
            .chain(filing.lines_for(table))
            .collect();
        assert_eq!(objects.len(), lines.len(), "{table}");
        for (obj, line) in objects.iter().zip(&lines) {
            // Keys in column order: line_no, then the layout's fields.
            let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
            let expected: Vec<&str> = std::iter::once("line_no")
                .chain(line.field_names())
                .collect();
            assert_eq!(keys, expected, "{table}: key order");
            assert_eq!(obj["line_no"], serde_json::json!(line.line_no));
            for (name, value) in line.iter() {
                assert_eq!(
                    obj[name],
                    serde_json::Value::String(value.to_string()),
                    "{table}.{name}"
                );
            }
        }
    }

    // A sampled field: the first SB21B's amount is the string as filed.
    let sch_b = std::fs::read_to_string(out.join("SchB.jsonl")).unwrap();
    let first: serde_json::Value = serde_json::from_str(sch_b.lines().next().unwrap()).unwrap();
    let line = filing.lines_for(Table::SchB).next().unwrap();
    assert_eq!(first["expenditure_amount"], "15.00");
    assert_eq!(
        first["expenditure_amount"],
        line.get("expenditure_amount").unwrap()
    );
    assert_eq!(first["payee_organization_name"], "California Bank & Trust");
}

// ---------------------------------------------------------------------------
// Parquet
// ---------------------------------------------------------------------------

fn read_parquet(path: &Path) -> arrow::record_batch::RecordBatch {
    let file = File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap_or_else(|e| panic!("{e}"))
        .build()
        .unwrap_or_else(|e| panic!("{e}"));
    let batches: Vec<_> = reader
        .map(|b| b.unwrap_or_else(|e| panic!("{e}")))
        .collect();
    assert!(!batches.is_empty(), "{}: no batches", path.display());
    arrow::compute::concat_batches(&batches[0].schema(), &batches).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn parquet_types_amounts_as_decimal_and_dates_as_date32() {
    let path = fixture(FIXTURE);
    let filing = parse(&path);
    let scratch = Scratch::new("parquet");
    let out = scratch.path("out.parquet");
    let report = export(&path, &out, &ExportOptions::new(Format::Parquet));
    assert_report_matches(&report, &filing);

    let batch = read_parquet(&out.join("SchB.parquet"));
    let lines: Vec<_> = filing.lines_for(Table::SchB).collect();
    assert_eq!(batch.num_rows(), lines.len());
    let schema = batch.schema();

    // Schema: line_no Int64, then every layout field, typed from the spec.
    assert_eq!(schema.field(0).name(), "line_no");
    assert_eq!(schema.field(0).data_type(), &DataType::Int64);
    let layout = lines[0].layout();
    assert_eq!(schema.fields().len(), layout.fields.len() + 1);
    for (i, f) in layout.fields.iter().enumerate() {
        let field = schema.field(i + 1);
        assert_eq!(field.name(), f.name);
        let expected = match layout.spec(f.name) {
            Some(s) if s.kind == FieldKind::Amount => DataType::Decimal128(38, 2),
            Some(s) if s.kind == FieldKind::Numeric && s.max_len == Some(8) => DataType::Date32,
            _ => DataType::Utf8,
        };
        assert_eq!(field.data_type(), &expected, "SchB.{}", f.name);
    }
    assert_eq!(
        schema
            .field_with_name("expenditure_amount")
            .unwrap()
            .data_type(),
        &DataType::Decimal128(38, 2)
    );
    assert_eq!(
        schema
            .field_with_name("expenditure_date")
            .unwrap()
            .data_type(),
        &DataType::Date32
    );
    assert_eq!(
        schema
            .field_with_name("payee_zip_code")
            .unwrap()
            .data_type(),
        &DataType::Utf8
    );

    // Values: the first SB21B is $15.00 on 2026-05-15.
    let amounts: &Decimal128Array = batch
        .column_by_name("expenditure_amount")
        .unwrap()
        .as_primitive::<Decimal128Type>();
    assert_eq!(amounts.value(0), 1500, "cents");
    assert_eq!(amounts.value_as_string(0), "15.00");
    assert_eq!(lines[0].get("expenditure_amount"), Some("15.00"));
    for (i, line) in lines.iter().enumerate() {
        let expected = parse_money(line.get("expenditure_amount").unwrap());
        let got = amounts
            .is_valid(i)
            .then(|| rust_decimal::Decimal::new(amounts.value(i) as i64, 2));
        assert_eq!(got, expected, "SchB row {i}");
    }
    let dates: &Date32Array = batch
        .column_by_name("expenditure_date")
        .unwrap()
        .as_primitive::<Date32Type>();
    for (i, line) in lines.iter().enumerate() {
        let expected = parse_fec_date(line.get("expenditure_date").unwrap());
        let got = dates.is_valid(i).then(|| dates.value_as_date(i).unwrap());
        assert_eq!(got, expected, "SchB row {i}");
    }
    assert_eq!(dates.value_as_date(0).unwrap().to_string(), "2026-05-15");

    // line_no and text columns are verbatim.
    let line_nos: &Int64Array = batch.column(0).as_primitive::<Int64Type>();
    for (i, line) in lines.iter().enumerate() {
        assert_eq!(line_nos.value(i), line.line_no as i64);
    }
    let names = batch
        .column_by_name("payee_organization_name")
        .unwrap()
        .as_string::<i32>();
    assert_eq!(names.value(0), "California Bank & Trust");

    // A blank amount column is null, not zero: semi_annual_refunded_bundled_amt.
    let blank: &Decimal128Array = batch
        .column_by_name("semi_annual_refunded_bundled_amt")
        .unwrap()
        .as_primitive::<Decimal128Type>();
    assert_eq!(lines[0].get("semi_annual_refunded_bundled_amt"), Some(""));
    assert!(blank.is_null(0));

    // The cover's amounts (F3X totals) are decimals too.
    let cover = read_parquet(&out.join("F3X.parquet"));
    assert_eq!(cover.num_rows(), 1);
    let receipts: &Decimal128Array = cover
        .column_by_name("col_a_total_receipts")
        .unwrap()
        .as_primitive::<Decimal128Type>();
    let expected = parse_money(filing.summary.get("col_a_total_receipts").unwrap()).unwrap();
    assert_eq!(
        rust_decimal::Decimal::new(receipts.value(0) as i64, 2),
        expected
    );
}

#[test]
fn parquet_row_counts_match_on_every_fixture() {
    for path in fixture_paths() {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let filing = parse(&path);
        let scratch = Scratch::new(&format!("pq-{name}"));
        let out = scratch.path("parquet");
        let report = export(&path, &out, &ExportOptions::new(Format::Parquet));
        assert_report_matches(&report, &filing);
        for t in &report.tables {
            let batch = read_parquet(&out.join(format!("{}.parquet", t.table)));
            assert_eq!(batch.num_rows() as u64, t.rows, "{name}/{}", t.table);
        }
    }
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

#[test]
fn sqlite_has_one_table_per_table_plus_filings_metadata() {
    let path = fixture(FIXTURE);
    let filing = parse(&path);
    let scratch = Scratch::new("sqlite");
    let out = scratch.path("out.sqlite");
    let report = export(&path, &out, &ExportOptions::new(Format::Sqlite));
    assert_report_matches(&report, &filing);
    assert!(report.tables.iter().all(|t| t.bytes.is_none()));
    assert_eq!(report.bytes, std::fs::metadata(&out).unwrap().len());

    let conn = rusqlite::Connection::open(&out).unwrap();
    for table in tables_in(&filing) {
        let n: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |r| {
                r.get(0)
            })
            .unwrap_or_else(|e| panic!("{table}: {e}"));
        assert_eq!(n as u64, expected_rows(&filing, table), "{table}");
    }

    // Amounts are TEXT, exactly as filed.
    let (amount, kind): (String, String) = conn
        .query_row(
            "SELECT expenditure_amount, typeof(expenditure_amount) FROM SchB ORDER BY line_no LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(amount, "15.00");
    assert_eq!(kind, "text");
    assert_eq!(
        amount,
        filing
            .lines_for(Table::SchB)
            .next()
            .unwrap()
            .get("expenditure_amount")
            .unwrap()
    );

    // No filing_id column without the option; line_no is INTEGER.
    let cols: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('SchB')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let expected: Vec<&str> = std::iter::once("line_no")
        .chain(filing.lines_for(Table::SchB).next().unwrap().field_names())
        .collect();
    assert_eq!(cols, expected);
    let line_no_type: String = conn
        .query_row("SELECT typeof(line_no) FROM SchB LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(line_no_type, "integer");

    // Metadata row.
    let (id, form, version, committee, src): (
        Option<i64>,
        String,
        String,
        Option<String>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT filing_id, form_type, version, committee_id, path FROM filings",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(id, None);
    assert_eq!(form, "F3XA");
    assert_eq!(version, "8.5");
    assert_eq!(committee.as_deref(), Some("C00922229"));
    assert_eq!(src.as_deref(), Some(path.display().to_string().as_str()));
}

#[test]
fn sqlite_appends_a_second_filing_into_the_same_database() {
    let a = fixture(FIXTURE);
    let b = fixture("F3XA_2011827.fec");
    let scratch = Scratch::new("sqlite-append");
    let out = scratch.path("both.sqlite");
    let opts_a = ExportOptions::new(Format::Sqlite)
        .include_filing_id_from(&a)
        .unwrap();
    let opts_b = ExportOptions::new(Format::Sqlite)
        .include_filing_id_from(&b)
        .unwrap();
    let ra = export(&a, &out, &opts_a);
    let rb = export(&b, &out, &opts_b);

    let conn = rusqlite::Connection::open(&out).unwrap();
    let filings: Vec<i64> = conn
        .prepare("SELECT filing_id FROM filings ORDER BY filing_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(filings, vec![2011821, 2011827]);
    let covers: i64 = conn
        .query_row("SELECT COUNT(*) FROM F3X", [], |r| r.get(0))
        .unwrap();
    assert_eq!(covers, 2);
    let a_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM SchB WHERE filing_id = 2011821",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        a_rows as u64,
        ra.tables
            .iter()
            .find(|t| t.table == Table::SchB)
            .unwrap()
            .rows
    );
    let _ = rb;
}

// ---------------------------------------------------------------------------
// Options: --only, --include-filing-id, --lenient, errors
// ---------------------------------------------------------------------------

#[test]
fn only_restricts_to_the_listed_tables_and_drops_the_cover() {
    let path = fixture(FIXTURE);
    let filing = parse(&path);
    for format in [Format::Csv, Format::Jsonl, Format::Parquet, Format::Sqlite] {
        let scratch = Scratch::new(&format!("only-{format}"));
        let out = scratch.path(&format!("out.{format}"));
        let only = [resolve_table("SA").unwrap(), resolve_table("SchB").unwrap()];
        assert_eq!(only, [Table::SchA, Table::SchB]);
        let report = export(&path, &out, &ExportOptions::new(format).only(only));
        let tables: Vec<Table> = report.tables.iter().map(|t| t.table).collect();
        assert_eq!(tables, vec![Table::SchA, Table::SchB], "{format}");
        assert_eq!(
            report.tables[0].rows,
            filing.lines_for(Table::SchA).count() as u64
        );
        assert_eq!(
            report.tables[1].rows,
            filing.lines_for(Table::SchB).count() as u64
        );
        if format.writes_directory() {
            let names: Vec<String> = std::fs::read_dir(&out)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
                .collect();
            assert_eq!(names.len(), 2, "{format}: {names:?}");
            assert!(
                !names.iter().any(|n| n.starts_with("F3X")),
                "{format}: cover excluded"
            );
        }
    }

    // Listing the cover's table includes it.
    let scratch = Scratch::new("only-cover");
    let out = scratch.path("out.csv");
    let report = export(
        &path,
        &out,
        &ExportOptions::new(Format::Csv).only([Table::F3X]),
    );
    assert_eq!(report.tables.len(), 1);
    assert_eq!(report.tables[0].table, Table::F3X);
    assert_eq!(report.tables[0].rows, 1);
}

#[test]
fn include_filing_id_prepends_the_id_from_the_file_name_in_every_format() {
    let path = fixture(FIXTURE);
    assert_eq!(filing_id_from_path(&path), Some(2011821));
    let filing = parse(&path);
    let opts_for = |f: Format| ExportOptions::new(f).include_filing_id_from(&path).unwrap();

    let scratch = Scratch::new("id-csv");
    let out = scratch.path("out.csv");
    export(&path, &out, &opts_for(Format::Csv));
    let (header, rows) = csv_records(&out.join("SchA.csv"));
    assert_eq!(&header[..3], &["filing_id", "line_no", "form_type"]);
    assert!(rows.iter().all(|r| r[0] == "2011821"));
    assert_eq!(rows.len(), filing.lines_for(Table::SchA).count());

    let scratch = Scratch::new("id-jsonl");
    let out = scratch.path("out.jsonl");
    export(&path, &out, &opts_for(Format::Jsonl));
    let text = std::fs::read_to_string(out.join("F3X.jsonl")).unwrap();
    let obj: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(text.trim()).unwrap();
    assert_eq!(
        obj.keys().take(2).collect::<Vec<_>>(),
        ["filing_id", "line_no"]
    );
    assert_eq!(obj["filing_id"], serde_json::json!(2011821));

    let scratch = Scratch::new("id-parquet");
    let out = scratch.path("out.parquet");
    export(&path, &out, &opts_for(Format::Parquet));
    let batch = read_parquet(&out.join("SchE.parquet"));
    assert_eq!(batch.schema().field(0).name(), "filing_id");
    assert_eq!(batch.schema().field(0).data_type(), &DataType::Int64);
    let ids: &Int64Array = batch.column(0).as_primitive::<Int64Type>();
    assert!(ids.iter().all(|v| v == Some(2011821)));
    assert_eq!(batch.num_rows(), filing.lines_for(Table::SchE).count());

    let scratch = Scratch::new("id-sqlite");
    let out = scratch.path("out.sqlite");
    export(&path, &out, &opts_for(Format::Sqlite));
    let conn = rusqlite::Connection::open(&out).unwrap();
    let (n, distinct): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COUNT(DISTINCT filing_id) FROM SchD WHERE filing_id = 2011821",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(n as usize, filing.lines_for(Table::SchD).count());
    assert_eq!(distinct, 1);
    let meta: i64 = conn
        .query_row("SELECT filing_id FROM filings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(meta, 2011821);
}

#[test]
fn filing_id_that_cannot_be_derived_is_an_error() {
    let err = ExportOptions::new(Format::Csv)
        .include_filing_id_from(Path::new("some/dir/filing.fec"))
        .expect_err("no digits in the name");
    assert!(
        matches!(&err, ExportError::FilingIdNotInFilename { path } if path == Path::new("some/dir/filing.fec"))
    );
    assert!(err.to_string().contains("filing.fec"), "{err}");
    assert_eq!(filing_id_from_path(Path::new("F3X_v8.5.fec")), None);
}

#[test]
fn lenient_skips_junk_lines_and_counts_them_strict_fails() {
    let path = fixture("F3XN_2011831.fec");
    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<&str> = text.lines().collect();
    lines.insert(5, "ZZZ\u{1c}not a real record");
    let mutated = lines.join("\n");
    let filing = Filing::parse_with(&mutated, &ParseOptions::LENIENT)
        .unwrap()
        .into_parts()
        .0;

    let scratch = Scratch::new("lenient");
    let reader = FilingReader::with_options(mutated.as_bytes(), ParseOptions::LENIENT).unwrap();
    let report = export_reader(
        reader,
        &scratch.path("lenient.csv"),
        &ExportOptions::new(Format::Csv),
    )
    .unwrap();
    assert_eq!(report.skipped, 1);
    assert_eq!(report.rows(), filing.lines.len() as u64 + 1);

    let reader = FilingReader::new(mutated.as_bytes()).unwrap();
    let err = export_reader(
        reader,
        &scratch.path("strict.csv"),
        &ExportOptions::new(Format::Csv),
    )
    .err();
    assert!(
        matches!(
            err,
            Some(ExportError::Fec(FecError::ParserMissing {
                line_no: Some(6),
                ..
            }))
        ),
        "{err:?}"
    );

    // `export_path` honours `lenient` itself.
    let junk = scratch.path("junk.fec");
    std::fs::write(&junk, &mutated).unwrap();
    let strict = export_path(
        &junk,
        &scratch.path("s.csv"),
        &ExportOptions::new(Format::Csv),
    );
    assert!(matches!(strict, Err(ExportError::Fec(_))));
    let lenient = export_path(
        &junk,
        &scratch.path("l.csv"),
        &ExportOptions::new(Format::Csv).lenient(true),
    )
    .unwrap();
    assert_eq!(lenient.skipped, 1);
}

#[test]
fn export_reader_leaves_the_sqlite_path_null_and_a_reader_export_matches_a_path_export() {
    let path = fixture("F24N_2011823.fec");
    let filing = parse(&path);
    let scratch = Scratch::new("reader");

    let file = std::io::BufReader::new(File::open(&path).unwrap());
    let reader = FilingReader::new(file).unwrap();
    let out = scratch.path("reader.sqlite");
    let report = export_reader(reader, &out, &ExportOptions::new(Format::Sqlite)).unwrap();
    assert_report_matches(&report, &filing);
    let conn = rusqlite::Connection::open(&out).unwrap();
    let src: Option<String> = conn
        .query_row("SELECT path FROM filings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(src, None);
}

#[test]
fn errors_name_the_output_path_and_unknown_tokens() {
    let path = fixture(FIXTURE);
    // A file where the directory should be.
    let scratch = Scratch::new("io-error");
    let blocker = scratch.path("not-a-dir");
    std::fs::write(&blocker, b"x").unwrap();
    let err = export_path(&path, &blocker, &ExportOptions::new(Format::Csv))
        .expect_err("cannot create dir");
    assert!(matches!(err, ExportError::Io(_)), "{err:?}");
    assert!(err.to_string().contains("not-a-dir"), "{err}");

    // A missing input names the input.
    let missing = scratch.path("missing.fec");
    let err = export_path(
        &missing,
        &scratch.path("o"),
        &ExportOptions::new(Format::Csv),
    )
    .err()
    .unwrap();
    assert!(matches!(err, ExportError::Io(_)), "{err:?}");
    assert!(err.to_string().contains("missing.fec"), "{err}");

    // SQLite: the database path is a directory.
    let err = export_path(&path, &scratch.0, &ExportOptions::new(Format::Sqlite))
        .err()
        .unwrap();
    assert!(matches!(err, ExportError::Sqlite(_)), "{err:?}");

    // Unknown --only token.
    let err = resolve_table("ScheA").err().unwrap();
    assert!(matches!(&err, ExportError::UnknownTable(t) if t == "ScheA"));
    assert!(err.to_string().contains("ScheA"), "{err}");
}

#[test]
fn default_out_follows_the_input_stem() {
    let path = fixture(FIXTURE);
    assert_eq!(
        Format::Csv.default_out(&path),
        PathBuf::from("F3XA_2011821.csv")
    );
    assert_eq!(
        Format::Parquet.default_out(&path),
        PathBuf::from("F3XA_2011821.parquet")
    );
    assert_eq!(
        Format::Sqlite.default_out(&path),
        PathBuf::from("F3XA_2011821.sqlite")
    );
}
