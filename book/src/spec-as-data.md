# The spec as data

The FEC defines its electronic filing format in a spreadsheet. hardmoney
compiles that spreadsheet into the crate and publishes it back out in three
machine-readable forms (JSON, JSON Schema, CSV), diffs it across versions,
and checks it against the FEC's sources every week. This chapter says what
the FEC publishes, what hardmoney publishes, how a vendor or the FECfile+
team can consume the JSON Schema, and exactly where every byte comes from.

## What the FEC publishes

**The workbook.** The *Electronic Filing Specification Requirements,
Part II* is an Excel file,
[`FEC_EFO_Format_Specifications.xlsx`](https://docquery.fec.gov/formatspecs/FEC_EFO_Format_Specifications.xlsx)
(v8.5, dated 2025-01-28, last modified 2025-09-02), with one sheet per form
or schedule. Each row is one column of the record: `COL SEQ`, `FIELD
DESCRIPTION`, `TYPE` (`A/N-200`, `AMT-12`, `NUM-8`), `REQUIRED` (`X
(error)`, `X (warning)`, or conditional prose), `SAMPLE DATA`, `VALUE
REFERENCE`, `RULE REFERENCE`, `FIELD-FORM ASSOCIATION`. Code lists and
formats are prose (`[CAN|CCM|COM|IND|ORG|PAC|PTY]`, `01 ... 99`). This is
the only form in which the 85 registered filing vendors receive the format.
The download is the current version; the FEC publishes no cross-version
column map.

**The JSON Schemas in fecfile-validate.** For FECfile+ the FEC maintains
[fecgov/fecfile-validate](https://github.com/fecgov/fecfile-validate):
JSON Schema draft-07 documents under `schema/`. Of the 135 files at the top
of that directory, 15 describe wire records (`HDR`, `F1M`, `F24`, `F3`,
`F3X`, `F99`, `SchA` through `SchF`, `Text`) and embed the spreadsheet row
for each property as `fec_spec` (`COL_SEQ`, `TYPE`, `REQUIRED`, ...) next
to machine-readable `enum` and `pattern` constraints; the other 120
describe FECfile+ transaction types (`INDIVIDUAL_RECEIPT`, ...). They cover
spec 8.5 only and only the forms FECfile+ files. The repository's
spreadsheet-to-schema consistency checker "has not been run in a long time"
([fecfile-validate#302](https://github.com/fecgov/fecfile-validate/issues/302)),
so nothing today tells the FEC when the workbook and the schemas disagree.

They do disagree in small ways. One found while building this chapter:
`SchE.json` gives the support/oppose code the pattern `[S|O]` with
`maxLength: 1`. In a regular expression `|` inside brackets is a literal,
and the pattern is unanchored, so the schema accepts `|` as a code. The
workbook's value reference `[S|O]` means S or O. hardmoney carries the
FEC's pattern verbatim in `x-fec.pattern` (with `maxLength: 1` the effect
is the same) and its validator checks S or O explicitly.

## What hardmoney publishes

`build.rs` compiles the workbook and the fecfile-validate constraints into
the crate (see [The schema](./library-schema.md)); `hardmoney spec` exposes
the result. None of it needs a database or a network.

| Command | What it is | For whom |
|---|---|---|
| `hardmoney spec export` | The raw data model as one JSON document: every table, every version-bucketed column layout since 2001, every 8.5 spec row; 0-based columns. About 1.7 MB. | Generating bindings; checking a vendor's own tables |
| `hardmoney spec export --format json-schema` | One JSON Schema (draft 2020-12) per table under `$defs`, describing a record as an object keyed by canonical field name; `--table SchA` for one standalone document; `--version 7.0` for an older layout. | Vendors and FECfile+: validate a record with any JSON Schema library |
| `hardmoney spec export --format csv` | One flat row per table, version bucket, and field (7,361 rows) with the FEC's spec columns joined: the spreadsheet given back as a spreadsheet, complete across versions. | Spreadsheet users; the per-version field inventory `openFEC#6717` asks for |
| `hardmoney spec diff 8.4 8.5` | Fields added, removed, and moved between two versions. | Anyone tracking a format bump |
| `hardmoney spec fields SchA --version 6.4` | One table at one version, as a table. | Reading |

The [CLI reference](./cli-reference.md#spec-export) has the options and
captured output for each.

**The weekly drift job.** `.github/workflows/spec-drift.yml` runs every
Monday (and on any pull request touching the spec data, the distiller, or
the exporters). It checks out `fecgov/fecfile-validate` at `develop`,
re-distils the workbook against those schemas, and compares the result with
the checked-in `data/fec-spec/spec-8.5.json`. Every run, drifted or not,
uploads one artifact named `spec-drift-<date>` containing:

- `drift.md`: a Markdown report of what changed between the checked-in spec
  and the FEC's current sources (tables added or removed, fields added or
  removed per table, and a table of per-field attribute changes: type,
  length, required level, rule text, code list, pattern), from
  `scripts/distill_fec_spec.py --diff-report`;
- `spec-8.5.regenerated.json` and `spec.diff`: the fresh distillation and
  its unified diff against the checked-in file;
- `fec-spec-8.5.schema.json`: the JSON Schema bundle hardmoney ships;
- `fec-spec.csv`: the CSV inventory.

The same report is written to the run's step summary, with the upstream
commit it was checked against. On drift the job fails, and the fix is a
data change: review `drift.md`, commit the regenerated JSON. Artifacts are
under the workflow's run page on GitHub (Actions, "FEC spec drift") for
the repository's artifact retention period (90 days by default); a
maintainer can attach `drift.md` to fecfile-validate#302 without anyone at
the FEC downloading a sheet.

## Consuming the JSON Schema

The schema for one table describes one record as an object whose keys are
the canonical field names and whose values are strings as filed. Build the
record dict from a `.fec` line (hardmoney's Python `Line.to_dict()` does
that) or from your own writer, and validate it with the
[`jsonschema`](https://pypi.org/project/jsonschema/) package:

```python
import json, subprocess
from jsonschema import Draft202012Validator
import hardmoney

schema = json.loads(subprocess.check_output(
    ["hardmoney", "spec", "export", "--format", "json-schema", "--table", "SchA"]))
validator = Draft202012Validator(schema)
record = hardmoney.parse_file("tests/fixtures/F3XN_2011831.fec").lines_for("SchA")[0].to_dict()
print("as filed:", [e.message for e in validator.iter_errors(record)])
record.update(contributor_state="Texas", entity_type="INDIVIDUAL", donor_candidate_district="1")
for e in validator.iter_errors(record):
    print(f"{'/'.join(e.path)}: {e.message}")
```

```text
as filed: []
entity_type: 'INDIVIDUAL' is too long
contributor_state: 'Texas' is too long
donor_candidate_district: '1' does not match '^$|^\\d{2}$'
```

To use the whole format at once, export the bundle and pick a table:
`Draft202012Validator(bundle["$defs"]["SchB"])`. In a schema that
references it, `{"$ref": "#/$defs/SchB"}`. The bundle has no `type` of its
own: a record dict would satisfy several tables' schemas at once (most
fields are optional and `additionalProperties` is not set), so there is no
useful "any table" schema and the bundle does not pretend to be one.

### What a schema says, and what it deliberately does not

The rendering is `hardmoney::parser::jsonschema` (`table_schema`,
`bundle`); the rules below are its documentation, repeated here because
they are the design decisions a consumer needs to know.

- **Every field is `type: string`.** Dates are `YYYYMMDD` strings and
  amounts are decimal strings on the wire; the schema does not invent
  formats the workbook does not state.
- **`maxLength` is the FEC's length**, except for `AMT-n` fields, where it
  is `n + 2`. The FEC bounds the digits of an amount, not the characters:
  it accepted a twelve-digit total written in thirteen characters
  (`2280311229.59`, FEC-1458871, an `AMT-12` field). `x-fec.max_len` keeps
  `n` and `x-fec.max_length_note` says so.
- **`enum` and `pattern` appear only at the version the workbook
  describes** (8.5, `x-hardmoney.field_rules_version`). At any other
  version the field rules are joined to that version's columns by field
  name, which is right for descriptions and lengths (no FEC-accepted fixture
  from 3.00 to 8.2 exceeds an 8.5 length) and wrong for code lists and
  formats: accepted 3.00 filings carry one-digit candidate districts, which
  fail the 8.5 pattern `^\d{2}$`. So at 7.0 the district property has no
  `pattern`, and `x-fec.pattern_omitted_reason` explains; the FEC's value
  is still in `x-fec.pattern`. The validator makes the same call
  (`Rule::demoted_on_superseded_format`).
- **Blank is always allowed.** `enum` includes `""` and `pattern` is the
  FEC's pattern with `^$|` in front (`x-fec.pattern` is the FEC's string
  verbatim). Nearly every coded or formatted field is optional or
  conditionally required (a donor candidate ID is blank on almost every
  Schedule A line), and the FEC checks a format only when a value is
  present. Without this, the schema would reject most accepted records.
- **`required` means present, not non-blank.** It lists the fields whose
  workbook requirement is `X (error)`. The schema does not add
  `minLength: 1` to them, because many `X (error)` rows are conditional in
  their rule text (`contributor_organization_name` is "Required if NOT
  [IND|CAN]") and a blank there is accepted. Whether a blank is acceptable
  is the requirement logic in `Filing::validate`, which reads the rule
  text; the schema carries the FEC's level in `x-fec.required_level` and
  the condition in `x-fec.required_condition`.
- **Cross-field rules are prose.** `= 11ai + 11aii` and `Required if NOT
  [IND|CAN]` are in `x-fec.rule`, not as `if`/`then`. `Filing::reconcile`
  and `Filing::validate` implement them.
- **`additionalProperties` is not set.** A record carrying a line number or
  a vendor's own key still validates.
- **A field the current workbook has no row for** (a field dropped before
  8.5, seen at an older version; or a table the workbook no longer
  documents, such as Schedule I) is `type: string` with `x-fec: {column,
  spec_row: false}`.
- **Four workbook tables have no schema at 8.5.** `F3Z1`, `F3Z2`, `F3PZ1`,
  and `F3PZ2` have 8.5 spec rows but no 8.5 column layout in hardmoney's
  layout data, so `table_schema` returns nothing for them at 8.5 and the
  bundle has 49 `$defs`, not 51. `spec export --format json` still carries
  their spec rows.

The contract that these choices enforce is tested: `tests/spec_export.rs`
checks every record of every real filing in `tests/fixtures/` (7,138
records, spec 3.00 through 8.5) against the schema for its table at its
version, once with a small in-crate reading of `maxLength`, `enum`,
`pattern`, and `required`, and once (under `HARDMONEY_ORACLE_TESTS=1`)
with the `jsonschema` package, which also checks every emitted document
against the draft 2020-12 metaschema. A schema that rejected FEC-accepted
data would fail the build, the same discipline the validator is held to.

### The document, by example

The first 40 lines of `hardmoney spec export --format json-schema --table SchA`:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "SchA record, FEC spec 8.5",
  "description": "One SchA record at FEC electronic filing spec version 8.5, as an object keyed by canonical field name. Every value is a string as filed (trimmed); a blank field is the empty string. x-fec.column is the 1-based position in the delimited line.",
  "type": "object",
  "properties": {
    "form_type": {
      "type": "string",
      "description": "FORM TYPE. Rule: Appendix C. SA3L must be used with the F3L",
      "maxLength": 8,
      "x-fec": {
        "column": 1,
        "type": "A/N-8",
        "kind": "alpha_numeric",
        "max_len": 8,
        "required_level": "error",
        "sample": "SA11AI",
        "value_reference": "SA[line# ref]",
        "rule": "Appendix C.  SA3L must be used \nwith the F3L",
        "forms": [
          "F3",
          "F3X",
          "F3P",
          "F3L"
        ],
        "allowed_values": [],
        "pattern": null
      }
    },
    "filer_committee_id_number": {
      "type": "string",
      "description": "FILER COMMITTEE ID NUMBER",
      "maxLength": 9,
      "x-fec": {
        "column": 2,
        "type": "A/N-9",
        "kind": "alpha_numeric",
        "max_len": 9,
        "required_level": "error",
        "sample": "C00123456",
```

Further down, a field with a format, and the document's own metadata:

```json
    "donor_candidate_district": {
      "type": "string",
      "description": "DONOR CANDIDATE DISTRICT. Rule: Req if Office = House",
      "maxLength": 2,
      "pattern": "^$|^\\d{2}$",
      "x-fec": {
        "column": 36,
        "type": "NUM-2",
        "kind": "numeric",
        "max_len": 2,
        "required_level": "conditional",
        "required_condition": "Conditional Warning",
        "sample": "35",
        "value_reference": "01 ... 99",
        "rule": "Req if Office = House",
        "forms": ["F3", "F3X", "F3P", "F3L"],
        "allowed_values": [],
        "pattern": "^\\d{2}$"
      }
    },
    ...
  },
  "required": ["form_type", "filer_committee_id_number", "transaction_id", "entity_type",
               "contributor_organization_name", "contributor_last_name", "contributor_first_name"],
  "x-hardmoney": {
    "spec_version": "8.5",
    "table": "SchA",
    "layout_versions": ["8.0", "8.1", "8.2", "8.3", "8.4", "8.5"],
    "field_rules_version": "8.5",
    "column_base": 1,
    "width": 45,
    "hardmoney_version": "3.0.1"
  }
}
```

(The second excerpt is reformatted onto fewer lines; values are as
emitted.) `x-fec.type` is the workbook's type code rebuilt from kind and
length; the workbook writes both `N-8` and `NUM-8` for numerics and the
distillation keeps only the kind, so numerics come back as `NUM-n`.

## Provenance

Every value in a schema traces to a public-domain FEC document through
five steps, each of them in the repository:

1. **The workbook.** `data/fec-spec/FEC_EFO_Format_Specifications_v8.5.xlsx`,
   vendored from docquery.fec.gov. Optionally, the `schema/*.json` files of
   a `fecgov/fecfile-validate` checkout, for `enum` and `pattern`.
2. **`scripts/distill_fec_spec.py`** reads every sheet (`SHEET_TO_TABLE`
   maps sheet names to table names), parses `TYPE` into a kind and a
   length and `REQUIRED` into a level, joins the fecfile-validate
   constraints by `(table, COL_SEQ)` while dropping the FEC's generic
   "any printable ASCII" patterns and its internal ISO date patterns, and
   writes
3. **`data/fec-spec/spec-8.5.json`**: 51 tables, 1,949 rows, one object per
   workbook row (`column` is 1-based here, as in the workbook). This file
   is what the drift job regenerates and diffs.
4. **`build.rs`** reads the newest `spec-*.json` and the column tables in
   `data/fec-csv-sources/` and generates `$OUT_DIR/tables.rs`: the `Table`
   enum, one `Layout` per version bucket per table, and one `FieldSpec`
   static per workbook row, with the canonical field name attached by
   matching the row's column to the 8.5 layout. Inconsistent data (two
   fields at one column, overlapping version buckets) fails the build.
5. **The statics** (`Table::layouts()`, `Table::specs()`) are what
   `hardmoney::parser::jsonschema::table_schema` reads, and
   `hardmoney spec export --format json-schema` prints what it returns.
   The validator (`Filing::validate`) reads the same statics, so the
   schema and the validator cannot disagree about a length or a code list.

When the FEC publishes 8.6, the change is: drop the new workbook into
`data/fec-spec/`, run the distiller, add a column table for any table whose
layout changed, rebuild. `hardmoney spec diff 8.5 8.6` then says what moved,
and `spec export --format json-schema --version 8.6` describes the new
records.
