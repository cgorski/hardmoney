# Loading a Full Election Cycle into Postgres (and Keeping It Fresh)

**Who this is for:** an academic, a newsroom data team, or anyone who
needs the FEC's bulk data for a whole election cycle in a database they
control -- reproducibly, with a record of what was loaded when, refreshed
on a schedule, and archivable.

**What you'll have at the end:** a namespace holding all ten bulk sources
for cycle 2026, a one-line weekly refresh that downloads nothing when the
FEC hasn't changed anything, a `loads` table that answers "where did this
row come from and when", a `pg_dump` archive of the namespace, and a
cross-schema query that diffs two snapshots.

Everything here was run against a local Postgres 18. Aurora PostgreSQL 18
notes are at the end.

## Namespaces are snapshots

hardmoney keeps every table in a Postgres *schema* you name with
`--schema` -- a **namespace**. Nothing else changes: same tables, same
commands, same API. That single mechanism is what makes a research
database manageable, because you can have as many independent copies as
you have questions:

- `tut_cycle_2026` -- the live copy, refreshed weekly;
- `tut_cycle_2026_w37` -- a frozen snapshot from week 37, for a paper
  whose numbers must not move;
- `tut_cycle_2024` -- the previous cycle, loaded once and never touched.

The tutorial namespaces are prefixed `tut_` so they can be dropped at the
end; in your own database you'd call them `cycle_2026` and so on. Names
are lowercase letters, digits, and underscores. Full details are in
[Namespaces](./namespaces.md).

```bash
export DATABASE_URL="postgres://chris.gorski@localhost:5432/hardmoney_v1_test"
hardmoney schema-init --schema tut_cycle_2026
```

```text
namespace 'tut_cycle_2026': schema at version 2 (2 migration(s) applied now)
```

## What a full cycle is, in bytes

`bulk-load-all` loads all ten of the FEC's pipe-delimited bulk files for
one cycle. Before running it, know what you're downloading. These are the
`Content-Length`s `fec.gov` reported for cycle 2026 on 2026-09-14:

| Source | FEC file | Size | What it is |
|---|---|---|---|
| `candidates` | `cn26.zip` | 307 KB | every registered candidate |
| `committees` | `cm26.zip` | 867 KB | every registered committee |
| `candidate_committee_links` | `ccl26.zip` | 88 KB | which committee belongs to which candidate |
| `schedule_a` | `indiv26.zip` | **2.19 GB** | every itemized individual contribution |
| `committee_to_committee_transactions` | `oth26.zip` | **213 MB** | transfers between committees |
| `committee_to_candidate_transactions` | `pas226.zip` | 8.1 MB | committee contributions and expenditures to candidates |
| `disbursements` | `oppexp26.zip` | 45 MB | itemized operating expenditures |
| `candidate_summary` | `weball26.zip` | 193 KB | one financial-summary row per candidate |
| `house_senate_summary` | `webl26.zip` | 146 KB | the same, current House/Senate campaigns only |
| `pac_party_summary` | `webk26.zip` | 462 KB | one financial-summary row per PAC/party committee |

Two of the ten are large, and `indiv` in particular will grow for as long
as the cycle runs. The full command is:

```bash
hardmoney bulk-load-all --schema tut_cycle_2026 --cycle 2026
```

That is a real multi-gigabyte download and, on the loading side, tens of
millions of rows through Postgres `COPY`. Plan for it to take a while the
first time, and run it from a machine with the disk to hold it. For the
purposes of this book -- which has to be re-runnable -- **the command was
run with `--limit 1000`**, which reads the first thousand rows of each
file and then stops the download:

```bash
hardmoney bulk-load-all --schema tut_cycle_2026 --cycle 2026 --limit 1000
```

