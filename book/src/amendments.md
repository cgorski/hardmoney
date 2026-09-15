# Amendments: Which Version of a Report Is Current?

Committees re-file reports. A treasurer finds a mis-keyed contribution, a
refund posts late, an auditor asks a question -- and the committee files
the same report again, corrected. Each re-filing is a new `.fec` with a
new filing id, and the FEC keeps every version. A committee's third-quarter
report may exist as three filings; only the last one is what the committee
now says happened.

If you ingest filings with `bulk-load-filing` and query the `filings`
table naively, you will double- or triple-count everything in a report
that was amended. This chapter is about the columns hardmoney derives so
that you don't have to.

## How an amendment identifies itself

Two things in the file mark it, and they live in different places:

1. The **cover form type ends in `A`**: `F3XA`, `F3A`, `F3PA`, `F1A`,
   `F24A`. (`N` is a new report, `T` a termination report.) The parser
   exposes this as `filing.is_amendment`.
2. The **header's `report_id` is `FEC-<n>`**, where `n` is the filing id of
   the report being amended, and the header's `report_number` is the
   sequential amendment number (`1`, `2`, ...). The parser exposes these as
   `filing.amends_filing` (`Option<u64>`) and
   `filing.header.amendment_number()` (`Option<u32>`).

The rule that makes the whole thing tractable is in the FEC's spec and
enforced by its validator: **the header must name the *original* filing,
never a prior amendment**. The second amendment of report 1118027 says
`FEC-1118027`, not `FEC-<first amendment>`. So every version of a report
points at the same id, and that id is the chain's key.

Here is a real chain, as openFEC reports it for committee C00554709's 2016
pre-general (`12G`) report:

| filing id | form | header `report_id` | `report_number` | openFEC `amendment_version` |
|---|---|---|---|---|
| 1118027 | `F3N` | *(blank)* | *(blank)* | 0 |
| 1131084 | `F3A` | `FEC-1118027` | `1` | 1 |
| 1151343 | `F3A` | `FEC-1118027` | `2` | 2 |

## What the ingester derives

Migration `0003_amendment_chains.sql` adds these columns to `filings`.
The first group is copied straight from the filing at ingest time; the
second is computed by the **resolver** over the whole chain, in the same
transaction as the insert (so the columns are never observably stale):

| Column | Type | From | Meaning |
|---|---|---|---|
| `report_id` | `TEXT` | header | `FEC-<n>` as filed; NULL if blank |
| `amendment_number` | `INT` | header | `report_number` as filed; NULL if blank or not a number |
| `report_code` | `TEXT` | cover line | `12G`, `Q1`, `M9`, `YE`, ...; `24`/`48` on the notice forms (F24, F5), which call the field `report_type` |
| `coverage_from`, `coverage_through` | `DATE` | cover line | parsed from `YYYYMMDD`; NULL if blank or malformed (see [Dates](./dates.md) for the policy) |
| `amendment_version` | `INT` | resolver | position in the chain: 0 for the original, then 1, 2, ... |
| `amendment_chain` | `BIGINT[]` | resolver | every filing id in the chain up to and including this one, in version order |
| `most_recent` | `BOOLEAN` | resolver | true on the highest version only |
| `most_recent_filing_id` | `BIGINT` | resolver | the highest version's id |
| `previous_filing_id` | `BIGINT` | resolver | the version before this one; the original points at itself |
| `chain_unresolved` | `BOOLEAN` | resolver | see below |

These are openFEC's `/filings/` semantics, field for field (openFEC calls
the last three `most_recent`, `most_recent_file_number`,
`previous_file_number`; the REST API uses openFEC's names, the table uses
hardmoney's `*_filing_id` convention). For the chain above, the table
holds:

```text
 filing_id | amendment_version |       amendment_chain       | most_recent | most_recent_filing_id | previous_filing_id
-----------+-------------------+-----------------------------+-------------+-----------------------+--------------------
   1118027 |                 0 | {1118027}                   | f           |               1151343 |            1118027
   1131084 |                 1 | {1118027,1131084}           | f           |               1151343 |            1118027
   1151343 |                 2 | {1118027,1131084,1151343}   | t           |               1151343 |            1131084
```

### Ordering within a chain

Members are ordered: the original first, then by `amendment_number`, then
by `filing_id` ascending. Two consequences worth knowing:

* **Same amendment number twice** (it happens): the later filing id wins,
  because the FEC assigns ids chronologically.
* **A blank amendment number** on an amendment (the FEC's validator
  rejects this; the parser does not) sorts *before* the numbered ones, so
  a malformed re-filing can never silently supersede a well-formed one.
* An original that carries a `report_number` (`F3XN_2011831.fec` in the
  fixtures says `0`) is still version 0: the head of the chain sorts first
  by construction, not because its number is blank.

### Forms that never chain, forms that do

* **Form 99** (miscellaneous text) and **RFAI responses** (`FRQ`) never
  carry the `A` designator, so they are never amendments: each is a
  one-filing chain with `most_recent = true`. That holds even when an F99's
  header cites another filing's id (some do, to say which report the text
  is about).
* **Registrations** (F1, F1M, F2) and **24/48-hour notices** (F24, F6,
  F5) amend by exactly the same header rule and are treated the same way
  as periodic reports.

## `filings_current`

The migration also creates a view:

```sql
CREATE VIEW filings_current AS SELECT * FROM filings WHERE most_recent;
```

One row per report -- the version the committee currently stands behind.
Join `schedule_e_lines` to it instead of to `filings` and superseded
versions drop out:

