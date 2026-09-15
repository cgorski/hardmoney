# Dates: two FEC formats, twin columns

The FEC ships every date in its bulk files as plain text, in two
different formats depending on which file you're looking at:

| Format | Example | Files |
|---|---|---|
| `MMDDYYYY` | `12202025` | `indiv` (`schedule_a`), `oth` (`committee_to_committee_transactions`), `pas2` (`committee_to_candidate_transactions`) |
| `MM/DD/YYYY` | `02/18/2026` | `oppexp` (`disbursements`), and the `cvg_end_dt` column of `weball` / `webl` / `webk` (the three summary tables) |

Neither one sorts correctly as text. Ordering `12202025`, `11182025`,
`09302025`, `12312024` as strings puts `12312024` (December 2024) first,
because `12` beats `11` and `09`: the sort is month-major, and the year
is the least significant thing about it. An `ORDER BY
transaction_dt DESC` on the raw text would return "newest first" with the
newest row nowhere near the top.

## Keep the raw string, add a parsed twin

Every raw `*_dt` column is still a `TEXT` column holding the FEC's exact
string, untouched. Alongside it, the loader adds a parsed `DATE` column
with the same name and a `_date` suffix:

| Raw (`TEXT`, verbatim) | Parsed (`DATE`) | Tables |
|---|---|---|
| `transaction_dt` | `transaction_date` | `schedule_a`, `committee_to_committee_transactions`, `committee_to_candidate_transactions`, `disbursements` |
| `cvg_end_dt` | `cvg_end_date` | `candidate_summary`, `house_senate_summary`, `pac_party_summary` |

Both formats are recognized, so the same loader code handles every file.
Here's the pair side by side in three tables from the `book_demo`
namespace, each loaded from a different FEC file:

```bash
$ psql "$DATABASE_URL" -c "SET search_path TO book_demo, public;" \
    -c "SELECT 'schedule_a' AS tbl, transaction_dt AS raw, transaction_date AS parsed FROM schedule_a ORDER BY sub_id LIMIT 2;" \
    -c "SELECT 'disbursements' AS tbl, transaction_dt AS raw, transaction_date AS parsed FROM disbursements ORDER BY sub_id LIMIT 2;" \
    -c "SELECT 'candidate_summary' AS tbl, cvg_end_dt AS raw, cvg_end_date AS parsed FROM candidate_summary ORDER BY cand_id LIMIT 2;"
```

```text
    tbl     |   raw    |   parsed
------------+----------+------------
 schedule_a | 10222025 | 2025-10-22
 schedule_a | 10222025 | 2025-10-22
(2 rows)

      tbl      |    raw     |   parsed
---------------+------------+------------
 disbursements | 11/28/2025 | 2025-11-28
 disbursements | 02/18/2026 | 2026-02-18
(2 rows)

        tbl        |    raw     |   parsed
-------------------+------------+------------
 candidate_summary | 07/22/2026 | 2026-07-22
 candidate_summary | 04/15/2025 | 2025-04-15
(2 rows)
```

You can always see exactly what the FEC shipped, and you can always sort,
filter, and do arithmetic on a real date. The parsed column is indexed
(`transaction_date DESC NULLS LAST`) on every transaction table, and the
[REST API](./rest-api.md) filters (`min_date`/`max_date`) and orders on
it while returning both columns.

## What happens to a bad date

The parsed column is only ever a real calendar date or `NULL`. A date is
never invented.

A blank raw value becomes `NULL`. Blank dates are normal in FEC data (a
summary row with no coverage end, a memo line), so they aren't counted
as errors.

A non-blank value that doesn't parse (all zeros, a year of `0000`,
`13/45/2026`, the wrong number of digits) also becomes `NULL`, and is
counted. The count is reported at the end of the load (`..., 3
unparseable date(s) set NULL`), returned as `LoadReport::dates_nulled`,
and stored in the `loads` table's `dates_nulled` column, so a file with
a systematic date problem is visible in `schema-status` and `GET
/schema` rather than silently producing a column full of nulls.

In the book's data, 6 of the 20,000 `committee_to_candidate_transactions`
rows have a `NULL` parsed date, and all 6 have a blank raw value, so the
load recorded `dates_nulled = 0`:

```bash
$ psql "$DATABASE_URL" -c "SELECT count(*) AS total, count(*) FILTER (WHERE transaction_date IS NULL) AS null_dates FROM committee_to_candidate_transactions;"
```

```text
 total | null_dates
-------+------------
 20000 |          6
```

A malformed date on one row never fails a batch: the row loads with
`NULL` in the parsed column, the raw text is preserved, and the count
goes up by one.

## The function behind it

The conversion is `hardmoney::bulk::loader::normalize_bulk_date`, which
returns the ISO `YYYY-MM-DD` string the `COPY` stream needs, or `None`:

```rust
use hardmoney::bulk::loader::normalize_bulk_date;

assert_eq!(normalize_bulk_date("12202025").as_deref(),   Some("2025-12-20")); // MMDDYYYY
assert_eq!(normalize_bulk_date("02/18/2026").as_deref(), Some("2026-02-18")); // MM/DD/YYYY
assert_eq!(normalize_bulk_date(""), None);           // blank: NULL, not counted
assert_eq!(normalize_bulk_date("00000000"), None);   // zero: NULL, counted
assert_eq!(normalize_bulk_date("13/45/2026"), None); // not a calendar date: NULL, counted
```

This is deliberately a separate function from `hardmoney::parse_fec_date`
(covered in [Tables and typed views](./typed-views.md#real-dates-not-string-shaped-placeholders)),
because the two live in different worlds: `parse_fec_date` handles the
`YYYYMMDD` format used inside `.fec` filings and returns a
`chrono::NaiveDate`; `normalize_bulk_date` handles the two formats used
in the FEC's bulk files and returns text for `COPY`. A `.fec` filing
and a bulk file describing the same transaction will write its date
three different ways.

## Where the twin columns come from

Which columns get a twin is declared per source in
`hardmoney::bulk::source` (`BulkSource::date_columns`, a list of
`DateColumn { raw, parsed }` pairs) and the migration creates the
matching `DATE` columns. A unit test checks that every declared raw
column really exists in its source's column list and that every parsed
name ends in `_date`, so the two can't drift apart.
