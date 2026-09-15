# What hardmoney does with messy FEC data

Who this is for: an FEC data engineer or RAD analyst, a downstream data
team, or anyone who has ever discovered a government dataset's
undocumented quirk the hard way, in production, at 2 a.m.

What you'll have at the end: a catalogue of nine specific things the
FEC's electronic filings and bulk files do that a naive parser or loader
gets wrong, with, for each one, what the FEC ships, what hardmoney does
about it, and a command you can run to see it yourself. None of this is
opinion about the FEC's data. Every item is a documented, reproducible
behaviour of real files, and every hardmoney behaviour described is
pinned by a test in the repository.

Some demonstrations use tiny synthetic files built from real fixtures,
because the real-world triggers (a filer's stray byte, a truncated
upload) aren't things you can download on demand. Where a demonstration
uses a namespace, it's `tut_dq`:

```bash
export DATABASE_URL="postgres://chris.gorski@localhost:5432/hardmoney_v1_test"
hardmoney schema-init --schema tut_dq
```

## 1. Two columns with the same name (six format tables)

**What the FEC ships.** The electronic filing spec defines each form's
fields by position. Human-readable field names are added by the
community-maintained column-position tables (`fech-sources`, descended
from the NYT's Fech) that hardmoney and most other open-source parsers
build on. Six of those tables (F2, F3P, F3X, F4, SchC1, SchL) assigned
the same name to two different positions within one spec-version
bucket. On Form 3X, for example, line 16 "Refunds of
Federal Contributions" (a receipt) and line 34 "Total Contribution
Refunds" (a disbursement) both mapped to `col_a_total_contributions_refunds`.

**What hardmoney does.** The vendored tables were audited and the
colliding names made distinct and form-accurate (every rename is listed
in the repository's `NOTICE`). Two safeguards prevent a regression: the
line parser returns a hard `FecError::DuplicateCanonicalField` if any
table ever assigns one name to two positions again, and
`tests/format_table_integrity.rs` scans every bundled table on every test
run.

**See it yourself.** A real Form 3X where the two fields hold different
values, so a name collision would have silently discarded one:

```bash
hardmoney parse tests/fixtures/F3XN_2011834.fec | grep -E 'refunds_of_federal_contributions|total_contributions_refunds'
```

```text
    "col_a_refunds_of_federal_contributions": "5000.00",
    "col_a_total_contributions_refunds": "0.00",
    "col_b_refunds_of_federal_contributions": "20000.00",
    "col_b_total_contributions_refunds": "5000.00",
```

Background in
[Parsing a filing, explained](./parsing-explained.md#a-real-bug-this-design-caught-the-field-name-collision-fix).

## 2. `oppexp` has one more column than its own header file

**What the FEC ships.** The data dictionary for the operating-expenditures
bulk file (`oppexp_header_file.csv`) lists 25 columns. Every row of the
actual `oppexp26.zip` has 26 pipe-delimited fields, a trailing empty
one. This is publicly tracked as
[fecgov/FEC#11052](https://github.com/fecgov/FEC/issues/11052).

**What hardmoney does.** The `disbursements` source is declared with
`allow_extra_trailing_fields: true`, so the extra field is dropped and
the row loads. Every other source is declared strict, so a row with
too many or too few fields fails the load with the line number rather
than shifting every column one place to the right.

**See it yourself.** `funzip` (part of Info-ZIP, present on macOS and most
Linux) can decompress the first entry of a zip from a partial stream, so
300 KB of the 45 MB file is enough:

```bash
curl -sL -r 0-300000 https://www.fec.gov/files/bulk-downloads/2026/oppexp26.zip | funzip 2>/dev/null | head -3 | awk -F'|' '{ print NF " fields" }'
curl -sL https://www.fec.gov/files/bulk-downloads/data_dictionaries/oppexp_header_file.csv | tr ',' '\n' | wc -l
```

```text
26 fields
26 fields
26 fields
25
```

And the strictness on the other side. A one-row `indiv`-layout file with
an extra 22nd field, loaded as `schedule_a` from disk:

```bash
python3 -c "
import zipfile
row = 'C00000001|N|Q1|P|1|15|IND|EXTRA, FIELD|CITY|NC|27601|ACME|CLERK|10222025|100||T9|1|||9000000000000009|'
with zipfile.ZipFile('/tmp/extrafield.zip', 'w', zipfile.ZIP_DEFLATED) as z:
    z.writestr('itcont.txt', row + '\n')
"
hardmoney bulk-load --schema tut_dq schedule_a --cycle 2026 --file /tmp/extrafield.zip
```

```text
error: malformed row at line 1: expected 21 fields, found 22
```

Exit status 1, nothing loaded.

## 3. Two date formats across the bulk files

**What the FEC ships.** Dates are text. `indiv`, `oth`, and `pas2` write
`MMDDYYYY` (`10222025`); `oppexp`, `weball`, `webl`, and `webk` write
`MM/DD/YYYY` (`02/12/2026`). Neither sorts as text: `12312024` sorts after
`11182025`.

**What hardmoney does.** The raw text is kept verbatim in the `*_dt`
column. A parsed `DATE` twin (`*_date`) is added, accepting both formats.
A blank raw value becomes `NULL` quietly; a non-blank value that isn't a
calendar date becomes `NULL` and is counted, in the load's output, in
`LoadReport::dates_nulled`, and in the `loads` table, so a file with a
systematic date problem is visible, not a silently empty column. No date
is ever invented.

**See it yourself.** The two formats, straight from the files:

```bash
curl -sL -r 0-300000 https://www.fec.gov/files/bulk-downloads/2026/indiv26.zip  | funzip 2>/dev/null | head -2 | cut -d'|' -f14
curl -sL -r 0-300000 https://www.fec.gov/files/bulk-downloads/2026/oppexp26.zip | funzip 2>/dev/null | head -2 | cut -d'|' -f13
```

```text
10222025
10222025
02/12/2026
02/17/2026
```

And the counting, with a four-row file covering the cases:

```bash
python3 -c "
import zipfile
rows = [
 'C00000001|N|Q1|P|1|15|IND|FINE, DATE|CITY|NC|27601|ACME|CLERK|10222025|100||T1|1|||9000000000000001',
 'C00000001|N|Q1|P|1|15|IND|BLANK, DATE|CITY|NC|27601|ACME|CLERK||100||T2|1|||9000000000000002',
 'C00000001|N|Q1|P|1|15|IND|ZEROS, DATE|CITY|NC|27601|ACME|CLERK|00000000|100||T3|1|||9000000000000003',
 'C00000001|N|Q1|P|1|15|IND|FEB30, DATE|CITY|NC|27601|ACME|CLERK|02302026|100||T4|1|||9000000000000004',
]
with zipfile.ZipFile('/tmp/baddates.zip', 'w', zipfile.ZIP_DEFLATED) as z:
    z.writestr('itcont.txt', '\n'.join(rows) + '\n')
"
hardmoney bulk-load --schema tut_dq schedule_a --cycle 2026 --file /tmp/baddates.zip
psql "$DATABASE_URL" -c "SET search_path TO tut_dq, public;" \
  -c "SELECT name, transaction_dt AS raw, transaction_date AS parsed FROM schedule_a ORDER BY sub_id;" \
  -c "SELECT source, row_count, dates_nulled FROM loads ORDER BY load_id DESC LIMIT 1;"
```

```text
schedule_a: loaded 4 rows for cycle 2026 (replace, 2 unparseable date(s) set NULL)
    name     |   raw    |   parsed
-------------+----------+------------
 FINE, DATE  | 10222025 | 2025-10-22
 BLANK, DATE |          |
 ZEROS, DATE | 00000000 |
 FEB30, DATE | 02302026 |
(4 rows)

   source   | row_count | dates_nulled
------------+-----------+--------------
 schedule_a |         4 |            2
(1 row)
```

Four rows, three `NULL`s, two counted: the blank is normal and isn't,
the all-zeros and the 30th of February are. The raw column still shows
exactly what was in the file. [Dates](./dates.md) has the full treatment.

## 4. Whole dollars in one file, cents in another

**What the FEC ships.** The FEC's description of the individual
contributions file documents `TRANSACTION_AMT` as `NUMBER(14,2)` with the
example `1000.00`. The file itself ships whole dollars: in a 200,000-row
sample of `indiv26.zip` loaded for the journalist tutorial, every row
was an integer with no decimal point (`3500`, `104`), and a 1,000-row
sample of `pas2` was the same. `oppexp`, by contrast, carries cents on
most rows (`138.6`, `1562.98`, `-50`). And the FEC's own `pg_dump` of
Schedule E declares `exp_amt NUMERIC(14,2)`, so every value there has
exactly two places (`2500.00`).

**What hardmoney does.** Money columns are `NUMERIC` with no forced scale,
loaded via `COPY` from the text exactly as shipped and read back as
`rust_decimal::Decimal`, never `f64`. So `725` stays `725`, `1562.98`
stays `1562.98`, `2500.00` stays `2500.00`; nothing is rounded,
re-scaled, or passed through binary floating point on the way in or out,
and the REST API serializes amounts as JSON strings so a client can't
round them either. Whether a whole-dollar figure was rounded by the filer
or by the FEC's export is not something the file records, and hardmoney
doesn't guess.

**See it yourself.** Three sources after a `--limit 1000` load of each.
These numbers came from the [researcher tutorial](./tutorial-researcher.md)'s
namespace; `hardmoney bulk-load-all --schema tut_dq --cycle 2026 --limit 1000`
followed by the query below reproduces them (against whatever the FEC's
current file contains):

```sql
SELECT 'schedule_a (indiv)' AS source,
       count(*) FILTER (WHERE transaction_amt::text !~ '\.') AS whole_dollar,
       count(*) FILTER (WHERE transaction_amt::text ~ '\.')  AS with_cents
FROM schedule_a
UNION ALL
SELECT 'disbursements (oppexp)',
       count(*) FILTER (WHERE transaction_amt::text !~ '\.'),
       count(*) FILTER (WHERE transaction_amt::text ~ '\.')
FROM disbursements
UNION ALL
SELECT 'committee_to_candidate (pas2)',
       count(*) FILTER (WHERE transaction_amt::text !~ '\.'),
       count(*) FILTER (WHERE transaction_amt::text ~ '\.')
FROM committee_to_candidate_transactions;
```

```text
            source             | whole_dollar | with_cents
-------------------------------+--------------+------------
 schedule_a (indiv)            |         1000 |          0
 disbursements (oppexp)        |          248 |        752
 committee_to_candidate (pas2) |         1000 |          0
```

## 5. Windows-1252 bytes in a file that's supposed to be text

**What the FEC ships.** Filings are produced by many vendors' software.
Free-text fields (a payee name with an accent, a memo pasted from a
word processor) occasionally contain single bytes that are valid
Windows-1252 but invalid UTF-8. The bulk files inherit the same bytes.

**What hardmoney does.** `Filing::parse_bytes` decodes as UTF-8 and, if
that fails, as Windows-1252. The bulk loader does the same per line
(`decode_line`), so one stray byte in a memo field never fails a 2 GB
load. In both cases the result is correct text, not a replacement
character and not an error.

**See it yourself.** Put a Windows-1252 `é` (byte `0xE9`) into a real
filing's payee name, then show that Python's UTF-8 decoder rejects the
file while hardmoney reads it:

```bash
python3 -c "
b = open('tests/fixtures/F24N_2011823.fec', 'rb').read()
open('/tmp/cp1252.fec', 'wb').write(b.replace(b'Strategic Media', b'Strat\xe9gic Media'))
"
python3 -c "open('/tmp/cp1252.fec', 'rb').read().decode('utf-8')" 2>&1 | tail -1
hardmoney parse --lines /tmp/cp1252.fec | grep payee_organization_name
```

```text
UnicodeDecodeError: 'utf-8' codec can't decode byte 0xe9 in position 161: invalid continuation byte
        "payee_organization_name": "STRATÉGIC MEDIA PLACEMENT INC.",
```

## 6. A `[BEGINTEXT]` block that never ends

**What the FEC ships.** Form 99 (miscellaneous text) and some other
filings carry a free-text block between `[BEGINTEXT]` and `[ENDTEXT]`
markers. A truncated upload, or filing software that forgets the closing
marker, leaves the block open to end-of-file.

**What hardmoney does.** That is a distinct, hard error,
`FecError::UnterminatedTextBlock { line_no }`, in both strict and
lenient mode, because it isn't a problem with one body line: the parser
can't know where the text ends and the records resume, so nothing after
that point can be trusted. Lenient mode only relaxes body-line problems
([Strict vs. lenient](./strict-vs-lenient.md#what-lenient-skips-and-what-still-fails)).

**See it yourself.** Cut a real Form 99 off before its `[ENDTEXT]`:

```bash
python3 -c "
b = open('tests/fixtures/F99_2011828.fec', 'rb').read()
open('/tmp/unterminated.fec', 'wb').write(b[:b.index(b'[ENDTEXT]')])
"
hardmoney parse /tmp/unterminated.fec
hardmoney parse --lenient /tmp/unterminated.fec
```

```text
error: unterminated [BEGINTEXT] block starting at line 3
error: unterminated [BEGINTEXT] block starting at line 3
```

## 7. Form 2S has no upstream format table

**What the FEC ships.** A Form 2 (statement of candidacy) lists the
candidate's authorized committees as `F2S` body lines. The community
`fech-sources` tables that describe column positions for every other
form have no `F2S.csv`, so a parser built purely on those tables can't
route the line.

**What hardmoney does.** `F2S.csv` is authored locally from the FEC's
spec and bundled with the others; `F2S` lines dispatch to `Table::F2S`.
The `NOTICE` file records it as a local addition.

**See it yourself.** A real amended Form 2:

```bash
hardmoney parse tests/fixtures/F2A_2011896.fec | grep -A2 lines_by_table
```

```text
  "lines_by_table": {
    "F2S": 3
  },
```

## 8. Schedule I exists through spec 8.4 and not in 8.5

**What the FEC ships.** Schedule I (Levin-fund account summaries) was
part of the electronic format from spec 3.x through 8.4 and was dropped
in 8.5. A parser has to know it for old filings and not pretend to know
it for new ones.

**What hardmoney does.** The `SI` token dispatches to `Table::SchI`,
whose column table has version buckets for `6`-`8.4` and `3`/`5` only.
An `SI` line in an 8.5 filing therefore hits a table with no layout for
that version: `FecError::NoMatchingVersionBucket` under strict parsing,
or a `SkipReason::NoLayoutForVersion` skip under lenient. That is a
different error from "unknown form type", on purpose: it tells you the
record type is real but shouldn't appear at this spec version.

**See it yourself.** Append an `SI` line to a real 8.5 filing and to a
real 6.1 filing:

```bash
cp tests/fixtures/F3XN_2011834.fec /tmp/with_si.fec
printf 'SI\x1cC00696443\x1cLEVIN ACCOUNT\n' >> /tmp/with_si.fec
hardmoney parse /tmp/with_si.fec
hardmoney parse --lenient /tmp/with_si.fec 2>&1 >/dev/null   # stderr only
```

```text
error: no column-position data to parse table SchI at spec version '8.5' at line 19
warning: 1 line(s) skipped:
  line 19: 'SI' skipped (no column layout for this spec version)
```

```bash
cp tests/fixtures/F3XN_320000_v6.1.fec /tmp/with_si_61.fec
printf 'SI\x1cC00425439\x1cLEVIN ACCOUNT\n' >> /tmp/with_si_61.fec
hardmoney parse /tmp/with_si_61.fec | grep -A5 lines_by_table
```

```text
  "lines_by_table": {
    "SchA": 4,
    "SchB": 15,
    "SchI": 1,
    "TEXT": 1
  },
```

Same line, two spec versions, two correct answers.

## 9. `F8II` and `F8III` on the wire, `F82` and `F83` in the tables

**What the FEC ships.** Form 8 (debt settlement plan, spec 5.x/6.x) has
parts II and III whose on-the-wire form-type tokens are `F8II` and
`F8III`. The format tables that describe them are named `F82.csv` and
`F83.csv`. A dispatcher keyed only on table names never matches the
token.

**What hardmoney does.** Both spellings dispatch: `F8II` or `F82` to
`Table::F82`, `F8III` or `F83` to `Table::F83`. The dispatch order in
`src/parser/form.rs` tries them before the bare `F8` pattern so the
sub-forms don't fall into the parent.

**See it yourself.** No Form 8 fixture ships with the crate (the form is
historical), so this is a three-line synthetic filing at spec 6.1:

```bash
printf 'HDR\x1cFEC\x1c6.1\x1cFECfile\x1c6.1.1.1\x1c\x1c\nF8N\x1cC00123456\x1cEXAMPLE PAC\nF8II\x1cC00123456\x1cD1\x1cORG\x1cACME PRINTING\nF8III\x1cC00123456\x1cE1\n' > /tmp/f8.fec
hardmoney parse --lines /tmp/f8.fec | grep -E '"(raw_form_type|table)"'
```

```text
      "raw_form_type": "F8II",
      "table": "F82"
      "raw_form_type": "F8III",
      "table": "F83"
```

## Where the counts live

Every one of these behaviours leaves a trace you can query:

| Behaviour | Where to look |
|---|---|
| Body lines a lenient parse skipped, with line number and reason | `hardmoney parse --lenient` (stderr + `skipped` in the JSON); `Lenient::into_parts()` in code; `filings.skipped_lines` and `GET /filings/{id}` after `bulk-load-filing` |
| Date cells nulled because they weren't calendar dates | the load's output line; `LoadReport::dates_nulled`; `loads.dates_nulled`; `schema-status`; `GET /schema` |
| Which exact FEC file a table came from | `loads.source_url`, `loads.source_etag`, `loads.source_last_modified` |
| Whether a row's amount had cents | the `NUMERIC` value itself: `transaction_amt::text` |
| Rows that failed to load | there are none: a malformed row fails the whole load with its line number, and the transaction rolls back |

## Clean up

```bash
hardmoney schema-drop tut_dq --yes
```

```text
dropped namespace 'tut_dq'
```

## Where to go next

- [Parsing a filing, explained](./parsing-explained.md) for the
  spec-version and delimiter history that makes items 1, 7, 8, and 9
  necessary.
- [Dates](./dates.md) and [Reloading](./reloading.md) for items 3 and 4
  on the database side.
- The repository's `NOTICE` file for every format-table correction, with
  the FEC form line that justifies each rename.
- [Troubleshooting](./troubleshooting.md) for the error messages these
  behaviours produce, one entry each.
