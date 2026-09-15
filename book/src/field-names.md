# Field names

Every value hardmoney parses is keyed by a canonical field name: the
`lower_snake_case` string in column 1 of `data/fec-csv-sources/<table>.csv`,
the one you pass to `line.get("contribution_amount")`, the one behind the
generated constant `tables::sch_a::CONTRIBUTION_AMOUNT`, the one that heads
a column in `hardmoney export`, and the one `LineCheck.field` and a
validation finding's `field` name. This chapter is the reference for those
names: the rules they follow, the concept table (one spelling per concept,
and which tables carry it), and an appendix listing every field of every
table with the FEC's own label.

The names descend from fech-sources, where each table was named on its
own. Before 3.0 the same concept was spelled differently from one table to
the next: `transaction_id` on Schedule A but `transaction_id_number` on
Schedule B; `memo_text` on H4 but `memo_text_description` on Schedule A;
Form 3 called line 11(a)(i) `col_a_individual_contributions_itemized` while
Form 3X called it `col_a_individuals_itemized`. Code that handled more than
one table had to try two names. 3.0 made one vocabulary; the map that did it
is `scripts/rename_canonical.py`, and a test in
`tests/format_table_integrity.rs` keeps the rules below true for any future
table change.

## The rules

1. **One spelling per concept, on every table.** If a field means the same
   thing on two kinds of record, it has the same name on both. Where the
   upstream tables disagreed, the shorter FEC-faithful spelling won
   (`transaction_id` over `transaction_id_number`; the FEC labels both
   "TRANSACTION ID NUMBER"), or the majority spelling where both were
   equally faithful (`expenditure_purpose_descrip`, the FEC's own
   abbreviation, on all seven tables).
2. **Suffixes are uniform.** ZIP fields end in `_zip_code` (`zip_code`,
   `contributor_zip_code`, `payee_zip_code`). Street fields end in
   `_street_1` / `_street_2`. Middle names end in `_middle_name`.
   Committee identifiers end in `_committee_id_number`. Dates are
   `<noun>_date` (`election_date`, `contribution_date`, `coverage_from_date`).
3. **Roles are prefixes.** The party on a schedule line is named by its
   role on that schedule: `contributor_*` (Schedule A, F56, F65, F92),
   `payee_*` (Schedule B, E, F, F57, F93, H4, H6), `lender_*`,
   `creditor_*`, `guarantor_*`, `conduit_*`, `donor_committee_*`,
   `beneficiary_candidate_*`. A role prefix is kept even where the FEC's
   label omits it, and a candidate or committee identifier without a role
   prefix (`candidate_id_number`, `candidate_name`) is "the candidate this
   record is about" (the supported/opposed candidate on Schedule E and
   F57, the filer on F2 and F2S).
4. **Cover-page lines keep `col_a_` / `col_b_` and the FEC's line
   semantics.** Column A is this period; Column B is year or cycle to date.
   Two forms share a name for a line only when the FEC's own labels
   describe the same line ("11(a)(i) Individuals Itemized" on Form 3 and
   "11(a)i Itemized" on Form 3X are both `col_a_individuals_itemized`). A
   line whose label genuinely differs keeps its form-specific name (Form
   3X's `col_a_total_loan_repayments_received`, Form 3P's
   `col_a_federal_funds`). Form 3's summary block and its detail block both
   have a "cash on hand at close" line, so it carries both
   `col_a_cash_on_hand_close_of_period` (line 8) and
   `col_a_cash_on_hand_close` (line 27), as do its F3Z schedules.
5. **Line-number-prefixed tables are their own convention.** F3S and F3PS
   (the per-election summary schedules) name fields by FEC line number,
   `11_a_i_individuals_itemized`, `20_d_total_offsets_to_operating_expenditures`,
   with no `col_a_` prefix because they have no Column A/B. They are left as
   they are. (The generated constants prefix these with `LINE_`:
   `f3s::LINE_11_A_I_INDIVIDUALS_ITEMIZED`.)
6. **A whole-name field and its split parts are different fields.**
   `treasurer_name` ("NAME/TREASURER (as signed)", spec 3.x to 5.x) and
   `treasurer_last_name` / `treasurer_first_name` / ... (6.x on) both exist
   on the cover pages; the typed views combine them for you
   (`combined_name`), the raw line keeps both.
7. **Nothing else is normalised.** The FEC's own abbreviations and typos
   survive where they are the only spelling (`calendar_y_t_d_per_election_office`,
   `item_contribution_aquired_date`, `loan_inccured_date_original`,
   `semi_annual_refunded_bundled_amt`); renaming them would trade one
   arbitrary string for another. Fields the FEC listed but no bundled
   version places (`agent_name` on F1S) get no constant.

## The concept table

The concepts that appear on more than one table and the one name each has.
"Tables" is where the field exists in at least one version bucket.

### Record identity

| Concept | Canonical name | Tables |
|---|---|---|
| record type token (column 1) | `form_type` | every table except HDR (`record_type`) and TEXT (`rec_type`; its `form_type` is the referenced form, spec 3.x to 5.3) |
| filer's committee id (column 2) | `filer_committee_id_number` | 56 tables (F2 and F2S have `candidate_id_number` there) |
| transaction id | `transaction_id` | F105 F132 F133 F3P31 F56 F57 F65 F76 F82 F83 F91 F92 F93 F94 H1 H2 H3 H4 H5 H6 SchA SchA3L SchB SchC SchC1 SchC2 SchD SchE SchF SchI SchL TEXT |
| back-referenced transaction id | `back_reference_tran_id` | F132 F133 F92 F93 F94 H3 H4 H6 SchA SchA3L SchB SchC1 SchC2 SchE SchF TEXT |
| back-referenced schedule | `back_reference_sched_name` | F132 F133 F92 F93 F94 H4 H6 SchA SchA3L SchB SchE SchF TEXT |
| entity type | `entity_type` | F132 F133 F3P31 F5 F56 F57 F65 F82 F83 F9 F92 F93 H4 H6 SchA SchA3L SchB SchC SchC1 SchC2 SchD SchE SchF |
| memo flag | `memo_code` | F132 F133 F3P31 H4 H6 SchA SchB SchC SchE SchF |
| memo text | `memo_text` | F132 F133 F3P31 H4 H6 SchA SchA3L SchB SchC SchE SchF |
| amended-record code (3.x to 5.x) | `amended_cd` | F105 F57 F82 F83 F91 H4 SchA SchB SchI |
| SI/SL account reference | `reference_code` | SchA SchA3L SchB |
| H2 event key | `activity_event_name` | H2 H3 |

### Elections and dates

| Concept | Canonical name | Tables |
|---|---|---|
| election code (P/G/S/R...) | `election_code` | F105 F3 F3P F3P31 F3X F5 F57 F76 F93 F94 SchA SchA3L SchB SchC SchE |
| "other" election description | `election_other_description` | F105 F57 F76 F93 F94 SchA SchA3L SchB SchC SchE |
| date of the report's election | `election_date` | F3 F3L F3P F3X F5 F7 |
| state of the report's election | `state_of_election` | F3 F3P F3X F5 F7 |
| candidate's election state / district (Form 3 header) | `election_state`, `election_district` | F3 F3L |
| coverage period | `coverage_from_date`, `coverage_through_date` | 16 tables |
| signature date | `date_signed` | 19 tables (`treasurer_date_signed` and `authorized_date_signed` on SchC1, which has two) |
| transaction dates | `contribution_date`, `expenditure_date`, `dissemination_date`, `disbursement_date`, `receipt_date` | by schedule |

### Parties to a transaction

| Concept | Canonical name | Tables |
|---|---|---|
| candidate id, no role | `candidate_id_number` | F1 F10 F2 F2S F3 F3P31 F56 F57 F6 F65 F76 F94 H4 H6 SchD SchE |
| candidate id, with role | `donor_candidate_fec_id`, `beneficiary_candidate_fec_id`, `lender_candidate_id_number`, `creditor_candidate_id_number`, `payee_candidate_id_number`, `affiliated_candidate_id_number` | SchA/SchA3L, SchB, SchC, F82/F83, SchF, F1/F1S |
| committee id, with role | `payee_committee_id_number` (F57 SchE SchF), `lender_committee_id_number`, `creditor_committee_id_number`, `subordinate_committee_id_number`, `designating_committee_id_number`, `affiliated_committee_id_number`, `authorized_committee_id_number`, `joint_fund_participant_committee_id_number`, `donor_committee_fec_id`, `beneficiary_committee_fec_id`, `contributor_fec_id` | by schedule |
| name parts | `<role>_organization_name`, `<role>_last_name`, `<role>_first_name`, `<role>_middle_name`, `<role>_prefix`, `<role>_suffix` | every party |
| whole name (3.x to 5.x) | `<role>_name` | every party |
| address | `<role>_street_1`, `<role>_street_2`, `<role>_city`, `<role>_state`, `<role>_zip_code` (the filer's own address is the bare `street_1` ... `zip_code`) | every party |
| employer / occupation | `<role>_employer`, `<role>_occupation` | SchA, SchA3L, SchC2, F56, F65, F91, F93, F9 |
| conduit | `conduit_name`, `conduit_street_1`, `conduit_street_2`, `conduit_city`, `conduit_state`, `conduit_zip_code` | F3P31 F56 F57 F65 H4 H6 SchA SchA3L SchB SchD SchE SchF |
| treasurer (cover pages) | `treasurer_last_name` ... `treasurer_suffix`; `treasurer_name` (3.x to 5.x) | F1 F10 F1M F24 F3 F3L F3P F3X F4 F8 F99 SchC1 |

### Amounts and purposes

| Concept | Canonical name | Tables |
|---|---|---|
| receipt amount | `contribution_amount` | F56 F65 F92 SchA |
| disbursement amount | `expenditure_amount` | F105 F57 F93 SchB SchE SchF |
| purpose text | `expenditure_purpose_descrip` (F57 F93 H4 H6 SchB SchE SchF), `contribution_purpose_descrip` (SchA SchA3L) | |
| purpose / category codes | `expenditure_purpose_code`, `contribution_purpose_code` (3.x to 5.x), `category_code` | by schedule |
| allocation shares | `federal_share`, `nonfederal_share` (H4), `levin_share` (H6), `total_amount` | H4 H6 |

### Cover-page lines shared across forms

The nine summary tables that share lines: F3 and its schedules (F3Z, F3Z1,
F3Z2), F3P and its schedules (F3PZ1, F3PZ2), F3X, F4, and the Levin account
summaries SchI and SchL. Each name below also has a `col_b_` twin where the
form has a Column B.

| Line (F3 / F3P / F3X numbering) | Canonical name | Tables |
|---|---|---|
| individuals itemized 11(a)(i) / 17(a)(i) / 11(a)(i) | `col_a_individuals_itemized` | F3 F3P F3X F3Z |
| individuals unitemized 11(a)(ii) / 17(a)(ii) / 11(a)(ii) | `col_a_individuals_unitemized` | F3 F3P F3X |
| individuals total 11(a)(iii) / 17(a)(iii) / 11(a)(iii) | `col_a_individual_contribution_total` | F3 F3P F3X |
| individuals, one line (Z schedules) | `col_a_individual_contributions` | F3PZ1 F3PZ2 F3Z1 F3Z2 |
| political party committees 11(b) / 17(b) / 11(b) | `col_a_political_party_contributions` | F3 F3P F3PZ1 F3PZ2 F3X F3Z F3Z1 F3Z2 |
| other political committees (PACs) 11(c) / 17(c) / 11(c) | `col_a_pac_contributions` | F3 F3P F3PZ1 F3PZ2 F3X F3Z F3Z1 F3Z2 |
| the candidate 11(d) / 17(d) | `col_a_candidate_contributions` | F3 F3P F3PZ1 F3PZ2 F3Z F3Z1 F3Z2 |
| total contributions 11(e) / 17(e) / 11(d) | `col_a_total_contributions` | F3 F3P F3PZ1 F3PZ2 F3X F3Z F3Z1 F3Z2 |
| transfers from other authorized committees 12 / 18 | `col_a_transfers_from_authorized` | F3 F3PZ1 F3PZ2 F3Z F3Z1 F3Z2 |
| transfers from affiliated/other party committees 18 / 12 | `col_a_transfers_from_aff_other_party_cmttees` | F3P F3X |
| loans from or guaranteed by the candidate 13(a) / 19(a) | `col_a_candidate_loans` | F3 F3P F3PZ1 F3PZ2 F3Z F3Z1 F3Z2 |
| other loans 13(b) / 19(b) | `col_a_other_loans` | F3 F3P F3PZ1 F3PZ2 F3Z F3Z1 F3Z2 |
| total loans 13(c) / 19(c) / 13 | `col_a_total_loans` | F3 F3P F3PZ1 F3PZ2 F3X F3Z F3Z1 F3Z2 |
| offsets to operating expenditures 14 / 20(a) | `col_a_offset_to_operating_expenditures` | F3 F3P F3PZ1 F3PZ2 F3Z F3Z1 F3Z2 |
| offsets to fundraising / legal 20(b) / 20(c) | `col_a_offset_to_fundraising_expenditures`, `col_a_offset_to_legal_expenditures` | F3P F3PZ1 F3PZ2 |
| total offsets 20(d) / 37 | `col_a_total_offsets_to_expenditures` | F3P F3PZ1 F3PZ2 F3X |
| other receipts 15 / 21 / 2 | `col_a_other_receipts` | F3 F3P F3PZ1 F3PZ2 F3Z F3Z1 F3Z2 SchL |
| total receipts 16 / 7 / 6(c) / 1 / 3 | `col_a_total_receipts` (and `_recap` for the second occurrence on F3P, F3X, F4) | F3 F3P F3PZ1 F3PZ2 F3X F3Z F3Z1 F3Z2 F4 SchI SchL |
| operating expenditures 17 / 23 | `col_a_operating_expenditures` | F3 F3P F3PZ1 F3PZ2 F3Z F3Z1 F3Z2 |
| transfers to other authorized committees 18 / 24 | `col_a_transfers_to_authorized` | F3 F3P F3PZ1 F3PZ2 F3Z F3Z1 F3Z2 |
| transfers to affiliated committees 22 | `col_a_transfers_to_affiliated` | F3X F4 |
| fundraising / exempt legal disbursements 25 / 26 | `col_a_fundraising_disbursements`, `col_a_exempt_legal_disbursements` | F3P F3PZ1 F3PZ2 |
| repayments of candidate loans 19(a) / 27(a) | `col_a_candidate_loan_repayments` | F3 F3P F3PZ1 F3PZ2 F3Z F3Z1 F3Z2 |
| other loan repayments 19(b) / 27(b) | `col_a_other_loan_repayments` | F3 F3P F3PZ1 F3PZ2 F3Z F3Z1 F3Z2 |
| total loan repayments 19(c) | `col_a_total_loan_repayments` | F3 F3Z F3Z1 F3Z2 |
| total loan repayments made 27(c) / 26 | `col_a_total_loan_repayments_made` | F3P F3PZ1 F3PZ2 F3X |
| refunds to individuals 20(a) / 28(a) | `col_a_refunds_to_individuals` | F3 F3P F3PZ1 F3PZ2 F3X F3Z F3Z1 F3Z2 |
| refunds to party committees 20(b) / 28(b) | `col_a_refunds_to_party_committees` | same |
| refunds to other committees 20(c) / 28(c) | `col_a_refunds_to_other_committees` | same |
| total refunds 20(d) / 28(d) | `col_a_total_refunds` | same |
| other disbursements 21 / 29 / 5 | `col_a_other_disbursements` | F3 F3P F3PZ1 F3PZ2 F3X F3Z F3Z1 F3Z2 SchI SchL |
| total disbursements 22 / 9 / 7 / 6 | `col_a_total_disbursements` (and `_recap`) | F3 F3P F3PZ1 F3PZ2 F3X F3Z F3Z1 F3Z2 F4 SchI SchL |
| cash on hand, beginning 23 / 6 / 6(b) / 7 | `col_a_cash_on_hand_beginning_period` | F3 F3P F3PZ1 F3PZ2 F3X F3Z F3Z1 F3Z2 F4 SchI SchL |
| subtotal 25 / 8 / 6(d) / 9 | `col_a_subtotal` | F3 F3P F3X F4 SchI SchL |
| cash on hand, close (summary block) 8 / 10 / 8 / 11 | `col_a_cash_on_hand_close_of_period` | F3 F3P F3PZ1 F3PZ2 F3X F4 SchI SchL |
| cash on hand, close (F3 detail block) 27 | `col_a_cash_on_hand_close` | F3 F3Z F3Z1 F3Z2 |
| debts owed to / by 9, 10 / 11, 12 | `col_a_debts_to`, `col_a_debts_by` | F3 F3P F3PZ1 F3PZ2 F3X F3Z F3Z1 F3Z2 F4 |
| net contributions / net operating expenditures 6(c), 7(c) / 14, 15 / 35, 38 | `col_a_net_contributions`, `col_a_net_operating_expenditures` | F3 F3P F3PZ1 F3PZ2 F3X F3Z F3Z1 F3Z2 |

## What changed in 3.0

The full map is `scripts/rename_canonical.py` (159 renames on 39 tables,
one family per concept with a comment on the rule that picked the winner)
and `NOTICE` lists every family. Running the script again reports "already
applied"; `python3 scripts/rename_canonical.py --appendix book/src/field-names.md`
regenerates the appendix below. The renames changed only column 1 of the
CSVs: no column position, label, or version bucket moved, so a filing parses
to the same values under the new keys.

## Appendix: every field, by table

Generated by `scripts/rename_canonical.py --appendix` from `data/fec-csv-sources/*.csv` and `data/fec-spec/spec-8.5.json`. The column is the 1-based position in the newest version bucket; blank means the field is placed only in an older or paper-format bucket. The label is the FEC's own (spec 8.5 where the table is still documented, otherwise the label from the format table; blank where the paper-format listings carry none).

### F1

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `change_of_committee_name` | CHANGE OF COMMITTEE NAME |
| 4 | `committee_name` | COMMITTEE NAME |
| 5 | `change_of_address` | CHANGE OF ADDRESS |
| 6 | `street_1` | STREET 1 |
| 7 | `street_2` | STREET 2 |
| 8 | `city` | CITY |
| 9 | `state` | STATE |
| 10 | `zip_code` | ZIP |
| 11 | `change_of_committee_email` | CHANGE OF COMMITTEE EMAIL |
| 12 | `committee_email` | COMMITTEE EMAIL |
| 13 | `change_of_committee_url` | CHANGE OF COMMITTEE WEB URL |
| 14 | `committee_url` | COMMITTEE WEB URL |
|  | `committee_fax_number` | COMMITTEE FAX NUMBER |
| 15 | `effective_date` | EFFECTIVE DATE |
|  | `signature_name` | NAME/TREASURER (as signed) |
| 16 | `signature_last_name` | SIGNATURE LAST NAME |
| 17 | `signature_first_name` | SIGNATURE FIRST NAME |
| 18 | `signature_middle_name` | SIGNATURE MIDDLE NAME |
| 19 | `signature_prefix` | SIGNATURE PREFIX |
| 20 | `signature_suffix` | SIGNATURE SUFFIX |
| 21 | `date_signed` | DATE SIGNED |
| 22 | `committee_type` | 5. COMMITTEE TYPE |
| 23 | `candidate_id_number` | 5. CANDIDATE ID NUMBER |
|  | `candidate_name` | 5. CANDIDATE NAME |
| 24 | `candidate_last_name` | 5. CANDIDATE LAST NAME |
| 25 | `candidate_first_name` | 5. CANDIDATE FIRST NAME |
| 26 | `candidate_middle_name` | 5. CANDIDATE MIDDLE NAME |
| 27 | `candidate_prefix` | 5. CANDIDATE PREFIX |
| 28 | `candidate_suffix` | 5. CANDIDATE SUFFIX |
| 29 | `candidate_office` | 5. CANDIDATE OFFICE |
| 30 | `candidate_state` | 5. CANDIDATE STATE |
| 31 | `candidate_district` | 5. CANDIDATE DISTRICT |
| 32 | `party_code` | 5. PARTY CODE |
| 33 | `party_type` | 5. PARTY TYPE |
| 34 | `organization_type` | 5 (e). ORGANIZATION TYPE |
| 35 | `lobbyist_registrant_pac` | 5 (e). LOBBYIST/REGISTRANT PAC |
| 36 | `lobbyist_registrant_pac_2` | 5 (f). LOBBYIST/REGISTRANT PAC |
| 37 | `leadership_pac` | 5 (f). LEADERSHIP PAC |
| 38 | `lobbyist_registrant_pac_3` | 5 (g). LOBBYIST/REGISTRANT PAC |
| 39 | `lobbyist_registrant_pac_4` | 5 (h). LOBBYIST/REGISTRANT PAC |
| 40 | `affiliated_committee_id_number` | 6. AFFILIATED CMTTE ID NUM |
| 41 | `affiliated_committee_name` | 6. AFFILIATED CMTTE NAME |
| 42 | `affiliated_candidate_id_number` | 6. AFFILIATED CANDIDATE ID NUM |
| 43 | `affiliated_last_name` | 6. AFFILIATED LAST NAME |
| 44 | `affiliated_first_name` | 6. AFFILIATED FIRST NAME |
| 45 | `affiliated_middle_name` | 6. AFFILIATED MIDDLE NAME |
| 46 | `affiliated_prefix` | 6. AFFILIATED PREFIX |
| 47 | `affiliated_suffix` | 6. AFFILIATED SUFFIX |
| 48 | `affiliated_street_1` | 6. AFFILIATED STREET 1 |
| 49 | `affiliated_street_2` | 6. AFFILIATED STREET 2 |
| 50 | `affiliated_city` | 6. AFFILIATED CITY |
| 51 | `affiliated_state` | 6. AFFILIATED STATE |
| 52 | `affiliated_zip_code` | 6. AFFILIATED ZIP |
| 53 | `affiliated_relationship_code` | 6. AFFILIATED RELATIONSHIP CODE (with Filing Committee named in field #4) |
|  | `custodian_name` | 7. IND/NAME  (Custodian Name) |
| 54 | `custodian_last_name` | 7. CUSTODIAN LAST NAME |
| 55 | `custodian_first_name` | 7. CUSTODIAN FIRST NAME |
| 56 | `custodian_middle_name` | 7. CUSTODIAN MIDDLE NAME |
| 57 | `custodian_prefix` | 7. CUSTODIAN PREFIX |
| 58 | `custodian_suffix` | 7. CUSTODIAN SUFFIX |
| 59 | `custodian_street_1` | 7. CUSTODIAN STREET 1 |
| 60 | `custodian_street_2` | 7. CUSTODIAN STREET 2 |
| 61 | `custodian_city` | 7. CUSTODIAN CITY |
| 62 | `custodian_state` | 7. CUSTODIAN STATE |
| 63 | `custodian_zip_code` | 7. CUSTODIAN ZIP |
| 64 | `custodian_title` | 7. CUSTODIAN TITLE |
| 65 | `custodian_telephone` | 7. CUSTODIAN TELEPHONE |
|  | `treasurer_name` | 8. IND/NAME  (Treasurer) |
| 66 | `treasurer_last_name` | 8. TREASURER LAST NAME |
| 67 | `treasurer_first_name` | 8. TREASURER FIRST NAME |
| 68 | `treasurer_middle_name` | 8. TREASURER MIDDLE NAME |
| 69 | `treasurer_prefix` | 8. TREASURER PREFIX |
| 70 | `treasurer_suffix` | 8. TREASURER SUFFIX |
| 71 | `treasurer_street_1` | 8. TREASURER STREET 1 |
| 72 | `treasurer_street_2` | 8. TREASURER STREET 2 |
| 73 | `treasurer_city` | 8. TREASURER CITY |
| 74 | `treasurer_state` | 8. TREASURER STATE |
| 75 | `treasurer_zip_code` | 8. TREASURER ZIP |
| 76 | `treasurer_title` | 8. TREASURER TITLE |
| 77 | `treasurer_telephone` | 8. TREASURER TELEPHONE |
|  | `agent_name` | 8. IND/NAME  (Designated Agent) |
| 78 | `agent_last_name` | 8. AGENT LAST NAME |
| 79 | `agent_first_name` | 8. AGENT FIRST NAME |
| 80 | `agent_middle_name` | 8. AGENT MIDDLE NAME |
| 81 | `agent_prefix` | 8. AGENT PREFIX |
| 82 | `agent_suffix` | 8. AGENT SUFFIX |
| 83 | `agent_street_1` | 8. AGENT STREET 1 |
| 84 | `agent_street_2` | 8. AGENT STREET 2 |
| 85 | `agent_city` | 8. AGENT CITY |
| 86 | `agent_state` | 8. AGENT STATE |
| 87 | `agent_zip_code` | 8. AGENT ZIP |
| 88 | `agent_title` | 8. AGENT TITLE |
| 89 | `agent_telephone` | 8. AGENT TELEPHONE |
| 90 | `bank_name` | 9. a) BANK NAME |
| 91 | `bank_street_1` | 9. a) BANK STREET 1 |
| 92 | `bank_street_2` | 9. a) BANK STREET 2 |
| 93 | `bank_city` | 9. a) BANK CITY |
| 94 | `bank_state` | 9. a) BANK STATE |
| 95 | `bank_zip_code` | 9. a) BANK ZIP |
| 96 | `bank2_name` | 9. b) BANK NAME |
| 97 | `bank2_street_1` | 9. b) BANK STREET 1 |
| 98 | `bank2_street_2` | 9. b) BANK STREET 2 |
| 99 | `bank2_city` | 9. b) BANK CITY |
| 100 | `bank2_state` | 9. b) BANK STATE |
| 101 | `bank2_zip_code` | 9. b) BANK ZIP |

