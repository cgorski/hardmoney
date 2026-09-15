# Exporting a filing

A `.fec` file is a stack of records of different shapes (one cover
line, then Schedule A lines, Schedule B lines, `TEXT` records), each
with its own columns. Nothing you would analyse it with wants that shape.
`hardmoney export` turns a filing into one table per record type, in
whichever of four formats your tool reads, and it does so by streaming
(see [Streaming large filings](./streaming.md)), so the 135 MB
presidential filing exports in the same footprint as a 3 KB one.

```text
$ hardmoney export tests/fixtures/F3XA_2011821.fec --format parquet --out /tmp/x
table  rows   bytes
-----  ----  ------
F3X       1  49,868
SchA      5  15,032
SchB      7  14,475
SchE      6  14,247
SchD      5   7,499
TEXT      3   2,901
6 table(s), 27 row(s), 104,022 bytes -> /tmp/x/

$ ls /tmp/x
F3X.parquet   SchA.parquet  SchB.parquet  SchD.parquet  SchE.parquet  TEXT.parquet
```

Each file is named after the `Table` its rows belong to (the names
`hardmoney spec tables` lists). The cover line (the Form 3X summary
here) is the single row of its own table, `F3X`. The Parquet files are
large for 27 rows because each carries its schema and per-column
metadata; on a real filing the ratio inverts (below).

## Which format

| | Use it when | Money is |
|---|---|---|
| `csv` | You want a spreadsheet, `COPY` into Postgres, or the format every tool reads. One `<Table>.csv` per table with a header row, RFC 4180 quoting. | text, exactly as filed (`15.00`) |
| `jsonl` | You are piping into `jq`, a JavaScript/Python script, or a document store. One JSON object per row, keys in column order. | a JSON string (`"15.00"`) |
| `parquet` | You want DuckDB, Polars, pandas, Spark, or an S3 data lake. Columnar, zstd-compressed, and typed: amounts are decimals, dates are dates. | `DECIMAL(38, 2)`, exact |
| `sqlite` | You want to query one or many filings with SQL right now, with nothing to install. One database, one table per `Table`, plus a `filings` table. | text, exactly as filed |

Money is never a float, in any of them. CSV, JSON Lines, and SQLite carry
the string the filer wrote; Parquet carries an exact decimal. This is the
same rule as the rest of the crate: an FEC amount is a decimal currency
value, and binary floating point cannot represent every decimal fraction
exactly. `hardmoney export` never gives you `3697.4999999`.

## Columns

Every output table has the same column set, in this order:

1. `filing_id`, only with `--include-filing-id`.
2. `line_no`, the 1-based physical line in the `.fec` file. Any row
   traces back to the raw bytes; the cover line is always line 2.
3. The table's canonical fields in FEC column order, starting with
   `form_type`. These are the same `lower_snake_case` names
   `ParsedLine::get` and `hardmoney spec fields` use, every field present
   even when blank.

```text
$ hardmoney export tests/fixtures/F3XA_2011821.fec --out /tmp/x.csv >/dev/null
$ head -c 200 /tmp/x.csv/SchB.csv
line_no,form_type,filer_committee_id_number,transaction_id,back_reference_tran_id,back_reference_sched_name,entity_type,payee_organization_name,payee_last_name,payee_first_name,payee_mid
```

All lines of one table in one filing share a layout (a filing has one
spec version), so the header fits every row.

## Parquet: the type mapping

Parquet is the one output with a real type system, so `export` types the
columns from the FEC's own field specifications (the `kind` column of
`hardmoney spec fields`):

| FEC type | Arrow / Parquet type | Blank or unparseable |
|---|---|---|
| `AMT-n` (`kind = amount`) | `Decimal128(38, 2)`, parsed with `parse_money` | `NULL` |
| `NUM-8` (`kind = numeric`, length 8; every such field in the spec is a `YYYYMMDD` date) | `Date32` (days since 1970-01-01), parsed with `parse_fec_date` | `NULL` |
| everything else, including other `NUM-n` | `Utf8` | `""` |
| `line_no`, `filing_id` | `Int64` | n/a |

Other numerics stay text on purpose: they include ZIP codes and
committee ids with leading zeros, and the spec's `NUM` columns are not
all integers in practice. `Decimal128(38, 2)` stores an amount as an
integer number of cents, so `15.00` is exactly `1500` and a `SUM` is
exact wherever you run it. Compression is zstd (default level), which
DuckDB, pyarrow, Polars, and Spark all read; on the 135 MB filing above
it produces a 16.5 MB `SchA.parquet` for 689,776 rows.

