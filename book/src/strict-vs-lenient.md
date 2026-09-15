# Strict vs. lenient parsing

Real filings are big (a presidential committee's Form 3P can run to
hundreds of thousands of body lines), and every so often one of those
lines has a form-type token the parser has never seen, or belongs to a
schedule whose format table has no column layout for that filing's spec
version. What should happen to the other 699,999 lines?

`hardmoney` has both answers and makes you pick one explicitly.

## Strict is the default

`Filing::parse` and `Filing::parse_bytes` are strict: the first body
line that cannot be parsed fails the whole filing. That is the right
default for a library, because silently dropping lines is how data goes
missing, and the error tells you exactly which line it was.

To see it, take a real fixture and append one line with a made-up form
type. (Body fields are separated by ASCII 28, which `printf` writes as
`\x1c`.)

```bash
$ cp tests/fixtures/F24N_2011832.fec /tmp/with_junk.fec
$ printf 'ZZZ\x1cthis line type does not exist\n' >> /tmp/with_junk.fec
$ cargo run --quiet --bin hardmoney -- parse /tmp/with_junk.fec
```

```text
error: no format table for form type 'ZZZ' (spec version 8.5) at line 5
```

The command exits with status 1 and prints no JSON. Line 5 is right: the
fixture has a header (line 1), a summary (line 2), and two Schedule E
lines (3 and 4), so the junk landed on line 5.

## Lenient: keep going and tell me what you skipped

Pass `--lenient` and the same file parses, with the skipped line reported
on stderr and recorded in the JSON:

```bash
$ cargo run --quiet --bin hardmoney -- parse --lenient /tmp/with_junk.fec
```

```text
warning: 1 line(s) skipped:
  line 5: 'ZZZ' skipped (unknown form type)
```

```json
{
  "amends_filing": null,
  "base_form_type": "F24",
  "form_type": "F24N",
  "is_amendment": false,
  "line_count": 2,
  "lines_by_table": {
    "SchE": 2
  },
  "skipped": [
    {
      "line_no": 5,
      "raw_form_type": "ZZZ",
      "reason": "UnknownFormType"
    }
  ],
  "skipped_count": 1,
  "version": "8.5"
}
```

(Header and summary trimmed.) The two good Schedule E lines are all
there; `skipped_count` is 1; and `skipped` says which line, which token,
and why. Nothing was lost silently.

## The same choice in Rust

The library mirrors this exactly. Strict:

```rust
use hardmoney::Filing;

let content = std::fs::read_to_string("/tmp/with_junk.fec")?;
match Filing::parse(&content) {
    Ok(filing) => println!("{} body lines", filing.lines.len()),
    Err(e) => eprintln!("refused at line {:?}: {e}", e.line_no()),
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
refused at line Some(5): no format table for form type 'ZZZ' (spec version 8.5) at line 5
```

Lenient, via `Filing::parse_with` and `ParseOptions::LENIENT`:

```rust
use hardmoney::{Filing, ParseOptions};

let content = std::fs::read_to_string("/tmp/with_junk.fec")?;
let (filing, skipped) = Filing::parse_with(&content, &ParseOptions::LENIENT)?.into_parts();

println!("kept {} lines, skipped {}", filing.lines.len(), skipped.len());
for s in &skipped {
    eprintln!("{s}"); // SkippedLine implements Display
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
kept 2 lines, skipped 1
line 5: 'ZZZ' skipped (unknown form type)
```

`parse_bytes_with(&bytes, &ParseOptions::LENIENT)` is the same thing for
raw bytes (with the UTF-8/Windows-1252 decoding `parse_bytes` does).

### You can't forget the skipped lines

`parse_with` doesn't return a `Filing`. It returns a `Lenient<Filing>`,
and the only ways to get the `Filing` out are:

- `into_parts()` gives you `(Filing, Vec<SkippedLine>)`. You receive the
  skipped lines whether you want them or not; discarding them is a
  visible `let (filing, _) = ...`.
- `into_strict()` gives you `Result<Filing, FecError>`: the `Filing` if
  nothing was skipped, otherwise `FecError::LinesSkipped { count, first }`
  carrying the count and the first underlying error. This is how
  `Filing::parse` itself is implemented
  (`parse_with(..., &ParseOptions::STRICT)?.into_strict()`).

`Lenient<T>` has no `Deref` to `T` and no public fields, and it's marked
`#[must_use]`, so dropping it on the floor is a compiler warning. You can
peek with `.skipped()` and `.value()` without consuming it, but those
don't discharge the `must_use`; they're for inspection, not for
extracting the value.

