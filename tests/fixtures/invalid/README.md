# Invalid fixtures for `Filing::validate`

Each file is a copy of a real, FEC-accepted filing from `..` with one or two
deliberate defects introduced, so `tests/validate_fixtures.rs` can assert
that exactly the expected rules fire (and nothing else). The FEC message
number refers to the FEC's *Validation errors explained* page.

Base filing: `F3XA_2011827.fec` (an 8.5 Form 3X amendment with one Schedule
A, four Schedule B, and one Schedule E line) unless noted.

| File | Defect | Expected rule(s) |
|---|---|---|
| `duplicate_tran_id.fec` | line 5 reuses line 4's transaction id `SB21B.4120` | `duplicate_transaction_id` (#40) on line 5 |
| `bad_filer_id.fec` | cover and every body line carry the 8-character id `C0094412` | `filer_id_format` (#5) on line 2 only |
| `filer_id_mismatch.fec` | line 6 is filed under `C00944125`, the cover under `C00944124` | `filer_id_mismatch` (#21) on line 6 |
| `field_too_long.fec` | 31-character contributor last name (max 30) on line 3; 201-character payee organization name (max 200) on line 4 | `field_too_long` (#7) twice |
| `dangling_back_reference.fec` | two memo Schedule A lines inserted: line 4 back-references the real `SA11AI.4116`, line 5 back-references the non-existent `SA11AI.9999` | `back_reference_not_found` (#10/#41) on line 5 only |
| `illegal_character.fec` | DEL (0x7F) inside a payee name on line 4; Windows-1252 byte 0x9E (`ž`, excluded by the FEC) on line 7 -- the file is therefore not valid UTF-8 and exercises the Windows-1252 fallback | `illegal_character` (#12) twice |
| `amendment_missing_ids.fec` | the amendment's header has a blank report id and amendment number; line 4's payee name is `"Election "CFO""` (a quote inside a quoted field) | `amendment_needs_original_id` (#17), `amendment_needs_number` (#9), `embedded_double_quote` (#30) |
| `bad_dates_and_amounts.fec` | cover date signed `20261301`; line 4 date `20260231`; line 5 date `2026-06-22`; line 6 amount `$5,500.00`; line 7 amount `1500.005` | `not_a_real_date` (#33) x2, `bad_date_format` (#32), `invalid_amount` (#34) x2 |
| `multi_form.fec` | a second `F3XN` cover record appended as line 9, with its treasurer last name blanked | `multiple_forms` (#23), `required_field_empty` (#3) on line 9 |
| `wrong_schedule_for_form.fec` | base `F24N_2011832.fec` with a Schedule A line appended as line 5 | `schedule_not_allowed_with_form` (#19) on line 5 |
