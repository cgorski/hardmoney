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

## "couldn't find a line parser for form type '...' at line N"

```text
error: couldn't find a line parser for form type 'ZZZ' (spec version '8.5') at line 5
```

A body line's form-type token (its first column) matched none of the
parser's dispatch patterns, and you're parsing strictly (the default), so
the whole filing was refused. The line number is the physical line in
the file. Your options:

- If the token is a real FEC record type the crate doesn't know yet,
  please [open an issue](https://github.com/cgorski/hardmoney/issues)
  with the filing id. The dispatch list in `src/parser/form.rs` covers
  every token in the crate's real-filing fixtures, but the FEC adds record
  types over time.
- If you'd rather keep the lines that *did* parse, use `--lenient` on the
  CLI or `Filing::parse_with(&content, &ParseOptions::LENIENT)` in code.
  See [Strict vs. Lenient Parsing](./strict-vs-lenient.md).

The sibling message `no column-position data to parse table X at spec
version 'Y' at line N` means the table is known but has no layout for
that spec version -- usually a filing newer than the bundled format
tables. Same two options.

## "namespace '...' has N pending migration(s); run `hardmoney schema-init` first"

Every database command checks the namespace's migration state before
touching data, and this is the refusal. Run:

```bash
hardmoney schema-init --schema <the same namespace>
```

and retry.

### `schema-init` itself fails with `column "..." does not exist`

The schema is *migrated*, not re-created: `schema-init` applies the
numbered files in `migrations/` in order and records each one in the
namespace's `_sqlx_migrations` table, and it never rewrites a table it
finds already there. If the namespace already contains a table with one
of hardmoney's names but a different layout -- created by hand, by
another tool, or by a `pg_restore` of something else -- the first
statement that touches the missing column fails, typically an index on a
`*_date` column:

```text
error: ... column "transaction_date" does not exist
```

Nothing is dropped or altered on your behalf. Point `schema-init` at a
fresh namespace (`--schema <new name>`), load into that, and drop the
old one with `schema-drop` when you no longer need it. See
[Namespaces](./namespaces.md).

## "error returned from database: database \"...\" does not exist"

```text
error: error returned from database: database "nonexistent_db_xyz" does not exist at line 1016
  caused by: database "nonexistent_db_xyz" does not exist at line 1016
```

`--database-url` (or `$DATABASE_URL`) points at a database name that
doesn't exist yet on your Postgres server. (The `at line 1016` is
Postgres's own error position and isn't meaningful here; the CLI also
prints each error's cause chain on `caused by:` lines.) Postgres doesn't auto-create
databases -- create it first:

```bash
createdb -h 127.0.0.1 -U postgres fec_dev
```

or via `psql`:

```sql
CREATE DATABASE fec_dev;
```

Then run `schema-init` before loading anything, as shown in
[Loading Bulk Data into Postgres](./bulk-etl.md#step-1-apply-the-schema).

## "role \"anonymous\" does not exist"

Your `DATABASE_URL` has no username in it (`postgres://localhost/fec`).
When the URL omits the user, `sqlx` falls back to your operating-system
username; if that lookup isn't available on your platform it connects as
`anonymous`, and Postgres has no such role. Either way, the fix is to add
the user explicitly: `postgres://youruser@localhost/fec`. That's the form
the CLI's own help text recommends and every example in this book uses.

## "invalid value '2027' for `--cycle <CYCLE>`"

```text
error: invalid value '2027' for '--cycle <CYCLE>': cycle 2027 is not an even year (cycles are the even year of a two-year period)
```

The FEC organizes bulk data by two-year cycle, named for its even year:
2025-2026 is cycle 2026. `--cycle` (and `?cycle=` on the API) accept even
years from 1976 through 2100. There is no `2027/` directory on fec.gov,
so rejecting the value up front saves you from downloading a 404 page and
trying to unzip it.

## "replacing X for cycle N would delete M existing rows; pass --yes to confirm"

`bulk-load`'s default `--mode replace` deletes the cycle's existing rows
before loading the new file, and it refuses to delete more than a million
of them without `--yes`. If you meant to refresh the table, add `--yes`.
If you didn't -- you typed the wrong source name, or the wrong namespace
-- nothing has happened yet; the check runs before the download. See
[Reloading](./reloading.md#the---yes-guard).

## "duplicate key value violates unique constraint" from `bulk-load`

You passed `--mode append` and at least one row already exists. Append
never deletes, so it can't be used to refresh a table. Drop the flag
(replace is the default) to reload the cycle, or use a different
namespace if you want both copies. See [Reloading](./reloading.md#--mode-append).

## `bulk-load --if-changed` says "unchanged" but I know the file changed

`--if-changed` compares against the most recent **full** load of that
source and cycle in the **same namespace**. A `--limit` sample doesn't
count as a baseline, and a load into a different `--schema` doesn't
either. If you need to force it, just drop `--if-changed`.

## "could not derive a filing id from '...'; pass --filing-id"

`bulk-load-filing` derives the filing id from the last run of four or
more digits in the file name (`F24N_2011832.fec` -> 2011832). A file
named `notes.fec` or `F99.fec` has no such run, so the command asks
rather than storing the filing under id 0. Pass `--filing-id <N>` with
the real FEC filing number.

## `bulk-restore-dump` prints "pg_restore reported N non-fatal message(s)"

That's expected on a first restore of `schedule_e`: the FEC's dump
references a trigger function that only exists inside the FEC's own
database, and `pg_restore` reports it and moves on. hardmoney judges
success by whether the target table exists and has rows, and the
command exits 0 when it does. If it exits 1 with `disclosure.X does not
exist after restore`, read the `pg_restore` output it prints -- the usual
culprits are `pg_restore` not being on `PATH`, or a permissions problem
creating the `disclosure` schema.

## The REST API returns 401 for everything except `/health`

The server was started with `--api-key` (or `HARDMONEY_API_KEY` is set in
its environment). Send the key as an `X-Api-Key: <key>` header or as
`?api_key=<key>`. `/health` is deliberately open so uptime checks don't
need the secret. See [Hardening the API](./api-hardening.md#the-api-key).

## The REST API rejects a filter value with a 400 error

This is intentional, not a bug. Numeric filters like `min_amount` are
typed as `Decimal`, date filters like `min_date` as ISO dates
(`YYYY-MM-DD`, not the FEC's `MM/DD/YYYY`), and `cycle` as a validated
even year -- so a bad value is rejected up front with a clear message
rather than silently matching nothing or crashing later. See
[Hardening the API](./api-hardening.md#errors-that-tell-the-client-the-right-amount).

## `/independent-expenditures` returns 503

```json
{"error":"independent_expenditures is not available: run `hardmoney bulk-restore-dump schedule_e` to load the FEC's official Schedule E pg_dump archive, then `hardmoney schema-init`"}
```

That route reads a view over the FEC's own Schedule E `pg_dump`, which
isn't part of `bulk-load-all`; it has to be restored once per database
with `bulk-restore-dump schedule_e`. The view is then created in your
namespace by `schema-init` (or the next `serve` start). Everything else
in the API works without it.

## `/schedule-a` or `/disbursements` returns `[]`

Check `GET /schema` (or `hardmoney schema-status`) -- it lists what's
actually loaded in the namespace the server is serving. Two common
causes: the server is serving a different `--schema` than you loaded
into, or the table only holds a `--limit` sample that doesn't include
what you're filtering for.

## A bulk-data download is very slow or very large

Some bulk sources -- `schedule_a` in particular, at over 2 GB per cycle --
are big. If you just want to explore the data's shape, use `--limit` to
cap how many rows are read; the download stops as soon as the loader has
them, so `--limit 100` across all ten sources takes seconds:

```bash
hardmoney bulk-load-all --schema scratch --cycle 2026 --limit 100
```

For the two dump-based sources that are tens of gigabytes
(`schedule_a_full`, `schedule_b_full` via `bulk-restore-dump`), the tool
requires `--allow-large` precisely so you don't start a huge download by
accident. Make sure you actually have the disk space (and time) before
passing it.

## Why does `bulk-load-filing` only extract Schedule E lines?

It's a deliberate scope: `bulk-load-filing` is the precise,
straight-from-the-filing counterpart to the FEC's aggregated Schedule E
data, and it stores the whole header and summary too. The extraction uses
`line.view::<ScheduleE>()` on `Table::SchE` lines only, so a filing with
no independent expenditures stores zero Schedule E rows -- see
[Views check the table](./typed-views.md#views-check-the-table) for why
the table check matters. If you want every schedule of a filing in
Postgres, the parser output (`filing.lines`) is `serde`-serializable;
storing it is a few lines of `sqlx`.

## My `match` on `Table` / `EntityType` / `FecError` won't compile without a `_` arm

All of hardmoney's public enums -- `Table`, `EntityType`,
`SupportOppose`, `FecError`, `TypedViewError`, `SkipReason`,
`OnUnparseableLine`, `LoadMode` -- are `#[non_exhaustive]`. The FEC adds
record types and codes over time, and the attribute lets a minor release
add a variant without breaking downstream code. The cost is that a
`match` in *your* crate needs a wildcard arm even if you list every
variant that exists today:

```rust
use hardmoney::Table;

fn describe(table: Table) -> &'static str {
    match table {
        Table::SchA => "itemized receipts",
        Table::SchB => "itemized disbursements",
        Table::SchE => "independent expenditures",
        _ => "other", // required: Table is #[non_exhaustive]
    }
}
```

The same attribute is on the public structs (`Filing`, `ParsedLine`,
`ParseOptions`, `LoadOptions`, `LoadReport`, `IngestReport`, the typed
views), which means you construct them through the provided constants,
constructors, and builders (`ParseOptions::LENIENT`,
`LoadOptions::new(cycle).limit(...)`) rather than struct literals, and
you can't destructure them exhaustively -- read the fields you need by
name instead.

## Why `Decimal` instead of `f64`?

Covered in depth in
[Tables and Typed Views](./typed-views.md#exact-money-with-rust_decimaldecimal).
The short version: binary floating point cannot represent most decimal
fractions exactly, so two real FEC amounts like `10170.37 + 237.93` come
out as `10408.300000000001` in `f64` and as `10408.30` in `Decimal` (see
[the Rust tutorial](./tutorial-rust-library.md#why-not-f64) for that
exact run). For any *one* value the error is invisible after rounding;
across millions of rows it is not something you want to have to reason
about. Using `rust_decimal::Decimal` throughout -- parser, database
`NUMERIC` columns, JSON -- makes exact decimal arithmetic a property of
the types themselves, checked by the compiler, rather than something
that happens to hold for the values seen so far. That is the right
foundation for a crate whose whole job is handling other people's money
data correctly.

## Where do I report a bug or ask a question?

Open an issue on the
[GitHub repository](https://github.com/cgorski/hardmoney/issues). If it's
a parsing question about a specific real filing, include the filing ID
(or the file itself, if you're able to share it) -- the maintainers have
found that real-world filings surface edge cases synthetic test data
never would, which is exactly how the
[field-name collision fix](./parsing-explained.md#a-real-bug-this-design-caught-the-field-name-collision-fix)
was found in the first place. [What hardmoney Does With Messy FEC Data](./tutorial-fec-data-quality.md)
catalogues the known quirks and how each one is handled.

## Where does the underlying data come from?

- FEC electronic filing spec documentation:
  <https://www.fec.gov/help-candidates-and-committees/filing-reports/>
- Format-table column-position data:
  [`dwillis/fech-sources`](https://github.com/dwillis/fech-sources),
  itself derived from the [Fech](https://github.com/dwillis/Fech) Ruby
  gem. `F2S.csv` is authored locally.
- Bulk-data column layouts: the FEC's own data dictionaries at
  `https://www.fec.gov/files/bulk-downloads/data_dictionaries/`.
- Parsing-approach inspiration (not copied code):
  [`newsdev/nyt-pyfec`](https://github.com/newsdev/nyt-pyfec).
- Real filing test fixtures used throughout this book: the FEC's own
  `docquery.fec.gov` document store and RSS feed, and
  [`esonderegger/fecfile`](https://github.com/esonderegger/fecfile)'s
  bundled historical test data.

Full attribution, including every correction made to the vendored format
tables, is in the repository's
[`NOTICE`](https://github.com/cgorski/hardmoney/blob/main/NOTICE) file.