```sql
SELECT c.committee_id, sum(e.expenditure_amt)
FROM schedule_e_lines e
JOIN filings_current c USING (filing_id)
GROUP BY 1;
```

Like every hardmoney table, the view is unqualified and binds to the
namespace it was created in (see [Namespaces](./namespaces.md)).

## Order of arrival does not matter

You will not always see the original first. A nightly job that pulls
"filings received today" sees the amendment on Tuesday and, if it is
back-filling, the original on Thursday. The resolver is re-run for the
affected chain after **every** ingest, so the end state is the same
whichever order the files arrive in:

1. Ingest `1151343` (amendment 2) alone. Its original is not in the
   table, so it stands alone: `amendment_chain = {1151343}`,
   `most_recent = true`, and **`chain_unresolved = true`**.
2. Ingest `1131084` (amendment 1). Same: alone, flagged.
3. Ingest `1118027` (the original). Both amendments are pulled into its
   chain and the table now reads exactly as above, flags cleared.

Re-ingesting a filing id (say, after a parser upgrade) recomputes its
chain too -- and if the re-ingest changed which chain it belongs to (a
corrected header), the chain it *left* is recomputed as well, so nothing
is left claiming to be `most_recent` when it no longer is.

## `chain_unresolved`

An amendment that hardmoney could not attach to an original stands alone,
exactly like step 1 above, with `chain_unresolved = true`. That happens
when:

* its header has no usable `FEC-<n>` (blank, or not of that form);
* the `n` it names is not in the `filings` table (not ingested yet, or
  ever -- `F3A_2011812.fec` in the fixtures amends `FEC-1997089`, which is
  not among the fixtures);
* the `n` it names *is* in the table but is itself an amendment (a header
  that violates the "name the original" rule).

Such a row is `most_recent` -- it is the newest version we know of -- but
its `amendment_version` is 0 and its chain is just itself, which would be
indistinguishable from a real original without the flag. So query for it:

```sql
SELECT filing_id, form_type, committee_id, report_id
FROM filings WHERE chain_unresolved;
```

and ingest the missing originals (`bulk-load-filing <n>` downloads by
id). The flag clears itself when they land.

## Batch loads: `--no-resolve` and `resolve_all_amendment_chains`

Resolving after every insert is one indexed `UPDATE` over the chain -- a
handful of rows -- so it is the right default. For a scripted load of many
thousands of filings you can skip it and resolve the whole table once at
the end:

```bash
for f in filings/*.fec; do
  hardmoney bulk-load-filing --schema nightly --no-resolve "$f"
done
```

then, from Rust:

```rust
# async fn run(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
let rows = hardmoney::db::resolve_all_amendment_chains(pool).await?;
println!("recomputed {rows} filings");
# Ok(())
# }
```

`resolve_all_amendment_chains` is a single set-based statement (a CTE
with `row_number()`, `array_agg()`, `lag()` and `last_value()` windowed
`PARTITION BY original`), not a loop; it is also what to run after
applying migration `0003` to a namespace that already held filings, whose
rows have NULL in every derived column until then. The per-chain
`resolve_amendment_chain(pool, original_id)` is the same statement scoped
to one key. The library equivalents of the flag are
`bulk::ingest::ChainResolution::{Resolve, Defer}` on
`ingest_filing_bytes_with` / `ingest_filing_with`.

## Over the REST API

`GET /filings/{id}` carries the chain with openFEC's field names, so a
client written against `api.open.fec.gov/v1/filings/` reads it unchanged:

```bash
$ curl -s -H 'X-Api-Key: demo-key' "http://127.0.0.1:18090/filings/1151343"
```

```json
{
  "filing_id": 1151343,
  "form_type": "F3A",
  "committee_id": "C00554709",
  "is_amendment": true,
  "amends_filing_id": 1118027,
  "amendment_indicator": "A",
  "amendment_version": 2,
  "amendment_chain": [1118027, 1131084, 1151343],
  "most_recent": true,
  "most_recent_file_number": 1151343,
  "previous_file_number": 1131084,
  "chain_unresolved": false,
  "report_type": "12G",
  "coverage_start_date": "2016-10-01",
  "coverage_end_date": "2016-10-19",
  "fec_url": "https://docquery.fec.gov/dcdev/posted/1151343.fec",
  "header": { "...": "..." },
  "summary": { "...": "..." },
  "skipped_lines": 0,
  "ingested_at": "2026-09-15T12:00:00Z"
}
```

`amendment_indicator` is `"A"` or `"N"` (openFEC's code). The original,
`GET /filings/1118027`, answers `"amendment_version": 0`,
`"most_recent": false`, `"most_recent_file_number": 1151343`, and --
matching openFEC -- `"previous_file_number": 1118027`, itself.

The new list route, `GET /filings`, is how you ask "what is current for
this committee?":

```bash
$ curl -s -H 'X-Api-Key: demo-key' \
    "http://127.0.0.1:18090/filings?committee_id=C00554709&most_recent=true"
```

returns just `1151343`. Filters, all optional:

| Parameter | Matches |
|---|---|
| `committee_id` | the cover line's filer id, exactly |
| `most_recent` | `true`: the latest version of each report (the `filings_current` view); `false`: superseded versions only. Unresolved rows (NULL) match neither. |
| `form_type` | a base form -- `F3X` matches `F3XN`, `F3XA`, `F3XT` -- or an exact as-filed token such as `F3XA`; case-insensitive |
| `limit`, `offset` | as on every list route: default 50, max 500, out-of-range is a 400 |

Results are newest filing id first.
