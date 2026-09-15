# Fidelity: what the parser does and does not change

A parser for a government filing format has one job before all others:
give back what the filer sent. This chapter states exactly what hardmoney
does to a field value between the bytes on disk and the `&str` you get
from `line.get(..)`.

## The rule: verbatim, except two wire conventions

Every field value is preserved byte for byte, except that leading and
trailing ASCII whitespace is trimmed and one pair of wrapping double
quotes is removed. That is the whole rule.

- Trimmed: space, tab, CR, LF, form feed, vertical tab, at the ends of
  the value only. The FEC's own ingestion trims these, and several
  historical filing vendors padded fields to fixed widths, so keeping
  them would be keeping noise.
- Unwrapped: exactly one pair of surrounding double quotes, then trimmed
  again (`"BRABANT"` -> `BRABANT`, `" SD12 "` -> `SD12`, `""` -> empty).
  Some vendors (CMDI Crimson Filer among them) quote every text field
  even in ASCII-28-delimited files, and the FEC's validator accepts and
  strips the wrapping (its message #26 is "Invalid double-quote surround
  text field"). A lone or unbalanced quote is data and is kept (`"abc`
  stays `"abc`); so is anything inside (`""x""` -> `"x"`, `say "hi"`
  stays `say "hi"`). Whatever quote survives this is therefore embedded
  in the value, which is what `hardmoney validate` reports as
  `embedded_double_quote`.
- Not trimmed: anything inside the value, and any non-ASCII whitespace
  (a non-breaking space in a name is data, not padding).
- Not touched: case, punctuation, angle brackets, ampersands,
  backslashes, pipes. `AT&T` stays `AT&T`; `O'Brien "Bob" <b>` stays
  exactly that.

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

## How this differs from nyt-pyfec-derived parsers

Several FEC parsers descend from `nyt-pyfec` and inherit its
`clean_entry`/`utf8_clean` field cleaning, which upper-cases every field
(`Smith, Jane` becomes `SMITH, JANE`), deletes the characters `&`, `<`,
`>`, `"`, and `\` (`AT&T` becomes `ATT`, `Barnes & Noble` becomes
`Barnes  Noble`), and turns `|` into `,`.

Those are reasonable defaults for a newsroom loading data into a search
index. They are wrong for a reference implementation: they make it
impossible to write a filing back out byte-for-byte, they change legal
names, and they hide from downstream code the fact that the FEC's
validator rejects some of those characters in the first place.
`hardmoney validate` reports an illegal character instead of removing
it.

If you need to compare against output from one of those parsers, their
transformation is reproducible from a hardmoney value: upper-case it,
delete `&<>"\`, and replace `|` with `,`. The reverse is not possible.

## Codes are compared case-insensitively

Preserving case raises an obvious question: real filings contain
`SB21b`, `x` in a memo-code column, `ind` as an entity type. The FEC's
spec defines these tokens as case-insensitive, so every place hardmoney
interprets a code does so case-insensitively, at the point of
interpretation, and leaves the stored value alone:

| What | How it is interpreted |
|---|---|
| form-type tokens (`SA11AI`, `sb21b`, `F3XN`) | dispatch to a `Table` is case-insensitive |
| `memo_code` | `ParsedLine::is_memo()` is true for `X` or `x` (trimmed) |
| entity types, support/oppose codes | `EntityType`/`SupportOppose` parse `IND`/`ind`/` Ind ` to the same variant; `Other(String)` keeps the raw spelling |
| spec versions | `8.5`, `8.5.0.1`, `P3.4`, `p3.4`; see [`SpecVersion`](./library-schema.md#specversion) |

So `line.get("memo_code")` may return `Some("x")`, and `line.is_memo()`
is how you ask the question.

## `raw_form_type` vs. the `form_type` field

There is one deliberate exception to "as filed," and it is a second
copy, not a change. Every `ParsedLine` (and the `Filing` itself) has a
`raw_form_type: String` that is the column-0 token upper-cased:
`"SA11AI"`, `"SB21B"`, `"F3XN"`. It exists so that code matching on the
token (`if line.raw_form_type == "SB21B"`) works regardless of how the
filer capitalised it, and it is what the JSON output and the bulk loader
use.

The `form_type` field (`line.get("form_type")`) keeps the token exactly
as filed, `"sb21b"` and all. That is what the writer emits, so a
round-tripped filing keeps the filer's spelling, and it is what you want
if you are auditing which vendor's software wrote a file.
`ParsedLine::set("form_type", ..)` updates both.

## What is still normalised

For completeness, these are the things that are transformed. Each is
documented on the type involved.

Encoding: `Filing::parse_bytes` decodes UTF-8, falling back to
Windows-1252 for the whole file if that fails. `FilingReader` decides
per line. Neither changes any character that decoded successfully.

Line splitting: CSV quoting in 3.x-5.x filings is removed (a quoted
comma is data; the quotes are not). ASCII-28 filings are split on the
delimiter and nothing else.

`[BEGINTEXT]` blocks are spliced into the preceding record's `text`
field; the markers themselves are not data.

Typed accessors (`Typed::money`, `Typed::date`, the typed views) parse a
copy; `get` on the same field still returns the string as filed.

Everything else you get back is what the filer sent, and because the
parser keeps it, the writer can put it back:
[Writing `.fec` files](./writing-fec.md) is the other half of this
chapter.