### F10

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `committee_name` | FILER {Principal Campaign} COMMITTEE NAME |
| 4 | `street_1` | COMMITTEE STREET 1 |
| 5 | `street_2` | COMMITTEE STREET 2 |
| 6 | `city` | COMMITTEE CITY |
| 7 | `state` | COMMITTEE STATE |
| 8 | `zip_code` | COMMITTEE ZIP |
| 9 | `candidate_id_number` | CANDIDATE ID NUMBER |
| 10 | `candidate_last_name` | CANDIDATE LAST NAME |
| 11 | `candidate_first_name` | CANDIDATE FIRST NAME |
| 12 | `candidate_middle_name` | CANDIDATE MIDDLE NAME |
| 13 | `candidate_prefix` | CANDIDATE PREFIX |
| 14 | `candidate_suffix` | CANDIDATE SUFFIX |
| 15 | `candidate_office` | CANDIDATE OFFICE |
| 16 | `candidate_state` | CANDIDATE STATE |
| 17 | `candidate_district` | CANDIDATE DIST |
| 18 | `previous_expenditure_aggregate` | PREVIOUS EXPENDITURE AGGREGATE |
| 19 | `expenditure_total_this_report` | EXPENDITURE TOTAL THIS REPORT |
| 20 | `expenditure_total_cycle_to_date` | EXPENDITURE TOTAL CYCLE-TO-DATE |
| 21 | `meets_f6_filing_requirements` | MEETS F6 FILING FILING REQUIREMENTS |
| 22 | `candidate_employer` | CANDIDATE EMPLOYER |
| 23 | `candidate_occupation` | CANDIDATE OCCUPATION |
| 24 | `treasurer_last_name` | TREASURER LAST NAME |
| 25 | `treasurer_first_name` | TREASURER FIRST NAME |
| 26 | `treasurer_middle_name` | TREASURER MIDDLE NAME |
| 27 | `treasurer_prefix` | TREASURER PREFIX |
| 28 | `treasurer_suffix` | TREASURER SUFFIX |
| 29 | `date_signed` | DATE SIGNED |
|  | `candidate_name` | CANDIDATE NAME |
|  | `signer_name` | NAME/CANDIDATE (as signed) |

### F105

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `election_code` | ELECTION CODE |
| 5 | `election_other_description` | ELECTION OTHER DESCRIPTION |
| 6 | `expenditure_date` | EXPENDITURE DATE |
| 7 | `expenditure_amount` | EXPENDITURE AMOUNT |
| 8 | `loan_check` | LOAN CHECK |
|  | `item_elect_cd` | ITEM ELECT CD |
|  | `item_elect_other` | ITEM ELECT OTHER |
|  | `amended_cd` | AMENDED CD |

### F13

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `committee_name` | COMMITTEE NAME |
| 4 | `change_of_address` | CHANGE OF ADDRESS |
| 5 | `street_1` | STREET 1 |
| 6 | `street_2` | STREET 2 |
| 7 | `city` | CITY |
| 8 | `state` | STATE |
| 9 | `zip_code` | ZIP |
| 10 | `report_code` | REPORT CODE |
| 11 | `amendment_date` | AMENDMENT DATE |
| 12 | `coverage_from_date` | COVERAGE FROM DATE |
| 13 | `coverage_through_date` | COVERAGE THROUGH DATE |
| 14 | `total_donations_accepted` | 5 TOTAL DONATIONS ACCEPTED |
| 15 | `total_donations_refunded` | 6 TOTAL DONATIONS REFUNDED |
| 16 | `net_donations` | 7 NET DONATIONS |
| 17 | `designated_last_name` | DESIGNATED LAST NAME |
| 18 | `designated_first_name` | DESIGNATED FIRST NAME |
| 19 | `designated_middle_name` | DESIGNATED MIDDLE NAME |
| 20 | `designated_prefix` | DESIGNATED PREFIX |
| 21 | `designated_suffix` | DESIGNATED SUFFIX |
| 22 | `date_signed` | DATE SIGNED |

### F132

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `back_reference_tran_id` | BACK REFERENCE TRAN ID NUMBER |
| 5 | `back_reference_sched_name` | BACK REFERENCE SCHED NAME |
| 6 | `entity_type` | ENTITY TYPE |
| 7 | `contributor_organization_name` | CONTRIBUTOR ORGANIZATION NAME |
| 8 | `contributor_last_name` | CONTRIBUTOR LAST NAME |
| 9 | `contributor_first_name` | CONTRIBUTOR FIRST NAME |
| 10 | `contributor_middle_name` | CONTRIBUTOR MIDDLE NAME |
| 11 | `contributor_prefix` | CONTRIBUTOR PREFIX |
| 12 | `contributor_suffix` | CONTRIBUTOR SUFFIX |
| 13 | `contributor_street_1` | CONTRIBUTOR STREET 1 |
| 14 | `contributor_street_2` | CONTRIBUTOR STREET 2 |
| 15 | `contributor_city` | CONTRIBUTOR CITY |
| 16 | `contributor_state` | CONTRIBUTOR STATE |
| 17 | `contributor_zip_code` | CONTRIBUTOR ZIP |
| 18 | `donation_date` | DONATION DATE |
| 19 | `donation_amount` | DONATION AMOUNT |
| 20 | `donation_aggregate_amount` | DONATION AGGREGATE AMOUNT |
| 21 | `memo_code` | MEMO CODE |
| 22 | `memo_text` | MEMO TEXT/DESCRIPTION |

### F133

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `back_reference_tran_id` | BACK REFERENCE TRAN ID NUMBER |
| 5 | `back_reference_sched_name` | BACK REFERENCE SCHED NAME |
| 6 | `entity_type` | ENTITY TYPE |
| 7 | `contributor_organization_name` | CONTRIBUTOR ORGANIZATION NAME |
| 8 | `contributor_last_name` | CONTRIBUTOR LAST NAME |
| 9 | `contributor_first_name` | CONTRIBUTOR FIRST NAME |
| 10 | `contributor_middle_name` | CONTRIBUTOR MIDDLE NAME |
| 11 | `contributor_prefix` | CONTRIBUTOR PREFIX |
| 12 | `contributor_suffix` | CONTRIBUTOR SUFFIX |
| 13 | `contributor_street_1` | CONTRIBUTOR STREET 1 |
| 14 | `contributor_street_2` | CONTRIBUTOR STREET 2 |
| 15 | `contributor_city` | CONTRIBUTOR CITY |
| 16 | `contributor_state` | CONTRIBUTOR STATE |
| 17 | `contributor_zip_code` | CONTRIBUTOR ZIP |
| 18 | `refund_date` | REFUND DATE |
| 19 | `refund_amount` | REFUND AMOUNT |
| 20 | `memo_code` | MEMO CODE |
| 21 | `memo_text` | MEMO TEXT/DESCRIPTION |

### F1M

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `committee_name` | COMMITTEE NAME |
| 4 | `street_1` | STREET 1 |
| 5 | `street_2` | STREET 2 |
| 6 | `city` | CITY |
| 7 | `state` | STATE |
| 8 | `zip_code` | ZIP |
| 9 | `committee_type` | COMMITTEE TYPE |
| 10 | `affiliated_date_f1_filed` | AFFILIATED - DATE FORM F1 FILED |
| 11 | `affiliated_committee_id_number` | AFFILIATED COMMITTEE FEC ID |
| 12 | `affiliated_committee_name` | AFFILIATED COMMITTEE NAME |
| 13 | `first_candidate_id_number` | I CANDIDATE ID NUMBER |
|  | `first_candidate_name` | CANDIDATE NAME |
| 14 | `first_candidate_last_name` | I CANDIDATE LAST NAME |
| 15 | `first_candidate_first_name` | I CANDIDATE FIRST NAME |
| 16 | `first_candidate_middle_name` | I CANDIDATE MIDDLE NAME |
| 17 | `first_candidate_prefix` | I CANDIDATE PREFIX |
| 18 | `first_candidate_suffix` | I CANDIDATE SUFFIX |
| 19 | `first_candidate_office` | I CANDIDATE OFFICE |
| 20 | `first_candidate_state` | I CANDIDATE STATE |
| 21 | `first_candidate_district` | I CANDIDATE DIST |
| 22 | `first_candidate_contribution_date` | I DATE OF CONTRIBUTION |
| 23 | `second_candidate_id_number` | II CANDIDATE ID NUMBER |
|  | `second_candidate_name` | CANDIDATE NAME |
| 24 | `second_candidate_last_name` | II CANDIDATE LAST NAME |
| 25 | `second_candidate_first_name` | II CANDIDATE FIRST NAME |
| 26 | `second_candidate_middle_name` | II CANDIDATE MIDDLE NAME |
| 27 | `second_candidate_prefix` | II CANDIDATE PREFIX |
| 28 | `second_candidate_suffix` | II CANDIDATE SUFFIX |
| 29 | `second_candidate_office` | II CANDIDATE OFFICE |
| 30 | `second_candidate_state` | II CANDIDATE STATE |
| 31 | `second_candidate_district` | II CANDIDATE DIST |
| 32 | `second_candidate_contribution_date` | II DATE OF CONTRIBUTION |
| 33 | `third_candidate_id_number` | III CANDIDATE ID NUMBER |
|  | `third_candidate_name` | CANDIDATE NAME |
| 34 | `third_candidate_last_name` | III CANDIDATE LAST NAME |
| 35 | `third_candidate_first_name` | III CANDIDATE FIRST NAME |
| 36 | `third_candidate_middle_name` | III CANDIDATE MIDDLE NAME |
| 37 | `third_candidate_prefix` | III CANDIDATE PREFIX |
| 38 | `third_candidate_suffix` | III CANDIDATE SUFFIX |
| 39 | `third_candidate_office` | III CANDIDATE OFFICE |
| 40 | `third_candidate_state` | III CANDIDATE STATE |
| 41 | `third_candidate_district` | III CANDIDATE DIST |
| 42 | `third_candidate_contribution_date` | III DATE OF CONTRIBUTION |
| 43 | `fourth_candidate_id_number` | IV CANDIDATE ID NUMBER |
|  | `fourth_candidate_name` | CANDIDATE NAME |
| 44 | `fourth_candidate_last_name` | IV CANDIDATE LAST NAME |
| 45 | `fourth_candidate_first_name` | IV CANDIDATE FIRST NAME |
| 46 | `fourth_candidate_middle_name` | IV CANDIDATE MIDDLE NAME |
| 47 | `fourth_candidate_prefix` | IV CANDIDATE PREFIX |
| 48 | `fourth_candidate_suffix` | IV CANDIDATE SUFFIX |
| 49 | `fourth_candidate_office` | IV CANDIDATE OFFICE |
| 50 | `fourth_candidate_state` | IV CANDIDATE STATE |
| 51 | `fourth_candidate_district` | IV CANDIDATE DIST |
| 52 | `fourth_candidate_contribution_date` | IV DATE OF CONTRIBUTION |
| 53 | `fifth_candidate_id_number` | V CANDIDATE ID NUMBER |
|  | `fifth_candidate_name` | CANDIDATE NAME |
| 54 | `fifth_candidate_last_name` | V CANDIDATE LAST NAME |
| 55 | `fifth_candidate_first_name` | V CANDIDATE FIRST NAME |
| 56 | `fifth_candidate_middle_name` | V CANDIDATE MIDDLE NAME |
| 57 | `fifth_candidate_prefix` | V CANDIDATE PREFIX |
| 58 | `fifth_candidate_suffix` | V CANDIDATE SUFFIX |
| 59 | `fifth_candidate_office` | V CANDIDATE OFFICE |
| 60 | `fifth_candidate_state` | V CANDIDATE STATE |
| 61 | `fifth_candidate_district` | V CANDIDATE DIST |
| 62 | `fifth_candidate_contribution_date` | V DATE OF CONTRIBUTION |
| 63 | `fifty_first_contributor_date` | DATE (Of 51st Contributor) |
| 64 | `original_registration_date` | DATE (Of Orig Registration) |
| 65 | `requirements_met_date` | DATE (Cmte Met Requirements) |
|  | `treasurer_name` | NAME/TREASURER (as signed) |
| 66 | `treasurer_last_name` | TREASURER LAST NAME |
| 67 | `treasurer_first_name` | TREASURER FIRST NAME |
| 68 | `treasurer_middle_name` | TREASURER MIDDLE NAME |
| 69 | `treasurer_prefix` | TREASURER PREFIX |
| 70 | `treasurer_suffix` | TREASURER SUFFIX |
| 71 | `date_signed` | DATE SIGNED |

