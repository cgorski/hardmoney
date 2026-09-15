# Parsing filings in Rust: from bytes to exact dollars

Who this is for: a Rust developer building their own pipeline (ingesting
filings into a system that isn't Postgres, running analysis in a
notebook-style binary, or embedding FEC parsing in a larger service) who
wants the parser without the database and web-server dependencies.

What you'll have at the end: a small crate with one `main.rs` that
fetches any filing by ID, prints its cover-page totals, buckets every
body line by table, sums Schedule A by who gave, reconciles that sum
against the cover page to the cent, and handles every error path with a
line number. Every snippet in this chapter was compiled and run from a
scratch crate while writing it, and the output shown is what it printed.

## The `Cargo.toml`

```toml
[package]
name = "fec_pipeline"
version = "0.1.0"
edition = "2024"

[dependencies]
hardmoney = { version = "2", default-features = false, features = ["fetch"] }
rust_decimal = "1"
```

Two things to notice:

- `default-features = false, features = ["fetch"]` gives you the parser
  and `Filing::fetch`, and nothing else: no `sqlx`, no `axum`, no
  `tokio`. (Drop `fetch` too if you only parse local files.) The default
  feature set pulls in the Postgres ETL and the REST server, which you
  don't want in a parser-only crate. The full table is in
  [Installation](./installation.md#feature-flags-reference).
- `rust_decimal` is listed directly because you'll name its `Decimal`
  type in your own code (`Decimal::ZERO`, type annotations). hardmoney
  re-exports nothing from it; depending on it yourself is the normal Rust
  arrangement and keeps the version you see identical to the one
  hardmoney uses.

The scratch crate that verified this chapter used
`hardmoney = { path = "/path/to/hardmoney", default-features = false, features = ["fetch"] }`
against the source tree; the `version = "2"` form is what you'd publish
with.

## The program

`src/main.rs`, complete:

```rust
use std::collections::BTreeMap;

use hardmoney::{EntityType, FecError, Filing, Form3XSummary, ParseOptions, ScheduleA, Table};
use rust_decimal::Decimal;

fn main() {
    let filing_id: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_011_834);

    if let Err(e) = run(filing_id) {
        report(&e);
        std::process::exit(1);
    }
}

fn run(filing_id: u64) -> Result<(), FecError> {
    // 1. Fetch the raw bytes from docquery.fec.gov.
    let bytes = Filing::fetch_bytes(filing_id)?;

    // 2. Parse leniently: an unknown line type is recorded, not fatal.
    let (filing, skipped) =
        Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT)?.into_parts();
    println!(
        "filing {filing_id}: {} (spec {}), {} body lines, {} skipped",
        filing.raw_form_type,
        filing.version,
        filing.lines.len(),
        skipped.len()
    );
    for s in &skipped {
        println!("  {s}");
    }

    // 3. Cover-page totals, if this is a Form 3X.
    let mut cover_receipts: Option<Decimal> = None;
    if filing.summary.table() == Table::F3X {
        match filing.summary.view::<Form3XSummary>() {
            Ok(cover) => {
                println!("committee:     {}", cover.committee_name.as_deref().unwrap_or("?"));
                println!(
                    "period:        {} to {}",
                    cover.coverage_from_date.map(|d| d.to_string()).unwrap_or_default(),
                    cover.coverage_through_date.map(|d| d.to_string()).unwrap_or_default()
                );
                println!("receipts:      ${}", cover.total_receipts.unwrap_or_default());
                println!("disbursements: ${}", cover.total_disbursements.unwrap_or_default());
                println!("cash on hand:  ${}", cover.cash_on_hand_close_of_period.unwrap_or_default());
                cover_receipts = cover.total_receipts;
            }
            Err(e) => eprintln!("no cover totals: {e}"),
        }
    }

    // 4. Every body line, bucketed by table.
    let mut by_table: BTreeMap<Table, usize> = BTreeMap::new();
    for line in &filing.lines {
        *by_table.entry(line.table()).or_insert(0) += 1;
    }
    for (table, n) in &by_table {
        let what = match table {
            Table::SchA => "itemized receipts",
            Table::SchB => "itemized disbursements",
            Table::SchE => "independent expenditures",
            Table::Text => "free-text memos",
            _ => "other",
        };
        println!("{table:>6} {n:>4}  {what}");
    }

    // 5. Schedule A, summed by who gave, in exact Decimal arithmetic.
    let mut by_entity: BTreeMap<&str, (usize, Decimal)> = BTreeMap::new();
    let mut memo_total = Decimal::ZERO;
    for a in filing.views::<ScheduleA>() {
        let amount = a.contribution_amount.unwrap_or_default();
        // Memo lines (memo_code "X") itemize money already counted on
        // another line (earmarks, joint-fundraising attributions), so
        // they are excluded from the committee's own totals.
        if a.memo_code.as_deref() == Some("X") {
            memo_total += amount;
            continue;
        }
        let who = match &a.entity_type {
            Some(EntityType::Individual) => "individuals",
            Some(EntityType::Pac) | Some(EntityType::Committee) | Some(EntityType::CandidateCommittee) => {
                "committees"
            }
            Some(EntityType::Organization) => "organizations",
            Some(EntityType::Other(_)) => "undocumented code",
            Some(_) => "other documented code",
            None => "no entity type",
        };
        let slot = by_entity.entry(who).or_insert((0, Decimal::ZERO));
        slot.0 += 1;
        slot.1 += amount;
    }
    let mut itemized = Decimal::ZERO;
    for (who, (n, total)) in &by_entity {
        println!("{who:<16} {n:>3} line(s)  ${total}");
        itemized += *total;
    }
    println!("{:<16} {:>3} line(s)  ${itemized}", "itemized total", by_entity.values().map(|(n, _)| n).sum::<usize>());
    println!("memo lines excluded: ${memo_total}");
    if let Some(cover) = cover_receipts {
        println!(
            "itemized == cover total receipts? {}",
            if itemized == cover { "yes" } else { "no (unitemized receipts or other lines)" }
        );
    }
    Ok(())
}

fn report(e: &FecError) {
    match e {
        FecError::ParserMissing { form_type, line_no, .. } => {
            eprintln!("unknown form type {form_type:?} at line {}", line_no.unwrap_or(0));
        }
        FecError::NoMatchingVersionBucket { table, version, .. } => {
            eprintln!("no column layout for {table} at spec version {version}");
        }
        FecError::UnterminatedTextBlock { line_no } => {
            eprintln!("[BEGINTEXT] opened at line {line_no} is never closed");
        }
        FecError::Fetch(err) => eprintln!("network: {err}"),
        other => match other.line_no() {
            Some(n) => eprintln!("line {n}: {other}"),
            None => eprintln!("{other}"),
        },
    }
}
```

Run it against the default filing, 2011834, a monthly Form 3X from a
PAC called Republican Majority Fund, chosen because its nine Schedule A
lines span five different entity types:

```bash
cargo run --quiet -- 2011834
```

```text
filing 2011834: F3XN (spec 8.5), 16 body lines, 0 skipped
committee:     REPUBLICAN MAJORITY FUND
period:        2026-08-01 to 2026-08-31
receipts:      $30408.30
disbursements: $22579.85
cash on hand:  $451291.56
SchA    9  itemized receipts
SchB    7  itemized disbursements
committees         5 line(s)  $30170.37
organizations      1 line(s)  $237.93
itemized total     6 line(s)  $30408.30
memo lines excluded: $10553.13
itemized == cover total receipts? yes
```

And against 2011821, an amended Form 3X from a super PAC, which has more
kinds of line:

```bash
cargo run --quiet -- 2011821
```

```text
filing 2011821: F3XA (spec 8.5), 26 body lines, 0 skipped
committee:     ABUNDANT FUTURE
period:        2026-05-14 to 2026-06-30
receipts:      $72000.00
disbursements: $427572.13
cash on hand:  $221991.82
SchA    5  itemized receipts
SchB    7  itemized disbursements
SchD    5  other
SchE    6  independent expenditures
TEXT    3  free-text memos
committees         1 line(s)  $20000.00
individuals        3 line(s)  $52000.00
itemized total     4 line(s)  $72000.00
memo lines excluded: $2000.00
itemized == cover total receipts? yes
```

Both reconcile to the cent. The rest of this chapter walks through the
five numbered steps and the error handler, and says what each API call
is doing.

## Step 1: fetch

`Filing::fetch_bytes(id)` downloads
`http://docquery.fec.gov/dcdev/posted/<id>.fec` (following the FEC's
redirect to HTTPS) and returns the raw bytes. There's also
`Filing::fetch(id)`, which fetches and parses strictly in one call,
convenient when you don't need lenient parsing or the bytes themselves.
Both need the `fetch` feature. A bad ID is an `FecError::Fetch`:

```bash
cargo run --quiet -- 1
```

```text
network: http status: 404
```

For a file on disk, `std::fs::read(path)?` gives you the same `Vec<u8>`
and everything after this step is identical.

## Step 2: parse leniently, and get the skips back

`Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT)` does two
things before parsing: decodes the bytes as UTF-8, falling back to
Windows-1252 if that fails (older filings contain such bytes in
free-text fields), and picks the delimiter and column layout from the
spec version in the header line. Then it parses every body line, and any
line it can't parse (an unknown form-type token, or a table with no
layout for this spec version) is recorded rather than fatal.

