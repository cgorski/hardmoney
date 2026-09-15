# For FEC staff

hardmoney is an open-source implementation of the `.fec` electronic filing
format the Commission publishes: a parser for every format version from
3.x (2001) through 8.5, a writer, the acceptance rules the FEC's validator
applies, and a reconciler that recomputes each cover-page line from the
schedules with exact decimal arithmetic. Those last two are the checks a
Reports Analysis Division analyst makes first on every report. Money is
never a float. It installs with `pip install hardmoney` (Python 3.9 or
newer, no dependencies) or `cargo install hardmoney`; there is no server,
no account, and no database unless you want one. It is not a filing
system: it does not submit to the Electronic Filing Office (the webload
service takes a vendor key and the treasurer's password; hardmoney holds
neither) and does not replace FECfile+ or a vendor's product. It checks
what those produce.

The chapter is arranged by who at the Commission would use it, Python
first and the CLI equivalent after. The scripts live in
[`python/examples/fec/`](https://github.com/cgorski/hardmoney/tree/main/python/examples/fec)
and are run by `python/tests/test_examples_fec.py`. Output was captured on
2026-09-15; the only edits are shortened paths and listings cut with
`...`. The [Python cookbook](./python-cookbook.md) and
[API reference](./python-api.md) cover the package itself.

## Reports Analysis Division: first-pass review of a report

The Inspector General describes RAD's review as confirming that "all forms
and applicable schedules are complete ... the calculations included are
correct, and no contribution limits have been exceeded", over 196,175
documents in the 2024 cycle with flat headcount
([FY 2026 Management Challenges](https://www.fec.gov/documents/6045/FY-2026-Mgmt-Challenges.pdf),
pp. 13-14). The FEC's validator treats the arithmetic as a warning only
("Subtotal ... not supported by Schedule", messages 3-4 on
[Validation errors explained](https://www.fec.gov/help-candidates-and-committees/filing-reports/validation-errors-explained/)),
so a report whose cover disagrees with its schedules is accepted as filed.

[`fec_01_first_pass_review.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/fec/fec_01_first_pass_review.py)
takes a path or a filing id and asks the opening questions in order:

```python
v = filing.validate()
blank = [f for f in v.findings if f.rule in BLANK]   # required_field_empty, conditionally_..., recommended_...
ids = [f for f in v.findings if f.rule in IDS]       # duplicate_transaction_id, back_reference_not_found
for c in filing.reconcile().mismatches():
    print(c.column, c.line, c.reported, c.expected, c.delta, c.rule)
    key = c.line.replace("(", "").replace(")", "").upper()          # 11(c) -> 11C, as in SA11C
    lines = [l for l in filing.iter_lines() if l.form_type[2:] == key]
    memos = sum((amount(l) for l in lines if l.is_memo), Decimal(0))  # == |delta|? memos were counted
```

The default input is real: the Republican Party of Minnesota's May 2026
monthly as amended on 2026-09-14 and accepted by the FEC
(`tests/fixtures/rad/F3XA_2011912.fec`, byte-identical to docquery).

```text
$ python examples/fec/fec_01_first_pass_review.py
F3XA v8.5: REPUBLICAN PARTY OF MINNESOTA - FEDERAL (C00001313)
report M6, 20260501 to 20260531; amends 1986128, amendment 1
software: VORTEXFILER A RED CURVE SOLUTION 3; 1232 body line(s), 0 skipped

acceptance rules: 0 error(s), 0 warning(s); the FEC would accept this file
fields the FEC asks for that are blank: 0
transaction id problems: 0

cover page vs schedules (F3X, 69 checks): 1 line(s) disagree
  col A line 11(c)      reported      2045.00 expected      1845.00 delta +200.00  = sum of SchA.contribution_amount on SA11C
      5 schedule line(s) carry this line number; memo entries among them total 0
      the cover page carries 200.00 that the other side does not
```

One $200 committee contribution is on the cover and not on Schedule A;
every formula that consumes 11(c) holds, so the gap is between cover and
schedule, not inside the cover. On the Georgia Republican Party's April
monthly (`F3XA_2011814.fec`) the same script lists the four
conditionally-required blanks WebCheck also flags, over a cover that
balances; on `invalid/duplicate_tran_id.fec` it says `the FEC would
REJECT this file`. The CLI equivalent is two commands, each taking a file
or a filing id, with `--json` for the same fields:

```text
$ hardmoney reconcile tests/fixtures/rad/F3XA_2011912.fec
DIFF col A line 11(c)      reported         2045.00 expected         1845.00 delta       200.00  = sum of SchA.contribution_amount on SA11C
F3X C00001313: 1 of 69 line(s) disagree (tolerance 0)
$ hardmoney validate tests/fixtures/rad/F3XA_2011912.fec
ACCEPTABLE: F3XA, 1232 body line(s), 0 error(s), 0 warning(s)
```

### An amendment against its original

`hardmoney filings` queries openFEC and, with `--json`, returns its
`amendment_chain`, `amendment_version`, and `most_recent`; `--fetch DIR`
downloads each raw `.fec`:

```text
$ hardmoney filings --committee C00922229 --form-type F3X --report-type Q2 --cycle 2026 --json --fetch /tmp/af
[ { "file_number": 2011821, "amendment_indicator": "A", "amendment_chain": [1996410, 2011821], "amendment_version": 1,
    "most_recent": true, "previous_file_number": 1996410, "actions": {"saved": "/tmp/af/2011821.fec"}, ... },
  { "file_number": 1996410, "amendment_indicator": "N", "amendment_chain": [1996410], "amendment_version": 0,
    "most_recent": false, "actions": {"saved": "/tmp/af/1996410.fec"}, ... } ]
```

Both files are fixtures (`F3XN_1996410.fec`, `F3XA_2011821.fec`).
[`fec_02_amendment_diff.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/fec/fec_02_amendment_diff.py)
compares cover pages field by field and schedules by `transaction_id`,
which the FEC requires to be stable across amendments; given one filing
id it fetches the amendment, reads `amends_filing` from its header, and
fetches the original.

```text
$ python examples/fec/fec_02_amendment_diff.py
original:  F3XN Q2 20260514..20260630, 24 line(s), signed 20260715, balances
amendment: F3XA, amendment 1 of filing 1996410, 26 line(s), signed 20260914, balances

cover-page lines that changed:
  col_a_debts_by                                   21177.00 ->       36177.50  (+15000.50)
other cover fields that changed: form_type, date_signed

transactions: 24 -> 26; 2 added, 0 removed, 8 edited
  + PD138              SD10    David Binder Research            15000.00
  + TEXT-F3XA          TEXT
  ~ BE10               date_signed: '20260715' -> '20260914'
  ...
  ~ PD76               incurred_amount_this_period: '5717.00' -> '5717.50'; balance_at_close_this_period: '5717.00' -> '5717.50'
```

A $15,000 debt added, another corrected by fifty cents, six Schedule E
lines re-signed. A related observation: amendment 2011912 above was
received at 19:50 ET on 2026-09-14 and was in openFEC's `/efile/filings/`
six seconds later, but on 2026-09-15 `/filings/` still listed only the
original 1986128 as `most_recent`. The same committee's three amendments
received between 18:56 and 19:11 that evening were listed; the three
received after 19:50 were not, consistent with
[openFEC#3800](https://github.com/fecgov/openFEC/issues/3800) (filings
after 7:30 pm ET miss the nightly refresh; open since 2019). The raw file
was on docquery throughout, which is what `hardmoney.fetch` reads.

## Electronic Filing Office: vendor conformance

The EFO certifies filing software against
[WebCheck](https://efoservices.fec.gov/webcheck/); 85 products are listed
as [active vendors](https://efilingapps.fec.gov/registration/softwarelogs.htm),
and the 49 failing and 31 warning messages exist only as prose. Each
`Finding` here has a stable `rule` name and a message worded after the
FEC's, and the `HDR` record names the software that wrote the file, so
[`fec_03_vendor_conformance.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/fec/fec_03_vendor_conformance.py)
can group a batch by `soft_name soft_ver` and count by rule. With
`--oracle` it sends each rejected file to WebCheck through the CLI and
appends the diff counts:

```text
$ python examples/fec/fec_03_vendor_conformance.py --oracle invalid/duplicate_tran_id.fec invalid/field_too_long.fec invalid/multi_form.fec F3XA_2011814.fec
4 file(s) from 2 software product(s)

software                                     files accepted errors warnings
FECfile 8.5.1.0(f34)                             3        0      5        0
Campaign Manager 360 1.0                         1        1      0        4
...
files the FEC would reject:
  duplicate_tran_id.fec              FECfile 8.5.1.0(f34)         1 error(s); webcheck 1 matched, 0 only ours, 0 only theirs
      line 5 SB21B transaction_id: Tran ID SB21B.4120 is NOT UNIQUE - This one is same as other(s) (first used on line 4)
  field_too_long.fec                 FECfile 8.5.1.0(f34)         2 error(s); webcheck 2 matched, 0 only ours, 0 only theirs
      ...
  multi_form.fec                     FECfile 8.5.1.0(f34)         2 error(s); webcheck 1 matched, 1 only ours, 2 only theirs
      line 9 F3XN form_type: Multi-Form Filings are NOT Allowed (F3XN record in the body of a F3XA filing)
      line 9 F3XN treasurer_last_name: TREASURER LAST NAME is Required, but field is Empty
```

`multi_form.fec` is a real disagreement, written up in
[Validating a filing](./validating.md#what-the-fecs-validator-says-about-our-fixtures):
both reject the file, but WebCheck attributes the second cover record to
the F3XA line and adds `Schedule does not belong with Form`. The CLI
shows the pairing finding by finding; this run was live against
`efoservices.fec.gov` (recorded responses are the test data in
`tests/webcheck_oracle.rs`).

```text
$ hardmoney validate tests/fixtures/invalid/dangling_back_reference.fec --oracle webcheck
ERROR line 5 SA11AI back_reference_tran_id: Back-Reference TRAN-ID SA11AI.9999 does not match Sched TRAN-ID (no transaction in this filing has that ID)
NOT ACCEPTABLE: F3XA, 8 body line(s), 1 error(s), 0 warning(s)
...
WebCheck NOT ACCEPTABLE (ERRORS): 1 error(s), 0 warning(s), filing type F3XA

--- Diff: hardmoney vs. WebCheck ---
Matched (1):
  ERROR line 5 SA11AI back_reference_tran_id: Back-Reference TRAN-ID SA11AI.9999 does not match Sched TRAN-ID (no transaction in this filing has that ID)
    ~ ERROR SA11AI #004 Transaction ID of Schedule referenced by this record {SMITH, DAVID BROCK}: No Match Found for Back-Reference to Schedule/TranID - SA11AI.9999
1 matched, 0 only ours, 0 only theirs
```

This is WebCheck's credential-free upload channel; `--webcheck-api-key`
switches to the vendor SOAP service, untested past the key check because
nobody on the project holds a key. `--strict-oracle` exits 1 on any
disagreement, for a vendor's CI. The file leaves the machine only with
`--oracle`.

### When the format version changes

The FEC publishes the format as a 53-sheet workbook
([`FEC_EFO_Format_Specifications.xlsx`](https://docquery.fec.gov/formatspecs/FEC_EFO_Format_Specifications.xlsx),
v8.5 dated 2025-01-28, last modified 2025-09-02). hardmoney carries the
column map of every table at every version and diffs two versions:

```text
$ hardmoney spec diff 8.4 8.5
8.4 -> 8.5
F1S: +joint_fund_participant_committee_type@5, affiliated_committee_id_number 5->6, affiliated_committee_name 6->7, ...
F99: +filing_frequency@16, +pdf_attachment@17, text 16->18
SchC2: +entity_type@5, +guarantor_employer_code@6, +guarantor_occupation_code@7, guarantor_last_name 5->8, ...
F3PZ1: lost its layout (44 fields at 8.4; none at 8.5)
...
unchanged (46): F1, F13, F132, F133, F1M, F2, F24, F2S, F3, F3L, F3P, F3P31, F3PS, F3S, F3X, F3Z, F4, F5, ...
```

`hardmoney spec export` writes the whole specification as JSON (1.7 MB:
every table and version, column positions, and for 8.5 each field's type,
length, requirement, and rule text). The per-field rules the validator
runs (`data/fec-spec/spec-8.5.json`) are distilled from that workbook and
from the JSON Schemas in
[fecgov/fecfile-validate](https://github.com/fecgov/fecfile-validate); a
weekly job (`.github/workflows/spec-drift.yml`, Mondays) re-runs the
distillation against that repository's `develop` branch and fails when
the checked-in JSON differs. The FEC's own spreadsheet-to-schema checker
"has not been run in a long time"
([fecfile-validate#302](https://github.com/fecgov/fecfile-validate/issues/302)).

## FECfile+ developers

FECfile+ (`fecgov/fecfile-web-api`) is Django on Python 3.13:
`fecfile_validate` (a Draft-7 JSON Schema wrapper, pinned by commit hash)
runs per record in a serializer mixin; `reports/form_3x/summary.py`
computes the cover page as a Celery task (`web_services/summary/tasks.py`);
`web_services/dot_fec/` serializes the `.fec` from the schemas' `COL_SEQ`.
There is no whole-file validation before submit; users are told to run
the downloaded `.fec` through WebCheck
([fecfile-web-app#2448](https://github.com/fecgov/fecfile-web-app/issues/2448)).
`summary.py` hard-codes lines 18(c), 21(a)(i), 21(a)(ii), 30(a)(i), and
30(a)(ii) to `Decimal(0)` ("Stubbed out until a future ticket");
`form_3/summary.py` sets every Form 3 line to zero.

| FECfile+ | hardmoney | Difference |
|---|---|---|
| `fecfile_validate.validate(schema, record)` per record | `Filing.validate()` over the file | adds what a schema cannot express: HDR first, one cover, unique transaction ids, back-references, filer id on every line, character set, version |
| `calculate_summary_columns()` from the transaction table | `Filing.reconcile()` from the filed `.fec` | Column A schedule sums for F3X, F3, F3P including H3-H6; formulas over the reported lines; Column B formulas only |
| `dot_fec_serializer.py` (8.5, 15 record types) | `Filing.to_fec()` (every table, every version) | round-trip tested over every fixture and by property tests |

[`fec_04_fecfile_plus_hook.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/fec/fec_04_fecfile_plus_hook.py)
is one function over the bytes `compose_dot_fec()` produces, returning the
validation in `ValidationResult`'s shape, the Column A schedule sums keyed
as `summary.py` keys them, and any cover line that disagrees:

```python
def check_dot_fec(data: bytes) -> dict[str, Any]:
    filing = hardmoney.parse(data, lenient=True)          # FecError -> {"errors": [...], "warnings": []}
    v = filing.validate()
    out = {"errors": [{"path": f"{f.form_type}.{f.field}", "message": f.message} for f in v.errors],
           "warnings": [...], "column_a": {}, "cover_disagrees": []}
    r = filing.reconcile()                                # UnsupportedForm -> return out
    for c in r.checks:
        if c.column == "A" and "sum of" in c.rule:        # the get_line("SA11AI", ...) sums, not the arithmetic
            out["column_a"]["line_" + c.line.replace("(", "").replace(")", "")] = str(c.expected)
    out["cover_disagrees"] = [{"line": c.line, "reported": str(c.reported), "expected": str(c.expected)} for c in r.mismatches()]
    return out
```

On the Georgia Republican Party's report (a filer with a nonfederal
account) the stubbed lines are the ones carrying money:

```text
$ python examples/fec/fec_04_fecfile_plus_hook.py
{ "form_type": "F3XA", "fec_version": "8.5", "errors": [],
  "warnings": [ {"path": "SA11B.donor_committee_fec_id", "message": "Conditionally Required field is Empty: DONOR COMMITTEE FEC ID (Conditional Warning)"}, ... ],
  "column_a": { "line_9": "0", "line_10": "13825.21", "line_11ai": "47147.22", "line_11b": "2500.00", "line_11c": "750.00", "line_12": "7500.00",
    ..., "line_18a": "78207.70", "line_18b": "0", "line_21ai": "28833.61", "line_21aii": "54233.29", "line_21b": "0",
    ..., "line_28a": "200.00", "line_30ai": "0", "line_30aii": "0", "line_30b": "0" },
  "cover_disagrees": [] }
```

Parity. The Form 3X rule table was checked against FECfile+'s own
`test_calculate_summary_column_a` (transaction set in
`web_services/summary/tests/utils.py`, public domain): the same
transactions, filed as a `.fec`, reconcile to the same value on every
Column A line FECfile+ computes, memo exclusions included
(`oracle_fecfile_plus_f3x_column_a` in `src/parser/reconcile.rs`). The
H3-H6 lines FECfile+ stubs were written from the spec's rule text and
checked against 45 accepted state-party reports; the module docs record
what the data settled (18(a) sums H3 `transferred_amount`, not
`total_amount_transferred`; H4 memo entries are excluded; 30(b) is a
floor). Two differences: FECfile+ holds every transaction and sums
sub-$200 items into lines like 21(b) and 24, while a filed `.fec` need
not itemize them, so hardmoney checks those lines as floors; and
11(a)(ii) is an input here, where FECfile+ stores it as internal
`SA11AII` transactions that never appear in a filing.

The FECfile+ repositories contain no `.fec` golden files; the serializer
tests split rows on the separator and assert columns.
[`fec_05_golden_fixtures.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/fec/fec_05_golden_fixtures.py)
builds a spec-8.5 Form 3X from `(form type, amount, memo)` tuples with
`hardmoney.layout()`, fills the cover page by fixed point, and writes it
with `to_fec()`:

```python
for _ in range(12):  # 11ai -> 11aiii -> 11d -> 19 -> ... -> 8: one layer of formulas per pass
    pending = [c for c in filing.reconcile().checks if c.reported is None or not c.matches]
    if not pending:
        break
    for c in pending:
        filing.summary.set(c.field, str(c.expected))
(out / "f3x_fecfile_plus_test_set.fec").write_bytes(filing.to_fec())
```

```text
$ python examples/fec/fec_05_golden_fixtures.py /tmp/golden
wrote /tmp/golden/f3x_fecfile_plus_test_set.fec: F3XN v8.5, 27 body line(s)
re-parsed: 0 error(s), 0 warning(s); reconcile balances: True (69 checks)
  Column A 11(a)(i)    10000.23
  Column A 15           2125.79
  Column A 17           1000.00
  Column A 21(b)         150.00
  Column A 24            151.00
  ...
```

The default transactions are the FECfile+ test set, so the `.json`
written beside the `.fec` is what that calculator should produce (15 =
2125.79, 17 = 1000.00, 24 = 151.00 with the memo SE excluded). The
writer's output is canonical (CRLF, ASCII 28, full record width,
Windows-1252 when representable) for every table and version, so it is
also a reference for what `dot_fec_serializer.py` should emit for F3P,
which FECfile+ does not yet support
([fecfile-web-api#1523](https://github.com/fecgov/fecfile-web-api/issues/1523));
[Writing `.fec` files](./writing-fec.md) lists what is preserved.

Limits: `reconcile()` has rule tables for F3X, F3, and F3P only (other
forms raise `UnsupportedForm`); Column B is checked by formula only; the
validator cannot see leading blanks (the parser trims them) and reports a
superseded format version as a warning where the FEC rejects it. Cookbook
example 32 is the Django view, example 34 the pytest fixture.

## Data division and researchers inside the agency

The FEC publishes `pg_dump` archives of four processed tables under
[`bulk-downloads/data-dump/schedules/`](https://www.fec.gov/files/bulk-downloads/data-dump/schedules/),
refreshed each weekend (Schedule A 90 GB, Schedule B 39 GB, Schedule E 43
MB, committee history 14 MB). `hardmoney dumps import` restores one into
any Postgres, with preflight checks, a plan, and the exact `pg_restore`
command under `--explain`. Inside the agency that is a way to reproduce a
user's report against last weekend's snapshot, or to test a schema change
against the real DDL. The Schedule E dump into an empty database on a
laptop:

```text
$ hardmoney dumps import independent-expenditures --yes --database-url postgres://chris.gorski@localhost/hardmoney_fec_staff
✓ pg_restore: version 18.3 is installed (the FEC's files need 15 or newer)
...
Plan
  1. independent-expenditures: use the 43.4 MB file already downloaded to ~/.cache/hardmoney/dumps/schedule_e.dump, then load about 550,000 rows into table disclosure.fec_fitem_sched_e in database hardmoney_fec_staff (about 1 to 2 minutes).
...
independent-expenditures: 549,525 rows loaded
Done in 6 s.
```

With the FEC's processed rows and the raw `.fec` in one database,
`bulk-dump-compare` puts a filing's Schedule E lines beside the rows the
FEC derived from them, matched on the filer's transaction id. Progressive
Turnout Project's amended May monthly (filing 2010970, five independent
expenditures, processed by the FEC on 2026-09-11), ingested with
`bulk-load-filing 2010970`:

```text
$ hardmoney bulk-dump-compare 2010970
filing 2010970 (F3XA, C00580068)
                    raw .fec (schedule_e_lines)  FEC dump (fec_fitem_sched_e)
rows                5                            5
total amount        239520.14                    239520.14
matched by tran_id  5                            5
only in raw filing:  (none)
only in FEC dump:    (none)
amount mismatches:   (none)
newest file_num in dump: 2011113
```

Anything filed after `file_num` 2011113 is not in that snapshot yet; the
[dumps chapter](./pg-dumps.md#raw-filing-against-processed-rows-bulk-dump-compare)
lists the other reasons the sides differ. Parquet has been asked of the
FEC since March 2024
([fecgov/FEC#13168](https://github.com/fecgov/FEC/issues/13168): "open to
exploring it", stalled on DBA time). `hardmoney export` writes any filing
as Parquet with amounts as `Decimal128(38, 2)` and dates as `Date32`
(also CSV, JSON Lines, SQLite); cookbook example 25 reads the result with
pyarrow and DuckDB.

```text
$ hardmoney export tests/fixtures/F3XA_2011814.fec --format parquet -o /tmp/hm-parquet
table  rows   bytes
F3X       1  49,800
SchA     62  19,155
H4       61  15,551
...
9 table(s), 135 row(s), 132,767 bytes -> /tmp/hm-parquet/
```

### Measuring the processing lag

Transaction-level data "takes up to 30 days to process"
([openFEC#5911](https://github.com/fecgov/openFEC/issues/5911));
`/operations-log/` records when each filing's summary and transaction
passes finished, and the raw filing says on day 0 what pass 2 has to load.
[`fec_06_processing_lag.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/fec/fec_06_processing_lag.py)
finds a committee's reports with `hardmoney filings --json --fetch`,
counts the non-memo Schedule A and B lines in each, and joins the
operations log on `sub_id` (the key comes from `FEC_API_KEY` or
`~/fec_api_key.txt` and is never printed):

```text
$ python examples/fec/fec_06_processing_lag.py C00140855 2026
C00140855: 8 most-recent F3X report(s) in the 2026 cycle

    file report received    summary  itemized  SchA lines SchB lines  (non-memo, from the raw .fec)
 2011831 M9     2026-09-14      +0d   pending         132         12
 2006786 M8     2026-08-17      +0d      +14d         131          2
 1995180 M7     2026-07-15      +0d      +11d         126         14
 1982198 M6     2026-06-10      +0d      +15d         128         48
 1974840 M5     2026-05-14      +0d      +22d         142         17
 1960794 M4     2026-04-13      +0d       +1d          90         17
 ...
itemization lag: median 14 day(s), max 23, 1 still pending
```

For the day-0 side at scale, `hardmoney efile watch` follows the
e-filing RSS feed (no key) and `efile backfill` walks the daily archives;
see [Finding filings](./discovery.md).

## Audit and Inspector General

`hardmoney filings --committee ... --reconcile` reconciles every report a
query returns without keeping the files:

```text
$ hardmoney filings --committee C00001313 --cycle 2026 --form-type F3X --report-type M6 --reconcile
file_number  form  report  coverage                receipt_date  amend  most_recent  total_receipts  committee
1986128      F3X   M6      2026-05-01..2026-05-31  2026-06-20    0      yes               409655.93  C00001313
  1986128: reconcile F3X: 1 of 69 line(s) disagree
1 filing(s), 0 action failure(s)
```

[`fec_07_cycle_reconcile.py`](https://github.com/cgorski/hardmoney/blob/main/python/examples/fec/fec_07_cycle_reconcile.py)
does that across a committee's cycle and adds the floors. A floor line
(`LineCheck.relation == "at_least"`) is one the $200 itemization
threshold applies to (Form 3X 15, 17, 21(b), 24, 28(a), 29, 30(b); Form 3
11(d), 14, 15, 17, 20(a), 21; ten lines on Form 3P): the cover total must
be at least the itemized sum, and `reported - itemized` is the unitemized
remainder. Positive is normal. Negative means the schedule itemizes more
than the cover admits, which the FEC's validator lets through; the same
line negative report after report is what an auditor follows up.

```text
$ python examples/fec/fec_07_cycle_reconcile.py C00001313 2026
C00001313  REPUBLICAN PARTY OF MINNESOTA - FEDERAL  (12 report(s))
  1879324.fec                    F3XA  M2   20250101..20250131  balances
      unitemized remainder on floor lines: 21(b) +925.49
  1882086.fec                    F3XN  M3   20250201..20250228  balances
      unitemized remainder on floor lines: 21(b) +597.40, 28(a) +17086.35
  ...
  1945938.fec                    F3XA  MY   20250401..20250630  21(a)(i) -0.04; 21(a)(ii) +0.04
      unitemized remainder on floor lines: 15 +249.66
  ...
  1986128.fec                    F3XN  M6   20260501..20260531  11(c) +200.00
      unitemized remainder on floor lines: 15 +105.77, 21(b) +287.61
  ...
```

Twelve accepted reports, ten of which balance on every line. The mid-year
report splits four cents between the federal and nonfederal shares of
Schedule H4 differently from its cover; the May report carries the $200
gap its September amendment did not fix. Across the wider corpus
(`tests/fixtures/ORACLE_NOTES.md`) 96 of 98 freshly fetched reports
balance on every line, and the exceptions are single lines.

What exists: acceptance rules from the filing alone (`validate`);
cover-versus-schedule arithmetic for F3X, F3, F3P (`reconcile`);
amendment-chain resolution when filings are ingested into Postgres
([Amendments](./amendments.md)); the WebCheck oracle. Not built:
`hardmoney review`, the RFAI-style heuristics (employer and occupation
blank where a donor's aggregate exceeds $200, per-election aggregates
against the limits, dates outside the coverage period), and a two-sided
match of what a PAC reports giving against what the candidate reports
receiving, the analysis behind the Inspector General's $1.2 million
variance across a 50-candidate sample
([audit of the public disclosure process](https://www.fec.gov/about/reports-about-fec/oig-reports/executive-summary-audit-commissions-public-disclosure-process/)).
Contribution limits are not checked. The fields those checks need are
parsed and loaded; the rules are not written.

## Working with the project

Where the rules live: column positions per table and version in
`data/fec-csv-sources/*.csv`; field types, lengths, requirement levels,
allowed values, and the workbook's rule text in
`data/fec-spec/spec-8.5.json` (regenerated by
`scripts/distill_fec_spec.py`); cover-page arithmetic in the `F3X_RULES`,
`F3_RULES`, and `F3P_RULES` tables of `src/parser/reconcile.rs`, in the
FEC's notation
(`"11(a)(iii)" col_a_individual_contribution_total { = "11(a)(i)" + "11(a)(ii)" }`).
Every validation rule names the FEC message it implements
(`Rule::fec_message`), and the module docs of `src/parser/validate.rs`
list the FEC messages with no rule and why.

Reporting a disagreement: if hardmoney and the FEC's validator disagree
on a filing, or `reconcile` flags a line you believe is right, open an
issue with the filing id (or the file, if it is not public) and the
output of `hardmoney validate <file> --oracle webcheck --json` or
`hardmoney reconcile <file> --json`. A filing id reproduces everything in
this chapter.

License and provenance: hardmoney is `Apache-2.0 OR BSD-3-Clause`. The
FEC artifacts it uses (the format workbook, the `fecfile-validate`
schemas, the FECfile+ test values) are works of the United States
Government in the public domain (17 U.S.C. § 105), as the FEC's
repositories state in their license files. The column-map tables descend
from `fech-sources`, extracted from the New York Times's Fech gem
(Apache-2.0); `NOTICE` records that lineage and the six collision fixes
made to it.

Network: nothing is sent anywhere except to `fec.gov` hosts
(`docquery.fec.gov` for raw filings, `api.open.fec.gov` for metadata with
a key that is never logged or printed, `efoservices.fec.gov` only with
`--oracle webcheck`, `www.fec.gov` for bulk files and dumps). There is no
telemetry, and `validate()`, `reconcile()`, and `to_fec()` make no
network calls at all.
