# CLI reference

This is the full `--help` output for `hardmoney` 2.2.0 and every
subcommand, captured directly from the built binary. Run
`hardmoney <subcommand> --help` yourself at any time to see this same
text; it is generated from the same argument definitions the program
runs with. (`bulk-load`, `bulk-load-all`, `validate`, and `export` have
long-form help; the others print the same text for `-h` and `--help`.)

## Top level

```text
$ hardmoney --help
FEC campaign-finance data: parse .fec filings, load bulk data, serve a REST API

Usage: hardmoney <COMMAND>

Commands:
  parse                Parse a single `.fec` filing and print a JSON summary
  write                Parse a `.fec` filing and write it back out in canonical form
  reconcile            Recompute a report's cover-page totals from its schedules and show every line that disagrees
  validate             Check a .fec file against the FEC's acceptance rules
  export               Export a .fec filing's records as CSV, JSON Lines, Parquet, or SQLite
  schema-init          Create or upgrade the Postgres schema in the target namespace
  schema-status        Show migration state and recorded loads for the target namespace
  schema-list          List hardmoney namespaces (schemas) in the database
  schema-drop          Drop a namespace and all data in it
  bulk-load            Load one bulk-data source into Postgres
  bulk-load-all        Load every bulk source for one cycle
  bulk-restore-dump    Restore one of the FEC's own official pg_dump archives
  bulk-dump-info       Show the FEC's dump files: remote size and date, cache, database state, restores
  bulk-dump-index      Create hardmoney's indexes on a restored dump table
  bulk-dump-compare    Compare an ingested filing's raw Schedule E lines with the FEC's processed dump rows
  bulk-load-filing     Ingest a single raw `.fec` filing directly
  bulk-resolve-chains  Recompute every ingested filing's amendment chain (after a batch of `bulk-load-filing --no-resolve`)
  dumps                Import the FEC's Postgres dump files (guided)
  filings              Find filings via the FEC's API and fetch, validate, or ingest them
  efile                Follow the FEC's electronic filing feed
  serve                Run the REST API server
  query                Search loaded data from the terminal (same results as the REST API)
  spec                 Export or diff the machine-readable FEC format specification
  help                 Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version

Database commands read DATABASE_URL (or --database-url) and an optional HARDMONEY_SCHEMA (or --schema) naming an isolated namespace inside that database. Include a username in the URL: postgres://user@host/db
```

## The two flags every database command shares

