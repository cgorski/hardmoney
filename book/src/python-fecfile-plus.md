# Using hardmoney from FECfile+ and other Python projects

FECfile+ (`fecgov/fecfile-web-api`) validates one record at a time with
`fecfile_validate.validate(schema_name, data)`, which returns a
`ValidationResult` whose `errors` and `warnings` are lists of
`ValidationError(message, path)`, and it has no check of the whole `.fec`
file it composes. Its repositories contain no `.fec` files at all. This
chapter is for a project in that position: a Django app, a vendor's
filing tool, or any Python code that writes `.fec` files and wants to
assert, in its own test suite, that the FEC would accept them. Three
things are provided:

- `hardmoney.compat.fecfile_validate`, the `fecfile-validate` result
  shape over a whole file, over a single record, and FECfile+'s
  `line_*`-keyed Column A;
- a pytest plugin, installed with the package, with `assert_fec_acceptable`,
  `assert_fec_balances`, a `fec_filing` factory, and a `fec_fixture`
  parametrised over a directory of `.fec` files;
- a golden fixture pack, `tests/fixtures/golden/`, with spec-8.5 files
  for six forms, three deliberately broken variants, and sidecars stating
  what each must produce.

Everything below was captured from a run against the golden pack; the
only edits are home-directory paths shortened to `~`. Amounts are
`decimal.Decimal` throughout.

## The compat layer

```python
from hardmoney.compat import fecfile_validate as fv
```

### A whole file: `validate_file`

