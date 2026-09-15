# Writing `.fec` files

Everything before this chapter reads filings. This one goes the other
way: `Filing::to_fec` turns a parsed `Filing` back into the `.fec` wire
format, and `hardmoney write` does the same from the command line. It
exists for three jobs: fixing a field and writing the filing back out,
normalising what a vendor's software produced, and proving that the
parser lost nothing. It is also the reason the parser preserves values
as filed (see [Fidelity](./fidelity.md)): a writer cannot put back what
the parser threw away.

## Three methods, none of which can fail

```rust
use hardmoney::Filing;

let bytes = std::fs::read("tests/fixtures/F3XA_2011827.fec")?;
let filing = Filing::parse_bytes(&bytes)?;

let text: String = filing.to_fec_string(); // canonical text
let out: Vec<u8> = filing.to_fec();        // encoded bytes (Windows-1252 or UTF-8)
filing.write_fec(std::fs::File::create("/tmp/F3XA_2011827.canonical.fec")?)?;
# let _ = (text, out);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`to_fec_string` and `to_fec` return a value directly, not a `Result`:
every `ParsedLine` carries the `Layout` it was parsed with, so each
field always has a column to go back to. `write_fec` only fails if the
`io::Write` you hand it does.

## The round-trip guarantee

For every filing the parser accepts,

```text
parse(to_fec(parse(f)))  ==  parse(f)
```

in the sense that matters: the same header, the same cover line, and the
same body lines with the same fields in the same tables. This is tested
over every real fixture in `tests/fixtures/` and with `proptest` over
arbitrary field values (`src/parser/writer.rs`). The output is also
idempotent: writing the re-parsed filing produces the same bytes again.

```rust
use hardmoney::Filing;

let bytes = std::fs::read("tests/fixtures/F3XA_2011827.fec")?;
let filing = Filing::parse_bytes(&bytes)?;
let out = filing.to_fec();

// Every record comes back field-for-field.
let again = Filing::parse_bytes(&out)?;
assert_eq!(again.header, filing.header);
assert!(again.summary.iter().eq(filing.summary.iter()));
assert_eq!(again.lines.len(), filing.lines.len());
for (a, b) in filing.lines.iter().zip(&again.lines) {
    assert_eq!(a.table(), b.table());
    assert!(a.iter().eq(b.iter()));
}

// And writing the re-parsed filing changes nothing: the form is canonical.
assert_eq!(again.to_fec(), out);

println!("{} bytes in, {} bytes out", bytes.len(), out.len());
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
1867 bytes in, 1876 bytes out
```

Nine bytes grew. That is the whole difference between "as filed" and
"canonical", and the next section says exactly where they came from.

## Canonical form: what is and is not preserved

The output is canonical, not byte-identical. The parser removes two
wire-format conventions (surrounding whitespace and one pair of wrapping
quotes) and does not care about line endings, trailing empty columns, or
blank lines; the writer emits one fixed choice for each:

| | Written as |
|---|---|
| Line endings | always `CRLF`, the FEC's convention |
| Delimiter | ASCII 28 for spec 6.0+, comma for 3.x-5.x |
| CSV quoting (3.x-5.x) | only where a value contains `,`, `"`, or a line break; `"` inside a value is doubled |
| Record width | every record has the full column count of its layout: trailing empty columns the original omitted are added, and padding beyond the layout is dropped |
| Field values | as the parser holds them: trimmed, one wrapping quote pair removed, otherwise verbatim |
| Form 99 free text | a `[BEGINTEXT]`/`[ENDTEXT]` block after the cover line, with the cover's `text` column blank, which is how FECfile itself writes it |
| Blank lines | dropped |
| A delimiter or line break inside a value | replaced with a space (it could not have survived parsing anyway) |

Here is the fixture above, original and canonical, through `cat -v` so
the control characters show (`^\` is ASCII 28, `^M` is the carriage
return):

```text
$ cat -v tests/fixtures/F3XA_2011827.fec | cut -c1-160 | head -2
HDR^\FEC^\8.5^\FECfile^\8.5.1.0(f34)^\FEC-1991972^\1
F3XA^\C00944124^\REVIVE OREGON^\^\PO BOX 26141^\^\ALEXANDRIA^\VA^\22313^\Q2^\^\^\^\20260401^\20260630^\^\MARSTON^\CHRIS^\^\^\^\20260914^\20000.00^\20000.00^\400