### F1S

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `joint_fund_participant_committee_name` | 5. JOINT FUND PARTICIPANT CMTTE NAME |
| 4 | `joint_fund_participant_committee_id_number` | 5. JOINT FUND PARTICIPANT CMTTE FEC-ID |
| 5 | `joint_fund_participant_committee_type` | JOINT FUND PARTICIPANT CMTE TYPE |
| 6 | `affiliated_committee_id_number` | 6. AFFILIATED CMTTE ID NUM |
| 7 | `affiliated_committee_name` | 6. AFFILIATED CMTTE NAME |
| 8 | `affiliated_candidate_id_number` | 6. AFFILIATED CANDIDATE ID NUM |
| 9 | `affiliated_last_name` | 6. AFFILIATED LAST NAME |
| 10 | `affiliated_first_name` | 6. AFFILIATED FIRST NAME |
| 11 | `affiliated_middle_name` | 6. AFFILIATED MIDDLE NAME |
| 12 | `affiliated_prefix` | 6. AFFILIATED PREFIX |
| 13 | `affiliated_suffix` | 6. AFFILIATED SUFFIX |
| 14 | `affiliated_street_1` | 6. AFFILIATED STREET 1 |
| 15 | `affiliated_street_2` | 6. AFFILIATED STREET 2 |
| 16 | `affiliated_city` | 6. AFFILIATED CITY |
| 17 | `affiliated_state` | 6. AFFILIATED STATE |
| 18 | `affiliated_zip_code` | 6. AFFILIATED ZIP |
| 19 | `affiliated_relationship_code` | 6. AFFILIATED RELATIONSHIP CODE (with Filing Committee named in field #4) |
|  | `agent_name` |  |
| 20 | `agent_last_name` | 8. AGENT LAST NAME |
| 21 | `agent_first_name` | 8. AGENT FIRST NAME |
| 22 | `agent_middle_name` | 8. AGENT MIDDLE NAME |
| 23 | `agent_prefix` | 8. AGENT PREFIX |
| 24 | `agent_suffix` | 8. AGENT SUFFIX |
| 25 | `agent_street_1` | 8. AGENT STREET 1 |
| 26 | `agent_street_2` | 8. AGENT STREET 2 |
| 27 | `agent_city` | 8. AGENT CITY |
| 28 | `agent_state` | 8. AGENT STATE |
| 29 | `agent_zip_code` | 8. AGENT ZIP |
| 30 | `agent_title` | 8. AGENT TITLE |
| 31 | `agent_telephone` | 8. AGENT TELEPHONE |
| 32 | `bank_name` | 9. BANK NAME |
| 33 | `bank_street_1` | 9. BANK STREET 1 |
| 34 | `bank_street_2` | 9. BANK STREET 2 |
| 35 | `bank_city` | 9. BANK CITY |
| 36 | `bank_state` | 9. BANK STATE |
| 37 | `bank_zip_code` | 9. BANK ZIP |

### F2

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `candidate_id_number` | FILER CANDIDATE ID NUMBER |
|  | `candidate_name` | CANDIDATE NAME |
| 3 | `candidate_last_name` | CANDIDATE LAST NAME |
| 4 | `candidate_first_name` | CANDIDATE FIRST NAME |
| 5 | `candidate_middle_name` | CANDIDATE MIDDLE NAME |
| 6 | `candidate_prefix` | CANDIDATE PREFIX |
| 7 | `candidate_suffix` | CANDIDATE SUFFIX |
| 8 | `vice_president_last_name` | VICE PRESIDENT LAST NAME |
| 9 | `vice_president_first_name` | VICE PRESIDENT FIRST NAME |
| 10 | `vice_president_middle_name` | VICE PRESIDENT MIDDLE NAME |
| 11 | `vice_president_prefix` | VICE PRESIDENT PREFIX |
| 12 | `vice_president_suffix` | VICE PRESIDENT SUFFIX |
| 13 | `change_of_address` | CHANGE OF ADDRESS |
| 14 | `candidate_street_1` | CANDIDATE STREET 1 |
| 15 | `candidate_street_2` | CANDIDATE STREET 2 |
| 16 | `candidate_city` | CANDIDATE CITY |
| 17 | `candidate_state` | CANDIDATE STATE |
| 18 | `candidate_zip_code` | CANDIDATE ZIP |
| 19 | `candidate_party_code` | CANDIDATE PARTY CODE |
| 20 | `candidate_office` | CANDIDATE OFFICE |
| 21 | `candidate_office_state` | CANDIDATE STATE |
| 22 | `candidate_district` | CANDIDATE DISTRICT |
| 23 | `election_year` | YEAR OF ELECTION 1900-2999 |
| 24 | `committee_id_number` | PCC COMMITTEE ID NUMBER |
| 25 | `committee_name` | PCC COMMITTEE NAME |
| 26 | `committee_street_1` | PCC STREET 1 |
| 27 | `committee_street_2` | PCC STREET 2 |
| 28 | `committee_city` | PCC CITY |
| 29 | `committee_state` | PCC STATE |
| 30 | `committee_zip_code` | PCC ZIP |
| 31 | `authorized_committee_id_number` | AUTH COMMITTEE ID NUMBER |
| 32 | `authorized_committee_name` | AUTH COMMITTEE NAME |
| 33 | `authorized_committee_street_1` | AUTH STREET 1 |
| 34 | `authorized_committee_street_2` | AUTH STREET 2 |
| 35 | `authorized_committee_city` | AUTH CITY |
| 36 | `authorized_committee_state` | AUTH STATE |
| 37 | `authorized_committee_zip_code` | AUTH ZIP |
|  | `candidate_signature_name` | NAME/CAN (as signed) |
| 38 | `candidate_signature_last_name` | CANDIDATE SIGNATURE LAST NAME |
| 39 | `candidate_signature_first_name` | CANDIDATE SIGNATURE FIRST NAME |
| 40 | `candidate_signature_middle_name` | CANDIDATE SIGNATURE MIDDLE NAME |
| 41 | `candidate_signature_prefix` | CANDIDATE SIGNATURE PREFIX |
| 42 | `candidate_signature_suffix` | CANDIDATE SIGNATURE SUFFIX |
| 43 | `date_signed` | DATE SIGNED |
|  | `primary_personal_funds_declared` | PRI PERSONAL FUNDS DECLARED |
|  | `general_personal_funds_declared` | GEN PERSONAL FUNDS DECLARED |

### F24

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `report_type` | REPORT TYPE {24/48 Hour} |
| 4 | `original_amendment_date` | ORIGINAL AMENDMENT DATE |
| 5 | `committee_name` | COMMITTEE NAME |
| 6 | `street_1` | STREET 1 |
| 7 | `street_2` | STREET 2 |
| 8 | `city` | CITY |
| 9 | `state` | STATE |
| 10 | `zip_code` | ZIP |
|  | `treasurer_name` | NAME/TREASURER (as signed) |
| 11 | `treasurer_last_name` | TREASURER LAST NAME |
| 12 | `treasurer_first_name` | TREASURER FIRST NAME |
| 13 | `treasurer_middle_name` | TREASURER MIDDLE NAME |
| 14 | `treasurer_prefix` | TREASURER PREFIX |
| 15 | `treasurer_suffix` | TREASURER SUFFIX |
| 16 | `date_signed` | DATE SIGNED |

### F2S

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `candidate_id_number` | FILER CANDIDATE ID NUMBER |
| 3 | `authorized_committee_id_number` | AUTH COMMITTEE ID NUMBER |
| 4 | `authorized_committee_name` | AUTH COMMITTEE NAME |
| 5 | `authorized_committee_street_1` | AUTH STREET 1 |
| 6 | `authorized_committee_street_2` | AUTH STREET 2 |
| 7 | `authorized_committee_city` | AUTH CITY |
| 8 | `authorized_committee_state` | AUTH STATE |
| 9 | `authorized_committee_zip_code` | AUTH ZIP |

### F3

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `committee_name` | COMMITTEE NAME |
| 4 | `change_of_address` | CHANGE OF ADDRESS |
| 5 | `street_1` | STREET 1 |
| 6 | `street_2` | STREET 2 |
| 7 | `city` | CITY |
| 8 | `state` | STATE |
| 9 | `zip_code` | ZIP |
| 12 | `report_code` | REPORT CODE |
| 13 | `election_code` | ELECTION CODE {was RPTPGI} |
| 14 | `election_date` | DATE OF ELECTION |
| 10 | `election_state` | ELECTION STATE |
| 11 | `election_district` | ELECTION DISTRICT |
| 15 | `state_of_election` | STATE OF ELECTION |
| 16 | `coverage_from_date` | COVERAGE FROM DATE |
| 17 | `coverage_through_date` | COVERAGE THROUGH DATE |
|  | `primary_election` | PRIMARY ELECTION |
|  | `general_election` | GENERAL ELECTION |
|  | `special_election` | SPECIAL ELECTION |
|  | `runoff_election` | RUNOFF ELECTION |
|  | `treasurer_name` | NAME/TREASURER (as signed) |
| 18 | `treasurer_last_name` | TREASURER LAST NAME |
| 19 | `treasurer_first_name` | TREASURER FIRST NAME |
| 20 | `treasurer_middle_name` | TREASURER MIDDLE NAME |
| 21 | `treasurer_prefix` | TREASURER PREFIX |
| 22 | `treasurer_suffix` | TREASURER SUFFIX |
| 23 | `date_signed` | DATE SIGNED |
|  | `candidate_id_number` | CANDIDATE ID NUMBER |
|  | `candidate_name` | CANDIDATE NAME |
|  | `candidate_last_name` | CANDIDATE LAST NAME |
|  | `candidate_first_name` | CANDIDATE FIRST NAME |
|  | `candidate_middle_name` | CANDIDATE MIDDLE NAME |
|  | `candidate_prefix` | CANDIDATE PREFIX |
|  | `candidate_suffix` | CANDIDATE SUFFIX |
|  | `report_type` | F3Z-1 REPORT TYPE |
| 24 | `col_a_total_contributions_no_loans` | (6a) Total Contributions (NO Loans) |
| 25 | `col_a_total_contributions_refunds` | (6b) Total Contribution Refunds |
| 26 | `col_a_net_contributions` | (6c) Net Contributions |
| 27 | `col_a_total_operating_expenditures` | (7a) Total Operating Expenditures |
| 28 | `col_a_total_offset_to_operating_expenditures` | (7b) Total Offset to Operating Expenditures |
| 29 | `col_a_net_operating_expenditures` | (7c) NET Operating Expenditures. |
| 30 | `col_a_cash_on_hand_close_of_period` | 8. CASH ON HAND AT CLOSE ... |
| 31 | `col_a_debts_to` | 9. DEBTS TO ( Totals from SCH C and/or D) |
| 32 | `col_a_debts_by` | 10. DEBTS BY (Totals from SCH C and/or D) |
| 33 | `col_a_individuals_itemized` | 11(a i.) Individuals Itemized |
| 34 | `col_a_individuals_unitemized` | 11(a.ii) Individuals Unitemized |
| 35 | `col_a_individual_contribution_total` | 11(a.iii) Individual Contribution Total |
| 36 | `col_a_political_party_contributions` | 11(b) Political Party Committees |
| 37 | `col_a_pac_contributions` | 11(c) Other Political Committees |
| 38 | `col_a_candidate_contributions` | 11(d) The Candidate |
| 39 | `col_a_total_contributions` | 11(e) Total Contributions |
| 40 | `col_a_transfers_from_authorized` | 12. Transfers From Other Authorized Cmttes |
| 41 | `col_a_candidate_loans` | 13(a) Loans made or guarn. by the Candidate |
| 42 | `col_a_other_loans` | 13(b) All Other Loans |
| 43 | `col_a_total_loans` | 13(c) Total Loans |
| 44 | `col_a_offset_to_operating_expenditures` | 14. Offsets to Operating Expenditures |
| 45 | `col_a_other_receipts` | 15. Other Receipts |
| 46 | `col_a_total_receipts` | 16. Total Receipts |
| 47 | `col_a_operating_expenditures` | 17. Operating Expenditures |
| 48 | `col_a_transfers_to_authorized` | 18. Transfers to Other Authorized Committees |
| 49 | `col_a_candidate_loan_repayments` | 19(a) Of Loans made or guar. by the Cand. |
| 50 | `col_a_other_loan_repayments` | 19(b) Loan Repayments, All Other Loans |
| 51 | `col_a_total_loan_repayments` | 19(c) Total Loan Repayments |
| 52 | `col_a_refunds_to_individuals` | 20(a) Refund/Individuals Other than Pol. Cmtes |
| 53 | `col_a_refunds_to_party_committees` | 20(b) Refund/Political Party Committees |
| 54 | `col_a_refunds_to_other_committees` | 20(c) Refund/Other Political Committees |
| 55 | `col_a_total_refunds` | 20(d) Total Contribution Refunds |
| 56 | `col_a_other_disbursements` | 21. Other Disbursements |
| 57 | `col_a_total_disbursements` | 22. Total Disbursements |
| 58 | `col_a_cash_on_hand_beginning_period` | 23. Cash Beginning Reporting Period |
| 59 | `col_a_total_receipts_period` | 24. Total Receipts this Period |
| 60 | `col_a_subtotal` | 25. Subtotals |
| 61 | `col_a_total_disbursements_period` | 26. Total Disbursements this Period |
| 62 | `col_a_cash_on_hand_close` | 27. Cash on hand at Close Period |
| 63 | `col_b_total_contributions_no_loans` | (6a) Total Contributions (No Loans) |
| 64 | `col_b_total_contributions_refunds` | (6b) Total Contribution Refunds |
| 65 | `col_b_net_contributions` | (6c) Net Contributions |
| 66 | `col_b_total_operating_expenditures` | (7a) Total Operating Expenditures |
| 67 | `col_b_total_offset_to_operating_expenditures` | (7b) Total Offsets to Operating Expenditures |
| 68 | `col_b_net_operating_expenditures` | (7c) NET Operating Expenditures. |
| 69 | `col_b_individuals_itemized` | 11(a i.) Individuals Itemized |
| 70 | `col_b_individuals_unitemized` | 11(a.ii) Individuals Unitemized |
| 71 | `col_b_individual_contribution_total` | 11(a.iii) Individuals Total |
| 72 | `col_b_political_party_contributions` | 11(b) Political Party Committees |
| 73 | `col_b_pac_contributions` | 11(c) All Other Political Committees (PACS) |
| 74 | `col_b_candidate_contributions` | 11(d) The Candidate |
| 75 | `col_b_total_contributions` | 11(e) Total Contributions |
| 76 | `col_b_transfers_from_authorized` | 12. Transfers From Other AUTH Committees |
| 77 | `col_b_candidate_loans` | 13(a) Loans made or guarn. by the Candidate |
| 78 | `col_b_other_loans` | 13(b) All Other Loans |
| 79 | `col_b_total_loans` | 13(c) Total Loans |
| 80 | `col_b_offset_to_operating_expenditures` | 14. Offsets to Operating Expenditures |
| 81 | `col_b_other_receipts` | 15. Other Receipts |
| 82 | `col_b_total_receipts` | 16. Total Receipts |
| 83 | `col_b_operating_expenditures` | 17 Operating Expenditures |
| 84 | `col_b_transfers_to_authorized` | 18. Transfers To Other AUTH Committees |
| 85 | `col_b_candidate_loan_repayments` | 19(a) Loan Repayment By Candidate |
| 86 | `col_b_other_loan_repayments` | 19(b) Loan Repayments, ALL Other Loans |
| 87 | `col_b_total_loan_repayments` | 19(c) Total Loan Repayments |
| 88 | `col_b_refunds_to_individuals` | 20(a) Refund/Individuals Other than Pol. Cmtes |
| 89 | `col_b_refunds_to_party_committees` | 20(b) Refund, Political Party Committees |
| 90 | `col_b_refunds_to_other_committees` | 20(c) Refund, Other Political Committees |
| 91 | `col_b_total_refunds` | 20(d) Total Contributions Refunds |
| 92 | `col_b_other_disbursements` | 21. Other Disbursements |
| 93 | `col_b_total_disbursements` | 22. Total Disbursements |
|  | `col_b_gross_receipts_authorized_primary` | Gross Receipts of Authorized Committees (primary) |
|  | `col_b_aggregate_personal_funds_primary` | Aggregate Amount from Personal Funds (primary) |
|  | `col_b_gross_receipts_minus_personal_funds_primary` | Gross Receipts Minus Personal from Candidate (primary) |
|  | `col_b_gross_receipts_authorized_general` | Gross Receipts of Authorized Committees (general) |
|  | `col_b_aggregate_personal_funds_general` | Aggregate Amount from Personal Funds (general) |
|  | `col_b_gross_receipts_minus_personal_funds_general` | Gross Receipts Minus Personal from Candidate (general) |

### F3L

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `committee_name` | COMMITTEE NAME |
| 4 | `change_of_address` | CHANGE OF ADDRESS |
| 5 | `street_1` | STREET 1 |
| 6 | `street_2` | STREET 2 |
| 7 | `city` | CITY |
| 8 | `state` | STATE |
| 9 | `zip_code` | ZIP |
| 12 | `report_code` | REPORT CODE |
| 13 | `election_date` | DATE OF ELECTION |
| 10 | `election_state` | ELECTION STATE |
| 11 | `election_district` | ELECTION DISTRICT |
| 16 | `coverage_from_date` | COVERAGE FROM DATE |
| 17 | `coverage_through_date` | COVERAGE THROUGH DATE |
| 15 | `semi_annual_period` | SEMI-ANNUAL PERIOD - Sect 5(c) or (d) |
| 18 | `semi_annual_period_jan_june` | SEMI-ANNUAL JAN-JUN - Sect 6(b) |
| 19 | `semi_annual_period_jul_dec` | SEMI-ANNUAL JUL-DEC - Sect 6(b) |
| 20 | `quarterly_monthly_bundled_contributions` | QTR/MON/POST BUNDLED CONTRIBUTIONS |
| 21 | `semi_annual_bundled_contributions` | SEMI-ANNUAL BUNDLED CONTRIBS |
| 22 | `treasurer_last_name` | TREASURER LAST NAME |
| 23 | `treasurer_first_name` | TREASURER FIRST NAME |
| 24 | `treasurer_middle_name` | TREASURER MIDDLE NAME |
| 25 | `treasurer_prefix` | TREASURER PREFIX |
| 26 | `treasurer_suffix` | TREASURER SUFFIX |
| 27 | `date_signed` | DATE SIGNED |

### F3P

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `committee_name` | COMMITTEE NAME |
| 4 | `change_of_address` | CHANGE OF ADDRESS |
| 5 | `street_1` | STREET 1 |
| 6 | `street_2` | STREET 2 |
| 7 | `city` | CITY |
| 8 | `state` | STATE |
| 9 | `zip_code` | ZIP |
| 10 | `activity_primary` | ACTIVITY PRIMARY |
| 11 | `activity_general` | ACTIVITY GENERAL |
| 12 | `report_code` | REPORT CODE |
| 13 | `election_code` | ELECTION CODE {was RPTPGI} |
| 14 | `election_date` | DATE OF ELECTION |
| 15 | `state_of_election` | STATE OF ELECTION |
| 16 | `coverage_from_date` | COVERAGE FROM DATE |
| 17 | `coverage_through_date` | COVERAGE THROUGH DATE |
|  | `treasurer_name` | NAME/TREASURER (as signed) |
| 18 | `treasurer_last_name` | TREASURER LAST NAME |
| 19 | `treasurer_first_name` | TREASURER FIRST NAME |
| 20 | `treasurer_middle_name` | TREASURER MIDDLE NAME |
| 21 | `treasurer_prefix` | TREASURER PREFIX |
| 22 | `treasurer_suffix` | TREASURER SUFFIX |
| 23 | `date_signed` | DATE SIGNED |
| 24 | `col_a_cash_on_hand_beginning_period` | 6. Cash on Hand Beginning Period |
| 25 | `col_a_total_receipts` | 7. Total Receipts |
| 26 | `col_a_subtotal` | 8. Subtotal |
| 27 | `col_a_total_disbursements` | 9. Total Disbursements |
| 28 | `col_a_cash_on_hand_close_of_period` | 10. Cash on Hand Close of Period |
| 29 | `col_a_debts_to` | 11. Debts to |
| 30 | `col_a_debts_by` | 12. Debts by |
| 31 | `col_a_expenditures_subject_to_limits` | 13. Expenditures Subject to Limits |
| 32 | `col_a_net_contributions` | 14. Net Contributions |
| 33 | `col_a_net_operating_expenditures` | 15. Net Operating Expenditures |
| 34 | `col_a_federal_funds` | 16. Federal Funds |
| 35 | `col_a_individuals_itemized` | 17(a.i) Individuals Itemized |
| 36 | `col_a_individuals_unitemized` | 17(a.ii) Individuals Unitemized |
| 37 | `col_a_individual_contribution_total` | 17(a.iii) Individual Contribution Total |
| 38 | `col_a_political_party_contributions` | 17(b) Political Party Committees |
| 39 | `col_a_pac_contributions` | 17(c) Other Political Committees (PACs) |
| 40 | `col_a_candidate_contributions` | 17(d) The Candidate |
| 41 | `col_a_total_contributions` | 17(e) Total Contributions |
| 42 | `col_a_transfers_from_aff_other_party_cmttees` | 18. Transfers From Aff/Other Party Cmttees |
| 43 | `col_a_candidate_loans` | 19(a) Received from or Guaranteed by Cand. |
| 44 | `col_a_other_loans` | 19(b) Other Loans |
| 45 | `col_a_total_loans` | 19(c) Total Loans |
| 46 | `col_a_offset_to_operating_expenditures` | 20(a) Operating |
| 47 | `col_a_offset_to_fundraising_expenditures` | 20(b) Fundraising |
| 48 | `col_a_offset_to_legal_expenditures` | 20(c) Legal and Accounting |
| 49 | `col_a_total_offsets_to_expenditures` | 20(d) Total offsets to Expenditures |
| 50 | `col_a_other_receipts` | 21. Other Receipts |
| 51 | `col_a_total_receipts_recap` | 22. Total Receipts |
| 52 | `col_a_operating_expenditures` | 23. Operating Expenditures |
| 53 | `col_a_transfers_to_authorized` | 24. Transfers to Other Authorized Committees |
| 54 | `col_a_fundraising_disbursements` | 25. Fundraising Disbursements |
| 55 | `col_a_exempt_legal_disbursements` | 26. Exempt Legal & Accounting Disbursement |
| 56 | `col_a_candidate_loan_repayments` | 27(a) Made or guaranteed by Candidate |
| 57 | `col_a_other_loan_repayments` | 27(b) Other Repayments |
| 58 | `col_a_total_loan_repayments_made` | 27(c) Total Loan Repayments Made |
| 59 | `col_a_refunds_to_individuals` | 28(a) Individuals |
| 60 | `col_a_refunds_to_party_committees` | 28(b) Political Party Committees |
| 61 | `col_a_refunds_to_other_committees` | 28(c) Other Political Committees |
| 62 | `col_a_total_refunds` | 28(d) Total Contributions Refunds |
| 63 | `col_a_other_disbursements` | 29. Other Disbursements |
| 64 | `col_a_total_disbursements_recap` | 30. Total Disbursements |
| 65 | `col_a_items_on_hand_to_be_liquidated` | 31. Items on Hand to be Liquidated |
| 66 | `col_a_alabama` | ALABAMA |
| 67 | `col_a_alaska` | ALASKA |
| 68 | `col_a_arizona` | ARIZONA |
| 69 | `col_a_arkansas` | ARKANSAS |
| 70 | `col_a_california` | CALIFORNIA |
| 71 | `col_a_colorado` | COLORADO |
| 72 | `col_a_connecticut` | CONNECTICUT |
| 73 | `col_a_delaware` | DELAWARE |
| 74 | `col_a_dist_of_columbia` | DIST OF COLUMBIA |
| 75 | `col_a_florida` | FLORIDA |
| 76 | `col_a_georgia` | GEORGIA |
| 77 | `col_a_hawaii` | HAWAII |
| 78 | `col_a_idaho` | IDAHO |
| 79 | `col_a_illinois` | ILLINOIS |
| 80 | `col_a_indiana` | INDIANA |
| 81 | `col_a_iowa` | IOWA |
| 82 | `col_a_kansas` | KANSAS |
| 83 | `col_a_kentucky` | KENTUCKY |
| 84 | `col_a_louisiana` | LOUISIANA |
| 85 | `col_a_maine` | MAINE |
| 86 | `col_a_maryland` | MARYLAND |
| 87 | `col_a_massachusetts` | MASSACHUSETTS |
| 88 | `col_a_michigan` | MICHIGAN |
| 89 | `col_a_minnesota` | MINNESOTA |
| 90 | `col_a_mississippi` | MISSISSIPPI |
| 91 | `col_a_missouri` | MISSOURI |
| 92 | `col_a_montana` | MONTANA |
| 93 | `col_a_nebraska` | NEBRASKA |
| 94 | `col_a_nevada` | NEVADA |
| 95 | `col_a_new_hampshire` | NEW HAMPSHIRE |
| 96 | `col_a_new_jersey` | NEW JERSEY |
| 97 | `col_a_new_mexico` | NEW MEXICO |
| 98 | `col_a_new_york` | NEW YORK |
| 99 | `col_a_north_carolina` | NORTH CAROLINA |
| 100 | `col_a_north_dakota` | NORTH DAKOTA |
| 101 | `col_a_ohio` | OHIO |
| 102 | `col_a_oklahoma` | OKLAHOMA |
| 103 | `col_a_oregon` | OREGON |
| 104 | `col_a_pennsylvania` | PENNSYLVANIA |
| 105 | `col_a_rhode_island` | RHODE ISLAND |
| 106 | `col_a_south_carolina` | SOUTH CAROLINA |
| 107 | `col_a_south_dakota` | SOUTH DAKOTA |
| 108 | `col_a_tennessee` | TENNESSEE |
| 109 | `col_a_texas` | TEXAS |
| 110 | `col_a_utah` | UTAH |
| 111 | `col_a_vermont` | VERMONT |
| 112 | `col_a_virginia` | VIRGINIA |
| 113 | `col_a_washington` | WASHINGTON |
| 114 | `col_a_west_virginia` | WEST VIRGINIA |
| 115 | `col_a_wisconsin` | WISCONSIN |
| 116 | `col_a_wyoming` | WYOMING |
| 117 | `col_a_puerto_rico` | PUERTO RICO |
| 118 | `col_a_guam` | GUAM |
| 119 | `col_a_virgin_islands` | VIRGIN ISLANDS |
| 120 | `col_a_totals` | TOTALS |
| 121 | `col_b_federal_funds` | 16. Federal Funds |
| 122 | `col_b_individuals_itemized` | 17(a.i) Individuals Itemized |
| 123 | `col_b_individuals_unitemized` | 17(a.ii) Individuals Unitemized |
| 124 | `col_b_individual_contribution_total` | 17(a.iii) Individual Contribution Total |
| 125 | `col_b_political_party_contributions` | 17(b) Political Party Committees |
| 126 | `col_b_pac_contributions` | 17(c) Other Political Committees (PACs) |
| 127 | `col_b_candidate_contributions` | 17(d) The Candidate |
| 128 | `col_b_total_contributions` | 17(e) Total contributions (Other than Loans) |
| 129 | `col_b_transfers_from_aff_other_party_cmttees` | 18. Transfers From Aff/Other Party Cmttees |
| 130 | `col_b_candidate_loans` | 19(a) Received from or Guaranteed by Cand. |
| 131 | `col_b_other_loans` | 19(b) Other Loans |
| 132 | `col_b_total_loans` | 19(c) Total Loans |
| 133 | `col_b_offset_to_operating_expenditures` | 20(a) Operating |
| 134 | `col_b_offset_to_fundraising_expenditures` | 20(b) Fundraising |
| 135 | `col_b_offset_to_legal_expenditures` | 20(c) Legal and Accounting |
| 136 | `col_b_total_offsets_to_expenditures` | 20(d) Total Offsets to Operating Expenditures |
| 137 | `col_b_other_receipts` | 21. Other Receipts |
| 138 | `col_b_total_receipts` | 22. Total Receipts |
| 139 | `col_b_operating_expenditures` | 23. Operating Expenditures |
| 140 | `col_b_transfers_to_authorized` | 24. Transfers to Other Authorized Committees |
| 141 | `col_b_fundraising_disbursements` | 25. Fundraising Disbursements |
| 142 | `col_b_exempt_legal_disbursements` | 26. Exempt Legal & Accounting Disbursement |
| 143 | `col_b_candidate_loan_repayments` | 27(a) Made or Guaranteed by the Candidate |
| 144 | `col_b_other_loan_repayments` | 27(b) Other Repayments |
| 145 | `col_b_total_loan_repayments_made` | 27(c) Total Loan Repayments Made |
| 146 | `col_b_refunds_to_individuals` | 28(a) Individuals |
| 147 | `col_b_refunds_to_party_committees` | 28(b) Political Party Committees |
| 148 | `col_b_refunds_to_other_committees` | 28(c) Other Political Committees |
| 149 | `col_b_total_refunds` | 28(d) Total Contributions Refunds |
| 150 | `col_b_other_disbursements` | 29. Other Disbursements |
| 151 | `col_b_total_disbursements` | 30. Total Disbursements |
| 152 | `col_b_alabama` | ALABAMA |
| 153 | `col_b_alaska` | ALASKA |
| 154 | `col_b_arizona` | ARIZONA |
| 155 | `col_b_arkansas` | ARKANSAS |
| 156 | `col_b_california` | CALIFORNIA |
| 157 | `col_b_colorado` | COLORADO |
| 158 | `col_b_connecticut` | CONNECTICUT |
| 159 | `col_b_delaware` | DELAWARE |
| 160 | `col_b_dist_of_columbia` | DIST OF COLUMBIA |
| 161 | `col_b_florida` | FLORIDA |
| 162 | `col_b_georgia` | GEORGIA |
| 163 | `col_b_hawaii` | HAWAII |
| 164 | `col_b_idaho` | IDAHO |
| 165 | `col_b_illinois` | ILLINOIS |
| 166 | `col_b_indiana` | INDIANA |
| 167 | `col_b_iowa` | IOWA |
| 168 | `col_b_kansas` | KANSAS |
| 169 | `col_b_kentucky` | KENTUCKY |
| 170 | `col_b_louisiana` | LOUISIANA |
| 171 | `col_b_maine` | MAINE |
| 172 | `col_b_maryland` | MARYLAND |
| 173 | `col_b_massachusetts` | MASSACHUSETTS |
| 174 | `col_b_michigan` | MICHIGAN |
| 175 | `col_b_minnesota` | MINNESOTA |
| 176 | `col_b_mississippi` | MISSISSIPPI |
| 177 | `col_b_missouri` | MISSOURI |
| 178 | `col_b_montana` | MONTANA |
| 179 | `col_b_nebraska` | NEBRASKA |
| 180 | `col_b_nevada` | NEVADA |
| 181 | `col_b_new_hampshire` | NEW HAMPSHIRE |
| 182 | `col_b_new_jersey` | NEW JERSEY |
| 183 | `col_b_new_mexico` | NEW MEXICO |
| 184 | `col_b_new_york` | NEW YORK |
| 185 | `col_b_north_carolina` | NORTH CAROLINA |
| 186 | `col_b_north_dakota` | NORTH DAKOTA |
| 187 | `col_b_ohio` | OHIO |
| 188 | `col_b_oklahoma` | OKLAHOMA |
| 189 | `col_b_oregon` | OREGON |
| 190 | `col_b_pennsylvania` | PENNSYLVANIA |
| 191 | `col_b_rhode_island` | RHODE ISLAND |
| 192 | `col_b_south_carolina` | SOUTH CAROLINA |
| 193 | `col_b_south_dakota` | SOUTH DAKOTA |
| 194 | `col_b_tennessee` | TENNESSEE |
| 195 | `col_b_texas` | TEXAS |
| 196 | `col_b_utah` | UTAH |
| 197 | `col_b_vermont` | VERMONT |
| 198 | `col_b_virginia` | VIRGINIA |
| 199 | `col_b_washington` | WASHINGTON |
| 200 | `col_b_west_virginia` | WEST VIRGINIA |
| 201 | `col_b_wisconsin` | WISCONSIN |
| 202 | `col_b_wyoming` | WYOMING |
| 203 | `col_b_puerto_rico` | PUERTO RICO |
| 204 | `col_b_guam` | GUAM |
| 205 | `col_b_virgin_islands` | VIRGIN ISLANDS |
| 206 | `col_b_totals` | TOTALS |

### F3P31

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `entity_type` | ENTITY TYPE |
| 5 | `contributor_organization_name` | CONTRIBUTOR ORGANIZATION NAME |
|  | `contributor_name` | NAME (Contributor/Lender) |
| 6 | `contributor_last_name` | CONTRIBUTOR LAST NAME |
| 7 | `contributor_first_name` | CONTRIBUTOR FIRST NAME |
| 8 | `contributor_middle_name` | CONTRIBUTOR MIDDLE NAME |
| 9 | `contributor_prefix` | CONTRIBUTOR PREFIX |
| 10 | `contributor_suffix` | CONTRIBUTOR SUFFIX |
| 11 | `contributor_street_1` | CONTRIBUTOR STREET 1 |
| 12 | `contributor_street_2` | CONTRIBUTOR STREET 2 |
| 13 | `contributor_city` | CONTRIBUTOR CITY |
| 14 | `contributor_state` | CONTRIBUTOR STATE |
| 15 | `contributor_zip_code` | CONTRIBUTOR ZIP |
| 16 | `election_code` | ELECTION CODE {was RPTPGI} |
| 17 | `item_description` | ITEM DESCRIPTION |
| 18 | `item_contribution_aquired_date` | ITEM CONTRIBUTION/AQUIRED DATE |
| 19 | `item_fair_market_value` | ITEM FAIR MARKET VALUE |
| 20 | `contributor_employer` | CONTRIBUTOR EMPLOYER |
| 21 | `contributor_occupation` | CONTRIBUTOR OCCUPATION |
| 22 | `memo_code` | MEMO CODE |
| 23 | `memo_text` | MEMO TEXT/DESCRIPTION |
|  | `transaction_code` | TRANSACTION CODE |
|  | `transaction_description` | TRANSDESC |
|  | `fec_committee_id_number` | FEC COMMITTEE ID NUMBER |
|  | `candidate_id_number` | FEC CANDIDATE ID NUMBER |
|  | `candidate_name` | CANDIDATE NAME |
|  | `candidate_office` | CAN/OFFICE |
|  | `candidate_state` | CAN/STATE |
|  | `candidate_district` | CAN/DIST |
|  | `conduit_name` | CONDUIT NAME |
|  | `conduit_street_1` | CONDUIT STREET  1 |
|  | `conduit_street_2` | CONDUIT STREET  2 |
|  | `conduit_city` | CONDUIT CITY |
|  | `conduit_state` | CONDUIT STATE |
|  | `conduit_zip_code` | CONDUIT ZIP |

### F3PS

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `date_general_election` | Date - General Election |
| 4 | `date_day_after_general_election` | Date - Day after General Election |
| 5 | `14_net_contributions` | 14. Net Contributions |
| 6 | `15_net_expenditures` | 15. Net Expenditures |
| 7 | `16_federal_funds` | 16. Federal Funds |
|  | `17_a_individuals` | 17(a)  Individuals |
| 8 | `17_a_i_individuals_itemized` | 17(a.i) Individuals Itemized |
| 9 | `17_a_ii_individuals_unitemized` | 17(a.ii) Individuals Unitemized |
| 10 | `17_a_iii_individual_contribution_total` | 17(a.iii) Individual Contribution Total |
| 11 | `17_b_political_party_committees` | 17(b) Political Party Committees |
| 12 | `17_c_other_political_committees_pacs` | 17(c) Other Political Committees (PACs) |
| 13 | `17_d_the_candidate` | 17(d) The Candidate |
| 14 | `17_e_total_contributions_other_than_loans` | 17(e) Total contributions (Other than Loans) |
| 15 | `18_transfers_from_aff_other_party_committees` | 18. Transfers From Aff/Other Party Committees |
| 16 | `19_a_received_from_or_guaranteed_by_candidate` | 19(a) Received from or Guaranteed by Candidate |
| 17 | `19_b_other_loans` | 19(b) Other Loans |
| 18 | `19_c_total_loans` | 19(c) Total Loans |
| 19 | `20_a_operating` | 20(a) Operating |
| 20 | `20_b_fundraising` | 20(b) Fundraising |
| 21 | `20_c_legal_and_accounting` | 20(c) Legal and Accounting |
| 22 | `20_d_total_offsets_to_operating_expenditures` | 20(d) Total Offsets to Operating Expenditures |
| 23 | `21_other_receipts` | 21. Other Receipts |
| 24 | `22_total_receipts` | 22. Total Receipts |
| 25 | `23_operating_expenditures` | 23. Operating Expenditures |
| 26 | `24_transfers_to_other_authorized_committees` | 24. Transfers to Other Authorized Committees |
| 27 | `25_fundraising_disbursements` | 25. Fundraising Disbursements |
| 28 | `26_exempt_legal_and_accounting_disbursements` | 26. Exempt Legal and Accounting Disbursements |
| 29 | `27_a_made_or_guaranteed_by_the_candidate` | 27(a) Made or Guaranteed by the Candidate |
| 30 | `27_b_other_repayments` | 27(b) Other Repayments |
| 31 | `27_c_total_loan_repayments_made` | 27(c) Total Loan Repayments Made |
| 32 | `28_a_individuals` | 28(a) Individuals |
| 33 | `28_b_political_party_committees` | 28(b) Political Party Committees |
| 34 | `28_c_other_political_committees` | 28(c) Other Political Committees |
| 35 | `28_d_total_contributions_refunds` | 28(d) Total Contributions Refunds |
| 36 | `29_other_disbursements` | 29. Other Disbursements |
| 37 | `30_total_disbursements` | 30. Total Disbursements |
| 38 | `alabama` | ALABAMA |
| 39 | `alaska` | ALASKA |
| 40 | `arizona` | ARIZONA |
| 41 | `arkansas` | ARKANSAS |
| 42 | `california` | CALIFORNIA |
| 43 | `colorado` | COLORADO |
| 44 | `connecticut` | CONNECTICUT |
| 45 | `delaware` | DELAWARE |
| 46 | `dist_of_columbia` | DIST OF COLUMBIA |
| 47 | `florida` | FLORIDA |
| 48 | `georgia` | GEORGIA |
| 49 | `hawaii` | HAWAII |
| 50 | `idaho` | IDAHO |
| 51 | `illinois` | ILLINOIS |
| 52 | `indiana` | INDIANA |
| 53 | `iowa` | IOWA |
| 54 | `kansas` | KANSAS |
| 55 | `kentucky` | KENTUCKY |
| 56 | `louisiana` | LOUISIANA |
| 57 | `maine` | MAINE |
| 58 | `maryland` | MARYLAND |
| 59 | `massachusetts` | MASSACHUSETTS |
| 60 | `michigan` | MICHIGAN |
| 61 | `minnesota` | MINNESOTA |
| 62 | `mississippi` | MISSISSIPPI |
| 63 | `missouri` | MISSOURI |
| 64 | `montana` | MONTANA |
| 65 | `nebraska` | NEBRASKA |
| 66 | `nevada` | NEVADA |
| 67 | `new_hampshire` | NEW HAMPSHIRE |
| 68 | `new_jersey` | NEW JERSEY |
| 69 | `new_mexico` | NEW MEXICO |
| 70 | `new_york` | NEW YORK |
| 71 | `north_carolina` | NORTH CAROLINA |
| 72 | `north_dakota` | NORTH DAKOTA |
| 73 | `ohio` | OHIO |
| 74 | `oklahoma` | OKLAHOMA |
| 75 | `oregon` | OREGON |
| 76 | `pennsylvania` | PENNSYLVANIA |
| 77 | `rhode_island` | RHODE ISLAND |
| 78 | `south_carolina` | SOUTH CAROLINA |
| 79 | `south_dakota` | SOUTH DAKOTA |
| 80 | `tennessee` | TENNESSEE |
| 81 | `texas` | TEXAS |
| 82 | `utah` | UTAH |
| 83 | `vermont` | VERMONT |
| 84 | `virginia` | VIRGINIA |
| 85 | `washington` | WASHINGTON |
| 86 | `west_virginia` | WEST VIRGINIA |
| 87 | `wisconsin` | WISCONSIN |
| 88 | `wyoming` | WYOMING |
| 89 | `puerto_rico` | PUERTO RICO |
| 90 | `guam` | GUAM |
| 91 | `virgin_islands` | VIRGIN ISLANDS |
| 92 | `totals` | TOTALS |

### F3PZ1

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER (PCC) |
| 3 | `principal_committee_name` | COMMITTEE NAME (PCC) |
| 4 | `coverage_from_date` | COVERAGE FROM DATE |
| 5 | `coverage_through_date` | COVERAGE THROUGH DATE |
| 6 | `authorized_committee_id_number` | COMMITTEE ID NUMBER (Auth) |
| 7 | `authorized_committee_name` | COMMITTEE NAME (Auth) |
| 8 | `col_a_cash_on_hand_beginning_period` | 6 Cash on Hand at Beginning of Reporting Period |
| 9 | `col_a_cash_on_hand_close_of_period` | 10 Cash on Hand at Close of Reporting Period |
| 10 | `col_a_debts_to` | 11 Debts and Obligations Owed TO the Committee |
| 11 | `col_a_debts_by` | 12 Debts and Obligations Owed BY the Committee |
| 12 | `col_a_expenditures_subject_to_limits` | 13 Expenditures Subject to Limitation |
| 13 | `col_a_net_contributions` | 14 Net Contributions |
| 14 | `col_a_net_operating_expenditures` | 15 Net Operating Expenditures |
| 15 | `col_a_federal_funds` | 16 Federal Funds |
| 16 | `col_a_individual_contributions` | 17(a)(iii) Contributions from Individuals/Persons Other Than Political Committees |
| 17 | `col_a_political_party_contributions` | 17(b) Contributions from Political Party Committees |
| 18 | `col_a_pac_contributions` | 17(c) Contributions from Other Political Committees |
| 19 | `col_a_candidate_contributions` | 17(d) Contributions from the Candidate |
| 20 | `col_a_total_contributions` | 17(e) Total Contributions |
| 21 | `col_a_transfers_from_authorized` | 18 Transfers from Other Authorized Committees |
| 22 | `col_a_candidate_loans` | 19(a) Loans Received From or Guaranteed by the Candidate |
| 23 | `col_a_other_loans` | 19(b) Other Loans |
| 24 | `col_a_total_loans` | 19(c) Total Loans |
| 25 | `col_a_offset_to_operating_expenditures` | 20(a) Offsets to Operating Expenditures |
| 26 | `col_a_offset_to_fundraising_expenditures` | 20(b) Offsets to Fundraising Expenditures |
| 27 | `col_a_offset_to_legal_expenditures` | 20(c) Offsets to Legal and Accounting Expenditures |
| 28 | `col_a_total_offsets_to_expenditures` | 20(d) Total Offsets to Expenditures |
| 29 | `col_a_other_receipts` | 21 Other Receipts |
| 30 | `col_a_total_receipts` | 22 Total Receipts |
| 31 | `col_a_operating_expenditures` | 23 Operating Expenditures |
| 32 | `col_a_transfers_to_authorized` | 24 Transfers to Other Authorized Committees |
| 33 | `col_a_fundraising_disbursements` | 25 Fundraising Disbursements |
| 34 | `col_a_exempt_legal_disbursements` | 26 Exempt Legal and Accounting Disbursements |
| 35 | `col_a_candidate_loan_repayments` | 27(a) Repayments of Loans Made or Guaranteed by Candidate |
| 36 | `col_a_other_loan_repayments` | 27(b) Other Loan Repayments |
| 37 | `col_a_total_loan_repayments_made` | 27(c) Total Loan Repayments Made |
| 38 | `col_a_refunds_to_individuals` | 28(a) Refunds of Contributions from Individuals/Persons |
| 39 | `col_a_refunds_to_party_committees` | 28(b) Refunds of Contributions from Political Party Committees |
| 40 | `col_a_refunds_to_other_committees` | 28(c) Refunds of Contributions from Other Political Committees |
| 41 | `col_a_total_refunds` | 28(d) Total Contributions Refunds |
| 42 | `col_a_other_disbursements` | 29 Other Disbursements |
| 43 | `col_a_total_disbursements` | 30 Total Disbursements |
| 44 | `col_a_items_on_hand_to_be_liquidated` | 31 Items on Hand to be Liquidated |

### F3PZ2

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER (PCC) |
| 3 | `principal_committee_name` | COMMITTEE NAME (PCC) |
| 4 | `coverage_from_date` | COVERAGE FROM DATE |
| 5 | `coverage_through_date` | COVERAGE THROUGH DATE |
| 6 | `col_a_cash_on_hand_beginning_period` | 6 Cash on Hand at Beginning of Reporting Period |
| 7 | `col_a_cash_on_hand_close_of_period` | 10 Cash on Hand at Close of Reporting Period |
| 8 | `col_a_debts_to` | 11 Debts and Obligations Owed TO the Committee |
| 9 | `col_a_debts_by` | 12 Debts and Obligations Owed BY the Committee |
| 10 | `col_a_expenditures_subject_to_limits` | 13 Expenditures Subject to Limitation |
| 11 | `col_a_net_contributions` | 14 Net Contributions |
| 12 | `col_a_net_operating_expenditures` | 15 Net Operating Expenditures |
| 13 | `col_a_federal_funds` | 16 Federal Funds |
| 14 | `col_a_individual_contributions` | 17(a)(iii) Contributions from Individuals/Persons Other Than Political Committees |
| 15 | `col_a_political_party_contributions` | 17(b) Contributions from Political Party Committees |
| 16 | `col_a_pac_contributions` | 17(c) Contributions from Other Political Committees |
| 17 | `col_a_candidate_contributions` | 17(d) Contributions from the Candidate |
| 18 | `col_a_total_contributions` | 17(e) Total Contributions |
| 19 | `col_a_transfers_from_authorized` | 18 Transfers from Other Authorized Committees |
| 20 | `col_a_candidate_loans` | 19(a) Loans Received From or Guaranteed by the Candidate |
| 21 | `col_a_other_loans` | 19(b) Other Loans |
| 22 | `col_a_total_loans` | 19(c) Total Loans |
| 23 | `col_a_offset_to_operating_expenditures` | 20(a) Offsets to Operating Expenditures |
| 24 | `col_a_offset_to_fundraising_expenditures` | 20(b) Offsets to Fundraising Expenditures |
| 25 | `col_a_offset_to_legal_expenditures` | 20(c) Offsets to Legal and Accounting Expenditures |
| 26 | `col_a_total_offsets_to_expenditures` | 20(d) Total Offsets to Expenditures |
| 27 | `col_a_other_receipts` | 21 Other Receipts |
| 28 | `col_a_total_receipts` | 22 Total Receipts |
| 29 | `col_a_operating_expenditures` | 23 Operating Expenditures |
| 30 | `col_a_transfers_to_authorized` | 24 Transfers to Other Authorized Committees |
| 31 | `col_a_fundraising_disbursements` | 25 Fundraising Disbursements |
| 32 | `col_a_exempt_legal_disbursements` | 26 Exempt Legal and Accounting Disbursements |
| 33 | `col_a_candidate_loan_repayments` | 27(a) Repayments of Loans Made or Guaranteed by Candidate |
| 34 | `col_a_other_loan_repayments` | 27(b) Other Loan Repayments |
| 35 | `col_a_total_loan_repayments_made` | 27(c) Total Loan Repayments Made |
| 36 | `col_a_refunds_to_individuals` | 28(a) Refunds of Contributions from Individuals/Persons |
| 37 | `col_a_refunds_to_party_committees` | 28(b) Refunds of Contributions from Political Party Committees |
| 38 | `col_a_refunds_to_other_committees` | 28(c) Refunds of Contributions from Other Political Committees |
| 39 | `col_a_total_refunds` | 28(d) Total Contributions Refunds |
| 40 | `col_a_other_disbursements` | 29 Other Disbursements |
| 41 | `col_a_total_disbursements` | 30 Total Disbursements |
| 42 | `col_a_items_on_hand_to_be_liquidated` | kk) 31 Items on Hand to be Liquidated |

