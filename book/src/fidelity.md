# Fidelity: What the Parser Does and Does Not Change

A parser for a government filing format has one job before all others:
give back what the filer sent. This chapter states exactly what hardmoney
2.0 does to a field value between the bytes on disk and the `&str` you
get from `line.get(..)`, because 1.x did more than it should have, and
the difference matters if you compare a 1.x pipeline's output with a 2.0
one.

## The 2.0 rule: verbatim, except trimming

Every field value is preserved **byte for byte, except that leading and
trailing ASCII whitespace is trimmed**. That is the whole rule.

- Trimmed: space, tab, CR, LF, form feed, vertical tab -- at the ends of
  the value only. The FEC's own ingestion trims these, and several
  historical filing vendors padded fields to fixed widths, so keeping
  them would be keeping noise.
- Not trimmed: anything inside the value, and any non-ASCII whitespace
  (a non-breaking space in a name is data, not padding).
- Not touched: case, punctuation, quotes, angle brackets, ampersands,
  backslashes, pipes -- anything at all. `AT&T` stays `AT&T`;
  `O'Brien "Bob" <b>` stays exactly that.

```rust
use hardmoney::Filing;

// A Schedule A line at spec 8.5 is 45 cells; fill in the ones we care
// about (columns are 0-based here, as in `Layout`).
let mut cells = vec![""; 45];
cells[0] = "sa11ai";
cells[1] = "C00123456";
cells[5] = "ORG";
cells[6] = " AT&T \"Services\" <Inc> ";
cells[42] = "x";
let text = format!(
    "HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF3XN\u{1c}C00123456\n{}",
    cells.join("\u{1c}")
);
let filing = Filing::parse(&text)?;
let line = &filing.lines[0];

assert_eq!(line.raw_form_type, "SA11AI");            // upper-cased for dispatch
assert_eq!(line.get("form_type"), Some("sa11ai"));   // as filed
assert_eq!(
    line.get("contributor_organization_name"),
    Some("AT&T \"Services\" <Inc>")                  // trimmed, nothing else touched
);
assert_eq!(line.get("memo_code"), Some("x"));
assert!(line.is_memo());                              // codes compare case-insensitively
assert_eq!(filing.raw_form_type, "F3XN");
# Ok::<(), Box<dyn std::error::Error>>(())
```

## What 1.x did instead

hardmoney 1.x inherited its field cleaning from `nyt-pyfec`'s
`clean_entry`/`utf8_clean`, which:

- **upper-cased every field**, so `Smith, Jane` became `SMITH, JANE` and
  a filer's own capitalisation of a committee name was lost;
- **deleted the characters `&`, `<`, `>`, `"`, and `\`**, so `AT&T`
  became `ATT` and `Barnes & Noble` became `Barnes  Noble`;
- **turned `|` into `,`**.

Those were reasonable defaults for a 2012 newsroom loading data into a
search index. They are wrong for a reference implementation: they made
it impossible to write a filing back out byte-for-byte, they changed
legal names, and they hid from downstream code the fact that the FEC's
validator rejects some of those characters in the first place
(`hardmoney validate` now reports an illegal character instead of
silently removing it).

If you have 1.x-era output and want to compare, the 1.x behaviour is
reproducible from a 2.0 value: upper-case it, delete `&<>"\`, and
replace `|` with `,`. The reverse is not possible, which is the point.

## Codes are compared case-insensitively

Preserving case raises an obvious question: real filings contain
`SB21b`, `x` in a memo-code column, `ind` as an entity type. The FEC's
spec defines these tokens as case-insensitive, so **every place
hardmoney interprets a code does so case-insensitively** -- at the point
of interpretation, leaving the stored value alone:

| What | How it is interpreted |
|---|---|
| form-type tokens (`SA11AI`, `sb21b`, `F3XN`) | dispatch to a `Table` is case-insensitive |
| `memo_code` | `ParsedLine::is_memo()` is true for `X` or `x` (trimmed) |
| entity types, support/oppose codes | `EntityType`/`SupportOppose` parse `IND`/`ind`/` Ind ` to the same variant; `Other(String)` keeps the raw spelling |
| spec versions | `8.5`, `8.5.0.1`, `P3.4`, `p3.4` -- see [`SpecVersion`](./library-schema.md#specversion) |

So `line.get("memo_code")` may return `Some("x")`, and `line.is_memo()`
is how you ask the question.

## `raw_form_type` vs. the `form_type` field

There is one deliberate exception to "as filed," and it is a *second
copy*, not a change. Every `ParsedLine` (and the `Filing` itself) has a
`raw_form_type: String` that is the column-0 token **upper-cased**:
`"SA11AI"`, `"SB21B"`, `"F3XN"`. It exists so that code matching on the
token -- `if line.raw_form_type == "SB21B"` -- works regardless of how
the filer capitalised it, and it is what the JSON output and the bulk
loader use.

The `form_type` *field* (`line.get("form_type")`) keeps the token exactly
as filed, `"sb21b"` and all. That is what the writer emits, so a
round-tripped filing is byte-identical, and it is what you want if you
are auditing which vendor's software wrote a file. `ParsedLine::set("form_type", ..)`
updates both.

## What is still normalised

For completeness, the things that *are* transformed, all documented on
the types involved:

- **Encoding.** `Filing::parse_bytes` decodes UTF-8, falling back to
  Windows-1252 for the whole file if that fails. `FilingReader` decides
  per line. Neither changes any character that decoded successfully.
- **Line splitting.** CSV quoting in 3.x-5.x filings is removed (a quoted
  comma is data; the quotes are not). ASCII-28 filings are split on the
  delimiter and nothing else.
- **`[BEGINTEXT]` blocks** are spliced into the preceding record's `text`
  field; the markers themselves are not data.
- **Typed accessors** (`Typed::money`, `Typed::date`, the typed views)
  parse a copy; `get` on the same field still returns the string as
  filed.

Everything else you get back is what the filer sent.
