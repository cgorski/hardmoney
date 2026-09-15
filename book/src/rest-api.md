# The REST API

Once you have a Postgres database loaded (via the previous chapters), you
can serve it over HTTP instead of requiring every consumer to write SQL
directly. This chapter runs the server and walks through every route with
real request/response pairs, all captured against a live local server
during the writing of this book, serving the `book_demo` namespace built
up in the bulk-ETL chapters.

## Starting the server

```bash
$ export DATABASE_URL="postgres://chris.gorski@localhost:5432/hardmoney_v1_test"
$ cargo run --quiet --bin hardmoney -- serve --schema book_demo --bind 127.0.0.1:18090 --api-key demo-key
```

```text
2026-09-15T02:07:24.564828Z  INFO hardmoney::api: hardmoney API listening bind=127.0.0.1:18090 cors="permissive" api_key=true
```

(One housekeeping step first: the `bulk-load-all --limit 100` smoke test
in the bulk-ETL chapter replaced the full `candidates` and `committees`
loads in `book_demo` with 100-row samples (that's what `--mode replace`
means), so both were reloaded in full with
`bulk-load --schema book_demo candidates --cycle 2026` and the same for
`committees` before starting the server.)

`--bind` defaults to `0.0.0.0:8080` if omitted. `--schema` picks which
[namespace](./namespaces.md) to serve; leave it off for `public`. Like
every database command, `serve` refuses to start if the namespace has
pending migrations, and on startup it (re)creates the
`independent_expenditures` view if the FEC's Schedule E dump has been
restored.

`--api-key` is optional, and you'd normally leave it off on your laptop.
This chapter turns it on so you can see what it does, and the
[next chapter](./api-hardening.md) covers it and the other hardening
flags in full. Every request below sends the key as an `X-Api-Key`
header. Leave the server running in a terminal (or behind a process
manager) for the rest of this chapter; it shuts down cleanly on Ctrl-C.

## Health check

`/health` is the one route that never needs the key:

```bash
$ curl -s -i http://127.0.0.1:18090/health
```

```text
HTTP/1.1 200 OK
ok
```

Everything else without the key is a 401:

```bash
$ curl -s -i "http://127.0.0.1:18090/candidates?limit=1"
```

```text
HTTP/1.1 401 Unauthorized
{"error":"missing or invalid API key"}
```

## Searching candidates

```bash
$ curl -s -H 'X-Api-Key: demo-key' "http://127.0.0.1:18090/candidates?state=ME&office=S&limit=3"
```

```json
[
  {"cand_id":"S0ME00111","cycle":2026,"cand_name":"GIDEON, SARA","cand_pty_affiliation":"DEM","cand_election_yr":"2020","cand_office_st":"ME","cand_office":"S","cand_office_district":"00","cand_ici":"C","cand_status":"P","cand_pcc":"C00709899","cand_city":"SOUTH FREEPORT","cand_st":"ME","cand_zip":"04078"},
  {"cand_id":"S2ME00109","cycle":2026,"cand_name":"KING, ANGUS S. JR.","cand_pty_affiliation":"IND","cand_election_yr":"2030","cand_office_st":"ME","cand_office":"S","cand_office_district":"00","cand_ici":"I","cand_status":"C","cand_pcc":"C00516047","cand_city":"BRUNSWICK","cand_st":"ME","cand_zip":"04011"},
  {"cand_id":"S4ME00113","cycle":2026,"cand_name":"COSTELLO, DAVID","cand_pty_affiliation":"DEM","cand_election_yr":"2026","cand_office_st":"ME","cand_office":"S","cand_office_district":"00","cand_ici":"C","cand_status":"C","cand_pcc":"C00907881","cand_city":"BRUNSWICK","cand_st":"ME","cand_zip":"04011"}
]
```

`office=S` means Senate (`H` is House, `P` is President). `q=` is a
case-insensitive substring match on the name, served by the trigram index
from migration 0002. Fetching one specific candidate by ID returns every
cycle that ID appears in:

```bash
$ curl -s -H 'X-Api-Key: demo-key' "http://127.0.0.1:18090/candidates/S0ME00111"
```