$ hardmoney write tests/fixtures/F3XA_2011827.fec | cat -v | cut -c1-160 | head -2
HDR^\FEC^\8.5^\FECfile^\8.5.1.0(f34)^\FEC-1991972^\1^\^M
F3XA^\C00944124^\REVIVE OREGON^\^\PO BOX 26141^\^\ALEXANDRIA^\VA^\22313^\Q2^\^\^\^\20260401^\20260630^\^\MARSTON^\CHRIS^\^\^\^\20260914^\20000.00^\20000.00^\400
```

FECfile wrote the header with seven columns and `LF` line endings; the
canonical form has the eighth (`HDRcomment`, blank) and `CRLF`. Eight
lines gained a `CR` each and the header gained one delimiter: nine bytes.

The same rules can also shrink a file. The 2001 Merck PAC filing at
spec 3.00 was written with its fields quoted throughout; canonical CSV
quotes only what needs it:

```text
$ hardmoney write --check tests/fixtures/F3XA_27789_v3.fec
OK: tests/fixtures/F3XA_27789_v3.fec round-trips (2837 body lines, 771325 bytes in, 565764 bytes out)

$ head -3 tests/fixtures/F3XA_27789_v3.fec | cut -c1-100
"HDR","FEC","3.00","KNOWLEDGE","XP.1108","","FEC-24088","1",""
"F3XA","C00097485","MERCK PAC, The Political Action Committee of Merck & Co., Inc.","601 Pennsylvani
"SA11A1","C00097485","IND","Aaland^Lyla L^Ms^","19017 Baldwin Street Nw","","Elk River","MN","553305

$ hardmoney write tests/fixtures/F3XA_27789_v3.fec | head -3 | cut -c1-100
HDR,FEC,3.00,KNOWLEDGE,XP.1108,,FEC-24088,1,
F3XA,C00097485,"MERCK PAC, The Political Action Committee of Merck & Co., Inc.","601 Pennsylvania Av
SA11A1,C00097485,IND,Aaland^Lyla L^Ms^,19017 Baldwin Street Nw,,Elk River,MN,553305533,,,Merck-Medco
```

`Merck & Co.` keeps its ampersand and `Aaland^Lyla L^Ms^` keeps its
carets.

In Rust, the same normalisation on a synthetic line with trailing
whitespace, a quoted field, a lower-case form-type token, and a record
that stops after nine of the 8.5 Schedule A layout's 45 cells:

```rust
use hardmoney::Filing;

let text = "HDR\u{1c}FEC\u{1c}8.5\u{1c}Vendor\u{1c}1.0\u{1c}\u{1c}\u{1c}\n\
            F3XN\u{1c}C00123456\u{1c}Example PAC\n\
            sa11ai\u{1c}C00123456\u{1c}A1\u{1c}\u{1c}\u{1c}IND\u{1c}\u{1c}\"Smith\"  \u{1c} Jane \n";
let filing = Filing::parse(text)?;
let out = filing.to_fec_string();

