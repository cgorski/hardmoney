# hardmoney

[![CI](https://github.com/cgorski/hardmoney/actions/workflows/ci.yml/badge.svg)](https://github.com/cgorski/hardmoney/actions/workflows/ci.yml)

A single Rust crate for FEC campaign-finance data, usable as a library or
as a CLI: a parser for raw `.fec` electronic filings, a Postgres ETL for
the FEC's bulk-data downloads, and an Axum REST API serving the tables it
builds.

License: `Apache-2.0 OR BSD-3-Clause`. See [`LICENSE-APACHE`](./LICENSE-APACHE),
[`LICENSE-BSD`](./LICENSE-BSD), and [`NOTICE`](./NOTICE) for third-party
data attribution.

## What's here

Three things, independent or combined:

1. **`hardmoney::parser`** -- parses raw FEC electronic filings (`.fec`
   files) into header, summary, and itemized line data. Handles every
   electronic filing spec era: the pre-2003 comma/quoted-CSV format
   (spec 3.x), the transitional comma-delimited format (spec 5.x), and
   the modern ASCII-28-delimited format (spec 6.x through the current
   8.5), each with correct version-bucketed field positions per form.
2. **`hardmoney::bulk`** -- a Postgres ETL that loads the FEC's own
   pre-aggregated bulk-data downloads (candidates, committees,
   contributions, disbursements, summaries, ...) into a normalized
   schema, plus direct single-filing ingestion via the parser for precise
   Schedule E (independent expenditure) extraction.
3. **`hardmoney::api`** -- an Axum REST API serving the tables `bulk`
   builds (candidates, committees, Schedule A search, disbursements,
   independent expenditures, filing lookups).

## Quick start

### As a library

```rust
use hardmoney::parser::Filing;

let bytes = std::fs::read("filing.fec").unwrap();
let filing = Filing::parse_bytes(&bytes).unwrap();
println!("{} ({})", filing.raw_form_type, filing.version);
println!("total receipts: {:?}", filing.summary.get("col_a_total_receipts"));
```

A downstream crate that only needs the parser (no Postgres, no Axum) can
depend on it with a slimmer feature set:

```toml
hardmoney = { version = "0.1", default-features = false, features = ["fetch"] }
```

### As a CLI

```bash
# Parse a single filing and print its header/summary as JSON.
hardmoney parse path/to/filing.fec

# Apply the Postgres schema (idempotent).
hardmoney schema-init --database-url postgres://user:pass@localhost/fec

# Load one bulk-data source for a cycle (downloads from fec.gov if --file omitted).
hardmoney bulk-load --database-url $DATABASE_URL candidates --cycle 2026

# Load every bulk source for a cycle in one shot.
hardmoney bulk-load-all --database-url $DATABASE_URL --cycle 2026

# Restore one of the FEC's own official pg_dump archives.
hardmoney bulk-restore-dump --database-url $DATABASE_URL schedule_e

# Ingest a single raw filing (by local path or FEC filing id) for precise
# Schedule E extraction, complementing the aggregate bulk tables.
hardmoney bulk-load-filing --database-url $DATABASE_URL 1666999

# Run the REST API server.
hardmoney serve --database-url $DATABASE_URL --bind 0.0.0.0:8080
```

## Feature flags

| Feature | Enables | Default |
|---|---|---|
| `fetch` | `Filing::fetch` (download a raw filing from docquery.fec.gov) | on |
| `serde` | `Serialize`/`Deserialize` on parser types | on (via `bulk`/`api`) |
| `bulk` | the `bulk` module (Postgres ETL, needs `sqlx`) | on |
| `api` | the `api` module (Axum server) | on |
| `cli` | the `hardmoney` binary | on |

## REST API

Once `serve` is running:

| Route | Description |
|---|---|
| `GET /health` | liveness check |
| `GET /candidates` | search candidates |
| `GET /candidates/{cand_id}` | one candidate |
| `GET /committees` | search committees |
| `GET /committees/{cmte_id}` | one committee |
| `GET /schedule-a` | search Schedule A (individual contributions) |
| `GET /disbursements` | search Schedule B disbursements |
| `GET /independent-expenditures` | search Schedule E independent expenditures |
| `GET /filings/{filing_id}` | one raw filing's parsed header/summary |
| `GET /filings/{filing_id}/schedule-e` | one filing's Schedule E line items |

## Parser correctness: the field-name collision fix

The FEC's own filing spec documentation only defines column *positions*
per form/version; canonical field *names* are a convenience layer added by
parsing libraries (this crate included, following the precedent set by
`nyt-pyfec` and `fech`). During development of this crate, an audit of the
vendored format tables found that six of them --- **F2, F3P, F3X, F4,
SchC1, SchL** --- assigned the same canonical name to two genuinely
different column positions within a single spec-version bucket. Because
the line parser builds an ordered `name -> value` map per line, an
unresolved collision meant the second-occurring field silently overwrote
the first, discarding real filing data. This was confirmed against real
filings: in `tests/fixtures/F3XN_2011834.fec`, two colliding "total
contribution refunds" fields held genuinely different values (5000.00 vs
0.00, and 20000.00 vs 5000.00) before the fix.

All six tables were corrected by renaming the colliding field to a
distinct, form-accurate canonical name (e.g. separating a Form 3X
"detailed summary recap" total from the plain receipts/disbursements
total it echoes, or fixing a straight mislabeling bug in Schedule L where
two columns were tagged Column B but structurally belong to Column A's
own per-period recap). Full details, including every renamed field and
its FEC form-line justification, are documented in [`NOTICE`](./NOTICE).

Two permanent safeguards now guard against regressions:

- `Line::from_csv_str` (`src/parser/line.rs`) returns a hard
  `FecError::DuplicateCanonicalField` error instead of silently
  overwriting if any future edit -- including a future upstream
  `fech-sources` update -- reintroduces a same-bucket collision.
- `tests/format_table_integrity.rs` scans every bundled format table on
  every test run and fails if any collision exists, plus pins the six
  corrected field names so a careless revert is caught even without a
  literal collision.

## FEC spec version handling

The FEC's electronic filing format has gone through multiple incompatible
eras since electronic filing began:

| Spec version | Delimiter | Header layout |
|---|---|---|
| 3.x, 4.x | comma, double-quoted CSV | "old" 8-field electronic header |
| 5.x | comma, unquoted | "old" 8-field electronic header |
| 6.x, 7.x, 8.x | ASCII character 28 (`\x1c`) | "new" 7-field electronic header |

`hardmoney` handles all three regimes: `header::parse` selects the correct
header field layout by version prefix, and `Filing::parse_bytes` picks the
comma-delimited or ASCII-28 body parser accordingly. Each format table's
column-position data is itself version-bucketed (e.g. F3X.csv defines
separate buckets for `8.5|8.4|...|6.1`, `5.3|5.2|5.1|5.0`, and `3`), so the
same canonical field resolves to the correct byte position for whichever
era a filing was actually submitted in.

This is verified against **real filings from every era**, not just
synthetic strings, in `tests/real_filings.rs`:

- Spec 8.5 (the current format): 17 live-downloaded filings across F24,
  F3, F3X, F5, F6, F99, and their schedules.
- Spec 3.00 (2001, quoted-CSV): a real amended Form 3X filing from Merck
  PAC (`tests/fixtures/F3XA_27789_v3.fec`, sourced from
  [`esonderegger/fecfile`](https://github.com/esonderegger/fecfile)'s
  bundled test data), with 2,837 real Schedule A/B body lines.
- Spec 5.1 and 5.3 (comma-delimited transitional era): real Form 6 and
  Form 3X filings pulled directly from `docquery.fec.gov`.
- Spec 6.1 (the first ASCII-28 era): a real Form 3X filing, also from
  `docquery.fec.gov`.

## Other correctness fixes worth knowing about

- **`oppexp` trailing-column quirk**: the FEC's own bulk `oppexp`
  (operating expenditures) files sometimes ship a trailing empty column
  not documented in the official layout; see
  [fecgov/FEC#11052](https://github.com/fecgov/FEC/issues/11052).
  `hardmoney`'s bulk loader tolerates this rather than failing the row.
- **Schedule A / Schedule B spreadsheet-float column positions**: the
  vendored `SchA.csv`/`SchB.csv` format tables were exported from a
  spreadsheet and encode most column positions as `"7.0"` rather than
  `"7"`. A naive integer parse silently rejects those cells, which drops
  every field past the first affected row with no error. The
  column-position parser (`src/parser/line.rs`) now accepts the
  float-formatted form as long as it represents a whole number.
- **`independent_expenditures` view migrations**: switching the
  `sub_id`/`file_num` columns from `numeric` to `bigint` requires
  `DROP VIEW` + `CREATE VIEW` rather than `CREATE OR REPLACE VIEW`, because
  Postgres refuses in-place column type changes through `REPLACE`; the
  schema migration in `src/db/schema.sql` accounts for this.

## Development

```bash
cargo build --all-features
cargo test --all-features
```

Test suite: unit tests colocated with parser modules, `tests/real_filings.rs`
(live-downloaded real filings across every spec era), and
`tests/format_table_integrity.rs` (collision/regression guard over every
vendored format table).

## Sources and provenance

- FEC electronic filing spec documentation: <https://www.fec.gov/help-candidates-and-committees/filing-reports/>
- Format-table column-position data: [`dwillis/fech-sources`](https://github.com/dwillis/fech-sources),
  itself derived from the [Fech](https://github.com/dwillis/Fech) Ruby
  gem (Apache-2.0, per [RubyGems metadata](https://rubygems.org/api/v1/gems/fech.json)).
  See [`NOTICE`](./NOTICE) for full attribution and the list of
  corrections made to this data.
- Parsing-approach inspiration (not copied code): [`newsdev/nyt-pyfec`](https://github.com/newsdev/nyt-pyfec)
  (Apache-2.0, Copyright 2015 The New York Times Company).
- Real filing test fixtures: the FEC's own `docquery.fec.gov` document
  store and RSS feed, and [`esonderegger/fecfile`](https://github.com/esonderegger/fecfile)'s
  bundled historical test data.