```json
[{"cand_id":"S0ME00111","cycle":2026,"cand_name":"GIDEON, SARA","cand_pty_affiliation":"DEM","cand_election_yr":"2020","cand_office_st":"ME","cand_office":"S","cand_office_district":"00","cand_ici":"C","cand_status":"P","cand_pcc":"C00709899","cand_city":"SOUTH FREEPORT","cand_st":"ME","cand_zip":"04078"}]
```

## Searching committees

```bash
$ curl -s -H 'X-Api-Key: demo-key' "http://127.0.0.1:18090/committees?q=SENATE&limit=2"
```

```json
[
  {"cmte_id":"C00068353","cycle":2026,"cmte_nm":"SENATE FUTURE FUND","tres_nm":"DOWD, JP","cmte_city":"WASHINGTON","cmte_st":"DC","cmte_dsgn":"U","cmte_tp":"N","cmte_pty_affiliation":null,"org_tp":null,"connected_org_nm":"NONE","cand_id":"S4VT00017"},
  {"cmte_id":"C00193342","cycle":2026,"cmte_nm":"MCCONNELL SENATE COMMITTEE","tres_nm":"LISKER, LISA","cmte_city":"LOUISVILLE","cmte_st":"KY","cmte_dsgn":"P","cmte_tp":"S","cmte_pty_affiliation":"REP","org_tp":null,"connected_org_nm":"MCCONNELL FOR MAJORITY LEADER COMMITTEE","cand_id":"S2KY00012"}
]
```

The key can also go in the query string, which is handy for a browser
tab. Here's the committee that filed the Form 24 we've been following
since [Quick start](./quick-start.md):

```bash
$ curl -s "http://127.0.0.1:18090/committees/C00865444?api_key=demo-key"
```

```json
[{"cmte_id":"C00865444","cycle":2026,"cmte_nm":"WINSENATE","tres_nm":"LAMBE, REBCCA","cmte_city":"WASHINGTON","cmte_st":"DC","cmte_dsgn":"U","cmte_tp":"O","cmte_pty_affiliation":null,"org_tp":null,"connected_org_nm":"SMP","cand_id":null}]
```

(`REBCCA` is what the FEC's file says. hardmoney doesn't correct data.)

## Looking up one filing

