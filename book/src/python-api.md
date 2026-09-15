<!-- Generated from python/python/hardmoney/_hardmoney.pyi by scripts/gen_python_api.py; do not edit. -->

# Python API reference

> Generated from
> [`_hardmoney.pyi`](https://github.com/cgorski/hardmoney/blob/main/python/python/hardmoney/_hardmoney.pyi)
> by `scripts/gen_python_api.py`; do not edit this page by hand. Edit the
> stub and run `python3 scripts/gen_python_api.py`.

Every name below is importable from the top-level package:
`import hardmoney`. The same text is available at the prompt as
`help(hardmoney.Filing)`, `help(hardmoney.parse)`, and so on, because
the compiled module carries the same docstrings as the stub. For a guided
introduction see [Getting started with Python](./python.md); for worked
examples see the [Python cookbook](./python-cookbook.md).

Types in signatures are written the way Python 3.10+ prints them
(`int | None` rather than `Optional[int]`); `Decimal` is
`decimal.Decimal` and `date` is `datetime.date`.

Contents:

- [Functions](#functions)
- [Constants](#constants)
- [`Filing`](#filing)
- [`Line`](#line)
- [`Validation`](#validation)
- [`Finding`](#finding)
- [`Reconciliation`](#reconciliation)
- [`LineCheck`](#linecheck)
- [`FecError`](#fecerror)
- [`UnsupportedForm`](#unsupportedform)

## Functions {#functions}

### `parse` {#parse}

```python
def parse(data: bytes | bytearray | str, *, lenient: bool = False) -> Filing
```

Parse a filing from `bytes` (UTF-8, falling back to Windows-1252) or `str`.

Strict by default: the first body line that cannot be parsed raises
[`FecError`](#fecerror). With `lenient=True` such lines are recorded in
[`Filing.skipped`](#filing-skipped) instead. Raises `TypeError` for any other type.

See also: [`Filing`](#filing).

### `parse_file` {#parse_file}

```python
def parse_file(path: str | os.PathLike[str], *, lenient: bool = False) -> Filing
```

Parse a `.fec` file from disk by streaming it.

Peak memory is the parsed lines alone rather than the file plus its
decoded text. A missing or unreadable file raises the matching
`OSError` (`FileNotFoundError`, ...); a malformed one raises
[`FecError`](#fecerror).

See also: [`Filing`](#filing).

### `fetch` {#fetch}

```python
def fetch(filing_id: int) -> Filing
```

Download a filing from `docquery.fec.gov/dcdev/posted/<filing_id>.fec`
and parse it strictly. Network failures raise [`FecError`](#fecerror).

See also: [`Filing`](#filing).

### `tables` {#tables}

```python
def tables() -> list[str]
```

Every table hardmoney knows, by FEC-style name (`"F3X"`, `"SchA"`,
`"TEXT"`), in name order.

### `layout` {#layout}

```python
def layout(table: str, version: str) -> list[tuple[str, int]]
```

The column layout of `table` at spec `version` as `(field, column)`
pairs in table order, `column` being 0-based.

Raises `ValueError` for an unknown table or a malformed version, and
[`FecError`](#fecerror) when the bundled data has no layout for that table at
that version.

### `field_spec` {#field_spec}

```python
def field_spec(table: str, name: str) -> dict[str, Any] | None
```

The FEC's specification of one field of `table` at
[`BUNDLED_SPEC_VERSION`](#bundled_spec_version), or `None` if the current spec does not
document that field. Raises `ValueError` for an unknown table.

Keys: `column` (int), `description` (str), `kind` (`"alpha"`,
`"alpha_numeric"`, `"numeric"`, `"amount"`, `"unknown"`),
`max_len` (int | None), `required` (`"none"`, `"error"`,
`"warning"`, `"conditional"`), `condition` (str | None),
`sample` (str | None), `value_reference` (str | None), `rule`
(str | None), `forms` (list[str]), `allowed_values` (list[str]),
`pattern` (str | None).

## Constants {#constants}

### `BUNDLED_SPEC_VERSION` {#bundled_spec_version}

```python
BUNDLED_SPEC_VERSION: str
```

The FEC spec version whose field specifications are bundled (`field_spec`
describes fields at this version), e.g. `"8.5"`.

### `__version__` {#version}

```python
__version__: str
```

The package version (same as the Rust crate's).

## Filing {#filing}

```python
@final
class Filing
```

A parsed FEC electronic filing: header, cover line, and every body line.

Obtain one with [`parse`](#parse), [`parse_file`](#parse_file), or [`fetch`](#fetch).

### `form_type` {#filing-form_type}

```python
@property
def form_type(self) -> str
```

*Read-only property.*

The top-level form type as filed, upper-cased, e.g. `"F3XA"`.

### `base_form_type` {#filing-base_form_type}

```python
@property
def base_form_type(self) -> str
```

*Read-only property.*

`form_type` with any amendment/new/termination designator
stripped, e.g. `"F3X"`.

### `version` {#filing-version}

```python
@property
def version(self) -> str
```

*Read-only property.*

The FEC spec version every line was parsed with, e.g. `"8.5"`.

### `is_amendment` {#filing-is_amendment}

```python
@property
def is_amendment(self) -> bool
```

*Read-only property.*

True when the form type designates an amendment.

### `amends_filing` {#filing-amends_filing}

```python
@property
def amends_filing(self) -> int | None
```

*Read-only property.*

The filing number this filing amends (from the header's `FEC-<n>`
report id), or `None`.

### `header` {#filing-header}

```python
@property
def header(self) -> dict[str, str]
```

*Read-only property.*

The `HDR` record keyed like the Rust `Header` struct:
`record_type`, `ef_type`, `fec_version_raw`, `version`,
`soft_name`, `soft_ver`, `report_id`, `report_number`,
`comment`, and -- on spec 3.x-5.x filings only -- `name_delim`.

### `summary` {#filing-summary}

```python
@property
def summary(self) -> Line
```

*Read-only property.*

The cover/summary line (row 2 of the file).

See also: [`Line`](#line).

### `lines` {#filing-lines}

```python
@property
def lines(self) -> list[Line]
```

*Read-only property.*

Every body line in file order. Builds a new list of handles on each
access; prefer [`iter_lines`](#filing-iter_lines) or [`lines_for`](#filing-lines_for) in a loop over
a large filing.

See also: [`Line`](#line).

### `skipped` {#filing-skipped}

```python
@property
def skipped(self) -> list[dict[str, Any]]
```

*Read-only property.*

Body lines a lenient parse could not interpret, each a dict with
`line_no` (int), `form_type` (str), and `reason` (str). Always
empty after a strict parse.

### `lines_for` {#filing-lines_for}

```python
def lines_for(self, table: str) -> list[Line]
```

The body lines belonging to one table, e.g. `"SchA"`. Raises
`ValueError` for a table name hardmoney does not know.

See also: [`Line`](#line).

### `iter_lines` {#filing-iter_lines}

```python
def iter_lines(self, tables: Sequence[str] | None = None) -> Iterator[Line]
```

Iterate body lines, optionally restricted to the given tables.
(v1 materialises the selection; the interface is the streaming one.)

See also: [`Line`](#line).

### `to_fec` {#filing-to_fec}

```python
def to_fec(self) -> bytes
```

The filing in canonical `.fec` form: Windows-1252 when every
character is representable (the FEC's character set), else UTF-8.

### `to_fec_string` {#filing-to_fec_string}

```python
def to_fec_string(self) -> str
```

The filing in canonical `.fec` form as text (CRLF line endings,
full record width, one wrapping quote pair and padding removed).

### `validate` {#filing-validate}

```python
def validate(self) -> Validation
```

Check the filing against the FEC's acceptance rules. Never raises.
Lines skipped by a lenient parse appear as `unrecognized_form_type`
warnings.

See also: [`Validation`](#validation).

### `reconcile` {#filing-reconcile}

```python
def reconcile(self) -> Reconciliation
```

Recompute every cover-page line from the schedules and other cover
lines. Raises [`UnsupportedForm`](#unsupportedform) unless the cover is F3X, F3,
or F3P.

See also: [`Reconciliation`](#reconciliation).

### `__repr__` {#filing-repr}

```python
def __repr__(self) -> str
```

`<hardmoney.Filing F3XN v8.5 (144 body lines)>`.

## Line {#line}

```python
@final
class Line
```

One record of a filing: the cover line or a schedule / sub-form / TEXT line.

Behaves like a read-mostly mapping from canonical field name to the
value as filed (trimmed, otherwise verbatim). A `Line` is a handle
into its `Filing`: [`set`](#line-set) is visible to [`Filing.to_fec`](#filing-to_fec).

### `table` {#line-table}

```python
@property
def table(self) -> str
```

*Read-only property.*

The format table this line was parsed with, e.g. `"SchA"`.

### `form_type` {#line-form_type}

```python
@property
def form_type(self) -> str
```

*Read-only property.*

The form-type token from column 0, upper-cased, e.g. `"SA11AI"`.
The `form_type` *field* keeps the token exactly as filed.

### `line_no` {#line-line_no}

```python
@property
def line_no(self) -> int
```

*Read-only property.*

1-based physical line number in the source file (0 if synthetic).

### `is_memo` {#line-is_memo}

```python
@property
def is_memo(self) -> bool
```

*Read-only property.*

True when `memo_code` is `X` (case-insensitive). Memo entries are
excluded from every cover-page total.

### `__getitem__` {#line-getitem}

```python
def __getitem__(self, name: str) -> str
```

The value as filed (`""` when blank); `KeyError` if the field
does not exist in this filing's layout for the table.

### `__contains__` {#line-contains}

```python
def __contains__(self, name: str) -> bool
```

Whether the layout has the field.

### `get` {#line-get}

```python
@overload
def get(self, name: str) -> str | None
@overload
def get(self, name: str, default: _T) -> str | _T
```

The value, or `default` if the field does not exist in the layout.
A blank field is `""`, not the default.

### `keys` {#line-keys}

```python
def keys(self) -> list[str]
```

Field names in layout (table) order.

### `items` {#line-items}

```python
def items(self) -> list[tuple[str, str]]
```

`(name, value)` pairs in layout order, blanks included.

### `to_dict` {#line-to_dict}

```python
def to_dict(self) -> dict[str, str]
```

Every field as a dict in layout order, blanks included.

### `set` {#line-set}

```python
def set(self, name: str, value: str) -> None
```

Set a field (trimmed like the parser). `KeyError` if the layout has
no such field. Setting `form_type` updates [`form_type`](#line-form_type) too.

### `amount` {#line-amount}

```python
def amount(self, name: str) -> Decimal | None
```

The field as an exact dollar amount (scale 2), or `None` when blank
or not a valid FEC amount. `KeyError` if the field is not in the
layout.

### `date` {#line-date}

```python
def date(self, name: str) -> date | None
```

The field as a `YYYYMMDD` date, or `None` when blank, zero-filled,
or not a real date. `KeyError` if the field is not in the layout.

### `__repr__` {#line-repr}

```python
def __repr__(self) -> str
```

`<hardmoney.Line SA11AI (SchA) line 3>`: form type, table, and
physical line number.

## Validation {#validation}

```python
@final
class Validation
```

The result of [`Filing.validate`](#filing-validate): every finding in file order.

`len(v)` is the number of findings, `str(v)` prints one finding per
line in the WebCheck style the CLI uses, and iterating yields
[`Finding`](#finding) objects.

### `findings` {#validation-findings}

```python
@property
def findings(self) -> list[Finding]
```

*Read-only property.*

Every finding, in line order.

See also: [`Finding`](#finding).

### `errors` {#validation-errors}

```python
@property
def errors(self) -> list[Finding]
```

*Read-only property.*

The error-severity findings: the FEC would reject the filing.

See also: [`Finding`](#finding).

### `warnings` {#validation-warnings}

```python
@property
def warnings(self) -> list[Finding]
```

*Read-only property.*

The warning-severity findings: reported, but the filing is accepted.

See also: [`Finding`](#finding).

### `is_acceptable` {#validation-is_acceptable}

```python
@property
def is_acceptable(self) -> bool
```

*Read-only property.*

True when there are no error-severity findings.

### `__len__` {#validation-len}

```python
def __len__(self) -> int
```

The number of findings, errors and warnings together.

### `__iter__` {#validation-iter}

```python
def __iter__(self) -> Iterator[Finding]
```

Yield every [`Finding`](#finding) in line order (the same list as
[`findings`](#validation-findings)).

### `__str__` {#validation-str}

```python
def __str__(self) -> str
```

One finding per line in the CLI's WebCheck style, e.g.
`ERROR line 2 F3XA date_signed: 20261301 is not a Real Date`.
Empty when there are no findings.

### `__repr__` {#validation-repr}

```python
def __repr__(self) -> str
```

`<hardmoney.Validation 5 error(s), 0 warning(s)>`.

## Finding {#finding}

```python
@final
class Finding
```

One validation message, tied to a line (and usually a field).

### `severity` {#finding-severity}

```python
@property
def severity(self) -> str
```

*Read-only property.*

`"error"` (the FEC rejects the filing) or `"warning"`.

### `rule` {#finding-rule}

```python
@property
def rule(self) -> str
```

*Read-only property.*

The rule's stable snake_case name, e.g. `"required_field_empty"`.

### `line_no` {#finding-line_no}

```python
@property
def line_no(self) -> int
```

*Read-only property.*

1-based physical line in the `.fec` file (1 = `HDR`, 2 = cover).

### `form_type` {#finding-form_type}

```python
@property
def form_type(self) -> str
```

*Read-only property.*

The record's form-type token, upper-cased (`"HDR"`, `"SA11AI"`).

### `field` {#finding-field}

```python
@property
def field(self) -> str | None
```

*Read-only property.*

The canonical field name when the finding is about one field.

### `message` {#finding-message}

```python
@property
def message(self) -> str
```

*Read-only property.*

A complete sentence a filer could act on, worded after the FEC's.

### `__str__` {#finding-str}

```python
def __str__(self) -> str
```

The finding as one line of `hardmoney validate` output:
`ERROR line 2 F3XA date_signed: 20261301 is not a Real Date`
(severity, line number, form type, field, message).

### `__repr__` {#finding-repr}

```python
def __repr__(self) -> str
```

`<hardmoney.Finding error not_a_real_date line 2 date_signed>`.

## Reconciliation {#reconciliation}

```python
@final
class Reconciliation
```

Every cover-page line check for one filing, from [`Filing.reconcile`](#filing-reconcile).

`str(r)` prints one check per line followed by a one-line summary, as
`hardmoney reconcile` does; `len(r)` is the number of checks.

### `form` {#reconciliation-form}

```python
@property
def form(self) -> str
```

*Read-only property.*

The cover form the rules came from: `"F3X"`, `"F3"`, or `"F3P"`.

### `checks` {#reconciliation-checks}

```python
@property
def checks(self) -> list[LineCheck]
```

*Read-only property.*

Every line check, Column A first, in rule-table order.

See also: [`LineCheck`](#linecheck).

### `balances` {#reconciliation-balances}

```python
@property
def balances(self) -> bool
```

*Read-only property.*

True when every line agrees exactly with its rule.

### `mismatches` {#reconciliation-mismatches}

```python
def mismatches(self) -> list[LineCheck]
```

The checks that do not match (`delta != 0`, or a floor undershot).

See also: [`LineCheck`](#linecheck).

### `line` {#reconciliation-line}

```python
def line(self, column: str, line: str) -> LineCheck | None
```

The check for one line label in one column (`"A"` or `"B"`), e.g.
`r.line("A", "11(a)(i)")`, or `None` if the form has no such rule
(or this spec version lacks the line). `ValueError` for any other
column.

See also: [`LineCheck`](#linecheck).

### `__len__` {#reconciliation-len}

```python
def __len__(self) -> int
```

The number of line checks (`len(r.checks)`).

### `__str__` {#reconciliation-str}

```python
def __str__(self) -> str
```

Every check as `hardmoney reconcile` prints it (one
[`LineCheck`](#linecheck) per line, `ok` or `DIFF`), then a summary
line such as `F3X: every line agrees with its rule` or
`F3X: 2 of 69 line(s) disagree`.

### `__repr__` {#reconciliation-repr}

```python
def __repr__(self) -> str
```

`<hardmoney.Reconciliation F3X 69 check(s), 0 mismatch(es)>`.

## LineCheck {#linecheck}

```python
@final
class LineCheck
```

The outcome of checking one cover-page line.

### `line` {#linecheck-line}

```python
@property
def line(self) -> str
```

*Read-only property.*

The FEC's line label, e.g. `"11(a)(i)"`, `"6(c)"`, `"31"`.

### `field` {#linecheck-field}

```python
@property
def field(self) -> str
```

*Read-only property.*

The canonical cover-page field holding the reported value.

### `column` {#linecheck-column}

```python
@property
def column(self) -> str
```

*Read-only property.*

`"A"` (this period) or `"B"` (year/cycle to date).

### `rule` {#linecheck-rule}

```python
@property
def rule(self) -> str
```

*Read-only property.*

The rule in the FEC's notation, e.g. `"= 11ai + 11aii"`.

### `reported` {#linecheck-reported}

```python
@property
def reported(self) -> Decimal | None
```

*Read-only property.*

The value on the cover page, or `None` if blank or not a valid
amount ([`reported_unparseable`](#linecheck-reported_unparseable) tells the two apart); treated as
0 in [`delta`](#linecheck-delta).

### `expected` {#linecheck-expected}

```python
@property
def expected(self) -> Decimal
```

*Read-only property.*

The value the schedules or formula imply.

### `delta` {#linecheck-delta}

```python
@property
def delta(self) -> Decimal
```

*Read-only property.*

`reported - expected` (a blank `reported` counts as 0).

### `relation` {#linecheck-relation}

```python
@property
def relation(self) -> str
```

*Read-only property.*

`"equal"` (must match exactly) or `"at_least"` (the itemized sum
is a floor: sub-$200 items may be reported unitemized).

### `matches` {#linecheck-matches}

```python
@property
def matches(self) -> bool
```

*Read-only property.*

True when the cover page satisfies the rule.

### `violation` {#linecheck-violation}

```python
@property
def violation(self) -> Decimal
```

*Read-only property.*

How far the rule is violated: `|delta|` for an equality, the
shortfall for a floor, `0` when it matches.

### `lines_summed` {#linecheck-lines_summed}

```python
@property
def lines_summed(self) -> int
```

*Read-only property.*

For schedule sums, how many body lines contributed.

### `reported_unparseable` {#linecheck-reported_unparseable}

```python
@property
def reported_unparseable(self) -> bool
```

*Read-only property.*

True when the cover page carries a value that is not a valid FEC
amount (e.g. `$5,500.00`). A blank value is `reported=None` with
this `False`.

### `__str__` {#linecheck-str}

```python
def __str__(self) -> str
```

The check as one line of `hardmoney reconcile` output:
`ok   col A line 11(a)(i)   reported 13736.02 expected 13736.02 delta 0.00  = sum of SchA.contribution_amount on SA11AI/SA11A1`
(`DIFF` instead of `ok` when it does not match; columns are
padded for alignment).

### `__repr__` {#linecheck-repr}

```python
def __repr__(self) -> str
```

`<hardmoney.LineCheck col A line 11(a)(i) ok>` (`DIFF` when the
check fails).

## FecError {#fecerror}

```python
class FecError(ValueError)
```

A filing could not be parsed, written, or interpreted.

`line_no` is the 1-based physical line the error refers to, or `None`
when the error is not about one line (a bad header, a missing cover line).

### `line_no` {#fecerror-line_no}

```python
line_no: int | None
```

The 1-based physical line the error is about, or `None`. Always
present, even on an exception constructed from Python.

## UnsupportedForm {#unsupportedform}

```python
class UnsupportedForm(FecError)
```

The cover form has no reconciliation rule table (only F3X, F3, and F3P do).

See also: [`FecError`](#fecerror).
