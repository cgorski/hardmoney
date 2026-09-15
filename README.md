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
| **Parser** | Every FEC electronic spec, 3.x → 8.5, including Form 1/2 registrations, F3Z consolidated reports, and Schedule I — forms most tools skip. Strict by default; lenient mode tells you exactly which lines it skipped and why. Field values come back **verbatim** (trimmed, nothing else); codes are interpreted case-insensitively. Streaming `Filing::open` / `FilingReader` for 135 MB presidential filings in constant memory. |
| **Schema as data** | The FEC's column positions for every spec version and its field specifications (type, length, required level, rule text) are compiled into the crate: `SpecVersion`, `Layout`, `FieldSpec`. Generated `Field<T>` constants (`sch_a::CONTRIBUTION_AMOUNT`) and `Typed<T>` views make asking an F3X cover page for a Schedule A field a **compile error**. `hardmoney spec` exports or diffs it. |
| **Validator** | `hardmoney validate` checks a filing against the FEC's acceptance rules — required fields, types and lengths, real dates, amount formats, ID formats, legal characters, unique transaction IDs, back-references — WebCheck-style, one finding per line, exit 1 if the FEC would reject it. |
| **Typed views** | `ScheduleA`, `ScheduleB`, `ScheduleE`, `Form3XSummary` with exact `Decimal` money, real dates, and names that resolve across old and new spec formats. Table-checked: a Schedule A line can't masquerade as a Schedule E. |
| **Postgres ETL** | All ten FEC bulk files, streamed straight into `COPY`. Transactional replace-reload that mirrors the FEC's weekly file exactly (including deletions). Isolated **namespaces** so one database holds many cycles, snapshots, or investigations. Versioned migrations. |
| **REST API + `query`** | Candidates, committees, itemized contributions and disbursements, independent expenditures, and per-filing Schedule E — with API keys, CORS allow-lists, timeouts, trigram-indexed search, and out-of-range `limit`/`offset` answered with a 400 rather than silently clamped. `hardmoney query` runs the same routes from the terminal, no server or `curl` needed. |

Also arriving in the 2.0 release: a `Filing::to_fec` writer (the exact
inverse of parsing, so a filing can be edited and written back out) and
`Filing::reconcile`, which checks a cover page's totals against its
schedules to the cent.

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
hardmoney = { version = "2", default-features = false, features = ["fetch"] }
```

Rust 1.94+. Postgres 18 (Aurora PostgreSQL's newest major); older majors may work but are untested.

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
moved.

**CLI — one filing, no database:**

```bash
hardmoney parse filing.fec --lenient        # JSON summary; which lines were skipped and why
hardmoney validate filing.fec               # the FEC's acceptance rules, one finding per line
hardmoney spec fields SchA --version 6.4    # where every Schedule A field lived in spec 6.4
hardmoney spec diff 7.0 8.5                 # what moved between two spec versions
hardmoney spec export > fec-spec.json       # the whole format, as data
```

**Rust — fetch a filing and sum its contributions, exactly:**

```rust
use hardmoney::parser::tables::{f3x, markers::F3X};
use hardmoney::{Filing, ScheduleA, Table};
use rust_decimal::Decimal;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let filing = Filing::fetch(2011831)?; // straight from docquery.fec.gov
    println!("{} spec {} ({} lines)", filing.raw_form_type, filing.version, filing.lines.len());

    // Compile-time-checked: only F3X fields are accepted for an F3X cover page.
    if let Ok(cover) = filing.summary_as::<F3X>() {
        println!("receipts this period: {:?}", cover.money(f3x::COL_A_TOTAL_RECEIPTS));
    }

    // Untyped, by canonical field name, for genuinely dynamic access.
    for line in filing.lines_for(Table::SchA).take(3) {
        println!("line {}: {:?}", line.line_no, line.get("contributor_last_name"));
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

## Upgrading from 1.x

2.0 preserves field values as filed (1.x upper-cased them and stripped
`& < > " \ |`), so text output changes; codes are still interpreted
case-insensitively and `raw_form_type` is still upper-cased. API renames:
`filing.headers` → `filing.header` (a typed `Header`), `line.table` →
`line.table()`, and the per-line `fields` map → `line.get(name)` /
`line.iter()`. The book's [Fidelity](https://cgorski.github.io/hardmoney/fidelity.html)
and [Schema](https://cgorski.github.io/hardmoney/library-schema.html)
chapters cover both.

## Not yet

CSV/Parquet/SQLite export, committee-scoped filing discovery with
amendment de-duplication, real-time e-file watching, and Python bindings
are the most-requested features not in 2.0. They're next.

## Development

```bash
cargo test --all-features                                   # no database needed
HARDMONEY_TEST_DATABASE_URL=postgres://user@localhost/t cargo test --all-features   # + Postgres integration
cargo clippy --all-features --all-targets -- -D warnings
```

See [`CONTRIBUTING.md`](./CONTRIBUTING.md) for the rules every change is
held to (no panics in library code, types over conventions, format facts
in `data/` not in Rust, docs that match code).

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
