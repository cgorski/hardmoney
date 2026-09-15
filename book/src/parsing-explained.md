# Parsing a Filing, Explained

This chapter walks through what's actually inside a `.fec` file and how
`hardmoney` turns it into structured data, in plain English.

## The shape of a `.fec` file

A `.fec` file is plain text with one filing per file, laid out as:

1. **A header line** -- identifies the filing software, the FEC "spec
   version" this file was written in, and (for amendments) which prior
   filing this one supersedes.
2. **A summary line** -- the form's cover-page fields: who's filing, what
   period this report covers, and the period's headline totals (total
   receipts, total disbursements, cash on hand).
3. **Zero or more body lines** -- one line per itemized transaction:
   individual contributions (Schedule A), disbursements (Schedule B),
   independent expenditures (Schedule E), debts, loans, and so on,
   depending on what the filer had to report that period.

Here's a real header and one real body line, straight from a bundled
fixture (`tests/fixtures/F24N_2011832.fec`), byte for byte:

```text
HDR^\FEC^\8.5^\NGP^\8^\^\^\
F24N^\C00865444^\48^\^\WinSenate^\1032 15th St NW^\Ste 247^\Washington^\DC^\20005^\Lambe^\Rebecca^\^\^\^\20260914
SE^\C00865444^\500196509^\^\^\ORG^\Declaration Media LLC^\...^\11282.23^\...^\S6OH00304^\Husted^\Jon^\...
```

The `^\` you see between every value is not a typo -- it's ASCII character
28 (the "file separator" control character), rendered here as its
caret-backslash escape because it doesn't print visibly in a terminal.
That single detail is the first thing that trips people up when they try
to open a `.fec` file in a text editor or a naive CSV parser: it looks
like garbled text unless you know to split on byte `0x1C`, not a comma or
a tab.

## Why the format isn't consistent across files

The FEC's electronic filing format has changed several times since
electronic filing began, and `hardmoney` has to detect and handle each
era correctly:

| Spec version | Delimiter | What it looks like |
|---|---|---|
| 3.x, 4.x | comma, double-quoted CSV | `"HDR","FEC","3.00","KNOWLEDGE","XP.1108","","FEC-24088","1",""` |
| 5.x | comma, unquoted | `HDR,FEC,5.1,FECWEB,5.1.1.0,,,` |
| 6.x, 7.x, 8.x (current, through 8.5) | ASCII character 28 | `HDR^\FEC^\8.5^\NGP^\8^\^\^\` (shown above with the visible escape) |

Those aren't just cosmetic differences -- the *number and order of fields*
in the header and in every schedule's body lines also changed between
these eras, sometimes multiple times within one era. A filing written
under spec 5.1 places its columns in different byte positions than the
same schedule under spec 8.5, even though both use human-readable
field names like "contribution amount." A parser that hard-codes column
positions for only the current spec will silently misread or crash on
older filings -- something that matters a lot if you're doing historical
research, not just watching current filings come in.

`hardmoney` handles this with two pieces working together:

- `header::parse` looks at the version string in the very first line and
  picks the correct header field layout (the "old" 8-field layout for
  3.x/4.x/5.x, or the "new" 7-field layout for 6.x and later).
- Every schedule's column-position data is itself *version-bucketed* --
  for example, the Form 3X format table defines separate column-position
  buckets for `8.5|8.4|...|6.1`, for `5.3|5.2|5.1|5.0`, and for `3`, so a
  "total receipts" field resolves to the correct byte position no matter
  which era wrote the file.

This is verified against real historical filings, not just synthetic test
strings -- `hardmoney`'s own test suite (`tests/real_filings.rs`) parses
a real 2001 Merck PAC Form 3X under spec 3.00 (2,837 real body lines),
real spec 5.1 and 5.3 filings, a real spec 6.1 filing (the first ASCII-28
era), a real spec 8.0 amended Form 3 with 641 body lines, and 20 current
spec-8.5 filings across every major form type. Every fixture is also
parsed a second time in lenient mode and must produce an identical result
with zero skipped lines.

## From bytes to structured data

Parsing a filing is one function call:

```rust
use hardmoney::Filing;

