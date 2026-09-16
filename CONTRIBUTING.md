# Contributing to hardmoney

hardmoney aims to be the reference implementation of the FEC electronic
filing format: something the FEC's own staff, the 85 registered filing
vendors, journalists, and researchers can all trust. That sets the bar for
every change. This document is the bar.

## The rules

### 1. No panics in library code

`unwrap()`, `expect()`, indexing (`v[i]`), `unreachable!()`, integer
overflow in debug, slicing by byte offsets you did not compute from the
same string -- none of these may appear in `src/` outside `#[cfg(test)]`.
Real FEC data is hostile: 135 MB files, fields in the wrong column,
Windows-1252 bytes, versions like `180.5`. A malformed filing must produce
an `Err` that names the line, never a crash.

* Use `.get(i)`, `checked_*`, `saturating_*`, `try_from`, `?`.
* Where an invariant genuinely cannot be violated, prefer restructuring the
  types so the compiler proves it over an `expect()` with a comment.
* `clippy::unwrap_used` / `clippy::expect_used` are not enforced crate-wide
  only because of test code; treat them as enforced in `src/`.

### 2. Make illegal states unrepresentable

Prefer a type to a convention:

* Newtypes and value types over primitives: `SpecVersion` not `String`,
  `Cycle` not `i32`, `Namespace` not `String`, `Decimal` (scale 2) for
  money -- **never `f64`** for a dollar amount anywhere in the crate.
* Closed sets are enums (`Table`, `FieldKind`, `Requirement`, `Severity`).
  Open sets from the FEC's data keep an `Other(String)` variant rather
  than rejecting the line (`EntityType`, `SupportOppose`).
* Compile-time table checking: field access goes through the generated
  `Field<T>` constants (`tables::sch_a::CONTRIBUTION_AMOUNT`) and
  `Typed<'_, T>` wherever the table is known statically. Reserve string
  field names (`line.get("...")`) for genuinely dynamic access.
* Results that must be looked at are `#[must_use]` and, where the data
  could be lost, cannot be opened without acknowledging it
  (`Lenient<T>::into_parts`).
* Public enums and structs that will grow are `#[non_exhaustive]`.

### 3. Data, not code, for facts about the FEC format

Column positions, field types, lengths, required levels, and rule text
live in `data/` and are compiled by `build.rs`. Do not hard-code a column
number or a max length in Rust. If the FEC changes the spec, the fix is a
data change plus a regenerated `data/fec-spec/spec-<version>.json`
(`scripts/distill_fec_spec.py`), never a code edit in fifty places.

The generator rejects inconsistent data (two fields at one column, one
field at two columns, non-numeric position cells, overlapping version
buckets). If your data change fails the build, the data is wrong, not the
check.

### 4. Docs match code

* Every public item has a doc comment that says what it does, what it
  returns for blank/absent input, and what it fails on. rustdoc runs with
  `-D warnings`; broken intra-doc links fail CI.
* Doc examples compile (`cargo test --doc`). Use `no_run` for examples
  that need a file or network, not `ignore`.
* When you change behaviour, change the docs, the book chapter, and the
  `CHANGELOG.md` entry in the same commit.
* Comments explain *why* (a constraint, a real-world quirk, an FEC rule
  reference), not *what* the code already says.

### 5. Test against the truth, not against yourself

* **Real filings**: every parser change runs over `tests/fixtures/` (61
  real filings, spec 3.00 through 8.5, across `tests/fixtures/`,
  `chain/`, `rad/`, and `golden/`). Assertions there were checked by
  hand against the raw bytes.
* **Fixtures are bytes, not text.** The FEC's files come CRLF- and
  LF-terminated, FS- and comma-delimited, ASCII and Windows-1252; the
  corpus keeps every regime (`tests/corpus_shape.rs` pins the floors) and
  nothing may rewrite one. `.gitattributes` marks `tests/fixtures/**`
  `-text` so git never converts them, and `tests/fixtures/MANIFEST.sha256`
  records the committed SHA-256 of each; the suite (and CI) fails if a
  file on disk differs. Adding or deliberately changing a fixture:
  `git add` it, run `python3 scripts/fixture_manifest.py`, commit both.
  Never open a fixture in an editor that normalises line endings or
  encodings.
* **Oracle tests**: where the FEC publishes an implementation or expected
  values (`fecfile-web-api`'s summary calculator and its test data, the
  spec workbook's rule column, WebCheck), encode the FEC's own expected
  results as tests. A rule we implement differently from the FEC is a bug
  in one of us; the test tells us which.
* **Round trips**: anything that writes must round-trip through the
  parser (`parse(write(parse(f))) == parse(f)`) on every fixture, plus
  property tests (`proptest`) over arbitrary field values.
* **Negative tests**: every error variant has a test that provokes it.
* Integration tests that need Postgres self-skip without
  `HARDMONEY_TEST_DATABASE_URL` (except under GitHub Actions, where a
  missing database is a failure) and use unique names per test (they run
  in parallel).

### 6. Dependencies

Add a crate when it is popular, actively maintained, and replaces
non-trivial code we would otherwise own. Current deliberate choices:
`rust_decimal` (money), `chrono` (dates), `compact_str` (inline small
strings), `strum` (enum derives), `regex`, `csv`, `thiserror`, `sqlx`,
`axum`, `clap`, `proptest` (dev). Keep the parser-only build
(`--no-default-features --features fetch`) light: no `tokio`, `sqlx`, or
`axum` there.

### 7. Style

* `cargo fmt`, `cargo clippy --all-features --all-targets -- -D warnings`,
  `RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps`, and
  `cargo test --all-features` must all pass before a commit.
* `#[must_use]` on pure functions returning a value.
* Errors are `thiserror` enums with the source attached and a `line_no`
  where one exists. Messages are complete sentences a filer could act on.
* Field names use the canonical `lower_snake_case` names from
  `data/fec-csv-sources/`; FEC labels are for display.

## Repository map

| Path | What |
|---|---|
| `build.rs` | Compiles `data/` into `$OUT_DIR/tables.rs` (`Table`, layouts, field constants, specs). |
| `data/fec-csv-sources/` | Column positions per spec version (from fech-sources, with fixes listed in `NOTICE`). |
| `data/fec-spec/` | The FEC's spec workbook and its distilled JSON (`scripts/distill_fec_spec.py`). |
| `src/parser/schema.rs` | `SpecVersion`, `Layout`, `FieldSpec`, `Field<T>`, `Typed<T>`. |
| `src/parser/filing.rs` | `Filing`, `ParsedLine`, strict/lenient parsing. |
| `src/parser/typed.rs` | Domain views (`ScheduleA`, ...). |
| `src/bulk/`, `src/db/` | Postgres ETL and migrations. |
| `src/api/` | Axum REST API. |
| `src/cli/` | The `hardmoney` binary. |
| `tests/` | Real-filing fixtures, integrity checks, Postgres integration. |
| `book/` | The mdBook (tutorials + reference). |

## Release checklist

```bash
cargo fmt --all --check
cargo clippy --all-features --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps
cargo test --all-features
cargo test --no-default-features --features fetch
cargo package --all-features
mdbook build book
```

Then bump `Cargo.toml`, add a `CHANGELOG.md` section, tag `vX.Y.Z`, push,
`cargo publish --all-features`.
