# Getting started with Python

The `hardmoney` package on PyPI is the parser, writer, validator, and
reconciler from the rest of this book, callable from Python. This page is
the place to start: install it, see the whole API in one short script,
learn the four ideas the API is built on, and find out where the rest of
the Python documentation lives.

The Python documentation is three pages plus one for a specific audience:

| Page | What it is for |
|---|---|
| this page | install, a first script, the mental model, how to look things up |
| [Python cookbook](./python-cookbook.md) | one runnable script per task: top donors, CSV validation reports, pandas, Parquet, SQLite, a CI exit code, a pre-submission check |
| [Python API reference](./python-api.md) | every class, method, property, and exception with its signature and docstring (generated from the type stub) |
| [For FEC staff](./for-fec-staff.md) | Python-first workflows for the people who receive filings rather than file them |

Every output block on this page was captured from a run against the
fixtures in `tests/fixtures/` in the repository, with the working
directory at the repository root.

## Install

```bash
pip install hardmoney
```

The package ships `abi3` wheels for Linux (x86_64 and aarch64,
manylinux2014) and macOS (x86_64 and Apple silicon), for CPython 3.9 and
newer; one wheel per platform covers every Python version. It has no
Python dependencies. On any other platform `pip` builds the source
distribution, which needs a Rust toolchain.

