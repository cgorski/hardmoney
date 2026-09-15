# The schema: versions, layouts, and compile-time-checked fields

The previous two chapters treated a `.fec` file as something the parser
does things to. This one is about the data the parser does them with:
the description of the format itself. hardmoney compiles the FEC's
per-version column positions and the FEC's own field specifications
into static Rust data, and exposes that data through a handful of types.
With them you can answer questions like "where does `contribution_amount`
live in a 6.4 filing?" without opening a spreadsheet, and the compiler
catches you asking an F3X cover page for a Schedule A field.

Everything in this chapter lives in `hardmoney::parser::schema` and
`hardmoney::parser::tables`; the commonly used names (`SpecVersion`,
`Table`, `Field`, `Typed`, `ParsedLine`) are re-exported at the crate
root. Every Rust snippet below was compiled and run against the source
tree, and the assertions shown all hold.

## Where the data comes from

Three inputs, all in the repository's `data/` directory, are compiled by
`build.rs` into `$OUT_DIR/tables.rs` on every build, so nothing here
can be stale relative to the data:

| Input | What it contributes | Rust surface |
|---|---|---|
| `data/fec-csv-sources/*.csv` | for each table, the canonical field name at each column, bucketed by spec version (fech-sources lineage, with the fixes in `NOTICE`; `F2S.csv` authored locally) | `Table`, `Layout`, `FieldDef`, the `tables::<table>` modules |
| `data/fec-spec/spec-8.5.json` | the FEC's "Electronic Filing Specification Requirements, Part II" workbook, distilled: type, length, required level, sample, rule text, allowed values | `FieldSpec`, `FieldKind`, `Requirement`, `BUNDLED_SPEC_VERSION` |
| the two together | which canonical field each spec row describes | `FieldSpec::canonical` |