let bytes = std::fs::read("tests/fixtures/F24N_2011832.fec")?;
let filing = Filing::parse_bytes(&bytes)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`parse_bytes` first decodes the raw bytes as UTF-8, falling back to
Windows-1252 if that fails (some older filer-entered free text, like a
contributor's name or memo, was written in that encoding) -- this is why
it's `parse_bytes` and not just `parse` on a `&str`: you often don't know
the encoding until you've tried. It then:

1. Reads the header line to determine the spec version and pick the
   correct header/body parsing rules. The result is `filing.header` (a
   typed `Header`: software name and version, report id, the raw
   version string) and `filing.version` (a `SpecVersion`, e.g. `8.5`).
2. Reads the summary line into `filing.summary`. This is a `ParsedLine`
   -- the same type as every body line -- so `filing.summary.table()`
   tells you which form it is (`Table::F24` here), and
   `filing.summary.get("committee_name")` returns `Option<&str>`.
   `filing.summary.iter()` walks every `(field, value)` pair in column
   order.
3. Reads every remaining line, looks up which schedule or sub-form table
   it belongs to from its first column (`SE` -> `Table::SchE`, `SA11AI` ->
   `Table::SchA`, `F3ZT` -> `Table::F3Z`, ...), parses it with that
   table's column positions for this filing's spec version, and pushes a
   `ParsedLine` onto `filing.lines`.

A `ParsedLine` has two public fields and a small set of accessors:

| | Type | Meaning |
|---|---|---|
| `raw_form_type` | `String` | column 0, upper-cased, e.g. `"SE"`, `"SA11AI"`, `"SC/10"` (the `form_type` *field* keeps it as filed -- see [Fidelity](./fidelity.md)) |
| `line_no` | `u64` | the 1-based physical line number in the file |
| `table()` | `Table` | which format table parsed it -- an **enum**, not a string |
| `layout()` | `&Layout` | the version-bucketed column layout it was parsed with |
| `get(name)` | `Option<&str>` | one field's value; `Some("")` if blank, `None` if this version has no such field |
| `iter()` | `(&str, &str)` pairs | every canonical field name and raw value, in column order |

The full API -- including `set`, `from_pairs`, and the compile-time-checked
`typed::<T>()` view -- is in [The Schema](./library-schema.md#parsedline-get-iter-set-from_pairs).

Here's what the two Schedule E body lines of the fixture above look like
after parsing. This is the real output of
`hardmoney parse --lines tests/fixtures/F24N_2011832.fec`, trimmed to a
handful of the 44 fields each line carries:

```json
{
  "raw_form_type": "SE",
  "table": "SchE",
  "line_no": 3,
  "fields": {
    "filer_committee_id_number": "C00865444",
    "transaction_id_number": "500196509",
    "entity_type": "ORG",
    "payee_organization_name": "DECLARATION MEDIA LLC",
    "expenditure_amount": "11282.23",
    "dissemination_date": "20260912",
    "disbursement_date": "",
    "support_oppose_code": "O",
    "candidate_id_number": "S6OH00304",
    "candidate_last_name": "HUSTED",
    "candidate_first_name": "JON",
    "candidate_office": "S",
    "candidate_state": "OH",
    "expenditure_purpose_descrip": "MEDIA PRODUCTION - ESTIMATE"
  }
}
```

(`line_no` is 3 because line 1 is the header and line 2 is the summary.
In JSON the `Table` serializes as its name, `"SchE"`; in Rust it's
`Table::SchE`.)

Because `table()` is an enum, filtering and matching on it is exact and
checked by the compiler -- there is no way to typo `"ScheE"` and silently
match nothing:

```rust
use hardmoney::{Filing, Table};

let bytes = std::fs::read("tests/fixtures/F24N_2011832.fec")?;
let filing = Filing::parse_bytes(&bytes)?;

// Just the lines of one table:
for line in filing.lines_for(Table::SchE) {
    println!(
        "line {}: {} -> {:?}",
        line.line_no,
        line.raw_form_type,
        line.get("expenditure_amount")
    );
}

// Or dispatch on the table yourself:
for line in &filing.lines {
    match line.table() {
        Table::SchA => println!("contribution"),
        Table::SchB => println!("disbursement"),
        Table::SchE => println!("independent expenditure"),
        other => println!("other: {other}"), // Table implements Display
    }
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
line 3: SE -> Some("11282.23")
line 4: SE -> Some("4555.33")
independent expenditure
independent expenditure
```

`Table` is `#[non_exhaustive]` (the FEC adds record types over time), so
a `match` on it always needs a wildcard arm like `other` above.
`Table::as_str()` gives the string name (`"SchE"`) and
`"SchE".parse::<Table>()` goes the other way; `Table::ALL` lists every
variant.

Every value at this layer is still a raw string -- `"11282.23"`, not a
number; `"20260912"`, not a date; `"O"`, not "oppose". That's intentional:
this layer's job is to faithfully reproduce the wire format with
human-readable field names, nothing more. Turning those strings into real
`Decimal` amounts, `NaiveDate` values, enums, and correctly-resolved
names is what the **typed views** layer does, covered in
[Tables and Typed Views](./typed-views.md).

## Which forms and schedules are covered

`hardmoney::Table` has 59 variants -- one per bundled format table. They
fall into three groups:

- **Top-level forms** that can appear on a filing's summary line: `F1`,
  `F1M`, `F2`, `F3`, `F3X`, `F3P`, `F3L`, `F4`, `F5`, `F6`, `F7`, `F8`,
  `F9`, `F10`, `F13`, `F24`, `F99`. The list of base form types the crate
  processes end to end is `hardmoney::parser::form::ALLOWED_TOP_LEVEL_FORMS`,
  checked by `Filing::is_allowed()`.
- **Sub-forms** that appear as body lines under one of those: `F1S`,
  `F2S`, `F3S`, `F3Z`/`F3Z1`/`F3Z2`, `F3PS`, `F3P31`/`F3PZ1`/`F3PZ2`,
  `F56`/`F57`, `F65`, `F76`, `F82`/`F83` (Form 8 parts II and III),
  `F91`-`F94`, `F105`, `F132`/`F133`, and the `H1`-`H6` allocation
  schedules.
- **Schedules**: `SchA`, `SchA3L`, `SchB`, `SchC`, `SchC1`, `SchC2`,
  `SchD`, `SchE`, `SchF`, `SchI`, `SchL`, plus `Text` (free-text `TEXT`
  records) and `Hdr` (the header, parsed separately).

Body-line dispatch is a list of regular expressions tried in order
against column 0, in `src/parser/form.rs`. Order matters -- `SA3L` has to
be tried before `SA`, `F3Z1` before `F3Z`, `F1S` and `F1M` before `F1` --
and the first match wins.

Twenty of those tokens (`F1`, `F1S`, `F1M`, `F2`, `F2S`, `F3Z`, `F3ZT`,
`F3Z1`, `F3Z2`, `F3P31`, `F3PZ1`, `F3PZ2`, `F8`, `F8II`, `F82`, `F8III`,
`F83`, `F10`, `F105`, `SI`) have format tables in the shared
`fech-sources` lineage that other parsers built on the same data commonly
leave unrouted. Some of them matter more than their obscurity suggests:

- **Form 1 / 1S / 1M and Form 2 / 2S** (committee and candidate
  registrations). These are about 26% of daily volume on the FEC's live
  feed, so a parser without them refuses a quarter of everything filed.
  `F2S.csv` is authored locally; the upstream `fech-sources` project has
  no table for it. Three real fixtures cover these: `F1A_2011905.fec`,
  `F1MN_2011755.fec`, and `F2A_2011896.fec`.
- **F3Z / F3Z1 / F3Z2 and F3P31 / F3PZ1 / F3PZ2** (consolidated
  candidate-committee sub-forms). An amended Form 3 typically carries a
  handful of `F3Z` lines among hundreds of Schedule A lines; under strict
  parsing, one unrouted token would refuse the whole filing and lose
  *every* body line with it. The real fixture `F3A_767339_v8.0.fec` has
  three:

  ```bash
  $ cargo run --quiet --bin hardmoney -- parse tests/fixtures/F3A_767339_v8.0.fec
  ```

  ```json
  {
    "amends_filing": "467627",
    "base_form_type": "F3",
    "form_type": "F3A",
    "is_amendment": true,
    "line_count": 641,
    "lines_by_table": {
      "F3Z": 3,
      "SchA": 576,
      "SchB": 60,
      "SchD": 2
    },
    "skipped_count": 0,
    "version": "8.0"
  }
  ```

  (Trimmed.) All 641 body lines parse, under a spec version -- 8.0 --
  that no other fixture covers.
- **Form 8 / 8II / 8III and Form 10 / 10.5** (historical debt-settlement
  and personal-funds forms). The on-the-wire tokens are `F8II`/`F8III`
  while the table names are `F82`/`F83`; both are accepted.
- **Schedule I** (Levin funds), spec 3.x through 8.4. The FEC dropped it
  in 8.5.

If a body line's form type matches nothing, strict parsing fails the
whole filing with `FecError::ParserMissing { form_type, version, line_no }`
-- and the line number tells you exactly where to look. The next chapter
covers what to do instead when you'd rather keep going.

## A real bug this design caught: the field-name collision fix

The FEC's own filing-spec documentation only defines column *positions*
per form and version -- human-readable field names are a convenience
layer that parsing libraries add on top (`hardmoney` included, following
the precedent set by `nyt-pyfec` and `fech`). While building `hardmoney`,
an audit of the vendored format tables found that six of them -- **F2,
F3P, F3X, F4, SchC1, SchL** -- assigned the *same* canonical name to two
genuinely different column positions within a single spec-version bucket.

Because a layout maps each canonical name to one value slot per line, an
unresolved collision would mean the second-occurring field silently
overwrote the first, discarding real data. This isn't a hypothetical: in
`tests/fixtures/F3XN_2011834.fec`, the two colliding "total contribution
refunds" fields hold genuinely different values (5000.00 vs 0.00, and
20000.00 vs 5000.00), so one of them would have been quietly thrown away.

All six tables were corrected with distinct, form-accurate names, and two
permanent safeguards guard against a regression ever passing silently:

- `build.rs` refuses to generate the tables at all if any future edit
  (including an upstream format-table update) reintroduces a same-bucket
  collision -- two fields at one column or one field at two columns is a
  build failure, not a runtime surprise.
- `tests/format_table_integrity.rs` scans every bundled format table on
  every test run and fails if any collision exists at all.

Full details, including every renamed field and its FEC form-line
justification, are documented in the repository's
[`NOTICE`](https://github.com/cgorski/hardmoney/blob/main/NOTICE) file.
