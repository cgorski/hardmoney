# Streaming large filings

`Filing::parse_bytes` is the right tool for almost every filing. It
decodes the whole file, parses every line, and hands them all back in
`filing.lines`, which is what an API, the validator, the reconciler,
and the writer need. A few filings are not "almost every filing". A
presidential committee's post-general Form 3P runs to 135 MB and
700,000 Schedule A lines, and a parsed `ParsedLine` is larger than its
wire form. A job that only needs one pass (sum Schedule A, copy
Schedule E into a database, count itemized lines) should not have to
hold all of that.

`FilingReader` is that one pass, and `Filing::open` is the everyday
convenience built on it.

## When to use which

| | `Filing::parse_bytes` / `Filing::open` | `FilingReader` |
|---|---|---|
| You get | a `Filing` with every line in `lines` | the header and cover up front, then body lines one at a time |
| Memory | the whole parsed filing (plus, for `parse_bytes`, the file's bytes and decoded text) | one record, plus one held-back record |
| Random access, `views()`, `lines_for()`, `validate()`, `reconcile()`, `to_fec()` | yes | no; one forward pass |
| Input | `&[u8]` / `&str` / a path | any `BufRead`: a file, a network body, a decompressor, bytes in memory |
| Use it for | anything that needs the filing as a whole | large filings; pipelines that fold lines into a total or a database as they go |

## The numbers

Measured on the largest filing in the local corpus (filing 2010101, a
Form 3P amendment covering the 2020 post-general period, 135,241,563
bytes, 704,651 body lines of which 689,776 are Schedule A) with a
release build, using the `#[ignore]`d `measure_*` tests in
`tests/stream_fixtures.rs` under `/usr/bin/time -l`:

| | Peak RSS | Wall time |
|---|---|---|
| `Filing::parse_bytes(&std::fs::read(path)?)` | 1,337,409,536 bytes (1.34 GB) | 0.50 s |
| `Filing::open(path)` (stream from disk into a `Filing`) | 1,067,433,984 bytes (1.07 GB) | 0.47 s |
| `FilingReader` + `filter_tables([SchA])`, running `Decimal` total | 9,814,016 bytes (9.8 MB) | 1.10 s |

Three things to read off that table. `Filing::open` saves the file's
bytes and its decoded text (about a fifth of the peak) and is otherwise
the same object as `parse_bytes` produces. The streaming pass is
independent of the file's size: 9.8 MB is the process itself (a
`BufReader`, one record, one held-back record, and a `Decimal`), and it
would be the same on a 1 GB filing. And streaming costs about twice the
wall time here, because the eager path decodes and splits in large
batches while the reader works a line at a time; at around a second
either way that is rarely the deciding factor, memory is.

## `FilingReader`

```rust
use std::fs::File;
use std::io::BufReader;

use hardmoney::{FilingReader, ScheduleA, Table};
use rust_decimal::Decimal;

let file = BufReader::new(File::open("tests/fixtures/F3XN_2011831.fec")?);
let reader = FilingReader::new(file)?.filter_tables([Table::SchA]);

let cover = reader.preamble();
println!(
    "{} for {} at spec {} ({} lines read so far)",
    cover.raw_form_type,
    cover.summary.get("committee_name").unwrap_or(""),
    cover.version,
    reader.lines_read()
);

let mut total = Decimal::ZERO;
let mut n = 0;
for line in reader {
    let line = line?; // Err ends the read: strict, like Filing::parse
    if line.is_memo() {
        continue;
    }
    let a = line.view::<ScheduleA>()?;
    total += a.contribution_amount.unwrap_or_default();
    n += 1;
}
println!("{n} non-memo Schedule A lines, total {total}");
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
F3XN for FirstEnergy Corp Political Action Committee at spec 8.5 (3 lines read so far)
132 non-memo Schedule A lines, total 13736.02
```

That total is the filing's line 11(a)(i) to the cent (compare
[Reconciling a filing](./reconciling.md)), computed without ever holding
more than one Schedule A line. Piece by piece:

`FilingReader::new(reader)` takes any `BufRead` and parses the header
and cover line immediately. They are always needed, and the header's
spec version decides how every other line is parsed. It chooses the
ASCII-28 or comma-delimited path from the first line exactly as
`Filing::parse` does, and fails with the same errors
(`DeprecatedHeaderFormat`, `MissingFormLine`,
`UnknownElectronicHeaderVersion`, `ParserMissing` for an unknown cover
form, `Io`). `with_options(reader, ParseOptions)` is the lenient form.

`preamble()` is a `Preamble`: everything a `Filing` has apart from its
body lines (`header`, `version`, `raw_form_type`, `base_form_type`,
`is_amendment`, `amends_filing`, `summary`), with the same meanings. For
a Form 99, `summary` already includes any `[BEGINTEXT]` block that
followed the cover.

The reader is an `Iterator<Item = Result<ParsedLine>>`, in file order,
and each `ParsedLine` is identical to what the eager parser would have
produced for that line (same `line_no`, same layout, same values). It is
fused: after `None` or an `Err` it stays exhausted.

`filter_tables(tables)` restricts the iterator to the tables you name.
Lines of other tables are dropped silently (they are not recorded as
skipped), but every line is still dispatched, so an unknown form-type
token is still an error (or a `SkippedLine` under lenient options). An
empty set yields nothing; a second call replaces the first.

`lines_read()` is how many physical lines have been consumed, header
and cover included. It is `3` right after `new()`, not `2`: the reader
has already looked at the first body line, for the reason the next
section explains.

`skipped()` is the lenient parse's skipped lines so far (always empty
under `STRICT`), and `into_filing()` drains the rest into an eager
`Filing`; see below.

## The one-record lookahead

Form 99 filings carry their free text in a `[BEGINTEXT]` ... `[ENDTEXT]`
block that comes after the record it belongs to, and the parser splices
that text into the preceding record's `text` field (see
[Parsing a filing, explained](./parsing-explained.md)). A streaming
reader therefore cannot hand you a record the moment it has parsed it:
the next line might be a text block that has to go into it. So the
reader holds each parsed record back until it has seen the following
line, and what you receive is always complete. That is the one extra
record in memory, and it is why `lines_read()` runs one ahead.

Blocks that directly follow the cover line (the normal Form 99 case) are
consumed by `FilingReader::new` itself, so `preamble().summary` is
complete before you read anything:

```rust
use hardmoney::FilingReader;

// Any BufRead works: a network body, a decompressor, or bytes in memory.
let text = "HDR\u{1c}FEC\u{1c}8.5\u{1c}Vendor\u{1c}1.0\u{1c}\u{1c}\u{1c}\n\
            F99\u{1c}C00123456\u{1c}Example PAC\n\
            [BEGINTEXT]\nDear FEC,\nthis is a miscellaneous report.\n[ENDTEXT]\n";
let reader = FilingReader::new(text.as_bytes())?;
assert_eq!(
    reader.preamble().summary.get("text"),
    Some("Dear FEC,\nthis is a miscellaneous report.")
);
assert_eq!(reader.count(), 0);
# Ok::<(), Box<dyn std::error::Error>>(())
```

A block that follows a line `filter_tables` dropped is dropped with it;
an unterminated block is `FecError::UnterminatedTextBlock` and ends the
read. (Comma-delimited 3.x-5.x filings predate the convention, so that
path has no lookahead.)

## `Filing::open` and `Filing::open_with`

`Filing::open(path)` streams a file from disk through a `FilingReader`
with a 64 KiB buffer and collects the result into a `Filing`. It is
field-for-field equal to `Filing::parse_bytes(&std::fs::read(path)?)`
(`tests/stream_fixtures.rs` asserts that on every fixture and on the
135 MB filing), and it peaks lower because the file's bytes and decoded
text never exist as a whole. Several of this book's Rust snippets use
it wherever they have a path rather than bytes.

```rust
use hardmoney::{Filing, ParseOptions};

let path = "tests/fixtures/F3A_767339_v8.0.fec";
let streamed = Filing::open(path)?;
let eager = Filing::parse_bytes(&std::fs::read(path)?)?;
assert_eq!(streamed.header, eager.header);
assert_eq!(streamed.lines, eager.lines);

// The lenient form returns the same Lenient<Filing> as parse_bytes_with.
let (filing, skipped) = Filing::open_with(path, ParseOptions::LENIENT)?.into_parts();
println!("{} lines, {} skipped", filing.lines.len(), skipped.len());
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
641 lines, 0 skipped
```