The generator refuses inconsistent data (two fields at one column, one
field at two columns, overlapping version buckets), which is why the
project's rule is "data, not code, for facts about the format": a column
number is never typed into a `.rs` file by hand. The same compiled data
is what [`hardmoney spec`](./cli-reference.md#spec) prints. The canonical
names themselves, the rules they follow and every field of every table with
its FEC label, are the subject of the next chapter, [Field names](./field-names.md).

## `SpecVersion`

A filing's header names the spec version it was written to, and every
column position in the file depends on it. `SpecVersion` is that version
as a value, not a string:

```rust
use hardmoney::SpecVersion;

let v: SpecVersion = "8.5".parse()?;
assert_eq!(v, SpecVersion::electronic(8, 5));
assert_eq!("3.00".parse::<SpecVersion>()?, SpecVersion::electronic(3, 0));
assert_eq!("8.5.0.1".parse::<SpecVersion>()?.to_string(), "8.5");
assert!(SpecVersion::electronic(8, 5) > SpecVersion::electronic(6, 4));
assert!(SpecVersion::paper(3, 4) > SpecVersion::electronic(8, 5));
assert!(SpecVersion::paper(3, 4).is_paper());
assert!(v.uses_fs_delimiter());
assert!(!SpecVersion::electronic(5, 3).uses_fs_delimiter());
assert!("8.a".parse::<SpecVersion>().is_err());
# Ok::<(), Box<dyn std::error::Error>>(())
```

Parsing is forgiving about spelling and strict about shape. `3.00`,
`3.0`, and `3` are one version; `8.5.0.1` is a build of `8.5` (only the
first minor digit is significant, which is how the FEC has always
numbered releases). Letters, empty components, and negative numbers are
errors, not versions. A well-formed but unknown version like `180.5`
(seen in a real corpus file) parses here and is rejected later by the
header parser, with the line number.

Ordering is numeric, so `8.5 > 8.4 > 7.0 > 6.4` and you can write
`if filing.version >= SpecVersion::electronic(8, 0)`. Paper-conversion
versions (`P3.4`, produced when the FEC keys in a paper report) sort
after every electronic one.

The wire spelling is not preserved. `Display` prints `3.0`, not `3.00`.
If you need the exact header string, it's in
`filing.header.fec_version_raw`.

`uses_fs_delimiter()` and `has_name_delim_header()` are the two
format-era facts the parser itself needs (ASCII-28 vs. comma delimiting,
and the extra header column that 3.x-5.x carried).

`SpecVersion` implements `FromStr`, `Display`, `Ord`, `Hash`, `Copy`, and
(with the `serde` feature) serializes as its string form, so it works as
a map key, a CLI argument, and a JSON value without conversion.

## `Table` and `Layout`

A `Table` is one format table: a form, schedule, or record type with
its own column layout. `Table::F3X`, `Table::SchA`, `Table::Text`, 59 in
all (`Table::ALL`). For each table, the compiled data holds one `Layout`
per version bucket, a run of spec versions across which the columns did
not change.

```rust
use hardmoney::{SpecVersion, Table};

let v85 = SpecVersion::electronic(8, 5);
let layout = Table::SchA.layout(v85).expect("SchA has an 8.5 layout");
assert!(layout.supports(SpecVersion::electronic(8, 0))); // same bucket
assert_eq!(layout.width, 45);
assert_eq!(layout.fields.len(), 45);

let amount = layout.field("contribution_amount").unwrap();
assert_eq!(amount.column, 20); // 0-based: the 21st cell on the wire
assert!(layout.field("contribution_purpose_code").is_none()); // dropped in 8.0

let old = Table::SchA.layout(SpecVersion::electronic(7, 0)).unwrap();
assert_eq!(old.field("contribution_purpose_code").unwrap().column, 22);
assert_eq!(old.field("contributor_employer").unwrap().column, 24);
assert_eq!(layout.field("contributor_employer").unwrap().column, 23);

assert!(Table::SchI.layout(v85).is_none()); // Schedule I ended with 8.4
assert!(Table::SchI.supports_version(SpecVersion::electronic(8, 4)));

for l in Table::SchA.layouts() {
    println!(
        "{} .. {}: {} fields in {} columns",
        l.versions.first().map(ToString::to_string).unwrap_or_default(),
        l.versions.last().map(ToString::to_string).unwrap_or_default(),
        l.fields.len(),
        l.width
    );
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
8.0 .. 8.5: 45 fields in 45 columns
6.4 .. 7.0: 46 fields in 46 columns
6.2 .. 6.3: 47 fields in 47 columns
6.1 .. 6.1: 46 fields in 46 columns
5.3 .. 5.3: 44 fields in 44 columns
5.2 .. 5.2: 44 fields in 44 columns
5.1 .. 5.1: 44 fields in 44 columns
5.0 .. 5.0: 38 fields in 38 columns
2.0 .. 3.9: 37 fields in 37 columns
1.0 .. 1.9: 32 fields in 32 columns
P3.2 .. P3.4: 24 fields in 24 columns
P2.6 .. P3.1: 23 fields in 23 columns
P1.0 .. P2.4: 24 fields in 24 columns
```

Thirteen layouts for one schedule. That is the whole reason a version
string is not enough and a `Layout` is what a line is parsed against.
The fields of a `Layout`:

| Field | Type | Meaning |
|---|---|---|
| `table` | `Table` | which table this is a layout of |
| `versions` | `&[SpecVersion]` | every version this bucket applies to (`supports(v)` checks membership) |
| `fields` | `&[FieldDef]` | `{ name, column }` for every canonical field, in the order the FEC lists them |
| `width` | `u16` | one more than the highest column used; the number of cells a writer must emit |

`Layout::field(name)` and `Layout::index_of(name)` are binary searches
over a precomputed name index, so per-field lookup on a hot path is
cheap. Columns are 0-based in the Rust API and the `spec export` JSON,
matching how the parser splits a record; the human-facing `spec fields`
and `spec diff` commands number them from 1 like the FEC's workbook
does.

`hardmoney spec diff 7.0 8.5 --table SchA` shows the same 7.0-to-8.5
change the snippet above probes by hand: `contribution_purpose_code`
dropped, everything after it shifted left by one.

## `FieldSpec`: what the FEC says about a field

Layouts say where a field is. `FieldSpec` says what the FEC says it is:
the row from the spec workbook, at the version named by
`BUNDLED_SPEC_VERSION`.

```rust
use hardmoney::parser::{BUNDLED_SPEC_VERSION, FieldKind, Requirement};
use hardmoney::parser::tables::sch_a;
use hardmoney::Table;

assert_eq!(BUNDLED_SPEC_VERSION, "8.5");

let spec = Table::SchA.spec("contribution_amount").unwrap();
assert_eq!(spec.description, "CONTRIBUTION AMOUNT {F3L Bundled}");
assert_eq!(spec.kind, FieldKind::Amount);
assert_eq!(spec.max_len, Some(12));
assert_eq!(spec.required, Requirement::Warning);
assert_eq!(spec.column, 20);

let receipts = Table::F3X.spec("col_a_total_receipts").unwrap();
assert_eq!(receipts.rule, Some("= 19"));

let entity = Table::SchA.spec("entity_type").unwrap();
assert_eq!(entity.required, Requirement::Error);
assert_eq!(entity.max_len, Some(3));

// Same row, reached through the field constant.
assert_eq!(sch_a::CONTRIBUTION_AMOUNT.spec(), Some(spec));
assert!(Table::SchI.specs().is_empty());
# Ok::<(), Box<dyn std::error::Error>>(())
```

| Field | Meaning |
|---|---|
| `column` | 0-based column at the bundled version |
| `canonical` | the canonical field name at that column, if the layout tables name it (a few spec columns are unnamed placeholders) |
| `description` | the FEC's label, e.g. `"CONTRIBUTOR ORGANIZATION NAME"`; for display, use `canonical` in code |
| `kind` | `FieldKind::{Alpha, AlphaNumeric, Numeric, Amount, Unknown}` from the workbook's `TYPE` column (`A/N-200`, `AMT-12`, ...) |
| `max_len` | the `n` in `A/N-n` |
| `required` | `Requirement::{None, Error, Warning, Conditional(text)}`: how hard the FEC's validator fails a blank |
| `sample`, `value_reference`, `rule` | the workbook's example value, allowed-values prose (`"[IND|ORG|COM]"`), and rule text (`"= 11ai + 11aii"` on a total) |
| `forms` | for a schedule column, the parent forms it applies to |
| `allowed_values`, `pattern` | machine-readable closed sets and regexes, where the FEC publishes them |

This is the data `hardmoney validate` checks a filing against (length,
type, required level, allowed values), and the `= 19`-style rule text is
what a cover-page reconciler needs to know which schedule lines a total
is supposed to equal. Tables absent from the current spec (Schedule I,
Form 8, Form 10) have layouts but no spec rows, which `hardmoney spec
tables` shows as `spec_rows 0`.

Both `FieldKind` and `Requirement` are `#[non_exhaustive]`: match them
with a wildcard arm.

## The generated `tables::<table>` modules and `Field<T>`

Here is where the schema stops being lookup tables and starts being
types. For every table, `build.rs` generates a module named in
`snake_case` (`tables::sch_a`, `tables::f3x`, `tables::hdr`, ...)
containing:

- a zero-sized marker type with the table's name (`sch_a::SchA`,
  `f3x::F3X`), also re-exported from `tables::markers`;
