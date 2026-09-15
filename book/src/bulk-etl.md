# Loading Bulk Data into Postgres

A single filing (as parsed in the previous chapters) tells you about one
committee's one reporting period. Most real questions -- "who gave this
candidate money," "how much has this PAC spent this cycle," "which
committees are active in Maine" -- need data across *every* filer at
once. The FEC publishes exactly that as **bulk data files**: large,
pre-aggregated downloads, refreshed weekly, one set per election cycle.
`hardmoney::bulk` loads those into a normal Postgres database with a
versioned, migrated schema.

This chapter assumes you have a running Postgres instance and a
`DATABASE_URL` (with a username in it), as described in
[Installation](./installation.md#what-you-need-for-each-part-of-the-crate).
Every command below reads `DATABASE_URL` from the environment; you can
pass `--database-url` instead.

The examples in this chapter were run inside a namespace called
`book_demo` (`--schema book_demo`) so they could be thrown away
afterwards. You can leave `--schema` off entirely to work in the default
`public` schema -- the output is identical -- but namespaces are worth
knowing about early, so they get the [next chapter](./namespaces.md).

## Step 1: apply the schema

Before loading anything, create the tables:

```bash
$ export DATABASE_URL="postgres://chris.gorski@localhost:5432/hardmoney_v1_test"
$ cargo run --quiet --bin hardmoney -- schema-init --schema book_demo
```

```text
namespace 'book_demo': schema at version 2 (2 migration(s) applied now)
```

The schema is a set of numbered SQL migrations in the repository's
`migrations/` directory, applied with `sqlx::migrate!`:

| File | What it does |
|---|---|
| `0001_initial_schema.sql` | every table: the ten bulk-data tables, `filings` + `schedule_e_lines` for directly-ingested filings, and `loads` (one row per bulk load) |
| `0002_trigram_search_indexes.sql` | `pg_trgm` GIN indexes on the columns the REST API searches with `ILIKE '%term%'` (candidate and committee names, contributor/payee names, employers) |

sqlx records each applied file in a `_sqlx_migrations` table and refuses
to run if an already-applied file has been edited, so a schema change is
always a *new* numbered file, never an edit to an old one. (A single
`CREATE TABLE IF NOT EXISTS` script can't offer that: it silently does
nothing when a column is added or a type changes.)

Running `schema-init` again is a no-op:

```bash
$ cargo run --quiet --bin hardmoney -- schema-init --schema book_demo
```

```text
namespace 'book_demo': schema at version 2 (0 migration(s) applied now)
```

`schema-status` shows where a namespace stands, and -- once you've loaded
something -- what's in it:

```bash
$ cargo run --quiet --bin hardmoney -- schema-status --schema book_demo
```

```text
namespace:  book_demo
applied:    [1, 2]
pending:    []
loads:      (none)
```

**Every database command checks this first.** If you try to load into a
namespace that hasn't been migrated (or is behind), you get a refusal,
not a `COPY` into a stale table layout:

```bash
$ cargo run --quiet --bin hardmoney -- bulk-load --schema fresh_ns candidates --cycle 2026
```

```text
error: namespace 'fresh_ns' has 2 pending migration(s); run `hardmoney schema-init` first
```

About the trigram indexes: `pg_trgm` is a trusted extension on community
Postgres 13+ and on Aurora PostgreSQL, so a non-superuser database owner
can create it. If it's unavailable on your host, the migration raises a
`WARNING`, skips the indexes, and everything still works -- substring
searches in the API just fall back to sequential scans.

## Step 2: load one bulk source

`bulk-load` downloads one source for one election cycle directly from
`fec.gov` and loads it:

```bash
$ cargo run --quiet --bin hardmoney -- bulk-load --schema book_demo candidates --cycle 2026
```

```text
candidates: loaded 8557 rows for cycle 2026 (replace)
```

That single command downloaded
`https://www.fec.gov/files/bulk-downloads/2026/cn26.zip`, decoded it,
and streamed the rows into a `candidates` table via Postgres's `COPY`
protocol -- in about a second and a half. The URL is built by
`hardmoney::bulk::source::download_url` from the cycle and the source's
file-name stem (covered by a unit test so a future refactor can't
quietly break the two-digit-year padding). The `(replace)` at the end is
the load mode; it's the default, and what it means is the subject of
[Reloading](./reloading.md).

Do the same for committees:

```bash
$ cargo run --quiet --bin hardmoney -- bulk-load --schema book_demo committees --cycle 2026
```

```text
committees: loaded 20674 rows for cycle 2026 (replace)
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
    "candidates_cand_name_trgm_idx" gin (cand_name gin_trgm_ops)
```

Column names mirror the FEC's own bulk-data header files rather than the
openFEC API's naming, so a column here maps 1:1 onto the FEC's own
file-description pages. Every bulk table is keyed by its natural key
*plus* `cycle`, so multiple two-year cycles can coexist in one table.

The full list of source names accepted by `bulk-load` is: `candidates`,
`committees`, `candidate_committee_links`, `schedule_a`,
`committee_to_committee_transactions`,
`committee_to_candidate_transactions`, `disbursements`,
`candidate_summary`, `house_senate_summary`, `pac_party_summary`.

### The cycle is validated

`--cycle` is a `hardmoney::Cycle`: an even year from 1976 through 2100.
An odd year is rejected before anything is downloaded -- fec.gov has no
`2027/` directory, so the alternative would be fetching a 404 page and
trying to unzip it:

```bash
$ cargo run --quiet --bin hardmoney -- bulk-load --schema book_demo candidates --cycle 2027
```

```text
error: invalid value '2027' for '--cycle <CYCLE>': cycle 2027 is not an even year (cycles are the even year of a two-year period)

For more information, try '--help'.
```

The same type is used in the library (`Cycle::new(2026)?`,
`"2026".parse::<Cycle>()?`, `cycle.two_digit()` for the `26` in
`cn26.zip`) and in the REST API, where `?cycle=2027` is a 400.

### Loading from a local file instead

If you've already downloaded a bulk file (or you're working somewhere
without direct internet access to `fec.gov`), pass `--file` to load from
disk instead of downloading:

```bash
hardmoney bulk-load --schema book_demo candidates --cycle 2026 --file ./cn26.zip
```

### Sampling a huge source with `--limit`

Some sources -- `schedule_a` (itemized individual contributions) in
particular, at over 2 GB per cycle -- are far too big to download just to
look at. `--limit` caps the number of rows read, and it stops the
download once it has them:

```bash
$ time cargo run --quiet --bin hardmoney -- bulk-load --schema book_demo disbursements --cycle 2026 --limit 50
```

```text
disbursements: loaded 50 rows for cycle 2026 (replace)

real	0m0.926s
```

Under a second, for a source whose zip is about 45 MB: the loader stops
pulling bytes from the network the moment the 50th row is written, so the
same flag on the 2 GB `schedule_a` file costs seconds, not gigabytes. A
limited load is recorded as a *sample* -- `schema-status` shows
`(sample, limit 50)` next to it, and it's never treated as a "current"
full load by `--if-changed`.

## Step 3: or load everything for a cycle in one shot

`bulk-load-all` runs `bulk-load` for every source in sequence. It accepts
the same `--limit`, `--mode`, `--yes`, and `--if-changed` flags, applied
to every source, so a quick smoke-test load across the whole schema is:

```bash
$ time cargo run --quiet --bin hardmoney -- bulk-load-all --schema book_demo --cycle 2026 --limit 100
```

```text
loading candidates from https://www.fec.gov/files/bulk-downloads/2026/cn26.zip ...
candidates: loaded 100 rows for cycle 2026 (replace, replaced 8557 existing)
loading committees from https://www.fec.gov/files/bulk-downloads/2026/cm26.zip ...
committees: loaded 100 rows for cycle 2026 (replace, replaced 20674 existing)
loading candidate_committee_links from https://www.fec.gov/files/bulk-downloads/2026/ccl26.zip ...
candidate_committee_links: loaded 100 rows for cycle 2026 (replace)
loading schedule_a from https://www.fec.gov/files/bulk-downloads/2026/indiv26.zip ...
schedule_a: loaded 100 rows for cycle 2026 (replace)
loading committee_to_committee_transactions from https://www.fec.gov/files/bulk-downloads/2026/oth26.zip ...
committee_to_committee_transactions: loaded 100 rows for cycle 2026 (replace)
loading committee_to_candidate_transactions from https://www.fec.gov/files/bulk-downloads/2026/pas226.zip ...
committee_to_candidate_transactions: loaded 100 rows for cycle 2026 (replace)
loading disbursements from https://www.fec.gov/files/bulk-downloads/2026/oppexp26.zip ...
disbursements: loaded 100 rows for cycle 2026 (replace, replaced 50 existing)
loading candidate_summary from https://www.fec.gov/files/bulk-downloads/2026/weball26.zip ...
candidate_summary: loaded 100 rows for cycle 2026 (replace)
loading house_senate_summary from https://www.fec.gov/files/bulk-downloads/2026/webl26.zip ...
house_senate_summary: loaded 100 rows for cycle 2026 (replace)
loading pac_party_summary from https://www.fec.gov/files/bulk-downloads/2026/webk26.zip ...
pac_party_summary: loaded 100 rows for cycle 2026 (replace)

real	0m7.969s
```