DuckDB reads the files directly. These are real results, captured
through DuckDB's Python package (`uvx --from duckdb python -c ...`); the
`duckdb` shell's `-c` prints the same tables:

```text
>>> duckdb.sql("""SELECT column_name, column_type FROM (DESCRIBE SELECT * FROM '/tmp/x/SchB.parquet')
                WHERE column_name IN ('line_no','payee_zip_code','expenditure_date','expenditure_amount')""")
┌────────────────────┬───────────────┐
│    column_name     │  column_type  │
│      varchar       │    varchar    │
├────────────────────┼───────────────┤
│ line_no            │ BIGINT        │
│ payee_zip_code     │ VARCHAR       │
│ expenditure_date   │ DATE          │
│ expenditure_amount │ DECIMAL(38,2) │
└────────────────────┴───────────────┘

>>> duckdb.sql("""SELECT line_no, payee_organization_name, expenditure_date, expenditure_amount
                FROM '/tmp/x/SchB.parquet' ORDER BY line_no LIMIT 3""")
┌─────────┬─────────────────────────┬──────────────────┬────────────────────┐
│ line_no │ payee_organization_name │ expenditure_date │ expenditure_amount │
│  int64  │         varchar         │       date       │   decimal(38,2)    │
├─────────┼─────────────────────────┼──────────────────┼────────────────────┤
│       8 │ California Bank & Trust │ 2026-05-15       │              15.00 │
│       9 │ The Political Law Group │ 2026-05-26       │            3697.50 │
│      10 │ Bedford Grove LLC       │ 2026-06-01       │            7500.00 │
└─────────┴─────────────────────────┴──────────────────┴────────────────────┘

>>> duckdb.sql("""SELECT SUM(expenditure_amount) AS total, typeof(SUM(expenditure_amount)) AS type
                FROM '/tmp/x/SchB.parquet'""")
┌───────────────┬───────────────┐
│     total     │     type      │
│ decimal(38,2) │    varchar    │
├───────────────┼───────────────┤
│      27247.50 │ DECIMAL(38,2) │
└───────────────┴───────────────┘
```

The total stays a decimal end to end.

## SQLite: text amounts and how to sum them

SQLite has no decimal type, and storing an amount as `REAL` would round
it to the nearest binary fraction, so every field column is `TEXT` with
the value exactly as filed; `filing_id` and `line_no` are `INTEGER`.

```text
$ hardmoney export tests/fixtures/F3XA_2011821.fec --format sqlite --out /tmp/x.sqlite --include-filing-id
table  rows
-----  ----
F3X       1
SchA      5
SchB      7
SchE      6
SchD      5
TEXT      3
6 table(s), 27 row(s), 45,056 bytes -> /tmp/x.sqlite

$ sqlite3 /tmp/x.sqlite ".tables"
F3X      SchA     SchB     SchD     SchE     TEXT     filings

$ sqlite3 /tmp/x.sqlite "SELECT filing_id, form_type, version, committee_id, path FROM filings"
2011821|F3XA|8.5|C00922229|tests/fixtures/F3XA_2011821.fec

$ sqlite3 /tmp/x.sqlite "SELECT line_no, payee_organization_name, expenditure_date, expenditure_amount
                         FROM SchB ORDER BY line_no LIMIT 3"
8|California Bank & Trust|20260515|15.00
9|The Political Law Group|20260526|3697.50
10|Bedford Grove LLC|20260601|7500.00
```

To total a column, use the `sqlite3` shell's built-in decimal extension,
which is exact:

```text
$ sqlite3 /tmp/x.sqlite "SELECT decimal_sum(expenditure_amount) FROM SchB"
27247.50
```

`CAST(expenditure_amount AS REAL)` also works and is fine for a quick
look, but it is a double: `SUM(CAST(... AS REAL))` prints `27247.5`
here and will not be cent-exact on a large schedule. If your application
needs exact arithmetic in SQLite without the shell, read the text and
parse it with a decimal type on your side, which is what `parse_money`
does in Rust.

The `filings` table records one row per export: `filing_id`,
`form_type`, `version`, `committee_id` (the cover's
`filer_committee_id_number`), `path`, and `exported_at` (RFC 3339, UTC).
Tables are created `IF NOT EXISTS` and rows appended, so a loop over
many filings with `--include-filing-id` into one `--out all.sqlite`
builds a database you can query across filings. Filings must share a
column layout (the same spec-version bucket) for that to work; a filing
whose layout has a column the existing table lacks fails rather than
silently dropping it. Each export is one transaction: on any error the
database is left as it was.