### F3S

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `date_general_election` | Date - General Election |
| 4 | `date_day_after_general_election` | Date - Day after General Election |
| 5 | `6a_total_contributions_no_loans` | (6a) Total Contributions (No Loans) |
| 6 | `6b_total_contribution_refunds` | (6b) Total Contribution Refunds |
| 7 | `6c_net_contributions` | (6c) Net Contributions |
| 8 | `7a_total_operating_expenditures` | (7a) Total Operating Expenditures |
| 9 | `7b_total_offsets_to_operating_expenditures` | (7b) Total Offsets to Operating Expenditures |
| 10 | `7c_net_operating_expenditures` | (7c) NET Operating Expenditures. |
| 11 | `11_a_i_individuals_itemized` | 11(a i.) Individuals Itemized |
| 12 | `11_a_ii_individuals_unitemized` | 11(a.ii) Individuals Unitemized |
| 13 | `11_a_iii_individuals_total` | 11(a.iii) Individuals Total |
| 14 | `11_b_political_party_committees` | 11(b) Political Party Committees |
| 15 | `11_c_all_other_political_committees_pacs` | 11(c) All Other Political Committees (PACS) |
| 16 | `11_d_the_candidate` | 11(d) The Candidate |
| 17 | `11_e_total_contributions` | 11(e) Total Contributions |
| 18 | `12_transfers_from_other_auth_committees` | 12. Transfers From Other AUTH Committees |
| 19 | `13_a_loans_made_or_guarn_by_the_candidate` | 13(a) Loans made or guarn. by the Candidate |
| 20 | `13_b_all_other_loans` | 13(b) All Other Loans |
| 21 | `13_c_total_loans` | 13(c) Total Loans |
| 22 | `14_offsets_to_operating_expenditures` | 14. Offsets to Operating Expenditures |
| 23 | `15_other_receipts` | 15. Other Receipts |
| 24 | `16_total_receipts` | 16. Total Receipts |
| 25 | `17_operating_expenditures` | 17 Operating Expenditures |
| 26 | `18_transfers_to_other_auth_committees` | 18. Transfers To Other AUTH Committees |
| 27 | `19_a_loan_repayment_by_candidate` | 19(a) Loan Repayment By Candidate |
| 28 | `19_b_loan_repayments_all_other_loans` | 19(b) Loan Repayments, ALL Other Loans |
| 29 | `19_c_total_loan_repayments` | 19(c) Total Loan Repayments |
| 30 | `20_a_refund_individuals_other_than_pol_cmtes` | 20(a) Refund/Individuals Other than Pol. Cmtes |
| 31 | `20_b_refund_political_party_committees` | 20(b) Refund, Political Party Committees |
| 32 | `20_c_refund_other_political_committees` | 20(c) Refund, Other Political Committees |
| 33 | `20_d_total_contributions_refunds` | 20(d) Total Contributions Refunds |
| 34 | `21_other_disbursements` | 21. Other Disbursements |
| 35 | `22_total_disbursements` | 22. Total Disbursements |

### F3X

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `committee_name` | COMMITTEE NAME |
| 4 | `change_of_address` | CHANGE OF ADDRESS |
| 5 | `street_1` | STREET 1 |
| 6 | `street_2` | STREET 2 |
| 7 | `city` | CITY |
| 8 | `state` | STATE |
| 9 | `zip_code` | ZIP |
| 10 | `report_code` | REPORT CODE |
| 11 | `election_code` | ELECTION CODE {was RPTPGI} |
| 12 | `election_date` | DATE OF ELECTION |
| 13 | `state_of_election` | STATE OF ELECTION |
| 14 | `coverage_from_date` | COVERAGE FROM DATE |
| 15 | `coverage_through_date` | COVERAGE THROUGH DATE |
| 16 | `qualified_committee` | QUALIFIED COMMITTEE |
|  | `treasurer_name` | NAME/TREASURER (as signed) |
| 17 | `treasurer_last_name` | TREASURER LAST NAME |
| 18 | `treasurer_first_name` | TREASURER FIRST NAME |
| 19 | `treasurer_middle_name` | TREASURER MIDDLE NAME |
| 20 | `treasurer_prefix` | TREASURER PREFIX |
| 21 | `treasurer_suffix` | TREASURER SUFFIX |
| 22 | `date_signed` | DATE SIGNED |
| 23 | `col_a_cash_on_hand_beginning_period` | 6(b) Cash on Hand beginning |
| 24 | `col_a_total_receipts` | 6(c) Total Receipts |
| 25 | `col_a_subtotal` | 6(d) Subtotal |
| 26 | `col_a_total_disbursements` | 7. Total Disbursements |
| 27 | `col_a_cash_on_hand_close_of_period` | 8. Cash on Hand at Close |
| 28 | `col_a_debts_to` | 9. Debts to |
| 29 | `col_a_debts_by` | 10. Debts by |
| 30 | `col_a_individuals_itemized` | 11(a)i Itemized |
| 31 | `col_a_individuals_unitemized` | 11(a)ii Unitemized |
| 32 | `col_a_individual_contribution_total` | 11(a)iii Total |
| 33 | `col_a_political_party_contributions` | 11(b) Political Party Committees |
| 34 | `col_a_pac_contributions` | 11(c) Other Political Committees (PACs) |
| 35 | `col_a_total_contributions` | 11(d) Total Contributions |
| 36 | `col_a_transfers_from_aff_other_party_cmttees` | 12. Transfers from Affiliated/Other Party Cmtes |
| 37 | `col_a_total_loans` | 13. All Loans Received |
| 38 | `col_a_total_loan_repayments_received` | 14. Loan Repayments Received |
| 39 | `col_a_offsets_to_expenditures` | 15. Offsets to Operating Expenditures (refunds) |
| 40 | `col_a_refunds_of_federal_contributions` | 16. Refunds of Federal Contributions |
| 41 | `col_a_other_federal_receipts` | 17. Other Federal Receipts (dividends) |
| 42 | `col_a_transfers_from_nonfederal_h3` | 18(a) Transfers from Nonfederal Account (H3) |
| 43 | `col_a_levin_funds` | 18(b) Transfers from Non-Federal (Levin - H5) |
| 44 | `col_a_total_nonfederal_transfers` | 18(c) Total Non-Federal Transfers (18a+18b) |
| 45 | `col_a_total_receipts_recap` | 19. Total Receipts |
| 46 | `col_a_total_federal_receipts` | 20. Total Federal Receipts |
| 47 | `col_a_shared_operating_expenditures_federal` | 21(a)i Federal Share |
| 48 | `col_a_shared_operating_expenditures_nonfederal` | 21(a)ii Non-Federal Share |
| 49 | `col_a_other_federal_operating_expenditures` | 21(b) Other Federal Operating Expenditures |
| 50 | `col_a_total_operating_expenditures` | 21(c) Total Operating Expenditures |
| 51 | `col_a_transfers_to_affiliated` | 22. Transfers to Affiliated/Other Party Cmtes |
| 52 | `col_a_contributions_to_candidates` | 23. Contributions to Federal Candidates/Cmtes |
| 53 | `col_a_independent_expenditures` | 24. Independent Expenditures |
| 54 | `col_a_coordinated_expenditures_by_party_committees` | 25. Coordinated Expend made by Party Cmtes |
| 55 | `col_a_total_loan_repayments_made` | 26. Loan Repayments |
| 56 | `col_a_loans_made` | 27. Loans Made |
| 57 | `col_a_refunds_to_individuals` | 28(a) Individuals/Persons |
| 58 | `col_a_refunds_to_party_committees` | 28(b) Political Party Committees |
| 59 | `col_a_refunds_to_other_committees` | 28(c) Other Political Committees |
| 60 | `col_a_total_refunds` | 28(d) Total Contributions Refunds |
| 61 | `col_a_other_disbursements` | 29. Other Disbursements |
| 62 | `col_a_federal_election_activity_federal_share` | 30(a)i Shared Federal Activity (H6) Fed Share |
| 63 | `col_a_federal_election_activity_levin_share` | 30(a)ii Shared Federal Activity (H6) Non-Fed |
| 64 | `col_a_federal_election_activity_all_federal` | 30(b) Non-Allocable 100% Fed Election Activity |
| 65 | `col_a_federal_election_activity_total` | 30(c) Total Federal Election Activity |
| 66 | `col_a_total_disbursements_recap` | 31. Total Disbursements |
| 67 | `col_a_total_federal_disbursements` | 32. Total Federal Disbursements |
| 68 | `col_a_total_contributions_recap` | 33. Total Contributions |
| 69 | `col_a_total_contributions_refunds` | 34. Total Contribution Refunds |
| 70 | `col_a_net_contributions` | 35. Net Contributions |
| 71 | `col_a_total_federal_operating_expenditures` | 36. Total Federal Operating Expenditures |
| 72 | `col_a_total_offsets_to_expenditures` | 37. Offsets to Operating Expenditures |
| 73 | `col_a_net_operating_expenditures` | 38. Net Operating Expenditures |
| 74 | `col_b_cash_on_hand_jan_1` | 6(a) Cash on Hand Jan 1, 19 |
| 75 | `col_b_year` | Year for Above |
| 76 | `col_b_total_receipts` | 6(c) Total Receipts |
| 77 | `col_b_subtotal` | 6(d) Subtotal |
| 78 | `col_b_total_disbursements` | 7. Total disbursements |
| 79 | `col_b_cash_on_hand_close_of_period` | 8. Cash on Hand Close |
| 80 | `col_b_individuals_itemized` | 11(a)i Itemized |
| 81 | `col_b_individuals_unitemized` | 11(a)ii Unitemized |
| 82 | `col_b_individual_contribution_total` | 11(a)iii Total |
| 83 | `col_b_political_party_contributions` | 11(b) Political Party committees |
| 84 | `col_b_pac_contributions` | 11(c) Other Political Committees (PACs) |
| 85 | `col_b_total_contributions` | 11(d) Total Contributions |
| 86 | `col_b_transfers_from_aff_other_party_cmttees` | 12. Transfers from Affiliated/Other Party Cmtes |
| 87 | `col_b_total_loans` | 13. All Loans Received |
| 88 | `col_b_total_loan_repayments_received` | 14. Loan Repayments Received |
| 89 | `col_b_offsets_to_expenditures` | 15. Offsets to Operating Expenditures (refunds) |
| 90 | `col_b_refunds_of_federal_contributions` | 16. Refunds of Federal Contributions |
| 91 | `col_b_other_federal_receipts` | 17. Other Federal Receipts (dividends) |
| 92 | `col_b_transfers_from_nonfederal_h3` | 18(a) Transfers from Nonfederal Account (H3) |
| 93 | `col_b_levin_funds` | 18(b) Transfers from Non-Federal (Levin - H5) |
| 94 | `col_b_total_nonfederal_transfers` | 18(c) Total Non-Federal Transfers (18a+18b) |
| 95 | `col_b_total_receipts_recap` | 19. Total Receipts |
| 96 | `col_b_total_federal_receipts` | 20. Total Federal Receipts |
| 97 | `col_b_shared_operating_expenditures_federal` | 21(a)i Federal Share |
| 98 | `col_b_shared_operating_expenditures_nonfederal` | 21(a)ii Non-Federal Share |
| 99 | `col_b_other_federal_operating_expenditures` | 21(b) Other Federal Operating Expenditures |
| 100 | `col_b_total_operating_expenditures` | 21(c) Total operating Expenditures |
| 101 | `col_b_transfers_to_affiliated` | 22. Transfers to Affiliated/Other Party Cmtes |
| 102 | `col_b_contributions_to_candidates` | 23. Contributions to Federal Candidates/Cmtes |
| 103 | `col_b_independent_expenditures` | 24. Independent Expenditures |
| 104 | `col_b_coordinated_expenditures_by_party_committees` | 25. Coordinated Expend made by Party Cmtes |
| 105 | `col_b_total_loan_repayments_made` | 26. Loan Repayments Made |
| 106 | `col_b_loans_made` | 27. Loans Made |
| 107 | `col_b_refunds_to_individuals` | 28(a) Individuals/Persons |
| 108 | `col_b_refunds_to_party_committees` | 28(b) Political Party Committees |
| 109 | `col_b_refunds_to_other_committees` | 28(c) Other Political Committees |
| 110 | `col_b_total_refunds` | 28(d) Total contributions Refunds |
| 111 | `col_b_other_disbursements` | 29. Other Disbursements |
| 112 | `col_b_federal_election_activity_federal_share` | 30(a)i Shared Federal Activity (H6) Fed Share |
| 113 | `col_b_federal_election_activity_levin_share` | 30(a)ii Shared Federal Activity (H6) Non-Fed |
| 114 | `col_b_federal_election_activity_all_federal` | 30(b) Non-Allocable 100% Fed Election Activity |
| 115 | `col_b_federal_election_activity_total` | 30(c) Total Federal Election Activity |
| 116 | `col_b_total_disbursements_recap` | 31. Total Disbursements |
| 117 | `col_b_total_federal_disbursements` | 32. Total Federal Disbursements |
| 118 | `col_b_total_contributions_recap` | 33. Total Contributions |
| 119 | `col_b_total_contributions_refunds` | 34. Total Contribution Refunds |
| 120 | `col_b_net_contributions` | 35. Net contributions |
| 121 | `col_b_total_federal_operating_expenditures` | 36. Total Federal Operating Expenditures |
| 122 | `col_b_total_offsets_to_expenditures` | 37. Offsets to Operating Expenditures |
| 123 | `col_b_net_operating_expenditures` | 38. Net Operating Expenditures |

