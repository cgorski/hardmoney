# Reloading: Replace, Append, and `--if-changed`

The FEC re-publishes each cycle's bulk files roughly weekly, and the new
file isn't just the old one plus new rows. Rows get **changed** (an
amended report supersedes the original's line items) and rows get
**removed** (a filing is withdrawn, or a duplicate is cleaned up). A
loader that only ever inserts -- or even one that upserts -- can't
reproduce that: there's no way to delete a row you never saw go missing.

This is why `bulk-load` has a `--mode`, and why the default is `replace`.

## `--mode replace` (the default)

Replace means: inside one transaction, delete every existing row for this
source *and this cycle*, then `COPY` the new file in. If anything fails
partway, the transaction rolls back and the previous data is untouched.
If it succeeds, the table holds exactly what the FEC's current file
holds -- additions, changes, and deletions included.

You saw the first load of `candidates` in the [bulk ETL chapter](./bulk-etl.md#step-2-load-one-bulk-source).
Run the identical command again:

```bash
$ cargo run --quiet --bin hardmoney -- bulk-load --schema book_demo candidates --cycle 2026
```

```text
candidates: loaded 8557 rows for cycle 2026 (replace, replaced 8557 existing)
```

Same 8,557 rows in, 8,557 rows out. Other cycles in the same table are
never touched -- a 2024 load and a 2026 load of `candidates` live side by
side, and replacing 2026 leaves 2024 alone. (Both facts are pinned by
`replace_reloads_exactly_and_append_conflicts` in
`tests/postgres_integration.rs`, which loads a two-row file, then a
two-row file with one row swapped, and checks the dropped row is gone.)

### The `--yes` guard

Deleting a cycle's rows is exactly what you want for a weekly refresh,
and exactly what you don't want when you've typed the wrong source name
against a production `schedule_a`. So a replace that would delete more
than **1,000,000** rows refuses to start unless you pass `--yes`:

```text
$ hardmoney bulk-load schedule_a --cycle 2026
error: replacing schedule_a for cycle 2026 would delete 48120345 existing rows; pass --yes to confirm
```

*(Illustrative: the book's database doesn't hold a million rows of
anything. The message text is the crate's
`BulkError::ConfirmationRequired`.)* The check runs *before* the
download, so you don't wait for two gigabytes to arrive and then get
refused. Loads under the threshold -- everything in this book -- never
prompt.

## `--mode append`

Append is the simpler operation: `COPY` on top of whatever is already
there, with no delete. It exists for genuine partial loads -- you're assembling
a table from several files you know don't overlap. If any row already
exists, the primary key rejects it and the whole `COPY` rolls back:

```bash
$ cargo run --quiet --bin hardmoney -- bulk-load --schema book_demo candidates --cycle 2026 --mode append
```

```text
error: database error: error returned from database: duplicate key value violates unique constraint "candidates_pkey" at line 673
```

The command exits 1 and the table is unchanged. (Postgres's `at line
673` refers to the line of the staged `COPY` input where the first
duplicate appeared, not a line of the FEC's zip.)

## `--if-changed`: don't reload what hasn't changed

The FEC's bulk files are served from S3, which sends an `ETag` and a
`Last-Modified` header. Every full load records both in the `loads`
table (see below). `--if-changed` makes one `HEAD` request before
downloading anything, compares against the most recent *full* load of
this source and cycle in this namespace, and skips the load if they
match:

```bash
$ cargo run --quiet --bin hardmoney -- bulk-load --schema book_demo candidates --cycle 2026 --if-changed
```

```text
candidates: unchanged since last full load, skipped
```

The command exits 0. That makes a weekly cron job for a whole cycle one
line, with no wasted downloads and no wasted `DELETE`s:

```bash
hardmoney bulk-load-all --schema cycle_2026 --cycle 2026 --if-changed --yes
```

Some details worth knowing:

- "Unchanged" means the `ETag` matches, or -- if the server sent no
  `ETag` -- the `Last-Modified` matches. If the server sends neither, or
  there's no previous full load to compare against, the load proceeds.
- Only *full* loads count as a baseline. A `--limit` sample is recorded
  with `row_limit` set and is never treated as "current," so sampling
  during development can't cause `--if-changed` to skip the real load
  later.
- `--if-changed` only means something for downloads. With `--file` there
  is no remote to ask, and the load always runs.

## Every load is recorded

Whichever mode you use, every successful load -- full or sampled -- writes
one row to a `loads` table in the namespace:

```bash
$ psql "$DATABASE_URL" -c "SET search_path TO book_demo, public;" \
    -c "SELECT source, cycle, mode, row_count, row_limit, dates_nulled, source_etag IS NOT NULL AS has_etag FROM loads ORDER BY load_id;"
```

```text
               source                | cycle |  mode   | row_count | row_limit | dates_nulled | has_etag
-------------------------------------+-------+---------+-----------+-----------+--------------+----------
 candidates                          |  2026 | replace |      8557 |           |            0 | t
 candidates                          |  2026 | replace |      8557 |           |            0 | t
 disbursements                       |  2026 | replace |        50 |        50 |            0 | t
 committees                          |  2026 | replace |     20674 |           |            0 | t
 candidates                          |  2026 | replace |       100 |       100 |            0 | t
 committees                          |  2026 | replace |       100 |       100 |            0 | t
 candidate_committee_links           |  2026 | replace |       100 |       100 |            0 | t
 schedule_a                          |  2026 | replace |       100 |       100 |            0 | t
 committee_to_committee_transactions |  2026 | replace |       100 |       100 |            0 | t
 committee_to_candidate_transactions |  2026 | replace |       100 |       100 |            0 | t
 disbursements                       |  2026 | replace |       100 |       100 |            0 | t
 candidate_summary                   |  2026 | replace |       100 |       100 |            0 | t
 house_senate_summary                |  2026 | replace |       100 |       100 |            0 | t
 pac_party_summary                   |  2026 | replace |       100 |       100 |            0 | t
(14 rows)
```

That's the complete history of the `book_demo` namespace so far: the two
full `candidates` loads, the 50-row `disbursements` sample, the full
`committees` load, and the ten 100-row samples from `bulk-load-all`. The
failed `--mode append` and the skipped `--if-changed` run are correctly
absent -- one rolled back, the other never started.

The full column set is `load_id`, `source`, `cycle`, `loaded_at`, `mode`,
`row_count`, `row_limit`, `dates_nulled` (see [Dates](./dates.md)),
`source_url`, `source_etag`, `source_last_modified`, and
`hardmoney_version`. `schema-status` prints the latest row per source and
cycle, and `GET /schema` returns the same thing as JSON, so "what data is
in this namespace and when did it get there?" is always answerable
without opening `psql`.

## In library code

`--mode`, `--limit`, `--yes`, and `--if-changed` are the fields of
`hardmoney::bulk::LoadOptions`, and the result is a `LoadReport`:

```rust
use hardmoney::Cycle;
use hardmoney::bulk::{self, Input, LoadMode, LoadOptions};

# async fn run(url: &str) -> Result<(), Box<dyn std::error::Error>> {
# let pool = hardmoney::db::connect(&hardmoney::db::DbConfig::new(url)).await?;
let cycle = Cycle::new(2026)?;
let source = &bulk::source::CANDIDATES;
let input = Input::Url(bulk::source::download_url(source, cycle));

let options = LoadOptions::new(cycle)   // mode Replace, no limit, threshold 1,000,000
    .mode(LoadMode::Replace)
    .if_changed(true)
    .confirmed(true);                    // --yes

let report = bulk::load(&pool, source, input, options).await?;
if report.skipped_unchanged {
    println!("{}: unchanged, skipped", report.table);
} else {
    println!(
        "{}: {} rows loaded, {} replaced, {} dates nulled",
        report.table, report.rows_loaded, report.rows_replaced, report.dates_nulled
    );
}
# Ok(())
# }
```

`LoadOptions::confirm_threshold` is a public field if you want a
different limit than a million. A refused replace is
`BulkError::ConfirmationRequired { table, cycle, existing }`; a namespace
with pending migrations is `BulkError::SchemaOutOfDate { pending }`.