```text
loading candidates from https://www.fec.gov/files/bulk-downloads/2026/cn26.zip ...
candidates: loaded 1000 rows for cycle 2026 (replace)
loading committees from https://www.fec.gov/files/bulk-downloads/2026/cm26.zip ...
committees: loaded 1000 rows for cycle 2026 (replace)
loading candidate_committee_links from https://www.fec.gov/files/bulk-downloads/2026/ccl26.zip ...
candidate_committee_links: loaded 1000 rows for cycle 2026 (replace)
loading schedule_a from https://www.fec.gov/files/bulk-downloads/2026/indiv26.zip ...
schedule_a: loaded 1000 rows for cycle 2026 (replace)
loading committee_to_committee_transactions from https://www.fec.gov/files/bulk-downloads/2026/oth26.zip ...
committee_to_committee_transactions: loaded 1000 rows for cycle 2026 (replace)
loading committee_to_candidate_transactions from https://www.fec.gov/files/bulk-downloads/2026/pas226.zip ...
committee_to_candidate_transactions: loaded 1000 rows for cycle 2026 (replace)
loading disbursements from https://www.fec.gov/files/bulk-downloads/2026/oppexp26.zip ...
disbursements: loaded 1000 rows for cycle 2026 (replace)
loading candidate_summary from https://www.fec.gov/files/bulk-downloads/2026/weball26.zip ...
candidate_summary: loaded 1000 rows for cycle 2026 (replace)
loading house_senate_summary from https://www.fec.gov/files/bulk-downloads/2026/webl26.zip ...
house_senate_summary: loaded 1000 rows for cycle 2026 (replace)
loading pac_party_summary from https://www.fec.gov/files/bulk-downloads/2026/webk26.zip ...
pac_party_summary: loaded 1000 rows for cycle 2026 (replace)
```

Nine seconds for the first thousand rows of all ten files, 2.19 GB one
included. Every one of those loads is recorded as a *sample*, which
matters for the refresh logic below. Without `--limit`, the output is
the same ten pairs of lines with real row counts.

By default `bulk-load-all` keeps going if one source fails and exits
non-zero at the end naming the ones that did; `--fail-fast` stops at the
first failure instead.

To make the rest of this tutorial meaningful on real data, the three small
registry sources were then loaded in full (each is under a megabyte):

```bash
hardmoney bulk-load --schema tut_cycle_2026 candidates --cycle 2026
hardmoney bulk-load --schema tut_cycle_2026 committees --cycle 2026
hardmoney bulk-load --schema tut_cycle_2026 candidate_committee_links --cycle 2026
```

```text
candidates: loaded 8557 rows for cycle 2026 (replace, replaced 1000 existing)
committees: loaded 20674 rows for cycle 2026 (replace, replaced 1000 existing)
candidate_committee_links: loaded 8085 rows for cycle 2026 (replace, replaced 1000 existing)
```

Note `replaced 1000 existing`: each full load deleted the sample first.
That's the default mode, and the next section is about why.

## The weekly refresh: replace, not upsert

The FEC re-publishes each cycle's files roughly weekly -- in the `loads`
table further down, the four transaction files carry a `Last-Modified`
of 2026-09-13 and the registries and summaries one from early on
2026-09-14. The new file is **not** the old file plus new rows. Amended reports replace the
original's line items; withdrawn filings and de-duplicated rows
disappear. An upsert -- insert new keys, update existing ones -- cannot
express a deletion: it has no way to know a row is gone, because the row
simply isn't in the file anymore.

`bulk-load`'s default `--mode replace` handles this the only way that
actually reproduces the FEC's file: inside one transaction, delete every
row for this source *and this cycle*, `COPY` the new file in, commit. If
anything fails, the transaction rolls back and last week's data is
untouched. Other cycles in the same table are never affected. The
mechanics are in [Reloading](./reloading.md).

Two flags turn that into a refresh you can put in cron:

- **`--if-changed`** sends one `HEAD` request per source, compares the
  `ETag` (or `Last-Modified`) with the most recent *full* load of that
  source and cycle in this namespace, and skips the download entirely if
  they match.
- **`--yes`** pre-confirms a replace that would delete more than a million
  rows -- which a full `schedule_a` always does -- so the job doesn't
  stop and ask.