It returns a `Lenient<Filing>`, not a `Filing`. The only ways to get the
filing out are `into_parts()`, which hands you `(Filing,
Vec<SkippedLine>)`, and `into_strict()`, which gives you the `Filing`
only if nothing was skipped and `FecError::LinesSkipped` otherwise. You
can't accidentally ignore that lines were dropped; the `let (filing,
skipped)` above is the idiomatic shape. Each `SkippedLine` has
`line_no`, `raw_form_type`, and a `reason`, and implements `Display`
(`line 5: 'ZZZ' skipped (unknown form type)`).

If you'd rather a bad line fail the whole filing, use
`Filing::parse_bytes(&bytes)`. That's strict, and returns
`Result<Filing, FecError>` directly. Which to pick is the subject of
[Strict vs. lenient parsing](./strict-vs-lenient.md); for an ingestion
job, lenient-and-record is usually right.

## Step 3: cover totals with `Form3XSummary`

`filing.summary` is the form's cover line, a `ParsedLine` like every
body line, so it has a `table` and a `view()` method. `Form3XSummary` is
the typed view for a Form 3X cover: committee name, coverage dates as
`Option<NaiveDate>`, and the headline totals as `Option<Decimal>`. The
`table == Table::F3X` check first means the view is only attempted on a
form it applies to; on any other form you still have every raw field via
`filing.summary.get("field_name")`.

`view()` returns `Result<Form3XSummary, TypedViewError>`, and
`TypedViewError` doesn't convert into `FecError` with `?`. They're
different failure domains (a filing that parsed fine but whose cover line
lacks a required field, versus a filing that didn't parse). The `match`
handles it inline; in a crate with its own error enum you'd add a
`From` impl.

## Step 4: bucketing lines by `Table`

Every `ParsedLine` carries a `Table` (an enum, not a string) saying
which format table parsed it. Counting into a `BTreeMap<Table, usize>`
works because `Table` is `Ord`, and the `match` that labels each bucket
needs a `_` arm because `Table` is `#[non_exhaustive]`: the FEC adds
record types, and the crate reserves the right to add variants in a minor
release. `SchD` (debts) fell into `"other"` in the second run; that's
what the wildcard is for. `Table` implements `Display` as its
name (`SchA`, `TEXT`), which is what the `{table:>6}` printed.

