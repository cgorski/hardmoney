# Golden fixture pack

Spec-8.5 `.fec` files with known contents and known expected outputs, for
anyone who writes or checks FEC electronic filings: FECfile+ QA, filing
vendors, and hardmoney's own tests. The FEC's own repositories contain no
`.fec` files; these are meant to fill that gap.

Every file here except this README is written by
`examples/golden_fixtures.rs`. Do not edit them by hand:
`tests/golden_fixtures.rs` regenerates the pack in memory and fails if a
committed byte differs. To change a fixture, change the generator, bump
`PACK_VERSION` if the change is deliberate, and run

```bash
cargo run --example golden_fixtures -- tests/fixtures/golden
cargo test --all-features --test golden_fixtures
```

## Files

| File | Form | Body lines | What it is |
|---|---|---|---|
| `f3x.fec` | F3XN | 38 | Form 3X quarterly with Schedules A, B, C, D, E, F, H3 and H4, built from FECfile+'s `test_calculate_summary_column_a` transaction set. Cover filled so every Column A and Column B line balances. |
| `f3.fec` | F3N | 6 | Form 3 (House) quarterly: SA11AI, SA11C, SB17, SB20A, a candidate loan (SC/9), a debt (SD10). Balanced. |
| `f3p.fec` | F3PN | 4 | Form 3P (presidential) quarterly: SA17A, SA18, SB23, SB28A. Balanced; lines 14 and 15 derived from Column B as the spec says. |
| `f24.fec` | F24N | 2 | Form 24 (48-hour) with two Schedule E lines. |
| `f1m.fec` | F1MN | 0 | Form 1M naming five candidates. |
| `f99.fec` | F99 | 0 | Form 99 with a two-paragraph `[BEGINTEXT]` block. |
| `f3x_cover_off_by_one_cent.fec` | F3XN | 38 | `f3x` with line 11(a)(i) reported as 10000.24. Validates clean; `reconcile` flags 11(a)(i) (+0.01) and 11(a)(iii) (-0.01). |
| `f3x_duplicate_transaction_id.fec` | F3XN | 38 | `f3x` with line 4 reusing line 3's transaction id. One `duplicate_transaction_id` error (FEC message #40). |
| `f3x_missing_required_field.fec` | F3XN | 38 | `f3x` with `contributor_last_name` blank on line 3. One `required_field_empty` error (FEC message #3). |

Sidecars, one set per fixture:

- `<name>.validation.json`: exactly what `hardmoney validate <name>.fec --json` prints from this directory.
- `<name>.reconciliation.json` (F3X, F3, F3P only): exactly what `hardmoney reconcile <name>.fec --json --all` prints.
- `f3x.expected_column_a.json`: every Column A line of `f3x.fec`, keyed `line_11ai`, `line_21b`, `line_6c` ... the way FECfile+'s `calculate_summary_column_a` keys its result, as strings with two decimals (a JSON number would be read back as a float). See below.
- `MANIFEST.json`: per fixture, the form, spec version, SHA-256, byte count, description, provenance, sidecar list, and the expected findings and reconciliation outcome; at the top, the pack version and seed.

`tests/golden_fixtures.rs` checks all of it against the committed files, and
`python/tests/test_compat.py` and `test_pytest_plugin.py` use the pack as
their oracle.

## The Form 3X and FECfile+

`f3x.fec` files the transactions FECfile+'s summary test
(`fecfiler/reports/form_3x/tests/test_summary.py::test_calculate_summary_column_a`,
public domain) creates in its database: the same tokens, amounts, and memo
flags, including the memo SA15 of 10000.23 and the memo SE of 57.00 that
both calculators must skip, the SC/9, SC/10, SD9, SD10 balances behind
lines 9 and 10, cash on hand January 1 of 61, and unitemized individual
receipts of 3.77 (an `SA11AII` transaction in FECfile+, a cover-page value
in a `.fec`).

On every line FECfile+ derives from Schedules A through F, `f3x.expected_column_a.json`
equals that test's assertions: `line_11ai` 10000.23, `line_11aii` 3.77,
`line_11b` 444.44, `line_11c` 555.55, `line_12` 1212.12, `line_13`
1313.13, `line_14` 1414.14, `line_15` 2125.79, `line_16` 16.00, `line_17`
1000.00, `line_21b` 150.00, `line_22` 22.00, `line_23` 14.00, `line_24`
151.00, `line_25` 133.00, `line_26` 44.00, `line_27` 31.00, `line_28a`
101.50, `line_28b` 201.50, `line_28c` 301.50, `line_28d` 604.50, `line_29`
201.50, `line_30b` 102.25, `line_9` and `line_10` 250.00, `line_20`
18085.17, `line_33` 11003.99, `line_35` 10399.49.

The file also carries what FECfile+'s calculator stubs to `Decimal(0)`
("Stubbed out until a future ticket"): two H3 records filing one 1000.00
nonfederal transfer as an `AD` share of 750.00 and a `DF` share of 250.00
(both repeating 1000.00 in `total_amount_transferred`), and one H4 allocated
disbursement of 1000.00 (federal 330.00, nonfederal 670.00) with a memo
breakdown of 600.00 that back-references it. From those:

| Key | Value | Rule |
|---|---|---|
| `line_18a` | 1000.00 | sum of H3 `transferred_amount`; summing `total_amount_transferred` would give 2000.00 |
| `line_18b` | 0.00 | sum of H5 `total_amount_transferred` (no H5 records) |
| `line_18c` | 1000.00 | 18(a) + 18(b) |
| `line_21ai` | 330.00 | sum of H4 `federal_share`, memo entries excluded |
| `line_21aii` | 670.00 | sum of H4 `nonfederal_share`, memo entries excluded |
| `line_30ai`, `line_30aii` | 0.00 | H6 shares (no H6 records) |

and so `line_19` and `line_6c` are 19085.17 rather than 18085.17,
`line_21c` 1150.00, `line_31` and `line_7` 2453.25, `line_32` 1783.25,
`line_36` 480.00, `line_38` -1645.79, `line_6d` 19146.17, `line_8`
16692.92. The JSON has two keys FECfile+'s dict lacks, `line_18a` and
`line_18b`, and the three `calculate_cash_on_hand_fields` adds
(`line_6b`, `line_6d`, `line_8`).

## WebCheck

Every fixture was submitted to the FEC's WebCheck (public upload channel,
`hardmoney validate --oracle webcheck`) on 2026-09-15:

| File | WebCheck result | Findings |
|---|---|---|
| `f3x.fec` | SUCCESS | none |
| `f3.fec` | SUCCESS | none |
| `f3p.fec` | SUCCESS | none |
| `f24.fec` | SUCCESS | none |
| `f1m.fec` | SUCCESS | none |
| `f99.fec` | SUCCESS | none |
| `f3x_cover_off_by_one_cent.fec` | WARNINGS | `Subtotal $10000.24 not supported by Schedule A FYI --> $10000.23 accumulated on Schedule A` |
| `f3x_duplicate_transaction_id.fec` | ERRORS | `Tran ID 'SA11AI.1' is NOT UNIQUE - This one is same as other(s)` |
| `f3x_missing_required_field.fec` | ERRORS | `Conditionally Required field is Empty` (SA11AI last name, required for an individual) |

The verdicts agree with `hardmoney validate` on every file. Re-run with
`HARDMONEY_NETWORK_TESTS=1 cargo test --all-features --test golden_fixtures -- --ignored --nocapture`.

WebCheck also taught the pack three things a first draft got wrong. It
checks every committee id in a filing, the filer's and the donors' and
beneficiaries', against the FEC's committee registry, so a fictitious id
such as `C00123456` fails with `ID# NOT Correct FEC ID# Format`. A Schedule
C `SC/10` loan on a Form 3X references summary line `13`, not `27`. An H3
direct-fundraising record needs an event name, and an SB23 contribution to
a candidate an election code.

## Provenance

The pack is synthetic. Committee names, treasurers, contributors, payees,
addresses, transaction ids, and dates are invented, and the `HDR` names the
generator (`hardmoney-golden`, version `1`) rather than any filing
software. The Form 3X amounts and line tokens are FECfile+'s public-domain
test data (17 U.S.C. 105). The `.fec` files and sidecars are offered under
the repository's licence (Apache-2.0 OR BSD-3-Clause).

Because of the registry check above, the committee ids are real: they
belong to committees that appear in the FEC's 1990 committee master file
and not in its 2024 one, chosen for that reason alone.

| Role in the pack | Id | Registry name |
|---|---|---|
| Filer of `f3x`, `f24`, `f1m`, `f99` ("Example PAC") | C00002253 | NATIONAL COUNCIL OF SAVINGS INSTITUTIONS (THRIFTPAC) |
| Affiliated PAC (SA12, SB22) | C00003459 | CALIFORNIA LEAGUE OF SAVINGS INSTITUTIONS FEDPAC |
| Other PAC (SA11C, SB28C) | C00003020 | CONNECTICUT MEDICAL POLITICAL ACTION COMMITTEE |
| Party committee (SA11B, SB28B) | C00078733 | ALEXANDRIA DEMOCRATIC FEDERAL CAMPAIGN COMMITTEE |
| Filer of `f3` ("Roe for Congress"); candidate committee on `f3x` (SA16, SB23, SB27) | C00015669 | WAMPLER FOR CONGRESS COMMITTEE |
| Filer of `f3p` ("Roe for President") | C00100834 | CRANE FOR PRESIDENT COMMITTEE |
| Exploratory committee (F3P SA18) | C00110601 | GRAYSON FOR PRESIDENT 1984 |

Nothing in any fixture describes those committees' activity. The
candidate ids (`H0VA01001` and the four on the Form 1M) are invented;
WebCheck does not check them.

## Determinism

The pack is a pure function of the generator's source and its constants,
not of the git commit or the crate version: the seed `hardmoney-golden/1`
is in `MANIFEST.json` and in every `HDR`. The sidecars do depend on the
current shape of `hardmoney validate --json` and `hardmoney reconcile
--json`; when either changes, the determinism test fails and the fix is to
regenerate, which is how a change in output format shows up in review.