Eight seconds for the first hundred rows of all ten files, including the
multi-gigabyte ones -- because the download stops at row 100. Note the
`replaced 8557 existing` on `candidates`: this sample *replaced* the full
load from Step 2, because `--mode replace` is the default. If you're
sampling alongside real data, use a separate namespace.

Without `--limit`, this is the command for a complete cycle. By default
it continues past a source that fails and exits non-zero at the end
listing which ones did; `--fail-fast` stops at the first failure
instead.

## An alternative: restoring the FEC's own database dumps

For a couple of specific tables, the FEC also publishes ready-made
Postgres `pg_dump` archives (updated Saturdays) rather than requiring you
to parse CSVs yourself. `bulk-restore-dump` downloads and restores one of
these directly; it needs `pg_restore` on your `PATH`:

```bash
hardmoney bulk-restore-dump --schema book_demo schedule_e
hardmoney bulk-restore-dump --schema book_demo committee_history
```

```text
restored 549525 rows into disclosure.fec_fitem_sched_e (dump cached at /Users/you/.cache/hardmoney/dumps/schedule_e.dump)
independent_expenditures view refreshed in namespace 'book_demo'
```

*(Illustrative: the output format is taken from the code, and 549,525 is
the real row count of the `schedule_e` dump restored into this book's
database -- covering report years 1975 through 2026 -- but the restore
command itself was not re-run while writing this chapter, and the FEC's
weekly refresh will change the number.)* A few things to know:

- `schedule_e` (~43 MB, independent expenditures 1975-present) and
  `committee_history` (~14 MB) are small enough to restore routinely.
  `schedule_a_full` (~90 GB) and `schedule_b_full` (~39 GB) are the
  complete, un-cycle-limited itemized histories, and `bulk-restore-dump`
  refuses to start on either unless you pass `--allow-large` explicitly.
- Downloads are cached under `~/.cache/hardmoney/dumps` (honouring
  `XDG_CACHE_HOME`; override with `--cache-dir` or `HARDMONEY_CACHE_DIR`),
  written to a `.partial` file and renamed only on completion, so an
  interrupted download is never mistaken for a good one.
- **The dump always lands in the `disclosure` schema**, whatever
  `--schema` you pass, because the FEC's own DDL hard-codes it. It's
  reference data shared by every namespace; what *is* per-namespace is
  the friendly `independent_expenditures` view that hardmoney creates
  over it (`db::ensure_views`, run after every `schema-init`, every
  `serve` start, and every restore). The REST API's
  `/independent-expenditures` route reads that view.
- Re-running a restore drops the table first, so it's a clean weekly
  refresh rather than a pile of "already exists" errors.