`open_with(path, options)` is the streaming counterpart of
`parse_bytes_with`, and returns the same `Lenient<Filing>`, so the two
can be swapped freely in code that already handles skipped lines the
way [Strict vs. lenient parsing](./strict-vs-lenient.md) describes. An
`Io` error from `open` names the path.

## Errors, lenient options, and `into_filing`

Errors about the filing as a whole come from `FilingReader::new`. Errors
about one body line come from the iterator as `Some(Err(..))`, after
which it is exhausted. Under `ParseOptions::STRICT` the first
unparseable line ends the read, exactly as it fails `Filing::parse`:

```rust
use std::fs::File;
use std::io::BufReader;

use hardmoney::{FecError, FilingReader};

let file = BufReader::new(File::open("/tmp/with_junk.fec")?);
let mut reader = FilingReader::new(file)?;
let mut n = 0;
let err = loop {
    match reader.next() {
        Some(Ok(_)) => n += 1,
        Some(Err(e)) => break e,
        None => panic!("expected an error"),
    }
};
assert!(matches!(err, FecError::ParserMissing { line_no: Some(5), .. }));
assert!(reader.next().is_none()); // fused: exhausted after an error
println!("{n} good lines, then: {err}");
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
2 good lines, then: no format table for form type 'ZZZ' (spec version 8.5) at line 5
```

(`/tmp/with_junk.fec` is a fixture with one made-up `ZZZ` line appended,
built in [Strict vs. lenient parsing](./strict-vs-lenient.md).) The two
good lines were yielded before the error: the held-back record is
released first, then the error is reported.

Under `ParseOptions::LENIENT` such lines are recorded in `skipped()`
instead and the read continues. I/O errors and an unterminated text
block always end it.

```rust
use std::fs::File;
use std::io::BufReader;

use hardmoney::{FilingReader, ParseOptions};

let file = BufReader::new(File::open("/tmp/with_junk.fec")?);
let mut reader = FilingReader::with_options(file, ParseOptions::LENIENT)?;

let mut kept = 0;
for line in reader.by_ref() {
    let _line = line?;
    kept += 1;
}
for s in reader.skipped() {
    println!("{s}");
}
println!("kept {kept}, skipped {}", reader.skipped().len());
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
line 5: 'ZZZ' skipped (unknown form type)
kept 2, skipped 1
```

`into_filing()` drains whatever is left into an eager `Filing`, wrapped
in the same `Lenient<Filing>` that `parse_bytes_with` and `open_with`
return, so the skipped lines cannot be forgotten. A `filter_tables` in
place means `filing.lines` holds only those tables:

```rust
use std::fs::File;
use std::io::BufReader;

use hardmoney::{FilingReader, ParseOptions, Table};

let file = BufReader::new(File::open("/tmp/with_junk.fec")?);
let lenient = FilingReader::with_options(file, ParseOptions::LENIENT)?
    .filter_tables([Table::SchE])
    .into_filing()?; // Lenient<Filing>, exactly as parse_bytes_with returns
let (filing, skipped) = lenient.into_parts();
println!("{} Schedule E lines, {} skipped", filing.lines.len(), skipped.len());
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
2 Schedule E lines, 1 skipped
```

## Encoding, per line

Filings are UTF-8, or Windows-1252 when older software wrote
filer-entered text as raw bytes. The eager parser decides once for the
whole file (UTF-8 if the whole thing decodes, else Windows-1252); the
streaming reader, which never sees the whole file, decides per line
(per record on the comma-delimited path). The two agree on every
fixture and on every filing that is consistently one encoding. They
differ only on a file that mixes valid multi-byte UTF-8 on some lines
with invalid bytes on others, where the streaming reader keeps the UTF-8
lines intact and the eager parser re-reads them as Windows-1252.
`Filing::open` takes the streaming behaviour.

## Where it fits

Use the eager parser when you need random access to the lines, the
writer, validation, or reconciliation. Use the streaming reader when the
filing is large and one forward pass is enough, or when the input is not
a file. The crate's own bulk loader uses the eager path, because it
needs the whole filing to store it, and `benches/parse.rs` (criterion)
tracks both paths against each other.
