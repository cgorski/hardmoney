# Working with Money, Dates, and Names

The raw parser layer (`filing.summary`, `filing.lines`) hands back every
field as a plain `String`, faithful to the wire format. In practice you
almost always want three things converted: money amounts, dates, and
names. That's what the typed-view layer in `hardmoney::parser::typed`
(also re-exported at the crate root) does. This chapter explains each
conversion and why it's built the way it is.

## Why not just parse the string yourself?

You could -- `filing.summary.get("col_a_total_receipts")` gives you
`Some("30408.30")`, and `"30408.30".parse::<f64>()` "works." But three
real problems show up quickly:

1. **`f64` can't represent all decimal currency exactly.** This is
   explained in full in the next section.
2. **Dates aren't always present, and "blank" isn't always blank.** Real
   filings routinely have empty date fields or an all-zeros date
   (`"00000000"`) instead of a genuinely missing value -- both should mean
   "no date," but a naive `chrono::NaiveDate::parse_from_str` call treats
   the second one as a parse error, not a `None`.
3. **The "name" field genuinely changed shape.** Spec versions before 8.0
   report a single combined name field (e.g. `contributor_name`). Spec 8.0
   and later -- what virtually every current electronic filing uses --
   drops that field entirely in favor of an `*_organization_name` field or
   split `*_first_name`/`*_middle_name`/`*_last_name` fields. Code that
   only checks the old field name silently gets `None` for every
   present-day filing.

The typed views handle all three so you don't have to re-solve them.

## Exact money with `rust_decimal::Decimal`

Money fields across every typed view (`ScheduleA.contribution_amount`,
`ScheduleB`/`ScheduleE.expenditure_amount`, and every dollar field on
`Form3XSummary`) are `Option<rust_decimal::Decimal>`, never `f64`.

Here's the concrete problem `Decimal` avoids. Binary floating point
represents numbers as sums of powers of two, and most decimal fractions
(0.1, 0.2, 0.30 -- ordinary money amounts) don't have an exact binary
representation, the same way 1/3 doesn't have a terminating decimal
representation. This is why, in almost any language using IEEE 754
floats, `0.1 + 0.2 == 0.3` is `false` -- the actual stored value is
something like `0.30000000000000004`. For a single value that's usually
invisible, but sum enough of them and the accumulated rounding error can
become visible in a total. `hardmoney`'s test suite includes a test
(`sums_many_amounts_with_zero_drift` in `src/parser/typed.rs`) that
builds up a running total from many parsed amounts and asserts the sum is
*exactly* correct -- not "close enough" -- which is a guarantee `f64`
summation cannot make in general.

`rust_decimal::Decimal` avoids the problem by storing a value as an
integer plus a power-of-ten scale (fixed-point decimal), so `11282.23` is
represented exactly, not approximately. The parser builds it directly
from the two integer pieces of an amount string (`whole` and `frac`) via
`Decimal::new(total_cents, 2)` -- there's no `f64` intermediate step
anywhere in the conversion.

This isn't just a parser-level guarantee. The same `Decimal` type is what
gets written to and read back from Postgres `NUMERIC` columns in the
[bulk-ETL layer](./bulk-etl.md), and what the [REST API](./rest-api.md)
serializes into JSON. A contribution amount is never round-tripped
through `f64` at any point from raw filing bytes to the JSON response a
client receives -- you can see this directly in the API chapter, where a
real query returns `"expenditure_amt":"11282.23"` as an exact JSON
string, not a floating-point number.

```rust
use hardmoney::{Filing, ScheduleA};
use rust_decimal::Decimal;

let bytes = std::fs::read("tests/fixtures/F3A_2011812.fec")?;
let filing = Filing::parse_bytes(&bytes)?;

let mut total = Decimal::ZERO;
for line in filing.lines.iter().filter(|l| l.table == "SchA") {
    let row: ScheduleA = line.try_into()?;
    if let Some(amount) = row.contribution_amount {
        total += amount; // exact decimal addition, no drift
    }
}
println!("total: ${total}");
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Real dates, not string-shaped placeholders

`parse_fec_date` converts the FEC's `YYYYMMDD` string fields into
`chrono::NaiveDate`, and specifically treats a blank string *and* an
all-zero string (`"00000000"`) as `None` rather than a parse error --
both show up routinely in real filings (an unset date, versus a date
field that's present but zeroed out by the filing software), and callers
almost always want to treat them the same way.

## Names that resolve correctly regardless of filing era

Each typed view's name field (`contributor_name` on `ScheduleA`,
`payee_name` on `ScheduleB`/`ScheduleE`, `candidate_name` on `ScheduleE`)
checks both possible shapes: the old combined field first, and if that's
absent, the modern organization-name field or the split
first/middle/last fields, joined together. This means the same field
resolves correctly whether you're reading a filing from 2001 or one filed
this year -- you don't need to know or check the filing's spec version
yourself.

## A real error type, not a panic

`TryFrom<&ParsedLine>` returns `Result<_, TypedViewError>`, not a panic,
if a field that's genuinely required for the row to make sense (the
filer's committee ID) is missing. Every other field degrades gracefully
to `None` instead of failing the whole conversion, since real-world FEC
data is routinely incomplete on optional fields -- a `.unwrap()` on every
field would make the typed views nearly unusable on real data.
[`examples/handle_parse_errors.rs`](https://github.com/cgorski/hardmoney/blob/main/examples/handle_parse_errors.rs)
demonstrates matching on this error type directly.

## Putting it together: a live-fetched filing

Combining all three typed conversions, here's `Form3XSummary` applied to
a filing fetched live over the network (requires the `fetch` feature, on
by default) rather than from a bundled fixture:

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
printed directly with `{}` -- `Decimal`'s `Display` implementation
already prints exactly two decimal places for a value built from cents,
so there's no `{:.2}` formatting (and no rounding) needed anywhere in this
example.