This is the filing ingested with `bulk-load-filing` in
[Loading bulk data](./bulk-etl.md#ingesting-a-single-filing-directly-for-precise-schedule-e-data):

```bash
$ curl -s -H 'X-Api-Key: demo-key' "http://127.0.0.1:18090/filings/2011832"
```

```json
{
  "filing_id": 2011832,
  "form_type": "F24N",
  "fec_version": "8.5",
  "committee_id": "C00865444",
  "is_amendment": false,
  "amends_filing_id": null,
  "header": { "ef_type": "FEC", "fec_version": "8.5", "record_type": "HDR", "report_id": "", "report_number": "", "soft_name": "NGP", "soft_ver": "8" },
  "summary": { "city": "WASHINGTON", "committee_name": "WINSENATE", "date_signed": "20260914", "filer_committee_id_number": "C00865444", "form_type": "F24N", "original_amendment_date": "", "report_type": "48", "state": "DC", "street_1": "1032 15TH ST NW", "street_2": "STE 247", "treasurer_first_name": "REBECCA", "treasurer_last_name": "LAMBE", "treasurer_middle_name": "", "treasurer_prefix": "", "treasurer_suffix": "", "zip_code": "20005" },
  "skipped_lines": 0,
  "ingested_at": "2026-09-15T02:08:37.331324Z"
}
```

(The response also carries the amendment-chain fields
`amendment_indicator`, `amendment_version`, `amendment_chain`,
`most_recent`, `most_recent_file_number`, and `previous_file_number`,
plus `report_type`, `coverage_start_date`, `coverage_end_date`, and
`fec_url`; see [Amendments](./amendments.md).)

Two fields to notice: `skipped_lines` is how many body lines the
(lenient, by default) ingest had to skip (0 here, so this record is
complete), and `ingested_at` is when the row was last written, which
changes if you re-ingest the same filing id. An unknown id is a 404:

```bash
$ curl -s -i -H 'X-Api-Key: demo-key' "http://127.0.0.1:18090/filings/1"
```

```text
HTTP/1.1 404 Not Found
{"error":"no filing with filing_id 1"}
```

## Exact money in JSON: the Schedule E route

The same filing's own Schedule E line items, served back over HTTP:

```bash
$ curl -s -H 'X-Api-Key: demo-key' "http://127.0.0.1:18090/filings/2011832/schedule-e"
```

```json
[
  {"line_index":0,"payee_name":"DECLARATION MEDIA LLC","expenditure_amt":"11282.23","expenditure_date":"2026-09-12","support_oppose_code":"O","candidate_id":"S6OH00304","candidate_name":"JON HUSTED","candidate_office_state":"OH"},
  {"line_index":1,"payee_name":"MVAR MEDIA, LLC","expenditure_amt":"4555.33","expenditure_date":"2026-09-12","support_oppose_code":"O","candidate_id":"S6ME00159","candidate_name":"SUSAN COLLINS","candidate_office_state":"ME"}
]
```

An ingested filing with no Schedule E lines answers `[]`; a filing id
that was never ingested is the same `404` as `GET /filings/{id}`.

`"expenditure_amt":"11282.23"` is a JSON string, not a bare number.
This is `rust_decimal::Decimal`'s serialization: it serializes as a
string so that no JSON parser on the receiving end (many of which parse
numbers as `f64` by default) can silently reintroduce the floating-point
precision loss `hardmoney` avoided in the first place. If you're
consuming this API from another Rust program, deserialize it back into a
`Decimal` (which `serde` handles) rather than an `f64` to preserve that
guarantee end to end. The same applies to
`transaction_amt` on the two bulk-transaction routes below; there the
string carries whatever scale the FEC's file had (`"725"`, `"1562.98"`),
exactly as stored in the `NUMERIC` column.

## Bulk transactions: Schedule A and disbursements

These two routes read the `schedule_a` (`indiv`) and `disbursements`
(`oppexp`) bulk tables, and both take a range of filters. The
`book_demo` namespace only holds a 100-row sample of each, so these are
small answers, but the shape is what matters:

```bash
$ curl -s -H 'X-Api-Key: demo-key' "http://127.0.0.1:18090/schedule-a?min_amount=1000&limit=1"
```

```json
[
  {
    "sub_id": 4011320261301382563,
    "cycle": 2026,
    "cmte_id": "C00000422",
    "name": "HANNAH, FRANK THOS MD",
    "city": "SHELBY",
    "state": "NC",
    "zip_code": "281506047",
    "employer": "MORGANTON EYE",
    "occupation": "PHYSICIAN",
    "transaction_dt": "10222025",
    "transaction_date": "2025-10-22",
    "transaction_amt": "1000",
    "transaction_tp": "15",
    "entity_tp": "IND",
    "memo_cd": null,
    "memo_text": null,
    "file_num": 1925579
  }
]
```

```bash
$ curl -s -H 'X-Api-Key: demo-key' "http://127.0.0.1:18090/disbursements?min_amount=500&limit=2"
```

```json
[
  {
    "sub_id": 4031720261423958768,
    "cycle": 2026,
    "cmte_id": "C00010124",
    "name": "SEXTON PAC CONSULTING",
    "city": "CAMBRIDGE",
    "state": "MD",
    "zip_code": "21613",
    "transaction_dt": "02/18/2026",
    "transaction_date": "2026-02-18",
    "transaction_amt": "725",
    "purpose": "PAC CONSULTING SERVICES",
    "category": "003",
    "category_desc": "Solicitation and Fundraising Expenses ",
    "entity_tp": "ORG",
    "memo_cd": null,
    "memo_text": "PAC CONSULTING SERVICES",
    "file_num": 1952981
  },
  {
    "sub_id": 4031720261423964563,
    "cycle": 2026,
    "cmte_id": "C00024968",
    "name": "MC-BANK OF AMERICA",
    "city": "TAMPA",
    "state": "FL",
    "zip_code": "336225518",
    "transaction_dt": "02/03/2026",
    "transaction_date": "2026-02-03",
    "transaction_amt": "1562.98",
    "purpose": "VISA/MASTER CARD FEES",
    "category": "001",
    "category_desc": "Administrative/Salary/Overhead Expenses ",
    "entity_tp": "ORG",
    "memo_cd": null,
    "memo_text": "VISA/MASTER CARD FEES",
    "file_num": 1952969
  }
]
```

Look at the two date fields. `transaction_dt` is the FEC's raw string
(`"10222025"` in the Schedule A file, `"02/18/2026"` in the disbursements
file, two different formats as described in [Dates](./dates.md)), and
`transaction_date` is the parsed ISO date (or `null` if the raw value
didn't parse). Both routes order by `transaction_date` descending
(nulls last, then `sub_id`), which is why the newest disbursement comes
first. Ordering by the raw text would put December 2024 ahead of
December 2025.

The date filters take ISO dates and apply to the parsed column:

```bash
$ curl -s -H 'X-Api-Key: demo-key' "http://127.0.0.1:18090/disbursements?min_date=2026-02-01&max_date=2026-02-28&limit=3"
```

```text
02/18/2026  2026-02-18   725     SEXTON PAC CONSULTING
02/18/2026  2026-02-18   167.33  HSBC BANK
02/17/2026  2026-02-17   138.6   BANK OF AMERICA FEDERAL
```

(Reformatted to four columns for the page.) `state` is matched exactly
after upper-casing, so `?state=ca` and `?state=CA` return the same rows;
`zip_code` is a prefix match (`?zip_code=281` matches `281506047`); and
`name`, `employer`, `occupation`, `city`, and `purpose` are
case-insensitive substring matches.

## Independent expenditures from the FEC's own dump

`/independent-expenditures` reads the `independent_expenditures` view
over the FEC's restored `schedule_e` `pg_dump`
([Bulk ETL](./bulk-etl.md#an-alternative-restoring-the-fecs-own-database-dumps)),
half a million rows going back to 1975 in the book's database:

```bash
$ curl -s -H 'X-Api-Key: demo-key' "http://127.0.0.1:18090/independent-expenditures?candidate_id=S6OH00163&support_oppose_code=O&limit=1"
```

```json
[
  {
    "sub_id": 4090820261595915975,
    "cmte_id": "C00687103",
    "committee_name": null,
    "payee_name": "TARGETED VICTORY LLC",
    "candidate_id": "S6OH00163",
    "candidate_name": "BROWN, SHERROD",
    "candidate_office_state": "OH",
    "support_oppose_code": "O",
    "support_oppose_desc": "OPPOSE",
    "expenditure_amt": "2500.00",
    "expenditure_date": "2026-07-13T00:00:00",
    "expenditure_description": "MEDIA PRODUCTION",
    "rpt_yr": 2026,
    "election_cycle": 2026
  }
]
```

If the dump hasn't been restored into this database, the view doesn't
exist and this route returns a 503, not a bare 500, with the fix in the
message:

```text
HTTP/1.1 503 Service Unavailable
{"error":"independent_expenditures is not available: run `hardmoney bulk-restore-dump schedule_e` to load the FEC's official Schedule E pg_dump archive, then `hardmoney schema-init`"}
```

(Illustrative: the book's database has the dump restored, so this
couldn't be triggered; the message is the crate's.)

## What is loaded here? The `/schema` route

`GET /schema` answers "what is this server serving?": the namespace,
the migration state, the most recent load of each
source and cycle (straight from the `loads` table described in
[Reloading](./reloading.md#every-load-is-recorded)), and whether the
independent-expenditures view exists:

```bash
$ curl -s -H 'X-Api-Key: demo-key' http://127.0.0.1:18090/schema
```

```json
{
  "hardmoney_version": "1.0.0",
  "namespace": "book_demo",
  "migrations_applied": [1, 2],
  "migrations_pending": [],
  "loads": [
    {
      "load_id": 7,
      "source": "candidate_committee_links",
      "cycle": 2026,
      "loaded_at": "2026-09-15T02:12:22.170841Z",
      "mode": "replace",
      "row_count": 100,
      "row_limit": 100,
      "dates_nulled": 0,
      "source_etag": "08c741fcc06db92bbfee161ef01724f3",
      "source_last_modified": "2026-09-14T05:41:56Z",
      "hardmoney_version": "1.0.0"
    },
    {
      "load_id": 15,
      "source": "candidates",
      "cycle": 2026,
      "loaded_at": "2026-09-15T02:22:07.108593Z",
      "mode": "replace",
      "row_count": 8557,
      "row_limit": null,
      "dates_nulled": 0,
      "source_etag": "07fc62b79883e5b8341f37d60cc7f5d5",
      "source_last_modified": "2026-09-14T05:41:47Z",
      "hardmoney_version": "1.0.0"
    },
    {
      "load_id": 11,
      "source": "disbursements",
      "cycle": 2026,
      "loaded_at": "2026-09-15T02:12:25.333860Z",
      "mode": "replace",
      "row_count": 100,
      "row_limit": 100,
      "dates_nulled": 0,
      "source_etag": "3831fa738a497cc9fe77d0d99a2e87f6-6",
      "source_last_modified": "2026-09-13T16:01:39Z",
      "hardmoney_version": "1.0.0"
    }
  ],
  "independent_expenditures_available": true
}
```

(Trimmed from ten `loads` entries to three.) A `row_limit` of `null`
means a full load; a number means a `--limit` sample. `source_etag` and
`source_last_modified` are what `--if-changed` compares against. This is
the route to poll from a dashboard or a deploy check: if
`migrations_pending` is non-empty, `serve` wouldn't have started, so in
practice it's always `[]` here, but a different tool looking at the
same namespace can use it to see that `schema-init` is due.

## All routes

Every list/search route accepts `limit`/`offset` for pagination
(`limit` defaults to 50 and must be 1-500; `offset` defaults to 0 and
must be >= 0; anything else is a `400` with `{"error": "limit must be
between 1 and 500"}` or `{"error": "offset must be >= 0"}`, never a
silently adjusted page), in addition to the filters listed below.
Substring filters are case-insensitive `ILIKE '%term%'` matches;
`min_date`/`max_date` are ISO dates (`2025-12-20`) applied to the parsed
`*_date` column; `min_amount`/`max_amount` are exact decimals.

| Route | Description | Query parameters |
|---|---|---|
| `GET /health` | liveness check; always open, even with `--api-key` | none |
| `GET /schema` | namespace, version, migrations, latest load per source/cycle, IE view availability | none |
| `GET /candidates` | search candidates | `cycle`, `state`, `office`, `q` (substring on name) |
| `GET /candidates/{cand_id}` | one candidate, every cycle | none |
| `GET /committees` | search committees | `cycle`, `cmte_tp`, `q` (substring on name) |
| `GET /committees/{cmte_id}` | one committee, every cycle | none |
| `GET /schedule-a` | search Schedule A (individual contributions, from `indiv`) | `cmte_id`, `cycle`, `name`, `employer`, `occupation`, `state`, `zip_code` (prefix), `min_amount`, `max_amount`, `min_date`, `max_date` |
| `GET /disbursements` | search Schedule B operating expenditures (from `oppexp`) | `cmte_id`, `cycle`, `name`, `city`, `state`, `purpose`, `min_amount`, `max_amount`, `min_date`, `max_date` |
| `GET /independent-expenditures` | search the FEC's own Schedule E dump | `candidate_id`, `cmte_id`, `support_oppose_code` |
| `GET /filings` | list ingested filings | `committee_id`, `most_recent` (`true`/`false`), `form_type` (base such as `F3X`, or exact such as `F3XA`) |
| `GET /filings/{filing_id}` | one ingested filing's header/summary, its amendment chain (openFEC field names: `amendment_indicator`, `amendment_version`, `amendment_chain`, `most_recent`, `most_recent_file_number`, `previous_file_number`), `report_type`, coverage dates, `fec_url`, plus `skipped_lines` and `ingested_at` | none |
| `GET /filings/{filing_id}/schedule-e` | that filing's own Schedule E line items (`[]` if it has none; 404 for an id that was never ingested) | none |

Exact field lists for each route's JSON response live in
`src/api/routes/*.rs` in the repository; the table above covers the
filters you'll reach for in practice.

## Running it from Rust

`serve` is a thin wrapper over `hardmoney::api`:

```rust
use hardmoney::api::{ApiConfig, serve};
use hardmoney::db::{DbConfig, Namespace, connect, migrate};

# async fn run(url: &str) -> Result<(), Box<dyn std::error::Error>> {
let pool = connect(&DbConfig::new(url).namespace(Namespace::new("book_demo")?)).await?;
migrate(&pool).await?;

let config = ApiConfig::new("127.0.0.1:18090".parse()?)
    .api_key(Some("demo-key".to_string()));
serve(pool, config).await?; // runs until SIGINT/SIGTERM
# Ok(())
# }
```

`hardmoney::api::router(pool, &config)` gives you the bare `axum::Router`
instead, if you want to mount it inside a larger application or drive it
from tests with `tower::ServiceExt::oneshot`, which is what
`tests/postgres_integration.rs` does.