let lines: Vec<&str> = out.split("\r\n").collect();
assert_eq!(lines[0], "HDR\u{1c}FEC\u{1c}8.5\u{1c}Vendor\u{1c}1.0\u{1c}\u{1c}\u{1c}");
assert!(lines[1].starts_with("F3XN\u{1c}C00123456\u{1c}Example PAC\u{1c}"));
assert_eq!(lines[1].matches('\u{1c}').count(), filing.summary.layout().width as usize - 1);
assert!(lines[2].starts_with("sa11ai\u{1c}C00123456\u{1c}A1\u{1c}\u{1c}\u{1c}IND\u{1c}\u{1c}Smith\u{1c}Jane\u{1c}"));
assert_eq!(lines[2].matches('\u{1c}').count(), 44); // padded to the full layout
assert!(out.ends_with("\r\n"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

The token `sa11ai` is written back in the filer's lower case (the
`form_type` field is as filed; only `raw_form_type` is upper-cased),
`"Smith"` lost its quotes and `Jane` its padding, and the short record
was padded to the 45 cells the layout defines.

## Encoding

The FEC's character set is single-byte: ASCII 32-126 plus Latin-1
128-168 and 173. `to_fec` therefore encodes as Windows-1252 when every
character is representable, and as UTF-8 otherwise. This is the mirror
image of the parser's "UTF-8 first, Windows-1252 fallback" decoding, so
a round trip always reproduces the same characters. (One guard: if the
Windows-1252 bytes would also be valid UTF-8 that decodes differently,
UTF-8 is used so the parser cannot misread them.) `to_fec_string` gives
you the text before encoding, if you want to choose yourself.

```rust
use hardmoney::Filing;

let text = "HDR\u{1c}FEC\u{1c}8.5\u{1c}Vendor\u{1c}1.0\u{1c}\u{1c}\u{1c}\n\
            F3XN\u{1c}C00123456\u{1c}José Martínez for Congress\n";
let filing = Filing::parse(text)?;

// Every character is representable in Windows-1252, so that is what
// is written: one byte per character, é = 0xE9.
let bytes = filing.to_fec();
assert!(bytes.windows(4).any(|w| w == b"Jos\xE9"));
assert_eq!(
    Filing::parse_bytes(&bytes)?.summary.get("committee_name"),
    Some("José Martínez for Congress")
);

// A character outside Windows-1252 forces UTF-8 for the whole file.
let text = text.replace("José", "Jos\u{0119}"); // ę
let filing = Filing::parse(&text)?;
let bytes = filing.to_fec();
assert!(std::str::from_utf8(&bytes).is_ok());
assert_eq!(
    Filing::parse_bytes(&bytes)?.summary.get("committee_name"),
    Some("Jos\u{0119} Martínez for Congress")
);
# Ok::<(), Box<dyn std::error::Error>>(())
```

The writer does not reject `ę`; that is the validator's job
(`hardmoney validate` reports `illegal_character`). The writer's job is
to give back what you have, decodably.

## `hardmoney write [--check]`

```text
$ hardmoney write --help
Parse a `.fec` filing and write it back out in canonical form

Usage: hardmoney write [OPTIONS] <PATH>

Arguments:
  <PATH>  Path to a `.fec` file

Options:
  -o, --out <OUT>  Output path. Defaults to stdout
      --lenient    Skip body lines that cannot be parsed instead of failing (they are omitted from the output and listed on stderr)
      --check      Instead of writing, re-parse the written bytes and report whether every record round-trips; exit 1 if not
  -h, --help       Print help
```

Without flags it writes the canonical bytes to stdout (or `-o FILE`).
`--lenient` parses with `ParseOptions::LENIENT`; a skipped line is
omitted from the output and reported on stderr as `warning: omitted
line N: 'ZZZ' skipped (unknown form type)`. The result is a filing the
parser fully understands, and also a filing with fewer lines than the
input.

`--check` is the round-trip test as a command. It writes the filing to
memory, parses that, and compares header, cover line, and every body
line:

```text
$ hardmoney write --check tests/fixtures/F3XA_2011827.fec
OK: tests/fixtures/F3XA_2011827.fec round-trips (6 body lines, 1867 bytes in, 1876 bytes out)
```

Exit 0. On a mismatch it prints one `MISMATCH: ...` line per problem
(`header differs`, `cover line differs`, `line count differs: 10 vs 9`,
`line 14 differs`; the per-line list is cut off after eleven) on stderr
and exits 1. Every one of the 25 bundled real filings, spec 3.00 through
8.5, passes; so did 144 of 144 parseable filings in a wider local corpus
when the writer was built.

## Editing a filing and writing it back

Because a `ParsedLine` knows its layout, `set` puts a value in the right
column for the filing's spec version, and the writer emits it there.
Together with `Filing::open` this is a complete edit-and-save loop:

```rust
use hardmoney::{Filing, Table};

let mut filing = Filing::open("tests/fixtures/F3XA_2011827.fec")?;

// Correct an employer on every Schedule A line, then write it back out.
let mut changed = 0;
for line in filing.lines.iter_mut().filter(|l| l.table() == Table::SchA) {
    if line.get("contributor_employer") == Some("SELF") {
        line.set("contributor_employer", "Self-employed")?;
        changed += 1;
    }
}
// A field the 8.5 layout does not have is an error, not a silent no-op.
let err = filing.lines[0].set("no_such_field", "x").unwrap_err();
println!("{err}");

filing.write_fec(std::fs::File::create("/tmp/F3XA_2011827.edited.fec")?)?;

let edited = Filing::open("/tmp/F3XA_2011827.edited.fec")?;
assert_eq!(edited.lines.len(), filing.lines.len());
println!("{changed} line(s) changed");
for line in edited.lines_for(Table::SchA) {
    println!("line {}: {:?}", line.line_no, line.get("contributor_employer"));
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
table SchA has no field named 'no_such_field'
1 line(s) changed
line 3: Some("Self-employed")
```

Some things to know when editing.

`set` normalises like the parser. The value is trimmed and one pair of
wrapping quotes is removed, so what you `set` is what `get` returns and
what the writer emits. Setting `form_type` also updates `raw_form_type`.

Unknown fields are `FecError::UnknownField`. The layout is the filing's
spec version, so a field that only exists in 8.x cannot be set on a 5.3
filing. The error names the table and the field.

New lines come from `ParsedLine::from_pairs`. Give it the table, the
filing's `version`, a line number (0 for synthetic), and `(field, value)`
pairs, then push it onto `filing.lines`. Anything you do not name is
blank. See
[The schema](./library-schema.md#parsedline-get-iter-set-from_pairs).

You cannot build a `Filing` from nothing. `Filing` is
`#[non_exhaustive]`; start from a parsed one (or a minimal synthetic
filing parsed from a string, as in the snippets above) and edit it.

The header is a plain struct. `filing.header.comment = "...".into()`
works; `Header::to_fields` is what the writer calls.

The writer does not validate. It will write a 31-character last name or
a `$5,500.00` amount without complaint. Run `hardmoney validate` on the
result (see [Validating a filing](./validating.md)) and, for a periodic
report whose totals you touched, [Reconciling a filing](./reconciling.md).