### F3Z

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `principal_committee_name` | PRINCIPAL COMMITTEE NAME |
| 4 | `coverage_from_date` | COVERAGE FROM DATE |
| 5 | `coverage_through_date` | COVERAGE THROUGH DATE |
| 6 | `authorized_committee_id_number` | AUTHORIZED COMMITTEE ID NUMBER |
| 7 | `authorized_committee_name` | AUTHORIZED COMMITTEE NAME |
| 8 | `col_a_individuals_itemized` | 11(a i.) Individuals Itemized |
| 9 | `col_a_political_party_contributions` | 11(b) Political Party Committees |
| 10 | `col_a_pac_contributions` | 11(c) Other Political Committees |
| 11 | `col_a_candidate_contributions` | 11(d) The Candidate |
| 12 | `col_a_total_contributions` | 11(e) Total Contributions |
| 13 | `col_a_transfers_from_authorized` | 12. Transfers  From Other Authorized Cmttes |
| 14 | `col_a_candidate_loans` | 13(a) Loans made or guarn. by the Candidate |
| 15 | `col_a_other_loans` | 13(b) All Other Loans |
| 16 | `col_a_total_loans` | 13(c) Total Loans |
| 17 | `col_a_offset_to_operating_expenditures` | 14. Offsets to Operating Expenditures |
| 18 | `col_a_other_receipts` | 15. Other Receipts |
| 19 | `col_a_total_receipts` | 16. Total Receipts |
| 20 | `col_a_operating_expenditures` | 17. Operating Expenditures |
| 21 | `col_a_transfers_to_authorized` | 18. Transfers to Other Authorized Committees |
| 22 | `col_a_candidate_loan_repayments` | 19(a) Of Loans made or guar. by the Cand. |
| 23 | `col_a_other_loan_repayments` | 19(b) Loan Repayments, All Other Loans |
| 24 | `col_a_total_loan_repayments` | 19(c) Total Loan Repayments |
| 25 | `col_a_refunds_to_individuals` | 20(a) Refund/Individuals Other than Pol. Cmtes |
| 26 | `col_a_refunds_to_party_committees` | 20(b) Refund/Political Party Committees |
| 27 | `col_a_refunds_to_other_committees` | 20(c) Refund/Other Political Committees |
| 28 | `col_a_total_refunds` | 20(d) Total Contribution Refunds |
| 29 | `col_a_other_disbursements` | 21. Other Disbursements |
| 30 | `col_a_total_disbursements` | 22. Total Disbursements |
| 31 | `col_a_cash_on_hand_beginning_period` | 23. Cash Beginning Reporting Period |
| 32 | `col_a_cash_on_hand_close` | 27. Cash on hand at Close Period |
| 33 | `col_a_debts_to` | 9. DEBTS TO ( Totals from SCH C and/or D) |
| 34 | `col_a_debts_by` | 10. DEBTS BY (Totals from SCH C and/or D) |
| 35 | `col_a_net_contributions` | (6c) Net Contributions |
| 36 | `col_a_net_operating_expenditures` | (7c) NET Operating Expenditures. |

### F3Z1

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER (PCC) |
| 3 | `principal_committee_name` | COMMITTEE NAME (PCC) |
| 4 | `coverage_from_date` | COVERAGE FROM DATE |
| 5 | `coverage_through_date` | COVERAGE THROUGH DATE |
| 6 | `authorized_committee_id_number` | COMMITTEE ID NUMBER (Auth) |
| 7 | `authorized_committee_name` | COMMITTEE NAME (Auth) |
| 8 | `col_a_net_contributions` | 6(c) Net Contributions |
| 9 | `col_a_net_operating_expenditures` | 7(c) Net Operating Expenditures |
| 10 | `col_a_debts_to` | 9 Debts and Obligations Owed TO the Committee |
| 11 | `col_a_debts_by` | 10 Debts and Obligations Owed BY the Committee |
| 12 | `col_a_individual_contributions` | 11(a) Contributions from Individuals/Persons Other Than Political Committees |
| 13 | `col_a_political_party_contributions` | 11(b) Contributions from Political Party Committees |
| 14 | `col_a_pac_contributions` | 11(c) Contributions from Other Political Committees |
| 15 | `col_a_candidate_contributions` | 11(d) Contributions from the Candidate |
| 16 | `col_a_total_contributions` | 11(e) Total Contributions |
| 17 | `col_a_transfers_from_authorized` | 12 Transfers from Other Authorized Committees |
| 18 | `col_a_candidate_loans` | 13(a) Loans Made or Guaranteed by the Candidate |
| 19 | `col_a_other_loans` | 13(b) All Other Loans |
| 20 | `col_a_total_loans` | 13(c) Total Loans |
| 21 | `col_a_offset_to_operating_expenditures` | 14 Offsets to Operating Expenditures |
| 22 | `col_a_other_receipts` | 15 Other Receipts |
| 23 | `col_a_total_receipts` | 16 Total Receipts |
| 24 | `col_a_operating_expenditures` | 17 Operating Expenditures |
| 25 | `col_a_transfers_to_authorized` | 18 Transfers to Other Authorized Committees |
| 26 | `col_a_candidate_loan_repayments` | 19(a) Repayments of Loans Made or Guaranteed by Candidate |
| 27 | `col_a_other_loan_repayments` | 19(b) Other Loan Repayments |
| 28 | `col_a_total_loan_repayments` | 19(c) Total Loan Repayments |
| 29 | `col_a_refunds_to_individuals` | 20(a) Refunds of Contributions to Individuals/Persons |
| 30 | `col_a_refunds_to_party_committees` | 20(b) Refunds of Contributions to Political Party Committees |
| 31 | `col_a_refunds_to_other_committees` | 20(c) Refunds of Contributions to Other Political Committees |
| 32 | `col_a_total_refunds` | 20(d) Total Contributions Refunds |
| 33 | `col_a_other_disbursements` | 21 Other Disbursements |
| 34 | `col_a_total_disbursements` | 22 Total Disbursements |
| 35 | `col_a_cash_on_hand_beginning_period` | 23 Cash on Hand at Beginning of Reporting Period |
| 36 | `col_a_cash_on_hand_close` | 27 Cash on Hand at Close of Reporting Period |

### F3Z2

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER (PCC) |
| 3 | `principal_committee_name` | COMMITTEE NAME (PCC) |
| 4 | `coverage_from_date` | COVERAGE FROM DATE |
| 5 | `coverage_through_date` | COVERAGE THROUGH DATE |
| 6 | `col_a_net_contributions` | 6(c) Net Contributions |
| 7 | `col_a_net_operating_expenditures` | 7(c) Net Operating Expenditures |
| 8 | `col_a_debts_to` | 9 Debts and Obligations Owed TO the Committee |
| 9 | `col_a_debts_by` | 10 Debts and Obligations Owed BY the Committee |
| 10 | `col_a_individual_contributions` | 11(a) Contributions from Individuals/Persons Other Than Political Committees |
| 11 | `col_a_political_party_contributions` | 11(b) Contributions from Political Party Committees |
| 12 | `col_a_pac_contributions` | 11(c) Contributions from Other Political Committees |
| 13 | `col_a_candidate_contributions` | 11(d) Contributions from the Candidate |
| 14 | `col_a_total_contributions` | 11(e) Total Contributions |
| 15 | `col_a_transfers_from_authorized` | 12 Transfers from Other Authorized Committees |
| 16 | `col_a_candidate_loans` | 13(a) Loans Made or Guaranteed by the Candidate |
| 17 | `col_a_other_loans` | 13(b) All Other Loans |
| 18 | `col_a_total_loans` | 13(c) Total Loans |
| 19 | `col_a_offset_to_operating_expenditures` | 14 Offsets to Operating Expenditures |
| 20 | `col_a_other_receipts` | 15 Other Receipts |
| 21 | `col_a_total_receipts` | 16 Total Receipts |
| 22 | `col_a_operating_expenditures` | 17 Operating Expenditures |
| 23 | `col_a_transfers_to_authorized` | 18 Transfers to Other Authorized Committees |
| 24 | `col_a_candidate_loan_repayments` | 19(a) Repayments of Loans Made or Guaranteed by Candidate |
| 25 | `col_a_other_loan_repayments` | 19(b) Other Loan Repayments |
| 26 | `col_a_total_loan_repayments` | 19(c) Total Loan Repayments |
| 27 | `col_a_refunds_to_individuals` | 20(a) Refunds of Contributions to Individuals/Persons |
| 28 | `col_a_refunds_to_party_committees` | 20(b) Refunds of Contributions to Political Party Committees |
| 29 | `col_a_refunds_to_other_committees` | 20(c) Refunds of Contributions to Other Political Committees |
| 30 | `col_a_total_refunds` | 20(d) Total Contributions Refunds |
| 31 | `col_a_other_disbursements` | 21 Other Disbursements |
| 32 | `col_a_total_disbursements` | 22 Total Disbursements |
| 33 | `col_a_cash_on_hand_beginning_period` | 23 Cash on Hand at Beginning of Reporting Period |
| 34 | `col_a_cash_on_hand_close` | 27 Cash on Hand at Close of Reporting Period |

### F4

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `committee_name` | COMMITTEE NAME |
| 4 | `street_1` | STREET 1 |
| 5 | `street_2` | STREET 2 |
| 6 | `city` | CITY |
| 7 | `state` | STATE |
| 8 | `zip_code` | ZIP |
| 9 | `committee_type` | COMMITTEE/ORG TYPE |
| 10 | `committee_type_description` | COMMITTEE/ORG TYPE - OTHER DESCRIPTION |
| 11 | `report_code` | REPORT CODE |
| 12 | `coverage_from_date` | COVERAGE FROM DATE |
| 13 | `coverage_through_date` | COVERAGE THROUGH DATE |
| 14 | `treasurer_last_name` | TREASURER LAST NAME |
| 15 | `treasurer_first_name` | TREASURER FIRST NAME |
| 16 | `treasurer_middle_name` | TREASURER MIDDLE NAME |
| 17 | `treasurer_prefix` | TREASURER PREFIX |
| 18 | `treasurer_suffix` | TREASURER SUFFIX |
| 19 | `date_signed` | DATE SIGNED |
| 20 | `col_a_cash_on_hand_beginning_period` | 6.(b) cash on hand beg. report per. |
| 21 | `col_a_total_receipts` | 6.(c) Total receipts |
| 22 | `col_a_subtotal` | 6.(d) Subtotal |
| 23 | `col_a_total_disbursements` | 7. Total disbursements |
| 24 | `col_a_cash_on_hand_close_of_period` | 8. COH CLOSE (COL A) |
| 25 | `col_a_debts_to` | 9. Debts to |
| 26 | `col_a_debts_by` | 10. Debts by |
| 27 | `col_a_convention_expenditures` | 11. Convention expenditures |
| 28 | `col_a_convention_refunds` | 12. refunds/rebates/returns relating to conv exp. |
| 29 | `col_a_expenditures_subject_to_limits` | 12(a) Expenditures subject to limits |
| 30 | `col_a_prior_expenditures_subject_to_limits` | 12(b) expend. from prior years subject to limits |
|  | `col_a_total_expenditures_subject_to_limits` | 12(c) total expenditures subject to limits |
| 31 | `col_a_federal_funds` | 13. Federal Funds SCH A |
| 32 | `col_a_contributions_itemized` | 14(a) Itemized |
| 33 | `col_a_contributions_unitemized` | 14(b) unitemized |
| 34 | `col_a_contributions_subtotal` | 14(c) subtotal |
| 35 | `col_a_transfers_from_affiliated` | 15. Transfers from affiliated cmtes. |
| 36 | `col_a_loans_received` | 16(a) loans received |
| 37 | `col_a_loan_repayments_received` | 16(b) loan repayments received |
| 38 | `col_a_loan_receipts_subtotal` | 16(c) subtotal loans/repayments |
| 39 | `col_a_convention_refunds_itemized` | 17(a) Itemized |
| 40 | `col_a_convention_refunds_unitemized` | 17(b) unitemized |
| 41 | `col_a_convention_refunds_subtotal` | 17(c) subtotal |
| 42 | `col_a_other_refunds_itemized` | 18(a) Itemized |
| 43 | `col_a_other_refunds_unitemized` | 18(b) unitemized |
| 44 | `col_a_other_refunds_subtotal` | 18(c) subtotal |
| 45 | `col_a_other_income_itemized` | 19(a) Itemized |
| 46 | `col_a_other_income_unitemized` | 19(b) unitemized |
| 47 | `col_a_other_income_subtotal` | 19(c) subtotal |
| 48 | `col_a_total_receipts_recap` | 20. total receipts |
| 49 | `col_a_convention_expenses_itemized` | 21(a) Itemized |
| 50 | `col_a_convention_expenses_unitemized` | 21(b) unitemized |
| 51 | `col_a_convention_expenses_subtotal` | 21(c) subtotal |
| 52 | `col_a_transfers_to_affiliated` | 22. Transfers to Affiliated Cmtes |
| 53 | `col_a_loans_made` | 23(a) loans made |
| 54 | `col_a_loan_repayments_made` | 23(b) loan repayments made |
| 55 | `col_a_loan_disbursements_subtotal` | 23(c) subtotal |
| 56 | `col_a_other_disbursements_itemized` | 24(a) Itemized |
| 57 | `col_a_other_disbursements_unitemized` | 24(b) unitemized |
| 58 | `col_a_other_disbursements_subtotal` | 24(c) subtotal |
| 59 | `col_a_total_disbursements_recap` | 25. Total disbursements |
| 60 | `col_b_cash_on_hand_beginning_year` | 6.(a) Cash on Hand |
| 61 | `col_b_beginning_year` | 6.(a) 19 -- (YEAR) |
| 62 | `col_b_total_receipts` | 6.(c) Total receipts |
| 63 | `col_b_subtotal` | 6.(d) Subtotal |
| 64 | `col_b_total_disbursements` | 7. Total disbursements |
| 65 | `col_b_cash_on_hand_close_of_period` | 8. COH CLOSE (COL B) |
| 66 | `col_b_convention_expenditures` | 11. Convention expenditures |
| 67 | `col_b_convention_refunds` | 12. refunds/rebates/returns relating to conv exp. |
| 68 | `col_b_expenditures_subject_to_limits` | 12(a) Expenditures subject to limits |
| 69 | `col_b_prior_expenditures_subject_to_limits` | 12(b) expend. from prior years subject to limits |
| 70 | `col_b_total_expenditures_subject_to_limits` | 12(c) total expenditures subject to limits |
| 71 | `col_b_federal_funds` | 13. Federal Funds |
| 72 | `col_b_contributions_subtotal` | 14(c) subtotal |
| 73 | `col_b_transfers_from_affiliated` | 15. Transfers from affiliated cmtes. |
| 74 | `col_b_loan_receipts_subtotal` | 16(c) subtotal loans/repayments |
| 75 | `col_b_convention_refunds_subtotal` | 17(c) subtotal |
| 76 | `col_b_other_refunds_subtotal` | 18(c) subtotal |
| 77 | `col_b_other_income_subtotal` | 19(c) subtotal |
| 78 | `col_b_total_receipts_recap` | 20. total receipts |
| 79 | `col_b_convention_expenses_subtotal` | 21(c) subtotal |
| 80 | `col_b_transfers_to_affiliated` | 22. Transfers to Affiliated Cmtes |
| 81 | `col_b_loan_disbursements_subtotal` | 23(c) subtotal |
| 82 | `col_b_other_disbursements_subtotal` | 24(c) subtotal |
| 83 | `col_b_total_disbursements_recap` | 25. Total disbursements |
|  | `treasurer_name` | NAME/TREASURER (as signed) |

### F5

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `entity_type` | ENTITY TYPE |
|  | `committee_name` | COMMITTEE NAME |
| 4 | `organization_name` | ORGANIZATION NAME |
| 5 | `individual_last_name` | INDIVIDUAL LAST NAME |
| 6 | `individual_first_name` | INDIVIDUAL FIRST NAME |
| 7 | `individual_middle_name` | INDIVIDUAL MIDDLE NAME |
| 8 | `individual_prefix` | INDIVIDUAL PREFIX |
| 9 | `individual_suffix` | INDIVIDUAL SUFFIX |
| 10 | `change_of_address` | CHANGE OF ADDRESS |
| 11 | `street_1` | STREET 1 |
| 12 | `street_2` | STREET 2 |
| 13 | `city` | CITY |
| 14 | `state` | STATE |
| 15 | `zip_code` | ZIP |
|  | `qualified_nonprofit` | YES/NO (Qualified Non-Profit Corporation) |
| 17 | `individual_employer` | INDIVIDUAL EMPLOYER |
| 16 | `individual_occupation` | INDIVIDUAL OCCUPATION |
| 18 | `report_code` | REPORT CODE |
| 19 | `report_type` | 24HOUR 48HOUR CODE |
| 20 | `original_amendment_date` | ORIGINAL AMENDMENT DATE |
|  | `election_code` | RPTPGI |
| 21 | `coverage_from_date` | COVERAGE FROM DATE |
| 22 | `coverage_through_date` | COVERAGE THROUGH DATE |
|  | `election_date` | DATE (Of Election) |
|  | `state_of_election` | STATE (Of Election) |
| 23 | `total_contribution` | TOTAL CONTRIBUTION |
| 24 | `total_independent_expenditure` | TOTAL INDEPENDENT EXPENDITURE |
|  | `person_completing_name` | IND/NAME (Person Completing Form) |
| 25 | `person_completing_last_name` | PERSON COMPLETING LAST NAME |
| 26 | `person_completing_first_name` | PERSON COMPLETING FIRST NAME |
| 27 | `person_completing_middle_name` | PERSON COMPLETING MIDDLE NAME |
| 28 | `person_completing_prefix` | PERSON COMPLETING PREFIX |
| 29 | `person_completing_suffix` | PERSON COMPLETING SUFFIX |
| 30 | `date_signed` | DATE SIGNED |
|  | `date_notarized` | DATE (Notarized) |
|  | `date_notary_commission_expires` | DATE (Notary Commission Expires) |
|  | `notary_name` | IND/NAME (Notary) |

### F56

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `entity_type` | ENTITY TYPE |
| 5 | `contributor_organization_name` | CONTRIBUTOR ORGANIZATION NAME |
|  | `contributor_name` | NAME (Contributor/Lender) |
| 6 | `contributor_last_name` | CONTRIBUTOR LAST NAME |
| 7 | `contributor_first_name` | CONTRIBUTOR FIRST NAME |
| 8 | `contributor_middle_name` | CONTRIBUTOR MIDDLE NAME |
| 9 | `contributor_prefix` | CONTRIBUTOR PREFIX |
| 10 | `contributor_suffix` | CONTRIBUTOR SUFFIX |
| 11 | `contributor_street_1` | CONTRIBUTOR STREET 1 |
| 12 | `contributor_street_2` | CONTRIBUTOR STREET 2 |
| 13 | `contributor_city` | CONTRIBUTOR CITY |
| 14 | `contributor_state` | CONTRIBUTOR STATE |
| 15 | `contributor_zip_code` | CONTRIBUTOR ZIP |
| 16 | `contributor_fec_id` | CONTRIBUTOR COMMITTEE FEC ID |
| 17 | `contribution_date` | CONTRIBUTION DATE |
| 18 | `contribution_amount` | CONTRIBUTION AMOUNT |
| 19 | `contributor_employer` | CONTRIBUTOR EMPLOYER |
| 20 | `contributor_occupation` | CONTRIBUTOR OCCUPATION |
|  | `candidate_id_number` | FEC CANDIDATE ID NUMBER |
|  | `candidate_name` | CANDIDATE NAME |
|  | `candidate_office` | CAN/OFFICE |
|  | `candidate_state` | CAN/STATE |
|  | `candidate_district` | CAN/DIST |
|  | `conduit_name` | CONDUIT NAME |
|  | `conduit_street_1` | CONDUIT STREET 1 |
|  | `conduit_street_2` | CONDUIT STREET 2 |
|  | `conduit_city` | CONDUIT CITY |
|  | `conduit_state` | CONDUIT STATE |
|  | `conduit_zip_code` | CONDUIT ZIP |

