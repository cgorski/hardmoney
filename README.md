# hardmoney

[![CI](https://github.com/cgorski/hardmoney/actions/workflows/ci.yml/badge.svg)](https://github.com/cgorski/hardmoney/actions/workflows/ci.yml)
[![Book](https://github.com/cgorski/hardmoney/actions/workflows/book.yml/badge.svg)](https://cgorski.github.io/hardmoney/)

## What this tool does, in plain English

Every U.S. political campaign and PAC has to periodically file a report
with the Federal Election Commission (FEC) disclosing who gave them money
and who they spent it on. The FEC publishes these reports as raw text
files (`.fec` filings) and as large pre-aggregated bulk-data downloads.
Both are useful, and both are harder to work with correctly than they
look: the file format has changed several times over the years, and money
amounts need to stay *exact* -- not approximated the way ordinary
floating-point math would approximate them.

`hardmoney` is a single Rust crate (usable as a code library or as a
command-line tool) that:

- **Reads a raw `.fec` filing** and turns it into structured data --
  who filed it, what period it covers, and every itemized contribution or
  expenditure in it -- correctly, no matter which year or format version
  the filing was submitted under.
- **Gives you exact dollar amounts, real dates, and resolved names**
  instead of raw, unparsed text fields, via an easy-to-use "typed view"
  layer.
- **Loads the FEC's own bulk-data downloads into a normal Postgres
  database** you can query with ordinary SQL -- candidates, committees,
  contributions, disbursements, and more, for an entire election cycle at
  once.
- **Serves that database over a REST API**, so other programs (or a
  website) can look up candidates, committees, and transactions with
  simple HTTP requests instead of needing direct database access.

You can use any one of these four things on its own, or all of them
together.

### New to this? Read the book

**[The hardmoney Book](https://cgorski.github.io/hardmoney/)** is a
beginner-friendly tutorial that walks through all of this step by step --
what's actually inside a `.fec` file, why the money handling is built the
way it is, and a full walkthrough of the bulk-data loading and REST API --
with real commands and real output at every step, not made-up examples.
It includes a quick-start tutorial, worked examples for every part of the
crate, a full CLI reference, and a troubleshooting/FAQ chapter. Start
there if you're new to FEC data or new to this crate; keep reading this
README for a denser, example-and-reference-style overview.

License: `Apache-2.0 OR BSD-3-Clause`. See [`LICENSE-APACHE`](./LICENSE-APACHE),
[`LICENSE-BSD`](./LICENSE-BSD), and [`NOTICE`](./NOTICE) for third-party
data attribution.

## What's here

Four things, independent or combined:

1. **`hardmoney::parser`** -- parses raw FEC electronic filings (`.fec`
   files) into header, summary, and itemized line data. Handles every
   electronic filing spec era: the pre-2003 comma/quoted-CSV format
   (spec 3.x), the transitional comma-delimited format (spec 5.x), and
   the modern ASCII-28-delimited format (spec 6.x through the current
   8.5), each with correct version-bucketed field positions per form.
2. **Typed views** (`ScheduleA`, `ScheduleB`, `ScheduleE`, `Form3XSummary`)
   -- an ergonomic layer over the raw parser output: exact
   `rust_decimal::Decimal` money (never lossy `f64`), real `NaiveDate`
   values, and a name that resolves correctly whether a filing uses an
   old-format combined name field or the current split
   organization/first/last fields.
3. **`hardmoney::bulk`** -- a Postgres ETL that loads the FEC's own
   pre-aggregated bulk-data downloads (candidates, committees,
   contributions, disbursements, summaries, ...) into a normalized
   schema, plus direct single-filing ingestion via the parser for precise
   Schedule E (independent expenditure) extraction.
4. **`hardmoney::api`** -- an Axum REST API serving the tables `bulk`
   builds (candidates, committees, Schedule A search, disbursements,
   independent expenditures, filing lookups).

## Quick start

### As a library

The crate root re-exports everything you need for the common path, so you
don't have to reach into `hardmoney::parser`:

```rust
use hardmoney::Filing;

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
hardmoney bulk-load-filing --database-url $DATABASE_URL 2011831

# Run the REST API server.
hardmoney serve --database-url $DATABASE_URL --bind 0.0.0.0:8080
```

Every command above was run against this crate's own test fixtures and a
live local Postgres instance while writing this README -- see
[Examples](#examples) for the same coverage as runnable library code.

## Ergonomic typed views

Raw parser output (`filing.lines`) hands back an
`IndexMap<String, String>` per line -- faithful to the wire format, but
every caller has to remember field names, hand-parse dates and money, and
work around the fact that a schedule's "name" field is genuinely shaped
differently depending on which spec era a filing was submitted under.
The typed views in `hardmoney::parser::typed` (re-exported at the crate
root) handle that:

```rust
use hardmoney::{Filing, ScheduleA};
use rust_decimal::Decimal;

let bytes = std::fs::read("filing.fec")?;
let filing = Filing::parse_bytes(&bytes)?;

for line in filing.lines.iter().filter(|l| l.table == "SchA") {
    let contribution: ScheduleA = line.try_into()?;
    println!(
        "{}: ${} on {:?}",
        contribution.contributor_name.as_deref().unwrap_or("(no name)"),
        contribution.contribution_amount.unwrap_or(Decimal::ZERO),
        contribution.contribution_date,
    );
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

What this buys you over the raw layer:

- **Exact money, end to end.** [`parse_money`] parses amounts into an
  exact [`rust_decimal::Decimal`], never `f64` -- binary floating point
  cannot represent every decimal currency value exactly, and that error
  compounds across millions of transactions. This isn't just the parser:
  the same `Decimal` type is what gets written to and read back from
  Postgres's `NUMERIC` columns and what the REST API returns in JSON, so
  a contribution amount is never round-tripped through `f64` at any layer
  from raw filing bytes to the JSON response you get back.
- **Real dates.** [`parse_fec_date`] parses the FEC's `YYYYMMDD` fields
  into `chrono::NaiveDate`, treating blank and all-zero dates (both
  common in real filings) as `None` rather than a parse error.
- **Cross-version names.** FEC spec versions before 8.0 report a single
  combined name field (`contributor_name`, `payee_name`,
  `candidate_name`). Spec 8.0 and later -- what virtually all real-world
  electronic filings use today -- drops that field entirely in favor of
  `*_organization_name` or split `*_first_name`/`*_middle_name`/
  `*_last_name`. Each typed view's name field checks both shapes, so it
  resolves correctly regardless of which era a filing came from instead
  of silently returning `None` for every current filing.
- **A real error type**, not a panic. `TryFrom<&ParsedLine>` returns
  `Result<_, TypedViewError>` if a genuinely required field (the filer
  committee id) is missing -- everything else degrades to `None`, since
  real-world FEC data is routinely incomplete on optional fields.

`ScheduleA` (itemized receipts), `ScheduleB` (itemized disbursements),
`ScheduleE` (independent expenditures), and `Form3XSummary` (committee
period/cycle totals) all follow this pattern. See
[`examples/typed_schedule_a_totals.rs`](./examples/typed_schedule_a_totals.rs)
for a complete runnable example.

## Feature flags

| Feature | Enables | Default |
|---|---|---|
| `fetch` | `Filing::fetch` (download a raw filing from docquery.fec.gov) | on |
| `serde` | `Serialize`/`Deserialize` on parser types | on (via `bulk`/`api`) |
| `bulk` | the `bulk` module (Postgres ETL, needs `sqlx`) | on |
| `api` | the `api` module (Axum server) | on |
| `cli` | the `hardmoney` binary | on |

## REST API

Once `serve` is running, all list/search routes accept `limit`/`offset`
for pagination in addition to the filters below:

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

Exact field lists live in `src/api/routes/*.rs`; the tables above cover
the filters you'll reach for in practice.

## Examples

Runnable, self-contained programs under [`examples/`](./examples/), each
verified against real fixture data (and, where noted, a live network
call):

| Example | Use case | Run with |
|---|---|---|
| [`parse_filing.rs`](./examples/parse_filing.rs) | Common: parse a `.fec` file and print header/summary/line-count breakdown. | `cargo run --example parse_filing -- path/to/filing.fec` |
| [`typed_schedule_a_totals.rs`](./examples/typed_schedule_a_totals.rs) | Common, ergonomic: sum itemized Schedule A contributions and find the largest, via the typed-view layer. | `cargo run --example typed_schedule_a_totals -- path/to/filing.fec` |
| [`fetch_live_filing.rs`](./examples/fetch_live_filing.rs) | Less common, very useful: download a filing directly from `docquery.fec.gov` (requires network and the `fetch` feature) and summarize it with `Form3XSummary`. | `cargo run --example fetch_live_filing -- 2011831` |
| [`handle_parse_errors.rs`](./examples/handle_parse_errors.rs) | Less common, very useful: `hardmoney`'s error type is designed to be matched on, including the field-name collision guard described below. | `cargo run --example handle_parse_errors` |

All four build under the minimal `--no-default-features --features fetch`
set as well as `--all-features`. The bulk-ETL and REST-API flows
(`bulk-load-filing`, `serve`, and the routes above) need a live Postgres
instance, so they're demonstrated as CLI commands in
[Quick start](#as-a-cli) rather than as zero-setup `cargo run --example`
programs.

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
- **Schedule E ingestion is scoped to genuine Schedule E lines**:
  `ScheduleE::try_from` only strictly requires a `filer_committee_id_number`
  field, which is present on nearly every schedule (Schedule A, B, C,
  ...). `bulk-load-filing`'s single-filing ingest now filters to
  `table == "SchE"` before attempting that conversion, so a filing with no
  independent expenditures correctly stores zero Schedule E rows instead
  of misfiling every other schedule's lines as all-null "Schedule E"
  entries. Covered by `bulk::ingest::tests::only_genuine_schedule_e_lines_are_extracted`.
- **Cross-version name resolution**: as described in
  [Ergonomic typed views](#ergonomic-typed-views), the `contributor_name`
  (Schedule A), `payee_name` (Schedule B/E), and `candidate_name`
  (Schedule E) typed fields fall back from the old-format combined name to
  the modern split organization/first/middle/last fields, so they resolve
  correctly for present-day (spec 8.x) filings rather than always
  returning `None`.

## Development

```bash
cargo build --all-features
cargo test --all-features
cargo fmt --all -- --check
cargo clippy --all-features --all-targets -- -D warnings
```

Test suite: unit tests colocated with parser/typed-view/bulk-ingest
modules, `tests/real_filings.rs` (live-downloaded real filings across
every spec era), and `tests/format_table_integrity.rs`
(collision/regression guard over every vendored format table). CI runs
all of the commands above on every push and pull request to `main`.

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

[`parse_money`]: https://docs.rs/hardmoney/latest/hardmoney/fn.parse_money.html
[`rust_decimal::Decimal`]: https://docs.rs/rust_decimal/latest/rust_decimal/struct.Decimal.html
[`parse_fec_date`]: https://docs.rs/hardmoney/latest/hardmoney/fn.parse_fec_date.html