- one `Field<Marker>` constant per canonical field, in
  `UPPER_SNAKE_CASE`: `sch_a::CONTRIBUTION_AMOUNT`,
  `f3x::COL_A_TOTAL_RECEIPTS`, `sch_e::SUPPORT_OPPOSE_CODE`. Each is
  documented with the FEC's description, so IDE completion on `sch_a::`
  is a browsable field list;
- the table's `LAYOUTS`, `SPECS`, and `FIELD_NAMES` statics that
  `Table::layouts()`, `Table::specs()`, and `Table::field_names()`
  dispatch to.

A `Field<T>` is a canonical field name statically tied to the table it
belongs to:

```rust
use hardmoney::parser::tables::{f3x, sch_a};
use hardmoney::Table;

assert_eq!(sch_a::CONTRIBUTION_AMOUNT.name(), "contribution_amount");
assert_eq!(sch_a::CONTRIBUTION_AMOUNT.table(), Table::SchA);
assert_eq!(format!("{:?}", f3x::COL_A_TOTAL_RECEIPTS), "F3X.col_a_total_receipts");
assert_eq!(f3x::COL_A_TOTAL_RECEIPTS.to_string(), "col_a_total_receipts");
# Ok::<(), Box<dyn std::error::Error>>(())
```

`Field::name()` gives you the plain string for any API that takes one
(`line.get(sch_a::CONTRIBUTION_AMOUNT.name())`), and `Field::spec()` is
the `FieldSpec` shortcut you saw above. The constants are `const`, so
they can sit in your own `static` tables and `match` arms.

