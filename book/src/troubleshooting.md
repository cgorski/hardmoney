# Troubleshooting & FAQ

## "error: No such file or directory (os error 2)" when parsing

```bash
$ hardmoney parse tests/fixtures/does_not_exist.fec
error: No such file or directory (os error 2)
```

This means exactly what it says -- the path given to `parse` doesn't
exist. Double-check the path is relative to your current working
directory (or use an absolute path). If you're following along with this
book's fixture examples, make sure you're running commands from the
repository root (`hardmoney/`), since the paths in this book (e.g.
`tests/fixtures/F24N_2011832.fec`) are relative to that.

## "error returned from database: database \"...\" does not exist"

```text
error: error returned from database: database "nonexistent_db_xyz" does not exist at line 1014
```

`--database-url` (or `$DATABASE_URL`) points at a database name that
doesn't exist yet on your Postgres server. Postgres doesn't auto-create
databases -- create it first:

```bash
createdb -h 127.0.0.1 -U postgres fec_dev
```

or via `psql`:

```sql
CREATE DATABASE fec_dev;
```

Then re-run `schema-init` before loading anything, as shown in
[Loading Bulk Data into Postgres](./bulk-etl.md#step-1-apply-the-schema).

## The REST API rejects a filter value with a 400 error

This is intentional, not a bug -- see
[Filters that take a `Decimal`](./rest-api.md#filters-that-take-a-decimal-and-what-happens-with-bad-input)
in the REST API chapter. Numeric filters like `min_amount` are strongly
typed, so a non-numeric value is rejected up front with a clear message
rather than silently matching nothing or crashing later.

## A bulk-data download is very slow or very large

Some bulk sources -- `schedule_a` in particular -- are multiple gigabytes
per election cycle. If you just want to explore the data's shape, use
`--limit` to cap how many rows are read instead of pulling the whole
file:

```bash
hardmoney bulk-load --database-url "$DATABASE_URL" schedule_a --cycle 2026 --limit 10000
```

For the two dump-based sources that are tens of gigabytes
(`schedule_a_full`, `schedule_b_full` via `bulk-restore-dump`), the tool
requires `--allow-large` precisely so you don't start a huge download by
accident. Make sure you actually have the disk space (and time) before
passing it.

## Why does `bulk-load-filing` only extract Schedule E lines?

See
[Why is this ingestion path scoped only to genuine Schedule E lines?](./bulk-etl.md#why-is-this-ingestion-path-scoped-only-to-genuine-schedule-e-lines)
in the Bulk ETL chapter -- it's a deliberate scope, not a missing
feature, and there's a regression test guarding it.

## Why `Decimal` instead of `f64`, and is this a "bug fix"?

Covered in depth in
[Working with Money, Dates, and Names](./typed-views.md#exact-money-with-rust_decimaldecimal).
To be precise about the framing: extensive testing (including large-scale
sampling of real cent values and summation tests) did not find a
reproducible case where this crate's prior `f64`-based money handling
actually produced a wrong dollar figure in practice. Using
`rust_decimal::Decimal` throughout is an architectural correctness choice
-- it makes exact decimal arithmetic a property of the types themselves,
provable by the type system, rather than something that happens to hold
today because of how the current code paths sum values. That's a stronger
guarantee than "we tested it and didn't find a problem," and it's the
right foundation for a crate whose whole job is handling other people's
money data correctly.

## Where do I report a bug or ask a question?

Open an issue on the
[GitHub repository](https://github.com/cgorski/hardmoney/issues). If it's
a parsing question about a specific real filing, include the filing ID
(or the file itself, if you're able to share it) -- the maintainers have
found that real-world filings surface edge cases synthetic test data
never would, which is exactly how the
[field-name collision fix](./parsing-explained.md#a-real-bug-this-design-caught-the-field-name-collision-fix)
described earlier in this book was found in the first place.

## Where does the underlying data come from?

- FEC electronic filing spec documentation:
  <https://www.fec.gov/help-candidates-and-committees/filing-reports/>
- Format-table column-position data:
  [`dwillis/fech-sources`](https://github.com/dwillis/fech-sources),
  itself derived from the [Fech](https://github.com/dwillis/Fech) Ruby
  gem.
- Parsing-approach inspiration (not copied code):
  [`newsdev/nyt-pyfec`](https://github.com/newsdev/nyt-pyfec).
- Real filing test fixtures used throughout this book: the FEC's own
  `docquery.fec.gov` document store and RSS feed, and
  [`esonderegger/fecfile`](https://github.com/esonderegger/fecfile)'s
  bundled historical test data.

Full attribution, including every correction made to the vendored format
tables, is in the repository's
[`NOTICE`](https://github.com/cgorski/hardmoney/blob/main/NOTICE) file.
