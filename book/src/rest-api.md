# The REST API

Once you have a Postgres database loaded (via the previous chapter), you
can serve it over HTTP instead of requiring every consumer to write SQL
directly. This chapter runs the server and walks through every route with
real request/response pairs -- all captured against a live local server
during the writing of this book, not invented.

## Starting the server

```bash
$ export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:5432/fec_dev"
$ cargo run --quiet --bin hardmoney -- serve --database-url "$DATABASE_URL" --bind 0.0.0.0:8080
```

```text
hardmoney API listening on 0.0.0.0:8080
```

`--bind` defaults to `0.0.0.0:8080` if omitted. Leave this running in a
terminal (or behind a process manager) for the rest of this chapter.

## Health check

```bash
$ curl -s http://127.0.0.1:8080/health
```

```text
ok
```

## Searching candidates

```bash
$ curl -s "http://127.0.0.1:8080/candidates?state=ME&office=S&limit=3"
```

```json
[
  {"cand_id":"S0ME00111","cycle":2026,"cand_name":"GIDEON, SARA","cand_pty_affiliation":"DEM","cand_election_yr":"2020","cand_office_st":"ME","cand_office":"S","cand_office_district":"00","cand_ici":"C","cand_status":"P","cand_pcc":"C00709899","cand_city":"SOUTH FREEPORT","cand_st":"ME","cand_zip":"04078"},
  {"cand_id":"S2ME00109","cycle":2026,"cand_name":"KING, ANGUS S. JR.","cand_pty_affiliation":"IND","cand_election_yr":"2030","cand_office_st":"ME","cand_office":"S","cand_office_district":"00","cand_ici":"I","cand_status":"C","cand_pcc":"C00516047","cand_city":"BRUNSWICK","cand_st":"ME","cand_zip":"04011"},
  {"cand_id":"S4ME00113","cycle":2026,"cand_name":"COSTELLO, DAVID","cand_pty_affiliation":"DEM","cand_election_yr":"2026","cand_office_st":"ME","cand_office":"S","cand_office_district":"00","cand_ici":"C","cand_status":"C","cand_pcc":"C00907881","cand_city":"BRUNSWICK","cand_st":"ME","cand_zip":"04011"}
]
```

`office=S` means Senate (`H` is House, `P` is President). Fetching one
specific candidate by ID:

```bash
$ curl -s "http://127.0.0.1:8080/candidates/S0ME00111"
```

```json
[{"cand_id":"S0ME00111","cycle":2026,"cand_name":"GIDEON, SARA","cand_pty_affiliation":"DEM","cand_election_yr":"2020","cand_office_st":"ME","cand_office":"S","cand_office_district":"00","cand_ici":"C","cand_status":"P","cand_pcc":"C00709899","cand_city":"SOUTH FREEPORT","cand_st":"ME","cand_zip":"04078"}]
```

## Searching committees

```bash
$ curl -s "http://127.0.0.1:8080/committees?q=SENATE&limit=2"
```

```json
[
  {"cmte_id":"C00068353","cycle":2026,"cmte_nm":"SENATE FUTURE FUND","tres_nm":"DOWD, JP","cmte_city":"WASHINGTON","cmte_st":"DC","cmte_dsgn":"U","cmte_tp":"N","cmte_pty_affiliation":null,"org_tp":null,"connected_org_nm":"NONE","cand_id":"S4VT00017"},
  {"cmte_id":"C00193342","cycle":2026,"cmte_nm":"MCCONNELL SENATE COMMITTEE","tres_nm":"LISKER, LISA","cmte_city":"LOUISVILLE","cmte_st":"KY","cmte_dsgn":"P","cmte_tp":"S","cmte_pty_affiliation":"REP","org_tp":null,"connected_org_nm":"MCCONNELL FOR MAJORITY LEADER COMMITTEE","cand_id":"S2KY00012"}
]
```

## Looking up one filing

```bash
$ curl -s "http://127.0.0.1:8080/filings/242011832"
```