## Options

`--only SA,SB` (or `--only SchA,SchB`) exports just those tables. A token
is either a table name as `hardmoney spec tables` lists it
(case-insensitive) or an upper-case form-type token as it appears in
column 0 of a line (`SA`, `SB21B`, `SE`, `F3XN`), resolved through the
same dispatch the parser uses. Upper case is required for tokens so that
a mistyped table name is an error rather than a silent wrong table
(`ScheA` would otherwise prefix-match Schedule C):

```text
$ hardmoney export tests/fixtures/F3XA_2011821.fec --only SA,ScheA
error: invalid value 'ScheA' for '--only <ONLY>': 'ScheA' is not a table name (SchA, F3X, TEXT, ...) or an upper-case form-type token (SA, SB21B, SE, ...)
```

The cover line is exported only if its table is listed (`--only
F3X,SA`). Under the hood this is `FilingReader::filter_tables`, so
skipped tables are never parsed into memory.

`--include-filing-id` prepends a `filing_id` column with the id from the
file name: the last run of four or more digits in the stem, so
`2011821.fec` (an FEC download), `F3XA_2011821.fec` (this crate's
fixtures), and `F3XA_27789_v3.fec` all work. A name with no such run is
an error; from the library you pass the id explicitly.

`--lenient` skips body lines that cannot be parsed (an unknown form-type
token, or no column layout for the filing's spec version) and reports
the count on stderr, exactly as `hardmoney parse --lenient` does. See
[Strict vs. lenient parsing](./strict-vs-lenient.md) for why strict is
the default.

`--out` is a directory for `csv`/`jsonl`/`parquet` (created if missing;
existing `<Table>.<ext>` files of the same names are replaced) and a
database file for `sqlite`. The default is `./<file-stem>.<format>`:
`F3XA_2011821.parquet/` or `F3XA_2011821.sqlite`.

## Memory and speed

Export streams through `FilingReader`, holding one parsed record plus
each open table's write buffer. Measured on filing 2010101 (135,241,563
bytes, 704,651 body lines, 689,776 of them Schedule A) with a release
build under `/usr/bin/time -l`:

| Format | Output size | Peak RSS | Wall time |
|---|---|---|---|
| `csv` | 155.3 MB | 13.3 MB | 1.55 s |
| `parquet` | 16.8 MB | 104 MB | 1.6 s |
| `sqlite` | 161.3 MB | 17.5 MB | 1.61 s |

CSV and SQLite are a few kilobytes of buffer per table above the process
itself. Parquet's peak is the encoder's: it buffers one `RecordBatch` of
16,384 rows per table and holds the encoded pages of the current row
group (up to 1,048,576 rows) until it flushes, so the peak is bounded by
a row group, not by the file. Smaller row groups did not lower it
measurably and cost about 15% in file size, so the defaults stand.

## From Rust

The same machinery is the `hardmoney::export` module (feature `export`,
on by default):

```rust
use std::path::Path;

use hardmoney::Table;
use hardmoney::export::{ExportOptions, Format, export_path};

let input = Path::new("tests/fixtures/F3XA_2011821.fec");
let opts = ExportOptions::new(Format::Parquet)
    .only([Table::SchA, Table::SchB])
    .include_filing_id_from(input)?;
let report = export_path(input, Path::new("/tmp/x"), &opts)?;
for t in &report.tables {
    println!("{}: {} rows, {:?} bytes", t.table, t.rows, t.bytes);
}
println!("{} rows, {} bytes, {} skipped", report.rows(), report.bytes, report.skipped);
# Ok::<(), hardmoney::export::ExportError>(())
```

`export_reader` takes any `FilingReader<impl BufRead>` instead of a path
(a network body, a decompressor) and follows that reader's
`ParseOptions`. Errors are one `ExportError` enum: the parser's
`FecError` (with its line number), I/O errors naming the path, the
`csv`/`serde_json`/`arrow`/`parquet`/`rusqlite` errors, `UnknownTable`
for a bad `--only` token, `FilingIdNotInFilename`, and
`FilingIdOutOfRange`.

The `export` feature pulls in `arrow`, `parquet`, and a bundled SQLite;
the parser-only build (`--no-default-features --features fetch`) has none
of them.
