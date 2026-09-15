# Parser oracle notes

`tests/oracle_fecfile.py` compares hardmoney's parse of every corpus filing,
field by field, against the [`fecfile`](https://github.com/esonderegger/fecfile)
Python library (MIT), whose column mappings (`fecfile/mappings.json`) are
maintained independently of ours. Run it with the venv described in the
script's docstring, or via `HARDMONEY_ORACLE_TESTS=1 cargo test --test
parser_oracle`. `--layouts` prints the static table-vs-table comparison
that the corpus run only exercises in part.

Last full run (2026-09-15): **150 files scanned, 141 parsed by both
libraries, 435,726 records and 19,516,660 field values compared, 24
disagreements in 6 classes, all explained.** The other 9 files are paper
(`P*`) conversions, a `/*`-style pre-3.0 header, `180.5`, and two files
with no cover line, which hardmoney rejects by design and fecfile either
reads with a paper mapping or yields nothing for.

Every disagreement class was investigated against the FEC's own artefacts:
the v8.5 specification workbook (`data/fec-spec/FEC_EFO_Format_Specifications_v8.5.xlsx`,
sheet per form, `COL SEQ` = 1-based column) and the FEC's per-version
column listings vendored with fech-sources (`data/fec-csv-sources/headers/<version>.csv`,
one row per record type; the 3.x and 5.2 files are clean, the 5.0/5.1/5.3
files interleave blank spacer columns and section headings and are only
usable for relative order). This file records who was right.

## hardmoney was wrong (fixed; recorded in `NOTICE`)

| Table / versions | Defect | Evidence | Fix |
|---|---|---|---|
| SchC2 3.x, 5.0-5.3 | Phantom `transaction_id` at column 3 shifted every guarantor field one column right. | headers/3.csv and headers/5.2.csv: BACK REFERENCE TRAN ID is col 3, IND/NAME 4, STREET 1-2 5-6, CITY 7, STATE 8, ZIP 9, INDEMP 10, INDOCC 11, AMOUNT 12. Same structure as SchC1 in those versions (verified against real `SC1/9` lines in `fec-parse/test/data/34411.fec`). fecfile's independent mapping agrees. | Columns corrected; `amended_cd` at 13 (5.x). |
| F5 6.1 | No layout: every 6.1 Form 5 failed with `NoMatchingVersionBucket`. | headers/6.1.csv F5 is byte-identical to 6.2. fecfile covers 6.1. | Bucket `^8.0\|7.0\|6.4\|6.3\|6.2` now includes 6.1. |
| F3Z 3.x-6.3 | No layout. | headers/3, 5.0-5.3, 6.1-6.3 list F3Z with the same 36 columns as 6.4-8.5 (labels differ only in wording). fecfile covers `^(8.1\|8.0\|7.0\|6\|5\|3\|2\|1)`. | Bucket widened to 3.x-8.5. |
| F3L all versions | Column 14 STATE OF ELECTION unnamed: unreadable, dropped on write. | v8.5 workbook sheet F3L row 14 (`A-2`, "X (warn if REPORT CODE=[12?\|30?])"); headers/6.4.csv-8.5.csv. The only workbook column no 8.5 layout named. fecfile also leaves it unnamed (`''`). | Named `state_of_election` (F3/F3X/F3P's name for the same field). |
| 3.x-5.x AMENDED CD | Unnamed on most tables, so real values were dropped on a round trip. | headers/3.csv and 5.2.csv list AMENDED CD (SchL: AMENDED CODE) before TRAN ID on F3P31 (31), F56 (26), F57 (31), F65 (26), F76 (15), F92 (33), F93 (30), F94 (10), SchC (25), SchC1 (41), SchC2 (13), SchD (27), SchE (36), SchF (36), H1 (28), H2 (10), H3 (10), H5 (10), H6 (38), SchL (40), TEXT (5); 5.3 relabels the slot SPACE HOLDER. Corpus values: `A` on the H1 line of `fec-parse/test/data/trailing-quote.fec` (3.00), `N` on SD10 lines of `character-encoding.fec` (3.00). fecfile names it on H1/H2/H3/SchC/SchD/SchF/F65. | Named `amended_cd` in every 3.x/5.x bucket of those tables. |

Two parser behaviours (not table data) were also wrong and are fixed in
`src/parser/filing.rs` / `stream.rs`:

* **Comma-delimited records were parsed as one CSV stream**, so a quoted
  field could span lines. `fec-parse/test/data/trailing-quote.fec`, a real
  Aristotle CM4 3.00 filing, ends every Schedule H4 record with an
  unbalanced `"`; that quote opened a field which swallowed the remaining
  113 records into one -- silently, with no error. fecfile, Fech and
  FastFEC all parse one record per physical line, as the FEC format
  defines. hardmoney now does too (`split_old_delimited`), on both the
  eager and streaming paths.
* **`line_no` on the comma path was off by the blank lines skipped before
  a record** (the `csv` crate stamps a record with the position it started
  scanning from) and the cover line was always reported as 2. It is now
  the physical line on both paths, and the oracle's `line_number` class --
  120 occurrences before -- is gone.

## fecfile is wrong (hardmoney unchanged)

| Class in the report | Where | Evidence |
|---|---|---|
| `only_hardmoney F24 treasurer_name` | F24 3.x-5.x col 9 | headers/3.csv and 5.2.csv: `NAME/TREASURER (as signed)` at col 9. fecfile's mapping has `''` there. |
| `only_hardmoney F57 payee_street_1`, `value F57 payee_street_2` | F57 3.x cols 5-6 | headers/3.csv: STREET 1 at 5, STREET 2 at 6. fecfile puts `payee_street_2` at 5 and nothing at 6 (the fech-sources collision hardmoney fixed in 2.0, see NOTICE). |
| `only_hardmoney F5 individual_occupation` | F5 5.3 col 12 | headers/5.3.csv INDOCC at 12 (relative order). fech-sources had it at 18 colliding with the coverage date; fecfile inherited that. |
| `only_hardmoney F3S ...` | F3S 3.x-5.x cols 26, 29, 30 | headers/3.csv, 5.2.csv list 19(b), 20(b), 20(c) there; fech-sources had label text in the position cells, fecfile has `''`. |
| `only_hardmoney H1 form_type` | H1 3.x col 1 | fecfile's `^3.0\|^2\|^1` H1 mapping names column 1 `ballot_local_candidates`. FORM TYPE per headers/3.csv. |
| `fecfile_duplicate_name` | F3X/F3P/F4 totals vs. recap totals, F2 `candidate_state`, SchC1 `description`/`date_signed`, SchL column-A/B | fecfile maps two columns to one key and its `dict` keeps the later column, so the earlier value is unreachable through fecfile. hardmoney renamed each pair (NOTICE). The oracle detects this automatically from fecfile's mapping list. |
| `fecfile_missing_mapping` (static) | F3Z 8.2-8.5, SchI 8.0-8.4, F2S everywhere | headers/8.2.csv-8.5.csv list F3Z; headers/7.0-8.4 list SchI; headers/6.1-8.5 list F2S. fecfile's `(^f2$)\|(^f2[^4])` regex parses F2S lines with the F2 layout (every F2S field lands under a candidate-name key). |
| SchI `transaction_id` (static) | SchI 6.x-7.0 col 3 | headers/6.1.csv-7.0.csv: TRANSACTION ID NUMBER at 3; fecfile has `''`. |
| F93 3.0 (static) | coverage | fecfile's `^(P3.1\|3.0\|P2\|P1)` claims electronic 3.0, but headers/3.csv has no F93 (Form 9 family arrived in 5.0). The `3.0` is a typo for `P3.0`. |
| `encoding` | F99 text blocks in `fech_862554.fec`, `ff_1260488.fec` | fecfile decodes non-UTF-8 lines as ISO-8859-1, so byte 0x92 becomes the C1 control U+0092; hardmoney decodes Windows-1252, where 0x92 is `'` (RIGHT SINGLE QUOTATION MARK) -- what the Windows software that wrote the file meant. |
| `line_number` (before the comma-path fix) | `undefined-row-type.fec` | fecfile numbered physical lines correctly; hardmoney did not. Now both agree. |

## Differences by design (neither is wrong)

* fecfile drops any record with fewer than two fields (`fecfile_short_line`);
  hardmoney parses a bare token as an empty record of its table.
* fecfile yields a `[BEGINTEXT]` block as a separate `F99_text` item;
  hardmoney splices it into the preceding record's `text` field. The oracle
  compares the two directly.
* hardmoney rejects paper-conversion versions (`P3.4`, ...), `/*`-style
  pre-3.0 headers, unknown versions (`180.5`), and files with no cover line
  (`empty.fec`, `last-value.fec`), each with a specific `FecError`.

## Observed but not changed

* **H1 5.2/5.3 `transaction_id` at column 3.** hardmoney and fecfile both
  put it there (both descend from fech). The FEC's headers/5.2.csv puts
  TRAN ID at column 29 with columns 3-27 as SPACE HOLDER and 28 INTERNAL
  USE ONLY. The corpus has no 5.2/5.3 H1 line to settle it; left as is and
  noted here.
* **F92/F93 5.x are partial layouts.** The FEC's 5.2 listing has 44 (F92)
  and 43 (F93) columns; fech-sources (and fecfile) name a subset -- e.g.
  F92 5.x lacks ITEM ELECT CD, AGGREGATE AMT Y-T-D, the candidate and
  conduit blocks, MEMO CODE/TEXT. Only `amended_cd` was added (position
  corroborated by all four 5.x listings); the rest would be filled in blind
  with no corpus data to check against.
* **F99 `text` (column 18)** is in the FEC's 8.5 listing but not in the
  workbook, whose F99 sheet stops at PDF ATTACHMENT (17) because the text
  travels in the `[BEGINTEXT]` block. The layout keeps it; asserted in
  `tests/parser_oracle.rs`.

# Reconciliation notes

`Filing::reconcile`'s rule tables were checked in September 2026 against
every Form 3X / 3 / 3P in the local corpus plus 45 state-party F3X reports
(the only filers of Schedules H1-H4), 24 national-party F3X reports, 12
House/Senate F3 reports with Schedule C loans and Schedule D debts, three
joint-fundraising-committee F3X reports, and 14 presidential F3P reports,
all fetched from the FEC's API with `hardmoney filings --fetch`. The
`RELATION` decisions (exact vs. floor) come from the FEC's own Form 3X, 3,
and 3P instructions (revised 05/2016), line by line; see
`Relation` in `src/parser/reconcile.rs`.

**Corpus (124 F3X/F3/F3P reports under `tests/fixtures`, `tmp/agent-misc`,
`tmp/competitors`): 102 satisfy every Column A rule.** Of the 22 that do
not, 17 are truncated or hand-edited third-party test samples (schedule
sums of zero or one line against six- and nine-figure cover totals:
`fech_723604`, `fech_97405`, `ff_1385191`, `psql-end-marker`,
`quote-left-open`, `trailing-quote`, `undefined-row-type`, `multi_text`,
`slash_form`, `text`, `too_few_fields`, `too_many_fields`, and their
duplicates) and five are genuine single-line filer discrepancies in
otherwise self-consistent reports. **Of the 98 freshly fetched reports, 96
balance on every line.** Each disagreement:

| Filing | Line | Reported | Itemized | Reading |
|---|---|---|---|---|
| `tmp/agent-misc/filings/2011912.fec` (state party F3XA, 8.5) | 11(c) | 2,045.00 | 1,845.00 | One $200 committee contribution on the cover and not on Schedule A. Every formula that consumes 11(c) holds. The FEC's validator only *warns* about subtotal support (its W4), so the report was accepted. |
| `tmp/agent-misc/filings/2010101.fec` (Trump 2020 F3P 30G) | 17(c) | 89,797.00 | 93,197.00 | 47 `SA17C` lines including four negative reversals; the cover nets $3,400 less than the schedule. Formulas hold. |
| `tmp/agent-misc/samples/ff_1162172.fec` (Trump 2016 F3PA, 8.1) | 17(c) | 37,379.50 | 36,574.50 | Cover $805 above 36 itemized committee contributions. Formulas hold. |
| `tmp/agent-misc/samples/fech_467627.fec` (Djou for Hawaii F3N, 6.4) | 11(a)(i) | 299,378.36 | 299,728.36 | Cover $350 below 476 itemized `SA11AI` lines. Formulas hold. |
| `tmp/competitors/FastFEC/.../1527862.fec` (Biden for President F3PN, 8.3) | 28(a) | 102,307.46 | 105,046.25 | 367 `SB28A` lines (61 negative voids) net $2,738.79 more than the cover's refunds to individuals; a floor line, and the cover is *below* it. |
| NRSC `2000696.fec` (F3XN M7 2026, 57,152 `SA12` lines) | 12 | 4,678,439.86 | 4,678,439.74 | Twelve cents. 40 non-memo transfers from joint fundraising committees; the 57,112 memo lines behind them are correctly excluded. Every formula holds. A filer rounding artefact the FEC accepted. |
| Vivek 2024 `1819163.fec` (F3PT termination, 8.4) | 23 | 30,882.14 | 30,974.10 | 36 `SB23` lines including eight paired +/- reversals; the cover is $91.96 *below* the itemized operating expenditures (a floor line). Formulas hold. |

What the allocation-schedule data settled (details in the module docs of
`src/parser/reconcile.rs`): 18(a) is the sum of H3 `transferred_amount`
(each event-type record repeats the transfer total in
`total_amount_transferred`, so summing that column double-counts split
transfers -- e.g. MN DFL transfer `4948AD`, $46,102.21 = $44,060.51 `AD` +
$2,041.70 `DF`); 21(a)(i)/(ii) are the H4 federal/nonfederal shares with
4,065 memo records across the sample correctly excluded; no report carried
an `SB21A`; and 30(b) is a floor (seven reports above their `SB30B` sum by
$5.42 to $335.88, consistent with 11 CFR 300.36(b)(2)(iv)). H5 and H6
appeared in no report; those lines rest on the workbook layout.

Rules changed as a result: F3X 24 and 30(b), F3 11(d), and F3P 17(d) are
now floors (`>=`); each is a line the FEC's instructions itemize only
"aggregating in excess of $200". Fixtures added: `F3XA_2011814.fec`
(Georgia Republican Party, H2/H3/H4/SL), `F3XN_1998773.fec` (Republican
Party of Virginia, H3/H4, 30(b) above its itemized sum),
`F3XA_2008083.fec` (New Hampshire Democratic Party, 13 H3 and 267 H4
records, `SA15`, `SB29`), `F3A_2004471.fec` (Keith Gross for Florida, 58
`SC/10` and 3 `SD10`), `F3XN_1965568.fec` (Emmer Majority Builders JFC,
`SA12`/`SB22`), `F3PA_1993032.fec` (Jill Stein for President 2024 at spec
8.5, `SD12`, `SA20A`, `SB28A`).

# Validation notes

The same fetched corpus (364 files that parse, of which 361 report zero
errors) was run through `Filing::validate`. Error-severity findings on
FEC-accepted filings are zero after three fixes found this way:

* `NUM-5` allocation percentages on Schedules H1/H2 are written `0.49`; a
  digits-only reading of `NUM` flagged them `non_numeric` on 16 of the 45
  state-party reports. A decimal point is now numeric.
* `AMT-12` bounds digits, not characters: ActBlue's accepted September
  2020 amendment (`tmp/competitors/feco3/test/fecs/multi_text.fec`,
  FEC-1458871, spec 8.3) carries `2280311229.59` in thirteen characters.
* A 2001 spec-3.00 report from Puerto Rico
  (`tmp/competitors/fec-parse/test/data/character-encoding.fec`) was
  accepted with `á` and `í` in contributor names, outside today's 32-168
  byte range; `illegal_character` is now demoted to a warning on superseded
  formats, as `required_field_empty` already was.

The three files still reporting errors are synthetic competitor test
cases, not accepted filings: `no-header-mapping.fec` (its cover is a
Schedule I), `undefined-row-type.fec` (its back-references point at the
deliberately unrecognised rows the lenient parse skipped), and
`too_many_fields.fec` (extra cells spliced into `SA11D` lines). Warnings
across the corpus, most common first: `recommended_field_empty` 549,
`required_field_empty` 337 (all demoted, on 3.x-6.x files),
`invalid_zip_code` 278 (foreign postal codes and four-digit ZIPs),
`invalid_election_code` 270 (bare `P`/`G` on pre-6.x files),
`invalid_district` 126 (one-digit districts, all on the 2001 Merck
filing), `current_format` 99, `invalid_event_type` 69 (pre-BCRA `A`/`D`
on 3.00 files), `illegal_character` 51 (the Puerto Rico filing),
`embedded_double_quote` 6, `f99_illegal_character` 5,
`unrecognized_form_type` 4, `pattern_mismatch` 4, `date_out_of_range` 3,
`address_in_second_line` 3.