```json
{
  "filing_id": 242011832,
  "form_type": "F24N",
  "fec_version": "8.5",
  "committee_id": "C00865444",
  "is_amendment": false,
  "amends_filing_id": null,
  "header": { "ef_type": "FEC", "fec_version": "8.5", "record_type": "HDR", "report_id": "", "report_number": "", "soft_name": "NGP", "soft_ver": "8" },
  "summary": { "city": "WASHINGTON", "committee_name": "WINSENATE", "date_signed": "20260914", "filer_committee_id_number": "C00865444", "form_type": "F24N", "original_amendment_date": "", "report_type": "48", "state": "DC", "street_1": "1032 15TH ST NW", "street_2": "STE 247", "treasurer_first_name": "REBECCA", "treasurer_last_name": "LAMBE", "treasurer_middle_name": "", "treasurer_prefix": "", "treasurer_suffix": "", "zip_code": "20005" }
}
```

## Exact money in JSON: the Schedule E route

This is the route from the [Bulk ETL](./bulk-etl.md) chapter's
`bulk-load-filing` example, served back over HTTP:

```bash
$ curl -s "http://127.0.0.1:8080/filings/242011832/schedule-e"
```

```json
[
  {"line_index":0,"payee_name":"DECLARATION MEDIA LLC","expenditure_amt":"11282.23","expenditure_date":"2026-09-12","support_oppose_code":"O","candidate_id":"S6OH00304","candidate_name":"JON HUSTED","candidate_office_state":"OH"},
  {"line_index":1,"payee_name":"MVAR MEDIA, LLC","expenditure_amt":"4555.33","expenditure_date":"2026-09-12","support_oppose_code":"O","candidate_id":"S6ME00159","candidate_name":"SUSAN COLLINS","candidate_office_state":"ME"}
]
```

Note `"expenditure_amt":"11282.23"` -- a JSON **string**, not a bare
number. This is `rust_decimal::Decimal`'s serialization: it serializes as
a string specifically so that no JSON parser on the receiving end (many
of which parse numbers as `f64` by default) can silently reintroduce the
floating-point precision loss `hardmoney` avoided in the first place. If
you're consuming this API from another Rust program, deserialize it back
into a `Decimal` (which `serde` handles automatically) rather than an
`f64` to preserve that guarantee end to end.

## Filters that take a `Decimal`, and what happens with bad input

`min_amount` on the Schedule A route is typed as `Option<Decimal>`, so it
round-trips the same exactness guarantee through query-string parsing.
A valid amount works like any other filter:

```bash
$ curl -s "http://127.0.0.1:8080/schedule-a?cmte_id=C00865444&min_amount=100.00&limit=2"
```

```json
[]
```

(An empty result here is expected and correct -- this particular
committee's loaded data in this example database doesn't have itemized
Schedule A rows above $100; try it against a committee you've loaded
Schedule A data for.)

An invalid amount is rejected with a clear `400 Bad Request`, not
silently coerced to zero or accepted as a string filter that would never
match anything:

```bash
$ curl -s "http://127.0.0.1:8080/schedule-a?min_amount=notanumber"
```

```text
Failed to deserialize query string: min_amount: invalid value: string "notanumber", expected a Decimal type representing a fixed-point number
```

## All routes

Every list/search route accepts `limit`/`offset` for pagination
(`limit` defaults to 50, clamps to a maximum of 500, and a minimum of 1;
`offset` defaults to 0), in addition to the filters listed below.

| Route | Description | Query parameters |
|---|---|---|
| `GET /health` | liveness check | -- |
| `GET /candidates` | search candidates | `cycle`, `state`, `office`, `q` |
| `GET /candidates/{cand_id}` | one candidate | -- |
| `GET /committees` | search committees | `cycle`, `cmte_tp`, `q` |
| `GET /committees/{cmte_id}` | one committee | -- |
| `GET /schedule-a` | search Schedule A (individual contributions) | `cmte_id`, `cycle`, `name`, `employer`, `min_amount` |
| `GET /disbursements` | search Schedule B disbursements | `cmte_id`, `name`, `city`, `state`, `transaction_dt`, `purpose` |
| `GET /independent-expenditures` | search Schedule E independent expenditures | `candidate_id`, `cmte_id`, `support_oppose_code` |
| `GET /filings/{filing_id}` | one raw filing's parsed header/summary | -- |
| `GET /filings/{filing_id}/schedule-e` | one filing's Schedule E line items | -- |

Exact field lists for each route's JSON response live in
`src/api/routes/*.rs` in the repository; the table above covers the
filters you'll reach for in practice.
