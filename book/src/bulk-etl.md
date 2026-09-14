# Loading Bulk Data into Postgres

A single filing (as parsed in the previous two chapters) tells you about
one committee's one reporting period. Most real questions -- "who gave
this candidate money," "how much has this PAC spent this cycle," "which
committees are active in Maine" -- need data across *every* filer at
once. The FEC publishes exactly that as **bulk data files**: large,
pre-aggregated downloads, refreshed periodically, one set per election
cycle. `hardmoney::bulk` loads those into a normal Postgres database with
a documented, normalized schema.

This chapter assumes you have a running Postgres instance and a
`DATABASE_URL`, as described in [Installation](./installation.md#what-you-need-for-each-part-of-the-crate).

## Step 1: apply the schema

Before loading anything, create the tables:

```bash
$ export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:5432/fec_dev"
$ cargo run --quiet --bin hardmoney -- schema-init --database-url "$DATABASE_URL"
```

```text
schema applied
```

This is idempotent -- run it again and it just reports every table
already exists and does nothing destructive. It's safe to run before
every load if you're scripting this.

## Step 2: load one bulk source

`bulk-load` downloads one source for one election cycle directly from
`fec.gov` and loads it:

```bash
$ cargo run --quiet --bin hardmoney -- bulk-load --database-url "$DATABASE_URL" candidates --cycle 2026
```

```text
loaded 8557 rows into candidates
```

That single command downloaded
`https://www.fec.gov/files/bulk-downloads/2026/cn26.zip`, unzipped it,
and streamed the rows into a `candidates` table via Postgres's `COPY`
protocol. The URL is built by `hardmoney::bulk::source::download_url`
from the cycle and the source's file-name stem -- covered by a unit test
(`download_url_pads_the_two_digit_year_and_uses_the_full_cycle_in_the_path`)
so a future refactor can't quietly break the two-digit-year padding.

Do the same for committees:

```bash
$ cargo run --quiet --bin hardmoney -- bulk-load --database-url "$DATABASE_URL" committees --cycle 2026
```

```text
loaded 20674 rows into committees
```

Every row landed in a real Postgres table with a real schema -- here's
what `candidates` actually looks like after that load
(`psql "$DATABASE_URL" -c '\d candidates'`):

```text
                    Table "public.candidates"
        Column        |  Type   | Collation | Nullable | Default
----------------------+---------+-----------+----------+---------
 cand_id              | text    |           | not null |
 cycle                | integer |           | not null |
 cand_name            | text    |           |          |
 cand_pty_affiliation | text    |           |          |
 cand_election_yr     | text    |           |          |
 cand_office_st       | text    |           |          |
 cand_office          | text    |           |          |
 cand_office_district | text    |           |          |
 cand_ici             | text    |           |          |
 cand_status          | text    |           |          |
 cand_pcc             | text    |           |          |
 cand_st1             | text    |           |          |
 cand_st2             | text    |           |          |
 cand_city            | text    |           |          |
 cand_st              | text    |           |          |
 cand_zip             | text    |           |          |
Indexes:
    "candidates_pkey" PRIMARY KEY, btree (cand_id, cycle)
```

The full list of source names accepted by `bulk-load` is: `candidates`,
`committees`, `candidate_committee_links`, `schedule_a`,
`committee_to_committee_transactions`,
`committee_to_candidate_transactions`, `disbursements`,
`candidate_summary`, `house_senate_summary`, `pac_party_summary`.

### Loading from a local file instead

If you've already downloaded a bulk file (or you're working somewhere
without direct internet access to `fec.gov`), pass `--file` to load from
disk instead of downloading:

```bash
hardmoney bulk-load --database-url "$DATABASE_URL" candidates --cycle 2026 --file ./cn26.zip
```

### Sampling a huge source with `--limit`

Some sources -- `schedule_a` (itemized individual contributions) in
particular -- are multiple gigabytes per cycle. If you just want to
explore the shape of the data without downloading and loading the whole
thing, `--limit` caps the number of rows read:

