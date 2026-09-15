# Python cookbook

[Python](./python.md) documents the package. This chapter is the same
package applied to the things people actually do with FEC filings, one
runnable script per task. The scripts live in
[`python/examples/`](https://github.com/cgorski/hardmoney/tree/main/python/examples)
in the repository; each takes a file path on the command line, defaults to
a fixture from `tests/fixtures/`, and is run by
`python/tests/test_examples.py`. Every output block below was captured
from a run; the only edits are temporary-directory paths shortened to
`/tmp/...` and long listings cut with `...`. Money is `decimal.Decimal`
throughout; no script converts an amount to a float.

## The whole API on one screen

```python
import hardmoney
filing = hardmoney.parse_file("report.fec")          # or parse(bytes, lenient=True), fetch(id)
filing.form_type, filing.version, filing.header       # "F3XN", "8.5", {"soft_name": ..., ...}
cover = filing.summary                                # the cover page, a Line
cover["committee_name"], cover.amount("col_a_total_receipts"), cover.date("coverage_from_date")
for line in filing.lines_for("SchA"):                 # or iter_lines(["SchA", "SchB"]); .lines for all
    line.form_type, line.is_memo, line.amount("contribution_amount"), line.to_dict()
line.set("contributor_state", "VA"); open("out.fec", "wb").write(filing.to_fec())
v = filing.validate();  v.is_acceptable, v.errors, v.warnings        # Finding: rule, line_no, field, message
r = filing.reconcile(); r.balances, r.mismatches(), r.line("A", "11(a)(i)").delta   # F3X, F3, F3P
hardmoney.tables(), hardmoney.layout("SchA", "8.5"), hardmoney.field_spec("SchA", "contribution_amount")
```

Errors are `hardmoney.FecError` (a `ValueError` with `.line_no`);
`reconcile()` on a form without cover-page rules raises
`hardmoney.UnsupportedForm`.

## Parsing

### 01: parse a file and print the header and cover page

[`01_parse_and_print_header.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/01_parse_and_print_header.py).
Cover pages differ by form (an F3X has receipts totals, an F99 does
not), so every lookup goes through `in` and `get` and skips what the
layout lacks.

```python
filing = hardmoney.parse_file(path)
print(f"form type:       {filing.form_type} (base {filing.base_form_type})")
for key, value in filing.header.items():
    print(f"  {key:16} {value!r}")
cover = filing.summary
for name in ["coverage_from_date", "coverage_through_date", "date_signed"]:
    if name in cover:
        print(f"  {name:38} {cover.date(name)}")       # datetime.date or None
for name in ["col_a_total_receipts", "col_a_total_disbursements"]:
    if name in cover:
        print(f"  {name:38} {cover.amount(name)}")     # Decimal or None
```

```text
file:            F3XN_2011831.fec
form type:       F3XN (base F3X)
spec version:    8.5
amendment:       False
body lines:      144

header (HDR record):
  record_type      'HDR'
  ef_type          'FEC'
  fec_version_raw  '8.5'
  version          '8.5'
  soft_name        'Vocus PAC Management'
  soft_ver         '8.02.1066'
  report_id        ''
  report_number    '0'
  comment          ''

cover page (line 2, table F3X):
  filer_committee_id_number              'C00140855'
  committee_name                         'FirstEnergy Corp Political Action Committee'
  report_code                            'M9'
  treasurer_last_name                    'Helinski'
  treasurer_first_name                   'Amanda M'
  coverage_from_date                     2026-08-01
  coverage_through_date                  2026-08-31
  date_signed                            2026-09-14
  col_a_cash_on_hand_beginning_period    1980327.93
  col_a_total_receipts                   15245.52
  col_a_total_disbursements              10023.01
  col_a_cash_on_hand_close_of_period     1985550.44
```

### 02: count lines by table

[`02_count_lines_by_table.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/02_count_lines_by_table.py).
`Line.table` is the format table a line was parsed with (`SchA`);
`Line.form_type` is the token in column 0 (`SA11AI`, `SA11C`). Several
tokens share one table.

```python
by_table: Counter[str] = Counter()
by_form_type: Counter[str] = Counter()
for line in filing.iter_lines():
    by_table[line.table] += 1
    by_form_type[line.form_type] += 1
    if line.is_memo:
        memos[line.table] += 1
```

```text
F3XA_2011821.fec: F3XA v8.5, 26 body lines

table     lines   memo
SchB          7      0
SchE          6      0
SchA          5      1
SchD          5      0
TEXT          3      0

form type   lines
SA11AI          4
SA11C           1
SB21B           7
SD10            5
SE              6
TEXT            3
```

### 03: Schedule A as a list of typed dicts

[`03_schedule_a_to_dicts.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/03_schedule_a_to_dicts.py).
`to_dict()` gives every field as a string; pick the fields you want and
convert the two that have better Python types.

```python
def schedule_a_record(line: hardmoney.Line) -> dict[str, Any]:
    return {
        "line_no": line.line_no,
        "transaction_id": line["transaction_id"],
        "contributor": contributor_name(line),
        "employer": line["contributor_employer"],
        "date": line.date("contribution_date"),        # datetime.date
        "amount": line.amount("contribution_amount"),  # Decimal
        "memo": line.is_memo,
    }

records = [schedule_a_record(line) for line in filing.lines_for("SchA")]
total = sum((r["amount"] for r in records if not r["memo"] and r["amount"] is not None), Decimal("0"))
```

```text
132 Schedule A record(s) from F3XN_2011831.fec

{'line_no': 3, 'form_type': 'SA11AI', 'transaction_id': 'PR12339386106229', 'entity_type': 'IND', 'contributor': 'Mroczynski, Mark', 'city': 'Akron', 'state': 'OH', 'employer': 'FirstEnergy', 'occupation': 'President Transmission -', 'date': datetime.date(2026, 8, 31), 'amount': Decimal('380.00'), 'aggregate': Decimal('3420.00'), 'memo': False}
...
amount is Decimal, date is date
total of non-memo contributions: 13736.02
```

`sum()` needs the `Decimal("0")` start value; without it an empty list
sums to the integer `0`.

### 04: top donors with a Counter of Decimals

[`04_top_donors.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/04_top_donors.py).
A `Counter`'s values can be anything that supports `+=`, so it
aggregates Decimals directly, and `most_common` sorts them.

```python
totals: Counter[tuple[str, str]] = Counter()
for line in filing.lines_for("SchA"):
    if line.is_memo:
        continue
    amount = line.amount("contribution_amount")
    if amount is None:
        continue
    totals[donor_key(line)] += amount
    if line.form_type in {"SA11AI", "SA11A1", "SA17A"}:
        individuals += amount
for rank, ((name, employer), total) in enumerate(totals.most_common(10), start=1):
    print(f"{rank:4}  {total:>12}  {count[(name, employer)]:3}  {name} / {employer}")
```

```text
F3A_2011812.fec: 208 non-memo Schedule A line(s), 58 distinct donor(s), total 83267.38

rank         total    n  donor / employer
   1      38123.83  111  MIZUSAWA, BERT / SELF
   2       7000.00    1  MIZUSAWA, KOREN / NONE
   3       7000.00    1  MIZUSAWA, SHANE / 535 PLUMBING LLC
   4       3500.00    1  WIGHTMAN, REGINA / RETIRED
   5       2500.00    1  JACOBS, JAMES / SELF
   6       1979.00    1  OCHMAN, ROBERT / SAIC
   7       1041.02    1  FLOOD, BRYAN / STRIDE, INC.
   8       1041.02    1  JONES, SCOTT / RETIRED
   9       1041.02    1  RECTOR, MICHELE / RETIRED
  10       1000.00    1  BRAGA, JEANNETTE / RETIRED

itemized individuals: Decimal sum 44776.25, cover page 44776.25, equal: True
```

The last line is the reason for `Decimal`: 96 itemized contributions
summed exactly equal the cover page's 11(a)(i).

### 05: lenient parsing

[`05_lenient_parsing.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/05_lenient_parsing.py).
The fixtures were all accepted by the FEC, so the script appends two bad
lines to the file's bytes to have something to skip.

```python
try:
    hardmoney.parse(data)
except hardmoney.FecError as e:
    print(f"  FecError at line {e.line_no}: {e}")

filing = hardmoney.parse(data, lenient=True)
for skipped in filing.skipped:
    print(f"  line {skipped['line_no']:>3} {skipped['form_type']:6} {skipped['reason']}")
for finding in filing.validate().warnings:
    if finding.rule == "unrecognized_form_type":
        print(f"  {finding}")
```

```text
strict parse:
  FecError at line 9: no format table for form type 'ZZZ' (spec version 8.5) at line 9

lenient parse:
  6 line(s) parsed, 2 skipped
  line   9 ZZZ    unknown form type
  line  10 SX99   unknown form type

validate() reports each skipped line as a warning:
  WARN  line 9 ZZZ form_type: Unrecognized Form Type / Record Ignored ('ZZZ': unknown form type)
  WARN  line 10 SX99 form_type: Unrecognized Form Type / Record Ignored ('SX99': unknown form type)

parsed lines identical to the original file's: True
```

### 06: handling FecError

[`06_handle_fec_error.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/06_handle_fec_error.py).
`line_no` is the 1-based physical line, so the handler can show the
filer the raw record. `bytes.splitlines()` is safe here; see example 21
for why `str.splitlines()` is not.

```python
try:
    filing = hardmoney.parse(data)
except hardmoney.FecError as e:
    print(f"  {type(e).__name__}: {e}")
    if e.line_no is not None:
        bad = data.splitlines()[e.line_no - 1]
        print(f"  line {e.line_no}: {bad.decode('utf-8', 'replace').replace(chr(0x1c), '|')}")
```

```text
the file as is:
  ok: F3XA with 6 body line(s)

an unknown form type spliced in as line 4:
  FecError: no format table for form type 'BOGUS' (spec version 8.5) at line 4
  line 4: BOGUS|C00944124|X

the header alone:
  FecError: filing has no cover/summary line
  (not about one line)

something that is not a filing:
  FecError: filing has no cover/summary line
  (not about one line)

a missing path:
  FileNotFoundError (an OSError, not a FecError): /Users/chris.gorski/repos/hardmoney/tests/fixtures/does-not-exist.fec: No such file or directory (os error 2)

caught as ValueError: FecError
```

### 07: memo entries

[`07_memo_entries.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/07_memo_entries.py).
A memo entry itemizes something already counted elsewhere. Here a joint
fundraising transfer of $10,170.37 (line 6) is broken down by original
donor in three memo lines that back-reference it. The cover total
excludes them; a recomputed total must too.

```python
for token in tokens:
    lines = [l for l in sched_a if l.form_type == token]
    with_memos = sum((l.amount("contribution_amount") or Decimal(0) for l in lines), Decimal("0"))
    without = sum((l.amount("contribution_amount") or Decimal(0) for l in lines if not l.is_memo), Decimal("0"))
```

```text
F3XN_2011834.fec: 9 Schedule A line(s), 3 memo(s)

line token  memo     amount  back ref       name / memo text
   3 SA11C          5000.00                 STEPHENS INC. FEDERAL PAC
   4 SA11C          5000.00                 THE TRAVELERS COMPANIES, INC. PAC
   5 SA11C          5000.00                 AMGEN INC. PAC
   6 SA12          10170.37                 ONE TEAM SENATE MAJORITY (TRANSFER OF JOINT FUNDRAISING PROCEEDS)
   7 SA12   X        553.13  SA12.574943    GONSOULIN (JFC ATTRIB: ONE TEAM SENATE MAJORITY)
   8 SA12   X       5000.00  SA12.574943    SCHWARZMAN (JFC ATTRIB: ONE TEAM SENATE MAJORITY)
   9 SA12   X       5000.00  SA12.574943    SCHWARZMAN (JFC ATTRIB: ONE TEAM SENATE MAJORITY)
  10 SA16           5000.00                 BILL CASSIDY FOR U.S. SENATE
  11 SA17            237.93                 CHAIN BRIDGE BANK

SA12: 4 line(s)
  sum including memos: 20723.50
  sum excluding memos: 10170.37

cover line 12 (col_a_transfers_from_aff_other_party_cmttees) reported 10170.37; expected 10170.37 from 1 non-memo line(s): match
```

### 08: old spec versions

[`08_old_spec_versions.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/08_old_spec_versions.py).
A 2001 filing in spec 3.00 next to a 2026 one in 8.5. The canonical
names are the same and the columns are not, so code that uses names
works on both. The one real difference: the FEC split the single
`contributor_name` field into last/first/middle/prefix/suffix and an
organization name in spec 5.x, and `layout()` says which a version has.

```python
def itemized_total(filing: hardmoney.Filing) -> Decimal:
    return sum((l.amount("contribution_amount") or Decimal(0)
                for l in filing.lines_for("SchA") if not l.is_memo), Decimal("0"))

old_names = {n for n, _ in hardmoney.layout("SchA", old.version)}
new_names = {n for n, _ in hardmoney.layout("SchA", new.version)}
```

```text
F3XA spec 3.0: header keys ['comment', 'ef_type', 'fec_version_raw', 'name_delim', 'record_type', 'report_id', 'report_number', 'soft_name', 'soft_ver', 'version']
  software: KNOWLEDGE XP.1108; raw version string '3.00'
F3XN spec 8.5: header keys ['comment', 'ef_type', 'fec_version_raw', 'record_type', 'report_id', 'report_number', 'soft_name', 'soft_ver', 'version']
  software: Vocus PAC Management 8.02.1066; raw version string '8.5'

the same field names on the first Schedule A line of each:
  field                                        spec 3.0                     spec 8.5
  contribution_date                          '20010801'                   '20260831'
  contribution_amount                           '20.00'                     '380.00'
  contribution_aggregate                       '260.00'                    '3420.00'
  contributor_city                          'Elk River'                      'Akron'
  contributor_state                                'MN'                         'OH'
  contributor_employer                    'Merck-Medco'                'FirstEnergy'
  contributor_occupation     'Reg VP Clinical Services'   'President Transmission -'

but in different columns:
  contribution_date        column 14 in 3.0, column 19 in 8.5
  contribution_amount      column 15 in 3.0, column 20 in 8.5
  contributor_employer     column 11 in 3.0, column 23 in 8.5

Schedule A fields: 37 in 3.0, 45 in 8.5, 33 shared
  only in 3.0: ['amended_cd', 'contribution_purpose_code', 'contributor_name', 'donor_candidate_name']
  only in 8.5: ['contributor_first_name', 'contributor_last_name', 'contributor_middle_name', 'contributor_organization_name', 'contributor_prefix', 'contributor_suffix', 'donor_candidate_first_name', 'donor_candidate_last_name', 'donor_candidate_middle_name', 'donor_candidate_prefix', 'donor_candidate_suffix', 'donor_committee_name']
  3.0 name field: 'Aaland^Lyla L^Ms^'
  8.5 name fields: 'Mroczynski', 'Mark'

one function, both versions:
  spec 3.0: itemized sum 149408.52 vs cover 11(a)(i) 149408.52 -> True
  spec 8.5: itemized sum 13736.02 vs cover 11(a)(i) 13736.02 -> True
```

### 09: a running total over a 135 MB filing

[`09_stream_large_filing.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/09_stream_large_filing.py).
`parse_file` streams from disk; `iter_lines(["SchA"])` walks one
schedule. The file (FEC filing 2010101, a presidential post-general
report) is not in the repository, and the script exits quietly without
it.

```python
filing = hardmoney.parse_file(path)
for line in filing.iter_lines(["SchA"]):
    seen += 1
    if seen % 100_000 == 0:
        print(f"  ... {seen:>9,} Schedule A lines, running total {sum(totals.values(), Decimal('0')):>16,}")
    if line.is_memo:
        continue
    amount = line.amount("contribution_amount")
    if amount is not None:
        totals[line.form_type] += amount
```

```text
2010101.fec: 135 MB, F3PA v8.5, 704,651 body lines, parsed in 0.55s
committee: DONALD J. TRUMP FOR PRESIDENT, INC.

  ...   100,000 Schedule A lines, running total     9,915,397.12
  ...   200,000 Schedule A lines, running total    19,936,406.93
  ...   300,000 Schedule A lines, running total    30,126,457.39
  ...   400,000 Schedule A lines, running total    39,784,165.50
  ...   500,000 Schedule A lines, running total    49,617,293.92
  ...   600,000 Schedule A lines, running total    52,437,847.74

689,776 Schedule A line(s) (237,007 memo) in 0.49s
token        lines              total
SA17A      452,549      52,344,650.74
SA18             6      35,493,259.84
SA20A           78       4,968,692.96
SA17C           47          93,197.00
SA21            89          91,390.98

SA17A total 52344650.74 vs cover page itemized individuals 52344650.74: equal
```

452,549 individual contributions summed in `Decimal` land on the cover
page to the cent.

## Validating

### 10: findings grouped by severity

[`10_validate_by_severity.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/10_validate_by_severity.py).
`severity` is `"error"` (the FEC rejects the filing) or `"warning"`;
`rule` is a stable name to switch on. The script exits 1 when there are
errors.

```python
filing = hardmoney.parse_file(path, lenient=True)
v = filing.validate()
by_severity: dict[str, list[hardmoney.Finding]] = defaultdict(list)
for finding in v:
    by_severity[finding.severity].append(finding)
for severity in ("error", "warning"):
    rules = Counter(f.rule for f in by_severity[severity])
    for f in by_severity[severity]:
        where = f"line {f.line_no} {f.form_type}" + (f" {f.field}" if f.field else "")
        print(f"    {where}: {f.message}")
return 0 if v.is_acceptable else 1
```

```text
bad_dates_and_amounts.fec: 5 error(s), 0 warning(s); would be REJECTED by the FEC

ERRORS (5):
  not_a_real_date x2
  invalid_amount x2
  bad_date_format x1
    line 2 F3XA date_signed: 20261301 is not a Real Date
    line 4 SB21B expenditure_date: 20260231 is not a Real Date
    line 5 SB21B expenditure_date: Bad Date - 2026-06-22 not YYYYMMDD format
    line 6 SB21B expenditure_amount: Invalid Amount format: '$5,500.00' in EXPENDITURE AMOUNT {F3L Bundled} (digits with an optional sign and up to two decimals; no $ or commas)
    line 7 SB21B expenditure_amount: Invalid Amount format: '1500.005' in EXPENDITURE AMOUNT {F3L Bundled} (digits with an optional sign and up to two decimals; no $ or commas)
```

### 11: a directory to a CSV report

[`11_validate_directory_csv.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/11_validate_directory_csv.py).
One row per finding with the `csv` module; a file that does not parse
becomes a `parse_error` row instead of stopping the run.

```python
writer = csv.DictWriter(out, fieldnames=COLUMNS, lineterminator="\n")
writer.writeheader()
for path in sorted(root.rglob("*.fec")):
    try:
        filing = hardmoney.parse_file(path, lenient=True)
    except hardmoney.FecError as e:
        writer.writerow({"file": rel, "severity": "parse_error", "line_no": e.line_no or "", "message": str(e), ...})
        continue
    for f in filing.validate():
        writer.writerow({"file": rel, "form_type": filing.form_type, "severity": f.severity, "rule": f.rule,
                         "line_no": f.line_no, "record": f.form_type, "field": f.field or "", "message": f.message, ...})
```

Run over `tests/fixtures/invalid/` (selected rows):

```text
file,form_type,version,severity,rule,line_no,record,field,message
amendment_missing_ids.fec,F3XA,8.5,error,amendment_needs_original_id,1,HDR,report_id,"Amended filing must have an ID of the ""Original"" (expected FEC-<filing number>, found '')"
amendment_missing_ids.fec,F3XA,8.5,error,amendment_needs_number,1,HDR,report_number,"Amended filing must have an ""Amendment Number"" (found '')"
amendment_missing_ids.fec,F3XA,8.5,warning,embedded_double_quote,4,SB21B,payee_organization_name,"Embedded double-quotes ("") not allowed in PAYEE ORGANIZATION NAME"
bad_dates_and_amounts.fec,F3XA,8.5,error,not_a_real_date,2,F3XA,date_signed,20261301 is not a Real Date
bad_filer_id.fec,F3XA,8.5,error,filer_id_format,2,F3XA,filer_committee_id_number,ID# C0094412 NOT Correct FEC ID# Format
duplicate_tran_id.fec,F3XA,8.5,error,duplicate_transaction_id,5,SB21B,transaction_id_number,Tran ID SB21B.4120 is NOT UNIQUE - This one is same as other(s) (first used on line 4)
illegal_character.fec,F3XA,8.5,error,illegal_character,7,SB21B,payee_organization_name,Illegal character(s) found in text field: U+017E (ž)
wrong_schedule_for_form.fec,F24N,8.5,error,schedule_not_allowed_with_form,5,SA11AI,form_type,"Schedule does not belong with Form F24 (SchA is filed with F3, F3X, F3P, F3L)"
```

### 12: an exit code for CI

[`12_validate_exit_code.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/12_validate_exit_code.py).
`python 12_validate_exit_code.py build/*.fec || exit 1` in a build step
or pre-commit hook; `--strict` fails on warnings too.

```python
def check(path: Path, strict: bool) -> bool:
    try:
        filing = hardmoney.parse_file(path, lenient=True)
    except (hardmoney.FecError, OSError) as e:
        print(f"FAIL {path}: cannot parse: {e}")
        return False
    v = filing.validate()
    failing = v.errors + (v.warnings if strict else [])
    for f in failing:
        print(f"       {f}")
    return not failing

return 1 if results.count(False) else 0
```

```text
ok   /Users/chris.gorski/repos/hardmoney/tests/fixtures/F3XA_2011827.fec: F3XA v8.5, 0 error(s), 0 warning(s)
FAIL /Users/chris.gorski/repos/hardmoney/tests/fixtures/invalid/duplicate_tran_id.fec: F3XA v8.5, 1 error(s), 0 warning(s)
       ERROR line 5 SB21B transaction_id_number: Tran ID SB21B.4120 is NOT UNIQUE - This one is same as other(s) (first used on line 4)

1 of 2 file(s) pass
```

### 13: explain a finding with the field spec

[`13_explain_finding.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/13_explain_finding.py).
A finding names a form-type token; `field_spec` is keyed by table, so
find the line the finding is about and use its `table`.

```python
def table_of(filing: hardmoney.Filing, finding: hardmoney.Finding) -> Optional[str]:
    if finding.line_no == 1:
        return "HDR"
    if finding.line_no == 2:
        return filing.summary.table
    for line in filing.iter_lines():
        if line.line_no == finding.line_no:
            return line.table
    return None

for finding in v.errors:
    spec = hardmoney.field_spec(table_of(filing, finding), finding.field)
    print(f"  description: {spec['description']}")
    print(f"  type:        {spec['kind']}, max length {spec['max_len']}, required: {spec['required']}")
```

```text
field_too_long.fec: 2 error(s), 0 warning(s)

ERROR line 3 SA11AI contributor_last_name [field_too_long]
  message:     CONTRIBUTOR LAST NAME exceeds maximum length of 30 (31 characters)
  value:       'YYYYYYYYYYYYYYYYYYYYYYYYYYYYYYY' (31 chars)
  description: CONTRIBUTOR LAST NAME
  type:        alpha_numeric, max length 30, required: error
  FEC rule:    Required if [IND|CAN]
  sample:      Smith

ERROR line 4 SB21B payee_organization_name [field_too_long]
  message:     PAYEE ORGANIZATION NAME exceeds maximum length of 200 (201 characters)
  value:       'XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX...' (201 chars)
  description: PAYEE ORGANIZATION NAME
  type:        alpha_numeric, max length 200, required: error
  FEC rule:    Required if NOT [IND|CAN]
  sample:      John Smith & Co.
```

## Reconciling

### 14: the mismatches table

[`14_reconcile_mismatches.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/14_reconcile_mismatches.py).
Every fixture balances, so after reporting that, the script overstates
11(a)(i) by $100 in memory and reconciles again. Formulas are evaluated
over reported values, so the bad line shows up twice: against Schedule A
and in 11(a)(iii) = 11(a)(i) + 11(a)(ii). That pinpoints it.

```python
r = filing.reconcile()
for c in r.mismatches():
    print(f"  {c.column:3} {c.line:10} {c.reported!s:>14} {c.expected!s:>14} {c.delta!s:>12}  {c.rule}")

c = r.line("A", "11(a)(i)")
filing.summary.set(c.field, str(c.reported + Decimal("100.00")))
```

```text
F3XN_2011831.fec: F3XN v8.5
F3X: 69 check(s), 0 mismatch(es); balances: True

overstating column A line 11(a)(i) (col_a_individuals_itemized) by 100.00 in memory: 13736.02 -> 13836.02
F3X: 69 check(s), 2 mismatch(es); balances: False
  col line             reported       expected        delta  rule
  A   11(a)(i)         13836.02       13736.02       100.00  = sum of SchA.contribution_amount on SA11AI/SA11A1
  A   11(a)(iii)       15245.52       15345.52      -100.00  = 11(a)(i) + 11(a)(ii)
```

### 15: equal vs. at_least, and the $200 threshold

[`15_relations_and_threshold.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/15_relations_and_threshold.py).
Receipts and disbursements need itemizing only once the aggregate with
one person exceeds $200 in the cycle (11 CFR 104.3). For those lines the
schedule sum is a floor (`relation == "at_least"`); a cover total above
it is normal, one below it is a discrepancy. Everything that must always
be itemized is `"equal"`.

```python
floors = [c for c in r.checks if c.relation == "at_least"]
for c in floors:
    if c.column == "A" and c.reported is not None and c.delta > 0:
        print(f"    -> {c.delta} reported on the cover but not itemized ... matches={c.matches}, violation={c.violation}")

filing.summary.set(opex.field, str(opex.expected - Decimal("100.00")))   # undershoot the floor
```

```text
F3XA_2011821.fec: F3X, 69 checks, balances: True

5 floor check(s) ('at_least'), 64 exact check(s) ('equal')

floors with a schedule (column A): cover total >= itemized sum
  A 15        at_least reported        0.00 expected           0 delta         0  ok
  A 17        at_least reported        0.00 expected           0 delta         0  ok
  A 21(b)     at_least reported    27326.50 expected    27247.50 delta     79.00  ok
    -> 79.00 reported on the cover but not itemized: items under the $200 aggregate threshold. matches=True, violation=0
  A 28(a)     at_least reported        0.00 expected           0 delta         0  ok
  A 29        at_least reported        0.00 expected           0 delta         0  ok

exact checks against a schedule (column A, non-zero):
  A 10        equal    reported    36177.50 expected    36177.50 delta      0.00  ok
  A 11(a)(i)  equal    reported    52000.00 expected    52000.00 delta      0.00  ok
  A 11(c)     equal    reported    20000.00 expected    20000.00 delta      0.00  ok
  A 24        equal    reported   400245.63 expected   400245.63 delta      0.00  ok

setting 21(b) below its itemized sum (27247.50 -> 27147.50) in memory:
  A 21(b)     at_least reported    27147.50 expected    27247.50 delta   -100.00  VIOLATION 100.00
    -> delta -100.00 is negative; violation is the shortfall, 100.00
```

### 16: recompute one line by hand

[`16_recompute_a_line.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/16_recompute_a_line.py).
The rule for 11(a)(i) is the `Decimal` sum of `contribution_amount` over
`SA11AI` lines (or `SA11A1` in pre-6 filings), skipping memo entries.
The by-hand sum equals `LineCheck.expected` and the count equals
`lines_summed`.

```python
for line in filing.lines_for("SchA"):
    if line.form_type not in {"SA11AI", "SA11A1"}:
        continue
    amount = line.amount("contribution_amount")
    if amount is None:
        continue
    if line.is_memo:
        memo_total += amount
    else:
        by_hand += amount
        counted += 1

check = filing.reconcile().line("A", "11(a)(i)")
assert check.expected == by_hand and check.lines_summed == counted
```

```text
F3A_2011812.fec: F3A
  SA11AI lines: 96 counted, 41 memo (skipped)
  by-hand sum of non-memo contribution_amount: 44776.25
  memo entries would have added:               23642.20
  cover page 11(a)(i) (col_a_individual_contributions_itemized): 44776.25

reconciler: ok   col A line 11(a)(i)   reported        44776.25 expected        44776.25 delta         0.00  = sum of SchA.contribution_amount on SA11AI/SA11A1
  rule:         = sum of SchA.contribution_amount on SA11AI/SA11A1
  expected:     44776.25  (by hand: 44776.25, equal: True)
  lines_summed: 96  (by hand: 96, equal: True)
  reported:     44776.25, delta 0.00, matches True
```

The 41 memo lines are redesignations and reattributions of earlier
contributions. Note the field name: Form 3 calls 11(a)(i)
`col_a_individual_contributions_itemized`, Form 3X and 3P
`col_a_individuals_itemized`. `LineCheck.field` tells you which.

### 17: reconcile a batch

[`17_reconcile_batch.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/17_reconcile_batch.py).
A directory in, one line per report out, then a `Counter` of the
(form, column, line) triples that disagree.

```python
for path in sorted(root.glob("*.fec")):
    try:
        r = hardmoney.parse_file(path, lenient=True).reconcile()
    except hardmoney.UnsupportedForm:
        unsupported += 1
        continue
    for c in r.mismatches():
        disagreeing[(r.form, c.column, c.line)] += 1
```

Over `tests/fixtures/` every report balances. Over a directory of 15
downloaded filings, two do not:

```text
file                         form  checks diff  mismatched lines
2009011.fec                  F3X       69    0
2009202.fec                  F3X       69    0
2010101.fec                  F3P       44    1  A 17(c) (-3400.00)
2011364.fec                  F3X       69    0
2011814.fec                  F3X       69    0
2011852.fec                  F3        50    0
2011863.fec                  F3X       69    0
2011904.fec                  F3X       69    0
2011910.fec                  F3X       69    0
2011912.fec                  F3X       69    1  A 11(c) (+200.00)

15 file(s): 10 reconciled (8 balance), 5 form(s) without cover-page rules, 0 unparseable

lines that disagree most often:
  F3P column A line 17(c)      1 filing(s)
  F3X column A line 11(c)      1 filing(s)
```

## Editing and writing

### 18: fix a field and write

[`18_fix_field_and_write.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/18_fix_field_and_write.py).
A `Line` is a handle into its `Filing`; `set` through any handle is what
`to_fec()` emits. The script trims each too-long field to the maximum
from `field_spec`, re-validates, writes, and re-parses.

```python
for finding in before.errors:
    if finding.rule != "field_too_long":
        continue
    line = line_at(filing, finding.line_no)
    spec = hardmoney.field_spec(line.table, finding.field)
    line.set(finding.field, line[finding.field][: spec["max_len"]].rstrip())

out.write_bytes(filing.to_fec())
again = hardmoney.parse_file(out)
```

```text
field_too_long.fec: 2 error(s) before

line 3 contributor_last_name: 31 -> 30 chars (max 30); now 'YYYYYYYYYYYYYYYYYYYYYYYYYYYYYY'
line 4 payee_organization_name: 201 -> 200 chars (max 200); now 'XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX'

2 field(s) fixed; 0 error(s) after; acceptable: True
wrote 2089 bytes to /tmp/.../field_too_long.fixed.fec
re-parsed: F3XA, 6 line(s), 0 error(s)
  line 3 contributor_last_name = 'YYYYYYYYYYYYYYYYYYYYYYYYYYYYYY'
  line 4 payee_organization_name = 'XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX'
```

### 19: bulk edit every state code

[`19_bulk_edit_states.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/19_bulk_edit_states.py).
Every table has its own state field names (`state`, `contributor_state`,
`payee_state`, `beneficiary_candidate_state`), so the edit goes by
`Line.keys()`. With no arguments the script lower-cases a fixture's
states in memory first, since the real fixtures are already clean.

```python
def upper_case_states(filing: hardmoney.Filing) -> list[tuple[int, str, str, str]]:
    changes = []
    for line in [filing.summary, *filing.iter_lines()]:
        for name in line.keys():
            if name == "state" or name.endswith("_state"):
                value = line[name]
                if value and value != value.upper():
                    line.set(name, value.upper())
                    changes.append((line.line_no, name, value, value.upper()))
    return changes
```

```text
F3XN_2011835.fec with its states lower-cased (demo input): F3XN, 8 body line(s)
validation before: 0 error(s), 1 warning(s)

  line   2 state                'ky' -> 'KY'
  line   3 contributor_state    'ca' -> 'CA'
  line   4 contributor_state    'ky' -> 'KY'
  line   5 contributor_state    'ky' -> 'KY'
  line   6 contributor_state    'ky' -> 'KY'
  line   7 contributor_state    'tx' -> 'TX'
  line   8 payee_state          'ca' -> 'CA'
  line   9 payee_state          'ky' -> 'KY'
  line  10 payee_state          'nj' -> 'NJ'
  line  10 beneficiary_candidate_state 'nj' -> 'NJ'

10 field(s) changed
validation after:  0 error(s), 0 warning(s)
wrote /tmp/.../states-fixed.fec
re-parsed output has 0 lower-case state(s) left
```

### 20: the round-trip check

[`20_round_trip_check.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/20_round_trip_check.py).
The bytes usually differ (canonical form is CRLF, full record width, one
wrapping quote pair and padding removed) while every field agrees, and
writing the re-parsed filing again gives identical bytes.

```python
first = hardmoney.parse(path.read_bytes())
written = first.to_fec()
second = hardmoney.parse(written)
ok = (second.header == first.header
      and second.summary.to_dict() == first.summary.to_dict()
      and [(l.table, l.form_type, l.to_dict()) for l in second.iter_lines()]
          == [(l.table, l.form_type, l.to_dict()) for l in first.iter_lines()]
      and second.to_fec() == written)
```

```text
ok   F1A_2011905.fec              F1A   v8.5      1 line(s)  818 -> 820 bytes
ok   F3A_2011812.fec              F3A   v8.5    587 line(s)  132032 -> 106253 bytes
ok   F3XA_27789_v3.fec            F3XA  v3.0   2837 line(s)  771325 -> 565764 bytes
ok   F3XN_2011831.fec             F3XN  v8.5    144 line(s)  32075 -> 32221 bytes
ok   F99_2011833.fec              F99   v8.5      0 line(s)  673 -> 695 bytes
...
25 of 25 filing(s) round-trip field for field
```

### 21: build a filing from scratch

[`21_build_minimal_filing.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/21_build_minimal_filing.py).
There is no `Filing` constructor; every object comes from the parser.
So build the text. `layout()` gives each field's column, so a record
comes from a dict without any column numbers, and the assembled filing
is parsed, validated, reconciled, and written like any other.

```python
FS = "\x1c"

def record(table: str, values: Mapping[str, str]) -> str:
    layout = hardmoney.layout(table, hardmoney.BUNDLED_SPEC_VERSION)
    cells = [""] * (max(col for _, col in layout) + 1)
    for name, col in layout:
        cells[col] = values.get(name, "")
    return FS.join(cells)

header = record("HDR", {"record_type": "HDR", "ef_type": "FEC", "fec_version": "8.5", ...})
cover = record("F3X", {"form_type": "F3XN", "filer_committee_id_number": "C00123456",
                       "col_a_individuals_itemized": str(itemized), ...})
body = [record("SchA", {"form_type": "SA11AI", "entity_type": "IND", **c}) for c in contributions]
filing = hardmoney.parse("\n".join([header, cover, *body]) + "\n")
```

```text
built F3XN v8.5 with 2 body line(s)
cover 11(a)(i) = 1250.00
validate: 0 error(s), 0 warning(s); acceptable: True
reconcile: balances True; no mismatches
wrote 597 bytes to /tmp/.../minimal-f3x.fec
first SchA record as written:
  SA11AI|C00123456|A1|||IND||Smith|Jane||||1 Main St||Springfield|VA|22150|P2026||20260315|250.00|250.00||Acme|E...
```

One trap worth knowing: `str.splitlines()` treats the FEC field
separator (0x1c) as a line break. Split `.fec` text on `"\r\n"`, or work
in `bytes`, where `splitlines()` only breaks on `\n` and `\r`.

## Spec

### 22: tables, layouts, and a diff of two versions

[`22_spec_tables_and_layouts.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/22_spec_tables_and_layouts.py).

```python
a = dict(hardmoney.layout(table, "7.0"))
b = dict(hardmoney.layout(table, "8.5"))
added = set(b) - set(a)
removed = set(a) - set(b)
moved = [(name, a[name], b[name]) for name in a if name in b and a[name] != b[name]]
```

```text
bundled spec version: 8.5
59 tables: F1 F10 F105 F13 F132 F133 F1M F1S F2 F24 F2S F3 F3L F3P F3P31 F3PS F3PZ1 F3PZ2 F3S F3X F3Z F3Z1 F3Z2 F4 F5 F56 F57 F6 F65 F7 F76 F8 F82 F83 F9 F91 F92 F93 F94 F99 H1 H2 H3 H4 H5 H6 HDR SchA SchA3L SchB SchC SchC1 SchC2 SchD SchE SchF SchI SchL TEXT

SchA layout at 8.5 (45 fields), first 12:
    0 form_type
    1 filer_committee_id_number
    2 transaction_id
    3 back_reference_tran_id_number
    4 back_reference_sched_name
    5 entity_type
    6 contributor_organization_name
    7 contributor_last_name
    8 contributor_first_name
    9 contributor_middle_name
   10 contributor_prefix
   11 contributor_suffix
  ...

SchA: 46 fields in 7.0, 45 in 8.5
  added in 8.5:   none
  removed in 8.5: contribution_purpose_code (col 22)
  moved: 23 field(s), e.g. contribution_purpose_descrip 23->22, contributor_employer 24->23, contributor_occupation 25->24, donor_committee_fec_id 26->25

SchA: 44 fields in 5.3, 45 in 8.5
  added in 8.5:   donor_committee_name (col 26), donor_candidate_last_name (col 28), donor_candidate_first_name (col 29), donor_candidate_middle_name (col 30), donor_candidate_prefix (col 31), donor_candidate_suffix (col 32)
  removed in 8.5: contributor_name (col 3), contribution_purpose_code (col 16), donor_candidate_name (col 20), amended_cd (col 32), increased_limit_code (col 37)
  moved: 37 field(s), e.g. transaction_id 33->2, back_reference_tran_id_number 34->3, back_reference_sched_name 35->4, entity_type 2->5

versions that have no layout raise FecError:
  no column layout for table SchA at spec version 9.9
```

### 23: a data dictionary

[`23_data_dictionary.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/23_data_dictionary.py)
prints a Markdown table for one table from `field_spec`. Redirect it to
a file and you have documentation that matches the parser exactly.

```python
for name, col in hardmoney.layout(table, hardmoney.BUNDLED_SPEC_VERSION):
    spec = hardmoney.field_spec(table, name)
    kind = spec["kind"] + (f"({spec['max_len']})" if spec["max_len"] is not None else "")
    print(f"| {col} | `{name}` | {kind} | {REQUIRED[spec['required']]} | {spec['description']} | ... |")
```

`python examples/23_data_dictionary.py SchB`, selected rows:

```text
# SchB (FEC spec 8.5)

44 fields, in column order. Column numbers are 0-based positions in the `.fec` record.

| col | field | type | required | description | rule / values | sample |
|---|---|---|---|---|---|---|
| 0 | `form_type` | alpha_numeric(8) | yes | FORM TYPE | Appendix C. SB3L must be used with the F3L | SB17 |
| 1 | `filer_committee_id_number` | alpha_numeric(9) | yes | FILER COMMITTEE ID NUMBER |  | C00123456 |
| 2 | `transaction_id_number` | alpha_numeric(20) | yes | TRANSACTION ID NUMBER | must be unique for the life of the report (original + all amendments) | B56123456789-1234 |
| 5 | `entity_type` | alpha_numeric(3) | yes | ENTITY TYPE | [CAN\|CCM\|COM\|IND\|ORG\|PAC\|PTY] | CCM |
| 6 | `payee_organization_name` | alpha_numeric(200) | yes | PAYEE ORGANIZATION NAME | Required if NOT [IND\|CAN] | John Smith & Co. |
| 19 | `expenditure_date` | numeric(8) | recommended | EXPENDITURE DATE |  | 20120720 |
| 20 | `expenditure_amount` | amount(12) | recommended | EXPENDITURE AMOUNT {F3L Bundled} | Expenditure (F3L Bundled Refund) Amt | 1500 |
| 24 | `beneficiary_committee_fec_id` | alpha_numeric(9) |  | BENEFICIARY COMMITTEE FEC ID | pattern `^(?:[PC][0-9]{8}\|[HS][0-9]{1}[A-Z]{2}[0-9]{5})$` | C00654323 |
```

## Interop

The package has no Arrow or pandas dependency. Build frames from lines,
or let the CLI write typed Parquet or SQLite and read that.

### 24: pandas, with Decimals in an object column

[`24_pandas_dataframe.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/24_pandas_dataframe.py).
pandas infers `float64` for a column of Decimals if you let it. Pin the
column to `object` and `sum` and `groupby(...).sum()` return exact
Decimals.

```python
rows = []
for line in filing.lines_for("SchA"):
    row = line.to_dict()
    row["is_memo"] = line.is_memo
    row["amount"] = line.amount("contribution_amount")
    row["date"] = line.date("contribution_date")
    rows.append(row)
df = pd.DataFrame(rows)
df["amount"] = df["amount"].astype(object)      # never float64

receipts = df[~df["is_memo"]]
total = receipts["amount"].sum()                 # Decimal('13736.02')
by_state = receipts.groupby("contributor_state")["amount"].agg(["sum", "count"])
```

```text
F3XN_2011831.fec: DataFrame with 132 rows x 49 columns
amount dtype: object; first value: Decimal('380.00')

 line_no contributor_last_name contributor_employer contributor_state       date amount
       3            Mroczynski          FirstEnergy                OH 2026-08-31 380.00
       4                Fickey          FirstEnergy                OH 2026-08-31 120.00
       5               Rossero          FirstEnergy                WV 2026-08-31 200.00
       6                 Welsh          FirstEnergy                OH 2026-08-31 100.00
       7              Helinski          FirstEnergy                OH 2026-08-31 202.00

sum of non-memo amounts: Decimal('13736.02')
cover page 11(a)(i):     Decimal('13736.02'); equal: True

by state (exact Decimal sums):
                        sum  count
contributor_state
OH                 10278.06     97
PA                  1303.76     16
NJ                   950.00      8
WV                   804.20      9
DC                   300.00      1
MD                   100.00      1

the same column as float64 sums to 13736.02, which is really 13736.02000000000043655745685100555419921875
```

The last line is why not `float64`: the value prints as 13736.02 and is
not 13736.02. Comparisons against the cover page, and against other
filings, stop being exact.

### 25: Parquet through the CLI, read with pyarrow and DuckDB

[`25_parquet_via_cli.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/25_parquet_via_cli.py).
`hardmoney export --format parquet` writes one file per table with
amounts as `decimal128(38, 2)` and dates as `date32`, so Arrow readers
sum money exactly with no conversion.

```python
subprocess.run([cli, "export", str(path), "--format", "parquet", "--out", str(out), "--include-filing-id"],
               check=True, capture_output=True, text=True)
schb = pq.read_table(out / "SchB.parquet")
total = pc.sum(schb["expenditure_amount"]).as_py()       # Decimal('27247.50')
duckdb.sql(f"SELECT SUM(expenditure_amount), typeof(SUM(expenditure_amount)) FROM '{out / 'SchB.parquet'}'")
```

```text
table  rows   bytes
-----  ----  ------
F3X       1  50,177
SchA      5  15,333
SchB      7  14,780
SchE      6  14,552
SchD      5   7,804
TEXT      3   3,206
6 table(s), 27 row(s), 105,852 bytes -> /tmp/.../parquet/

SchB.parquet: 7 rows; column types:
  filing_id                  int64
  line_no                    int64
  payee_organization_name    string
  expenditure_date           date32[day]
  expenditure_amount         decimal128(38, 2)

pyarrow sum of expenditure_amount: Decimal('27247.50') (Decimal)

DuckDB, top 3 disbursements:
┌─────────────────────────┬────────────┬───────────────┐
│          payee          │    date    │    amount     │
│         varchar         │    date    │ decimal(38,2) │
├─────────────────────────┼────────────┼───────────────┤
│ The Media Company LLC   │ 2026-06-02 │      15000.00 │
│ Bedford Grove LLC       │ 2026-06-01 │       7500.00 │
│ The Political Law Group │ 2026-05-26 │       3697.50 │
└─────────────────────────┴────────────┴───────────────┘

DuckDB SUM: Decimal('27247.50') as DECIMAL(38,2)
```

### 26: polars with a Decimal column

[`26_polars_dataframe.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/26_polars_dataframe.py).
polars has a real decimal type; a column of Python Decimals becomes
`Decimal(precision=38, scale=2)` and aggregations stay exact.

```python
df = pl.DataFrame(
    {"form_type": [l.form_type for l in lines],
     "employer": [l["contributor_employer"] for l in lines],
     "date": [l.date("contribution_date") for l in lines],
     "amount": [l.amount("contribution_amount") for l in lines],
     "is_memo": [l.is_memo for l in lines]},
    schema_overrides={"amount": pl.Decimal(scale=2), "date": pl.Date},
)
receipts = df.filter(~pl.col("is_memo") & (pl.col("form_type") == "SA11AI"))
receipts.group_by("employer").agg(pl.col("amount").sum().alias("total"), pl.len().alias("n"))
```

```text
F3A_2011812.fec: 249 rows; schema:
  line_no    Int64
  form_type  String
  last_name  String
  employer   String
  state      String
  date       Date
  amount     Decimal(precision=38, scale=2)
  is_memo    Boolean

sum of non-memo SA11AI amounts: 44776.25 (Decimal); cover 11(a)(i): 44776.25; equal: True

top employers by exact total:
shape: (5, 3)
┌──────────────────┬───────────────┬─────┐
│ employer         ┆ total         ┆ n   │
│ ---              ┆ ---           ┆ --- │
│ str              ┆ decimal[38,2] ┆ u32 │
╞══════════════════╪═══════════════╪═════╡
│ RETIRED          ┆ 19445.37      ┆ 73  │
│ NONE             ┆ 7000.00       ┆ 1   │
│ 535 PLUMBING LLC ┆ 7000.00       ┆ 1   │
│ SELF             ┆ 3670.51       ┆ 5   │
│ SAIC             ┆ 1979.00       ┆ 1   │
└──────────────────┴───────────────┴─────┘
```

### 27: SQLite through the CLI, exact sums in sqlite3

[`27_sqlite_via_cli.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/27_sqlite_via_cli.py).
The SQLite export stores every field as `TEXT` exactly as filed. The
standard `sqlite3` module sums those exactly with a Python aggregate, and
`register_adapter(Decimal, str)` lets Decimals be query parameters.

```python
class DecimalSum:
    def __init__(self) -> None:
        self.total = Decimal("0")
    def step(self, value: Optional[str]) -> None:
        if value:
            self.total += Decimal(value)
    def finalize(self) -> str:
        return str(self.total)

sqlite3.register_adapter(Decimal, str)
conn = sqlite3.connect(db)
conn.create_aggregate("decimal_sum", 1, DecimalSum)
exact = Decimal(conn.execute("SELECT decimal_sum(expenditure_amount) FROM SchB").fetchone()[0])
```

```text
table  rows
-----  ----
F3X       1
SchA      5
SchB      7
SchE      6
SchD      5
TEXT      3
6 table(s), 27 row(s), 45,056 bytes -> /tmp/.../F3XA_2011821.sqlite

tables: F3X, SchA, SchB, SchD, SchE, TEXT, filings
filings: (2011821, 'F3XA', '8.5', 'C00922229')

largest disbursements (amounts are TEXT as filed):
  (13, 'The Media Company LLC', '20260602', '15000.00')
  (10, 'Bedford Grove LLC', '20260601', '7500.00')
  (9, 'The Political Law Group', '20260526', '3697.50')

exact total via decimal_sum: Decimal('27247.50')
SUM(CAST(... AS REAL)):      27247.5 (a float; fine for a glance, not for the books)
disbursements of at least 1000.00: 4
```

### 28: JSON with default=str

[`28_json_dump.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/28_json_dump.py).
`to_dict()` is all strings and serialises as is. For the typed values,
`default=str` makes `json` call `str()` on anything it cannot encode:
`Decimal("380.00")` becomes `"380.00"`, exact, and a `date` becomes ISO
8601. `json.loads` gives the string back and `Decimal(s)` restores it.

```python
doc = {
    "form_type": filing.form_type,
    "header": filing.header,
    "cover": line_record(filing.summary),
    "lines": [line_record(l) for l in filing.iter_lines()],   # to_dict() plus Decimal/date values
}
text = json.dumps(doc, indent=2, default=str)
```

```text
{
  "form_type": "F3XA",
  "version": "8.5",
  "is_amendment": true,
  "amends_filing": 1991972
}
{
  "line_no": 3,
  "table": "SchA",
  "typed": {
    "contribution_amount": "20000.00",
    "contribution_aggregate": "20000.00",
    "contribution_date": "2026-05-27"
  },
  "fields (3 of 45)": {
    "entity_type": "CAN",
    "contributor_organization_name": "",
    "contributor_last_name": "SMITH"
  }
}
... 18063 characters for 6 body line(s)

first typed amount after json.loads: '20000.00' -> Decimal('20000.00')
```

## FEC data online

These need the network; the test suite runs them only with
`HARDMONEY_NETWORK_TESTS=1`. Examples 30 and 31 use the CLI for
discovery (it handles the API, paging, the RSS feed, and the download
cache) and Python for what to do with each filing.

### 29: fetch a filing by id

[`29_fetch_filing.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/29_fetch_filing.py).

```python
try:
    filing = hardmoney.fetch(filing_id)      # docquery.fec.gov/dcdev/posted/<id>.fec
except hardmoney.FecError as e:
    print(f"could not fetch filing {filing_id}: {e}")
    return 1
```

```text
filing 2011831: F3XN v8.5, 144 body line(s)
  committee: FirstEnergy Corp Political Action Committee (C00140855)
  coverage:  2026-08-01 to 2026-08-31
  receipts:  15245.52; disbursements: 10023.01
  validate:  0 error(s), 0 warning(s)
  reconcile: balances
```

### 30: search a committee's filings and validate each

[`30_filings_search_and_validate.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/30_filings_search_and_validate.py).
Needs an openFEC key in `FEC_API_KEY` or `~/fec_api_key.txt`.
`--json` gives openFEC's records; `--fetch DIR` downloads each `.fec`
and adds `actions.saved`.

```python
cmd = [cli, "filings", "--committee", committee, "--form-type", "F3X", "--most-recent",
       "--limit", str(limit), "--json", "--fetch", str(out_dir)]
records = json.loads(subprocess.run(cmd, check=True, capture_output=True, text=True).stdout)
for rec in records:
    filing = hardmoney.parse_file(rec["actions"]["saved"], lenient=True)
    v, r = filing.validate(), filing.reconcile()
```

```text
3 filing(s) for C00140855

file_number report coverage                    receipts  errors warnings  reconcile
    2011831 M9     2026-08-01 to 2026-08-31     15245.52       0        0  balances
    2006786 M8     2026-07-01 to 2026-07-31     22932.28       0        0  balances
    1995180 M7     2026-06-01 to 2026-06-30     15761.52       0        0  balances
```

### 31: poll the e-file feed

[`31_efile_watch.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/31_efile_watch.py).
`efile watch --once --json` prints one JSON object per filing not seen
before and records the ids under the cache directory, so a cron job that
runs this gets only what is new. No API key. `HARDMONEY_CACHE_DIR`
(the CLI's own variable) picks the cache; point it somewhere disposable
to try it.

```python
cmd = [cli, "efile", "watch", "--once", "--json", "--form-type", form_type]
for line in subprocess.run(cmd, check=True, capture_output=True, text=True).stdout.splitlines():
    entry = json.loads(line)
    filing = hardmoney.fetch(int(entry["filing_id"]))
    v, r = filing.validate(), filing.reconcile()
```

```text
437 new F3X filing(s) in the feed (seen-list in /tmp/hm-demo)

2010983 F3XN  C00429787 SEPTEMBER MONTHLY    UNITED FOR PROGRESS LEADERSHIP COMMITTEE
    0 line(s), 0 error(s), 0 warning(s), balances
2010984 F3XN  C00002469 SEPTEMBER MONTHLY    MACHINISTS NON PARTISAN POLITICAL LEAGUE
    1038 line(s), 0 error(s), 0 warning(s), balances
2010985 F3XN  C00368142 SEPTEMBER MONTHLY    OFFICE OF THE COMMISSIONER OF MAJOR LEAG
    22 line(s), 0 error(s), 0 warning(s), balances
... 434 more not processed (raise the limit)
```

## Integration patterns

### 32: a pre-submission check for a web application

[`32_presubmission_check.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/32_presubmission_check.py).
One pure function from `bytes` to a JSON-ready dict: validation errors
and warnings, plus the cover-page lines that disagree with the schedules
for F3X/F3/F3P. Amounts are strings in the result so they stay exact
through `JsonResponse`. No Django import; the view is three lines.

```python
def check_fec_bytes(data: bytes) -> dict[str, Any]:
    try:
        filing = hardmoney.parse(data, lenient=True)
    except hardmoney.FecError as e:
        return {"parsed": False, "acceptable": False, "error": str(e), "line": e.line_no, ...}
    v = filing.validate()
    report = {"parsed": True, "form_type": filing.form_type, "acceptable": v.is_acceptable,
              "errors": [_finding(f) for f in v.errors], "warnings": [_finding(f) for f in v.warnings],
              "reconciliation": None}
    try:
        r = filing.reconcile()
        report["reconciliation"] = {"form": r.form, "balances": r.balances,
                                    "mismatches": [_mismatch(c) for c in r.mismatches()]}
    except hardmoney.UnsupportedForm:
        pass
    return report

# views.py
def precheck(request):
    report = check_fec_bytes(request.FILES["filing"].read())
    return JsonResponse(report, status=200 if report["acceptable"] else 422)
```

```text
{
  "parsed": true,
  "form_type": "F3XA",
  "version": "8.5",
  "committee_id": "C00944124",
  "body_lines": 6,
  "skipped_lines": 0,
  "acceptable": false,
  "errors": [
    {
      "rule": "not_a_real_date",
      "line": 2,
      "record": "F3XA",
      "field": "date_signed",
      "message": "20261301 is not a Real Date"
    },
    ...
  ],
  "warnings": [],
  "reconciliation": {
    "form": "F3X",
    "checks": 69,
    "balances": true,
    "mismatches": []
  }
}
```

### 33: an HTTP endpoint with http.server

[`33_http_validation_server.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/33_http_validation_server.py).
`POST /validate` with the raw `.fec` as the body; 200 when acceptable,
422 when not, 400 when unparseable, JSON either way. `--serve PORT`
runs it; without that the script starts the server on a free port,
POSTs a file to itself with `urllib`, and prints the response.

```python
class Handler(BaseHTTPRequestHandler):
    def do_POST(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        status, body = validate_bytes(self.rfile.read(length))
        payload = json.dumps(body).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(payload)
```

```text
POST duplicate_tran_id.fec -> http://127.0.0.1:56130/validate
HTTP 422
{
  "ok": false,
  "form_type": "F3XA",
  "version": "8.5",
  "lines": 6,
  "findings": [
    {
      "severity": "error",
      "rule": "duplicate_transaction_id",
      "line": 5,
      "record": "SB21B",
      "field": "transaction_id_number",
      "message": "Tran ID SB21B.4120 is NOT UNIQUE - This one is same as other(s) (first used on line 4)"
    }
  ],
  "reconciliation": {
    "balances": true,
    "mismatches": []
  }
}
```

### 34: a pytest fixture for generated filings

[`34_pytest_fixture_pattern.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/34_pytest_fixture_pattern.py).
For software that produces `.fec` files: two assertions with useful
failure messages, a parametrised fixture over the output directory
(`FEC_OUTPUT_DIR`), and two tests. Copy them into your `conftest.py`.

```python
def assert_validates_clean(filing: hardmoney.Filing, *, warnings_ok: bool = True) -> None:
    v = filing.validate()
    bad = v.errors if warnings_ok else v.findings
    assert not bad, f"{len(bad)} finding(s):\n" + "\n".join(str(f) for f in bad)

def assert_cover_page_balances(filing: hardmoney.Filing) -> None:
    try:
        r = filing.reconcile()
    except hardmoney.UnsupportedForm:
        return
    assert r.balances, "\n".join(str(c) for c in r.mismatches())

@pytest.fixture(params=FEC_FILES, ids=lambda p: p.name)
def generated_filing(request: pytest.FixtureRequest) -> hardmoney.Filing:
    return hardmoney.parse_file(request.param)

def test_generated_filing_is_acceptable(generated_filing: hardmoney.Filing) -> None:
    assert_validates_clean(generated_filing)
```

```text
$ pytest -q examples/34_pytest_fixture_pattern.py
..................................................                       [100%]
50 passed in 0.06s
```

### 35: compare an original with its amendment

[`35_compare_amendment.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/35_compare_amendment.py).
Cover pages are compared by field name; schedules are matched on
transaction id, which the FEC requires to be stable across amendments.
Schedule A calls it `transaction_id`, the other schedules
`transaction_id_number`. The repository has no real pair, so the script
derives an amendment from a filing (one line removed, one added, one
amount changed by $5.00 with the cover adjusted) and diffs the two.

```python
before = {transaction_id(l): l for l in original.iter_lines() if transaction_id(l)}
after = {transaction_id(l): l for l in amendment.iter_lines() if transaction_id(l)}
removed = sorted(set(before) - set(after))
added = sorted(set(after) - set(before))
modified = [t for t in sorted(set(before) & set(after)) if before[t].to_dict() != after[t].to_dict()]

a, b = original.summary, amendment.summary
changed = [(k, a[k], b[k]) for k in a.keys() if k in b and a[k] != b[k]]
```

```text
(amendment derived from F3XN_2011831.fec for demonstration)

original:  F3XN v8.5, 144 line(s), report id ''
amendment: F3XA v8.5, 144 line(s), report id 'FEC-2011831', is_amendment=True, amends_filing=2011831

cover page fields that changed:
  form_type                                'F3XN' -> 'F3XA'
  col_a_total_receipts                     '15245.52' -> '15250.52'  (delta +5.00)
  col_a_subtotal                           '1995573.45' -> '1995578.45'  (delta +5.00)
  col_a_cash_on_hand_close_of_period       '1985550.44' -> '1985555.44'  (delta +5.00)
  col_a_individuals_itemized               '13736.02' -> '13741.02'  (delta +5.00)
  col_a_individual_contribution_total      '15245.52' -> '15250.52'  (delta +5.00)
  col_a_total_contributions                '15245.52' -> '15250.52'  (delta +5.00)
  col_a_total_receipts_recap               '15245.52' -> '15250.52'  (delta +5.00)
  col_a_total_federal_receipts             '15245.52' -> '15250.52'  (delta +5.00)
  col_a_total_contributions_recap          '15245.52' -> '15250.52'  (delta +5.00)
  col_a_net_contributions                  '15245.52' -> '15250.52'  (delta +5.00)

transactions: 144 before, 144 after; 1 removed, 1 added, 1 changed
  - PR12339386106229     SA11AI Mroczynski 380.00
  + NEW0001              SA11AI Newdonor 380.00
  ~ PR13165176106229     contribution_amount: '200.00' -> '205.00'

amendment reconciles: True
```

## For FEC developers

FECfile+ does two things with a report before it goes to the Commission:
it runs the validator so the filer sees failing and warning messages
before upload, and it computes the summary page from the transactions
the filer entered. Both map onto one method call each here, on the same
`Filing` object, from the same bytes the filer would upload.

Validate before submit is `Filing.validate()`. It runs the acceptance
rules that can be evaluated from the filing alone (structure, IDs, field
types and lengths, the character set, dates, amounts, code lists,
transaction-id integrity), driven by the FEC's own spec workbook rather
than hard-coded lengths. Each `Finding` carries the stable `rule` name,
the `line_no` and `field`, and a message worded after the FEC's, so it
can be shown to a filer or joined to `field_spec()` for the rule text
(example 13). `is_acceptable` is the yes/no the upload would get.
Example 32 is the whole thing as a function a view can call; example 12
is the same check as a CI step for software that generates filings;
example 34 is it as a pytest fixture.

Compute the summary page is `Filing.reconcile()`, run in reverse: it
recomputes every Column A line from the schedules (memo entries
excluded, floors for the lines the $200 threshold applies to) and every
formula line from the other cover lines, and reports `reported`,
`expected`, and `delta` per line as exact Decimals. `r.line("A",
"11(a)(i)").expected` is what the summary page should say; examples 14
through 16 show the rules, and `LineCheck.rule` prints each one in the
FEC's notation. Where the FEC's validator only warns that a subtotal is
"not supported by Schedule", this says by how much and which line.

Both are the parser-only Rust crate behind a stable-ABI wheel with no
runtime dependencies, so they drop into a Django service without a
database, a server, or a float anywhere in the path.