### F57

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `entity_type` | ENTITY TYPE |
| 5 | `payee_organization_name` | PAYEE ORGANIZATION NAME |
|  | `payee_name` | NAME (Payee) |
| 6 | `payee_last_name` | PAYEE LAST NAME |
| 7 | `payee_first_name` | PAYEE FIRST NAME |
| 8 | `payee_middle_name` | PAYEE MIDDLE NAME |
| 9 | `payee_prefix` | PAYEE PREFIX |
| 10 | `payee_suffix` | PAYEE SUFFIX |
| 11 | `payee_street_1` | PAYEE STREET 1 |
| 12 | `payee_street_2` | PAYEE STREET 2 |
| 13 | `payee_city` | PAYEE CITY |
| 14 | `payee_state` | PAYEE STATE |
| 15 | `payee_zip_code` | PAYEE ZIP |
| 16 | `election_code` | ELECTION CODE |
| 17 | `election_other_description` | ELECTION OTHER DESCRIPTION |
| 18 | `dissemination_date` | DISSEMINATION DATE |
| 19 | `expenditure_amount` | EXPENDITURE AMOUNT |
| 20 | `calendar_y_t_d_per_election_office` | CALENDAR Y-T-D (per election/office) |
|  | `expenditure_purpose_code` | EXPENDITURE PURPOSE CODE |
| 21 | `expenditure_purpose_descrip` | EXPENDITURE PURPOSE DESCRIP |
| 22 | `category_code` | CATEGORY CODE |
| 23 | `payee_committee_id_number` | PAYEE CMTTE FEC ID NUMBER |
| 24 | `support_oppose_code` | SUPPORT/OPPOSE CODE |
| 25 | `candidate_id_number` | S/O CANDIDATE ID NUMBER |
|  | `candidate_name` | S/O CAN/NAME |
| 26 | `candidate_last_name` | S/O CANDIDATE LAST NAME |
| 27 | `candidate_first_name` | S/O CANDIDATE FIRST NAME |
| 28 | `candidate_middle_name` | S/O CANDINATE MIDDLE NAME |
| 29 | `candidate_prefix` | S/O CANDIDATE PREFIX |
| 30 | `candidate_suffix` | S/O CANDIDATE SUFFIX |
| 31 | `candidate_office` | S/O CANDIDATE OFFICE |
| 32 | `candidate_state` | S/O CANDIDATE STATE |
| 33 | `candidate_district` | S/O CANDIDATE DISTRICT |
|  | `conduit_name` | CONDUIT NAME |
|  | `conduit_street_1` | CONDUIT STREET 1 |
|  | `conduit_street_2` | CONDUIT STREET 2 |
|  | `conduit_city` | CONDUIT CITY |
|  | `conduit_state` | CONDUIT STATE |
|  | `conduit_zip_code` | CONDUIT ZIP |
|  | `amended_cd` | AMENDED CD |

### F6

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `original_amendment_date` | ORIGINAL AMENDMENT DATE |
| 4 | `committee_name` | COMMITTEE NAME |
| 5 | `street_1` | STREET 1 |
| 6 | `street_2` | STREET 2 |
| 7 | `city` | CITY |
| 8 | `state` | STATE |
| 9 | `zip_code` | ZIP |
| 10 | `candidate_id_number` | CANDIDATE ID NUMBER |
|  | `candidate_name` | CANDIDATE NAME |
| 11 | `candidate_last_name` | CANDIDATE LAST NAME |
| 12 | `candidate_first_name` | CANDIDATE FIRST NAME |
| 13 | `candidate_middle_name` | CANDIDATE MIDDLE NAME |
| 14 | `candidate_prefix` | CANDIDATE PREFIX |
| 15 | `candidate_suffix` | CANDIDATE SUFFIX |
| 16 | `candidate_office` | CANDIDATE OFFICE |
| 17 | `candidate_state` | CANDIDATE STATE |
| 18 | `candidate_district` | CANDIDATE DISTRICT |
| 19 | `signer_last_name` | SIGNER LAST NAME |
| 20 | `signer_first_name` | SIGNER FIRST NAME |
| 21 | `signer_middle_name` | SIGNER MIDDLE NAME |
| 22 | `signer_prefix` | SIGNER PREFIX |
| 23 | `signer_suffix` | SIGNER SUFFIX |
| 24 | `date_signed` | DATE SIGNED |

### F65

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `entity_type` | ENTITY TYPE |
| 5 | `contributor_organization_name` | CONTRIBUTOR ORGANIZATION NAME |
|  | `contributor_name` | NAME (Contributor/Lender) |
| 6 | `contributor_last_name` | CONTRIBUTOR LAST NAME |
| 7 | `contributor_first_name` | CONTRIBUTOR FIRST NAME |
| 8 | `contributor_middle_name` | CONTRIBUTOR MIDDLE NAME |
| 9 | `contributor_prefix` | CONTRIBUTOR PREFIX |
| 10 | `contributor_suffix` | CONTRIBUTOR SUFFIX |
| 11 | `contributor_street_1` | CONTRIBUTOR STREET 1 |
| 12 | `contributor_street_2` | CONTRIBUTOR STREET 2 |
| 13 | `contributor_city` | CONTRIBUTOR CITY |
| 14 | `contributor_state` | CONTRIBUTOR STATE |
| 15 | `contributor_zip_code` | CONTRIBUTOR ZIP |
| 16 | `contributor_fec_id` | CONTRIBUTOR COMMITTEE FEC ID |
| 17 | `contribution_date` | CONTRIBUTION DATE |
| 18 | `contribution_amount` | CONTRIBUTION AMOUNT |
| 19 | `contributor_employer` | CONTRIBUTOR EMPLOYER |
| 20 | `contributor_occupation` | CONTRIBUTOR OCCUPATION |
|  | `candidate_id_number` | FEC CANDIDATE ID NUMBER |
|  | `candidate_name` | CANDIDATE NAME |
|  | `candidate_office` | CAN/OFFICE |
|  | `candidate_state` | CAN/STATE |
|  | `candidate_district` | CAN/DIST |
|  | `conduit_name` | CONDUIT NAME |
|  | `conduit_street_1` | CONDUIT STREET 1 |
|  | `conduit_street_2` | CONDUIT STREET 2 |
|  | `conduit_city` | CONDUIT CITY |
|  | `conduit_state` | CONDUIT STATE |
|  | `conduit_zip_code` | CONDUIT ZIP |

### F7

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `organization_name` | ORGANIZATION NAME |
| 4 | `street_1` | ORGANIZATION STREET 1 |
| 5 | `street_2` | ORGANIZATION STREET 2 |
| 6 | `city` | ORGANIZATION CITY |
| 7 | `state` | ORGANIZATION STATE |
| 8 | `zip_code` | ORGANIZATION ZIP |
| 9 | `organization_type` | ORGANIZATION TYPE |
| 10 | `report_code` | REPORT CODE |
| 11 | `election_date` | DATE OF ELECTION |
| 12 | `state_of_election` | STATE OF ELECTION |
| 13 | `coverage_from_date` | COVERAGE FROM DATE |
| 14 | `coverage_through_date` | COVERAGE THROUGH DATE |
| 15 | `total_costs` | TOTAL COSTS |
|  | `person_designated_name` | NAME/FILER (as signed) |
| 16 | `person_designated_last_name` | PERSON DESIGNATED TO SIGN LAST NAME |
| 17 | `person_designated_first_name` | PERSON DESIGNATED TO SIGN FIRST NAME |
| 18 | `person_designated_middle_name` | PERSON DESIGNATED TO SIGN MIDDLE NAME |
| 19 | `person_designated_prefix` | PERSON DESIGNATED TO SIGN PREFIX |
| 20 | `person_designated_suffix` | PERSON DESIGNATED TO SIGN SUFFIX |
| 21 | `person_designated_title` | PERSON DESIGNATED TO SIGN TITLE |
| 22 | `date_signed` | DATE SIGNED |

### F76

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `communication_type` | COMMUNICATION TYPE |
| 5 | `communication_type_description` | COMMUNICATION TYPE - OTHER DESCRIPTION |
| 6 | `communication_class` | COMMUNICATION CLASS |
| 7 | `communication_date` | COMMUNICATION DATE |
| 8 | `communication_cost` | COMMUNICATION COST (per candidate) |
| 9 | `election_code` | ELECTION CODE |
| 10 | `election_other_description` | ELECTION OTHER DESCRIPTION |
| 11 | `support_oppose_code` | SUPPORT/OPPOSE |
| 12 | `candidate_id_number` | S/O CANDIDATE ID NUMBER |
|  | `candidate_name` | CANDIDATE NAME |
| 13 | `candidate_last_name` | S/O CANDIDATE LAST NAME |
| 14 | `candidate_first_name` | S/O CANDIDATE FIRST NAME |
| 15 | `candidate_middle_name` | S/O CANDIDATE MIDDLE NAME |
| 16 | `candidate_prefix` | S/O CANDIDATE PREFIX |
| 17 | `candidate_suffix` | S/O CANDIDATE SUFFIX |
| 18 | `candidate_office` | S/O CANDIDATE OFFICE |
| 19 | `candidate_state` | S/O CANDIDATE STATE |
| 20 | `candidate_district` | S/O CANDIDATE DISTRICT |

### F8

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `committee_name` | COMMITTEE NAME |
| 4 | `street_1` | STREET 1 |
| 5 | `street_2` | STREET 2 |
| 6 | `city` | CITY |
| 7 | `state` | STATE |
| 8 | `zip_code` | ZIP |
| 9 | `cash_on_hand` | 1. CASH ON HAND |
| 10 | `cash_on_hand_as_of_date` | 1(a). AS OF |
| 11 | `total_assets_to_be_liquidated` | 2. Total assets to be liquidated |
| 12 | `total_assets` | 3. Total (assets) |
| 13 | `receipts_ytd` | 4. Year to date receipts |
| 14 | `disbursements_ytd` | 5. Year to date disbursements |
| 15 | `total_debts_owed` | 6. Total amount of debts owed by committee |
| 16 | `total_num_creditors_owed` | 7. Total number of creditors owed |
| 17 | `num_creditors_part_ii` | 8. Number of creditors in part II of this plan |
| 18 | `total_debts_owed_part_ii` | 9. Total amount of debts owed to creditors in part II of plan |
| 19 | `total_to_be_paid_to_creditors` | 10. Total amount to be paid to creditors |
| 20 | `committee_is_terminating_activities` | 11. YES/NO (Is the committee terminating activities) |
| 21 | `planned_termination_report_date` | 11. YES DATE (PLANNED FOR TERMINATION REPORT) |
| 22 | `other_auth_committees` | 12. YES/NO (If this is an AUTH committee are there other AUTH committees) |
| 23 | `other_auth_committees_description` | 12. DESCRIPTION (IF YES list AUTH COMMITTEE ID/NAMES) |
| 24 | `sufficient_funds_to_pay_total` | 13. YES/NO (sufficient funds to pay total amount) |
| 25 | `steps_taken_description` | 13. DESCRIPTION (IF NO, steps taken to obtain the funds) |
| 26 | `committee_filed_previous_plans` | 14. YES/NO (Has the committee filed previous plans) |
| 27 | `residual_funds` | 15. YES/NO (After disposing, any residual funds?) |
| 28 | `residual_funds_description` | 15. DESCRIPTION (IF YES, how will the funds be disbursed) |
| 29 | `sufficient_funds_part_iii` | PART III YES/NO (Does committee have sufficient funds to pay the remaining) |
| 30 | `sufficient_funds_part_iii_description` | PART III DESCRIPTION (IF NO, steps taken to obtain the funds.) |
| 31 | `treasurer_last_name` | TREASURER LAST NAME |
| 32 | `treasurer_first_name` | TREASURER FIRST NAME |
| 33 | `treasurer_middle_name` | TREASURER MIDDLE NAME |
| 34 | `treasurer_prefix` | TREASURER PREFIX |
| 35 | `treasurer_suffix` | TREASURER SUFFIX |
| 36 | `date_signed` | DATE SIGNED |
|  | `treasurer_name` | NAME/TREASURER (as signed) |

### F82

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `entity_type` | ENTITY TYPE |
| 5 | `creditor_organization_name` | CREDITOR ORGANIZATION NAME |
| 6 | `creditor_last_name` | CREDITOR LAST NAME |
| 7 | `creditor_first_name` | CREDITOR FIRST NAME |
| 8 | `creditor_middle_name` | CREDITOR MIDDLE NAME |
| 9 | `creditor_prefix` | CREDITOR PREFIX |
| 10 | `creditor_suffix` | CREDITOR SUFFIX |
| 11 | `creditor_street_1` | CREDITOR STREET 1 |
| 12 | `creditor_street_2` | CREDITOR STREET 2 |
| 13 | `creditor_city` | CREDITOR CITY |
| 14 | `creditor_state` | CREDITOR STATE |
| 15 | `creditor_zip_code` | CREDITOR ZIP |
| 16 | `date_incurred` | DATE INCURRED |
| 17 | `amount_owed_to` | AMOUNT OWED TO |
| 18 | `amount_offered_in` | AMOUNT OFFERED IN |
| 19 | `creditor_code` | CREDITOR CODE |
| 20 | `nature_of_debt_description` | A. DESCRIPTION (Initial Terms And Nature Of Debt) |
| 21 | `efforts_made_to_pay_debt` | B. DESCRIPTION (Efforts Made By Committee To Pay Debt) |
| 22 | `steps_taken_to_collect` | C. DESCRIPTION (Steps Taken By Creditor To Collect) |
| 23 | `effort_made_by_creditor` | D. YES/NO (effort made by creditor to collect…) |
| 24 | `no_effort_description` | D. DESCRIPTION (IF NO, Explain) |
| 25 | `terms_of_settlement_comparable` | E. YES/NO (terms of debt settlement comparable…) |
| 26 | `not_comparable_description` | E. DESCRIPTION (IF NO, Explain) |
| 27 | `creditor_committee_id_number` | CREDITOR COMMITTEE ID NUMBER |
| 28 | `creditor_candidate_id_number` | CREDITOR CANDIDATE ID NUMBER |
| 29 | `creditor_candidate_last_name` | CREDITOR CANDIDATE LAST NAME |
| 30 | `creditor_candidate_first_name` | CREDITOR CANDIDATE FIRST NAME |
| 31 | `creditor_candidate_middle_name` | CREDITOR CANDIDATE MIDDLE NAME |
| 32 | `creditor_candidate_prefix` | CREDITOR CANDIDATE PREFIX |
| 33 | `creditor_candidate_suffix` | CREDITOR CANDIDATE SUFFIX |
| 34 | `creditor_candidate_office` | CREDITOR CANDIDATE OFFICE |
| 35 | `creditor_candidate_state` | CREDITOR CANDIDATE STATE |
| 36 | `creditor_candidate_district` | CREDITOR CANDIDATE DISTRICT |
| 37 | `signer_last_name` | SIGNATURE OF REPRESENTATIVE LAST NAME |
| 38 | `signer_first_name` | SIGNATURE OF REPRESENTATIVE FIRST NAME |
| 39 | `signer_middle_name` | SIGNATURE OF REPRESENTATIVE MIDDLE NAME |
| 40 | `signer_prefix` | SIGNATURE OF REPRESENTATIVE PREFIX |
| 41 | `signer_suffix` | SIGNATURE OF REPRESENTATIVE SUFFIX |
| 42 | `date_signed` | DATE SIGNED |
|  | `creditor_name` | NAME (Contributor/Lender) |
|  | `creditor_candidate_name` | CANDIDATE NAME |
|  | `signer_name` | NAME of creditor or representative |
|  | `amended_cd` | AMENDED CD |

### F83

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `entity_type` | ENTITY TYPE |
| 5 | `creditor_organization_name` | CREDITOR ORGANIZATION NAME |
| 6 | `creditor_last_name` | CREDITOR LAST NAME |
| 7 | `creditor_first_name` | CREDITOR FIRST NAME |
| 8 | `creditor_middle_name` | CREDITOR MIDDLE NAME |
| 9 | `creditor_prefix` | CREDITOR PREFIX |
| 10 | `creditor_suffix` | CREDITOR SUFFIX |
| 11 | `creditor_street_1` | CREDITOR STREET 1 |
| 12 | `creditor_street_2` | CREDITOR STREET 2 |
| 13 | `creditor_city` | CREDITOR CITY |
| 14 | `creditor_state` | CREDITOR STATE |
| 15 | `creditor_zip_code` | CREDITOR ZIP |
| 16 | `date_incurred` | DATE INCURRED |
| 17 | `amount_owed_to` | AMOUNT OWED TO CREDITOR |
| 18 | `amount_expected_to_pay` | AMOUNT EXPECTED TO PAY/OFFER |
| 19 | `creditor_code` | CREDITOR CODE |
| 20 | `disputed_debt` | YES/NO (Disputed Debt?) |
| 21 | `creditor_committee_id_number` | CREDITOR COMMITTEE ID NUMBER |
| 22 | `creditor_candidate_id_number` | CREDITOR CANDIDATE ID NUMBER |
| 23 | `creditor_candidate_last_name` | CREDITOR CANDIDATE LAST NAME |
| 24 | `creditor_candidate_first_name` | CREDITOR CANDIDATE FIRST NAME |
| 25 | `creditor_candidate_middle_name` | CREDITOR CANDIDATE MIDDLE NAME |
| 26 | `creditor_candidate_prefix` | CREDITOR CANDIDATE PREFIX |
| 27 | `creditor_candidate_suffix` | CREDITOR CANDIDATE SUFFIX |
| 28 | `creditor_candidate_office` | CREDITOR CANDIDATE OFFICE |
| 29 | `creditor_candidate_state` | CREDITOR CANDIDATE STATE |
| 30 | `creditor_candidate_district` | CREDITOR CANDIDATE DISTRICT |
|  | `creditor_name` | NAME (Contributor/Lender) |
|  | `creditor_candidate_name` | CANDIDATE NAME |
|  | `amended_cd` | AMENDED CD |

### F9

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `entity_type` | ENTITY TYPE |
| 4 | `organization_name` | ORGANIZATION NAME |
| 5 | `individual_last_name` | INDIVIDUAL LAST NAME |
| 6 | `individual_first_name` | INDIVIDUAL FIRST NAME |
| 7 | `individual_middle_name` | INDIVIDUAL MIDDLE NAME |
| 8 | `individual_prefix` | INDIVIDUAL PREFIX |
| 9 | `individual_suffix` | INDIVIDUAL SUFFIX |
| 10 | `change_of_address` | CHANGE OF ADDRESS |
| 11 | `street_1` | STREET 1 |
| 12 | `street_2` | STREET 2 |
| 13 | `city` | CITY |
| 14 | `state` | STATE |
| 15 | `zip_code` | ZIP |
| 16 | `individual_employer` | INDIVIDUAL EMPLOYER |
| 17 | `individual_occupation` | INDIVIDUAL OCCUPATION |
| 18 | `original_amendment_date` | ORIGINAL AMENDMENT DATE |
| 19 | `coverage_from_date` | COVERAGE FROM DATE |
| 20 | `coverage_through_date` | COVERAGE THROUGH DATE |
| 21 | `date_public_distribution` | DATE OF PUBLIC DISTRIBUTION |
| 22 | `communication_title` | COMMUNICATION TITLE |
| 23 | `filer_code` | FILER CODE |
| 24 | `filer_code_description` | FILER CODE DESCRIPTION |
|  | `qualified_nonprofit` | QUALIFIED NON-PROFIT |
| 25 | `segregated_bank_account` | SEGREGATED BANK ACCOUNT |
| 26 | `custodian_last_name` | CUSTODIAN LAST NAME |
| 27 | `custodian_first_name` | CUSTODIAN FIRST NAME |
| 28 | `custodian_middle_name` | CUSTODIAN MIDDLE NAME |
| 29 | `custodian_prefix` | CUSTODIAN PREFIX |
| 30 | `custodian_suffix` | CUSTODIAN SUFFIX |
| 31 | `custodian_street_1` | CUSTODIAN STREET 1 |
| 32 | `custodian_street_2` | CUSTODIAN STREET 2 |
| 33 | `custodian_city` | CUSTODIAN CITY |
| 34 | `custodian_state` | CUSTODIAN STATE |
| 35 | `custodian_zip_code` | CUSTODIAN ZIP |
| 36 | `custodian_employer` | CUSTODIAN EMPLOYER |
| 37 | `custodian_occupation` | CUSTODIAN OCCUPATION |
| 38 | `total_donations` | 9. TOTAL DONATIONS THIS STATEMENT |
| 39 | `total_disbursements` | 10. TOTAL DISB./OBLIG. THIS STATEMENT |
| 40 | `person_completing_last_name` | PERSON COMPLETING LAST NAME |
| 41 | `person_completing_first_name` | PERSON COMPLETING FIRST NAME |
| 42 | `person_completing_middle_name` | PERSON COMPLETING MIDDLE NAME |
| 43 | `person_completing_prefix` | PERSON COMPLETING PREFIX |
| 44 | `person_completing_suffix` | PERSON COMPLETING SUFFIX |
| 45 | `date_signed` | DATE SIGNED |

### F91

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `controller_last_name` | CONTROLLER LAST NAME (Share/Exer Control) |
| 5 | `controller_first_name` | CONTROLLER FIRST NAME |
| 6 | `controller_middle_name` | CONTROLLER MIDDLE NAME |
| 7 | `controller_prefix` | CONTROLLER PREFIX |
| 8 | `controller_suffix` | CONTROLLER SUFFIX |
| 9 | `controller_street_1` | CONTROLLER STREET 1 |
| 10 | `controller_street_2` | CONTROLLER STREET 2 |
| 11 | `controller_city` | CONTROLLER CITY |
| 12 | `controller_state` | CONTROLLER STATE |
| 13 | `controller_zip_code` | CONTROLLER ZIP |
| 14 | `controller_employer` | CONTROLLER EMPLOYER |
| 15 | `controller_occupation` | CONTROLLER OCCUPATION |
|  | `amended_cd` | AMENDED CD |

### F92

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `back_reference_tran_id` | BACK REFERENCE TRAN ID NUMBER |
| 5 | `back_reference_sched_name` | BACK REFERENCE SCHED NAME |
| 6 | `entity_type` | ENTITY TYPE |
| 7 | `contributor_organization_name` | DONOR ORGANIZATION NAME |
| 8 | `contributor_last_name` | DONOR LAST NAME |
| 9 | `contributor_first_name` | DONOR FIRST NAME |
| 10 | `contributor_middle_name` | DONOR MIDDLE NAME |
| 11 | `contributor_prefix` | DONOR PREFIX |
| 12 | `contributor_suffix` | DONOR SUFFIX |
| 13 | `contributor_street_1` | DONOR STREET 1 |
| 14 | `contributor_street_2` | DONOR STREET 2 |
| 15 | `contributor_city` | DONOR CITY |
| 16 | `contributor_state` | DONOR STATE |
| 17 | `contributor_zip_code` | DONOR ZIP |
| 18 | `contribution_date` | DATE RECEIVED |
| 19 | `contribution_amount` | AMOUNT RECEIVED |
|  | `contributor_employer` | INDEMP |
|  | `contributor_occupation` | INDOCC |
|  | `transaction_code` | TRANS CODE |

### F93

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `back_reference_tran_id` | BACK REFERENCE TRAN ID NUMBER |
| 5 | `back_reference_sched_name` | BACK REFERENCE SCHED NAME |
| 6 | `entity_type` | ENTITY TYPE |
| 7 | `payee_organization_name` | PAYEE ORGANIZATION NAME |
| 8 | `payee_last_name` | PAYEE LAST NAME |
| 9 | `payee_first_name` | PAYEE FIRST NAME |
| 10 | `payee_middle_name` | PAYEE MIDDLE NAME |
| 11 | `payee_prefix` | PAYEE PREFIX |
| 12 | `payee_suffix` | PAYEE SUFFIX |
| 13 | `payee_street_1` | PAYEE STREET 1 |
| 14 | `payee_street_2` | PAYEE STREET 2 |
| 15 | `payee_city` | PAYEE CITY |
| 16 | `payee_state` | PAYEE STATE |
| 17 | `payee_zip_code` | PAYEE ZIP |
| 18 | `election_code` | ELECTION CODE |
| 19 | `election_other_description` | ELECTION OTHER DESCRIPTION |
| 20 | `expenditure_date` | EXPENDITURE DATE |
| 21 | `expenditure_amount` | EXPENDITURE AMOUNT |
| 22 | `expenditure_purpose_descrip` | EXPENDITURE PURPOSE DESCRIP |
| 23 | `payee_employer` | PAYEE EMPLOYER |
| 24 | `payee_occupation` | PAYEE OCCUPATION |
| 25 | `communication_date` | COMMUNICATION DATE |
|  | `expenditure_purpose_code` | TRANS {Purpose} CODE |

