# hardmoney (Python)

FEC electronic filings (`.fec`) in Python: parse any filing from spec 3.x
(2001) through 8.5, read fields by their canonical names, edit and write
the filing back, validate it against the FEC's acceptance rules, and
reconcile a cover page against its schedules. Every dollar amount is an
exact `decimal.Decimal`, never a `float`.

This package is a thin [PyO3](https://pyo3.rs) wrapper over the Rust crate
[`hardmoney`](https://crates.io/crates/hardmoney); the parser, writer,
validator, and reconciler are the same code the `hardmoney` CLI and
library use, tested against real filings. One wheel per platform covers
every CPython from 3.9 on (stable ABI).

## Install

```bash
pip install hardmoney            # once published to PyPI
```

From a checkout of the repository (needs Rust and
[maturin](https://www.maturin.rs)):

```bash
cd python
pip install maturin
maturin develop --release        # builds and installs into the active venv
```

## Quick start

```python
import hardmoney

filing = hardmoney.parse_file("tests/fixtures/F3XN_2011831.fec")
print(filing.form_type, filing.version, len(filing.lines))
# F3XN 8.5 144

cover = filing.summary
print(cover["committee_name"], repr(cover.amount("col_a_total_receipts")))
# FirstEnergy Corp Political Action Committee Decimal('15245.52')

for line in filing.lines_for("SchA")[:2]:
    print(line.line_no, line["contributor_last_name"], line.amount("contribution_amount"),
          line.date("contribution_date"))
# 3 Mroczynski 380.00 2026-08-31
# 4 Fickey 120.00 2026-08-31

v = filing.validate()
print(v.is_acceptable, len(v.errors), len(v.warnings))
# True 0 0
print(v)                       # one finding per line, as `hardmoney validate` prints

r = filing.reconcile()         # F3X, F3, F3P; raises hardmoney.UnsupportedForm otherwise
print(r.balances)
# True
for c in r.mismatches():
    print(c.column, c.line, c.reported, c.expected, c.delta, c.rule)

# Edit and write back: the same canonical bytes the CLI's `hardmoney write` produces.
filing.lines[0].set("contributor_employer", "Self-employed")
open("edited.fec", "wb").write(filing.to_fec())
```

Errors are `hardmoney.FecError` (a `ValueError`) with a `line_no`
attribute naming the offending line when there is one:

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

| | |
|---|---|
| `parse(data, *, lenient=False)`, `parse_file(path, *, lenient=False)`, `fetch(filing_id)` | `bytes`/`str`, a path (streamed), or a download from docquery.fec.gov |
| `Filing` | `form_type`, `base_form_type`, `version`, `is_amendment`, `amends_filing`, `header`, `summary`, `lines`, `skipped`; `lines_for(table)`, `iter_lines(tables)`, `to_fec()`, `to_fec_string()`, `validate()`, `reconcile()` |
| `Line` | `table`, `form_type`, `line_no`, `is_memo`; `line[name]`, `name in line`, `get`, `keys`, `items`, `to_dict`, `set`, `amount(name) -> Decimal`, `date(name) -> date` |
| `Validation` / `Finding` | the FEC's acceptance rules: `findings`, `errors`, `warnings`, `is_acceptable`; `severity`, `rule`, `line_no`, `form_type`, `field`, `message` |
| `Reconciliation` / `LineCheck` | cover page vs. schedules: `form`, `checks`, `balances`, `mismatches()`, `line(column, label)`; `reported`, `expected`, `delta`, `relation`, `matches`, `violation`, `lines_summed` |
| `tables()`, `layout(table, version)`, `field_spec(table, name)`, `BUNDLED_SPEC_VERSION` | the format tables, per-version column positions, and the FEC's field specifications, all compiled in from data |

Full documentation, with captured output, is in the
[Python chapter of the hardmoney book](https://cgorski.github.io/hardmoney/python.html).

## Why a Python package

Python is where FEC data users are, and a wheel is the only realistic way
hardmoney's validator and reconciler reach the FEC's own Django codebase
(FECfile+) and the analysts, journalists, and researchers who work in
notebooks. The wheel is parser-only (no database, no HTTP server), so it
stays small and has no runtime dependencies.

## License

Apache-2.0 OR BSD-3-Clause, like the Rust crate. The column layouts derive
from the fech-sources project (MIT); see `NOTICE` in the repository.