Every subcommand except `parse`, `write`, `reconcile`, `validate`,
`export`, `spec`, `filings`, and `efile` (which work on files, bundled
data, and the FEC's public endpoints, with no database) takes
these, so they're described once here and shown verbatim in each block
below. `filings` and `efile` accept the same two flags but need them only
with `--ingest`:

| Flag | Environment variable | Default | Meaning |
|---|---|---|---|
| `--database-url <DATABASE_URL>` | `DATABASE_URL` | required | Postgres connection URL. Include a username: `postgres://user@host/db`. |
| `--schema <SCHEMA>` | `HARDMONEY_SCHEMA` | `public` | Namespace (Postgres schema) to operate in. See [Namespaces](./namespaces.md). |

Every database command other than `schema-init`, `schema-status`,
`schema-list`, and `schema-drop` also refuses to run if the namespace has
pending migrations.

The binary loads a `.env` file from the current directory if one exists,
so `DATABASE_URL=...` and `HARDMONEY_SCHEMA=...` can live there. Real
environment variables and explicit flags always win over `.env`. Log
output goes to stderr and is controlled by `RUST_LOG` (default
`info,sqlx=warn`).

## `parse`

```text
$ hardmoney parse --help
Parse a single `.fec` filing and print a JSON summary

Usage: hardmoney parse [OPTIONS] <PATH>

Arguments:
  <PATH>  Path to a `.fec` file

Options:
      --lenient  Skip body lines that cannot be parsed (unknown form type, or no column layout for this spec version) instead of failing. Skipped lines are listed on stderr and counted in the JSON output
      --lines    Include every parsed body line (all fields) in the output. Large
  -h, --help     Print help
```

The JSON output has the keys `form_type`, `base_form_type`, `version`,
`is_amendment`, `amends_filing`, `line_count`, `lines_by_table`,
`skipped_count`, `skipped`, `header`, `summary`, and (with `--lines`)
`lines`. See [Quick start](./quick-start.md) for real examples and
[Strict vs. lenient parsing](./strict-vs-lenient.md) for `--lenient`.
Exits 1 on a parse error.

## `write`

```text
$ hardmoney write --help
Parse a `.fec` filing and write it back out in canonical form

Usage: hardmoney write [OPTIONS] <PATH>

Arguments:
  <PATH>  Path to a `.fec` file

Options:
  -o, --out <OUT>  Output path. Defaults to stdout
      --lenient    Skip body lines that cannot be parsed instead of failing (they are omitted from the output and listed on stderr)
      --check      Instead of writing, re-parse the written bytes and report whether every record round-trips; exit 1 if not
  -h, --help       Print help
```

Parses the file and writes it back through `Filing::to_fec`: `CRLF`
line endings, ASCII-28 delimiting for spec 6.0+ (comma with CSV quoting
only where needed for 3.x-5.x), every record padded to its layout's
full column count, fields trimmed with wrapping quotes removed, a Form
99's free text as a `[BEGINTEXT]` block, and Windows-1252 encoding when
every character is representable (else UTF-8). Output goes to stdout or
`-o FILE`. With `--lenient`, a body line that cannot be parsed is
omitted from the output and reported as `warning: omitted line N: ...`
on stderr.

`--check` writes nothing: it re-parses the canonical bytes and compares
header, cover line, and every body line with the original parse:

```text
$ hardmoney write --check tests/fixtures/F3XA_2011827.fec
OK: tests/fixtures/F3XA_2011827.fec round-trips (6 body lines, 1867 bytes in, 1876 bytes out)
```

A difference prints `MISMATCH: ...` lines on stderr (`header differs`,
`cover line differs`, `line count differs: A vs B`, `line N differs`)
and exits 1. Exits 1 on a parse error. See
[Writing `.fec` files](./writing-fec.md).

## `export`

```text
$ hardmoney export -h
Export a .fec filing's records as CSV, JSON Lines, Parquet, or SQLite

Usage: hardmoney export [OPTIONS] <PATH>

Arguments:
  <PATH>  Path to a `.fec` file

Options:
  -f, --format <FORMAT>    Output format. csv/jsonl/parquet write one `<Table>.<ext>` file per table into a directory; sqlite writes one database file [default: csv] [possible values: csv, jsonl, parquet, sqlite]
  -o, --out <OUT>          Output directory (csv/jsonl/parquet) or database file (sqlite). Created if missing; existing per-table files are replaced, existing SQLite tables are appended to. [default: ./<file-stem>.<format>]
      --only <ONLY>        Export only these tables, comma-separated: table names (SchA, F3X, TEXT; case-insensitive) or upper-case form-type tokens (SA, SB21B, SE). The cover line is included only if its table is listed
      --include-filing-id  Prepend a `filing_id` column, taken from the file name (the last run of 4+ digits in its stem, e.g. 2011821 for F3XA_2011821.fec). Fails if the name has none
      --lenient            Skip body lines that cannot be parsed (unknown form type, or no column layout for this spec version) instead of failing; the count is reported on stderr
  -h, --help               Print help (see more with '--help')
```

(`--help` additionally describes each `--format` value.) Streams the
filing through `FilingReader` and writes one table per record type
(`F3X` for the cover line, `SchA`, `SchB`, `TEXT`, ...), named as
`hardmoney spec tables` lists them. Every table has the columns
`filing_id` (with `--include-filing-id`), `line_no`, then the layout's
fields in FEC column order starting with `form_type`. `csv`, `jsonl`,
and `parquet` write `<out>/<Table>.<ext>`; `sqlite` writes one database
with one table per `Table` plus a `filings` metadata row (`filing_id`,
`form_type`, `version`, `committee_id`, `path`, `exported_at`). Amounts
are exact everywhere: text as filed in csv/jsonl/sqlite,
`Decimal128(38, 2)` in Parquet, where `NUM-8` dates are also `Date32`
and everything else is `Utf8`. Ends with a summary table:

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
```

(For `sqlite` the `bytes` column is omitted and the total is the
database file's size.) With `--lenient`, `warning: N unparseable
line(s) skipped (--lenient)` goes to stderr. `--only` takes table names
(case-insensitive) or upper-case form-type tokens; anything else is a
usage error (exit 2) naming the token. Exits 1 on a parse error, an
unwritable output, or `--include-filing-id` on a file name with no
4+-digit run. See [Exporting a filing](./exporting.md).

## `reconcile`

```text
$ hardmoney reconcile --help
Recompute a report's cover-page totals from its schedules and show every line that disagrees

Usage: hardmoney reconcile [OPTIONS] <PATH>

Arguments:
  <PATH>  Path to a `.fec` file (Form 3X, 3, or 3P)

Options:
      --json                   Emit the full check list as JSON
      --all                    Show every line, not just the ones that disagree
      --tolerance <TOLERANCE>  Treat violations up to this amount as agreeing (e.g. 0.01 for a filer who rounds each line). A violation is the absolute delta for an `=` line, or the shortfall below the itemized sum for a `>=` line [default: 0]
      --column <COLUMN>        Only check this column (A = this period, B = year/cycle to date) [possible values: a, b]
      --lenient                Skip body lines that cannot be parsed instead of failing
  -h, --help                   Print help
```

Recomputes every cover-page line of a Form 3X, 3, or 3P from the
schedules (memo entries excluded) and from the other cover lines'
reported values, with exact `Decimal` arithmetic. Prints one line per
disagreeing check (or every check with `--all`) in the form
`STATUS col C line L reported R expected E delta D  RULE`, then a
verdict:

```text
$ hardmoney reconcile --lenient tmp/agent-misc/filings/2011912.fec
DIFF col A line 11(c)      reported         2045.00 expected         1845.00 delta       200.00  = sum of SchA.contribution_amount on SA11C
F3X C00001313: 1 of 69 line(s) disagree (tolerance 0)
error: 1 line(s) disagree
```

A rule beginning `=` must hold exactly; one beginning `>=` is a floor
(the cover total may exceed the itemized sum, because sub-$200 items
need not be itemized). `--tolerance N` treats a violation of at most
`N` as agreement; `--column a` or `b` restricts the checks. `--lenient`
skips unparseable body lines with a stderr warning that they are
excluded from the sums. `--json` emits `{form, file, tolerance, checks,
disagreeing, lines: [{line, field, column, rule, reported, expected,
delta, relation, lines_summed, reported_unparseable}]}` with amounts as
strings. Exit 0 when every (selected) line agrees within the
tolerance, 1 when any disagrees, on a parse error, or on a form with no
rules (`no reconciliation rules for form F24; supported: F3X, F3, F3P`).
See [Reconciling a filing](./reconciling.md).

## `validate`

```text
$ hardmoney validate --help
Check a .fec file against the FEC's acceptance rules

Usage: hardmoney validate [OPTIONS] <PATH>

Arguments:
  <PATH>
          Path to a `.fec` file

Options:
      --json
          Print the findings as a JSON document instead of one line each

      --strict-warnings
          Exit 1 on warnings too, not only on errors

      --oracle <ORACLE>
          Also submit the file to an external validator and diff its findings against ours. Sends the file over the network

          Possible values:
          - webcheck: The FEC's WebCheck (<https://efoservices.fec.gov/webcheck/>). The public upload channel needs no credentials; with `--webcheck-api-key` the vendor SOAP service is used instead

      --strict-oracle
          With `--oracle`, exit 1 if the oracle and hardmoney disagree on any finding

      --webcheck-api-key <WEBCHECK_API_KEY>
          FEC vendor API key for WebCheck's SOAP service. Without it the credential-free upload channel (what the WebCheck web page uses) is taken. Ignored without `--oracle webcheck`

          [env: WEBCHECK_API_KEY]

      --webcheck-email <WEBCHECK_EMAIL>
          Contact e-mail to pass WebCheck's SOAP service (it e-mails results for files over 20 MB). Ignored without `--webcheck-api-key`

          [env: WEBCHECK_EMAIL]

  -h, --help
          Print help (see a summary with '-h')
```

Parses the file leniently (a line the FEC would ignore is a finding, not
a reason to stop) and checks it against the rules the FEC's own
validator applies: required fields, field types and lengths from the
spec workbook, real dates, amount formats, committee/candidate ID
formats, legal characters, unique transaction IDs, back-references that
resolve, and schedules allowed on the parent form. One finding per
line, `SEVERITY line N FORM_TYPE field: message`, then a verdict and a
per-rule tally:

```text
$ hardmoney validate tests/fixtures/invalid/bad_dates_and_amounts.fec
ERROR line 2 F3XA date_signed: 20261301 is not a Real Date
ERROR line 4 SB21B expenditure_date: 20260231 is not a Real Date
ERROR line 5 SB21B expenditure_date: Bad Date - 2026-06-22 not YYYYMMDD format
ERROR line 6 SB21B expenditure_amount: Invalid Amount format: '$5,500.00' in EXPENDITURE AMOUNT {F3L Bundled} (digits with an optional sign and up to two decimals; no $ or commas)
ERROR line 7 SB21B expenditure_amount: Invalid Amount format: '1500.005' in EXPENDITURE AMOUNT {F3L Bundled} (digits with an optional sign and up to two decimals; no $ or commas)
NOT ACCEPTABLE: F3XA, 6 body line(s), 5 error(s), 0 warning(s)
       2 error   not_a_real_date
       2 error   invalid_amount
       1 error   bad_date_format
error: validation failed with 5 error(s) and 0 warning(s)
```

A clean filing prints one line, `ACCEPTABLE: F3XN, 16 body line(s), 0
error(s), 0 warning(s)`, and exits 0. Exit status is 1 if there is any
`ERROR` finding (the FEC would reject the filing), or with
`--strict-warnings` any finding at all; a file that cannot be parsed at
all (bad header, no cover line) is also exit 1, reported on stderr.
`--json` emits `{file, form_type, version, line_count, acceptable,
errors, warnings, findings_by_rule, findings: [{severity, rule,
line_no, form_type, field, message}]}`; with `--oracle` it gains
`oracle` (WebCheck's own report) and `oracle_diff` (`matched`,
`only_ours`, `only_theirs`). `--oracle webcheck` sends the file to the
FEC's validator and prints its findings and the diff after ours; it never
changes the exit status unless `--strict-oracle`. The full rule table,
the deliberate deviations from the FEC's validator, the oracle, and the
library API are in [Validating a filing](./validating.md); the
`FieldSpec` data the per-field rules read is described in
[The schema](./library-schema.md#fieldspec-what-the-fec-says-about-a-field).

## `schema-init`

```text
$ hardmoney schema-init --help
Create or upgrade the Postgres schema in the target namespace

Usage: hardmoney schema-init [OPTIONS] --database-url <DATABASE_URL>

Options:
      --database-url <DATABASE_URL>  Postgres connection URL. Include a username: postgres://user@host/db [env: DATABASE_URL=]
      --schema <SCHEMA>              Namespace (Postgres schema) to operate in. Each namespace is a fully isolated set of hardmoney tables: use one per cycle, per snapshot, per investigation, or per CI run. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
  -h, --help                         Print help
```

Applies any pending migrations from `migrations/`, then (re)creates the
`independent_expenditures` view if the FEC's Schedule E dump is present.
Idempotent. See [Bulk ETL](./bulk-etl.md#step-1-apply-the-schema).

## `schema-status`

```text
$ hardmoney schema-status --help
Show migration state and recorded loads for the target namespace

Usage: hardmoney schema-status [OPTIONS] --database-url <DATABASE_URL>

Options:
      --database-url <DATABASE_URL>  Postgres connection URL. Include a username: postgres://user@host/db [env: DATABASE_URL=]
      --schema <SCHEMA>              Namespace (Postgres schema) to operate in. Each namespace is a fully isolated set of hardmoney tables: use one per cycle, per snapshot, per investigation, or per CI run. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
  -h, --help                         Print help
```

Prints the applied and pending migration versions, then the most recent
load per source and cycle from the `loads` table. Example output:

```text
namespace:  public
applied:    [1, 2]
pending:    []
loads (latest per source/cycle):
  candidates                               cycle 2026          8557 rows  replace 2026-09-15 01:56:55Z
  committee_to_candidate_transactions      cycle 2026         20000 rows  replace 2026-09-15 01:57:07Z (sample, limit 20000)
  disbursements                            cycle 2026            50 rows  replace 2026-09-15 01:57:09Z (sample, limit 50)
```

## `schema-list`

```text
$ hardmoney schema-list --help
List hardmoney namespaces (schemas) in the database

Usage: hardmoney schema-list [OPTIONS] --database-url <DATABASE_URL>

Options:
      --database-url <DATABASE_URL>  Postgres connection URL. Include a username: postgres://user@host/db [env: DATABASE_URL=]
      --schema <SCHEMA>              Namespace (Postgres schema) to operate in. Each namespace is a fully isolated set of hardmoney tables: use one per cycle, per snapshot, per investigation, or per CI run. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
  -h, --help                         Print help
```

Lists every schema in the database that has a `_sqlx_migrations` table,
one per line. `--schema` is accepted but has no effect on the result.

## `schema-drop`

```text
$ hardmoney schema-drop --help
Drop a namespace and all data in it

Usage: hardmoney schema-drop [OPTIONS] --database-url <DATABASE_URL> <NAME>

Arguments:
  <NAME>  The namespace to drop (must not be `public`)

Options:
      --database-url <DATABASE_URL>  Postgres connection URL. Include a username: postgres://user@host/db [env: DATABASE_URL=]
      --schema <SCHEMA>              Namespace (Postgres schema) to operate in. Each namespace is a fully isolated set of hardmoney tables: use one per cycle, per snapshot, per investigation, or per CI run. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
      --yes                          Confirm the drop
  -h, --help                         Print help
```

Without `--yes`, prints what would be deleted and exits 1. Refuses
`public` even with `--yes`. Never touches the shared `disclosure` schema.

## `bulk-load`

```text
$ hardmoney bulk-load --help
Load one bulk-data source into Postgres

Usage: hardmoney bulk-load [OPTIONS] --database-url <DATABASE_URL> --cycle <CYCLE> <SOURCE>

Arguments:
  <SOURCE>
          One of: candidates, committees, candidate_committee_links, schedule_a, committee_to_committee_transactions, committee_to_candidate_transactions, disbursements, candidate_summary, house_senate_summary, pac_party_summary

Options:
      --database-url <DATABASE_URL>
          Postgres connection URL. Include a username: postgres://user@host/db

          [env: DATABASE_URL=]

      --schema <SCHEMA>
          Namespace (Postgres schema) to operate in. Each namespace is a fully isolated set of hardmoney tables: use one per cycle, per snapshot, per investigation, or per CI run. Default: public

          [env: HARDMONEY_SCHEMA=]
          [default: public]

      --cycle <CYCLE>
          4-digit even-year election cycle, e.g. 2026

      --limit <LIMIT>
          Cap the number of rows read (samples a multi-gigabyte source without downloading all of it). Recorded as a sample load

      --mode <MODE>
          Possible values:
          - replace: Delete this cycle's existing rows, then load, in one transaction (reproduces the FEC's current file exactly, including deletions)
          - append:  Load on top of existing rows; fails on any duplicate key

          [default: replace]

      --yes
          Confirm a `replace` that would delete more than 1,000,000 rows

      --if-changed
          Skip the load if the FEC's file is unchanged (ETag/Last-Modified) since the last full load of this source and cycle

      --file <FILE>
          Load from a local zip instead of downloading from fec.gov

  -h, --help
          Print help (see a summary with '-h')