To work on the package from a checkout of the repository, with Rust and
[maturin](https://www.maturin.rs) installed:

```bash
cd python
python -m venv .venv && . .venv/bin/activate
pip install maturin pytest
maturin develop --release        # builds the extension into the venv
pytest -q                        # runs the suite over tests/fixtures/*.fec
```

## A tour in sixty seconds

Parse a Form 3X (a PAC's periodic report), read the cover page and the
first Schedule A contributions, check that the itemized contributions add
up to the cover-page line that reports them, validate, reconcile, change a
field, and write the filing back out:

```python
import hardmoney

filing = hardmoney.parse_file("tests/fixtures/F3XN_2011831.fec")
print(filing)
print(filing.form_type, filing.base_form_type, filing.version, filing.is_amendment)

cover = filing.summary
print(cover["committee_name"])
print(cover.amount("col_a_total_receipts"), cover.date("coverage_from_date"), cover.date("coverage_through_date"))

for line in filing.lines_for("SchA")[:3]:
    print(line.line_no, line["contributor_last_name"], line.amount("contribution_amount"), line.date("contribution_date"))

total = sum(l.amount("contribution_amount") for l in filing.lines_for("SchA") if not l.is_memo)
print(total, total == cover.amount("col_a_individuals_itemized"))

v = filing.validate()
print(repr(v), v.is_acceptable)

r = filing.reconcile()
print(repr(r), r.balances)

cover.set("committee_name", "FirstEnergy Corp PAC")
data = filing.to_fec()
print(type(data).__name__, len(data), hardmoney.parse(data).summary["committee_name"])
```

```text
<hardmoney.Filing F3XN v8.5 (144 body lines)>
F3XN F3X 8.5 False
FirstEnergy Corp Political Action Committee
15245.52 2026-08-01 2026-08-31
3 Mroczynski 380.00 2026-08-31
4 Fickey 120.00 2026-08-31
5 Rossero 200.00 2026-08-31
13736.02 True
<hardmoney.Validation 0 error(s), 0 warning(s)> True
<hardmoney.Reconciliation F3X 69 check(s), 0 mismatch(es)> True
bytes 32198 FirstEnergy Corp PAC
```

That is most of the API. The rest of this page explains what each of
those objects is and how to find your way around a filing you have not
seen before.

## The mental model

Four ideas cover almost everything.

### A filing is a header, a cover line, and body lines

`hardmoney.parse(data)`, `hardmoney.parse_file(path)`, and
`hardmoney.fetch(filing_id)` all return a `Filing`. It has a `header`
(the `HDR` record, as a `dict`), a `summary` (the cover line, row 2 of the
file), and `lines` (every body line in file order). `lines_for("SchA")`
selects one table; `iter_lines(["SchA", "SchB"])` iterates a selection.

```python
filing = hardmoney.parse_file("tests/fixtures/F3XN_2011831.fec")
print(filing.header)
print(filing.summary, filing.summary.table)
print(len(filing.lines), {t: len(filing.lines_for(t)) for t in ("SchA", "SchB")})
```

```text
{'record_type': 'HDR', 'ef_type': 'FEC', 'fec_version_raw': '8.5', 'version': '8.5', 'soft_name': 'Vocus PAC Management', 'soft_ver': '8.02.1066', 'report_id': '', 'report_number': '0', 'comment': ''}
<hardmoney.Line F3XN (F3X) line 2> F3X
144 {'SchA': 132, 'SchB': 12}
```

Table names (`"F3X"`, `"SchA"`, `"SchB"`, `"TEXT"`, ...) are the FEC's
own; `hardmoney.tables()` lists all 59. A filing's `version` is the spec
version its lines were parsed with, and the same field names work across
every version from 3.x to 8.5 even though the column positions moved.

### A line is a mapping from canonical field name to the value as filed

Every body line and the cover line is a `Line`. It behaves like a
read-mostly mapping whose keys are the canonical `lower_snake_case` field
names documented in [Field names](./field-names.md), and whose values are
the strings as filed (trimmed, otherwise verbatim; see
[Fidelity](./fidelity.md)).

```python
line = filing.lines[0]
print(line)
print(line.table, line.form_type, line.line_no, line.is_memo)
print(line.keys()[:6])
print(repr(line["contribution_amount"]), repr(line.amount("contribution_amount")), repr(line.date("contribution_date")))
print(repr(line["contributor_middle_name"]), line.get("no_such_field", "n/a"))
try:
    line["no_such_field"]
except KeyError as e:
    print("KeyError:", e)
```

```text
<hardmoney.Line SA11AI (SchA) line 3>
SchA SA11AI 3 False
['form_type', 'filer_committee_id_number', 'transaction_id', 'back_reference_tran_id', 'back_reference_sched_name', 'entity_type']
'380.00' Decimal('380.00') datetime.date(2026, 8, 31)
'' n/a
KeyError: 'no_such_field'
```

The rules, precisely:

- `line[name]` is the value as filed, `""` when blank, and `KeyError`
  when the field is not in this filing's layout for the table (a typo, or
  a field that did not exist in that spec version).
- `name in line`, `line.get(name, default=None)`, `line.keys()`,
  `line.items()`, and `line.to_dict()` work as on a `dict`, in layout
  order, blanks included. `get` returns the default only for a missing
  field; a blank one is `""`.
- `line.table` is the format table, `line.form_type` the form-type token
  upper-cased (`"SA11AI"`), `line.line_no` the 1-based physical line in
  the file, and `line.is_memo` whether `memo_code` is `X`. Memo entries
  are excluded from every cover-page total, which is why the tour
  filtered them out before summing.

### Money is `Decimal`, dates are `date`

`line.amount(name)` returns a `decimal.Decimal` with two decimal places,
built from the exact string in the file, or `None` when the field is
blank or is not a valid FEC amount (`$5,500.00` is `None`; the validator
reports why). `line.date(name)` returns a `datetime.date`, or `None` when
the field is blank, zero-filled, or not a real date. Nothing in the
package is a `float`.

```python
amounts = [l.amount("contribution_amount") for l in filing.lines_for("SchA") if not l.is_memo]
print(len(amounts), sum(amounts), sum(amounts) == filing.summary.amount("col_a_individuals_itemized"))
print(repr(filing.summary.amount("col_a_total_receipts")), repr(filing.summary["col_a_total_receipts"]))
```

```text
132 13736.02 True
Decimal('15245.52') '15245.52'
```

That `True` is the point: 132 contributions summed with `Decimal` equal
the cover page's line 11(a)(i) to the cent, and `Filing.reconcile()` does
this comparison for every cover-page line. `line.amount(name) ==
Decimal(line[name])` holds for every valid amount.

### Strict by default, lenient on request

A strict parse raises `hardmoney.FecError` at the first body line it
cannot interpret. `lenient=True` records such lines in `filing.skipped`
and keeps going; the validator then reports them as
`unrecognized_form_type` warnings, as the CLI does. The trade-off is
discussed in [Strict vs. lenient parsing](./strict-vs-lenient.md).

```python
raw = b"HDR\x1cFEC\x1c8.5\x1cX\x1c1\nF3XN\x1cC00123456\nZZZ\x1cC00123456"
try:
    hardmoney.parse(raw)
except hardmoney.FecError as e:
    print(f"{type(e).__name__}: {e} (line_no={e.line_no})")
f = hardmoney.parse(raw, lenient=True)
print(f.skipped)
print([str(w) for w in f.validate().warnings if w.rule == "unrecognized_form_type"])
```

```text
FecError: no format table for form type 'ZZZ' (spec version 8.5) at line 3 (line_no=3)
[{'line_no': 3, 'form_type': 'ZZZ', 'reason': 'unknown form type'}]
["WARN  line 3 ZZZ form_type: Unrecognized Form Type / Record Ignored ('ZZZ': unknown form type)"]
```

## Editing and writing back

`line.set(name, value)` changes a field (trimmed like the parser;
`KeyError` for a field not in the layout). A `Line` is a handle into its
`Filing`, not a copy, so `filing.to_fec()` sees every edit made through
any handle. `to_fec()` returns the canonical `.fec` bytes described in
[Writing `.fec` files](./writing-fec.md), Windows-1252 when every
character is representable and UTF-8 otherwise; `to_fec_string()` is the
text before encoding.

```python
filing = hardmoney.parse_file("tests/fixtures/F3XA_2011827.fec")
changed = 0
for line in filing.lines_for("SchA"):
    if line["contributor_employer"] == "SELF":
        line.set("contributor_employer", "Self-employed")
        changed += 1
out = filing.to_fec()
again = hardmoney.parse(out)
print(changed, len(out), again.lines_for("SchA")[0]["contributor_employer"])
print([l.to_dict() for l in again.lines] == [l.to_dict() for l in filing.lines], again.to_fec() == out)
```

```text
1 1885 Self-employed
True True
```

The second line is the round-trip guarantee: parsing what the writer
produced gives the same lines field for field, and writing that again
gives the same bytes. `python/tests/test_write.py` checks this for every
fixture in `tests/fixtures/`.

## Validating and reconciling

`filing.validate()` applies the FEC's acceptance rules
([Validating a filing](./validating.md)) and never raises. The result
prints one finding per line as `hardmoney validate` does, and exposes
`errors`, `warnings`, `findings`, and `is_acceptable`. Each `Finding` has
a `severity`, a stable `rule` name, `line_no`, `form_type`, `field`, and
`message`.

```python
v = hardmoney.parse_file("tests/fixtures/invalid/bad_dates_and_amounts.fec").validate()
print(repr(v), v.is_acceptable)
print(v)
f0 = v.errors[0]
print(repr(f0))
print((f0.severity, f0.rule, f0.line_no, f0.form_type, f0.field))
print(f0.message)
```

```text
<hardmoney.Validation 5 error(s), 0 warning(s)> False
ERROR line 2 F3XA date_signed: 20261301 is not a Real Date
ERROR line 4 SB21B expenditure_date: 20260231 is not a Real Date
ERROR line 5 SB21B expenditure_date: Bad Date - 2026-06-22 not YYYYMMDD format
ERROR line 6 SB21B expenditure_amount: Invalid Amount format: '$5,500.00' in EXPENDITURE AMOUNT {F3L Bundled} (digits with an optional sign and up to two decimals; no $ or commas)
ERROR line 7 SB21B expenditure_amount: Invalid Amount format: '1500.005' in EXPENDITURE AMOUNT {F3L Bundled} (digits with an optional sign and up to two decimals; no $ or commas)

<hardmoney.Finding error not_a_real_date line 2 date_signed>
('error', 'not_a_real_date', 2, 'F3XA', 'date_signed')
20261301 is not a Real Date
```

`filing.reconcile()` recomputes every cover-page line from the schedules
and the other cover lines ([Reconciling a filing](./reconciling.md)). It
works for F3X, F3, and F3P covers and raises `hardmoney.UnsupportedForm`
for anything else. Overstate line 11(a)(i) by $100 and both the schedule
sum and the formula that depends on it disagree, which is what isolates
the bad line:

```python
filing = hardmoney.parse_file("tests/fixtures/F3XN_2011831.fec")
filing.summary.set("col_a_individuals_itemized", "13836.02")
r = filing.reconcile()
print(repr(r), r.balances)
for c in r.mismatches():
    print(c)
c = r.line("A", "11(a)(i)")
print((c.column, c.line, c.field, c.reported, c.expected, c.delta, c.relation, c.matches))
try:
    hardmoney.parse_file("tests/fixtures/F99_2011828.fec").reconcile()
except hardmoney.UnsupportedForm as e:
    print(type(e).__name__ + ":", e)
```

```text
<hardmoney.Reconciliation F3X 69 check(s), 2 mismatch(es)> False
DIFF col A line 11(a)(i)   reported        13836.02 expected        13736.02 delta       100.00  = sum of SchA.contribution_amount on SA11AI/SA11A1
DIFF col A line 11(a)(iii) reported        15245.52 expected        15345.52 delta      -100.00  = 11(a)(i) + 11(a)(ii)
('A', '11(a)(i)', 'col_a_individuals_itemized', Decimal('13836.02'), Decimal('13736.02'), Decimal('100.00'), 'equal', False)
UnsupportedForm: no reconciliation rules for form F99; supported: F3X, F3, F3P
```

A `LineCheck` carries `reported`, `expected`, `delta`, and `relation`
(`"equal"`, or `"at_least"` for the itemization-threshold floors), among
others; the [API reference](./python-api.md#linecheck) lists them all.

## Finding field names

Field names are the part of any FEC library that takes getting used to.
Four ways to find the one you need, from quickest to most thorough:

1. Ask a line: `line.keys()` is every field in this filing's layout for
   the table, in column order, and `name in line` tests one.
2. Ask the package: `hardmoney.layout(table, version)` is the column
   layout the parser uses for a table at a spec version, and
   `hardmoney.field_spec(table, name)` is the FEC's own description of a
   field (type, maximum length, whether it is required, the validation
   rule text, allowed values, pattern) at `hardmoney.BUNDLED_SPEC_VERSION`.
3. Read the [Field names](./field-names.md) chapter, which lists every
   field of every table with the FEC's label and explains the naming
   rules (`transaction_id` everywhere, `*_zip_code`, `col_a_` and
   `col_b_` prefixes on cover pages, and so on).
4. Run `hardmoney spec fields SchA` from the command line
   ([CLI reference](./cli-reference.md#spec)) for the same information as
   an aligned table.

```python
print(hardmoney.BUNDLED_SPEC_VERSION)
print(len(hardmoney.tables()), hardmoney.tables()[:8])
lay = hardmoney.layout("SchA", "8.5")
print(len(lay), lay[:4])
print([p for p in lay if "amount" in p[0]])
print([p for p in hardmoney.layout("SchA", "5.3") if p[0] == "contribution_amount"])
spec = hardmoney.field_spec("SchA", "contribution_amount")
print(spec["description"], spec["kind"], spec["max_len"], spec["required"], spec["forms"])
print(hardmoney.field_spec("F3X", "filer_committee_id_number")["pattern"])
print(hardmoney.field_spec("SchA", "no_such_field"))
filing = hardmoney.parse_file("tests/fixtures/F3XN_2011831.fec")
print([k for k in filing.summary.keys() if "total_receipts" in k])
print("contribution_amount" in filing.lines[0], "expenditure_amount" in filing.lines[0])
```

```text
8.5
59 ['F1', 'F10', 'F105', 'F13', 'F132', 'F133', 'F1M', 'F1S']
45 [('form_type', 0), ('filer_committee_id_number', 1), ('transaction_id', 2), ('back_reference_tran_id', 3)]
[('contribution_amount', 20)]
[('contribution_amount', 15)]
CONTRIBUTION AMOUNT {F3L Bundled} amount 12 warning ['F3', 'F3X', 'F3P', 'F3L']
^[C|P][0-9]{8}$|^[H|S][0-9]{1}[A-Z]{2}[0-9]{5}$
None
['col_a_total_receipts', 'col_a_total_receipts_recap', 'col_b_total_receipts', 'col_b_total_receipts_recap']
True False
```

`contribution_amount` is column 15 in spec 5.3 and column 20 in 8.5; the
name is the same in both, which is what lets one script handle a 2003
filing and a 2026 one. [The schema](./library-schema.md) explains where
these tables come from.

## Error handling

| Exception | When |
|---|---|
| `hardmoney.FecError` (a `ValueError`) | the filing cannot be parsed or interpreted; `.line_no` is the 1-based physical line, or `None` when the error is not about one line |
| `hardmoney.UnsupportedForm` (a `FecError`) | `reconcile()` on a form without a rule table |
| `KeyError` | `line[name]`, `line.set`, `line.amount`, `line.date` with a field not in the layout |
| `ValueError` | an unknown table name, a malformed spec version, a reconciliation column other than `"A"`/`"B"` |
| `TypeError` | `parse()` given something other than `bytes`/`str` |
| `OSError` (`FileNotFoundError`, ...) | `parse_file()` cannot open or read the path; deliberately not a `FecError`, because that is what Python code expects to catch |

```python
try:
    hardmoney.parse(b"not a filing")
except hardmoney.FecError as e:
    print(f"{type(e).__name__}: {e} (line_no={e.line_no})")
try:
    hardmoney.parse_file("no/such/file.fec")
except OSError as e:
    print(f"{type(e).__name__}: {e}")
try:
    filing.lines_for("SchZ")
except ValueError as e:
    print(f"{type(e).__name__}: {e}")
try:
    hardmoney.parse(42)
except TypeError as e:
    print(f"{type(e).__name__}: {e}")
```

```text
FecError: filing has no cover/summary line (line_no=None)
FileNotFoundError: no/such/file.fec: No such file or directory (os error 2)
ValueError: unknown table 'SchZ'; hardmoney.tables() lists the 59 known tables
TypeError: parse() expects bytes or str, not int
```

The extension never panics: every failure is one of these exceptions.

## Help at the prompt

Every class, method, property, and function carries the same docstring
the [API reference](./python-api.md) shows, so `help()` and editor
tooltips work without leaving the interpreter, and the package ships type
stubs (`py.typed`) for `mypy`, `pyright`, and completion:

```python
help(hardmoney.Line.amount)
```

```text
Help on method descriptor amount:

amount(self, /, name) unbound hardmoney.Line method
    The field parsed as an exact dollar amount (`decimal.Decimal`, scale
    2), or `None` when blank or not a valid FEC amount. `KeyError` if
    the field is not in the layout.
```

## Design notes

Every class is frozen: `Filing`, `Line`, `Validation`, `Finding`,
`Reconciliation`, and `LineCheck` cannot be constructed or have attributes
assigned from Python. The one mutation, `Line.set`, goes through a lock
inside the shared `Filing`.

Parsing releases the GIL, so other threads keep running while a large
filing is read. `parse_file` streams from disk, so peak memory is the
parsed lines alone.

The wheel is parser-only: no database, no HTTP server, no runtime
dependencies. The bulk-data ETL and REST API in the rest of this book are
Rust and CLI only. There is no built-in Arrow or pandas interop; the
cookbook shows `DataFrame` construction from `[l.to_dict() for l in
filing.lines_for("SchA")]`.

## Where to go next

- [Python cookbook](./python-cookbook.md): thirty-five runnable scripts
  with their output, from top donors to a Django-style pre-submission
  check.
- [Python API reference](./python-api.md): every signature and docstring.
- [For FEC staff](./for-fec-staff.md): workflows for reviewing incoming
  filings in Python.
- [Field names](./field-names.md): every field of every table.
- [Validating a filing](./validating.md) and
  [Reconciling a filing](./reconciling.md): what the rules are and where
  they come from.
- The `python/examples/` and `python/tests/` directories in the
  repository, which are run in CI against the fixtures.