### F94

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `back_reference_tran_id` | BACK REFERENCE TRAN ID |
| 5 | `back_reference_sched_name` | BACK REFERENCE SCHED NAME |
| 6 | `candidate_id_number` | CANDIDATE ID NUMBER |
|  | `candidate_name` | CANDIDATE NAME |
| 7 | `candidate_last_name` | CANDIDATE LAST NAME |
| 8 | `candidate_first_name` | CANDIDATE FIRST NAME |
| 9 | `candidate_middle_name` | CANDIDATE MIDDLE NAME |
| 10 | `candidate_prefix` | CANDIDATE PREFIX |
| 11 | `candidate_suffix` | CANDIDATE SUFFIX |
| 12 | `candidate_office` | CANDIDATE OFFICE |
| 13 | `candidate_state` | CANDIDATE STATE |
| 14 | `candidate_district` | CANDIDATE DIST |
| 15 | `election_code` | ELECTION CODE |
| 16 | `election_other_description` | ELECTION OTHER DESCRIPTION |

### F99

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `committee_name` | COMMITTEE NAME |
| 4 | `street_1` | STREET 1 |
| 5 | `street_2` | STREET 2 |
| 6 | `city` | CITY |
| 7 | `state` | STATE |
| 8 | `zip_code` | ZIP |
|  | `treasurer_name` | NAME/TREASURER (as signed) |
| 9 | `treasurer_last_name` | TREASURER LAST NAME |
| 10 | `treasurer_first_name` | TREASURER FIRST NAME |
| 11 | `treasurer_middle_name` | TREASURER MIDDLE NAME |
| 12 | `treasurer_prefix` | TREASURER PREFIX |
| 13 | `treasurer_suffix` | TREASURER SUFFIX |
| 14 | `date_signed` | DATE SIGNED |
| 15 | `text_code` | TEXT CODE |
| 16 | `filing_frequency` | FILING FREQUENCY |
| 17 | `pdf_attachment` | PDF ATTACHMENT |
| 18 | `text` | TEXT |

### H1

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `presidential_only_election_year` | State and Local Party Committee Presidential-Only Election Year (28% Federal) |
| 5 | `presidential_senate_election_year` | State and Local Party Committee Presidential and Senate Election Year (36% Federal) |
| 6 | `senate_only_election_year` | State and Local Party Committee Senate-Only Election Year (21% Federal) |
| 7 | `non_presidential_non_senate_election_year` | State and Local Party Committee Non-Presidential and Non-Senate Election Year (15% Federal) |
|  | `flat_minimum_federal_percentage` | FLAT MINIMUM FEDERAL PERCENTAGE |
| 8 | `federal_percent` | FEDERAL PERCENT |
| 9 | `nonfederal_percent` | NONFEDERAL PERCENT |
| 10 | `administrative_ratio_applies` | ADMINISTRATIVE RATIO APPLIES |
| 11 | `generic_voter_drive_ratio_applies` | GENERIC VOTER DRIVE RATIO APPLIES |
| 12 | `public_communications_referencing_party_ratio_applies` | PUBLIC COMMUNICATIONS REFERENCING PARTY ONLY RATIO APPLIES |
|  | `national_party_committee_percentage` | NAT PARTY CMTES % |
|  | `house_senate_party_committees_minimum_federal_percentage` | HSE/SEN PTY CMTES MINIMUM FED % |
|  | `house_senate_party_committees_percentage_federal_candidate_support` | HSE/SEN PTY CMTES PERCENTAGE ESTIMATED FEDERAL CAN SUPPORT |
|  | `house_senate_party_committees_percentage_nonfederal_candidate_support` | HSE/SEN PTY CMTES PERCENTAGE ESTIMATED NON FEDERAL CAN SUPPORT |
|  | `house_senate_party_committees_actual_federal_candidate_support` | HSE/SEN PTY CMTES ACTUAL FEDERAL CAN SUPPORT |
|  | `house_senate_party_committees_actual_nonfederal_candidate_support` | HSE/SEN PTY CMTES ACTUAL NON FEDERAL CAN SUPPORT |
|  | `house_senate_party_committees_percentage_actual_federal` | HSE/SEN PTY CMTES PERCENTAGE ACTUAL  FEDERAL |
|  | `actual_direct_candidate_support_federal` | Actual Direct Candidate Support - Federal Amount |
|  | `actual_direct_candidate_support_nonfederal` | Actual Direct Candidate Support - Non-Fed Amount |
|  | `actual_direct_candidate_support_federal_percent` | Actual Direct Candidate Support - Federal Percent |
|  | `ballot_presidential` | 1. BALLOT COMP PRES BLANK OR 1 |
|  | `ballot_senate` | 2. BALLOT COMP SEN  BLANK OR 1 |
|  | `ballot_house` | 3. BALLOT COMP HSE   BLANK OR 1 |
|  | `subtotal_federal` | 4. SUBTOTAL FED |
|  | `ballot_governor` | 5. BALLOT COMP GOV  BLANK OR 1 |
|  | `ballot_other_statewide` | 6. OTHER STATEWIDE |
|  | `ballot_state_senate` | 7. STATE SENATE |
|  | `ballot_state_representative` | 8. STATE REP. |
|  | `ballot_local_candidates` | 9. LOCAL CANDIDATES  BLANK, 1 OR 2 |
|  | `extra_nonfederal_point` | 10. EXTRA NON-FED POINT  BLANK OR 1 |
|  | `subtotal` | 11. SUBTOTAL |
|  | `total_points` | 12. TOTAL POINTS |

### H2

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `activity_event_name` | ACTIVITY/EVENT NAME |
| 5 | `direct_fundraising` | YES/NO (Direct Fundraising?) |
| 6 | `direct_candidate_support` | YES/NO (Direct Candidate Support?) |
| 7 | `ratio_code` | RATIO CODE |
| 8 | `federal_percentage` | FEDERAL PERCENTAGE |
| 9 | `nonfederal_percentage` | NON-FEDERAL PERCENTAGE |
|  | `exempt_activity` | YESNO  (Activity Is Exempt) |

### H3

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `back_reference_tran_id` | BACK REFERENCE TRAN ID |
| 5 | `account_name` | ACCOUNT NAME |
| 6 | `event_type` | EVENT TYPE |
| 7 | `activity_event_name` | EVENT/ACTIVITY ID/NAME |
| 8 | `receipt_date` | RECEIPT DATE |
| 9 | `total_amount_transferred` | TOTAL AMOUNT TRANSFERRED |
| 10 | `transferred_amount` | TRANSFERRED AMOUNT |

### H4

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `back_reference_tran_id` | BACK REFERENCE TRAN ID NUMBER |
| 5 | `back_reference_sched_name` | BACK REFERENCE SCHED NAME |
| 6 | `entity_type` | ENTITY TYPE |
|  | `payee_name` | NAME  (Payee) |
| 7 | `payee_organization_name` | PAYEE ORGANIZATION NAME |
| 8 | `payee_last_name` | PAYEE LAST NAME |
| 9 | `payee_first_name` | PAYEE FIRST NAME |
| 10 | `payee_middle_name` | PAYEE MIDDLE NAME |
| 11 | `payee_prefix` | PAYEE PREFIX |
| 12 | `payee_suffix` | PAYEE SUFFIX |
| 13 | `payee_street_1` | PAYEE STREET 1 |
| 14 | `payee_street_2` | PAYEE STREET 2 |
| 15 | `payee_city` | PAYEE CITY |
| 16 | `payee_state` | PAYEE STATE |
| 17 | `payee_zip_code` | PAYEE ZIP |
| 18 | `account_identifier` | ACCOUNT/EVENT IDENTIFIER |
| 19 | `expenditure_date` | EXPENDITURE DATE |
| 20 | `total_amount` | TOTAL FED-NONFED AMOUNT |
| 21 | `federal_share` | FEDERAL SHARE |
| 22 | `nonfederal_share` | NONFEDERAL SHARE |
| 23 | `event_year_to_date` | ACTIVITY/EVENT TOTAL YTD |
|  | `expenditure_purpose_code` | EXPENDITURE PURPOSE CODE |
| 24 | `expenditure_purpose_descrip` | EXPENDITURE PURPOSE DESCRIP |
| 25 | `category_code` | CATEGORY CODE |
| 26 | `administrative_voter_drive_activity` | YES/NO (Activity is Administrative - Only) |
| 27 | `fundraising_activity` | YES/NO (Activity is Direct Fundraising) |
| 28 | `exempt_activity` | YES/NO (Activity is an Exempt Activity) |
| 29 | `generic_voter_drive_activity` | YES/NO (Activity is Generic Voter Drive - Only) |
| 30 | `direct_candidate_support_activity` | YES/NO (Activity is Direct Candidate Support) |
| 31 | `public_communications_party_activity` | YES/NO (Activity is Public Communications {Referring Only to Party} Made by PAC |
| 32 | `memo_code` | MEMO CODE |
| 33 | `memo_text` | MEMO TEXT/DESCRIPTION |
|  | `fec_committee_id_number` | FEC COMMITTEE ID NUMBER |
|  | `candidate_id_number` | FEC CANDIDATE ID NUMBER |
|  | `candidate_name` | CANDIDATE NAME |
|  | `candidate_office` | CAN/OFFICE |
|  | `candidate_state` | CAN/STATE |
|  | `candidate_district` | CAN/DIST |
|  | `conduit_name` | CONDUIT NAME |
|  | `conduit_street_1` | CONDUIT STREET 1 |
|  | `conduit_street_2` | CONDUIT STREET 2 |
|  | `conduit_city` | CONDUIT CITY |
|  | `conduit_state` | CONDUIT STATE |
|  | `conduit_zip_code` | CONDUIT ZIP |
|  | `amended_cd` | AMENDED CD |

### H5

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `account_name` | ACCOUNT NAME |
| 5 | `receipt_date` | RECEIPT DATE |
| 6 | `total_amount_transferred` | TOTAL AMOUNT TRANSFERRED |
| 7 | `voter_registration_amount` | VOTER REGISTRATION AMOUNT |
| 8 | `voter_id_amount` | VOTER ID AMOUNT |
| 9 | `gotv_amount` | GOTV AMOUNT |
| 10 | `generic_campaign_amount` | GENERIC CAMPAIGN AMOUNT |

### H6

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `back_reference_tran_id` | BACK REFERENCE TRAN ID NUMBER |
| 5 | `back_reference_sched_name` | BACK REFERENCE SCHED NAME |
| 6 | `entity_type` | ENTITY TYPE |
|  | `payee_name` | NAME  (Payee) |
| 7 | `payee_organization_name` | PAYEE ORGANIZATION NAME |
| 8 | `payee_last_name` | PAYEE LAST NAME |
| 9 | `payee_first_name` | PAYEE FIRST NAME |
| 10 | `payee_middle_name` | PAYEE MIDDLE NAME |
| 11 | `payee_prefix` | PAYEE PREFIX |
| 12 | `payee_suffix` | PAYEE SUFFIX |
| 13 | `payee_street_1` | PAYEE STREET 1 |
| 14 | `payee_street_2` | PAYEE STREET 2 |
| 15 | `payee_city` | PAYEE CITY |
| 16 | `payee_state` | PAYEE STATE |
| 17 | `payee_zip_code` | PAYEE ZIP |
| 18 | `account_identifier` | ACCOUNT/EVENT IDENTIFIER |
| 19 | `expenditure_date` | EXPENDITURE DATE |
| 20 | `total_amount` | TOTAL FED-LEVIN AMOUNT |
| 21 | `federal_share` | FEDERAL SHARE |
| 22 | `levin_share` | LEVIN SHARE |
| 23 | `event_year_to_date` | ACTIVITY/EVENT TOTAL YTD |
|  | `expenditure_purpose_code` | EXPENDITURE PURPOSE CODE |
| 24 | `expenditure_purpose_descrip` | EXPENDITURE PURPOSE DESCRIP |
| 25 | `category_code` | CATEGORY CODE |
| 26 | `voter_registration_activity` | YES/NO (Activity Is Voter Registration) |
| 27 | `gotv_activity` | YES/NO (Activity GOTV) |
| 28 | `voter_id_activity` | YES/NO (Activity Is Voter ID) |
| 29 | `generic_campaign_activity` | YES/NO (Activity is Generic Campaign) |
| 30 | `memo_code` | MEMO CODE |
| 31 | `memo_text` | MEMO TEXT/DESCRIPTION |
|  | `fec_committee_id_number` | FEC COMMITTEE ID NUMBER |
|  | `candidate_id_number` | FEC CANDIDATE ID NUMBER |
|  | `candidate_name` | CANDIDATE NAME |
|  | `candidate_office` | CAN/OFFICE |
|  | `candidate_state` | CAN/STATE |
|  | `candidate_district` | CAN/DIST |
|  | `conduit_committee_id` | CONDUIT COMMITTEE ID |
|  | `conduit_name` | CONDUIT NAME |
|  | `conduit_street_1` | CONDUIT STREET 1 |
|  | `conduit_street_2` | CONDUIT STREET 2 |
|  | `conduit_city` | CONDUIT CITY |
|  | `conduit_state` | CONDUIT STATE |
|  | `conduit_zip_code` | CONDUIT ZIP |

### HDR

| Column | Field | FEC label |
|---:|---|---|
| 1 | `record_type` | Record Type |
| 2 | `ef_type` | EF Type |
| 3 | `fec_version` | FEC Version # |
| 4 | `soft_name` | Soft Name |
| 5 | `soft_ver` | Soft Ver |
|  | `name_delim` | Name Delim |
| 6 | `report_id` | Rpt ID |
| 7 | `report_number` | Rpt Number |
| 8 | `comment` | HDRcomment |

### SchA

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID |
| 4 | `back_reference_tran_id` | BACK REFERENCE TRAN ID NUMBER |
| 5 | `back_reference_sched_name` | BACK REFERENCE SCHED NAME |
| 6 | `entity_type` | ENTITY TYPE |
|  | `contributor_name` | CONTRIBUTOR NAME |
| 7 | `contributor_organization_name` | CONTRIBUTOR ORGANIZATION NAME |
| 8 | `contributor_last_name` | CONTRIBUTOR LAST NAME |
| 9 | `contributor_first_name` | CONTRIBUTOR FIRST NAME |
| 10 | `contributor_middle_name` | CONTRIBUTOR MIDDLE NAME |
| 11 | `contributor_prefix` | CONTRIBUTOR PREFIX |
| 12 | `contributor_suffix` | CONTRIBUTOR SUFFIX |
| 13 | `contributor_street_1` | CONTRIBUTOR STREET 1 |
| 14 | `contributor_street_2` | CONTRIBUTOR STREET 2 |
| 15 | `contributor_city` | CONTRIBUTOR CITY |
| 16 | `contributor_state` | CONTRIBUTOR STATE |
| 17 | `contributor_zip_code` | CONTRIBUTOR ZIP |
| 18 | `election_code` | ELECTION CODE |
| 19 | `election_other_description` | ELECTION OTHER DESCRIPTION |
| 20 | `contribution_date` | CONTRIBUTION DATE |
| 21 | `contribution_amount` | CONTRIBUTION AMOUNT {F3L Bundled} |
| 22 | `contribution_aggregate` | CONTRIBUTION AGGREGATE {F3L Semi-annual Bundled} |
|  | `contribution_purpose_code` | CONTRIBUTION PURPOSE CODE |
| 23 | `contribution_purpose_descrip` | CONTRIBUTION PURPOSE DESCRIP |
|  | `increased_limit_code` | INCREASED LIMIT CODE |
| 24 | `contributor_employer` | CONTRIBUTOR EMPLOYER |
| 25 | `contributor_occupation` | CONTRIBUTOR OCCUPATION |
| 26 | `donor_committee_fec_id` | DONOR COMMITTEE FEC ID |
| 27 | `donor_committee_name` | DONOR COMMITTEE NAME |
| 28 | `donor_candidate_fec_id` | DONOR CANDIDATE FEC ID |
|  | `donor_candidate_name` | CANDIDATE NAME |
| 29 | `donor_candidate_last_name` | DONOR CANDIDATE LAST NAME |
| 30 | `donor_candidate_first_name` | DONOR CANDIDATE FIRST NAME |
| 31 | `donor_candidate_middle_name` | DONOR CANDIDATE MIDDLE NAME |
| 32 | `donor_candidate_prefix` | DONOR CANDIDATE PREFIX |
| 33 | `donor_candidate_suffix` | DONOR CANDIDATE SUFFIX |
| 34 | `donor_candidate_office` | DONOR CANDIDATE OFFICE |
| 35 | `donor_candidate_state` | DONOR CANDIDATE STATE |
| 36 | `donor_candidate_district` | DONOR CANDIDATE DISTRICT |
| 37 | `conduit_name` | CONDUIT NAME |
| 38 | `conduit_street_1` | CONDUIT STREET1 |
| 39 | `conduit_street_2` | CONDUIT STREET2 |
| 40 | `conduit_city` | CONDUIT CITY |
| 41 | `conduit_state` | CONDUIT STATE |
| 42 | `conduit_zip_code` | CONDUIT ZIP |
| 43 | `memo_code` | MEMO CODE |
| 44 | `memo_text` | MEMO TEXT/DESCRIPTION |
| 45 | `reference_code` | Reference to SI or SL system code that identifies the Account |
|  | `amended_cd` |  |
|  | `image_number` |  |

### SchA3L

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID |
| 4 | `back_reference_tran_id` | BACK REFERENCE TRAN ID NUMBER |
| 5 | `back_reference_sched_name` | BACK REFERENCE SCHED NAME |
| 6 | `entity_type` | ENTITY TYPE |
| 7 | `lobbyist_registrant_organization_name` | LOBBYIST/REGISTRANT ORGANIZATION NAME |
| 8 | `lobbyist_registrant_last_name` | LOBBYIST/REGISTRANT LAST NAME |
| 9 | `lobbyist_registrant_first_name` | LOBBYIST/REGISTRANT FIRST NAME |
| 10 | `lobbyist_registrant_middle_name` | LOBBYIST/REGISTRANT MIDDLE NAME |
| 11 | `lobbyist_registrant_prefix` | LOBBYIST/REGISTRANT PREFIX |
| 12 | `lobbyist_registrant_suffix` | LOBBYIST/REGISTRANT SUFFIX |
| 13 | `lobbyist_registrant_street_1` | LOBBYIST/REGISTRANT STREET  1 |
| 14 | `lobbyist_registrant_street_2` | LOBBYIST/REGISTRANT STREET  2 |
| 15 | `lobbyist_registrant_city` | LOBBYIST/REGISTRANT CITY |
| 16 | `lobbyist_registrant_state` | LOBBYIST/REGISTRANT STATE |
| 17 | `lobbyist_registrant_zip_code` | LOBBYIST/REGISTRANT ZIP |
| 18 | `election_code` | ELECTION CODE |
| 19 | `election_other_description` | ELECTION OTHER DESCRIPTION |
| 20 | `contribution_date` | CONTRIBUTION DATE |
| 21 | `bundled_amount_period` | BUNDLED AMOUNT PERIOD |
| 22 | `bundled_amount_semi_annual` | BUNDLED AMOUNT SEMI-ANNUAL |
|  | `contribution_purpose_code` | CONTRIBUTION PURPOSE CODE |
| 23 | `contribution_purpose_descrip` | CONTRIBUTION PURPOSE DESCRIP |
| 24 | `lobbyist_registrant_employer` | LOBBYIST/REGISTRANT EMPLOYER |
| 25 | `lobbyist_registrant_occupation` | LOBBYIST/REGISTRANT OCCUPATION |
| 26 | `donor_committee_fec_id` | DONOR COMMITTEE FEC ID |
| 27 | `donor_committee_name` | DONOR COMMITTEE NAME |
| 28 | `donor_candidate_fec_id` | DONOR CANDIDATE FEC ID |
| 29 | `donor_candidate_last_name` | DONOR CANDIDATE LAST NAME |
| 30 | `donor_candidate_first_name` | DONOR CANDIDATE FIRST NAME |
| 31 | `donor_candidate_middle_name` | DONOR CANDIDATE MIDDLE NAME |
| 32 | `donor_candidate_prefix` | DONOR CANDIDATE PREFIX |
| 33 | `donor_candidate_suffix` | DONOR CANDIDATE SUFFIX |
| 34 | `donor_candidate_office` | DONOR CANDIDATE OFFICE |
| 35 | `donor_candidate_state` | DONOR CANDIDATE STATE |
| 36 | `donor_candidate_district` | DONOR CANDIDATE DISTRICT |
| 37 | `conduit_name` | CONDUIT NAME |
| 38 | `conduit_street_1` | CONDUIT STREET1 |
| 39 | `conduit_street_2` | CONDUIT STREET2 |
| 40 | `conduit_city` | CONDUIT CITY |
| 41 | `conduit_state` | CONDUIT STATE |
| 42 | `conduit_zip_code` | CONDUIT ZIP |
| 43 | `associated_text_record` | ASSOCIATED TEXT RECORD |
| 44 | `memo_text` | MEMO TEXT |
| 45 | `reference_code` | Reference to SI or SL system code that identifies the Account |

### SchB

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `back_reference_tran_id` | BACK REFERENCE TRAN ID NUMBER |
| 5 | `back_reference_sched_name` | BACK REFERENCE SCHED NAME |
| 6 | `entity_type` | ENTITY TYPE |
|  | `payee_name` | RECIPIENT NAME |
| 7 | `payee_organization_name` | PAYEE ORGANIZATION NAME |
| 8 | `payee_last_name` | PAYEE LAST NAME |
| 9 | `payee_first_name` | PAYEE FIRST NAME |
| 10 | `payee_middle_name` | PAYEE MIDDLE NAME |
| 11 | `payee_prefix` | PAYEE PREFIX |
| 12 | `payee_suffix` | PAYEE SUFFIX |
| 13 | `payee_street_1` | PAYEE STREET 1 |
| 14 | `payee_street_2` | PAYEE STREET 2 |
| 15 | `payee_city` | PAYEE CITY |
| 16 | `payee_state` | PAYEE STATE |
| 17 | `payee_zip_code` | PAYEE ZIP |
| 18 | `election_code` | ELECTION CODE |
| 19 | `election_other_description` | ELECTION OTHER DESCRIPTION |
| 20 | `expenditure_date` | EXPENDITURE DATE |
| 21 | `expenditure_amount` | EXPENDITURE AMOUNT {F3L Bundled} |
| 22 | `semi_annual_refunded_bundled_amt` | SEMI-ANNUAL REFUNDED BUNDLED AMT |
|  | `expenditure_purpose_code` | EXPENDITURE PURPOSE CODE |
| 23 | `expenditure_purpose_descrip` | EXPENDITURE PURPOSE DESCRIP |
| 24 | `category_code` | CATEGORY CODE |
| 25 | `beneficiary_committee_fec_id` | BENEFICIARY COMMITTEE FEC ID |
| 26 | `beneficiary_committee_name` | BENEFICIARY COMMITTEE NAME |
| 27 | `beneficiary_candidate_fec_id` | BENEFICIARY CANDIDATE FEC ID |
|  | `beneficiary_candidate_name` | CANDIDATE NAME |
| 28 | `beneficiary_candidate_last_name` | BENEFICIARY CANDIDATE LAST NAME |
| 29 | `beneficiary_candidate_first_name` | BENEFICIARY CANDIDATE FIRST NAME |
| 30 | `beneficiary_candidate_middle_name` | BENEFICIARY CANDIDATE MIDDLE NAME |
| 31 | `beneficiary_candidate_prefix` | BENEFICIARY CANDIDATE PREFIX |
| 32 | `beneficiary_candidate_suffix` | BENEFICIARY CANDIDATE SUFFIX |
| 33 | `beneficiary_candidate_office` | BENEFICIARY CANDIDATE OFFICE |
| 34 | `beneficiary_candidate_state` | BENEFICIARY CANDIDATE STATE |
| 35 | `beneficiary_candidate_district` | BENEFICIARY CANDIDATE DISTRICT |
| 36 | `conduit_name` | CONDUIT NAME |
| 37 | `conduit_street_1` | CONDUIT STREET 1 |
| 38 | `conduit_street_2` | CONDUIT STREET 2 |
| 39 | `conduit_city` | CONDUIT CITY |
| 40 | `conduit_state` | CONDUIT STATE |
| 41 | `conduit_zip_code` | CONDUIT ZIP |
| 42 | `memo_code` | MEMO CODE |
| 43 | `memo_text` | MEMO TEXT/DESCRIPTION |
| 44 | `reference_code` | Reference to SI or SL system code that identifies the Account |
|  | `refund_or_disposal_of_excess` | REFUND OR DISPOSAL OF EXCESS |
|  | `communication_date` | COMMUNICATION DATE |
|  | `amended_cd` |  |
|  | `image_number` |  |

### SchC

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `receipt_line_number` | RECEIPT LINE NUMBER |
| 5 | `entity_type` | ENTITY TYPE |
|  | `lender_name` | NAME (Loan Source) |
| 6 | `lender_organization_name` | LENDER ORGANIZATION NAME |
| 7 | `lender_last_name` | LENDER LAST NAME |
| 8 | `lender_first_name` | LENDER FIRST NAME |
| 9 | `lender_middle_name` | LENDER MIDDLE NAME |
| 10 | `lender_prefix` | LENDER PREFIX |
| 11 | `lender_suffix` | LENDER SUFFIX |
| 12 | `lender_street_1` | LENDER STREET 1 |
| 13 | `lender_street_2` | LENDER STREET 2 |
| 14 | `lender_city` | LENDER CITY |
| 15 | `lender_state` | LENDER STATE |
| 16 | `lender_zip_code` | LENDER ZIP |
| 17 | `election_code` | ELECTION CODE |
| 18 | `election_other_description` | ELECTION OTHER DESCRIPTION |
| 19 | `loan_amount_original` | LOAN AMOUNT (Original) |
| 20 | `loan_payment_to_date` | LOAN PAYMENT TO DATE |
| 21 | `loan_balance` | LOAN BALANCE |
| 22 | `loan_incurred_date_terms` | LOAN INCURRED DATE (Terms) |
| 23 | `loan_due_date_terms` | LOAN DUE DATE (Terms) |
| 24 | `loan_interest_rate_terms` | LOAN INTEREST RATE % (Terms) |
| 25 | `secured` | YES/NO (Secured?) |
| 26 | `personal_funds` | YES/NO (Personal Funds) |
| 27 | `lender_committee_id_number` | LENDER COMMITTEE ID NUMBER |
| 28 | `lender_candidate_id_number` | LENDER CANDIDATE ID NUMBER |
|  | `lender_candidate_name` | CANDIDATE NAME |
| 29 | `lender_candidate_last_name` | LENDER CANDIDATE LAST NAME |
| 30 | `lender_candidate_first_name` | LENDER CANDIDATE FIRST NAME |
| 31 | `lender_candidate_middle_name` | LENDER CANDIDATE MIDDLE NM |
| 32 | `lender_candidate_prefix` | LENDER CANDIDATE PREFIX |
| 33 | `lender_candidate_suffix` | LENDER CANDIDATE SUFFIX |
| 34 | `lender_candidate_office` | LENDER CANDIDATE OFFICE |
| 35 | `lender_candidate_state` | LENDER CANDIDATE STATE |
| 36 | `lender_candidate_district` | LENDER CANDIDATE DISTRICT |
| 37 | `memo_code` | MEMO CODE |
| 38 | `memo_text` | MEMO TEXT/DESCRIPTION |

### SchC1

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `back_reference_tran_id` | BACK REFERENCE TRAN ID NUMBER |
| 5 | `lender_organization_name` | LENDER ORGANIZATION NAME |
| 6 | `lender_street_1` | LENDER STREET 1 |
| 7 | `lender_street_2` | LENDER STREET 2 |
| 8 | `lender_city` | LENDER CITY |
| 9 | `lender_state` | LENDER STATE |
| 10 | `lender_zip_code` | LENDER ZIP |
| 11 | `loan_amount` | LOAN AMOUNT |
| 12 | `loan_interest_rate` | LOAN INTEREST RATE % |
| 13 | `loan_incurred_date` | LOAN INCURRED DATE |
| 14 | `loan_due_date` | LOAN DUE DATE |
| 15 | `loan_restructured` | A1.YES/NO (Loan Restructured) |
| 16 | `loan_inccured_date_original` | A2. DATE (Of Original Loan) |
| 17 | `credit_amount_this_draw` | B.1. CREDIT AMOUNT THIS DRAW |
| 18 | `total_balance` | B.2. TOTAL BALANCE |
| 19 | `others_liable` | C. YES/NO (Others liable?) |
| 20 | `collateral` | D. YES/NO (Collateral?) |
| 21 | `collateral_description` | D.1 DESC (Collateral) |
| 22 | `collateral_value_amount` | D.2 COLLATERAL VALUE/AMOUNT |
| 23 | `perfected_interest` | D.3 YES/NO (Perfected Interest?)) |
| 24 | `future_income` | E.1 YES/NO (Future Income) |
| 25 | `future_income_description` | E.2 DESC (Specification of the above) |
| 26 | `estimated_value` | E.3 ESTIMATED VALUE |
| 27 | `established_date` | E.4 DATE (Depository account established) |
| 28 | `account_location_name` | E.5 IND/NAME (Account Location) |
| 29 | `street_1` | E.6 STREET 1 |
| 30 | `street_2` | E.7 STREET 2 |
| 31 | `city` | E.8 CITY |
| 32 | `state` | E.9 STATE |
| 33 | `zip_code` | E.10 ZIP |
| 34 | `deposit_acct_auth_date_presidential` | E.11 DEP ACCT AUTH DATE (Presidential) |
| 35 | `f_basis_of_loan_description` | F. BASIS OF LOAN DESCRIPTION |
|  | `treasurer_name` | G. NAME/TREASURER (as signed) |
| 36 | `treasurer_last_name` | G. TREASURER LAST NAME |
| 37 | `treasurer_first_name` | G. TREASURER FIRST NAME |
| 38 | `treasurer_middle_name` | G. TREASURER MIDDLE NAME |
| 39 | `treasurer_prefix` | G. TREASURER PREFIX |
| 40 | `treasurer_suffix` | G. TREASURER SUFFIX |
| 41 | `treasurer_date_signed` | G. DATE SIGNED |
|  | `authorized_name` | H. NAME/AUTH (as signed) |
| 42 | `authorized_last_name` | H. AUTHORIZED LAST NAME |
| 43 | `authorized_first_name` | H. AUTHORIZED FIRST NAME |
| 44 | `authorized_middle_name` | H. AUTHORIZED MIDDLE NAME |
| 45 | `authorized_prefix` | H. AUTHORIZED PREFIX |
| 46 | `authorized_suffix` | H. AUTHORIZED SUFFIX |
| 47 | `authorized_title` | H. AUTHORIZED TITLE |
| 48 | `authorized_date_signed` | H. DATE SIGNED |
|  | `entity_type` | ENTITY TYPE |

