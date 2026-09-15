# hardmoney

[![CI](https://github.com/cgorski/hardmoney/actions/workflows/ci.yml/badge.svg)](https://github.com/cgorski/hardmoney/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/hardmoney.svg)](https://crates.io/crates/hardmoney)
[![docs.rs](https://docs.rs/hardmoney/badge.svg)](https://docs.rs/hardmoney)
[![Book](https://github.com/cgorski/hardmoney/actions/workflows/book.yml/badge.svg)](https://cgorski.github.io/hardmoney/)

**U.S. campaign-finance data, done exactly.** A Rust library and CLI that
parses FEC electronic filings, loads the FEC's bulk downloads into
Postgres, and serves them over a REST API — with money that is never a
float, and every form type the FEC has published since 2001.

> **New here? Read [the hardmoney Book](https://cgorski.github.io/hardmoney/).**
> It has step-by-step tutorials for real jobs — *who is funding this
> candidate?*, *load a full election cycle and keep it fresh*, *track super
> PAC spending against a candidate* — written for people who have never
> touched FEC data, and a full reference for people who have.

## Why this exists

Every federal candidate, party, and PAC must report who gave them money and
where it went. The FEC publishes that as raw `.fec` filings (a delimited
format that has changed eight times since 2001) and as multi-gigabyte bulk
files (which ship two different date formats and an undocumented extra
column). Getting from there to *"show me every donor over $1,000 to this
committee"* is harder than it should be. hardmoney makes it one command.

## What you get

| | |
|---|---|
| **Parser** | Every FEC electronic spec, 3.x → 8.5, including Form 1/2 registrations, F3Z consolidated reports, and Schedule I — forms most tools skip. Strict by default; lenient mode tells you exactly which lines it skipped and why. |
| **Typed views** | `ScheduleA`, `ScheduleB`, `ScheduleE`, `Form3XSummary` with exact `Decimal` money, real dates, and names that resolve across old and new spec formats. Table-checked: a Schedule A line can't masquerade as a Schedule E. |
| **Postgres ETL** | All ten FEC bulk files, streamed straight into `COPY`. Transactional replace-reload that mirrors the FEC's weekly file exactly (including deletions). Isolated **namespaces** so one database holds many cycles, snapshots, or investigations. Versioned migrations. |
| **REST API** | Candidates, committees, itemized contributions and disbursements, independent expenditures, and per-filing Schedule E — with API keys, CORS allow-lists, timeouts, and trigram-indexed search. |

**Where it stands out** (per a September 2026 survey of FEC tooling): the
only active parser covering pre-v6 filings; exact decimal money where others
leak floats; detects the six duplicate-field bugs still present in the
upstream column tables; handles F99 free-text blocks that crash other
parsers; the only crate on crates.io for FEC data; and the only project
bundling ETL + API in one binary. Built to run on Aurora PostgreSQL 18.

## Install

```bash
cargo install hardmoney            # CLI (needs Postgres for the ETL/API parts)
```

```toml
# Library — parser only, no Postgres/Axum/Tokio:
hardmoney = { version = "1", default-features = false, features = ["fetch"] }
```

Rust 1.94+. Postgres 14+ (tested on 18, Aurora's newest major).

## Sixty-second tour

**CLI — load 2026 candidates and their committees, then search:**

```bash
export DATABASE_URL=postgres://user@localhost/fec

hardmoney schema-init --schema cycle_2026
hardmoney bulk-load candidates --cycle 2026 --schema cycle_2026
hardmoney bulk-load committees --cycle 2026 --schema cycle_2026
hardmoney bulk-load schedule_a --cycle 2026 --schema cycle_2026 --limit 200000  # sample; drop --limit for all 2.2 GB
hardmoney serve --schema cycle_2026 --api-key my-secret &

# No curl needed: `query` runs the API's own code against the database.
hardmoney query candidates -q cooper --cycle 2026 --schema cycle_2026
hardmoney query contributions --committee C00913566 --min-amount 1000 --schema cycle_2026
```

Re-run any `bulk-load` next week: it replaces that cycle's rows in one
transaction, or skips entirely with `--if-changed` if the FEC's file hasn't
moved. Parse a single filing with `hardmoney parse filing.fec [--lenient]`.

**Rust — fetch a filing and sum its contributions, exactly:**

```rust
use hardmoney::{Filing, Form3XSummary, ScheduleA};
use rust_decimal::Decimal;

fn main() -> Result<(), hardmoney::FecError> {
    let filing = Filing::fetch(2011831)?; // straight from docquery.fec.gov
    println!("{} spec {} ({} lines)", filing.raw_form_type, filing.version, filing.lines.len());

    if let Ok(cover) = filing.summary.view::<Form3XSummary>() {
        println!("receipts this period: ${}", cover.total_receipts.unwrap_or_default());
    }

    let itemized: Decimal = filing
        .views::<ScheduleA>()                 // only Table::SchA lines, typed
        .filter_map(|a| a.contribution_amount)
        .sum();                               // exact to the cent, never f64
    println!("itemized Schedule A: ${itemized}");
    Ok(())
}
```

More: [`examples/`](./examples/) has four runnable programs, and the
[Book](https://cgorski.github.io/hardmoney/) has the full tutorials, CLI
reference, and REST API reference.

## Not yet

Streaming parse for multi-GB filings, CSV/Parquet/SQLite export,
committee-scoped filing discovery with amendment de-duplication, real-time
e-file watching, and Python bindings are the most-requested features not in
1.0. They're next.

## Development

```bash
cargo test --all-features                                   # 118 tests, no database needed
HARDMONEY_TEST_DATABASE_URL=postgres://user@localhost/t cargo test --all-features   # + Postgres integration
cargo clippy --all-features --all-targets -- -D warnings
```

CI runs fmt, clippy, docs (deny warnings), an MSRV build, the full suite
against Postgres 18, and `cargo package`. Publishing to crates.io is a
manual, gated workflow.

## License and provenance

`Apache-2.0 OR BSD-3-Clause`. See [`LICENSE-APACHE`](./LICENSE-APACHE),
[`LICENSE-BSD`](./LICENSE-BSD), and [`NOTICE`](./NOTICE).

Column-position tables derive from [`dwillis/fech-sources`](https://github.com/dwillis/fech-sources)
(Fech lineage, Apache-2.0) with six collision fixes documented in `NOTICE`;
`F2S.csv` is authored here. Parsing approach inspired by
[`newsdev/nyt-pyfec`](https://github.com/newsdev/nyt-pyfec) (Apache-2.0). Test
fixtures are real filings from the FEC's own document store and from
[`esonderegger/fecfile`](https://github.com/esonderegger/fecfile)'s bundled
data. Spec documentation: <https://www.fec.gov/help-candidates-and-committees/filing-reports/>.
