# CLI Reference

This is the full `--help` output for `hardmoney` 1.0.0 and every
subcommand, captured directly from the built binary. Run
`hardmoney <subcommand> --help` yourself at any time to see this same
text -- it's the authoritative reference, since it's generated from the
same argument definitions the program actually runs with. (`bulk-load`
and `bulk-load-all` have long-form help; the others print the same text
for `-h` and `--help`.)

## Top level

```text
$ hardmoney --help
FEC campaign-finance data: parse .fec filings, load bulk data, serve a REST API

Usage: hardmoney <COMMAND>

Commands:
  parse              Parse a single `.fec` filing and print a JSON summary
  schema-init        Create or upgrade the Postgres schema in the target namespace
  schema-status      Show migration state and recorded loads for the target namespace
  schema-list        List hardmoney namespaces (schemas) in the database
  schema-drop        Drop a namespace and all data in it
  bulk-load          Load one bulk-data source into Postgres
  bulk-load-all      Load every bulk source for one cycle
  bulk-restore-dump  Restore one of the FEC's own official pg_dump archives
  bulk-load-filing   Ingest a single raw `.fec` filing directly
  serve              Run the REST API server
  help               Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version

Database commands read DATABASE_URL (or --database-url) and an optional HARDMONEY_SCHEMA (or --schema) naming an isolated namespace inside that database. Include a username in the URL: postgres://user@host/db
```

## The two flags every database command shares

Every subcommand except `parse` takes these, so they're described once
here and shown verbatim in each block below:

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
`lines`. See [Quick Start](./quick-start.md) for real examples and
[Strict vs. Lenient Parsing](./strict-vs-lenient.md) for `--lenient`.
Exits 1 on a parse error.

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
      --allow-large                  Required for the multi-gigabyte schedule_a_full / schedule_b_full
      --cache-dir <CACHE_DIR>        Where to keep downloaded dumps (re-used across runs) [env: HARDMONEY_CACHE_DIR=] [default: /Users/chris.gorski/.cache/hardmoney/dumps]
  -h, --help                         Print help
```

The `--cache-dir` default shown is `$XDG_CACHE_HOME/hardmoney/dumps` if
`XDG_CACHE_HOME` is set, else `~/.cache/hardmoney/dumps`, else the
system temp directory -- it will read differently on your machine. Needs
`pg_restore` on `PATH`. The dump always restores into the shared
`disclosure` schema; `--schema` controls where the
`independent_expenditures` view is refreshed. See
[Bulk ETL](./bulk-etl.md#an-alternative-restoring-the-fecs-own-database-dumps).

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
  -h, --help                         Print help
```

For a local path, the filing id is the last run of four or more digits
in the file name (`F24N_2011832.fec` -> 2011832); if there isn't one,
the command exits 1 asking for `--filing-id`. For a numeric argument,
the filing is downloaded from `docquery.fec.gov` and that number is the
id unless `--filing-id` overrides it. See
[Bulk ETL](./bulk-etl.md#ingesting-a-single-filing-directly-for-precise-schedule-e-data).

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
  -h, --help
          Print help
```

Runs until `SIGINT`/`SIGTERM`. See [The REST API](./rest-api.md) and
[Hardening the API](./api-hardening.md).

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success (including `bulk-load --if-changed` skipping an unchanged file, and `bulk-restore-dump` succeeding despite `pg_restore`'s expected warnings) |
| 1 | a runtime error: parse failure, database error, refused replace, pending migrations, `bulk-load-all` with at least one failed source, `schema-drop` without `--yes` |
| 2 | invalid command-line usage (clap): unknown flag, odd `--cycle`, invalid `--schema` name |