- `pg_restore` itself exits non-zero for an expected, ignorable warning
  (the dump references a trigger function that exists only inside the
  FEC's own database), so hardmoney judges success by whether the target
  table exists and has rows afterwards -- and exits 0 when it does. Any
  messages `pg_restore` emitted are summarized on stderr
  (`pg_restore reported N non-fatal message(s); first: ...`).

## Ingesting a single filing directly, for precise Schedule E data

The bulk `schedule_e` dump above is *aggregated* -- the FEC rolls up
independent-expenditure filings into one big table on its own schedule.
If you need today's just-filed independent expenditures before the next
refresh, or you want to ingest one specific filing precisely (using the
same parser and typed views from the earlier chapters, not the FEC's own
aggregation logic), use `bulk-load-filing`:

```bash
$ cargo run --quiet --bin hardmoney -- bulk-load-filing --schema book_demo tests/fixtures/F24N_2011832.fec
```

```text
ingested filing 2011832 (F24N): 2 Schedule E line(s), 0 skipped
```

Three things are happening in that one line of output.

**The filing id came from the filename.** The FEC's document store names
downloads `<id>.fec`; this crate's fixtures are `<FORM>_<id>[_v<spec>].fec`.
The id is the **last** run of four or more digits in the stem, so
`F24N_2011832.fec` is 2011832 and `F3XA_27789_v3.fec` is 27789 -- the
`24` in the form name and the `3` in the spec-version suffix are
ignored. If the name has no such run, the command asks rather than
guessing:

```bash
$ cargo run --quiet --bin hardmoney -- bulk-load-filing --schema book_demo /tmp/notes.fec
```

```text
error: could not derive a filing id from '/tmp/notes.fec'; pass --filing-id
```

`--filing-id N` overrides whatever the filename says. You can also pass
a bare numeric id instead of a path, and the filing is downloaded live
from `docquery.fec.gov` first (requires the `fetch` feature, on by
default) -- the same mechanism as
[Quick Start](./quick-start.md#fetching-a-filing-live-from-the-fec-optional):

```bash
hardmoney bulk-load-filing --schema book_demo 2011831
```

**Only genuine Schedule E lines were extracted.** The ingester uses
`line.view::<ScheduleE>()` on the `Table::SchE` lines, so a filing with
Schedule A and B lines but no independent expenditures stores zero
Schedule E rows -- a Schedule A line can't be mistaken for an empty
Schedule E record (see
[Views check the table](./typed-views.md#views-check-the-table)):

```bash
$ cargo run --quiet --bin hardmoney -- bulk-load-filing --schema book_demo tests/fixtures/F3A_767339_v8.0.fec
```

```text
ingested filing 767339 (F3A): 0 Schedule E line(s), 0 skipped
```

That's an amended Form 3 with 641 body lines (576 Schedule A, 60
Schedule B, 3 F3Z, 2 Schedule D) and, correctly, no Schedule E rows.

**Parsing was lenient, and `0 skipped` says nothing was dropped.**
`bulk-load-filing` uses `ParseOptions::LENIENT` by default -- an
ingestion job shouldn't die because one line type in a 700,000-line
filing is unknown -- and stores the skip count in `filings.skipped_lines`,
where `GET /filings/{id}` reports it. Using the junk file from
[Strict vs. Lenient Parsing](./strict-vs-lenient.md):

```bash
$ cargo run --quiet --bin hardmoney -- bulk-load-filing --schema book_demo --filing-id 999 /tmp/with_junk.fec
```

```text
ingested filing 999 (F24N): 2 Schedule E line(s), 1 skipped
  skipped line 5: 'ZZZ' skipped (unknown form type)
```

Pass `--strict` when you'd rather the job fail:

```bash
$ cargo run --quiet --bin hardmoney -- bulk-load-filing --schema book_demo --strict --filing-id 998 /tmp/with_junk.fec
```

```text
error: filing parse error: no format table for form type 'ZZZ' (spec version 8.5) at line 5
```

Re-ingesting the same filing id replaces the previous rows (header,
summary, and Schedule E lines) in one transaction, so it's safe to
re-run on an amendment or after a parser upgrade.

Confirming the exact values landed correctly in Postgres:

```bash
$ psql "$DATABASE_URL" -c "SET search_path TO book_demo, public;" \
    -c "SELECT payee_name, expenditure_amt FROM schedule_e_lines WHERE filing_id = 2011832;"
```

```text
      payee_name       | expenditure_amt
-----------------------+-----------------
 DECLARATION MEDIA LLC |        11282.23
 MVAR MEDIA, LLC       |         4555.33
(2 rows)
```

`expenditure_amt` is a Postgres `NUMERIC` column, storing the exact same
`Decimal` value the parser produced -- sqlx binds `rust_decimal::Decimal`
straight to the `NUMERIC` wire format, so there is no floating-point
column and no precision lost between the parser and the database. This
continues all the way to the REST API, covered after the next three
shorter chapters on [namespaces](./namespaces.md),
[reloading](./reloading.md), and [dates](./dates.md).

## Doing this from Rust instead of the CLI

Everything above is a thin wrapper over `hardmoney::bulk` and
`hardmoney::db`. The equivalent of Steps 1 and 2 plus a filing ingest,
as library calls:

```rust
use hardmoney::{Cycle, ParseOptions};
use hardmoney::bulk::{self, Input, LoadMode, LoadOptions};
use hardmoney::db::{self, DbConfig, Namespace};

# async fn run(url: &str) -> Result<(), Box<dyn std::error::Error>> {
let ns = Namespace::new("book_demo")?;
let pool = db::connect(&DbConfig::new(url).namespace(ns)).await?;
db::migrate(&pool).await?; // schema-init

let cycle = Cycle::new(2026)?;
let source = &bulk::source::CANDIDATES;
let input = Input::Url(bulk::source::download_url(source, cycle));
let report = bulk::load(&pool, source, input, LoadOptions::new(cycle).mode(LoadMode::Replace)).await?;
println!("{}: {} rows loaded, {} replaced", report.table, report.rows_loaded, report.rows_replaced);

let path = std::path::Path::new("tests/fixtures/F24N_2011832.fec");
let id = bulk::filing_id_from_path(path).ok_or("no id in filename")?;
let bytes = std::fs::read(path)?;
let ingest = bulk::ingest_filing_bytes(&pool, id, &bytes, &ParseOptions::LENIENT).await?;
println!("{} Schedule E lines, {} skipped", ingest.schedule_e_lines, ingest.skipped.len());
# Ok(())
# }
```

`bulk::load` returns a `LoadReport` (`rows_loaded`, `rows_replaced`,
`dates_nulled`, `skipped_unchanged`, `load_id`); `ingest_filing_bytes`
returns an `IngestReport` with the skipped lines themselves, not just a
count. Both require the `bulk` feature and a Tokio runtime.