### SchC2

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `back_reference_tran_id` | BACK REFERENCE TRAN ID NUMBER |
| 5 | `entity_type` | GUARANTOR ENTITY |
| 6 | `guarantor_employer_code` | GUARANTOR ORGANIZATION NAME |
| 7 | `guarantor_occupation_code` | GUARANTOR COMMITTEE FEC ID |
|  | `guarantor_name` | IND/NAME (Endorser/Guarantor) |
| 8 | `guarantor_last_name` | GUARANTOR LAST NAME |
| 9 | `guarantor_first_name` | GUARANTOR FIRST NAME |
| 10 | `guarantor_middle_name` | GUARANTOR MIDDLE NAME |
| 11 | `guarantor_prefix` | GUARANTOR PREFIX |
| 12 | `guarantor_suffix` | GUARANTOR SUFFIX |
| 13 | `guarantor_street_1` | GUARANTOR STREET 1 |
| 14 | `guarantor_street_2` | GUARANTOR STREET 2 |
| 15 | `guarantor_city` | GUARANTOR CITY |
| 16 | `guarantor_state` | GUARANTOR STATE |
| 17 | `guarantor_zip_code` | GUARANTOR ZIP |
| 18 | `guarantor_employer` | GUARANTOR EMPLOYER |
| 19 | `guarantor_occupation` | GUARANTOR OCCUPATION |
| 20 | `guaranteed_amount` | GUARANTEED AMOUNT |

### SchD

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `entity_type` | ENTITY TYPE |
|  | `creditor_name` | NAME (Debtor/Creditor) |
| 5 | `creditor_organization_name` | CREDITOR ORGANIZATION NAME |
| 6 | `creditor_last_name` | CREDITOR LAST NAME |
| 7 | `creditor_first_name` | CREDITOR FIRST NAME |
| 8 | `creditor_middle_name` | CREDITOR MIDDLE NAME |
| 9 | `creditor_prefix` | CREDITOR PREFIX |
| 10 | `creditor_suffix` | CREDITOR SUFFIX |
| 11 | `creditor_street_1` | CREDITOR STREET 1 |
| 12 | `creditor_street_2` | CREDITOR STREET 2 |
| 13 | `creditor_city` | CREDITOR CITY |
| 14 | `creditor_state` | CREDITOR STATE |
| 15 | `creditor_zip_code` | CREDITOR ZIP |
| 16 | `purpose_of_debt_or_obligation` | PURPOSE OF DEBT OR OBLIGATION |
| 17 | `beginning_balance_this_period` | BEGINNING BALANCE (This Period) |
| 18 | `incurred_amount_this_period` | INCURRED AMOUNT (This Period) |
| 19 | `payment_amount_this_period` | PAYMENT AMOUNT (This Period) |
| 20 | `balance_at_close_this_period` | BALANCE AT CLOSE (This Period) |
|  | `fec_committee_id_number` | FEC COMMITTEE ID NUMBER |
|  | `candidate_id_number` | FEC CANDIDATE ID NUMBER |
|  | `candidate_name` | CANDIDATE NAME |
|  | `candidate_office` | CAN/OFFICE |
|  | `candidate_state` | CAN/STATE |
|  | `candidate_district` | CAN/DIST |
|  | `conduit_name` | CONDUIT NAME |
|  | `conduit_street_1` | CONDUIT STREET 1 |
|  | `conduit_street_2` | CONDUIT STREET 2 |
|  | `conduit_city` | CONDUIT CITY |
|  | `conduit_state` | CONDUIT STATE |
|  | `conduit_zip_code` | CONDUIT ZIP |

### SchE

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `back_reference_tran_id` | BACK REFERENCE TRAN ID NUMBER |
| 5 | `back_reference_sched_name` | BACK REFERENCE SCHED NAME |
| 6 | `entity_type` | ENTITY TYPE |
|  | `payee_name` | NAME (Payee) |
| 7 | `payee_organization_name` | PAYEE ORGANIZATION NAME |
| 8 | `payee_last_name` | PAYEE LAST NAME |
| 9 | `payee_first_name` | PAYEE FIRST NAME |
| 10 | `payee_middle_name` | PAYEE MIDDLE NAME |
| 11 | `payee_prefix` | PAYEE PREFIX |
| 12 | `payee_suffix` | PAYEE SUFFIX |
| 13 | `payee_street_1` | PAYEE STREET 1 |
| 14 | `payee_street_2` | PAYEE STREET 2 |
| 15 | `payee_city` | PAYEE CITY |
| 16 | `payee_state` | PAYEE STATE |
| 17 | `payee_zip_code` | PAYEE ZIP |
| 18 | `election_code` | ELECTION CODE |
| 19 | `election_other_description` | ELECTION OTHER DESCRIPTION |
| 20 | `dissemination_date` | DISSEMINATION DATE |
| 21 | `expenditure_amount` | EXPENDITURE AMOUNT |
| 22 | `disbursement_date` | DISBURSEMENT DATE |
| 23 | `calendar_y_t_d_per_election_office` | CALENDAR Y-T-D (per election/office) |
|  | `expenditure_purpose_code` | EXPENDITURE PURPOSE CODE |
| 24 | `expenditure_purpose_descrip` | EXPENDITURE PURPOSE DESCRIP |
| 25 | `category_code` | CATEGORY CODE |
| 26 | `payee_committee_id_number` | PAYEE CMTTE FEC ID NUMBER |
| 27 | `support_oppose_code` | SUPPORT/OPPOSE CODE |
| 28 | `candidate_id_number` | S/O CANDIDATE ID NUMBER |
|  | `candidate_name` | S/O CAN/NAME |
| 29 | `candidate_last_name` | S/O CANDIDATE LAST NAME |
| 30 | `candidate_first_name` | S/O CANDIDATE FIRST NAME |
| 31 | `candidate_middle_name` | S/O CANDINATE MIDDLE NAME |
| 32 | `candidate_prefix` | S/O CANDIDATE PREFIX |
| 33 | `candidate_suffix` | S/O CANDIDATE SUFFIX |
| 34 | `candidate_office` | S/O CANDIDATE OFFICE |
| 36 | `candidate_state` | S/O CANDIDATE STATE |
| 35 | `candidate_district` | S/O CANDIDATE DISTRICT |
| 37 | `completing_last_name` | COMPLETING LAST NAME |
| 38 | `completing_first_name` | COMPLETING FIRST NAME |
| 39 | `completing_middle_name` | COMPLETING MIDDLE NAME |
| 40 | `completing_prefix` | COMPLETING PREFIX |
| 41 | `completing_suffix` | COMPLETING SUFFIX |
| 42 | `date_signed` | DATE SIGNED |
| 43 | `memo_code` | MEMO CODE |
| 44 | `memo_text` | MEMO TEXT/DESCRIPTION |
|  | `date_notarized` | DATE (Notarized) |
|  | `date_notary_commission_expires` | DATE (Notary Commission Expires) |
|  | `conduit_name` | CONDUIT NAME |
|  | `conduit_street_1` | CONDUIT STREET 1 |
|  | `conduit_street_2` | CONDUIT STREET 2 |
|  | `conduit_city` | CONDUIT CITY |
|  | `conduit_state` | CONDUIT STATE |
|  | `conduit_zip_code` | CONDUIT ZIP |
|  | `completing_name` | IND/NAME (as signed) |
|  | `notary_name` | IND/NAME (Notary) |

### SchF

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `back_reference_tran_id` | BACK REFERENCE TRAN ID NUMBER |
| 5 | `back_reference_sched_name` | BACK REFERENCE SCHED NAME |
| 6 | `coordinated_expenditures` | YES/NO (Has filer been designated to make Coordinated Expenditures?) |
| 7 | `designating_committee_id_number` | DESIGNATING COMMITTEE ID NUMBER |
| 8 | `designating_committee_name` | DESIGNATING COMMITTEE NAME |
| 9 | `subordinate_committee_id_number` | SUBORDINATE COMMITTEE ID NUMBER |
| 10 | `subordinate_committee_name` | SUBORDINATE COMMITTEE NAME |
| 11 | `subordinate_street_1` | SUBORDINATE STREET 1 |
| 12 | `subordinate_street_2` | SUBORDINATE STREET 2 |
| 13 | `subordinate_city` | SUBORDINATE CITY |
| 14 | `subordinate_state` | SUBORDINATE STATE |
| 15 | `subordinate_zip_code` | SUBORDINATE ZIP |
| 16 | `entity_type` | ENTITY TYPE |
|  | `payee_name` | NAME (Payee) |
| 17 | `payee_organization_name` | PAYEE ORGANIZATION NAME |
| 18 | `payee_last_name` | PAYEE LAST NAME |
| 19 | `payee_first_name` | PAYEE FIRST NAME |
| 20 | `payee_middle_name` | PAYEE MIDDLE NAME |
| 21 | `payee_prefix` | PAYEE PREFIX |
| 22 | `payee_suffix` | PAYEE SUFFIX |
| 23 | `payee_street_1` | PAYEE STREET 1 |
| 24 | `payee_street_2` | PAYEE STREET 2 |
| 25 | `payee_city` | PAYEE CITY |
| 26 | `payee_state` | PAYEE STATE |
| 27 | `payee_zip_code` | PAYEE ZIP |
| 28 | `expenditure_date` | EXPENDITURE DATE |
| 29 | `expenditure_amount` | EXPENDITURE AMOUNT |
| 30 | `aggregate_general_elec_expended` | AGGREGATE GENERAL ELEC EXPENDED |
|  | `expenditure_purpose_code` | EXPENDITURE PURPOSE CODE |
| 31 | `expenditure_purpose_descrip` | EXPENDITURE PURPOSE DESCRIP |
| 32 | `category_code` | CATEGORY CODE |
|  | `increased_limit` | INCREASED LIMIT |
| 33 | `payee_committee_id_number` | PAYEE COMMITTEE ID NUMBER |
| 34 | `payee_candidate_id_number` | PAYEE CANDIDATE ID NUMBER |
|  | `payee_candidate_name` | CANDIDATE NAME |
| 35 | `payee_candidate_last_name` | PAYEE CANDIDATE LAST NAME |
| 36 | `payee_candidate_first_name` | PAYEE CANDIDATE FIRST NAME |
| 37 | `payee_candidate_middle_name` | PAYEE CANDIDATE MIDDLE NAME |
| 38 | `payee_candidate_prefix` | PAYEE CANDIDATE PREFIX |
| 39 | `payee_candidate_suffix` | PAYEE CANDIDATE SUFFIX |
| 40 | `payee_candidate_office` | PAYEE CANDIDATE OFFICE |
| 41 | `payee_candidate_state` | PAYEE CANDIDATE STATE |
| 42 | `payee_candidate_district` | PAYEE CANDIDATE DISTRICT |
| 43 | `memo_code` | MEMO CODE |
| 44 | `memo_text` | MEMO TEXT/DESCRIPTION |
|  | `conduit_name` | CONDUIT NAME |
|  | `conduit_street_1` | CONDUIT STREET 1 |
|  | `conduit_street_2` | CONDUIT STREET 2 |
|  | `conduit_city` | CONDUIT CITY |
|  | `conduit_state` | CONDUIT STATE |
|  | `conduit_zip_code` | CONDUIT ZIP |

### SchI

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `record_id_number` | RECORD ID NUMBER (for account name) |
| 5 | `account_name` | ACCOUNT NAME |
| 6 | `bank_account_id` | Bank Account ID Number |
| 7 | `coverage_from_date` | COVERAGE FROM DATE |
| 8 | `coverage_through_date` | COVERAGE THROUGH DATE |
| 9 | `col_a_total_receipts` | 1. Total Receipts |
| 10 | `col_a_transfers_to_fed` | 2. Transfers to FED or allocation |
| 11 | `col_a_transfers_to_state_local` | 3. Transfers to state/local Party organizations |
| 12 | `col_a_direct_state_local_support` | 4. Direct state/local candidate support |
| 13 | `col_a_other_disbursements` | 5. Other Disbursements |
| 14 | `col_a_total_disbursements` | 6. Total Disbursements |
| 15 | `col_a_cash_on_hand_beginning_period` | 7. Beginning COH |
| 16 | `col_a_receipts_period` | 8. Receipts |
| 17 | `col_a_subtotal` | 9. Subtotal |
| 18 | `col_a_disbursements_period` | 10. Disbursements |
| 19 | `col_a_cash_on_hand_close_of_period` | 11. Ending COH |
| 20 | `col_b_total_receipts` | 1. Total receipts |
| 21 | `col_b_transfers_to_fed` | 2. Transfers to FED or allocation |
| 22 | `col_b_transfers_to_state_local` | 3. Transfers to state/local Party organizations |
| 23 | `col_b_direct_state_local_support` | 4. Direct state/local candidate support |
| 24 | `col_b_other_disbursements` | 5. Other Disbursements |
| 25 | `col_b_total_disbursements` | 6. Total Disbursements |
| 26 | `col_b_cash_on_hand_beginning_period` | 7. Beginning COH (as of Jan 1) |
| 27 | `col_b_receipts_period` | 8. Receipts |
| 28 | `col_b_subtotal` | 9. Subtotal |
| 29 | `col_b_disbursements_period` | 10. Disbursements |
| 30 | `col_b_cash_on_hand_close_of_period` | 11. Ending COH |
|  | `amended_cd` | AMENDED CD |
|  | `account_identifier` | System code for account named in Field #4 |

### SchL

| Column | Field | FEC label |
|---:|---|---|
| 1 | `form_type` | FORM TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `record_id_number` | RECORD ID NUMBER (for account name) |
| 5 | `account_name` | ACCOUNT NAME |
| 6 | `coverage_from_date` | COVERAGE FROM DATE |
| 7 | `coverage_through_date` | COVERAGE THROUGH DATE |
| 8 | `col_a_itemized_receipts_persons` | 1a. Itemized Receipts From Persons |
| 9 | `col_a_unitemized_receipts_persons` | 1b. Unitemized Receipts From Persons |
| 10 | `col_a_total_receipts_persons` | 1c. Total Receipts From Persons |
| 11 | `col_a_other_receipts` | 2. OTHER RECEIPTS |
| 12 | `col_a_total_receipts` | 3. TOTAL RECEIPTS |
| 13 | `col_a_voter_registration_disbursements` | 4(a). Voter Registration DISBURSEMENTS |
| 14 | `col_a_voter_id_disbursements` | 4(b). Voter ID DISBURSEMENTS |
| 15 | `col_a_gotv_disbursements` | 4(c). GOTV DISBURSEMENTS |
| 16 | `col_a_generic_campaign_disbursements` | 4(d). Generic Campaign DISBURSEMENTS |
| 17 | `col_a_disbursements_subtotal` | 4(e) Line 4 Total |
| 18 | `col_a_other_disbursements` | 5. OTHER DISBURSEMENTS |
| 19 | `col_a_total_disbursements` | 6. TOTAL DISBURSEMENTS |
| 20 | `col_a_cash_on_hand_beginning_period` | 7. CASH ON HAND (Beginning) |
| 21 | `col_a_receipts_period` | 8. RECEIPTS |
| 22 | `col_a_subtotal` | 9. SUBTOTAL |
| 23 | `col_a_disbursements_period` | 10. DISBURSEMENTS |
| 24 | `col_a_cash_on_hand_close_of_period` | 11. ENDING CASH ON HAND |
| 25 | `col_b_itemized_receipts_persons` | 1a. Itemized Receipts From Persons |
| 26 | `col_b_unitemized_receipts_persons` | 1b. Unitemized Receipts From Persons |
| 27 | `col_b_total_receipts_persons` | 1c. Total Receipts From Persons |
| 28 | `col_b_other_receipts` | 2. OTHER RECEIPTS |
| 29 | `col_b_total_receipts` | 3. TOTAL RECEIPTS |
| 30 | `col_b_voter_registration_disbursements` | 4(a). Voter Registration DISBURSEMENTS |
| 31 | `col_b_voter_id_disbursements` | 4(b). Voter ID DISBURSEMENTS |
| 32 | `col_b_gotv_disbursements` | 4(c). GOTV DISBURSEMENTS |
| 33 | `col_b_generic_campaign_disbursements` | 4(d). Generic Campaign DISBURSEMENTS |
| 34 | `col_b_disbursements_subtotal` | 4(e) Line 4 Total |
| 35 | `col_b_other_disbursements` | 5. OTHER DISBURSEMENTS |
| 36 | `col_b_total_disbursements` | 6. TOTAL DISBURSEMENTS |
| 37 | `col_b_cash_on_hand_beginning_period` | 7. CASH ON HAND as of Jan 1 |
| 38 | `col_b_receipts_period` | 8. RECEIPTS |
| 39 | `col_b_subtotal` | 9. SUBTOTAL |
| 40 | `col_b_disbursements_period` | 10. DISBURSEMENTS |
| 41 | `col_b_cash_on_hand_close_of_period` | 11. ENDING CASH ON HAND |

### TEXT

| Column | Field | FEC label |
|---:|---|---|
| 1 | `rec_type` | REC TYPE |
| 2 | `filer_committee_id_number` | FILER COMMITTEE ID NUMBER |
| 3 | `transaction_id` | TRANSACTION ID NUMBER |
| 4 | `back_reference_tran_id` | BACK REFERENCE TRAN ID NUMBER |
| 5 | `back_reference_sched_name` | BACK REFERENCE SCHED / FORM NAME |
| 6 | `text` | TEXT4000 |
|  | `form_type` | FORM TYPE |