Here it is on one source, immediately after the full load above:

```bash
hardmoney bulk-load --schema tut_cycle_2026 candidates --cycle 2026 --mode replace --if-changed --yes
```

```text
candidates: unchanged since last full load, skipped
```

Four tenths of a second, exit status 0, nothing downloaded, nothing
deleted. The whole-cycle version -- the line to schedule -- is:

```bash
hardmoney bulk-load-all --schema tut_cycle_2026 --cycle 2026 --mode replace --if-changed --yes
```

*(Not run for this book: in the tutorial namespace seven of the ten
sources hold only samples, and a sample is never a baseline for
`--if-changed`, so that command would correctly proceed to download all
2.4 GB. That is the right behaviour -- a sample must never suppress a
real load -- but it's worth knowing before you paste the line into a
namespace you've only ever sampled into.)*

## `loads`: every row's provenance

Every load, full or sampled, writes one row to a `loads` table in the
namespace. `schema-status` prints the latest row per source and cycle:

```bash
hardmoney schema-status --schema tut_cycle_2026
```

```text
namespace:  tut_cycle_2026
applied:    [1, 2]
pending:    []
loads (latest per source/cycle):
  candidate_committee_links                cycle 2026          8085 rows  replace 2026-09-15 02:43:08Z
  candidate_summary                        cycle 2026          1000 rows  replace 2026-09-15 02:42:52Z (sample, limit 1000)
  candidates                               cycle 2026          8557 rows  replace 2026-09-15 02:43:05Z
  committee_to_candidate_transactions      cycle 2026          1000 rows  replace 2026-09-15 02:42:51Z (sample, limit 1000)
  committee_to_committee_transactions      cycle 2026          1000 rows  replace 2026-09-15 02:42:50Z (sample, limit 1000)
  committees                               cycle 2026         20674 rows  replace 2026-09-15 02:43:07Z
  disbursements                            cycle 2026          1000 rows  replace 2026-09-15 02:42:52Z (sample, limit 1000)
  house_senate_summary                     cycle 2026          1000 rows  replace 2026-09-15 02:42:53Z (sample, limit 1000)
  pac_party_summary                        cycle 2026          1000 rows  replace 2026-09-15 02:42:54Z (sample, limit 1000)
  schedule_a                               cycle 2026          1000 rows  replace 2026-09-15 02:42:49Z (sample, limit 1000)
```

The full table has the history, not just the latest, and the columns
`--if-changed` uses:

```bash
psql "$DATABASE_URL" -c "SET search_path TO tut_cycle_2026, public;" \
  -c "SELECT load_id, source, row_count, row_limit, dates_nulled, source_etag, source_last_modified
      FROM loads ORDER BY load_id;"
```

```text
 load_id |               source                | row_count | row_limit | dates_nulled |             source_etag              |  source_last_modified
---------+-------------------------------------+-----------+-----------+--------------+--------------------------------------+------------------------
       1 | candidates                          |      1000 |      1000 |            0 | 07fc62b79883e5b8341f37d60cc7f5d5     | 2026-09-14 01:41:47-04
       2 | committees                          |      1000 |      1000 |            0 | 9455166b75a0e35283a7b52165c02fca     | 2026-09-14 01:41:52-04
       3 | candidate_committee_links           |      1000 |      1000 |            0 | 08c741fcc06db92bbfee161ef01724f3     | 2026-09-14 01:41:56-04
       4 | schedule_a                          |      1000 |      1000 |            0 | 3aa48208d54c6998ac0a62b899200cf0-261 | 2026-09-13 11:59:54-04
       5 | committee_to_committee_transactions |      1000 |      1000 |            0 | 53d3104bed587d60d532033287bf81e7-26  | 2026-09-13 12:01:25-04
       6 | committee_to_candidate_transactions |      1000 |      1000 |            0 | 209b0109b25a1a9e3205c1887ae71c88     | 2026-09-13 12:00:18-04
       7 | disbursements                       |      1000 |      1000 |            0 | 3831fa738a497cc9fe77d0d99a2e87f6-6   | 2026-09-13 12:01:39-04
       8 | candidate_summary                   |      1000 |      1000 |            0 | 581f8487c87127b8b50220b00eebc994     | 2026-09-14 01:41:34-04
       9 | house_senate_summary                |      1000 |      1000 |            0 | 241d0733933845722d45f8e18ac36a40     | 2026-09-14 01:41:27-04
      10 | pac_party_summary                   |      1000 |      1000 |            0 | 17ca48e8e83f7ab7e8108c30819bcbf4     | 2026-09-14 01:41:43-04
      11 | candidates                          |      8557 |           |            0 | 07fc62b79883e5b8341f37d60cc7f5d5     | 2026-09-14 01:41:47-04
      12 | committees                          |     20674 |           |            0 | 9455166b75a0e35283a7b52165c02fca     | 2026-09-14 01:41:52-04
      13 | candidate_committee_links           |      8085 |           |            0 | 08c741fcc06db92bbfee161ef01724f3     | 2026-09-14 01:41:56-04
(13 rows)
```

Read it as a lab notebook. `row_limit IS NULL` marks a full load; the
`--if-changed` run that was skipped left no row at all, correctly,
because nothing happened.
`source_etag` is the S3 ETag of the exact file that was loaded -- if the
FEC ever asks "which version of `indiv26.zip` did you analyse?", that is
the answer, and the `-261` suffix on the `indiv` ETag is S3's multipart
marker for a file uploaded in 261 parts. `dates_nulled` is explained
next. The remaining columns (`loaded_at`, `mode`, `source_url`,
`hardmoney_version`) are in [Reloading](./reloading.md#every-load-is-recorded),
and the REST API's `GET /schema` returns the latest rows as JSON.

For a citation in a paper, the pair (`source_etag`, `hardmoney_version`)
plus the migration versions from `schema-status` identifies the dataset
and the software that built it.

## Dates: two FEC formats, and the `*_date` twin columns

The FEC writes dates as text and uses **two different formats across its
own files**: `MMDDYYYY` in `indiv`, `oth`, and `pas2`, and `MM/DD/YYYY`
in `oppexp` and the three summary files. Neither sorts correctly as text.
hardmoney keeps the raw string exactly as shipped in the `*_dt` column
and adds a parsed `DATE` twin with a `*_date` suffix:

```bash
psql "$DATABASE_URL" -c "SET search_path TO tut_cycle_2026, public;" \
  -c "SELECT 'schedule_a' AS tbl, transaction_dt AS raw, transaction_date AS parsed FROM schedule_a ORDER BY sub_id LIMIT 2;" \
  -c "SELECT 'disbursements' AS tbl, transaction_dt AS raw, transaction_date AS parsed FROM disbursements ORDER BY sub_id LIMIT 2;" \
  -c "SELECT 'pac_party_summary' AS tbl, cvg_end_dt AS raw, cvg_end_date AS parsed FROM pac_party_summary ORDER BY cmte_id LIMIT 2;"
```

```text
    tbl     |   raw    |   parsed
------------+----------+------------
 schedule_a | 10222025 | 2025-10-22
 schedule_a | 10222025 | 2025-10-22
(2 rows)

      tbl      |    raw     |   parsed
---------------+------------+------------
 disbursements | 07/21/2025 | 2025-07-21
 disbursements | 07/25/2025 | 2025-07-25
(2 rows)

        tbl        |    raw     |   parsed
-------------------+------------+------------
 pac_party_summary | 08/31/2026 | 2026-08-31
 pac_party_summary | 07/31/2026 | 2026-07-31
(2 rows)
```

Use the `*_date` column for sorting, filtering, and arithmetic; it is
indexed on every transaction table. A raw value that is blank becomes
`NULL` silently (blank dates are normal); a raw value that is *present
but not a calendar date* also becomes `NULL` and is **counted** in the
`dates_nulled` column of `loads`, so a file with a systematic date problem
is visible in `schema-status` rather than producing a quietly empty
column. Every load above shows `0`. The full story, including what
`13/45/2026` does, is in [Dates](./dates.md).

## Archiving a namespace with `pg_dump`

Because a namespace is a Postgres schema, archiving one is standard
`pg_dump` with `-n`:

```bash
pg_dump "$DATABASE_URL" -n tut_cycle_2026 -Fc -f tut_cycle_2026.dump
ls -lh tut_cycle_2026.dump
pg_restore -l tut_cycle_2026.dump | grep 'TABLE DATA'
```

```text
-rw-r--r--  1 chris.gorski  wheel   1.6M Sep 14 22:43 tut_cycle_2026.dump
4394; 0 6905950 TABLE DATA tut_cycle_2026 _sqlx_migrations chris.gorski
4397; 0 6905983 TABLE DATA tut_cycle_2026 candidate_committee_links chris.gorski
4402; 0 6906041 TABLE DATA tut_cycle_2026 candidate_summary chris.gorski
4395; 0 6905964 TABLE DATA tut_cycle_2026 candidates chris.gorski
4400; 0 6906016 TABLE DATA tut_cycle_2026 committee_to_candidate_transactions chris.gorski
4399; 0 6906005 TABLE DATA tut_cycle_2026 committee_to_committee_transactions chris.gorski
4396; 0 6905973 TABLE DATA tut_cycle_2026 committees chris.gorski
4401; 0 6906030 TABLE DATA tut_cycle_2026 disbursements chris.gorski
4405; 0 6906068 TABLE DATA tut_cycle_2026 filings chris.gorski
4403; 0 6906050 TABLE DATA tut_cycle_2026 house_senate_summary chris.gorski
4409; 0 6906105 TABLE DATA tut_cycle_2026 loads chris.gorski
4404; 0 6906059 TABLE DATA tut_cycle_2026 pac_party_summary chris.gorski
4398; 0 6905994 TABLE DATA tut_cycle_2026 schedule_a chris.gorski
4407; 0 6906085 TABLE DATA tut_cycle_2026 schedule_e_lines chris.gorski
```

1.6 MB for the mostly-sampled tutorial namespace; a full cycle will be
on the order of the compressed downloads themselves, i.e. gigabytes (an
estimate -- not measured for this book). The archive carries `loads` and `_sqlx_migrations` with it, so
a restored copy knows its own provenance and schema version. Restoring
into a *different* database was tested while writing this, and two things
need doing first on the target:

1. `CREATE EXTENSION IF NOT EXISTS pg_trgm;` -- the trigram indexes in
   the dump reference it, and a schema-only dump does not carry the
   extension. Without it `pg_restore` skips those six indexes with
   errors, and because `_sqlx_migrations` already says migration 2 was
   applied, `schema-init` will not recreate them.
2. If you want `/independent-expenditures` in the restored copy, run
   `hardmoney bulk-restore-dump schedule_e` there and then
   `hardmoney schema-init --schema tut_cycle_2026`. The dump contains the
   `independent_expenditures` view definition, which references the
   shared `disclosure.fec_fitem_sched_e` table; on a database without it,
   `pg_restore` reports that one view as an error and restores everything
   else. `schema-init` creates the view once the table exists.

With `pg_trgm` created first, the restore reported exactly two ignored
errors (the view and its `ALTER ... OWNER`), all fourteen tables and their
data arrived, and `schema-status` against the restored namespace showed
`applied: [1, 2]`.

## Comparing two snapshots side by side

A frozen snapshot is just a second namespace you stop refreshing. Create
one and load the same registry files into it:

```bash
hardmoney schema-init --schema tut_cycle_2026_w37
hardmoney bulk-load --schema tut_cycle_2026_w37 candidates --cycle 2026
hardmoney bulk-load --schema tut_cycle_2026_w37 committees --cycle 2026
```

```text
namespace 'tut_cycle_2026_w37': schema at version 2 (2 migration(s) applied now)
candidates: loaded 8557 rows for cycle 2026 (replace)
committees: loaded 20674 rows for cycle 2026 (replace)
```

Since every namespace is a schema in the same database, comparing them is
a schema-qualified join -- no export, no second connection:

```bash
psql "$DATABASE_URL" -c "
SELECT 'only in w37' AS which, count(*)
  FROM tut_cycle_2026_w37.candidates a
  LEFT JOIN tut_cycle_2026.candidates b USING (cand_id, cycle)
  WHERE b.cand_id IS NULL
UNION ALL
SELECT 'only in current', count(*)
  FROM tut_cycle_2026.candidates b
  LEFT JOIN tut_cycle_2026_w37.candidates a USING (cand_id, cycle)
  WHERE a.cand_id IS NULL
UNION ALL
SELECT 'in both, changed', count(*)
  FROM tut_cycle_2026.candidates b
  JOIN tut_cycle_2026_w37.candidates a USING (cand_id, cycle)
  WHERE row(a.*) IS DISTINCT FROM row(b.*)
UNION ALL
SELECT 'in both, identical', count(*)
  FROM tut_cycle_2026.candidates b
  JOIN tut_cycle_2026_w37.candidates a USING (cand_id, cycle)
  WHERE row(a.*) IS NOT DISTINCT FROM row(b.*);"
```

```text
       which        | count
--------------------+-------
 in both, identical |  8557
 only in current    |     0
 only in w37        |     0
 in both, changed   |     0
(4 rows)
```

Zero differences -- which is correct, and the `loads` table says why: both
namespaces loaded the file with ETag `07fc62b79883e5b8341f37d60cc7f5d5`,
thirty-four seconds apart. Run the same query after next Sunday's refresh
of `tut_cycle_2026` and the first three rows are your changelog: new
filers, withdrawn candidates, and changed records (a new principal
committee, a party switch). Swap in `committees` or -- on a full load --
`schedule_a` keyed on `sub_id` for the same diff on any table.

`row(a.*) IS DISTINCT FROM row(b.*)` compares every column at once and
treats two `NULL`s as equal, which is what you want for "did anything
change".

## Aurora PostgreSQL 18

hardmoney's schema is plain Postgres and runs unchanged on Aurora
PostgreSQL 18. Two things to know:

- **`pg_trgm` is a trusted extension** on Aurora (and on community
  Postgres 13+), so `schema-init` can create it as the database owner
  without superuser rights. If your host has locked it down anyway, the
  migration raises a `WARNING`, skips the six trigram indexes, and
  everything still works -- searches are just slower.
- **Parallel query is off by default on Aurora**, so a `name ILIKE
  '%term%'` search over a full `schedule_a` cannot fall back on a parallel
  sequential scan the way it can on a laptop. The trigram GIN indexes are
  what make the API's substring searches fast there; make sure the
  `WARNING` above didn't fire.

`bulk-restore-dump` (the FEC's own `pg_dump` archives) runs `pg_restore`
on the machine running hardmoney against whatever `DATABASE_URL` points
at, so the tool needs to be installed there, not on the server.

## Clean up

```bash
hardmoney schema-drop tut_cycle_2026 --yes
hardmoney schema-drop tut_cycle_2026_w37 --yes
```

```text
dropped namespace 'tut_cycle_2026'
dropped namespace 'tut_cycle_2026_w37'
```

## Where to go next

- [Reloading](./reloading.md) has the `--mode append` case, the exact
  `--yes` refusal message, and the library equivalents (`LoadOptions`).
- [Tracking Independent Expenditures](./tutorial-independent-expenditures.md)
  adds the one dataset `bulk-load-all` doesn't cover -- the FEC's
  Schedule E `pg_dump` -- and shows how it relates to the
  `committee_to_candidate_transactions` table you just loaded.
- [The REST API](./rest-api.md) serves any namespace with
  `hardmoney serve --schema <name>`.
