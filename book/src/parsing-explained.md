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
strings -- `hardmoney`'s own test suite includes a real 2001 Merck PAC
Form 3X filing under spec 3.00 (2,837 real body lines), real spec 5.1 and
5.3 filings, a real spec 6.1 filing (the first ASCII-28 era), and 17
current spec-8.5 filings across every major form type.

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
   correct header/body parsing rules.
2. Reads the summary line into `filing.summary`, an
   `IndexMap<String, String>` keyed by canonical field name (e.g.
   `"col_a_total_receipts"`).
3. Reads every remaining line, looks up which schedule/table it belongs
   to (`filing.lines[i].table`, e.g. `"SchA"`, `"SchE"`), and builds a
   `ParsedLine` with its own `IndexMap<String, String>` of canonical
   field names to raw string values.

Every value at this layer is still a raw string -- `"11282.23"`, not a
number; `"20260912"`, not a date. That's intentional: this layer's job is
to faithfully reproduce the wire format with human-readable field names,
nothing more. Turning those strings into real `Decimal` amounts,
`NaiveDate` values, and correctly-resolved names is what the **typed
views** layer does, covered next in
[Working with Money, Dates, and Names](./typed-views.md).

## A real bug this design caught: the field-name collision fix

The FEC's own filing-spec documentation only defines column *positions*
per form and version -- human-readable field names are a convenience
layer that parsing libraries add on top (`hardmoney` included, following
the precedent set by `nyt-pyfec` and `fech`). While building `hardmoney`,
an audit of the vendored format tables found that six of them -- **F2,
F3P, F3X, F4, SchC1, SchL** -- assigned the *same* canonical name to two
genuinely different column positions within a single spec-version bucket.

Because the line parser builds an ordered `name -> value` map per line, an
unresolved collision meant the second-occurring field silently overwrote
the first, discarding real data. This wasn't a hypothetical: in
`tests/fixtures/F3XN_2011834.fec`, two colliding "total contribution
refunds" fields held genuinely different values (5000.00 vs 0.00, and
20000.00 vs 5000.00) before the fix -- one was quietly being thrown away.

All six tables were corrected with distinct, form-accurate names, and two
permanent safeguards now guard against a regression ever passing silently
again:

- `Line::from_csv_str` returns a hard `FecError::DuplicateCanonicalField`
  error instead of silently overwriting, if any future edit (including an
  upstream format-table update) reintroduces a same-bucket collision.
- `tests/format_table_integrity.rs` scans every bundled format table on
  every test run and fails if any collision exists at all.

Full details, including every renamed field and its FEC form-line
justification, are documented in the repository's
[`NOTICE`](https://github.com/cgorski/hardmoney/blob/main/NOTICE) file.