```rust
use hardmoney::{Filing, Lenient, ParseOptions};

let bytes = std::fs::read("/tmp/with_junk.fec")?;
let lenient: Lenient<Filing> = Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT)?;
println!("{} skipped", lenient.skipped().len());

// Decide, at this point, that any skip is unacceptable after all:
let filing: Filing = lenient.into_strict()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

On the junk file that last line returns
`Err(LinesSkipped { count: 1, first: ParserMissing { form_type: "ZZZ", version: "8.5", line_no: Some(5) } })`.

## What lenient skips, and what still fails

Lenient mode only relaxes body-line problems. Two kinds of line can be
skipped, and each `SkippedLine` says which via its `reason`:

| `SkipReason` | Meaning | Strict-mode error |
|---|---|---|
| `UnknownFormType` | column 0 matched no dispatch pattern | `FecError::ParserMissing { form_type, version, line_no }` |
| `NoLayoutForVersion` | the table exists, but has no column layout for this filing's spec version (typically a filing newer than the bundled tables) | `FecError::NoMatchingVersionBucket { table, version, line_no }` |

Everything that isn't about one body line always fails, in either
mode: an unrecognized header version
(`FecError::UnknownElectronicHeaderVersion`), a missing summary line
(`FecError::MissingFormLine`), or a `[BEGINTEXT]` free-text block that's
never closed with `[ENDTEXT]` (`FecError::UnterminatedTextBlock { line_no
}`). Those mean the file as a whole can't be trusted, so there's nothing
sensible to "keep going" with. (An amendment whose header doesn't say
what it amends is not a parse error: `filing.amends_filing` is `None`,
and [`hardmoney validate`](./validating.md) reports it as
`amendment_needs_original_id`, which is how the FEC treats it.)

`ParseOptions` is two independent knobs, so you can also mix them: skip
unknown form types but fail on missing version layouts, say:

```rust
use hardmoney::parser::{OnUnparseableLine, ParseOptions};

let mut opts = ParseOptions::STRICT;
opts.on_unknown_line = OnUnparseableLine::Skip;
// opts.on_missing_version stays OnUnparseableLine::Fail
```

`ParseOptions::STRICT`, `ParseOptions::LENIENT`, `ParseOptions::lenient()`,
and `ParseOptions::default()` (same as `STRICT`) cover the common cases.

## Matching on the error

Every `FecError` that can be traced to a physical line carries its
1-based `line_no`, and `FecError::line_no()` returns it (or `None` for
errors that aren't about one line). The variants you'll most often want
to match on:

```rust
use hardmoney::{FecError, Filing};

let content = std::fs::read_to_string("/tmp/with_junk.fec")?;
match Filing::parse(&content) {
    Ok(filing) => println!("{} lines", filing.lines.len()),
    Err(FecError::ParserMissing { form_type, line_no, .. }) => {
        eprintln!("unknown form type {form_type} at line {line_no:?}");
    }
    Err(FecError::NoMatchingVersionBucket { table, version, line_no }) => {
        eprintln!("no layout for {table} at spec {version} (line {line_no:?})");
    }
    Err(FecError::UnterminatedTextBlock { line_no }) => {
        eprintln!("[BEGINTEXT] at line {line_no} never closed");
    }
    Err(FecError::LinesSkipped { count, first }) => {
        eprintln!("{count} skipped; first: {first}");
    }
    Err(other) => eprintln!("{other}"),
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

`FecError` is `#[non_exhaustive]`, so the `other` arm is required.
[`examples/handle_parse_errors.rs`](https://github.com/cgorski/hardmoney/blob/main/examples/handle_parse_errors.rs)
walks through the strict/lenient split on a synthetic filing (along with
two other error scenarios); its real output ends:

```text
strict: refused 'ZZZ' at line Some(4), as expected
lenient: kept 1 line(s), skipped 1: line 4: 'ZZZ' skipped (unknown form type)
```

## Which should you use?

For analysis, tests, and anything where a wrong answer is worse than no
answer: strict. If the parser doesn't understand a line, you want to
know before you compute a total that's missing it.

For ingestion pipelines, where one filing failing shouldn't stop the
job: lenient, and record the skip count somewhere you'll look. This is
what `hardmoney bulk-load-filing` does by default. It parses leniently,
stores the number of skipped lines in `filings.skipped_lines`, and
`GET /filings/{id}` reports it. Pass `--strict` to opt out. See
[Loading bulk data into Postgres](./bulk-etl.md#ingesting-a-single-filing-directly-for-precise-schedule-e-data).

The crate's own real-filing test suite parses every fixture both ways
and asserts the lenient result is identical to the strict one with zero
skips, so on the 25 real filings it ships with, the two modes agree.
The difference only shows up on data the parser doesn't understand,
which is exactly when you want to be told.
