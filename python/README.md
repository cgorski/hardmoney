# hardmoney

Read, check, edit, and write FEC electronic filings (`.fec` files) from
Python. `hardmoney` parses any filing from spec version 3.x (2001) through
8.5, exposes every field by a canonical name that is the same across
versions, returns dollar amounts as exact `decimal.Decimal` values,
validates a filing against the FEC's own acceptance rules, reconciles a
cover page against its schedules to the cent, and writes the filing back
out as canonical `.fec` bytes. It is a thin [PyO3](https://pyo3.rs)
binding over the Rust crate [`hardmoney`](https://crates.io/crates/hardmoney),
so the parser, writer, validator, and reconciler are the same code the
`hardmoney` command-line tool uses, tested against real filings.

## Install

```bash
pip install hardmoney
```

Wheels are published for Linux (x86_64, aarch64) and macOS (x86_64 and
Apple silicon) for CPython 3.9 and newer; one `abi3` wheel per platform
works on every Python version. The package has no Python dependencies.
On other platforms `pip` builds the source distribution, which needs a
Rust toolchain.

## Quick start

```python
import hardmoney

# Any .fec file. This one is FEC filing 2011831; hardmoney.fetch(2011831) downloads it.
filing = hardmoney.parse_file("F3XN_2011831.fec")
print(filing.form_type, filing.version, len(filing.lines))
# F3XN 8.5 144

# The cover page and every schedule line are mappings keyed by canonical field name.
cover = filing.summary
print(cover["committee_name"])
# FirstEnergy Corp Political Action Committee
print(cover.amount("col_a_total_receipts"), cover.date("coverage_through_date"))
# 15245.52 2026-08-31

for line in filing.lines_for("SchA")[:2]:
    print(line.line_no, line["contributor_last_name"], line.amount("contribution_amount"))
# 3 Mroczynski 380.00
# 4 Fickey 120.00

# Amounts are decimal.Decimal, so a sum over 132 lines equals the cover page exactly.
itemized = sum(l.amount("contribution_amount") for l in filing.lines_for("SchA") if not l.is_memo)
print(itemized, itemized == cover.amount("col_a_individuals_itemized"))
# 13736.02 True

# The FEC's acceptance rules. Never raises; errors mean the FEC would reject the filing.
v = filing.validate()
print(v.is_acceptable, len(v.errors), len(v.warnings))
# True 0 0
for finding in v.errors:
    print(finding.line_no, finding.field, finding.message)

# Every cover-page line recomputed from the schedules (F3X, F3, and F3P covers).
r = filing.reconcile()
print(r.balances)
# True
for check in r.mismatches():
    print(check.line, check.reported, check.expected, check.delta)

# Edit a field and write the filing back as canonical .fec bytes.
cover.set("treasurer_last_name", "Smith")
with open("edited.fec", "wb") as f:
    f.write(filing.to_fec())
```

Parse errors are `hardmoney.FecError`, a `ValueError` whose `line_no`
names the offending line when there is one:

```python
try:
    hardmoney.parse(b"HDR\x1cFEC\x1c8.5\x1cX\x1c1\nF3XN\x1cC00123456\nZZZ\x1cC00123456")
except hardmoney.FecError as e:
    print(e.line_no, e)
# 3 no format table for form type 'ZZZ' (spec version 8.5) at line 3
```

`hardmoney.parse(data, lenient=True)` records such lines in
`filing.skipped` instead of raising.

## What the package contains

| Name | Purpose |
|---|---|
| `parse(data, *, lenient=False)`, `parse_file(path, *, lenient=False)`, `fetch(filing_id)` | a `Filing` from `bytes`/`str`, from a path (streamed), or downloaded from `docquery.fec.gov` |
| `Filing` | `form_type`, `base_form_type`, `version`, `is_amendment`, `amends_filing`, `header`, `summary`, `lines`, `skipped`; `lines_for(table)`, `iter_lines(tables)`, `to_fec()`, `to_fec_string()`, `validate()`, `reconcile()` |
| `Line` | `table`, `form_type`, `line_no`, `is_memo`; `line[name]`, `name in line`, `get`, `keys`, `items`, `to_dict`, `set`, `amount(name) -> Decimal`, `date(name) -> date` |
| `Validation`, `Finding` | `findings`, `errors`, `warnings`, `is_acceptable`; `severity`, `rule`, `line_no`, `form_type`, `field`, `message` |
| `Reconciliation`, `LineCheck` | `form`, `checks`, `balances`, `mismatches()`, `line(column, label)`; `reported`, `expected`, `delta`, `relation`, `matches`, `violation`, `lines_summed` |
| `tables()`, `layout(table, version)`, `field_spec(table, name)`, `BUNDLED_SPEC_VERSION` | the format tables, per-version column positions, and the FEC's field specifications |
| `FecError`, `UnsupportedForm` | a `ValueError` with `line_no`; its subclass raised by `reconcile()` on other forms |

Every class, method, and function has a docstring, so `help(hardmoney.Filing)`
works at the prompt, and the package ships type stubs (`py.typed`).

## Where things are documented

- [Getting started with Python](https://cgorski.github.io/hardmoney/python.html):
  install, a first script, the mental model, how to find field names,
  error handling.
- [Python cookbook](https://cgorski.github.io/hardmoney/python-cookbook.html):
  thirty-five runnable scripts with their output (top donors, CSV
  validation reports, pandas, Parquet, SQLite, a CI exit code, a
  pre-submission check).
- [Python API reference](https://cgorski.github.io/hardmoney/python-api.html):
  every signature and docstring.
- [For FEC staff](https://cgorski.github.io/hardmoney/for-fec-staff.html):
  Python-first workflows for reviewing incoming filings.
- [Field names](https://cgorski.github.io/hardmoney/field-names.html):
  every field of every table with the FEC's label.
- [Changelog](https://github.com/cgorski/hardmoney/blob/main/CHANGELOG.md).
- [Issues and questions](https://github.com/cgorski/hardmoney/issues).
- [Source](https://github.com/cgorski/hardmoney): the Python package is
  in `python/`, the example scripts in `python/examples/`.

## Why exact decimals

FEC amounts are dollars and cents, written in the file as decimal text.
Binary floating point cannot represent most decimal fractions exactly
(`0.1 + 0.2 != 0.3` in Python), so a `float` sum has to be rounded before
it can be compared to a cover-page total, and `==` between two `float`
totals is not a meaningful test. `line.amount(name)` builds a
`decimal.Decimal` from the exact string in the file, `Decimal` arithmetic
is exact at that scale, and `Filing.reconcile()` compares schedule sums to
the cover page with `==`, no rounding step, the way an auditor would.
Nothing in the package converts an amount to a `float`; if a plotting or
statistics library needs one, convert at the last step.

## License

Apache-2.0 OR BSD-3-Clause, like the Rust crate. The column layouts derive
from the fech-sources project (MIT); see `NOTICE` in the repository.