`validate_file(data)` takes `.fec` content as `bytes` or `str` (what
FECfile+'s `compose_dot_fec` returns), or a path as any `os.PathLike`
(a `pathlib.Path`) or as a `str` naming an existing file. A multi-line
`str` is always content; a single-line `str` is read as a path when a
file of that name exists.

```python
result = fv.validate_file(golden / "f3x_duplicate_transaction_id.fec")
print(result)
for e in result.errors:
    print(e)
    print(f"  path={e.path!r} validator={e.validator!r} instance={e.instance!r} "
          f"line_no={e.line_no} form_type={e.form_type!r}")
```

```text
<ValidationResult 1 error(s), 0 warning(s)>
<ValidationError error line 4 'transaction_id': 'Tran ID SA11AI.1 is NOT UNIQUE - This one is same as other(s) (first used on line 3)'>
  path='transaction_id' validator='duplicate_transaction_id' instance='SA11AI.1' line_no=4 form_type='SA11B'
```

`ValidationError` has fecfile-validate's two attributes with the same
meaning, `message` and `path` (the field name inside the record, `""`
when the finding is not about a field), and both are positional in the
constructor, so code written against `fecfile_validate.ValidationError`
reads it unchanged. Five attributes are added because a whole-file check
has to say which record it means: `validator` (hardmoney's rule name,
like jsonschema's attribute of the same name), `instance` (the value as
filed), `severity`, `line_no`, and `form_type`. `ValidationResult` has
`errors`, `warnings`, and an `is_acceptable` property; unlike
fecfile-validate's class it does not share one mutable default list
between instances.

A file that does not parse at all is not an exception but one error:

```python
>>> fv.validate_file(b"not a filing").errors[0]
<ValidationError error '': 'filing has no cover/summary line'>
```

with `validator="parse_error"`. Body lines the FEC would ignore are
parsed leniently and come back as `unrecognized_form_type` warnings, as
`hardmoney validate` reports them.

### One record: `validate_record`

```python
record = {
    "filer_committee_id_number": "C00002253", "transaction_id": "SA11AI.1", "entity_type": "IND",
    "contributor_first_name": "Jane", "contributor_street_1": "1 Main St", "contributor_city": "Springfield",
    "contributor_state": "VA", "contributor_zip": "22150", "contribution_date": "20260315",
    "contribution_amount": Decimal("250.00"), "contribution_aggregate": Decimal("250.00"),
    "transaction_type_identifier": "INDIVIDUAL_RECEIPT", "aggregation_group": "GENERAL",
}
r = fv.validate_record("SA11AI", record)
print(r)
for e in r.errors:
    print(" ", e.path, "|", e.message, "|", e.validator)
```

```text
<ValidationResult 1 error(s), 0 warning(s)>
  contributor_last_name | CONTRIBUTOR LAST NAME is Required, but field is Empty | required_field_empty
```

The dictionary above is shaped like a FECfile+ transaction: it uses
FECfile+'s `contributor_zip` for the spec's `contributor_zip_code`, and
it carries keys that are not `.fec` fields (`transaction_type_identifier`,
`aggregation_group`). Both are handled: the three spellings in which
FECfile+'s schemas differ from the spec's canonical names (`*_zip`,
`back_reference_tran_id_number`, `memo_text_description`) are mapped, and
unknown keys are ignored. Values may be `str`, `Decimal`, `int`, `date`,
`bool` (`True` writes `X`, for `memo_code`), or `None`; a `float` raises
`TypeError`, because a float cannot hold every dollar amount.

How it works, since hardmoney validates filings and not records: the
record is serialised to one `.fec` line at the columns
`hardmoney.layout(table, "8.5")` gives, wrapped in a minimal header and a
minimal cover page of a form the record's table belongs with, and the
three-line filing is validated; only findings on the record's line are
kept. Rules a single record cannot satisfy are dropped
(`back_reference_not_found`; `duplicate_transaction_id` cannot fire). A
record that is itself a cover page (`"F3XN"`, `"F24N"`) is recognised and
validated as the cover. The token is dispatched by the parser itself, so
`validate_record` knows every record type hardmoney knows and raises
`ValueError` for one it does not.

### FECfile+'s Column A: `summary_column_a`

```python
a = fv.summary_column_a(golden / "f3x.fec")
for k in ["line_11ai", "line_11aii", "line_15", "line_18a", "line_21ai", "line_21aii", "line_6c", "line_8"]:
    print(f"{k:12} {a[k]!r}")
print(len(a), "keys")
```

```text
line_11ai    Decimal('10000.23')
line_11aii   Decimal('3.77')
line_15      Decimal('2125.79')
line_18a     Decimal('1000.00')
line_21ai    Decimal('330.00')
line_21aii   Decimal('670.00')
line_6c      Decimal('19085.17')
line_8       Decimal('16692.92')
51 keys
```

The keys are those of `reports/form_3x/summary.py`'s
`calculate_summary_column_a` (`fv.line_key("11(a)(i)") == "line_11ai"`),
and the values are what that function computes from its transaction
table: schedule-sourced lines are sums of the matching non-memo schedule
lines, lines with no schedule (unitemized individuals, cash on hand at
the start of the period) are the cover's values, and formula lines are
evaluated over those rather than over the cover as filed. Two
consequences worth knowing. A cover that legitimately reports more on an
itemization-threshold line than its itemized sum (sub-$200 operating
expenditures, say) is *not* what comes back; the dictionary is the
schedules' view. And the five lines FECfile+ sets to `Decimal(0)`
(`line_18c`, `line_21ai`, `line_21aii`, `line_30ai`, `line_30aii`) carry
the values Schedules H3 to H6 imply, which is the whole point of the
golden Form 3X below. Forms 3 and 3P use their own labels
(`line_17ai`, `line_23`, ...); any other form raises
`hardmoney.UnsupportedForm`.

## The pytest plugin

The plugin is registered through the `pytest11` entry point when the
`hardmoney` wheel is installed, so it is active in every project that
depends on hardmoney (`pytest --markers` lists `@pytest.mark.hardmoney`).
It adds four fixtures and one option; `path_or_bytes` follows the same
convention as `validate_file`.

| Fixture | What it does |
|---|---|
| `assert_fec_acceptable(path_or_bytes, *, allow_warnings=True)` | fails, listing every finding, when the FEC would reject the filing; `allow_warnings=False` fails on any finding. Returns the parsed `Filing`. |
| `assert_fec_balances(path_or_bytes, tolerance=Decimal("0"))` | fails, listing every cover-page line whose violation exceeds `tolerance`; forms without a rule table (F1, F24, F99, ...) return `None`, there being nothing to reconcile. A `float` tolerance raises `TypeError`. |
| `fec_filing(path_or_bytes)` | a factory returning `hardmoney.Filing`. |
| `fec_fixture` | one `pathlib.Path` per `.fec` under `--hardmoney-fixtures=DIR` (recursively); without the option, tests using it are skipped. |

Tests that use any of them are marked `hardmoney`, so `-m hardmoney` and
`-m "not hardmoney"` select or skip them. A Django test that composes a
report is two lines:

```python
def test_composed_report_is_acceptable(assert_fec_acceptable, assert_fec_balances):
    dot_fec = compose_dot_fec(report.id)      # str or bytes
    assert_fec_acceptable(dot_fec)
    assert_fec_balances(dot_fec)
```

and a sweep over a directory of files, here the golden pack, is one
parametrised test per file:

```python
# test_golden.py
from decimal import Decimal
import pytest

def test_acceptable(fec_fixture, assert_fec_acceptable):
    if fec_fixture.name in {"f3x_duplicate_transaction_id.fec", "f3x_missing_required_field.fec"}:
        pytest.xfail("deliberately broken fixture")
    assert_fec_acceptable(fec_fixture)

def test_balances(fec_fixture, assert_fec_balances):
    tolerance = Decimal("0.01") if "off_by_one_cent" in fec_fixture.name else Decimal("0")
    assert_fec_balances(fec_fixture, tolerance=tolerance)
```

```text
$ pytest --hardmoney-fixtures=~/hardmoney/tests/fixtures/golden -v
============================= test session starts ==============================
platform darwin -- Python 3.14.2, pytest-9.1.1, pluggy-1.6.0
plugins: hardmoney-3.0.1
collected 18 items

test_golden.py::test_acceptable[f1m.fec] PASSED                          [  5%]
test_golden.py::test_acceptable[f24.fec] PASSED                          [ 11%]
test_golden.py::test_acceptable[f3.fec] PASSED                           [ 16%]
test_golden.py::test_acceptable[f3p.fec] PASSED                          [ 22%]
test_golden.py::test_acceptable[f3x.fec] PASSED                          [ 27%]
test_golden.py::test_acceptable[f3x_cover_off_by_one_cent.fec] PASSED    [ 33%]
test_golden.py::test_acceptable[f3x_duplicate_transaction_id.fec] XFAIL  [ 38%]
test_golden.py::test_acceptable[f3x_missing_required_field.fec] XFAIL    [ 44%]
test_golden.py::test_acceptable[f99.fec] PASSED                          [ 50%]
test_golden.py::test_balances[f1m.fec] PASSED                            [ 55%]
test_golden.py::test_balances[f24.fec] PASSED                            [ 61%]
test_golden.py::test_balances[f3.fec] PASSED                             [ 66%]
test_golden.py::test_balances[f3p.fec] PASSED                            [ 72%]
test_golden.py::test_balances[f3x.fec] PASSED                            [ 77%]
test_golden.py::test_balances[f3x_cover_off_by_one_cent.fec] PASSED      [ 83%]
test_golden.py::test_balances[f3x_duplicate_transaction_id.fec] PASSED   [ 88%]
test_golden.py::test_balances[f3x_missing_required_field.fec] PASSED     [ 94%]
test_golden.py::test_balances[f99.fec] PASSED                            [100%]

======================== 16 passed, 2 xfailed in 0.02s =========================
```

Use the `--hardmoney-fixtures=DIR` form with the equals sign; with a space
pytest also takes the directory into account when it infers `rootdir`.

When an assertion fails, the message is the finding list in
`hardmoney validate` / `hardmoney reconcile` format, with no Python
traceback:

```text
_________________________________ test_broken __________________________________
~/hardmoney/tests/fixtures/golden/f3x_duplicate_transaction_id.fec (F3XN v8.5) has 1 error(s):
  ERROR line 4 SA11B transaction_id: Tran ID SA11AI.1 is NOT UNIQUE - This one is same as other(s) (first used on line 3)
__________________________________ test_cent ___________________________________
~/hardmoney/tests/fixtures/golden/f3x_cover_off_by_one_cent.fec (F3XN): 2 of 69 cover line(s) disagree with the schedules (tolerance 0):
  DIFF col A line 11(a)(i)   reported        10000.24 expected        10000.23 delta         0.01  = sum of SchA.contribution_amount on SA11AI/SA11A1
  DIFF col A line 11(a)(iii) reported        10004.00 expected        10004.01 delta        -0.01  = 11(a)(i) + 11(a)(ii)
```

## The golden fixture pack

[`tests/fixtures/golden/`](https://github.com/cgorski/hardmoney/tree/main/tests/fixtures/golden)
holds nine spec-8.5 `.fec` files and their expected outputs, generated by
`examples/golden_fixtures.rs` from constants alone (the seed
`hardmoney-golden/1` is in every `HDR`), so the pack is the same bytes at
every commit until someone changes the generator on purpose and bumps
its version. `tests/golden_fixtures.rs` regenerates it in memory and
fails on any difference.

| File | What it is |
|---|---|
| `f3x.fec` | Form 3X with Schedules A, B, C, D, E, F, H3, H4; every Column A and Column B line balances. Built from FECfile+'s own `test_calculate_summary_column_a` transaction set (public domain), plus H3/H4 records. |
| `f3.fec`, `f3p.fec` | Form 3 and Form 3P, balanced, with the schedule lines their Column A rules sum. |
| `f24.fec`, `f1m.fec`, `f99.fec` | A 48-hour report with two Schedule E lines, a multicandidate notification, a Form 99 with a `[BEGINTEXT]` block. |
| `f3x_cover_off_by_one_cent.fec` | 11(a)(i) reported one cent high. Validates clean; `reconcile` flags 11(a)(i) and 11(a)(iii). |
| `f3x_duplicate_transaction_id.fec` | one `duplicate_transaction_id` error (FEC message #40). |
| `f3x_missing_required_field.fec` | one `required_field_empty` error (FEC message #3). |

Beside each file: `<name>.validation.json` (exactly what `hardmoney
validate <name>.fec --json` prints), `<name>.reconciliation.json` for the
three periodic forms (`hardmoney reconcile <name>.fec --json --all`),
`f3x.expected_column_a.json`, and a `MANIFEST.json` with SHA-256, form,
description, provenance, and the expected findings per file. All nine were
submitted to the FEC's WebCheck on 2026-09-15: the six clean files
`SUCCESS` with no findings, the one-cent file `WARNINGS` (`Subtotal
$10000.24 not supported by Schedule A`), the other two `ERRORS` with the
single intended message each. The pack's `README.md` has the table and
the provenance, including why the committee ids are those of committees
terminated decades ago (WebCheck checks every id against the FEC's
registry, so no fictitious id can pass it).

`f3x.expected_column_a.json` is the artifact for FECfile+'s stubbed
lines. On every line `calculate_summary_column_a` derives from Schedules
A to F it equals that test's own assertions (`line_11ai` 10000.23,
`line_15` 2125.79, `line_24` 151.00, `line_28d` 604.50, ...). On the five
lines the calculator sets to zero it carries what the H records imply:
`line_18a` 1000.00 (the sum of H3 `transferred_amount` over an `AD` share
of 750 and a `DF` share of 250, where summing `total_amount_transferred`
would give 2000), `line_21ai` 330.00 and `line_21aii` 670.00 (H4 shares
with the memo breakdown excluded), `line_30ai` and `line_30aii` 0.00 (no
H6). Values are strings with two decimals so that `Decimal(v)` is exact.

Using it in a Django test, without the plugin:

```python
import json
from decimal import Decimal
from pathlib import Path

from hardmoney.compat import fecfile_validate as fv

GOLDEN = Path(settings.BASE_DIR) / "fixtures" / "golden"   # a copy of tests/fixtures/golden

class GoldenFormThreeXTest(TestCase):
    def test_summary_matches_the_golden_column_a(self):
        expected = {k: Decimal(v) for k, v in json.loads((GOLDEN / "f3x.expected_column_a.json").read_text()).items()}
        self.assertEqual(fv.summary_column_a(GOLDEN / "f3x.fec"), expected)

    def test_composer_output_is_acceptable(self):
        result = fv.validate_file(compose_dot_fec(self.report.id))
        self.assertEqual([e.message for e in result.errors], [])
```

The first test is the shape of the parity check proposed to the FECfile+
team: load the transactions of `f3x.fec` (or `generate_data` plus two H3
and two H4 records) into a report, run `calculate_summary_column_a`, and
compare with the JSON; today the five stubbed keys are where the two
disagree.

## Limits

- **Record-level validation is emulated.** `validate_record` wraps the
  record in a synthetic filing and filters findings to its line. It sees
  what a per-record schema sees and a little more (state and ZIP formats,
  dates, amounts, the character set), but not what only the whole file can
  show: duplicate transaction ids, a memo's parent, the filer id on every
  line, one cover per file. Use `validate_file` on the composed `.fec`
  for those.
- **Summaries are for F3X, F3, and F3P only.** `summary_column_a` and
  `assert_fec_balances` rest on `Filing.reconcile()`, which has rule
  tables for those three forms; `summary_column_a` raises
  `UnsupportedForm` for anything else and `assert_fec_balances` returns
  `None`.
- **Column B is not chained.** The golden files are first-quarter
  reports, so their Column B equals Column A and balances by formula. On a
  later report Column B sums prior filings; `reconcile()` checks its
  formulas, not its sums, and `summary_column_a` returns Column A only.
- **`ValidationError` is a superset, not a copy.** `message` and `path`
  match fecfile-validate; the added attributes are hardmoney's. jsonschema's
  `schema`, `schema_path`, and `validator_value` have no meaning here and
  are not provided.
- **Committee ids in the golden pack are real registry ids** of
  long-terminated committees, by necessity (see the pack's `README.md`);
  everything else in the files is invented.