```bash
hardmoney bulk-load --database-url "$DATABASE_URL" schedule_a --cycle 2026 --limit 10000
```

## Step 3: or load everything for a cycle in one shot

`bulk-load-all` runs `bulk-load` for every source in sequence:

```bash
hardmoney bulk-load-all --database-url "$DATABASE_URL" --cycle 2026
```

It accepts the same `--limit` flag, applied to every source, which is
useful for a quick smoke-test load across the whole schema without
committing to a full multi-gigabyte download.

## An alternative: restoring the FEC's own database dumps

For a couple of specific tables, the FEC also publishes ready-made
Postgres `pg_dump` archives rather than requiring you to parse CSVs
yourself. `bulk-restore-dump` restores one of these directly:

```bash
hardmoney bulk-restore-dump --database-url "$DATABASE_URL" schedule_e
hardmoney bulk-restore-dump --database-url "$DATABASE_URL" committee_history
```

`schedule_e` and `committee_history` are small enough to restore
routinely. `schedule_a_full` and `schedule_b_full` are the complete,
un-cycle-limited itemized contribution and disbursement history --
**tens of gigabytes** -- and `bulk-restore-dump` refuses to start on
either unless you pass `--allow-large` explicitly, so you don't
accidentally kick off a huge download and disk-space commitment. Archives
are cached under `--cache-dir` (default `/tmp/hardmoney-dumps`) so a
retry doesn't re-download from scratch.

## Ingesting a single filing directly, for precise Schedule E data

The bulk `schedule_e` source above is *aggregated* -- the FEC periodically
rolls up independent-expenditure filings into one big file. If you need
today's just-filed independent expenditures before the next bulk refresh,
or you want to ingest one specific filing precisely (using the same exact
parser and typed views from the earlier chapters, not the FEC's
own bulk-aggregation logic), use `bulk-load-filing`:

```bash
$ cargo run --quiet --bin hardmoney -- bulk-load-filing --database-url "$DATABASE_URL" tests/fixtures/F24N_2011832.fec
```

```text
ingested F24N (2 Schedule E lines)
```

You can also pass a numeric filing ID instead of a local path, and it
downloads the filing live from `docquery.fec.gov` first (requires the
`fetch` feature, on by default) -- this is the same live-fetch mechanism
used in [Quick Start](./quick-start.md#fetching-a-filing-live-from-the-fec-optional):

```bash
hardmoney bulk-load-filing --database-url "$DATABASE_URL" 2011831
```

Confirming the exact values landed correctly in Postgres:

```bash
$ psql "$DATABASE_URL" -c \
    "SELECT payee_name, expenditure_amt FROM schedule_e_lines WHERE filing_id = 242011832;"
```

```text
      payee_name       | expenditure_amt
-----------------------+-----------------
 DECLARATION MEDIA LLC |        11282.23
 MVAR MEDIA, LLC       |         4555.33
(2 rows)
```

`expenditure_amt` is a Postgres `NUMERIC` column, storing the exact same
`Decimal` value the parser produced -- no floating-point column, and no
precision lost between the parser and the database. This continues all
the way to the REST API, covered next.

### Why is this ingestion path scoped only to genuine Schedule E lines?

One subtlety worth knowing: `ScheduleE::try_from` only strictly requires a
`filer_committee_id_number` field, which is present on nearly every
schedule (A, B, C, ...), not just E. Early in development,
`bulk-load-filing` naively tried converting *every* line in a filing to
`ScheduleE`, which meant a filing with, say, Schedule A contributions but
no independent expenditures would misfile every Schedule A line as an
all-null "Schedule E" row. The ingestion code now filters to
`line.table == "SchE"` before attempting the conversion, so a filing with
no independent expenditures correctly stores zero Schedule E rows. This
is covered by
`bulk::ingest::tests::only_genuine_schedule_e_lines_are_extracted` in the
test suite.