```

`--cycle` must be an even year from 1976 through 2100; anything else is
rejected before connecting. See [Bulk ETL](./bulk-etl.md#step-2-load-one-bulk-source)
and [Reloading](./reloading.md).

## `bulk-load-all`

```text
$ hardmoney bulk-load-all --help
Load every bulk source for one cycle

Usage: hardmoney bulk-load-all [OPTIONS] --database-url <DATABASE_URL> --cycle <CYCLE>

Options:
      --database-url <DATABASE_URL>
          Postgres connection URL. Include a username: postgres://user@host/db

          [env: DATABASE_URL=]

      --schema <SCHEMA>
          Namespace (Postgres schema) to operate in. Each namespace is a fully isolated set of hardmoney tables: use one per cycle, per snapshot, per investigation, or per CI run. Default: public

          [env: HARDMONEY_SCHEMA=]
          [default: public]

      --cycle <CYCLE>
          4-digit even-year election cycle, e.g. 2026

      --limit <LIMIT>
          Cap the number of rows read (samples a multi-gigabyte source without downloading all of it). Recorded as a sample load

      --mode <MODE>
          Possible values:
          - replace: Delete this cycle's existing rows, then load, in one transaction (reproduces the FEC's current file exactly, including deletions)
          - append:  Load on top of existing rows; fails on any duplicate key

          [default: replace]

      --yes
          Confirm a `replace` that would delete more than 1,000,000 rows

      --if-changed
          Skip the load if the FEC's file is unchanged (ETag/Last-Modified) since the last full load of this source and cycle

      --fail-fast
          Stop at the first source that fails (default: continue and exit non-zero at the end if any failed)

  -h, --help
          Print help (see a summary with '-h')
```

Runs the ten sources in the order listed under `bulk-load`'s `<SOURCE>`,
each with the same flags. See [Bulk ETL](./bulk-etl.md#step-3-or-load-everything-for-a-cycle-in-one-shot).

## `bulk-restore-dump`

```text
$ hardmoney bulk-restore-dump --help
Restore one of the FEC's own official pg_dump archives

Usage: hardmoney bulk-restore-dump [OPTIONS] --database-url <DATABASE_URL> <NAME>

Arguments:
  <NAME>  One of: schedule_e, committee_history, schedule_a_full, schedule_b_full

Options:
      --database-url <DATABASE_URL>  Postgres connection URL. Include a username: postgres://user@host/db [env: DATABASE_URL=]
      --schema <SCHEMA>              Namespace (Postgres schema) to operate in. Each namespace is a fully isolated set of hardmoney tables: use one per cycle, per snapshot, per investigation, or per CI run. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
      --cycles <CYCLES>              Restore only these two-year periods of a cycle-split dump (schedule_a_full, schedule_b_full), e.g. `--cycles 2024,2026`: the parent table plus the named child tables, data only. Repeat with other cycles later to add them
      --no-indexes                   Skip the archive's indexes, primary keys, and triggers (the FEC's figures for Schedule A: about 5 hours instead of 35). Add hardmoney's indexes afterwards with `bulk-dump-index`
      --allow-large                  Required to restore all of schedule_a_full / schedule_b_full (tens of gigabytes); not needed with --cycles
      --dump-file <DUMP_FILE>        Restore from an already-downloaded .dump file instead of the cache
      --jobs <JOBS>                  pg_restore parallel workers (helps when restoring several cycles) [default: 1]
      --cache-dir <CACHE_DIR>        Where to keep downloaded dumps (re-used across runs; an interrupted download resumes from its .partial file) [env: HARDMONEY_CACHE_DIR=] [default: /Users/chris.gorski/.cache/hardmoney/dumps]
  -h, --help                         Print help
```

The `--cache-dir` default shown is `$XDG_CACHE_HOME/hardmoney/dumps` if
`XDG_CACHE_HOME` is set, else `~/.cache/hardmoney/dumps`, else the
system temp directory; it will read differently on your machine. Needs
`pg_restore` on `PATH`. The dump always restores into the shared
`disclosure` schema; `--schema` controls where the
`independent_expenditures` and `dump_*` views are refreshed. A download
that stops early leaves `<name>.dump.partial` and `<name>.dump.meta`
(the server's ETag) in the cache and resumes on the next run with
`If-Range`; a changed ETag, whether the server answers it with a 200 or
with a 206, restarts the download from zero. The guided
[`dumps`](#dumps) commands wrap this one; see
[The FEC's Postgres dump files](./pg-dumps.md).

## `bulk-dump-info`

```text
$ hardmoney bulk-dump-info --help
Show the FEC's dump files: remote size and date, cache, database state, restores

Usage: hardmoney bulk-dump-info [OPTIONS] --database-url <DATABASE_URL>

