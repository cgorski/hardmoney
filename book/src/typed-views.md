# Tables and typed views

The raw parser layer (`filing.summary`, `filing.lines`) hands back every
field as a plain string, as it appears on the wire. In practice you
almost always want four things converted: money amounts, dates, names,
and the FEC's one-to-three-letter codes. You also want a guarantee that
the "Schedule A contribution" you're summing came from a Schedule A
line. That's what the typed-view layer in `hardmoney::parser::typed`
(re-exported at the crate root) does. This chapter explains how to use
it and why it's built the way it is.

## Three ways in

There are four typed views (`ScheduleA`, `ScheduleB`, `ScheduleE`, and
`Form3XSummary`) and three ways to reach them, all built on the `Table`
enum introduced in [the previous chapter](./parsing-explained.md#from-bytes-to-structured-data):

| You have... | Call | You get |
|---|---|---|
| a whole `Filing` and want every line of one schedule, typed | `filing.views::<ScheduleA>()` | an iterator of `ScheduleA` |
| one `ParsedLine` | `line.view::<ScheduleA>()` | `Result<ScheduleA, TypedViewError>` |
| a whole `Filing` and want the raw lines of one table | `filing.lines_for(Table::SchA)` | an iterator of `&ParsedLine` |

The first one is the one you'll use most. Summing every itemized
contribution in a filing is three lines:

```rust
use hardmoney::{Filing, ScheduleA};
use rust_decimal::Decimal;

let bytes = std::fs::read("tests/fixtures/F3A_2011812.fec")?;
let filing = Filing::parse_bytes(&bytes)?;

let total: Decimal = filing
    .views::<ScheduleA>()
    .filter_map(|a| a.contribution_amount)
    .sum();
println!("total: ${total}");
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
total: $106909.58
```

`views::<V>()` is `lines_for(V::TABLE)` followed by a conversion of each
line, dropping any line that fails to convert. For a line of the right
table, the only way conversion fails is if a required field (the
filer's committee ID) is blank. Every other field degrades to `None`
rather than failing the row, because real-world FEC data is routinely
incomplete on optional fields. If you want to see why a particular line
was dropped, use the second form:

```rust
use hardmoney::{Filing, ScheduleA, Table, TypedViewError};

let bytes = std::fs::read("tests/fixtures/F3A_2011812.fec")?;
let filing = Filing::parse_bytes(&bytes)?;

for line in filing.lines_for(Table::SchA) {
    match line.view::<ScheduleA>() {
        Ok(a) => println!("{:?} gave {:?}", a.contributor_name, a.contribution_amount),
        Err(TypedViewError::MissingField { field, line_no, .. }) => {
            eprintln!("line {line_no:?}: missing required field {field}");
        }
        Err(e) => eprintln!("{e}"),
    }
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

The bundled example
[`examples/typed_schedule_a_totals.rs`](https://github.com/cgorski/hardmoney/blob/main/examples/typed_schedule_a_totals.rs)
does the sum plus a "largest contribution" search:

```bash
$ cargo run --quiet --example typed_schedule_a_totals
```

```text
249 itemized Schedule A contributions
total: $106909.58
largest: $20000.00 from BERT K MIZUSAWA on Some(2026-06-11)
```

## Views check the table

Every typed view declares which `Table` it reads (`ScheduleA::TABLE` is
`Table::SchA`, `ScheduleE::TABLE` is `Table::SchE`, and so on), and
`view()` refuses a line from any other table:

```rust
use hardmoney::{Filing, ScheduleE, Table, TypedViewError};

let bytes = std::fs::read("tests/fixtures/F3A_2011812.fec")?;
let filing = Filing::parse_bytes(&bytes)?;

let sched_a_line = filing.lines_for(Table::SchA).next().unwrap();
let err = sched_a_line.view::<ScheduleE>().unwrap_err();
assert!(matches!(err, TypedViewError::WrongTable { .. }));
println!("{err}");
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
line 3 is a SchA line, not SchE
```

The check matters because the only field a `ScheduleE` strictly
requires is `filer_committee_id_number`, and every schedule carries
that. A conversion that looked only at the field map would not fail on a
Schedule A line: it would "succeed" and produce a phantom independent
expenditure with a filer ID and `None` for everything else. Run over
`tests/fixtures/F3A_2011812.fec`, that would misfile all 587 of its
non-Schedule-E lines as empty Schedule E rows. With the table check, that
class of bug is impossible: a `ScheduleE` can only come from a
`Table::SchE` line, and `views::<ScheduleE>()` on a filing with no
independent expenditures yields nothing.

## Why not parse the strings yourself?

You could. `filing.summary.get("col_a_total_receipts")` gives you
`Some("30408.30")`, and `"30408.30".parse::<f64>()` "works." But four
problems show up quickly.

`f64` can't represent all decimal currency exactly. Explained in full in
the next section.

Dates aren't always present, and "blank" isn't always blank. Real
filings routinely have empty date fields or an all-zeros date
(`"00000000"`) instead of a missing value. Both should mean "no date,"
but a naive `chrono::NaiveDate::parse_from_str` call treats the second
one as a parse error, not a `None`.

The "name" field changed shape. Spec versions before 8.0 report a single
combined name field (e.g. `contributor_name`). Spec 8.0 and later, which
nearly every current electronic filing uses, drops that field in favor
of an `*_organization_name` field or split
`*_first_name`/`*_middle_name`/`*_last_name` fields. Code that only
checks the old field name silently gets `None` for every present-day
filing.

Codes have documented values and undocumented ones. The entity type is
supposed to be one of seven three-letter codes; real filings contain
typos, vendor extensions, and stray whitespace. Rejecting the line loses
data; keeping the raw string means every caller re-derives "is this an
individual?" by hand.

The typed views handle all four.

## Exact money with `rust_decimal::Decimal`

Money fields across every typed view (`ScheduleA.contribution_amount`,
`ScheduleB`/`ScheduleE.expenditure_amount`, and every dollar field on
`Form3XSummary`) are `Option<rust_decimal::Decimal>`, never `f64`.

Here's the concrete problem `Decimal` avoids. Binary floating point
represents numbers as sums of powers of two, and most decimal fractions
(0.1, 0.2, 0.30, ordinary money amounts) don't have an exact binary
representation, the same way 1/3 doesn't have a terminating decimal
representation. This is why, in almost any language using IEEE 754
floats, `0.1 + 0.2 == 0.3` is `false`: the stored value is
`0.30000000000000004`. For a single value that's usually invisible, but
sum enough of them and the accumulated rounding error can become visible
in a total. `hardmoney`'s test suite includes a test
(`sums_many_amounts_with_zero_drift` in `src/parser/typed.rs`) that
builds up a running total from many parsed amounts and asserts the sum is
exactly correct, which is a guarantee `f64` summation cannot make in
general.

`rust_decimal::Decimal` avoids the problem by storing a value as an
integer plus a power-of-ten scale (fixed-point decimal), so `11282.23` is
represented exactly, not approximately. `hardmoney::parse_money` builds
it directly from the digits of the amount string via
`Decimal::new(total_cents, 2)`, with no `f64` intermediate step anywhere
in the conversion, and it fails closed on anything suspicious:

```rust
use hardmoney::parse_money;
use rust_decimal::Decimal;

assert_eq!(parse_money("11282.23"), Some(Decimal::new(1128223, 2)));
assert_eq!(parse_money("1000"),     Some(Decimal::new(100000, 2)));  // bare integer is fine
assert_eq!(parse_money("-75"),      Some(Decimal::new(-7500, 2)));   // so are negatives
assert_eq!(parse_money("1.234"),    None); // three decimals: almost certainly a column-alignment bug upstream
assert_eq!(parse_money("99999999999999999999"), None); // would overflow: None, not a panic
```

That last line matters: the cents arithmetic is checked, so no amount
field in any filing, however malformed, can make the parser panic.

The same `Decimal` type is what gets written to and read back from
Postgres `NUMERIC` columns in the [bulk-ETL layer](./bulk-etl.md), and
what the [REST API](./rest-api.md) serializes into JSON. A contribution
amount is never round-tripped through `f64` at any point from raw filing
bytes to the JSON response a client receives. You can see this in the
API chapter, where a real query returns `"expenditure_amt":"11282.23"`
as an exact JSON string, not a floating-point number.

## Real dates, not string-shaped placeholders

`hardmoney::parse_fec_date` converts the FEC's `YYYYMMDD` string fields
into `chrono::NaiveDate`. It requires exactly eight digits and treats a
blank string and an all-zero string (`"00000000"`) as `None` rather than
a parse error. Both show up routinely in real filings (an unset date,
versus a date field that's present but zeroed out by the filing
software), and callers almost always want to treat them the same way.

```rust
use chrono::NaiveDate;
use hardmoney::parse_fec_date;

assert_eq!(parse_fec_date("20260912"), NaiveDate::from_ymd_opt(2026, 9, 12));
assert_eq!(parse_fec_date("00000000"), None);
assert_eq!(parse_fec_date(""), None);
assert_eq!(parse_fec_date("2026091"), None);   // seven digits: not a date
assert_eq!(parse_fec_date("202609121"), None); // nine digits: not a date either
```

(A looser parse, anything `%Y%m%d` can make sense of, would let a
truncated or over-long field come back as a plausible-looking wrong
date.) This is the filing date format; the FEC's bulk files use two
different ones, covered in [Dates](./dates.md).

## Names that resolve correctly regardless of filing era

Each typed view's name field (`contributor_name` on `ScheduleA`,
`payee_name` on `ScheduleB`/`ScheduleE`, `candidate_name` on `ScheduleE`)
checks both possible shapes: the old combined field first, and if that's
absent, the modern organization-name field or the split
prefix/first/middle/last/suffix fields, joined together. The same field
resolves correctly whether you're reading a filing from 2001 or one
filed this year; you don't need to know or check the filing's spec
version yourself. In the Schedule E line shown in the previous
chapter, `candidate_last_name: "HUSTED"` and `candidate_first_name: "JON"`
become `candidate_name: Some("JON HUSTED")`.

## Codes as enums, with the unknowns preserved

`ScheduleA.entity_type` and `ScheduleB.entity_type` are
`Option<EntityType>`; `ScheduleE.support_oppose` is
`Option<SupportOppose>`. Both enums have a variant per documented FEC code
and an `Other(String)` variant that keeps anything else verbatim:

| `EntityType` | code | | `SupportOppose` | code |
|---|---|---|---|---|
| `Individual` | `IND` | | `Support` | `S` |
| `Organization` | `ORG` | | `Oppose` | `O` |
| `Candidate` | `CAN` | | `Other(String)` | as filed |
| `CandidateCommittee` | `CCM` | | | |
| `Committee` | `COM` | | | |
| `Pac` | `PAC` | | | |
| `Party` | `PTY` | | | |
| `Other(String)` | as filed | | | |

Parsing is case-insensitive and trims whitespace. `.code()` gives the FEC
string back, `.is_known()` is false for `Other`, and both implement
`Display` as their code. `ScheduleE::support_oppose_code()` is a
shortcut for the raw `Option<&str>`; it's what the bulk ingester stores
in the `support_oppose_code` column.

```rust
use hardmoney::{EntityType, Filing, ScheduleA, ScheduleE, SupportOppose};

let bytes = std::fs::read("tests/fixtures/F3A_2011812.fec")?;
let filing = Filing::parse_bytes(&bytes)?;

for a in filing.views::<ScheduleA>() {
    match &a.entity_type {
        Some(EntityType::Individual) => { /* a person */ }
        Some(EntityType::Pac) | Some(EntityType::Committee) => { /* another committee */ }
        Some(EntityType::Other(code)) => eprintln!("undocumented entity code {code:?}"),
        Some(other) => println!("{}", other.code()),
        None => { /* field blank */ }
    }
}

for e in filing.views::<ScheduleE>() {
    match e.support_oppose {
        Some(SupportOppose::Support) => {}
        Some(SupportOppose::Oppose) => {}
        Some(SupportOppose::Other(ref code)) => eprintln!("odd support/oppose code {code:?}"),
        Some(_) => {} // both enums are #[non_exhaustive]: a wildcard arm is required
        None => {}
    }
    let raw: Option<&str> = e.support_oppose_code();
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

Like `Table`, both enums are `#[non_exhaustive]`, so a `match` needs a
wildcard arm even after listing every current variant.

## A real error type, not a panic

`TypedViewError` has two variants, both carrying enough context to find
the line in a large filing:

- `WrongTable { expected, found, line_no }`: the line belongs to a
  different table than the view reads (shown above).
- `MissingField { table, field, line_no }`: a field the view requires
  (the filer's committee ID) is absent or blank.

Every other field degrades to `None` instead of failing the whole
conversion. `TypedViewError` is `#[non_exhaustive]`.

Behind all four views is one trait, `hardmoney::TypedView`, with a
`TABLE` constant and a `from_line` method. You won't normally call it
directly (`view()` and `views()` are the intended surface), but it's
public, so you can write your own view over a table the crate doesn't
cover and use it with the same two methods.

For a table nobody has written a view for, you don't have to: every
table has a compile-time-checked `Typed<T>` view with generated field
constants (`line.typed::<H4>()?.money(h4::TOTAL_AMOUNT)`), covered in
[The schema](./library-schema.md#typed_-t-the-compiler-checks-the-table).

## The summary line is a `ParsedLine` too: `Form3XSummary`

`filing.summary` has the same type as a body line, so it has the same
`view()` method. `Form3XSummary` reads a Form 3X cover line into exact
`Decimal` totals and real coverage dates:

```rust
use hardmoney::{Filing, Form3XSummary, Table};

let bytes = std::fs::read("tests/fixtures/F3XN_2011834.fec")?;
let filing = Filing::parse_bytes(&bytes)?;

if filing.summary.table() == Table::F3X {
    let s = filing.summary.view::<Form3XSummary>()?;
    println!("receipts this period: ${}", s.total_receipts.unwrap_or_default());
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

Combining everything in this chapter, here's that view applied to a
filing fetched live over the network (requires the `fetch` feature, on by
default) rather than from a bundled fixture:

```bash
$ cargo run --quiet --example fetch_live_filing -- 2011831
```

```text
fetching filing 2011831 from docquery.fec.gov ...
F3XN (F3X), spec 8.5
committee:            Some("FIRSTENERGY CORP POLITICAL ACTION COMMITTEE")
receipts this period: $15245.52
disbursements period: $10023.01
cash on hand (close): $1985550.44
```

Every dollar figure above came from `Form3XSummary`'s `Decimal` fields,
printed directly with `{}`. `Decimal`'s `Display` implementation prints
exactly two decimal places for a value built from cents, so there's no
`{:.2}` formatting (and no rounding) anywhere in this example.