## `Typed<'_, T>`: the compiler checks the table

A `Typed<'_, T>` is a `ParsedLine` that has been checked (at
construction, at runtime) to belong to table `T`, and whose accessors
therefore only accept `Field<T>`. Asking it for a field of a different
table is a compile error, not a `None`:

```rust
use hardmoney::Filing;
use hardmoney::parser::tables::markers::{F3X, SchA};
use hardmoney::parser::tables::{f3x, sch_a};
use hardmoney::Table;

let filing = Filing::open("tests/fixtures/F3XN_2011834.fec")?;

let cover = filing.summary_as::<F3X>()?;
let receipts = cover.money(f3x::COL_A_TOTAL_RECEIPTS);
assert_eq!(receipts.map(|d| d.to_string()), Some("30408.30".to_string()));
let from = cover.date(f3x::COVERAGE_FROM_DATE);
assert_eq!(from.map(|d| d.to_string()), Some("2026-08-01".to_string()));

let mut total = rust_decimal::Decimal::ZERO;
for line in filing.lines_for(Table::SchA) {
    let a = line.typed::<SchA>()?;
    if let Some(amount) = a.money(sch_a::CONTRIBUTION_AMOUNT) {
        total += amount;
    }
    let _who = a.get(sch_a::CONTRIBUTOR_LAST_NAME);   // Option<&str>, None if blank
    let _when = a.date(sch_a::CONTRIBUTION_DATE);     // Option<NaiveDate>
}
println!("Schedule A total: {total}");

// The wrong table at runtime is an error that names the line...
let a_line = filing.lines_for(Table::SchA).next().unwrap();
let err = a_line.typed::<F3X>().unwrap_err();
println!("{err}");
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
Schedule A total: 40961.43
line 3 is a SchA line, not F3X
```

...and the wrong table at compile time does not build at all:

```rust,ignore
let cover = filing.summary_as::<F3X>()?;
cover.money(sch_a::CONTRIBUTION_AMOUNT);
// error[E0308]: mismatched types
//   expected `Field<F3X>`, found `Field<SchA>`
```

The four accessors are `get` (the trimmed string, `None` if blank or if
this filing's version has no such column), `money` (exact `Decimal` via
`parse_money`), `date` (`NaiveDate` via `parse_fec_date`, so `00000000`
is `None`), and `string` (an owned copy). `Typed::line()` gets the
underlying `ParsedLine` back.

How this relates to the [typed views](./typed-views.md) chapter:
`ScheduleA`, `ScheduleB`, `ScheduleE`, and `Form3XSummary` are domain
views. They pick fields, resolve names across old and new formats, and
parse codes into enums, so they are what you want for the common
schedules. `Typed<T>` is the general mechanism, one per table, with no
interpretation beyond money and dates. It is how you read an `H4`
line's `FEDERAL_SHARE`/`NONFEDERAL_SHARE` split or an `F1M` line's
dates without waiting for someone to write a view for it, and it is how
the crate's own views and validator are written underneath.