If you want the raw lines of one table rather than a count,
`filing.lines_for(Table::SchE)` is an iterator of `&ParsedLine`, and
`line.get("expenditure_amount")` reads any field as `Option<&str>`.

## Step 5: summing Schedule A by `EntityType`

`filing.views::<ScheduleA>()` iterates only the `Table::SchA` lines,
converted to the `ScheduleA` typed view: `contributor_name` resolved
from whichever name fields this spec version uses, `contribution_amount`
as an exact `Option<Decimal>`, `contribution_date` as
`Option<NaiveDate>`, and `entity_type` as `Option<EntityType>`. The enum
has a variant per documented FEC code (`Individual` for `IND`, `Pac`,
`Committee`, `CandidateCommittee`, `Organization`, `Candidate`, `Party`)
and `Other(String)` preserving anything undocumented verbatim. Real
filings contain typos and vendor extensions, and the view keeps the line
rather than rejecting it. Like `Table`, it's `#[non_exhaustive]`, hence
the `Some(_)` arm.

The `memo_code` check is FEC accounting, not a hardmoney concept, but
it's the difference between the totals matching and not: a Schedule A
line with memo code `X` itemizes money that is also reported on another
line. In 2011834, a $10,170.37 net transfer from a joint fundraising
committee is one line, and three memo lines attribute the gross
$10,553.13 that donors gave through that JFC to the individuals (the
$382.76 difference is the JFC's deducted costs). Add both and you
overstate receipts by $10,553.13; exclude the memo lines and the itemized
total equals the cover page exactly.

## Why not `f64`

That `==` on the last line is the reason for `Decimal`. Binary floating
point can't represent most decimal fractions, so sums of ordinary dollar
amounts drift by tiny amounts that make exact comparisons unreliable and
accumulate across a large file; `rust_decimal::Decimal` stores an integer
and a power-of-ten scale, so `11282.23` is exactly `11282.23` and a sum of
cents is exactly right. Two real amounts from filing 2011834 make the
point without any contrivance:

```rust
use hardmoney::parse_money;
use rust_decimal::Decimal;

fn main() {
    // Two real Schedule A amounts from filing 2011834 (lines 6 and 11).
    let as_f64 = "10170.37".parse::<f64>().unwrap() + "237.93".parse::<f64>().unwrap();
    let as_decimal: Decimal = parse_money("10170.37").unwrap() + parse_money("237.93").unwrap();
    println!("f64:     {as_f64:?}");
    println!("Decimal: {as_decimal}");
}
```

```text
f64:     10408.300000000001
Decimal: 10408.30
```

`hardmoney::parse_money` is the function every typed view uses for
amounts. It builds the `Decimal` straight from the digit string with no
`f64` step, tolerates a bare integer (`"1000"`) or one decimal place, and
returns `None` for blank input, more than two decimal places (almost
always an upstream column-alignment bug), or anything that would
overflow. It never panics. Details in
[Tables and typed views](./typed-views.md#exact-money-with-rust_decimaldecimal).

## The error handler

`report` matches on `FecError`, which is `#[non_exhaustive]`. The
`other` arm is required, and it's also where `FecError::line_no()` earns
its place. Three variants carry a physical 1-based line number
(`ParserMissing`, `NoMatchingVersionBucket`, `UnterminatedTextBlock`);
`line_no()` returns it as `Option<u64>` for any variant, `None` for
errors that aren't about one line (a bad header, a network failure), so
the fallback arm can print a useful location without enumerating
variants.

To exercise the line-number path you need a broken filing. This second
binary parses a local file both ways. It's also where the
`ScheduleE`/`SupportOppose` types from the
[independent-expenditures tutorial](./tutorial-independent-expenditures.md)
show up in library form:

```rust
// src/bin/strict.rs
use hardmoney::{FecError, Filing, ParseOptions, ScheduleE, SupportOppose};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).expect("usage: strict <file.fec>");
    let bytes = std::fs::read(&path)?;

    // Strict: refuse the whole filing if any body line is unparseable.
    match Filing::parse_bytes(&bytes) {
        Ok(filing) => println!("strict: {} body lines", filing.lines.len()),
        Err(FecError::ParserMissing { form_type, line_no, .. }) => {
            println!("strict: unknown form type {form_type:?} at line {line_no:?}");
        }
        Err(e) => println!("strict: {e} (line {:?})", e.line_no()),
    }

    // Lenient: keep what parses, and get the skipped lines back explicitly.
    let (filing, skipped) = Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT)?.into_parts();
    println!("lenient: kept {}, skipped {}", filing.lines.len(), skipped.len());
    for s in &skipped {
        println!("  {s}");
    }

    for e in filing.views::<ScheduleE>() {
        let verb = match e.support_oppose {
            Some(SupportOppose::Support) => "for",
            Some(SupportOppose::Oppose) => "against",
            _ => "(no S/O code)",
        };
        println!(
            "  ${} {} {} on {:?}",
            e.expenditure_amount.unwrap_or_default(),
            verb,
            e.candidate_name.as_deref().unwrap_or("?"),
            e.dissemination_date
        );
    }
    Ok(())
}
```

Make a broken file from a real one (body fields are separated by ASCII
28, which `printf` writes as `\x1c`) and run it:

```bash
cp /path/to/hardmoney/tests/fixtures/F24N_2011823.fec /tmp/with_junk.fec
printf 'ZZZ\x1cthis line type does not exist\n' >> /tmp/with_junk.fec
cargo run --quiet --bin strict -- /tmp/with_junk.fec
```

```text
strict: unknown form type "ZZZ" at line Some(4)
lenient: kept 1, skipped 1
  line 4: 'ZZZ' skipped (unknown form type)
  $1074900.00 against SHERROD BROWN on Some(2026-09-14)
```

Line 4 is right: header, cover line, one Schedule E line, then the junk.
The strict parse refused the file and said where; the lenient parse kept
the one good line and reported the skip; and the Schedule E view read
the one real independent expenditure with its exact amount.

## Where to go next

- The rest of the typed layer (`ScheduleB`, name resolution across
  spec versions, `TypedViewError`, writing your own `TypedView`) is in
  [Tables and typed views](./typed-views.md).
- The raw layer (`ParsedLine`, `Header`, spec-version handling) is in
  [Parsing a filing, explained](./parsing-explained.md); the schema
  behind it (`SpecVersion`, `Layout`, and the compile-time-checked
  `Typed<T>` field access) is in [The schema](./library-schema.md).
- What the parser does and does not change about a field value (values
  are preserved verbatim, apart from trimming and one pair of wrapping
  quotes) is in [Fidelity](./fidelity.md).
- The three things you can do with a parsed filing besides read it:
  check it against the FEC's acceptance rules
  ([Validating a filing](./validating.md)), check its cover page against
  its schedules ([Reconciling a filing](./reconciling.md)), and write it
  back out ([Writing `.fec` files](./writing-fec.md)).
- If the filing is a 135 MB presidential report, `Filing::parse_bytes`
  needs 1.3 GB; [Streaming large filings](./streaming.md) shows the same
  Schedule A total in 10 MB with `FilingReader`.
- If your pipeline's destination is Postgres after all, enable the
  `bulk` feature and see the library section of
  [Loading bulk data](./bulk-etl.md#doing-this-from-rust-instead-of-the-cli);
  `ingest_filing_bytes` stores a parsed filing in two lines.
- The bundled `examples/` directory (`parse_filing`, `fetch_live_filing`,
  `typed_schedule_a_totals`, `handle_parse_errors`) all build under the
  same minimal feature set used here.
