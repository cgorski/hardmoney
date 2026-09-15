# Python

Everything in this book so far is Rust or the command line. This chapter
is the same parser, writer, validator, and reconciler from Python:

```python
import hardmoney

filing = hardmoney.parse_file("tests/fixtures/F3XN_2011831.fec")
for line in filing.lines_for("SchA"):
    print(line["contributor_last_name"], line.amount("contribution_amount"))

assert filing.validate().is_acceptable
assert filing.reconcile().balances
```

This chapter is the reference. For worked examples with their output
(top donors, memo entries, a CSV validation report, a CI exit code,
pandas and Parquet, a Django-style pre-submission check, and more), see
the [Python cookbook](./python-cookbook.md); the scripts are in
`python/examples/` in the repository.

## Why a Python package

Python is where FEC data users are. The Rust crate `feco3` has ten times
the downloads of any other Rust FEC crate, through its PyPI wheel rather
than its crate, and a wheel is the only realistic way hardmoney's
validator and reconciler reach the FEC's own Django codebase (FECfile+)
or the analysts, journalists, and researchers who work in notebooks and
`pandas`. So the `hardmoney` Python package is a thin
[PyO3](https://pyo3.rs) layer over the Rust crate's parser-only build:
no `tokio`, `sqlx`, or `axum` in the wheel, no runtime dependencies, and
one `abi3` wheel per platform that works on every CPython from 3.9 on.

What you get is exactly what the Rust library gives you, with Python
types where Python has the right ones: money is `decimal.Decimal` (built
from the exact string form of the Rust `Decimal`, never a `float`), dates
are `datetime.date`, records are mapping-like, and errors are exceptions
with a `line_no`.

## Install

```bash
pip install hardmoney            # once published to PyPI
```

From a checkout, with Rust and [maturin](https://www.maturin.rs)
installed:

```bash
cd python
python -m venv .venv && . .venv/bin/activate
pip install maturin pytest
maturin develop --release        # builds the extension into the venv
pytest -q                        # runs the suite over tests/fixtures/*.fec
```

The package lives in `python/` in the repository: a standalone Cargo
crate (`hardmoney-py`, producing the `_hardmoney` extension module) that
depends on the parent crate with `default-features = false, features =
["fetch", "serde"]`, plus the pure-Python `hardmoney` package
(`__init__.py`, type stubs in `_hardmoney.pyi`, `py.typed`).

## Parsing

Three entry points, all returning a `Filing`:

| | |
|---|---|
| `hardmoney.parse(data, *, lenient=False)` | from `bytes` (UTF-8, falling back to Windows-1252) or `str` |
| `hardmoney.parse_file(path, *, lenient=False)` | from disk, streamed through `FilingReader` so peak memory is the parsed lines alone |
| `hardmoney.fetch(filing_id)` | downloaded from `docquery.fec.gov/dcdev/posted/<id>.fec` |

Parsing releases the GIL, so other Python threads keep running while a
135 MB presidential filing is read.

```python
import hardmoney

filing = hardmoney.parse_file("tests/fixtures/F3XN_2011831.fec")
print(filing)
print(filing.form_type, filing.base_form_type, filing.version, filing.is_amendment, filing.amends_filing)
print(filing.header)
```

```text
<hardmoney.Filing F3XN v8.5 (144 body lines)>
F3XN F3X 8.5 False None
{'record_type': 'HDR', 'ef_type': 'FEC', 'fec_version_raw': '8.5', 'version': '8.5', 'soft_name': 'Vocus PAC Management', 'soft_ver': '8.02.1066', 'report_id': '', 'report_number': '0', 'comment': ''}
```

`header` is a `dict` keyed like the Rust `Header` struct; spec 3.x-5.x
filings carry one extra key, `name_delim`. `amends_filing` is the
original filing number from an amendment's `FEC-<n>` report id, or
`None`.

### Lines

`filing.summary` is the cover line; `filing.lines` is every body line in
file order; `filing.lines_for("SchA")` is one table's worth; and
`filing.iter_lines(["SchA", "SchB"])` iterates a selection. A `Line`
behaves like a read-mostly mapping from canonical field name (the
`lower_snake_case` names from `data/fec-csv-sources/`) to the value as
filed: trimmed, otherwise verbatim, as [Fidelity](./fidelity.md)
describes.

```python
cover = filing.summary
print(cover)
print(cover["committee_name"])
print(cover.amount("col_a_total_receipts"), cover.date("coverage_from_date"), cover.date("coverage_through_date"))

for line in filing.lines_for("SchA")[:3]:
    print(line.line_no, line.form_type, line["contributor_last_name"], line["contributor_first_name"],
          line.amount("contribution_amount"), line.date("contribution_date"), line.is_memo)

total = sum(l.amount("contribution_amount") for l in filing.lines_for("SchA") if not l.is_memo)
print(total, total == cover.amount("col_a_individuals_itemized"))
```

```text
<hardmoney.Line F3XN (F3X) line 2>
FirstEnergy Corp Political Action Committee
15245.52 2026-08-01 2026-08-31
3 SA11AI Mroczynski Mark 380.00 2026-08-31 False
4 SA11AI Fickey Karl 120.00 2026-08-31 False
5 SA11AI Rossero Daniel 200.00 2026-08-31 False
13736.02 True
```

That last line is the whole point of `Decimal`: 132 contributions summed
exactly equal the cover page's 11(a)(i), which they would not reliably
do in binary floating point.

The mapping protocol, precisely:

- `line[name]` is the value as filed, `""` when blank, and `KeyError`
  when the field is not in this filing's layout for the table (a typo, or
  a field that did not exist in that spec version).
- `name in line`, `line.get(name, default=None)`, `line.keys()`,
  `line.items()`, `line.to_dict()`, all in layout order, blanks
  included. `get` returns the default only for a missing field; a blank
  one is `""`.
- `line.amount(name)` is a `decimal.Decimal` with scale 2, or `None` when
  the field is blank or not a valid FEC amount (`$5,500.00` is `None`;
  the validator will tell you why). `line.date(name)` is a
  `datetime.date`, or `None` when blank, zero-filled, or not a real date.
  Both raise `KeyError` for an unknown field, like `line[name]`.
- `line.table`, `line.form_type` (the token upper-cased; the `form_type`
  field is as filed), `line.line_no`, `line.is_memo`.

```python
line = filing.lines[0]
print(line.keys()[:6])
print(line.get("contributor_employer"), line.get("no_such_field", "n/a"))
try:
    line["no_such_field"]
except KeyError as e:
    print("KeyError:", e)
```

```text
['form_type', 'filer_committee_id_number', 'transaction_id', 'back_reference_tran_id_number', 'back_reference_sched_name', 'entity_type']
FirstEnergy n/a
KeyError: 'no_such_field'
```

## Editing and writing back

`line.set(name, value)` puts a value in the right column for the filing's
spec version (trimmed like the parser; `KeyError` for an unknown field),
and `filing.to_fec()` / `filing.to_fec_string()` write the canonical
`.fec` form described in [Writing `.fec` files](./writing-fec.md). A
`Line` is a handle into its `Filing`, not a copy, so an edit through
any handle is what the writer emits:

```python
filing = hardmoney.parse_file("tests/fixtures/F3XA_2011827.fec")
changed = 0
for line in filing.lines_for("SchA"):
    if line["contributor_employer"] == "SELF":
        line.set("contributor_employer", "Self-employed")
        changed += 1
try:
    filing.lines[0].set("no_such_field", "x")
except KeyError as e:
    print("KeyError:", e)

out = filing.to_fec()
print(changed, "line(s) changed;", len(open("tests/fixtures/F3XA_2011827.fec", "rb").read()), "bytes in,", len(out), "bytes out")
again = hardmoney.parse(out)
print([l.to_dict() for l in again.lines] == [l.to_dict() for l in filing.lines], again.to_fec() == out)
for l in again.lines_for("SchA"):
    print(l.line_no, repr(l["contributor_employer"]))
```

```text
KeyError: 'no_such_field'
1 line(s) changed; 1867 bytes in, 1885 bytes out
True True
3 'Self-employed'
```

The round-trip guarantee holds from Python as it does from Rust: for every
fixture in `tests/fixtures/`, `parse(to_fec(parse(f)))` has the same
header, cover, and body lines field for field, and writing the re-parsed
filing gives the same bytes (`python/tests/test_write.py`). `to_fec()`
returns Windows-1252 when every character is representable and UTF-8
otherwise; `to_fec_string()` is the text before encoding.

## Validating

`filing.validate()` runs the FEC's acceptance rules from
[Validating a filing](./validating.md) and never raises. The result is
iterable, sized, and prints one finding per line exactly as `hardmoney
validate` does.

```python
v = hardmoney.parse_file("tests/fixtures/F3A_767339_v8.0.fec").validate()
print(repr(v), v.is_acceptable)
print("\n".join(str(v).splitlines()[:3]))

v = hardmoney.parse_file("tests/fixtures/invalid/bad_dates_and_amounts.fec").validate()
print(repr(v), v.is_acceptable)
print(v)
f = v.errors[0]
print((f.severity, f.rule, f.line_no, f.form_type, f.field))
print(f.message)
```

```text
<hardmoney.Validation 0 error(s), 15 warning(s)> True
WARN  line 1 HDR fec_version: Filing must be in the current FEC format (found 8.0, current is 8.5)
WARN  line 541 SA11C contributor_city: CONTRIBUTOR CITY is Missing
WARN  line 541 SA11C contributor_state: CONTRIBUTOR STATE is Missing
<hardmoney.Validation 5 error(s), 0 warning(s)> False
ERROR line 2 F3XA date_signed: 20261301 is not a Real Date
ERROR line 4 SB21B expenditure_date: 20260231 is not a Real Date
ERROR line 5 SB21B expenditure_date: Bad Date - 2026-06-22 not YYYYMMDD format
ERROR line 6 SB21B expenditure_amount: Invalid Amount format: '$5,500.00' in EXPENDITURE AMOUNT {F3L Bundled} (digits with an optional sign and up to two decimals; no $ or commas)
ERROR line 7 SB21B expenditure_amount: Invalid Amount format: '1500.005' in EXPENDITURE AMOUNT {F3L Bundled} (digits with an optional sign and up to two decimals; no $ or commas)

('error', 'not_a_real_date', 2, 'F3XA', 'date_signed')
20261301 is not a Real Date
```

`Validation` has `findings`, `errors`, `warnings` (lists of `Finding`)
and `is_acceptable` (no error-severity findings: the FEC would accept
the filing). A `Finding` has `severity` (`"error"`/`"warning"`), `rule`
(the stable snake_case name from the Rust `Rule` enum), `line_no`,
`form_type`, `field` (or `None`), and `message`. Lines a lenient parse
skipped show up as `unrecognized_form_type` warnings, as they do in the
CLI.

## Reconciling

`filing.reconcile()` is [Reconciling a filing](./reconciling.md): every
cover-page line recomputed from the schedules and the other cover lines,
exactly. It raises `hardmoney.UnsupportedForm` (a `FecError`) unless the
cover is F3X, F3, or F3P.

```python
filing = hardmoney.parse_file("tests/fixtures/F3XN_2011831.fec")
r = filing.reconcile()
print(repr(r), r.form, r.balances)
c = r.line("A", "11(a)(i)")
print(c)
print((c.column, c.line, c.field, c.relation, c.reported, c.expected, c.delta, c.matches, c.lines_summed))
print(str(r).splitlines()[-1])
```

```text
<hardmoney.Reconciliation F3X 69 check(s), 0 mismatch(es)> F3X True
ok   col A line 11(a)(i)   reported        13736.02 expected        13736.02 delta         0.00  = sum of SchA.contribution_amount on SA11AI/SA11A1
('A', '11(a)(i)', 'col_a_individuals_itemized', 'equal', Decimal('13736.02'), Decimal('13736.02'), Decimal('0.00'), True, 132)
F3X: every line agrees with its rule
```

Now overstate 11(a)(i) by $100 and look again. The schedule sum does
not match, and because formulas are evaluated over the reported values
of their inputs, 11(a)(iii) = 11(a)(i) + 11(a)(ii) breaks too. That is
what isolates the bad line:

```python
filing.summary.set("col_a_individuals_itemized", "13836.02")
r = filing.reconcile()
print(r.balances)
for c in r.mismatches():
    print(c)
print(str(r).splitlines()[-1])

try:
    hardmoney.parse_file("tests/fixtures/F99_2011828.fec").reconcile()
except hardmoney.UnsupportedForm as e:
    print(type(e).__name__ + ":", e)
```

```text
False
DIFF col A line 11(a)(i)   reported        13836.02 expected        13736.02 delta       100.00  = sum of SchA.contribution_amount on SA11AI/SA11A1
DIFF col A line 11(a)(iii) reported        15245.52 expected        15345.52 delta      -100.00  = 11(a)(i) + 11(a)(ii)
F3X: 2 of 69 line(s) disagree
UnsupportedForm: no reconciliation rules for form F99; supported: F3X, F3, F3P
```

A `LineCheck` has `line`, `field`, `column` (`"A"`/`"B"`), `rule`,
`reported` (`Decimal` or `None` when blank or unparseable),
`expected`, `delta`, `relation` (`"equal"`, or `"at_least"` for the
itemization-threshold floors), `matches`, `violation`, `lines_summed`,
and `reported_unparseable`. `r.line(column, label)` looks one up;
`r.mismatches()` lists the ones that fail; `r.balances` is the one-bit
answer.

## Errors

| Exception | When |
|---|---|
| `hardmoney.FecError` (a `ValueError`) | the filing cannot be parsed or interpreted; `.line_no` is the 1-based physical line, or `None` when the error is not about one line |
| `hardmoney.UnsupportedForm` (a `FecError`) | `reconcile()` on a form without a rule table |
| `KeyError` | `line[name]`, `line.set`, `line.amount`, `line.date` with a field not in the layout |
| `ValueError` | an unknown table name, a malformed spec version, a reconciliation column other than `"A"`/`"B"` |
| `TypeError` | `parse()` given something other than `bytes`/`str` |
| `OSError` (`FileNotFoundError`, ...) | `parse_file()` cannot open or read the path (deliberately not a `FecError`, because that is what Python code expects to catch) |

```python
try:
    hardmoney.parse(b"HDR\x1cFEC\x1c8.5\x1cX\x1c1\nF3XN\x1cC00123456\nZZZ\x1cC00123456")
except hardmoney.FecError as e:
    print(f"{type(e).__name__}: {e} (line_no={e.line_no})")
try:
    hardmoney.parse(b"not a filing")
except hardmoney.FecError as e:
    print(f"{type(e).__name__}: {e} (line_no={e.line_no})")

f = hardmoney.parse(b"HDR\x1cFEC\x1c8.5\x1cX\x1c1\nF3XN\x1cC00123456\nZZZ\x1cC00123456", lenient=True)
print(f.skipped)
print([str(w) for w in f.validate().warnings if w.rule == "unrecognized_form_type"])
```

```text
FecError: no format table for form type 'ZZZ' (spec version 8.5) at line 3 (line_no=3)
FecError: filing has no cover/summary line (line_no=None)
[{'line_no': 3, 'form_type': 'ZZZ', 'reason': 'unknown form type'}]
["WARN  line 3 ZZZ form_type: Unrecognized Form Type / Record Ignored ('ZZZ': unknown form type)"]
```

`lenient=True` is [Strict vs. lenient parsing](./strict-vs-lenient.md):
unparseable body lines are recorded in `filing.skipped` (each a dict
with `line_no`, `form_type`, `reason`) instead of failing the parse.

## The spec from Python

The format tables that `build.rs` compiles from `data/` are queryable:

```python
print(hardmoney.BUNDLED_SPEC_VERSION)
print(hardmoney.tables())
lay = hardmoney.layout("SchA", "8.5")
print(len(lay), lay[:3], [p for p in lay if p[0] == "contribution_amount"])
print([p for p in hardmoney.layout("SchA", "5.3") if p[0] == "contribution_amount"])
print(hardmoney.field_spec("SchA", "contribution_amount"))
print(hardmoney.field_spec("F3X", "filer_committee_id_number")["pattern"])
print(hardmoney.field_spec("SchA", "no_such_field"))
```

```text
8.5
['F1', 'F10', 'F105', 'F13', 'F132', 'F133', 'F1M', 'F1S', 'F2', 'F24', 'F2S', 'F3', 'F3L', 'F3P', 'F3P31', 'F3PS', 'F3PZ1', 'F3PZ2', 'F3S', 'F3X', 'F3Z', 'F3Z1', 'F3Z2', 'F4', 'F5', 'F56', 'F57', 'F6', 'F65', 'F7', 'F76', 'F8', 'F82', 'F83', 'F9', 'F91', 'F92', 'F93', 'F94', 'F99', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'HDR', 'SchA', 'SchA3L', 'SchB', 'SchC', 'SchC1', 'SchC2', 'SchD', 'SchE', 'SchF', 'SchI', 'SchL', 'TEXT']
45 [('form_type', 0), ('filer_committee_id_number', 1), ('transaction_id', 2)] [('contribution_amount', 20)]
[('contribution_amount', 15)]
{'column': 20, 'description': 'CONTRIBUTION AMOUNT {F3L Bundled}', 'kind': 'amount', 'max_len': 12, 'required': 'warning', 'condition': None, 'sample': '250', 'value_reference': None, 'rule': 'Contribution (F3L Bundled) Amount', 'forms': ['F3', 'F3X', 'F3P', 'F3L'], 'allowed_values': [], 'pattern': None}
^[C|P][0-9]{8}$|^[H|S][0-9]{1}[A-Z]{2}[0-9]{5}$
None
```

`layout(table, version)` is the column layout (`(field, 0-based
column)` pairs, in table order) the parser uses for that spec version.
`contribution_amount` moves from column 15 in 5.3 to 20 in 8.x.
`field_spec(table, name)` is the FEC's own specification of the field at
`BUNDLED_SPEC_VERSION` (type, maximum length, required level, rule text,
allowed values, pattern), or `None` if the current spec does not document
it. See [The schema](./library-schema.md) for what these mean.

## Design notes

Every class is frozen. `Filing`, `Line`, `Validation`, `Finding`,
`Reconciliation`, and `LineCheck` cannot be constructed or have
attributes assigned from Python. The one mutation, `Line.set`, goes
through a lock inside the shared `Filing`, which is why an edit through
any `Line` handle is visible to `to_fec()`.

The extension does not panic. It follows the crate's rule: every failure
is a Python exception, never an aborted interpreter.

`Decimal` values cross the boundary as their string form (`"13736.02"`),
so `line.amount(name) == Decimal(line[name])` holds for every valid
amount and the scale is always 2.

There is one wheel per platform. The extension uses the CPython stable
ABI (`abi3`, minimum 3.9); the same wheel loads on 3.9 through 3.14.

`iter_lines` materialises its selection in memory rather than streaming
from disk. There is no Arrow/`pandas` interop; build a `DataFrame` from
`[l.to_dict() for l in filing.lines_for("SchA")]`. The bulk-data ETL and
REST API are Rust-only.

## Tests and CI

`python/tests/` is pytest over the real fixtures in `tests/fixtures/`:
every filing parses (via `parse` and `parse_file`), round-trips through
`to_fec`, and validates with no errors; `F3XN_2011831.fec` reconciles and
a doctored copy does not; every `invalid/` fixture is rejected; the spec
helpers agree with the generated tables; and `test_api.py` parses the
type stubs with `ast` and checks that every class member they declare
exists on the compiled module and vice versa, so the stubs cannot drift.
`.github/workflows/python.yml` runs `cargo fmt`/`clippy`/`doc` on the
extension, `maturin develop` + pytest on Linux and macOS with Python 3.9
and 3.13, and builds `abi3` wheels (and an sdist) as artifacts. Nothing
is published to PyPI from CI.