## `ParsedLine`: `get`, `iter`, `set`, `from_pairs`

Every line, the cover line and every body line, is a `ParsedLine`: its
values as a flat slice, parallel to the fields of the `Layout` it was
parsed with. The layout is what makes the untyped string API precise
about absent versus blank, and what lets a line be edited and written
back at the right columns:

```rust
use hardmoney::{ParsedLine, SpecVersion, Table};

let mut line = ParsedLine::from_pairs(
    Table::SchA,
    SpecVersion::electronic(8, 5),
    0,
    [
        ("form_type", "SA11AI"),
        ("filer_committee_id_number", "C00123456"),
        ("contribution_amount", "250.00"),
    ],
)?;
assert_eq!(line.get("contribution_amount"), Some("250.00"));
assert_eq!(line.get("contributor_last_name"), Some(""));   // exists, blank
assert_eq!(line.get("contribution_purpose_code"), None);   // not in 8.5
assert_eq!(line.get_non_empty("contributor_last_name"), None);

line.set("contributor_last_name", "  Smith ")?;
assert_eq!(line.get("contributor_last_name"), Some("Smith"));
assert!(line.set("contribution_purpose_code", "x").is_err());

let filled: Vec<(&str, &str)> = line.iter().filter(|(_, v)| !v.is_empty()).collect();
assert_eq!(
    filled,
    [
        ("form_type", "SA11AI"),
        ("filer_committee_id_number", "C00123456"),
        ("contributor_last_name", "Smith"),
        ("contribution_amount", "250.00"),
    ]
);
assert_eq!(line.iter().len(), 45);
assert_eq!(line.raw_form_type, "SA11AI");
assert_eq!(line.table(), Table::SchA);
assert_eq!(line.layout().width, 45);
assert_eq!(line.to_cells().len(), 45);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`get(name)` returns `Some("")` for a field that exists in this layout
but is blank, and `None` for a field this version does not have at all.
That distinction is the one that matters when you're processing a 2003
filing with 2026 code: `None` means "don't look for it here," not "the
filer left it empty." `get_non_empty` folds the two together when you
don't care.

`iter()` yields every `(field, value)` in layout order, blanks included,
which is exactly what the writer emits and what `--lines` JSON shows.
`field_names()` is the same walk without values.

`set(name, value)` trims and stores, fails with `FecError::UnknownField`
if the layout has no such field, and keeps `raw_form_type` in step if
you change `form_type`. Together with `from_pairs` this is how you build
synthetic lines for tests, fix a field before writing a filing back out,
or construct records in an editor.

`from_pairs(table, version, line_no, pairs)` builds a line from names
and values; anything unspecified is blank, `form_type` defaults to the
table's name, and an unknown name is an error rather than a silently
dropped value.

`to_cells()` is the record in wire order (`layout.width` cells, with
unassigned columns blank), which is what a writer joins with the
version's delimiter to reproduce the line.

Field names are shared `&'static str`s from the layout, not per-line
`String`s, and values are stored inline when short (most FEC values
are), so a parsed 700,000-line presidential filing is compact in memory.

## Putting it together: version-aware code without version checks

The point of all this machinery is that most code never needs to look
at `filing.version`. A `Typed<SchA>` view over a 5.3 line and one over
an 8.5 line answer `a.money(sch_a::CONTRIBUTION_AMOUNT)` the same way,
because each line carries the layout that puts that field in the right
place; a field that a version lacks comes back `None`. When you do need
the version (to decide whether a split-name field can exist, say),
`layout.field(name).is_some()` asks the data instead of hard-coding
`>= 8.0`.

To see the full picture for any table from the terminal:

```bash
hardmoney spec tables                       # every table, buckets, fields, spec coverage
hardmoney spec fields SchA --version 6.4    # columns + FEC description/type/rule
hardmoney spec diff 5.3 6.1 --table SchA    # what changed between two versions
hardmoney spec export > fec-spec.json       # the whole thing, as data
```

See [CLI reference](./cli-reference.md#spec) for the output formats.