Options:
      --database-url <DATABASE_URL>  Postgres connection URL. Include a username: postgres://user@host/db [env: DATABASE_URL=]
      --schema <SCHEMA>              Namespace (Postgres schema) to operate in. Each namespace is a fully isolated set of hardmoney tables: use one per cycle, per snapshot, per investigation, or per CI run. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
      --offline                      Skip the HEAD requests to fec.gov (sizes and dates come from the cache's sidecar files only)
      --cache-dir <CACHE_DIR>        Where downloaded dumps are kept [env: HARDMONEY_CACHE_DIR=] [default: /Users/chris.gorski/.cache/hardmoney/dumps]
  -h, --help                         Print help
```

One row per dump (remote size and `Last-Modified`, what is cached, what
is in `disclosure`), then the restores recorded in this namespace's
`loads` table. `dumps status` is the plain-language version.

## `bulk-dump-index`

```text
$ hardmoney bulk-dump-index --help
Create hardmoney's indexes on a restored dump table

Usage: hardmoney bulk-dump-index [OPTIONS] --database-url <DATABASE_URL> <NAME>

Arguments:
  <NAME>  One of: schedule_e, committee_history, schedule_a_full, schedule_b_full

Options:
      --database-url <DATABASE_URL>  Postgres connection URL. Include a username: postgres://user@host/db [env: DATABASE_URL=]
      --schema <SCHEMA>              Namespace (Postgres schema) to operate in. Each namespace is a fully isolated set of hardmoney tables: use one per cycle, per snapshot, per investigation, or per CI run. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
      --cycles <CYCLES>              Index only these cycles' child tables (default: every restored one)
  -h, --help                         Print help
```

Idempotent: an index that exists is reported, not rebuilt; a unique
index the archive's own primary key already provides, and a trigram
index on a server without `pg_trgm`, are skipped with the reason. The
index list per dump is `hardmoney::bulk::dump::DumpSource::indexes`.

## `bulk-dump-compare`

```text
$ hardmoney bulk-dump-compare --help
Compare an ingested filing's raw Schedule E lines with the FEC's processed dump rows

Usage: hardmoney bulk-dump-compare [OPTIONS] --database-url <DATABASE_URL> <FILING_ID>

Arguments:
  <FILING_ID>  A filing already ingested with `bulk-load-filing`

Options:
      --database-url <DATABASE_URL>  Postgres connection URL. Include a username: postgres://user@host/db [env: DATABASE_URL=]
      --schema <SCHEMA>              Namespace (Postgres schema) to operate in. Each namespace is a fully isolated set of hardmoney tables: use one per cycle, per snapshot, per investigation, or per CI run. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
  -h, --help                         Print help
```

Counts, amount totals, transaction ids on one side only, and amount
disagreements between `schedule_e_lines` (the filing's own bytes) and
`disclosure.fec_fitem_sched_e` (the FEC's weekly snapshot) for one
`file_num`. Differences are expected, not errors: exits 1 only if the
filing is not ingested or the Schedule E dump is not restored.

## `bulk-load-filing`

```text
$ hardmoney bulk-load-filing --help
Ingest a single raw `.fec` filing directly

Usage: hardmoney bulk-load-filing [OPTIONS] --database-url <DATABASE_URL> <FILING>

Arguments:
  <FILING>  A local `.fec` file path, or a numeric filing id to download from the FEC's document store

Options:
      --database-url <DATABASE_URL>  Postgres connection URL. Include a username: postgres://user@host/db [env: DATABASE_URL=]
      --schema <SCHEMA>              Namespace (Postgres schema) to operate in. Each namespace is a fully isolated set of hardmoney tables: use one per cycle, per snapshot, per investigation, or per CI run. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
      --strict                       Fail on any unparseable body line instead of skipping and recording the count (the default is lenient, because one unknown line type should not fail an ingestion job)
      --filing-id <FILING_ID>        Override the filing id derived from the filename
      --no-resolve                   Skip recomputing the filing's amendment chain (`most_recent`, `amendment_chain`, ...) after the insert. For scripted batch loads: ingest every file with this flag, then resolve the whole table once
  -h, --help                         Print help
```

For a local path, the filing id is the last run of four or more digits
in the file name (`F24N_2011832.fec` -> 2011832); if there isn't one,
the command exits 1 asking for `--filing-id`. For a numeric argument,
the filing is downloaded from `docquery.fec.gov` and that number is the
id unless `--filing-id` overrides it. See
[Bulk ETL](./bulk-etl.md#ingesting-a-single-filing-directly-for-precise-schedule-e-data).

## `bulk-resolve-chains`

```text
$ hardmoney bulk-resolve-chains --help
Recompute every ingested filing's amendment chain (after a batch of `bulk-load-filing --no-resolve`)

Usage: hardmoney bulk-resolve-chains [OPTIONS] --database-url <DATABASE_URL>

Options:
      --database-url <DATABASE_URL>  Postgres connection URL. Include a username: postgres://user@host/db [env: DATABASE_URL=]
      --schema <SCHEMA>              Namespace (Postgres schema) to operate in. Each namespace is a fully isolated set of hardmoney tables: use one per cycle, per snapshot, per investigation, or per CI run. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
  -h, --help                         Print help
```

One set-based `UPDATE` over the whole `filings` table
(`hardmoney::db::resolve_all_amendment_chains`). Run it once after a
batch of `bulk-load-filing --no-resolve`, or after `schema-init` applied
migration `0003` to a namespace that already held filings (their chain
columns are NULL until then). Prints the rows recomputed and how many
amendments still name an original that is not ingested. See
[Amendments](./amendments.md#batch-loads---no-resolve-and-bulk-resolve-chains).

## `dumps`

The guided way to import the FEC's Postgres dump files; see
[The FEC's Postgres dump files](./pg-dumps.md) and the
[tutorial](./tutorial-dumps.md). The four flags in the group's own
`--help` (`--database-url`, `--schema`, `--cache-dir`, `--json`) are
accepted by every subcommand, before or after it. `--database-url` is
optional here: without it, `dumps` and `dumps check` explain how to
create a database instead of failing.

```text
$ hardmoney dumps --help
Import the FEC's Postgres dump files (guided)

Usage: hardmoney dumps [OPTIONS] [COMMAND]

Commands:
  check   Check that this computer and the database are ready: pg_restore, the connection, permissions, disk space, and the namespace
  import  Download one of the FEC's files and load it into the database, after showing a plan with time and disk estimates
  status  What is imported, what is downloaded, and whether fec.gov has newer files than you have
  update  Re-import whatever is already imported when fec.gov has a newer file (the FEC refreshes them every weekend)
  remove  Drop an imported table and delete its downloaded file
  help    Print this message or the help of the given subcommand(s)

Options:
      --database-url <URL>  Postgres connection URL, e.g. postgres://you@localhost/fec. Read from DATABASE_URL (or a .env file in the current folder) when not given [env: DATABASE_URL=]
      --schema <SCHEMA>     Namespace (Postgres schema) for hardmoney's own tables and the friendly views. The FEC's tables always land in the `disclosure` schema, which every namespace shares. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
      --cache-dir <DIR>     Folder where downloaded dump files are kept between runs (an interrupted download resumes from there) [env: HARDMONEY_CACHE_DIR=] [default: /Users/chris.gorski/.cache/hardmoney/dumps]
      --json                Print one JSON object instead of text (for scripts). Implies no questions are asked
  -h, --help                Print help
```

With no subcommand, `hardmoney dumps` prints the overview: the four
files with today's sizes from fec.gov (approximate, marked `~`, when
offline), which are downloaded, which are in the database with row
counts and import dates, and the commands to run next.

### `dumps check`

```text
$ hardmoney dumps check --help
Check that this computer and the database are ready: pg_restore, the connection, permissions, disk space, and the namespace

Usage: hardmoney dumps check [OPTIONS]

Options:
      --database-url <URL>  Postgres connection URL, e.g. postgres://you@localhost/fec. Read from DATABASE_URL (or a .env file in the current folder) when not given [env: DATABASE_URL=]
      --for <WHAT>          Size the disk-space checks for this import (same names as `dumps import`). Default: all-small
      --cycles <YEARS>      For receipts and disbursements: the cycles the sizing assumes. Default: the current cycle
      --schema <SCHEMA>     Namespace (Postgres schema) for hardmoney's own tables and the friendly views. The FEC's tables always land in the `disclosure` schema, which every namespace shares. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
      --cache-dir <DIR>     Folder where downloaded dump files are kept between runs (an interrupted download resumes from there) [env: HARDMONEY_CACHE_DIR=] [default: /Users/chris.gorski/.cache/hardmoney/dumps]
      --offline             Do not ask fec.gov for current file sizes; use the approximate ones
      --json                Print one JSON object instead of text (for scripts). Implies no questions are asked
  -y, --yes                 If the namespace is not set up yet, run `schema-init` without asking
  -h, --help                Print help
```

Prints one line per check (`✓` pass, `!` warning, `✗` failure) with a
`fix:` line under anything that is not a pass, then offers to run
`schema-init` if the namespace needs it. Exits 1 if any check failed.
The checks, in order: `pg_restore`, `database`, `connection`,
`Postgres version`, `disclosure schema`, `extensions`, `disk space for
downloads`, `disk space for the database`, `namespace`. With `--json`,
the object has `ok`, `worst`, `checks` (each with `name`, `status`,
`detail`, `fix`), and `needs`.

### `dumps import`

```text
$ hardmoney dumps import --help
Download one of the FEC's files and load it into the database, after showing a plan with time and disk estimates

Usage: hardmoney dumps import [OPTIONS] <WHAT>

Arguments:
  <WHAT>  What to import: committees, independent-expenditures, receipts, disbursements, or all-small (the first two, a few minutes). The technical names (committee_history, schedule_e, schedule_a_full, schedule_b_full) work too

Options:
      --cycles <YEARS>      For receipts and disbursements: which two-year election cycles to load, e.g. 2024,2026. Default: the current cycle. Run again with other cycles to add them
      --database-url <URL>  Postgres connection URL, e.g. postgres://you@localhost/fec. Read from DATABASE_URL (or a .env file in the current folder) when not given [env: DATABASE_URL=]
      --schema <SCHEMA>     Namespace (Postgres schema) for hardmoney's own tables and the friendly views. The FEC's tables always land in the `disclosure` schema, which every namespace shares. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
  -y, --yes                 Do not ask for confirmation
      --cache-dir <DIR>     Folder where downloaded dump files are kept between runs (an interrupted download resumes from there) [env: HARDMONEY_CACHE_DIR=] [default: /Users/chris.gorski/.cache/hardmoney/dumps]
      --explain             Print the plan and the exact pg_restore command(s) that would run, then exit without changing anything
      --dump-file <PATH>    Load from a dump file you already have instead of downloading
      --json                Print one JSON object instead of text (for scripts). Implies no questions are asked
      --jobs <N>            pg_restore parallel workers (helps when loading several cycles) [default: 1]
  -h, --help                Print help
```

Runs the checks (stops if any fail, unless `--explain`), prints the
plan (what is downloaded, how many rows into which table in which
database, disk and time estimates), and asks `Continue? [y/N]` unless
`--yes` is given or stdin is not a terminal. If the namespace has not
had `schema-init`, it asks and runs it. Then, per dump: download with a
progress bar (resuming a partial file), `pg_restore` with a progress
line every 30 seconds, and, for a cycle-selective restore, hardmoney's
indexes. Finishes with rows and time per dump, commands to try (a
`hardmoney query` where one applies, `psql` one-liners against the
views), and where the data and the downloaded file live. The small
dumps are restored whole; `receipts` and `disbursements` default to the
current cycle, data only, followed by `bulk-dump-index`. `--dump-file`
applies to one dump, so not to `all-small`. With `--json`, the object
has `ok`, `elapsed_secs`, `imports` (each with `name`, `dump`, `table`,
`tables`, `cycles`, `rows`, `data_only`, `indexes_created`,
`dump_path`, `recorded`, `elapsed_secs`), and `checks`; with
`--explain`, `plan`, `pg_restore_commands`, and `statements` instead of
`imports`.

### `dumps status`

```text
$ hardmoney dumps status --help
What is imported, what is downloaded, and whether fec.gov has newer files than you have

Usage: hardmoney dumps status [OPTIONS]

Options:
      --database-url <URL>  Postgres connection URL, e.g. postgres://you@localhost/fec. Read from DATABASE_URL (or a .env file in the current folder) when not given [env: DATABASE_URL=]
      --offline             Do not contact fec.gov (skip the "newer file available" check)
      --schema <SCHEMA>     Namespace (Postgres schema) for hardmoney's own tables and the friendly views. The FEC's tables always land in the `disclosure` schema, which every namespace shares. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
      --cache-dir <DIR>     Folder where downloaded dump files are kept between runs (an interrupted download resumes from there) [env: HARDMONEY_CACHE_DIR=] [default: /Users/chris.gorski/.cache/hardmoney/dumps]
      --json                Print one JSON object instead of text (for scripts). Implies no questions are asked
  -h, --help                Print help
```

Per dump: the table and view, whether it is in the database (estimated
rows, index count, cycles present, when it was imported and from which
fec.gov file, from the namespace's `loads` table), whether it is
downloaded, and what fec.gov has now, with "NEWER than what you
imported" when the ETag or `Last-Modified` differs. With `--json`, the
object has `dumps` (each with `name`, `dump`, `table`, `view`, `fec`,
`downloaded`, `database`, `last_import`, `fec_has_newer`), `database`,
`cache_dir`, and `offline`.

### `dumps update`

```text
$ hardmoney dumps update --help
Re-import whatever is already imported when fec.gov has a newer file (the FEC refreshes them every weekend)

Usage: hardmoney dumps update [OPTIONS]

Options:
      --database-url <URL>  Postgres connection URL, e.g. postgres://you@localhost/fec. Read from DATABASE_URL (or a .env file in the current folder) when not given [env: DATABASE_URL=]
  -y, --yes                 Do not ask for confirmation (for cron)
      --jobs <N>            pg_restore parallel workers [default: 1]
      --schema <SCHEMA>     Namespace (Postgres schema) for hardmoney's own tables and the friendly views. The FEC's tables always land in the `disclosure` schema, which every namespace shares. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
      --cache-dir <DIR>     Folder where downloaded dump files are kept between runs (an interrupted download resumes from there) [env: HARDMONEY_CACHE_DIR=] [default: /Users/chris.gorski/.cache/hardmoney/dumps]
      --json                Print one JSON object instead of text (for scripts). Implies no questions are asked
  -h, --help                Print help
```

For each imported dump, compares the fec.gov file's ETag (then
`Last-Modified`) with what the last import recorded (or, failing that,
the downloaded file's sidecar). Dumps with a newer file are re-imported
the way they were imported (the same cycles for a split table), after
the old download is deleted so the new one is fetched; dumps imported
from a local file with nothing recorded are reported and skipped.
Always prints a `crontab` line (Monday 06:00) for unattended use.

### `dumps remove`

```text
$ hardmoney dumps remove --help
Drop an imported table and delete its downloaded file

Usage: hardmoney dumps remove [OPTIONS] <WHAT>

Arguments:
  <WHAT>  What to remove (same names as `dumps import`)

Options:
      --database-url <URL>  Postgres connection URL, e.g. postgres://you@localhost/fec. Read from DATABASE_URL (or a .env file in the current folder) when not given [env: DATABASE_URL=]
  -y, --yes                 Do not ask for confirmation
      --keep-file           Drop the table but keep the downloaded file
      --schema <SCHEMA>     Namespace (Postgres schema) for hardmoney's own tables and the friendly views. The FEC's tables always land in the `disclosure` schema, which every namespace shares. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
      --cache-dir <DIR>     Folder where downloaded dump files are kept between runs (an interrupted download resumes from there) [env: HARDMONEY_CACHE_DIR=] [default: /Users/chris.gorski/.cache/hardmoney/dumps]
      --json                Print one JSON object instead of text (for scripts). Implies no questions are asked
  -h, --help                Print help
```

Says what it will drop (table, estimated rows, partitions) and delete
(the downloaded file and its sidecars), then asks. Without a terminal
it refuses unless `--yes` is given. The `loads` history is kept.

## `filings`

```text
$ hardmoney filings --help
Find filings via the FEC's API and fetch, validate, or ingest them

Usage: hardmoney filings [OPTIONS]

Options:
      --committee <ID>               Committee id, e.g. C00554709
      --candidate <ID>               Candidate id, e.g. H4CA11114
      --cycle <CYCLE>                Two-year cycle, e.g. 2026
      --form-type <FORM>             Base form type, e.g. F3X, F3, F24 (no amendment suffix)
      --report-type <CODE>           Report type code, e.g. Q2, 12G, M9
      --most-recent                  Only the latest version of each report
      --since <DATE>                 Only filings received on or after this date (YYYY-MM-DD)
      --until <DATE>                 Only filings received on or before this date (YYYY-MM-DD)
      --limit <N>                    Stop after this many filings (pages are fetched as needed)
      --json                         Print the records as a JSON array (openFEC's field names) instead of a table. With an action flag, each record gains an `actions` object
      --fetch <DIR>                  Download each filing's raw .fec (cache-first) into this directory as <id>.fec
      --validate                     Run the FEC's acceptance rules on each filing and print the error/warning counts
      --reconcile                    Recompute each report's cover-page totals from its schedules and print whether it balances
      --ingest                       Ingest each filing into Postgres (needs --database-url)
      --database-url <DATABASE_URL>  Postgres connection URL for --ingest. Include a username: postgres://user@host/db [env: DATABASE_URL=]
      --schema <SCHEMA>              Namespace (Postgres schema) for --ingest. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
      --cache-dir <CACHE_DIR>        Where downloaded filings and daily archives are kept across runs [env: HARDMONEY_CACHE_DIR=] [default: /Users/chris.gorski/.cache/hardmoney]
  -h, --help                         Print help
```

Queries openFEC's `/filings/` with an API key from `FEC_API_KEY` or
`~/fec_api_key.txt` (exit 1 with both locations named if neither has
one). Without any filter the command refuses to run unless `--limit` is
given. Results are newest first; the table shows `file_number`, `form`,
`report`, `coverage`, `receipt_date`, `amend` (openFEC's
`amendment_version`), `most_recent`, `total_receipts`, and `committee`.
Paper filings have negative file numbers and are marked `(paper)`; the
action flags skip them. The `--cache-dir` default shown is
`$XDG_CACHE_HOME/hardmoney` if that variable is set, else
`~/.cache/hardmoney`. See
[Finding filings](./discovery.md).

## `efile`

```text
$ hardmoney efile --help
Follow the FEC's electronic filing feed

Usage: hardmoney efile <COMMAND>

Commands:
  watch        Poll the e-filing RSS feed and process every filing not seen before
  backfill     Walk the FEC's daily e-filing archives over a date range
  cache-info   Show where the download cache is and what it holds
  cache-clear  Delete cached raw filings and daily archives
  help         Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

None of these needs an API key. See [Finding filings](./discovery.md).

### `efile watch`

```text
$ hardmoney efile watch --help
Poll the e-filing RSS feed and process every filing not seen before

Usage: hardmoney efile watch [OPTIONS]

Options:
      --interval <SECONDS>           Seconds between polls [default: 300]
      --once                         Poll once and exit (exit 1 if the poll or any filing failed)
      --form-type <FORMS>            Only these form types, comma-separated; a base type matches its amendments too (F3X matches F3XN, F3XA, F3XT)
      --committee <IDS>              Only filings by these committee ids, comma-separated
      --mark-seen                    Record every id currently in the feed as seen, without processing anything, then exit: "start following from now"
      --exec <PROGRAM>               After the other actions, run `PROGRAM <id> <path-to-cached-fec>` with HARDMONEY_FILING_ID and HARDMONEY_FILING_PATH in its environment. A non-zero exit is reported as a failure
      --json                         One JSON object per new filing (JSON Lines) instead of text
      --out <DIR>                    Copy each filing's raw .fec into this directory as <id>.fec
      --validate                     Run the FEC's acceptance rules on each filing and print the error/warning counts
      --reconcile                    Recompute each report's cover-page totals from its schedules and print whether it balances
      --ingest                       Ingest each filing into Postgres (needs --database-url)
      --database-url <DATABASE_URL>  Postgres connection URL for --ingest. Include a username: postgres://user@host/db [env: DATABASE_URL=]
      --schema <SCHEMA>              Namespace (Postgres schema) for --ingest. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
      --cache-dir <CACHE_DIR>        Where downloaded filings and daily archives are kept across runs [env: HARDMONEY_CACHE_DIR=] [default: /Users/chris.gorski/.cache/hardmoney]
  -h, --help                         Print help
```

Seen ids live in `<cache-dir>/efile-seen.txt`; every new id is recorded,
whether or not the filters selected it. Without `--once` the command
runs until interrupted, reporting (not exiting on) failed polls and
failed filings. One line per processed filing goes to stdout (`id`,
form type as filed, committee id, publication time, committee name, then
indented action results); the per-poll summary goes to stderr.

### `efile backfill`

```text
$ hardmoney efile backfill --help
Walk the FEC's daily e-filing archives over a date range

Usage: hardmoney efile backfill [OPTIONS] --from <DATE>

Options:
      --from <DATE>                  First day to read (YYYYMMDD or YYYY-MM-DD; archives exist from 2001-02-01 through yesterday)
      --to <DATE>                    Last day to read, inclusive [default: --from]
      --form-type <FORMS>            Only these form types, comma-separated; a base type matches its amendments too (F3X matches F3XN, F3XA, F3XT)
      --json                         One JSON object per filing (JSON Lines) instead of text
      --out <DIR>                    Copy each filing's raw .fec into this directory as <id>.fec
      --validate                     Run the FEC's acceptance rules on each filing and print the error/warning counts
      --reconcile                    Recompute each report's cover-page totals from its schedules and print whether it balances
      --ingest                       Ingest each filing into Postgres (needs --database-url)
      --database-url <DATABASE_URL>  Postgres connection URL for --ingest. Include a username: postgres://user@host/db [env: DATABASE_URL=]
      --schema <SCHEMA>              Namespace (Postgres schema) for --ingest. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
      --cache-dir <CACHE_DIR>        Where downloaded filings and daily archives are kept across runs [env: HARDMONEY_CACHE_DIR=] [default: /Users/chris.gorski/.cache/hardmoney]
  -h, --help                         Print help
```

Archives are cached under `<cache-dir>/efile/YYYYMMDD.zip`. A day with no
archive (today; the odd holiday) is reported and skipped, and the command
exits 1 at the end if any day was unavailable or any filing failed.

### `efile cache-info`

```text
$ hardmoney efile cache-info --help
Show where the download cache is and what it holds

Usage: hardmoney efile cache-info [OPTIONS]

Options:
      --json                   Print JSON instead of text
      --cache-dir <CACHE_DIR>  Where downloaded filings and daily archives are kept across runs [env: HARDMONEY_CACHE_DIR=] [default: /Users/chris.gorski/.cache/hardmoney]
  -h, --help                   Print help
```

### `efile cache-clear`

```text
$ hardmoney efile cache-clear --help
Delete cached raw filings and daily archives

Usage: hardmoney efile cache-clear [OPTIONS]

Options:
      --seen                   Also forget which e-filings `watch` has already processed
      --cache-dir <CACHE_DIR>  Where downloaded filings and daily archives are kept across runs [env: HARDMONEY_CACHE_DIR=] [default: /Users/chris.gorski/.cache/hardmoney]
  -h, --help                   Print help
```

Removes `<cache-dir>/filings/` and `<cache-dir>/efile/` only; `dumps/`
(from `bulk-restore-dump`) and, without `--seen`, `efile-seen.txt` are
left alone.

## `serve`

```text
$ hardmoney serve --help
Run the REST API server

Usage: hardmoney serve [OPTIONS] --database-url <DATABASE_URL>

Options:
      --database-url <DATABASE_URL>
          Postgres connection URL. Include a username: postgres://user@host/db [env: DATABASE_URL=]
      --schema <SCHEMA>
          Namespace (Postgres schema) to operate in. Each namespace is a fully isolated set of hardmoney tables: use one per cycle, per snapshot, per investigation, or per CI run. Default: public [env: HARDMONEY_SCHEMA=] [default: public]
      --bind <BIND>
          [default: 0.0.0.0:8080]
      --timeout-secs <TIMEOUT_SECS>
          Per-request timeout in seconds. Queries are also bounded server-side by a Postgres statement_timeout slightly below this [default: 30]
      --cors-origin <CORS_ORIGINS>
          Allowed CORS origin (repeatable). Default: any origin
      --api-key <API_KEY>
          Require this key in `X-Api-Key` (or `?api_key=`) on every route except /health [env: HARDMONEY_API_KEY]
      --max-connections <MAX_CONNECTIONS>
          Maximum database connections in the pool [default: 10]
      --ui
          Also serve the browser UI at /ui and the filing tools at /tools/* (parse, validate, reconcile, write). Raises the request body cap from 64 KiB to 32 MiB so a filing can be uploaded
  -h, --help
          Print help
```

Runs until `SIGINT`/`SIGTERM`. See [The REST API](./rest-api.md) and
[Hardening the API](./api-hardening.md).

`--ui` adds the filing workbench and data browser at `/ui` and the
database-free `/tools/*` routes behind them; both are embedded in the
binary. The `/ui` pages load without the API key (they are static files);
every `/tools/*` call needs it like any other route. See
[The browser UI](./web-ui.md).

## `query`

```text
$ hardmoney query --help
Search loaded data from the terminal (same results as the REST API)

Usage: hardmoney query [OPTIONS] <COMMAND>

Commands:
  candidates     Find candidates by name, state, office, or cycle
  candidate      Show one candidate (all cycles) by ID, e.g. S6NC00407
  committees     Find committees by name, type, or cycle
  committee      Show one committee (all cycles) by ID, e.g. C00913566
  contributions  Itemized contributions (Schedule A) from the bulk `indiv` file
  disbursements  Operating expenditures (Schedule B) from the bulk `oppexp` file
  ies            Independent expenditures (super PAC spending) for or against a candidate, from the FEC's Schedule E pg_dump
  filing         A directly-ingested filing's header/summary
  filing-ies     A directly-ingested filing's Schedule E lines
  schema         What's loaded in this namespace
  help           Print this message or the help of the given subcommand(s)

Options:
      --database-url <DATABASE_URL>  Postgres connection URL (used for in-process queries). Ignored when --api-url is set [env: DATABASE_URL=]
      --schema <SCHEMA>              Namespace (Postgres schema) to query. Ignored when --api-url is set [env: HARDMONEY_SCHEMA=] [default: public]
      --api-url <API_URL>            Query a running `hardmoney serve` at this base URL instead of the database, e.g. http://localhost:8080 [env: HARDMONEY_API_URL=]
      --api-key <API_KEY>            API key for --api-url (sent as X-Api-Key) [env: HARDMONEY_API_KEY]
      --json                         Print the raw JSON the API returns instead of a table
      --limit <LIMIT>                Maximum rows (1-500) [default: 25]
      --offset <OFFSET>              Skip this many rows (for paging) [default: 0]
  -h, --help                         Print help
```

Every `query` subcommand builds the same request the REST API serves
and runs it one of two ways. By default it constructs the API's router
in-process against `--database-url`/`--schema` and dispatches the
request to it directly. No server needs to be running, and the results
are the API's results because it is the API. With `--api-url` the
request is sent over HTTP to a running `hardmoney serve` instead (with
`--api-key` if that server requires one), for when the database isn't
reachable from your machine but the API is. Output is a fixed-width
table of the most useful columns, or the API's raw JSON with `--json`.
The six global options are accepted before or after the subcommand.

Each list subcommand's flags map one-to-one onto the corresponding
route's query parameters (see [All routes](./rest-api.md#all-routes));
the flag names are the human-readable ones. `contributions`, for
example:

```text
$ hardmoney query contributions --help
Itemized contributions (Schedule A) from the bulk `indiv` file

Usage: hardmoney query contributions [OPTIONS]

Options:
      --committee <COMMITTEE>        Committee that received the money
      --database-url <DATABASE_URL>  Postgres connection URL (used for in-process queries). Ignored when --api-url is set [env: DATABASE_URL=]
      --cycle <CYCLE>
      --schema <SCHEMA>              Namespace (Postgres schema) to query. Ignored when --api-url is set [env: HARDMONEY_SCHEMA=] [default: public]
      --api-url <API_URL>            Query a running `hardmoney serve` at this base URL instead of the database, e.g. http://localhost:8080 [env: HARDMONEY_API_URL=]
  -q, --name <NAME>                  Substring of contributor name
      --api-key <API_KEY>            API key for --api-url (sent as X-Api-Key) [env: HARDMONEY_API_KEY]
      --employer <EMPLOYER>
      --json                         Print the raw JSON the API returns instead of a table
      --occupation <OCCUPATION>
      --limit <LIMIT>                Maximum rows (1-500) [default: 25]
      --state <STATE>
      --offset <OFFSET>              Skip this many rows (for paging) [default: 0]
      --zip <ZIP>                    ZIP prefix
      --min-amount <MIN_AMOUNT>
      --max-amount <MAX_AMOUNT>
      --since <SINCE>                YYYY-MM-DD
      --until <UNTIL>                YYYY-MM-DD
  -h, --help                         Print help
```

| `query ...` | Route | Flag -> parameter |
|---|---|---|
| `candidates` | `GET /candidates` | `-q/--name` -> `q`, `--cycle`, `--state`, `--office` |
| `candidate <CAND_ID>` | `GET /candidates/{cand_id}` | none |
| `committees` | `GET /committees` | `-q/--name` -> `q`, `--cycle`, `--type` -> `cmte_tp` |
| `committee <CMTE_ID>` | `GET /committees/{cmte_id}` | none |
| `contributions` | `GET /schedule-a` | `--committee` -> `cmte_id`, `--cycle`, `-q/--name` -> `name`, `--employer`, `--occupation`, `--state`, `--zip` -> `zip_code`, `--min-amount`, `--max-amount`, `--since` -> `min_date`, `--until` -> `max_date` |
| `disbursements` | `GET /disbursements` | `--committee` -> `cmte_id`, `--cycle`, `-q/--name` -> `name`, `--city`, `--state`, `--purpose`, `--min-amount`, `--max-amount`, `--since`, `--until` |
| `ies` | `GET /independent-expenditures` | `--candidate` -> `candidate_id`, `--committee` -> `cmte_id`, `--support-oppose` -> `support_oppose_code` |
| `filing <FILING_ID>` | `GET /filings/{filing_id}` | none |
| `filing-ies <FILING_ID>` | `GET /filings/{filing_id}/schedule-e` | none |
| `schema` | `GET /schema` | none (always prints JSON) |

`--limit` is validated before anything runs: a value outside 1-500 is
a usage error (exit 2), the same bounds the API enforces with a 400.
When a page comes back full, a note on stderr gives the `--offset` for
the next one. A non-2xx response from the API is exit 1 with the API's
error message. Every `curl` in
[Who is funding a candidate?](./tutorial-journalist.md) is shown next to
its `query` equivalent.

## `spec`

```text
$ hardmoney spec --help
Export or diff the machine-readable FEC format specification

Usage: hardmoney spec <COMMAND>

Commands:
  tables  List every table: version buckets, fields, and FEC spec rows
  fields  The fields of one table at one spec version, in column order
  export  The complete machine-readable specification: JSON, JSON Schema, or CSV
  diff    Fields added, removed, and moved between two spec versions
  help    Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

The FEC publishes its format only as an Excel workbook plus per-version
column tables; hardmoney compiles both into the binary (see
[The schema](./library-schema.md)), and `spec` is that data on the
command line. It needs no database and no filing. Table names are the
ones `Table` uses (`SchA`, `F3X`, `TEXT`, ...), case-insensitive; an
unknown name is a usage error listing every valid one, and if what you
typed is a form-type token like `SA11AI` the error tells you which table
a line with that token is parsed with.

### `spec tables`

```text
$ hardmoney spec tables --help
List every table: version buckets, fields, and FEC spec rows

Usage: hardmoney spec tables [OPTIONS]

Options:
      --json  Print JSON instead of an aligned table
  -h, --help  Print help
```

```text
$ hardmoney spec tables
table   buckets  fields  spec_rows  oldest  newest  paper
------  -------  ------  ---------  ------  ------  -----
F1      5        107     101        3.0     8.5     0
F10     3        31      0          5.0     6.9     0
...
SchA    13       51      45         1.0     8.5     19
SchI    2        33      0          3.0     8.4     0
...
TEXT    4        7       6          3.0     8.5     0
59 tables; spec rows describe version 8.5
```

(Trimmed.) `buckets` is the number of distinct column layouts, `fields`
the canonical fields known across all versions, `spec_rows` how many
rows the FEC's 8.5 workbook has for the table (0 for forms absent from
the current spec), `oldest`/`newest` the electronic versions
with a layout, and `paper` the number of `P`-prefixed paper-conversion
versions. `--json` emits the same as an array of objects with those
keys (`version_buckets`, `fields`, `spec_rows`, `oldest_version`,
`newest_version`, `paper_versions`).

### `spec fields`

```text
$ hardmoney spec fields --help
The fields of one table at one spec version, in column order

Usage: hardmoney spec fields [OPTIONS] <TABLE>

Arguments:
  <TABLE>  Table name as `hardmoney spec tables` lists it, e.g. SchA, F3X, TEXT (case-insensitive)

Options:
      --version <VERSION>  Spec version whose column layout to show, e.g. 8.5, 6.4, 3.00 [default: the bundled spec version]. Description, type, length, required level, and rule come from the FEC's spec rows, which describe the bundled version; columns come from the layout for the version asked for
      --json               Print a JSON array (one object per field) instead of an aligned table. Columns are 1-based in both
  -h, --help               Print help
```

```text
$ hardmoney spec fields SchA
SchA at 8.5: 45 fields in 45 columns, 45 FEC spec rows
col  field                          kind           len  required     description                    rule
---  -----------------------------  -------------  ---  -----------  -----------------------------  ------------------------------------------------------------------------------------
1    form_type                      alpha_numeric  8    error        FORM TYPE                      Appendix C. SA3L must be used with the F3L
2    filer_committee_id_number      alpha_numeric  9    error        FILER COMMITTEE ID NUMBER
3    transaction_id                 alpha_numeric  20   error        TRANSACTION ID                 must be unique and UPPER CASE for the life of the report (original + all amendments)
4    back_reference_tran_id         alpha_numeric  20                BACK REFERENCE TRAN ID NUMBER  Reference to the Tran ID of a Related Record
5    back_reference_sched_name      alpha_numeric  8                 BACK REFERENCE SCHED NAME      Ref to the Schedule that has the Related Record. SA3L must be used with the F3L
6    entity_type                    alpha_numeric  3    error        ENTITY TYPE                    [CAN|CCM|COM|IND|ORG|PAC|PTY]
7    contributor_organization_name  alpha_numeric  200  error        CONTRIBUTOR ORGANIZATION NAME  Required if NOT [IND|CAN]
...
```

(Trimmed.) Columns are 1-based here, as in the FEC's workbook. The
`required` column is `error`, `warning`, `conditional`, or blank for
optional; `kind` is `alpha`, `alpha_numeric`, `numeric`, or `amount`.
For a version other than 8.5, columns come from that version's layout
while the descriptive columns are joined by canonical field name from
the 8.5 spec rows, so a field the current spec has dropped (e.g.
`contribution_purpose_code` at 7.0) shows its column with the rest
blank. A version the table has no layout for is exit 1 with the list of
versions it does have:

```text
$ hardmoney spec fields SchI --version 8.5
error: SchI has no column layout for spec version 8.5; it has layouts for: 3.0, 3.1, ..., 8.3, 8.4
```

`--json` gives `[{column, name, description, kind, max_len, required,
rule}]`, where `required` is `{"level": "error"}` or
`{"level": "conditional", "condition": "..."}`.

### `spec export`

```text
$ hardmoney spec export --help
The complete machine-readable specification: JSON, JSON Schema, or CSV

Usage: hardmoney spec export [OPTIONS]

Options:
      --format <FORMAT>
          Output format. `json`: the raw data model (every table, every version-bucketed layout, every FEC spec row; 0-based columns). `json-schema`: one JSON Schema (draft 2020-12) per table describing a record as an object keyed by canonical field name, as a `$defs` bundle or, with --table, a standalone document. `csv`: one row per table, version bucket, and field, with the FEC's spec columns, for spreadsheet users. Columns are 1-based in json-schema and csv

          Possible values:
          - json:        The raw data model, 0-based columns
          - json-schema: JSON Schema draft 2020-12, one schema per table
          - csv:         One flat row per table, version bucket, and field

          [default: json]

      --table <TABLE>
          Only this table (e.g. SchA, F3X, TEXT; case-insensitive)

      --version <VERSION>
          For json-schema: the spec version the record is at, e.g. 8.5, 7.0 [default: the bundled spec version]; columns come from that version's layout, field rules from the bundled workbook. For json and csv: only layouts that cover this version [default: all]

      --json
          Accepted for consistency with the other subcommands; the same as `--format json`

  -h, --help
          Print help (see a summary with '-h')
```

Three renderings of the same compiled data; [The spec as
data](./spec-as-data.md) explains each and where the data comes from.
`--table` restricts any of them to one table; `--version` restricts `json`
and `csv` to the layouts covering that version and, for `json-schema`,
selects the version the record is at. A table with no layout at the
version, or a version no table has, is exit 1 with the same messages as
`spec fields` and `spec diff`.

**`--format json`** (the default) is the spec-as-data artifact: every
table, every version-bucketed layout (version list, width, and each
field's column), and every FEC spec row, in one JSON document.

```json
{
  "bundled_spec_version": "8.5",
  "column_base": 0,
  "tables": [
    {
      "table": "F1",
      "layouts": [
        { "versions": ["8.4", "8.5"], "width": 101,
          "fields": [ { "name": "form_type", "column": 0 },
                      { "name": "filer_committee_id_number", "column": 1 }, ... ] },
        ...
      ],
      "specs": [
        { "column": 0, "canonical": "form_type", "description": "FORM TYPE",
          "kind": "alpha_numeric", "max_len": 3, "required": { "level": "error" },
          "sample": "F1N", "value_reference": "F1+[N|A]", "rule": null,
          "forms": [], "allowed_values": [], "pattern": null },
        ...
      ]
    },
    ...
  ]
}
```

(Real values; lists trimmed with `...`. The whole document is about
1.7 MB.) It uses 0-based columns throughout, matching the Rust
`FieldDef::column` and `FieldSpec::column`; it says so in
`column_base`. It is the complete machine-readable version map for the
format, suitable for generating bindings in another language or checking
a vendor's own tables against.

**`--format json-schema`** is one JSON Schema (draft 2020-12) per table.
Without `--table` it is a bundle with the tables under `$defs`, keyed by
table name; with `--table SchA` it is one standalone document. Each
describes a record as an object keyed by canonical field name, every value
a string: `maxLength` from the FEC's type, `enum` from its code lists and
`pattern` from its formats (both allowing blank, and both only at the
bundled version), `required` for `X (error)` fields, and an `x-fec`
object per field with the workbook row (`column`, 1-based; `type`,
`required_level`, `sample`, `value_reference`, `rule`, `forms`,
`allowed_values`, `pattern`). `x-hardmoney` carries `spec_version`,
`table`, `layout_versions`, and `field_rules_version`.

```text
$ hardmoney spec export --format json-schema --table SchA | head -12
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "SchA record, FEC spec 8.5",
  "description": "One SchA record at FEC electronic filing spec version 8.5, as an object keyed by canonical field name. Every value is a string as filed (trimmed); a blank field is the empty string. x-fec.column is the 1-based position in the delimited line.",
  "type": "object",
  "properties": {
    "form_type": {
      "type": "string",
      "description": "FORM TYPE. Rule: Appendix C. SA3L must be used with the F3L",
      "maxLength": 8,
      "x-fec": {
        "column": 1,
```

The bundle at 8.5 has 49 `$defs` (1,875 fields) and is about 1.1 MB
pretty-printed. `--version 7.0` describes a record at that version: the
columns come from the 7.0 layout and the field rules are joined by name
from the 8.5 workbook, with `enum` and `pattern` left out (see the
chapter for why). `tests/spec_export.rs` checks that no record of any
fixture in `tests/fixtures/` is rejected by the schema for its table.

**`--format csv`** is one flat row per table, version bucket, and field,
with the FEC's spec columns joined by field name (7,361 rows, about
970 KB): the FEC's spreadsheet given back as a spreadsheet, complete across
versions. Columns: `table, versions` (space-separated, the versions the
bucket covers), `first_version, last_version, width, column` (1-based),
`field, rules_version` (`8.5`, or blank when the current workbook has no
row for the field), `description, type, kind, max_len, required_level,
required_condition, sample, value_reference, rule, forms, allowed_values,
pattern`. Multi-valued cells are `|`-separated, the workbook's own
convention; rule text is collapsed to one line.

```text
$ hardmoney spec export --format csv | grep '^SchA,8.0' | head -3
SchA,8.0 8.1 8.2 8.3 8.4 8.5,8.0,8.5,45,1,form_type,8.5,FORM TYPE,A/N-8,alpha_numeric,8,error,,SA11AI,SA[line# ref],Appendix C. SA3L must be used with the F3L,F3|F3X|F3P|F3L,,
SchA,8.0 8.1 8.2 8.3 8.4 8.5,8.0,8.5,45,2,filer_committee_id_number,8.5,FILER COMMITTEE ID NUMBER,A/N-9,alpha_numeric,9,error,,C00123456,,,F3|F3X|F3P|F3L,,
SchA,8.0 8.1 8.2 8.3 8.4 8.5,8.0,8.5,45,3,transaction_id,8.5,TRANSACTION ID,A/N-20,alpha_numeric,20,error,,A56123456789-1234,,must be unique and UPPER CASE for the life of the report (original + all amendments),F3|F3X|F3P|F3L,,
```

### `spec diff`

```text
$ hardmoney spec diff --help
Fields added, removed, and moved between two spec versions

Usage: hardmoney spec diff [OPTIONS] <FROM> <TO>

Arguments:
  <FROM>  The "before" spec version, e.g. 7.0
  <TO>    The "after" spec version, e.g. 8.5

Options:
      --table <TABLE>  Only diff this table
      --json           Print JSON instead of the one-line-per-table summary. Columns are 1-based in both
  -h, --help           Print help
```

```text
$ hardmoney spec diff 7.0 8.5
7.0 -> 8.5
F1: +lobbyist_registrant_pac_3@38, +lobbyist_registrant_pac_4@39, affiliated_committee_id_number 38->40, ...
F24: +original_amendment_date@4, committee_name 4->5, street_1 5->6, ...
F99: +filing_frequency@16, +pdf_attachment@17, text 16->18
SchA: -contribution_purpose_code@23, contribution_purpose_descrip 24->23, contributor_employer 25->24, ...
SchE: +disbursement_date@22, -expenditure_purpose_code@23, calendar_y_t_d_per_election_office 22->23, candidate_district 36->35, candidate_state 35->36
SchI: lost its layout (30 fields at 7.0; none at 8.5)
unchanged (31): F13, F132, F133, F1M, F2S, F3, F3L, F3P, F3P31, F3PS, F3S, F3X, F3Z, F4, ...
```

(Trimmed.) One line per table whose fields differ: `+name@col` was
added, `-name@col` was removed, `name a->b` moved (1-based columns).
Then tables that gained or lost a layout entirely, then the unchanged
ones. Tables with no layout at either version are left out. Both
versions must be ones some table has a layout for, otherwise exit 1 with
the known versions; with `--table`, a table with no layout at either end
is exit 1 naming the versions it does have. `--json` gives `{from, to,
column_base: 1, gained: [{table, fields}], lost: [...], changed:
[{table, from_width, to_width, changes: [{change: added|removed|moved,
name, column | from, to}]}], unchanged: [table, ...]}`.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success (including `bulk-load --if-changed` skipping an unchanged file, `bulk-restore-dump` succeeding despite `pg_restore`'s expected warnings, and `validate` finding only warnings without `--strict-warnings`) |
| 1 | a runtime error: parse failure, database error, refused replace, pending migrations, `bulk-load-all` with at least one failed source, `schema-drop` without `--yes`, `write --check` with a record that does not round-trip, `reconcile` with a disagreeing line (or an unsupported form), `validate` with an error finding (or any finding under `--strict-warnings`), a non-2xx API response from `query`, a `spec` version or table the bundled data does not have, `export` to an unwritable path or `export --include-filing-id` on a file name with no id, `filings` with no API key, no filter and no `--limit`, an openFEC error, or a filing whose download/parse/ingest failed, `efile watch --once` with a failed poll or filing, `efile backfill` with an unavailable day or a failed filing |
| 2 | invalid command-line usage (clap): unknown flag, odd `--cycle`, invalid `--schema` name, `query --limit` outside 1-500, an unknown `spec` table name or malformed version, an unknown `export --only` token or `--format`, a malformed `--since`/`--until`/`--from`/`--to` date |
