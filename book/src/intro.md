# Introduction

## What is this book?

This is a tutorial for **hardmoney** 2.0, a Rust crate (library and
command-line tool) for working with United States Federal Election
Commission (FEC) campaign-finance data. It's written for people who have
never touched FEC data before and want to understand, step by step, what
the tool does and why -- with real commands and real output at every
step, not made-up examples.

If you already know Rust and just want the API surface, the
[crate documentation on docs.rs](https://docs.rs/hardmoney) and the
[README](https://github.com/cgorski/hardmoney#readme) are faster references.
This book is for the parts in between: what the data actually looks like,
why certain design decisions were made, and how to go from "I have a
`.fec` file" or "I want a searchable database of committees" to a working
program.

If you'd rather start from a concrete goal than from the concepts, the
[Tutorials](./tutorials.md) section has five end-to-end walkthroughs --
each written for one kind of reader (a reporter, a research team, a
watchdog, a Rust developer, a data engineer) with real commands and real
output from start to finish.

## What is FEC data, in plain English?

Every political campaign committee, party committee, and PAC (political
action committee) registered with the FEC has to periodically tell the
government, in a standardized electronic format, who gave them money, who
they spent money on, and how much cash they have on hand. Those reports are
called **filings**. A single filing is one `.fec` text file: a header line
identifying the filer and report, a summary line with the period's totals,
and then hundreds or thousands of itemized line items -- one line per
contribution received, one line per bill paid, and so on.

The FEC also publishes two other kinds of data derived from these filings:

- **Bulk data files** -- large pre-aggregated CSV-like downloads (one per
  election cycle) of candidates, committees, individual contributions,
  operating expenditures, and independent expenditures across *all*
  filers, not just one committee. These are what most journalists and
  researchers actually want, because they cover everyone at once.
- **A live document store** (`docquery.fec.gov`) where you can fetch any
  individual filing by its filing ID, as soon as it's submitted -- often
  within hours, well before it shows up in the next bulk-data refresh.

`hardmoney` handles all three: it can parse a single raw `.fec` filing
(from disk or fetched live), it can load the FEC's own bulk-data downloads
into a normal Postgres database, and it can serve that database over a
REST API. You can use any one of these three things independently, or all
three together. Around the parser, 2.0 adds the rest of what a filing
needs: a writer that turns a parsed filing back into a `.fec` file, the
FEC's own acceptance rules (`hardmoney validate`), a reconciler that
checks a report's cover-page totals against its schedules to the cent
(`hardmoney reconcile`), and a streaming reader for filings too large to
hold in memory.

## Why does this tool exist?

FEC data is messier than it looks at first. The electronic filing format
has changed several times since electronic filing began -- what's called
a filing's "spec version" -- and each version changed the delimiter
character, the header layout, and sometimes the column positions within a
schedule. A parser that only handles the current spec version will fail,
silently misread fields, or crash on older real-world filings. `hardmoney`
was built to handle every spec version the FEC has used, verified against
real historical filings, not just synthetic test strings.

The second reason is coverage of the *forms*. Committee and candidate
registrations -- Form 1, Form 1M, and Form 2 -- are roughly 26% of the
FEC's daily filing volume, and a parser that only routes the
money-reporting forms refuses all of them. `hardmoney` dispatches those,
the F3Z/F3P consolidated sub-forms, Form 8, Form 10, and Schedule I, along
with every schedule and form the format tables describe (see
[Parsing a Filing, Explained](./parsing-explained.md#which-forms-and-schedules-are-covered)).

The third reason is money correctness. Money amounts in FEC filings are
decimal currency values (dollars and cents). Representing them as
floating-point numbers (`f64`) risks the well-known problem that binary
floating point cannot represent every decimal fraction exactly -- `0.1 +
0.2` famously doesn't equal `0.3` in IEEE 754 arithmetic. Across millions
of transactions, small representation errors can accumulate. `hardmoney`
avoids this entirely by using
[`rust_decimal::Decimal`](https://docs.rs/rust_decimal), a fixed-point
decimal type, everywhere a money value flows through the crate: parsing,
the typed views, the Postgres schema (`NUMERIC` columns, not
floating-point ones), and the REST API's JSON responses. A contribution
amount is never round-tripped through `f64` at any layer.

## Who is this for?

- Someone who has a `.fec` file (maybe downloaded from `docquery.fec.gov`,
  or received from a client) and needs to read it programmatically.
- Someone who wants a local, queryable database of FEC bulk data instead
  of re-downloading and re-parsing giant CSV files every time -- and who
  wants to keep several independent copies (this cycle, last cycle, a
  snapshot from last week) in one database without them colliding.
- Someone building a small internal tool or public-facing site on top of
  campaign-finance data and wants a REST API rather than direct database
  access.
- Someone curious how a real-world data-format parser handles decades of
  format drift, or how to represent money correctly in Rust.

## How this book is organized

Each chapter builds on the last, but you can also jump straight to the one
that matches what you're trying to do:

1. [Installation](./installation.md) -- get `hardmoney` building.
2. [Quick Start](./quick-start.md) -- parse your first filing in under a
   minute.
3. [Tutorials](./tutorials.md) -- five goal-driven walkthroughs: funding
   a candidate, loading a full cycle, tracking independent expenditures,
   building a Rust pipeline, and what the tool does with messy FEC data.
4. [Parsing a Filing, Explained](./parsing-explained.md) -- what's inside
   a `.fec` file and how the parser turns it into structured data.
   - [Strict vs. Lenient Parsing](./strict-vs-lenient.md) -- what happens
     when one line of a filing can't be parsed, and how to choose.
   - [Fidelity](./fidelity.md) -- exactly what the parser does and does
     not change about a field value, and how that differs from 1.x.
   - [Streaming Large Filings](./streaming.md) -- `FilingReader` and
     `Filing::open`: one pass over a 135 MB filing in 10 MB of memory.
5. [Tables and Typed Views](./typed-views.md) -- the `Table` enum, the
   `view()`/`views()` typed layer, and why money, dates, and names need
   special handling.
6. [The Schema](./library-schema.md) -- `SpecVersion`, per-version
   `Layout`s, the FEC's `FieldSpec` rows, and compile-time-checked field
   access with `Field<T>` and `Typed<T>`.
7. [Writing `.fec` Files](./writing-fec.md) -- `Filing::to_fec` and
   `hardmoney write`: the exact inverse of parsing, and how to edit a
   filing and write it back.
8. [Reconciling a Filing](./reconciling.md) -- does the cover page
   agree with the schedules? `Filing::reconcile`, the $200 itemization
   threshold, and a real filing that is $200 off.
9. [Validating a Filing](./validating.md) -- the FEC's acceptance rules
   as `Filing::validate` and `hardmoney validate`: the 32 rules, where
   they come from, and where hardmoney deliberately differs.
10. [Loading Bulk Data into Postgres](./bulk-etl.md) -- go from "the FEC's
    own bulk downloads" to a normalized, migrated, queryable schema.
    - [Namespaces](./namespaces.md) -- many isolated sessions in one
      database.
    - [Reloading](./reloading.md) -- `--mode replace`, `--mode append`,
      `--if-changed`, and the `loads` table.
    - [Dates](./dates.md) -- the FEC's two date formats and the raw/parsed
      twin columns.
11. [The REST API](./rest-api.md) -- serve that database over HTTP, with
    real request/response examples.
    - [Hardening the API](./api-hardening.md) -- API keys, CORS, timeouts,
      and what error responses look like.
12. [CLI Reference](./cli-reference.md) -- every subcommand, with options,
    including `write`, `reconcile`, `validate`, `query`, and `spec`.
13. [Troubleshooting & FAQ](./troubleshooting.md).

Every command and every piece of output shown in this book was actually
run against hardmoney (the chapters new in 2.0 -- Fidelity, Streaming,
The Schema, Writing, Reconciling, Validating, and the
`write`/`reconcile`/`validate`/`query`/`spec` sections of the CLI
Reference -- against 2.0.0; the rest against the 1.0.0 release they were
written for, with the API surface updated where 2.0 changed it) --
against the crate's own bundled test fixtures, a local Postgres 18
database, or (where noted) a live network call to `docquery.fec.gov` or
`fec.gov` -- while writing it. The handful
of outputs that could not be reproduced on demand (for example, a
confirmation prompt that only fires when a million rows are about to be
deleted) are explicitly marked *illustrative*.
