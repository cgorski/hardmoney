# CLI Reference

This is the full `--help` output for `hardmoney` and every subcommand,
captured directly from the built binary. Run `hardmoney <subcommand>
--help` yourself at any time to see this same text -- it's the
authoritative reference, since it's generated from the same argument
definitions the program actually runs with.

## Top level

```text
$ hardmoney --help
FEC campaign-finance data: parse .fec filings, load bulk data, serve a REST API

Usage: hardmoney <COMMAND>

Commands:
  parse              Parse a single `.fec` filing and print its header/summary as JSON
  schema-init        Apply the Postgres schema (idempotent)
  bulk-load          Load one bulk-data source into Postgres
  bulk-load-all      Load every bulk source for one cycle
  bulk-restore-dump  Restore one of the FEC's own official pg_dump archives (schedule_e and committee_history are small and practical; schedule_a_full and schedule_b_full are tens of gigabytes and require --allow-large)
  bulk-load-filing   Ingest a single raw `.fec` filing directly (precise Schedule E extraction; complements the bulk-loaded, aggregate tables)
  serve              Run the REST API server
  help               Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

## `parse`

```text
$ hardmoney parse --help
Parse a single `.fec` filing and print its header/summary as JSON

Usage: hardmoney parse <PATH>

Arguments:
  <PATH>  Path to a `.fec` file

Options:
  -h, --help  Print help
```

See [Quick Start](./quick-start.md) for real examples.

## `schema-init`

```text
$ hardmoney schema-init --help
Apply the Postgres schema (idempotent)

Usage: hardmoney schema-init --database-url <DATABASE_URL>

Options:
      --database-url <DATABASE_URL>  [env: DATABASE_URL=]
  -h, --help                         Print help
```

`--database-url` can also be set via the `DATABASE_URL` environment
variable, as shown throughout [Loading Bulk Data into Postgres](./bulk-etl.md) --
every command below that takes `--database-url` accepts the same
environment variable.

## `bulk-load`

```text
$ hardmoney bulk-load --help
Load one bulk-data source into Postgres

Usage: hardmoney bulk-load [OPTIONS] --database-url <DATABASE_URL> --cycle <CYCLE> <SOURCE>

Arguments:
  <SOURCE>  One of: candidates, committees, candidate_committee_links, schedule_a, committee_to_committee_transactions, committee_to_candidate_transactions, disbursements, candidate_summary, house_senate_summary, pac_party_summary

Options:
      --database-url <DATABASE_URL>  [env: DATABASE_URL=]
      --cycle <CYCLE>                4-digit even-year election cycle, e.g. 2026
      --file <FILE>                  Load from a local file instead of downloading from fec.gov
      --limit <LIMIT>                Cap the number of rows read (samples without a full download for multi-gigabyte sources like `schedule_a`)
  -h, --help                         Print help
```

## `bulk-load-all`

```text
$ hardmoney bulk-load-all --help
Load every bulk source for one cycle

Usage: hardmoney bulk-load-all [OPTIONS] --database-url <DATABASE_URL> --cycle <CYCLE>

Options:
      --database-url <DATABASE_URL>  [env: DATABASE_URL=]
      --cycle <CYCLE>
      --limit <LIMIT>
  -h, --help                         Print help
```

## `bulk-restore-dump`

```text
$ hardmoney bulk-restore-dump --help
Restore one of the FEC's own official pg_dump archives (schedule_e and committee_history are small and practical; schedule_a_full and schedule_b_full are tens of gigabytes and require --allow-large)

Usage: hardmoney bulk-restore-dump [OPTIONS] --database-url <DATABASE_URL> <NAME>

Arguments:
  <NAME>  One of: schedule_e, committee_history, schedule_a_full, schedule_b_full

Options:
      --database-url <DATABASE_URL>  [env: DATABASE_URL=]
      --allow-large
      --cache-dir <CACHE_DIR>        [default: /tmp/hardmoney-dumps]
  -h, --help                         Print help
```

## `bulk-load-filing`

```text
$ hardmoney bulk-load-filing --help
Ingest a single raw `.fec` filing directly (precise Schedule E extraction; complements the bulk-loaded, aggregate tables)

Usage: hardmoney bulk-load-filing --database-url <DATABASE_URL> <FILING>

Arguments:
  <FILING>  A local `.fec` file path, or a numeric filing id to download from the FEC's own document store (requires the `fetch` feature, on by default)

Options:
      --database-url <DATABASE_URL>  [env: DATABASE_URL=]
  -h, --help                         Print help
```

## `serve`

```text
$ hardmoney serve --help
Run the REST API server

Usage: hardmoney serve [OPTIONS] --database-url <DATABASE_URL>

Options:
      --database-url <DATABASE_URL>  [env: DATABASE_URL=]
      --bind <BIND>                  [default: 0.0.0.0:8080]
  -h, --help                         Print help
```
