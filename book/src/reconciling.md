# Reconciling a filing

A periodic report has two halves that are supposed to agree. The cover
page (line 2 of the file, `filing.summary`) states totals: "Line
11(a)(i), itemized contributions from individuals, this period:
$13,736.02." The schedules that follow itemize them, one line per
transaction. Every cover-page total is defined either as the sum of
particular schedule lines or as arithmetic over other cover-page lines,
and checking that those definitions hold is the first pass a Reports
Analysis Division analyst makes on every report the FEC receives.

`Filing::reconcile` and `hardmoney reconcile` do that pass: recompute
every line with exact `Decimal` arithmetic, and report `reported`,
`expected`, and `delta` for each. The FEC's own validator only warns
about a summary that does not match its schedules ("Subtotal ... not
supported by Schedule"), and the FEC's new filing tool computes several
lines as zero, so this is a check filers and analysts have mostly done
by hand.

## The command

```text
$ hardmoney reconcile --help
Recompute a report's cover-page totals from its schedules and show every line that disagrees

Usage: hardmoney reconcile [OPTIONS] <PATH>

Arguments:
  <PATH>  Path to a `.fec` file (Form 3X, 3, or 3P)

Options:
      --json                   Emit the full check list as JSON
      --all                    Show every line, not just the ones that disagree
      --tolerance <TOLERANCE>  Treat violations up to this amount as agreeing (e.g. 0.01 for a filer who rounds each line). A violation is the absolute delta for an `=` line, or the shortfall below the itemized sum for a `>=` line [default: 0]
      --column <COLUMN>        Only check this column (A = this period, B = year/cycle to date) [possible values: a, b]
      --lenient                Skip body lines that cannot be parsed instead of failing
  -h, --help                   Print help
```

By default it prints only the lines that disagree, then a one-line
verdict. On a filing that balances, that is just the verdict:

```text
$ hardmoney reconcile tests/fixtures/F3XN_2011831.fec
F3X C00140855: 0 of 69 line(s) disagree (tolerance 0)
```

`--all` shows every check. Here are the first twenty Column A lines of
that Form 3X (FirstEnergy Corp PAC, a 144-line monthly report):

```text
$ hardmoney reconcile --all --column a tests/fixtures/F3XN_2011831.fec | head -20
ok   col A line 9          reported            0.00 expected               0 delta            0  = sum of SchC.loan_balance on SC/9 + SchD.balance_at_close_this_period on SD9
ok   col A line 10         reported            0.00 expected               0 delta            0  = sum of SchC.loan_balance on SC/10 + SchD.balance_at_close_this_period on SD10
ok   col A line 11(a)(i)   reported        13736.02 expected        13736.02 delta         0.00  = sum of SchA.contribution_amount on SA11AI/SA11A1
ok   col A line 11(a)(iii) reported        15245.52 expected        15245.52 delta         0.00  = 11(a)(i) + 11(a)(ii)
ok   col A line 11(b)      reported            0.00 expected               0 delta            0  = sum of SchA.contribution_amount on SA11B
ok   col A line 11(c)      reported            0.00 expected               0 delta            0  = sum of SchA.contribution_amount on SA11C
ok   col A line 11(d)      reported        15245.52 expected        15245.52 delta         0.00  = 11(a)(iii) + 11(b) + 11(c)
ok   col A line 12         reported            0.00 expected               0 delta            0  = sum of SchA.contribution_amount on SA12
ok   col A line 13         reported            0.00 expected               0 delta            0  = sum of SchA.contribution_amount on SA13
ok   col A line 14         reported            0.00 expected               0 delta            0  = sum of SchA.contribution_amount on SA14
ok   col A line 15         reported            0.00 expected               0 delta            0  >= sum of SchA.contribution_amount on SA15
ok   col A line 16         reported            0.00 expected               0 delta            0  = sum of SchA.contribution_amount on SA16
ok   col A line 17         reported            0.00 expected               0 delta            0  >= sum of SchA.contribution_amount on SA17
ok   col A line 18(a)      reported            0.00 expected               0 delta            0  = sum of H3.transferred_amount on H3
ok   col A line 18(b)      reported            0.00 expected               0 delta            0  = sum of H5.total_amount_transferred on H5
ok   col A line 18(c)      reported            0.00 expected            0.00 delta         0.00  = 18(a) + 18(b)
ok   col A line 19         reported        15245.52 expected        15245.52 delta         0.00  = 11(d) + 12 + 13 + 14 + 15 + 16 + 17 + 18(c)
ok   col A line 20         reported        15245.52 expected        15245.52 delta         0.00  = 19 - 18(c)
ok   col A line 21(a)(i)   reported            0.00 expected               0 delta            0  = sum of H4.federal_share on H4
ok   col A line 21(a)(ii)  reported            0.00 expected               0 delta            0  = sum of H4.nonfederal_share on H4
```

Each row is one `LineCheck`: status (`ok` or `DIFF`), the column, the
FEC's line label, what the cover page says, what the schedules or
formula say it should be, `reported - expected`, and the rule in the
FEC's own notation. Read the rules column and you can see the three
kinds of rule the rest of this chapter explains: `= sum of ...` (an
exact schedule sum), `>= sum of ...` (a floor), and `= 11(a)(i) +
11(a)(ii)` (a formula over other cover lines).

## Schedule sums exclude memo entries

Line 11(a)(i) above is the sum of `contribution_amount` over the body
lines whose form-type token is `SA11AI` (or `SA11A1`, the spelling
spec 3.x-5.x used), excluding memo entries. A Schedule A line with
`memo_code = X` is informational: the earmark behind a conduit
contribution, the individual behind a partnership's gift, a
reattribution. Its amount is already counted on another line, so
including it double-counts. Forgetting this is the single most common
reason a recomputed total disagrees with a filing, and
`ParsedLine::is_memo()` (case-insensitive, trimmed) is the test the
reconciler uses.

## `Relation`: `=` and `>=`, and the $200 threshold

Federal law requires a committee to itemize a receipt or disbursement
only once the aggregate from that contributor (or to that payee) exceeds
$200 in the election cycle (11 CFR 104.3). Anything smaller is included
in the cover-page total but need not appear on the schedule at all. So
for those lines the schedule sum is a floor, not an identity: a cover
total below its itemized sum is a discrepancy (money on the schedule
that is not in the total), a cover total above it is normal.

Lines that must be itemized regardless of size (contributions from
committees, transfers, loans and repayments, coordinated party
expenditures, refunds to committees, debts, the allocation schedules)
must match exactly. `Relation` is that distinction, and which line is
which is taken from the FEC's own Form 3X, 3, and 3P instructions,
whose text for each line says either "must be itemized ... regardless
of the amount" or "aggregating in excess of $200":

| `Relation` | Check | Written as | Form 3X lines |
|---|---|---|---|
| `Equal` | `reported == expected` | `= sum of ...` | 9, 10, 11(a)(i), 11(b), 11(c), 12, 13, 14, 16, 18(a), 18(b), 21(a)(i), 21(a)(ii), 22, 23, 25, 26, 27, 28(b), 28(c), 30(a)(i), 30(a)(ii) |
| `AtLeast` | `reported >= expected` | `>= sum of ...` | 15 (offsets to expenditures), 17 (other receipts), 21(b) (operating expenditures), 24 (independent expenditures), 28(a) (refunds to individuals), 29 (other disbursements), 30(b) (100%-federal election activity) |

Two of those floors are easy to get wrong. Line 24: Schedule E itemizes
payees "aggregating in excess of $200", and the paper form has a
"Line (b)" for the unitemized remainder that the electronic `SE` record
has no field for, so the cover total legitimately exceeds the `SE` sum.
Line 30(b): "Itemize all such disbursements of $200 or more on Schedule
B for Line 30(b)" (and 11 CFR 300.36(b)(2)(iv)); seven of the 45
state-party reports described below have a 30(b) total above their
`SB30B` sum, by $5 to $336.

Form 3 has six floors (11(d) contributions from the candidate, 14, 15,
17, 20(a), 21) and Form 3P ten (17(d) contributions from the candidate,
20(a), 20(b), 20(c), 21, 23, 25, 26, 28(a), 29): a candidate is a
"person" under 11 CFR 104.3(a)(4)(i), itemized only above $200, unlike
the candidate's loans. Every formula is `Equal`.

`LineCheck::violation()` folds the relation in: it is `|delta|` for an
`Equal` line and `max(0, -delta)` for an `AtLeast` line, so `matches()`
is `violation() == 0` and `--tolerance` compares against it.

Unitemized individual contributions (11(a)(ii) on Form 3X, 17(a)(ii)
on Form 3P) have no schedule by definition; they are the contributions
too small to itemize. They are inputs: no check of their own, but they
appear in formulas (`11(a)(iii) = 11(a)(i) + 11(a)(ii)`), which is how
they get checked.

## Formulas are evaluated over reported values

A formula line (`11(d) = 11(a)(iii) + 11(b) + 11(c)`, `8 = 6(d) - 7`)
is computed from the reported values of the lines it names, not from
their recomputed values. That is deliberate, and it is what makes a
reconciliation report readable: if 11(a)(i) disagrees with Schedule A
but `11(a)(iii) = 11(a)(i) + 11(a)(ii)` holds, the cover page is
internally consistent and the discrepancy is between the cover and the
schedule. If the formula also failed, the filer's arithmetic on the
cover is wrong too. One `DIFF` line means one problem, not a cascade.

The formulas come from the FEC's own format specification, whose `RULE
REFERENCE` column for each cover-page field is exactly this arithmetic.
You can read it for any form with `hardmoney spec fields`:

```text
$ hardmoney spec fields F3X --version 8.5 | grep -E "col_a_total_receipts |col_a_cash_on_hand_close|col_a_individuals_itemized|col_a_individual_contribution_total"
24   col_a_total_receipts                                amount         12                6(c) Total Receipts                              = 19
27   col_a_cash_on_hand_close_of_period                  amount         12                8. Cash on Hand at Close                         = 6d - 7
30   col_a_individuals_itemized                          amount         12                11(a)i Itemized                                  = Total on Sch A
32   col_a_individual_contribution_total                 amount         12                11(a)iii Total                                   = 11ai + 11aii
```

## Column A and Column B

Every periodic report has two columns: A, this reporting period, and
B, the calendar year to date (Form 3X) or election cycle to date
(Forms 3 and 3P). Column A is checked completely, schedule sums and
formulas. Column B is checked by formula only: its sums span every
prior report in the year or cycle, which one file cannot see, so its
schedule-sum lines are inputs and only the arithmetic among them is
verified (`11(d) = 11(a)(iii) + 11(b) + 11(c)` must hold in Column B
too). `--column a` / `--column b` restrict the output; the 69 checks
above are 49 in Column A and 20 in Column B.

Lines a filing's spec version does not carry are omitted rather than
reported as blank: a 2001 filing at spec 3.00 has no line 17(a)(i), so
there is no check for it.

## A filing that does not balance

Here is a real one: an amended Form 3X filed at spec 8.5 by a state
party committee, 1,232 body lines. (`--lenient` is a habit worth having
on unfamiliar files; nothing was skipped here.)

```text
$ hardmoney reconcile --lenient tmp/agent-misc/filings/2011912.fec
DIFF col A line 11(c)      reported         2045.00 expected         1845.00 delta       200.00  = sum of SchA.contribution_amount on SA11C
F3X C00001313: 1 of 69 line(s) disagree (tolerance 0)
error: 1 line(s) disagree
```

Exit status 1. Line 11(c) is contributions from other political
committees (fully itemizable, so an `=` line), and the cover page says
$2,045.00 while the five `SA11C` lines in the file (four candidate
committees, amounts $300 to $500) sum to $1,845.00. The gap is exactly
$200.00: one contribution is on the cover and not on the schedule, or
the cover is over by one. Meanwhile every formula that includes 11(c)
(`11(d)`, `19`, `20`, `33`, `6(c)`, `6(d)`, `8`) is `ok`, so the cover
page is arithmetically consistent with itself; the $2,045.00 was carried
into every total. That is the shape of a filer discrepancy, and it is
what an analyst would write to the committee about.

`--json` gives the same checks as data:

```text
$ hardmoney reconcile --json --lenient tmp/agent-misc/filings/2011912.fec
{
  "form": "F3X",
  "file": "tmp/agent-misc/filings/2011912.fec",
  "tolerance": "0",
  "checks": 69,
  "disagreeing": 1,
  "lines": [
    {
      "line": "11(c)",
      "field": "col_a_pac_contributions",
      "column": "A",
      "rule": "= sum of SchA.contribution_amount on SA11C",
      "reported": "2045.00",
      "expected": "1845.00",
      "delta": "200.00",
      "relation": "equal",
      "lines_summed": 5,
      "reported_unparseable": false
    }
  ]
}
error: 1 line(s) disagree
```

With `--all`, `lines` holds every check. Amounts are strings, because
they are `Decimal`s and JSON numbers are floats.

## Tolerance

Comparisons are exact by default, to the cent, because the data is.
Some filers round each cover line independently, which leaves
one-cent disagreements that are noise; `--tolerance 0.01` (or
`Reconciliation::mismatches_over(dec)`) treats any violation up to that
amount as agreement. It is a threshold on `violation()`, so a floor line
that is over its itemized sum is never a violation whatever the
tolerance.

```text
$ hardmoney reconcile --tolerance 200 --lenient tmp/agent-misc/filings/2011912.fec
F3X C00001313: 0 of 69 line(s) disagree (tolerance 200)
```

That silences the $200 gap above, which is why a tolerance larger than
a rounding error should be a conscious choice, not a default.

## Which forms, and where the rules come from

Rules exist for Form 3X (PACs and party committees), Form 3 (House and
Senate candidates), and Form 3P (presidential candidates). Any other
cover form is `ReconcileError::UnsupportedForm`:

```text
$ hardmoney reconcile tests/fixtures/F24N_2011832.fec
error: no reconciliation rules for form F24; supported: F3X, F3, F3P
```

The rule tables live in `src/parser/reconcile.rs`, declared in a small
macro DSL that reads like the FEC's own notation:

```rust,ignore
pub static F3X_RULES: &[LineRule] = rules! {
    A:
        "6(b)"       col_a_cash_on_hand_beginning_period { input };
        "11(a)(i)"   col_a_individuals_itemized          { <- SchA["SA11AI", "SA11A1"].contribution_amount };
        "11(a)(ii)"  col_a_individuals_unitemized        { input };
        "11(a)(iii)" col_a_individual_contribution_total { = "11(a)(i)" + "11(a)(ii)" };
        "21(b)"      col_a_other_federal_operating_expenditures { >= SchB["SB21B"].expenditure_amount };
        "9"          col_a_debts_to { <- SchC["SC/9"].loan_balance  SchD["SD9"].balance_at_close_this_period };
        "8"          col_a_cash_on_hand_close_of_period  { = "6(d)" - "7" };
        // ...
    B:
        "11(a)(iii)" col_b_individual_contribution_total { = "11(a)(i)" + "11(a)(ii)" };
        // ...
};
```

Each entry is the FEC line label, the canonical cover-page field that
holds it, and a source: `<-` an exact schedule sum (`Relation::Equal`),
`>=` a floor (`Relation::AtLeast`), `=` a formula over other lines, or
`{ input }` for a line with no rule of its own. A source may name
several schedules: debts (lines 9 and 10) are a Schedule C loan balance
plus a Schedule D debt balance.

Two sources fed those tables. The first is the FEC's format
specification. The `RULE REFERENCE` column of each form's sheet is the
formula text shown by `hardmoney spec fields` above, and the
schedule-sum lines say `= Total on Sch A`. Form 3 and Form 3P are taken
from their sheets.

The second is FECfile+, the FEC's own open-source filing tool
(`fecfile-web-api`), whose `reports/form_3x/summary.py` computes Form
3X Column A. Its test suite asserts the totals its calculator produces
for a fixed set of transactions, and hardmoney encodes that as an oracle
test: the same transactions, parsed as a filing, must reconcile to the
same values on every Column A line. They do. Where FECfile+ stubs a line
to zero (18(c), 21(a)(i), 21(a)(ii), 30(a)(i), 30(a)(ii), the shared
federal/non-federal allocation lines fed by Schedules H3-H6), hardmoney
implements the spec's rule text instead. FECfile+ computes Form 3 as all
zeros, so those rules come from the spec alone.

The third is the FEC's form instructions, for the exact-versus-floor
decision on each line, as the table above describes.

## The allocation schedules, checked against party committees

The H-schedule lines were the one part of the tables written from the
spec text alone, because no filing in the original corpus carried a
Schedule H. Party committees with a nonfederal account do: in September
2026, 45 recent Form 3X reports from 24 state parties (both parties;
Ohio, Florida, Michigan, Wisconsin, Pennsylvania, North Carolina,
Arizona, Nevada, Georgia, California, Texas, Minnesota, Virginia, Iowa,
Colorado, New Hampshire, Maine), with between 3 and 648 H4 records each
and up to 33 H3 records, were fetched with `hardmoney filings --fetch`
and reconciled. All 45 balance on every line. (With 30(b) still coded
as an identity, seven had differed there, each with a cover total above
its itemized sum -- the shape of a $200 threshold, and the evidence that
made it a floor.) What the records settled:

- **18(a) sums H3's `transferred_amount`, not
  `total_amount_transferred`.** A transfer from the nonfederal account
  is one `AD` (administrative) record plus a record per other event
  type (`DF` direct fundraising, `DC` direct candidate support, ...)
  back-referencing it. Every record in the group repeats the transfer's
  total; each carries its own share. From the Minnesota DFL's May 2026
  report:

  ```text
  H3 | 4948AD | 4948AD | MN DFL State Checking | AD |                             | 20260527 | 46102.21 | 44060.51
  H3 | 11388Q | 4948AD | MN DFL State Checking | DF | 2026 Humphrey Mondale Dinner | 20260527 | 46102.21 |  2041.70
  ```

  Summing the totals would count that $46,102.21 twice.
- **21(a)(i) and 21(a)(ii) sum H4's `federal_share` and
  `nonfederal_share` with memo entries excluded.** 4,065 of the H4
  records in the sample were memos (credit-card and payroll breakdowns
  back-referencing a parent record). No report carried an `SB21A`
  line: Schedule H4 is the only itemization of 21(a).
- **30(b) is a floor** (above).
- **H5 and H6 appear in none of the 45 reports**, nor in the 24 national
  party reports (RNC, DNC, NRCC, DCCC, DSCC, NRSC), which carry no H
  schedules at all: BCRA bars national parties from nonfederal accounts,
  and Levin funds have all but vanished since 2002. Lines 18(b),
  30(a)(i), and 30(a)(ii) therefore rest on the workbook's layout: H5 is
  one record per transfer with a total and four category breakdowns (the
  FEC's warning #49 is the check that they agree), and H6 mirrors H4 with
  `federal_share` / `levin_share`.

Three of the party reports are now fixtures (`F3XA_2011814.fec`,
`F3XN_1998773.fec`, `F3XA_2008083.fec`) with tests that assert the
H3/H4 sums are non-zero, summed over the right number of records, and
exact -- and that 30(b) is a floor on real data.

## How real filings fare

Every Form 3X, 3, and 3P fixture in `tests/fixtures/` (all accepted by
the FEC, spec 3.00 through 8.5, now including a House amendment with 58
Schedule C loans and 3 Schedule D debts, a joint fundraising committee
with 106 `SB22` transfers out, and a 2026 presidential amendment at
spec 8.5) satisfies every Column A rule and every Column B formula
(`tests/reconcile_fixtures.rs`). Across the wider local corpus of 124
real periodic reports, 102 satisfy every rule; 17 of the other 22 are
truncated or hand-edited third-party samples (a filing cut off
mid-schedule cannot balance) and five are single-line filer
discrepancies of the kind shown above. Of the 98 reports fetched fresh
from the FEC for this check -- national and state parties, House and
Senate candidates with debts, joint fundraising committees, presidential
campaigns -- 96 balance on every line; the two that do not are a
twelve-cent gap on the NRSC's line 12 across 57,152 `SA12` records and a
$91.96 shortfall on a termination report's operating expenditures. Every
disagreement is written up in `tests/fixtures/ORACLE_NOTES.md`. The rule
set is tight enough that when it flags a line, the line is worth a look.

## From Rust

`Filing::reconcile` returns a `Reconciliation` (the `form` and a `Vec`
of `LineCheck`s) or `ReconcileError::UnsupportedForm`:

```rust
use hardmoney::Filing;
use hardmoney::parser::reconcile::{Column, Relation};

let filing = Filing::open("tests/fixtures/F3XN_2011831.fec")?;
let r = filing.reconcile()?; // Err(ReconcileError::UnsupportedForm) for anything but F3X/F3/F3P

println!("{} checks; balances: {}", r.checks.len(), r.balances());
for c in r.mismatches() {
    println!("{c}");
}

let itemized = r.line(Column::A, "11(a)(i)").unwrap();
assert_eq!(itemized.relation, Relation::Equal);
println!(
    "11(a)(i): reported {:?}, expected {} from {} non-memo SA11AI line(s)  [{}]",
    itemized.reported, itemized.expected, itemized.lines_summed, itemized.rule
);

let opex = r.line(Column::A, "21(b)").unwrap();
assert_eq!(opex.relation, Relation::AtLeast);
println!("21(b): {}", opex);

let cash = r.line(Column::A, "8").unwrap();
println!("8: {}", cash.rule);
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
69 checks; balances: true
11(a)(i): reported Some(13736.02), expected 13736.02 from 132 non-memo SA11AI line(s)  [= sum of SchA.contribution_amount on SA11AI/SA11A1]
21(b): ok   col A line 21(b)      reported          173.01 expected          173.01 delta         0.00  >= sum of SchB.expenditure_amount on SB21B
8: = 6(d) - 7
```

A `LineCheck` carries `line`, `field`, `column`, `rule` (the text shown
above), `reported: Option<Decimal>` (`None` if the cover field is blank,
counted as zero in `delta`), `expected`, `delta`, `relation`,
`lines_summed` (how many body lines contributed to a schedule sum), and
`reported_unparseable` (true if the cover field held something that is
not an amount). `matches()` and `violation()` apply the relation.
`Reconciliation` has `mismatches()`, `mismatches_over(tolerance)`,
`balances()`, `column(Column)`, and `line(Column, label)`; both types
implement `Display` (the CLI's text output) and, with the `serde`
feature, `Serialize` (its JSON).

Tolerance from Rust is the same threshold on `violation()`:

```rust
use hardmoney::Filing;
use rust_decimal::Decimal;

let filing = Filing::open("tests/fixtures/F3XN_2011831.fec")?;
let r = filing.reconcile()?;
let cent: Decimal = "0.01".parse()?;
let strict = r.mismatches().count();
let loose = r.mismatches_over(cent).count();
println!("{strict} exact mismatches, {loose} over one cent");
for c in r.mismatches_over(cent) {
    println!("{} {} violation {}", c.column, c.line, c.violation());
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
0 exact mismatches, 0 over one cent
```

And the rule tables themselves are public, if you want to list what is
checked or build your own report:

```rust
use hardmoney::Table;
use hardmoney::parser::reconcile::{Column, Source, rules_for};

let rules = rules_for(Table::F3X).unwrap();
let col_a = rules.iter().filter(|r| r.column == Column::A).count();
let col_b = rules.iter().filter(|r| r.column == Column::B).count();
println!("F3X: {col_a} Column A rules, {col_b} Column B rules");

for r in rules.iter().filter(|r| r.column == Column::A).take(12) {
    let kind = match r.source {
        Source::Schedules(..) => "schedules",
        Source::Formula(_) => "formula  ",
        Source::Input => "input    ",
        _ => "other    ",
    };
    println!("{:<11} {kind}  {}", r.line, r.formula_text());
}
assert!(rules_for(Table::F24).is_none());
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
F3X: 51 Column A rules, 49 Column B rules
9           schedules  = sum of SchC.loan_balance on SC/9 + SchD.balance_at_close_this_period on SD9
10          schedules  = sum of SchC.loan_balance on SC/10 + SchD.balance_at_close_this_period on SD10
11(a)(i)    schedules  = sum of SchA.contribution_amount on SA11AI/SA11A1
11(a)(ii)   input      (input)
11(a)(iii)  formula    = 11(a)(i) + 11(a)(ii)
11(b)       schedules  = sum of SchA.contribution_amount on SA11B
11(c)       schedules  = sum of SchA.contribution_amount on SA11C
11(d)       formula    = 11(a)(iii) + 11(b) + 11(c)
12          schedules  = sum of SchA.contribution_amount on SA12
13          schedules  = sum of SchA.contribution_amount on SA13
14          schedules  = sum of SchA.contribution_amount on SA14
15          schedules  >= sum of SchA.contribution_amount on SA15
```

(51 Column A rules minus the two inputs, 6(b) and 11(a)(ii), are the 49
Column A checks reported above; `Source` is `#[non_exhaustive]`, hence
the wildcard arm.)

Reconciliation is about whether the filing agrees with itself. Whether
it would be accepted (field lengths, dates, IDs, required fields) is the
next chapter, [Validating a filing](./validating.md). The FEC's
validator and this reconciler check different things, and a filing can
pass either while failing the other.
