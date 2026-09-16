# Reviewing a filing

Once the FEC accepts a report, an analyst in the Reports Analysis
Division (RAD) reads it. Where the report has "an error, omission or
possible prohibited activity", the committee gets a Request for
Additional Information (RFAI): a public letter, filed as form type `FRQ`,
that the committee has 35 days to answer. RAD applies "an internal,
Commission-approved review policy and its thresholds" to every report
([Request for Additional Information](https://www.fec.gov/help-candidates-and-committees/request-additional-information/));
the policy itself is not published, but the letters are, with a
`request_type` code (1 Statement of Organization, 2 report of receipts
and expenditures, 3 second notice, 5 informational, 7 failure to file;
openFEC `docs.py`). In the 2024 cycle RAD reviewed 196,175 documents; the
`/filings/?form_type=FRQ&cycle=2024` list ran to 187 pages of 100 letters
when it was read for the evaluation below.

`Filing::review` and `hardmoney review` do the part of that review a file
can support: deterministic checks whose inputs are all in the report (and,
with the committee's earlier reports, across them). Two things to keep in
view:

- A review is advisory. `validate` says whether the FEC would accept the
  file; `review` says what an analyst might ask about. Its exit status is
  always 0.
- A review sees the file and nothing else. RAD also sees the committee's
  registration, its other reports and notices, its earlier letters, and
  the other side of every transaction. An empty review does not mean no
  letter will come. The [measured agreement](#how-well-it-agrees-with-rad)
  with the FEC's letters is below.

## The command

```text
$ hardmoney review tests/fixtures/rad/F3XA_2011912.fec
tests/fixtures/rad/F3XA_2011912.fec F3XA C00001313 (20260501..20260531)
  best_efforts_claimed: 16 observation(s), 23558.33 at issue (heuristic)
    line 97 SA11AI.172101904 240.00: BRYNGELSON, LEE aggregates 240.00 and the employer/occupation fields read 'INFORMATION REQUESTED' / 'INFORMATION REQUESTED': the committee is reporting under best efforts (11 CFR 104.7(b))
    ...
  cover_not_supported: 1 observation(s), 200.00 at issue
    line 2 200.00: column A line 11(c) reports 2045.00 but the rule = sum of SchA.contribution_amount on SA11C gives 1845.00; delta 200.00
  negative_itemization: 5 observation(s), 1100.00 at issue
    line 139 SA11AI.172033559 -600.00: SA11AI carries a negative counted amount -600.00 from COHAN, LIANA with no reason in its text; a refund of a contribution belongs on Schedule B, and a reattribution, redesignation, or returned check should say so
    ...
  duplicate_transaction: 3 observation(s), 705.00 at issue (heuristic)
    line 87 SA11AI.172101907 240.00: SA11AI from BRADFORD, CHUCK dated 2026-05-29 for 240.00 repeats line 86 (SA11AI.172101906); if this is one contribution reported twice, the second entry overstates the totals
    ...
  25 observation(s) in 4 concern(s); amount at issue 25563.33
```

That is the Republican Party of Minnesota's May 2026 monthly as amended,
accepted by the FEC with no validation finding. The $200 on line 11(c) is
the discrepancy [Reconciling a filing](./reconciling.md) walks through;
the review adds the five unexplained negative entries, the three
same-day repeats, and the sixteen contributors reported under best
efforts.

Observations are grouped by concern with a count and the amount at
issue, then listed (the first twenty per concern; `--all` lists every
one). Inputs may be paths or filing ids, which are downloaded through the
filing cache. `--json` prints one array with an object per input
(`file`, `form_type`, `committee_id`, coverage dates, `recipient`,
`observations`, `summary`); `--with-prior FILE...` adds the chain checks
(one current report only); `--recipient` names the committee's kind for
the contribution limit.

A presidential committee's June 2024 amendment, spec 8.5:

```text
$ hardmoney review tests/fixtures/F3PA_1993032.fec
tests/fixtures/F3PA_1993032.fec F3PA C00856112 (20240601..20240630), limits as candidate
  employer_occupation_missing: 1 observation(s), 240.00 at issue
    line 953 SA17A.39243 240.00: Winthrop, Grant aggregates 240.00 (as reported), over the $200 threshold, and the employer and occupation field is blank; 11 CFR 104.3(a)(4)(i) requires both once the aggregate exceeds $200
  over_limit_aggregate: 2 observation(s), 309.90 at issue
    line 493 SA17A.40914 300.00: Kuriloff, Ronald's contributions for election P2024 total 3600.00 in this report, 300.00 over the 3300 per-election limit for the 2024 cycle (11 CFR 110.1(b)); redesignation and reattribution memo entries are already netted
    line 971 SA17A.40768 9.90: Yadon, August's contributions for election P2024 total 3309.90 in this report, 9.90 over the 3300 per-election limit for the 2024 cycle (11 CFR 110.1(b)); redesignation and reattribution memo entries are already netted
  duplicate_transaction: 1 observation(s), 400.00 at issue (heuristic)
    line 523 SA17A.39428 400.00: SA17A from Manning, Michelle dated 2024-06-20 for 400.00 repeats line 522 (SA17A.39427); if this is one contribution reported twice, the second entry overstates the totals
  4 observation(s) in 3 concern(s); amount at issue 949.90
```

## The concerns

Each concern is one class of question. The regulation or FEC instruction
behind it is in the rustdoc of `hardmoney::parser::review::Concern`; this
is the short form.

| Concern | Fires when | Basis |
|---|---|---|
| `employer_occupation_missing` | A Schedule A line from an individual (`entity_type` `IND`, not a memo) whose aggregate exceeds $200 has a blank employer or occupation, and the other field does not say retired, not employed, homemaker, or student. The aggregate is the larger of the filer's `contribution_aggregate` and the sum of the contributor's counted lines in the file (and in prior reports, with `--with-prior`). | 52 U.S.C. 30104(b)(3)(A); 11 CFR 104.3(a)(4)(i) and 100.12; spec Schedule A rows 24-25, "Req if Donor aggregate >$200" |
| `best_efforts_claimed` | The same, but the field carries the best-efforts language ("Information Requested", "Information Requested per best efforts"). Informational: the filer's claim of compliance. | 11 CFR 104.7(b) |
| `over_limit_aggregate` | An individual's contributions exceed the limit: per election for a candidate committee (Forms 3 and 3P; per `election_code`, redesignation and reattribution memo entries netted), per calendar year for a PAC, state party, or national party. The amount is the excess. | 52 U.S.C. 30116(a); 11 CFR 110.1(b), (c), (d), 110.17; the limits table below |
| `date_outside_coverage` | A counted, positive Schedule A or B line is dated after `coverage_through_date`. Dates before the period are not flagged (see the evaluation). | 11 CFR 104.3(a)(4), (b)(4); Form 3X instructions (date of receipt) |
| `treasurer_signature_missing` | The cover has no treasurer name or no `date_signed`. | 11 CFR 104.14(a), 104.18(g) |
| `memo_double_count` | A counted positive Schedule A line back-references another counted Schedule A line: an earmark or itemization under a total that is itself counted. | 11 CFR 110.6(c)(1); Form 3X instructions on memo entries |
| `memo_without_parent` | A memo entry references a transaction id not in the file, or an individual's `SA11AI` memo has no back-reference and no reason in its text. Heuristic. | spec Schedule A row 4 |
| `cover_not_supported` | A cover line fails its rule in `reconcile`; one observation per failing line, the amount being the violation. With `--with-prior`, Column B against the period's schedule sums too. | FEC validation messages 3-4; [Reconciling a filing](./reconciling.md) |
| `negative_itemization` | A counted Schedule A or B line is negative and its text gives no reason (reattribution, redesignation, refund, returned check, void, chargeback, correction). | 11 CFR 103.3(b), 110.1(b)(5); the forms' instructions for refund lines |
| `unitemized_inconsistent` | Unitemized individual receipts are negative, or $25,000 or more with no individual contribution itemized. Heuristic. | 11 CFR 104.3(a)(3)(i) |
| `duplicate_transaction_id` | Two lines share a transaction id (from `validate`). | spec Schedule A row 3; FEC validation message 40 |
| `duplicate_transaction` | Two counted Schedule A lines of $200 or more share contributor (name and ZIP), date, and amount; with `--with-prior`, a counted line repeats one from an earlier report. Heuristic. | the forms' instructions: each contribution once |
| `chain_cash_mismatch` | Cash on hand at the beginning of the period differs from the prior report's close (or, on Form 3X, line 6(a) from the prior year's last report). `--with-prior` only. | Form 3X instructions, line 6(b) |

Concerns marked heuristic fire on accepted, un-lettered filings often
enough to be weighed rather than trusted; `Review::strict()` and the
`heuristic` flag on each observation separate them.

Two rules were narrowed by the evaluation below. `date_outside_coverage`
began by flagging dates on either side of the period; dates before it
turned out to be routine on reports that drew no letter (a conduit's
batch dated the last days of the previous month, a new committee's first
report reaching back before its registration; one clean May monthly had
634 April-dated receipts), so only dates after the period are flagged.
`memo_double_count` began by flagging any counted child of a counted
parent; a counted *negative* child is a chargeback that nets its parent,
so only positive children are flagged.

### Contribution limits

The over-limit check needs to know what kind of committee received the
money. A Form 3 or 3P is a candidate's committee. A Form 3X may be a PAC,
a state party, a national party, a joint fundraising committee, or an
independent-expenditure-only committee that may accept any amount, and
the file does not say which; a first draft that assumed a party limit
flagged 31 contributors on a joint fundraising committee's clean report.
So a Form 3X gets no limit check unless `--recipient` (or
`ReviewOptions::recipient`, or `Filing.review(recipient=...)`) says which;
the evaluation reads openFEC's `committee_type`. A national party
committee's Form 3X mixes its main account with the three additional
accounts of 52 U.S.C. 30116(a)(9), so `national-party` checks the four
limits together.

| Cycle | To a candidate, per election | To a PAC, per year | To a national party (main + 3 additional accounts), per year | To a state party, per year |
|---|---|---|---|---|
| 2019-2020 | $2,800 | $5,000 | $35,500 + 3 × $106,500 | $10,000 |
| 2021-2022 | $2,900 | $5,000 | $36,500 + 3 × $109,500 | $10,000 |
| 2023-2024 | $3,300 | $5,000 | $41,300 + 3 × $123,900 | $10,000 |
| 2025-2026 | $3,500 | $5,000 | $44,300 + 3 × $132,900 | $10,000 |

Source: the FEC's
[contribution limits](https://www.fec.gov/help-candidates-and-committees/candidate-taking-receipts/contribution-limits/)
page and its archived tables; `hardmoney::parser::review::CONTRIBUTION_LIMITS`
holds the same numbers. A report from a cycle without a row is not
checked. The cycle for a candidate's per-election limit comes from the
election code's year (`P2026`); the calendar-year limits use the report's
coverage year.

## Across reports

One file cannot check its own opening cash balance or its Column B, and
it does not know that a $150 contributor gave $100 last month. With the
committee's earlier reports on the same form:

```text
$ hardmoney review tests/fixtures/rad/F3XA_2011912.fec --recipient state-party \
    --with-prior tests/fixtures/chain/F3XN_1943038.fec tests/fixtures/chain/F3XN_1948502.fec \
                 tests/fixtures/chain/F3XA_2011895.fec tests/fixtures/chain/F3XA_2011898.fec \
                 tests/fixtures/chain/F3XA_2011901.fec
...
  cover_not_supported: 2 observation(s), 400.00 at issue
    line 2 200.00: column A line 11(c) reports 2045.00 but the rule = sum of SchA.contribution_amount on SA11C gives 1845.00; delta 200.00
    line 2 200.00: column B line 11(c) reports 42045.00 but the rule = sum of SchA.contribution_amount on SA11C over 5 report(s), year to date gives 41845.00 over 5 report(s); delta 200.00
...
```

The chain (`ReportChain`, from [Reconciling a filing](./reconciling.md#column-b-across-a-chain-of-reports))
adds `chain_cash_mismatch` for a broken carry-forward, Column B
`cover_not_supported`, `employer_occupation_missing` where the aggregate
crosses $200 only with earlier reports counted, per-election and
per-year sums over the period, and `duplicate_transaction` for a
contribution already reported in an earlier period. On the five
Minnesota reports every carry-forward holds; the only chain-level
observation is the same $200 in Column B.

## How well it agrees with RAD

The point of a review is to ask what RAD asks, so the checks were
measured against RAD's own letters. `hardmoney review --eval --cycle 2024
--sample 100` does the following, with an openFEC key:

1. Reads pages of `/filings/?form_type=FRQ&cycle=2024` spread over the
   whole list (page stride 46 of 187 pages, newest first), keeps letters
   with `request_type` 2 (a report), coverage dates, and a committee type
   that files Form 3, 3P, or 3X, and takes one letter per committee until
   it has 100.
2. For each letter, lists the committee's filings on that form for the
   cycle and picks the report with the letter's coverage period: the
   latest version received on or before the letter (the version RAD
   reviewed), else the earliest. The letters carry no file number, so
   this is the join; 2 of 100 found no match.
3. For each (form, report type) among those, lists other committees'
   most-recent filings of the same kind, spread over the list, and for
   each candidate fetches the committee's FRQ letters for the cycle. A
   report is "clean" if no letter names its coverage period; the review
   runs on its original version. The committee's total letter count is
   kept, so the metrics can also be cut at committee level (committees
   with no letter of any kind).
4. Reviews every report with the limit for its openFEC `committee_type`,
   and reports precision, recall, and F1 of "any observation" against
   "an RFAI names this report", overall and per concern, plus lift
   (precision over the base rate).

Run on 2026-09-16 against the 2024 cycle: 98 RFAI'd reports (61 F3X, 34
F3, 3 F3P; letters dated 2024-01-09 to 2026-06-29; coverage periods from
2023-01-01 to 2024-12-31) and 96 clean reports of the same forms and
report types, 257 openFEC requests, no download or parse failures. The
first pass used both directions of `date_outside_coverage` and the
broader `memo_double_count`; the numbers below are the shipped rules,
re-scored on the same 194 reports from the cache
(`hardmoney review --eval --rescore hardmoney-review-eval-2024.json`).

| Predicate | Clean set | tp | fp | fn | tn | Precision | Recall | F1 |
|---|---|---|---|---|---|---|---|---|
| any observation | report not named by a letter (96) | 36 | 34 | 62 | 62 | 51.4% | 36.7% | 42.9% |
| any non-heuristic observation | same | 21 | 15 | 77 | 81 | 58.3% | 21.4% | 31.3% |
| any observation | committee with no letter in the cycle (36) | 36 | 9 | 62 | 27 | 80.0% | 36.7% | 50.3% |
| any non-heuristic observation | same | 21 | 4 | 77 | 32 | 84.0% | 21.4% | 34.1% |

Per concern, over the report-level clean set (base rate 50.5%, so a
lift of 1.98 would mean every firing was on a lettered report):

| Concern | Fired on RFAI'd (of 98) | Fired on clean (of 96) | Precision | Recall | Lift |
|---|---|---|---|---|---|
| `over_limit_aggregate` | 14 | 6 | 70.0% | 14.3% | 1.39 |
| `negative_itemization` | 7 | 3 | 70.0% | 7.1% | 1.39 |
| `best_efforts_claimed` (heuristic) | 14 | 9 | 60.9% | 14.3% | 1.20 |
| `duplicate_transaction` (heuristic) | 26 | 17 | 60.5% | 26.5% | 1.20 |
| `employer_occupation_missing` | 6 | 5 | 54.5% | 6.1% | 1.08 |
| `cover_not_supported` | 2 | 3 | 40.0% | 2.0% | 0.79 |
| `memo_without_parent` (heuristic) | 0 | 3 | 0% | 0% | 0 |
| `unitemized_inconsistent` (heuristic) | 0 | 2 | 0% | 0% | 0 |
| `date_outside_coverage` | 0 | 0 | n/a | 0% | n/a |
| `treasurer_signature_missing`, `memo_double_count`, `duplicate_transaction_id` | 0 | 0 | n/a | 0% | n/a |

What the numbers say, and what they cannot:

- The signal is real but modest. Reports that drew a letter are more
  likely to carry an observation (37% against 35% of the report-level
  clean set, 25% of the committee-level one), and the non-heuristic
  concerns are more selective than the heuristic ones. On the 36
  committees that received no letter of any kind, four in five reports
  with an observation were lettered reports.
- Recall is low because most letters are about things a file cannot
  show. RAD's letters also cover the adequacy of purpose-of-disbursement
  text (the FEC publishes a list of inadequate purposes), 48-hour
  notices the committee failed to file, disclaimers, registration
  details, debts carried without a Schedule D, excessive contributions
  visible only across the contributor's history, and questions raised by
  the committee's own earlier answers. 62 of the 98 lettered reports had
  no observation at all.
- The report-level clean set is not clean. A report no letter names may
  still have a problem RAD chose to raise on a neighbouring report of
  the same committee (the letters read "on a per report basis", but
  analysts batch), and 60 of the 96 clean reports came from committees
  that received at least one letter in the cycle. That is why the
  committee-level cut has higher precision with the same recall. One
  clean Form 3X carried 25 individuals over $200 with blank employer and
  occupation; its committee had four letters that cycle.
- The join is by coverage period, not by file number, because the
  letters carry none. Where a committee amended a report before the
  letter, the amended version is what was reviewed here.
- One cycle, one sample of 100 and 100, one day. The numbers are a
  measurement of these rules on that sample, not a property of RAD's
  policy; a second cycle or a larger sample can be run with the command
  above (about 2.6 requests per report against a 1,000-an-hour key).

The JSON the command writes holds every report's file number, committee,
type, coverage, label, letter date, and per-concern counts, so any row
can be re-examined with `hardmoney review <file number>`.

## The library

```rust
use hardmoney::Filing;
use hardmoney::parser::review::{Concern, Recipient, ReviewOptions};

let filing = Filing::open("tests/fixtures/rad/F3XA_2011912.fec")?;
let review = filing.review();                    // limits implied by the form
for o in review.by_concern(Concern::CoverNotSupported) {
    println!("{}: {} {:?}", o.line_no.unwrap_or(0), o.detail, o.amount);
}
let as_party = filing.review_with(&ReviewOptions::new().recipient(Recipient::StateParty));
println!("{}", as_party.summary.observations);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`Review` holds `observations` (each with `concern`, `line_no`,
`transaction_id`, `amount`, `detail`, `rfai_request_type`) and a
`summary` (per-concern counts and amounts, the total at issue); it
implements `Display` (one observation per line, then a summary line) and
`Serialize`. `ReportChain::review` takes the chain from
`ReportChain::new`. Nothing in the module can fail on a parsed filing.

In Python, `Filing.review(recipient=None)` returns a `Review` with the
same fields, `Decimal` amounts, `len()`, iteration, `str()`, a
`summary` dict, and `by_concern("...")`; see the
[Python API reference](./python-api.md#review).
