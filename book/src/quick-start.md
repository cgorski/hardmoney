# Quick start

This chapter parses a real filing two ways, as a CLI command and as a
five-line Rust program, so you can see the shape of the data before
getting into how the parser works.

## Parsing a filing from the command line

The repository ships with real, bundled `.fec` fixture files under
`tests/fixtures/` (sourced from the FEC's own document store and, for one
historical fixture, a real 2001 Merck PAC filing; see
[Sources and provenance](https://github.com/cgorski/hardmoney#sources-and-provenance)
in the README). Point the CLI at one:

```bash
$ cargo run --quiet --bin hardmoney -- parse tests/fixtures/F24N_2011832.fec
```

```json
{
  "amends_filing": null,
  "base_form_type": "F24",
  "form_type": "F24N",
  "header": {
    "ef_type": "FEC",
    "fec_version": "8.5",
    "record_type": "HDR",
    "report_id": "",
    "report_number": "",
    "soft_name": "NGP",
    "soft_ver": "8"
  },
  "is_amendment": false,
  "line_count": 2,
  "lines_by_table": {
    "SchE": 2
  },
  "skipped": [],
  "skipped_count": 0,
  "summary": {
    "city": "WASHINGTON",
    "committee_name": "WINSENATE",
    "date_signed": "20260914",
    "filer_committee_id_number": "C00865444",
    "form_type": "F24N",
    "original_amendment_date": "",
    "report_type": "48",
    "state": "DC",
    "street_1": "1032 15TH ST NW",
    "street_2": "STE 247",
    "treasurer_first_name": "REBECCA",
    "treasurer_last_name": "LAMBE",
    "treasurer_middle_name": "",
    "treasurer_prefix": "",
    "treasurer_suffix": "",
    "zip_code": "20005"
  },
  "version": "8.5"
}
```

This is Form 24 (an independent expenditure notice) filed by committee
`C00865444` ("WINSENATE"). Reading the fields from the top:

- `form_type` is the exact form variant as written in the file (`F24N`;
  an "N" suffix means "new", i.e. not an amendment); `base_form_type`
  strips that suffix down to the form family (`F24`) if you want to group
  F24N/F24A/F24T together.
- `is_amendment` and `amends_filing` go together: for an amendment
  (`F24A`, `F3XA`, ...), `amends_filing` holds the filing number of the
  report being amended, recovered from the header. It's `null` here
  because this is an original.
- `line_count` says there are 2 itemized body lines beyond the
  header/summary, and `lines_by_table` breaks that down by which table
  (schedule or sub-form) each line was parsed with: here, 2 Schedule E
  (independent expenditure) line items. You'll see them parsed
  in full in [Parsing a filing, explained](./parsing-explained.md).
- `skipped_count` and `skipped` are both empty because every body line
  parsed. They matter when you pass `--lenient`; see
  [Strict vs. lenient parsing](./strict-vs-lenient.md).

Try a different fixture to see a real Form 3X (a PAC's periodic financial
summary, with itemized receipts and disbursements):

```bash
$ cargo run --quiet --bin hardmoney -- parse tests/fixtures/F3XN_2011834.fec
```

```json
{
  "amends_filing": null,
  "base_form_type": "F3X",
  "form_type": "F3XN",
  "is_amendment": false,
  "line_count": 16,
  "lines_by_table": {
    "SchA": 9,
    "SchB": 7
  },
  "skipped_count": 0,
  "summary": {
    "committee_name": "REPUBLICAN MAJORITY FUND",
    "col_a_total_receipts": "30408.30",
    "col_a_total_disbursements": "22579.85"
  },
  "version": "8.5"
}
```

(The JSON above is trimmed to the fields discussed here. Run the
command yourself to see the full output, including the header and every
summary field. Add `--lines` to also dump every body line with every
field; it's large.)

And a registration form, an amended Form 2 (candidate registration),
whose body lines are `F2S` records listing the candidate's authorized
committees:

```bash
$ cargo run --quiet --bin hardmoney -- parse tests/fixtures/F2A_2011896.fec
```

```json
{
  "amends_filing": "1890108",
  "base_form_type": "F2",
  "form_type": "F2A",
  "is_amendment": true,
  "line_count": 3,
  "lines_by_table": {
    "F2S": 3
  },
  "summary": {
    "candidate_id_number": "S6IL00458",
    "candidate_last_name": "STRATTON",
    "candidate_first_name": "JULIANA",
    "candidate_office": "S",
    "candidate_state": "IL",
    "election_year": "2026"
  },
  "version": "8.5"
}
```

(Also trimmed.) `is_amendment` is `true` and `amends_filing` is
`"1890108"`: this filing supersedes FEC filing number 1890108.

## Parsing a filing as a Rust program

The same Form 3X, parsed as a five-line library call
(from [`examples/parse_filing.rs`](https://github.com/cgorski/hardmoney/blob/main/examples/parse_filing.rs)):

```rust
use hardmoney::Filing;

let bytes = std::fs::read("tests/fixtures/F3XN_2011834.fec")?;
let filing = Filing::parse_bytes(&bytes)?;

println!("form: {} (spec version {})", filing.raw_form_type, filing.version);
println!("total receipts: {:?}", filing.summary.get("col_a_total_receipts"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

Run the bundled example, which does exactly this plus a few more fields:

```bash
$ cargo run --quiet --example parse_filing -- tests/fixtures/F3XN_2011834.fec
```

```text
form:          F3XN (base: F3X)
spec version:  8.5
is amendment:  false
body lines:    16
summary.committee_name               Some("REPUBLICAN MAJORITY FUND")
summary.col_a_total_receipts         Some("30408.30")
summary.col_a_total_disbursements    Some("22579.85")
lines by table: {SchA: 9, SchB: 7}
```

Two things about that output:

- `col_a_total_receipts` is a raw `&str` (`"30408.30"`), not a number.
  That's what the raw parser layer (`filing.summary`, `filing.lines`)
  hands back for every field: the value as it appears on the wire.
- `lines by table` prints `{SchA: 9, SchB: 7}` without quotes around
  `SchA`, because it's a map keyed by `hardmoney::Table`, an enum, not
  by a string. Every parsed line carries a `Table` saying which
  schedule/form it belongs to.

The typed views layer covered in [Tables and typed views](./typed-views.md)
converts those raw strings into an exact numeric type, real dates, and
resolved names. Run
`cargo run --example typed_schedule_a_totals -- tests/fixtures/F3XN_2011834.fec`
for a preview:

```text
9 itemized Schedule A contributions
total: $40961.43
largest: $10170.37 from ONE TEAM SENATE MAJORITY on Some(2026-08-05)
```

## Fetching a filing live from the FEC (optional)

If you have the `fetch` feature enabled (on by default) and network
access, you can skip having a local `.fec` file at all and pull one
straight from `docquery.fec.gov` by its filing ID:

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

This downloads the filing, parses it, and prints a `Form3XSummary`,
covered in full in [Tables and typed views](./typed-views.md).

## Where to go next

- Want to understand exactly what's inside a `.fec` file and how the
  parser handles decades of format changes? Read
  [Parsing a filing, explained](./parsing-explained.md).
- Want to know what happens when one line of a filing is unparseable, and
  how to keep going anyway? Read
  [Strict vs. lenient parsing](./strict-vs-lenient.md).
- Want exact money math and real dates instead of raw strings? Read
  [Tables and typed views](./typed-views.md).
- Want a queryable database of every candidate, committee, and
  contribution across all filers? Read
  [Loading bulk data into Postgres](./bulk-etl.md).
