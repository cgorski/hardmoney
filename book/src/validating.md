# Validating a Filing

The FEC does not accept a filing on trust. Every electronic submission
goes through a validator -- the same engine ships as *WebCheck* and
inside FECFile -- that emits **failing** messages, which reject the
filing outright, and **warning** messages, which are reported to the
filer but do not block acceptance. Committees and their software vendors
run it before they file; Reports Analysis Division analysts read its
output after.

`Filing::validate` and `hardmoney validate` reimplement the checks that
can be evaluated from a filing alone -- structure, IDs, field types,
lengths, character set, dates, amounts, code lists, transaction-ID
integrity -- with the same two severities, wording modelled on the FEC's
published messages, and, for every per-field rule, the FEC's own field
specification as the source of truth rather than a length typed into
Rust.

## The command

```text
$ hardmoney validate --help
Check a .fec file against the FEC's acceptance rules

Usage: hardmoney validate [OPTIONS] <PATH>

Arguments:
  <PATH>  Path to a `.fec` file

Options:
      --json
          Print the findings as a JSON document instead of one line each
      --strict-warnings
          Exit 1 on warnings too, not only on errors
      --oracle <ORACLE>
          Also submit the file to an external validator and diff its findings against ours. Sends the file over the network [possible values: webcheck]
      --strict-oracle
          With `--oracle`, exit 1 if the oracle and hardmoney disagree on any finding
      --webcheck-api-key <WEBCHECK_API_KEY>
          FEC vendor API key for WebCheck's SOAP service. Without it the credential-free upload channel (what the WebCheck web page uses) is taken. Ignored without `--oracle webcheck` [env: WEBCHECK_API_KEY]
      --webcheck-email <WEBCHECK_EMAIL>
          Contact e-mail to pass WebCheck's SOAP service (it e-mails results for files over 20 MB). Ignored without `--webcheck-api-key` [env: WEBCHECK_EMAIL]
  -h, --help
          Print help (see more with '--help')
```

A clean filing prints one line and exits 0:

```text
$ hardmoney validate tests/fixtures/F3XN_2011831.fec
ACCEPTABLE: F3XN, 144 body line(s), 0 error(s), 0 warning(s)
```

A defective one prints one finding per line -- `SEVERITY line N
FORM_TYPE field: message`, the shape of a row of WebCheck output -- then
the verdict and a per-rule tally, and exits 1. This is one of the
deliberately broken fixtures in `tests/fixtures/invalid/`: a real 8.5
Form 3X amendment whose header has had its report id and amendment
number blanked, and whose line 4 payee is `"Election "CFO""`:

```text
$ hardmoney validate tests/fixtures/invalid/amendment_missing_ids.fec
ERROR line 1 HDR report_id: Amended filing must have an ID of the "Original" (expected FEC-<filing number>, found '')
ERROR line 1 HDR report_number: Amended filing must have an "Amendment Number" (found '')
WARN  line 4 SB21B payee_organization_name: Embedded double-quotes (") not allowed in PAYEE ORGANIZATION NAME
NOT ACCEPTABLE: F3XA, 6 body line(s), 2 error(s), 1 warning(s)
       1 error   amendment_needs_original_id
       1 error   amendment_needs_number
       1 warning embedded_double_quote
error: validation failed with 2 error(s) and 1 warning(s)
```

And one with a field-level defect, where the messages carry the FEC's
own description of the field and the number that matters:

```text
$ hardmoney validate tests/fixtures/invalid/field_too_long.fec
ERROR line 3 SA11AI contributor_last_name: CONTRIBUTOR LAST NAME exceeds maximum length of 30 (31 characters)
ERROR line 4 SB21B payee_organization_name: PAYEE ORGANIZATION NAME exceeds maximum length of 200 (201 characters)
NOT ACCEPTABLE: F3XA, 6 body line(s), 2 error(s), 0 warning(s)
       2 error   field_too_long
error: validation failed with 2 error(s) and 0 warning(s)
```

The file is parsed **leniently**: a body line the FEC would ignore
(unknown record type) becomes an `unrecognized_form_type` warning
rather than a reason to stop. Only a file that cannot be parsed at all
-- no cover line, an unknown header version -- is reported on stderr as
a plain error.

### Exit codes

| Exit | When |
|---|---|
| 0 | no `ERROR` finding: the FEC would accept the filing, possibly with warnings |
| 1 | at least one `ERROR` finding; or, with `--strict-warnings`, any finding at all; or the file could not be parsed |

`--strict-warnings` is for a pipeline that wants a completely clean
file, or a vendor testing their own output.

### `--json`

```text
$ hardmoney validate --json tests/fixtures/invalid/duplicate_tran_id.fec
{
  "file": "tests/fixtures/invalid/duplicate_tran_id.fec",
  "form_type": "F3XA",
  "version": "8.5",
  "line_count": 6,
  "acceptable": false,
  "errors": 1,
  "warnings": 0,
  "findings_by_rule": {
    "duplicate_transaction_id": 1
  },
  "findings": [
    {
      "severity": "error",
      "rule": "duplicate_transaction_id",
      "line_no": 5,
      "form_type": "SB21B",
      "field": "transaction_id_number",
      "message": "Tran ID SB21B.4120 is NOT UNIQUE - This one is same as other(s) (first used on line 4)"
    }
  ]
}
error: validation failed with 1 error(s) and 0 warning(s)
```

Every finding has `severity`, `rule` (the `snake_case` name), `line_no`
(1-based physical line; 1 is `HDR`, 2 the cover), `form_type`, `field`
(the canonical name, or `null` for a structural finding), and `message`.
The exit status is the same as without `--json`.

## The rules

`Rule` is an enum of 32 variants. Each maps to a message in the FEC's
published *Validation errors explained* list and has a fixed
`Severity`: `Error` is a failing message, `Warning` is not. The table
is generated from the enum itself (`Rule::severity()`,
`Rule::fec_message()`), so it is the code:

| Rule | Severity | FEC message |
|---|---|---|
| **Structure** | | |
| `header_first` | error | HDR record must be First in File |
| `cover_second` | error | "Cover" (eg. F3A, F3XN, ...) must be 2nd in File |
| `filing_type_fec` | error | Filing must be an "FEC" Type of filing |
| `current_format` | warning | Filing must be in the current FEC format |
| `amendment_needs_original_id` | error | Amended filing must have an ID of the "Original" |
| `amendment_needs_number` | error | Amended filing must have an "Amendment Number" |
| `header_inconsistent_with_amendment_status` | warning | Header (HDR) inconsistent with Orig/Amend status |
| `multiple_forms` | error | Multi-Form Filings are NOT Allowed |
| `schedule_not_allowed_with_form` | error | Schedule does not belong with Form ____ |
| `unrecognized_form_type` | warning | Unrecognized Form Type / Record Ignored |
| **IDs** | | |
| `filer_id_format` | error | ID# _________ NOT Correct FEC ID# Format |
| `filer_id_mismatch` | error | ID# _________ NOT SAME AS Cover Page ID# _________ |
| **Per field** (driven by `FieldSpec`) | | |
| `required_field_empty` | error | {field} is Required, but field is Empty |
| `recommended_field_empty` | warning | {field} is Missing |
| `conditionally_required_field_empty` | warning | Conditionally Required field is Empty |
| `field_too_long` | error | {field} exceeds maximum length of ______ |
| `illegal_character` | error | Illegal character(s) found in text field |
| `f99_illegal_character` | warning | Illegal character(s) found in text line #____ (Used for F99's) |
| `embedded_double_quote` | warning | Embedded double-quotes (") not allowed |
| `bad_date_format` | error | Bad Date - ________ not YYYYMMDD format |
| `not_a_real_date` | error | ________ is not a Real Date |
| `date_out_of_range` | warning | __{date}__ is outside range of 1960-2099 |
| `invalid_amount` | error | Invalid Amount format: ____________ |
| `non_numeric` | error | Non-numeric data in Numeric Field |
| `invalid_allowed_value` | warning | Value "_" is Invalid for this field |
| `pattern_mismatch` | warning | Value "_" does not match the required format |
| `invalid_state_code` | warning | __ not a valid 2-character USPS State Code |
| `invalid_entity_type` | warning | Entity Type [___] is not an acceptable value |
| `invalid_support_oppose_code` | warning | Sup/Opp Code "___" Invalid (Valid Codes: S, O) |
| **Cross-line** | | |
| `duplicate_transaction_id` | error | Tran ID is NOT UNIQUE - This one is same as other(s) |
| `back_reference_not_found` | error | Back-Reference TRAN-ID does not match Sched TRAN-ID |
| **Form 99** | | |
| `f99_text_too_long` | error | Body of text exceeds maximum of 20,000 characters (F99 filings) |

`Rule` implements `Display`/`FromStr` in `snake_case` (which is also
its JSON form) and iteration via `strum::IntoEnumIterator`; it is
`#[non_exhaustive]`, so match it with a wildcard.

### Structural rules versus per-field rules

The **structural** and **cross-line** rules are written out in
`src/parser/validate.rs`: the header must be first and say `FEC`; an
amendment (`F3XA`) must carry `FEC-<n>` and an amendment number while a
new report (`F3XN`) should carry neither; the body may not contain a
second cover record; a schedule must be one the spec associates with the
filing's form (`SchA` belongs with F3, F3X, F3P, F3L -- not F24); every
body line's filer id must be a well-formed committee or candidate id
and match the cover's; transaction ids must be unique
(case-insensitively) and every `back_reference_tran_id_number` must name
one that exists in the file; a Form 99's text block is capped at 20,000
characters.

The **per-field** rules have no field names or lengths in them at all.
For every body line, the validator walks `line.table().specs()` -- the
FEC's spec workbook rows for that table, compiled in from
`data/fec-spec/spec-8.5.json` (see
[The Schema](./library-schema.md#fieldspec-what-the-fec-says-about-a-field))
-- and for each field applies what the row says:

| `FieldSpec` | Drives |
|---|---|
| `required` = `Error` / `Warning` / `Conditional(text)` | `required_field_empty` / `recommended_field_empty` / `conditionally_required_field_empty` on a blank -- with the `rule` text parsed for a condition, so `CONTRIBUTOR LAST NAME` (`Required if [IND|CAN]`) is only required when the entity type is one of those |
| `max_len` | `field_too_long` |
| `kind` = `Numeric` on a date field | `bad_date_format`, `not_a_real_date`, `date_out_of_range` |
| `kind` = `Numeric` otherwise | `non_numeric` |
| `kind` = `Amount` | `invalid_amount` (`parse_money` must accept it: `[-]digits[.dd]`, no `$` or commas) |
| `allowed_values`, `pattern` | `invalid_allowed_value`, `pattern_mismatch` |
| the field's name | `invalid_entity_type`, `invalid_support_oppose_code`, `invalid_state_code` |
| every text field | `illegal_character` (outside ASCII 32-126 and Latin-1 128-168, 173), `embedded_double_quote` |

```rust
use hardmoney::Table;
use hardmoney::parser::Requirement;

let spec = Table::SchA.spec("contributor_last_name").unwrap();
println!(
    "{}: kind {:?}, max_len {:?}, required {:?}, rule {:?}",
    spec.description, spec.kind, spec.max_len, spec.required, spec.rule
);
assert_eq!(spec.max_len, Some(30));
assert_eq!(spec.required, Requirement::Error);
assert_eq!(spec.rule, Some("Required if [IND|CAN]"));

let donor = Table::SchA.spec("donor_committee_fec_id").unwrap();
assert!(matches!(donor.required, Requirement::Conditional(_)));
println!("{}: required {:?}", donor.description, donor.required);
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
CONTRIBUTOR LAST NAME: kind AlphaNumeric, max_len Some(30), required Error, rule Some("Required if [IND|CAN]")
DONOR COMMITTEE FEC ID: required Conditional("Conditional Warning")
```

That `30` is why `field_too_long.fec` above failed at 31 characters, and
it is a data value: if the FEC raises it, the fix is a regenerated
`spec-8.6.json`, not a code change. Tables the current spec no longer
documents -- Schedule I, Form 8, Form 10 -- have layouts but no spec
rows, so they get the structural checks only.

### The FEC's wording

Finding messages are worded after the FEC's templates so that a filing
vendor, or anyone who has read a WebCheck report, recognises them: `ID#
C0094412 NOT Correct FEC ID# Format`, `20261301 is not a Real Date`,
`Tran ID SB21B.4120 is NOT UNIQUE - This one is same as other(s)`. Where
the FEC's text has blanks, the finding fills them and, where it helps,
adds what a filer needs to act -- the line the duplicate was first used
on, the field's description, the character's code point:

```text
ERROR line 4 SB21B payee_organization_name: Illegal character(s) found in text field: U+007F
ERROR line 7 SB21B payee_organization_name: Illegal character(s) found in text field: U+017E (ž)
```

## Where hardmoney deliberately differs

These are documented on the module (`src/parser/validate.rs`) and are
worth knowing before you compare its verdict with WebCheck's:

- **A superseded spec version is a warning, not a rejection.**
  `current_format` fires on anything but 8.5, at warning severity.
  hardmoney parses every version since 3.x on purpose; refusing a 2004
  filing would defeat that. The FEC rejects non-current formats *at
  upload time*, which is a different question from whether the file is
  well-formed.
- **On a superseded format, `required_field_empty` is demoted to a
  warning.** The workbook's required levels describe the *current*
  format, and real, accepted 3.x and 5.x filings leave `entity_type`
  blank. So on such a filing each `required_field_empty` finding carries
  `Severity::Warning` even though `Rule::RequiredFieldEmpty.severity()`
  is `Error` -- filter on the finding's `severity`, not the rule's.
  Every other check is format-stable and keeps its severity.
- **Embedded double quotes are a warning.** The parser already removes
  one pair of wrapping quotes (some vendors quote every field), so any
  `"` the validator sees is genuinely embedded; the FEC has accepted real
  filings with a stray one in free text.
- **Characters in a Form 99's free-text block are a warning**
  (`f99_illegal_character`), because the FEC has accepted real F99s with
  out-of-range bytes there. Elsewhere `illegal_character` is an error.
- **Leading blanks cannot be detected** (FEC #6/#31): the parser trims
  them before the validator sees the value.
- **Summary arithmetic is not here.** "Subtotal not supported by
  Schedule" is [Reconciling a Filing](./reconciling.md).
- **Cross-filing checks are not possible** from one file: transaction-id
  uniqueness "for the life of the report" (across amendments), and
  report-type-versus-form consistency.

The 2001 Merck PAC filing shows the first two at once:

```rust
use hardmoney::Filing;
use hardmoney::parser::{Rule, Severity};

let filing = Filing::open("tests/fixtures/F3XA_27789_v3.fec")?;
let report = filing.validate();
assert!(report.is_acceptable());
let counts = report.counts_by_rule();
println!("{:?}", counts.get(&Rule::CurrentFormat));
println!("{:?}", counts.get(&Rule::RequiredFieldEmpty));

// RequiredFieldEmpty is normally an error...
assert_eq!(Rule::RequiredFieldEmpty.severity(), Severity::Error);
// ...but on a superseded format each finding is demoted to a warning.
for f in report.iter().filter(|f| f.rule == Rule::RequiredFieldEmpty) {
    assert_eq!(f.severity, Severity::Warning);
}
for f in report.iter().filter(|f| f.rule == Rule::RequiredFieldEmpty).take(1) {
    println!("{f}");
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
Some(1)
Some(6)
WARN  line 2629 SB21B entity_type: ENTITY TYPE is Required, but field is Empty
```

## How real filings fare

Every one of the 25 real filings in `tests/fixtures/` was accepted by the
FEC, so an error-severity finding on any of them would be a bug in the
rule, not the filing. There are none -- and across the wider local
corpus of **102 accepted filings, zero error-severity findings**. The
warnings each fixture carries are pinned exactly in
`tests/validate_fixtures.rs`, so a change in either direction is
visible.

What the warnings on accepted filings typically are, in order of
frequency: `recommended_field_empty` (a payee's street or ZIP left
blank -- the FEC marks address fields `X (warning)`), `current_format`
on anything older than 8.5, `pattern_mismatch` (a one-digit candidate
district in the 2001 Merck filing; the FEC later required two), and on
3.x-5.x filings the demoted `required_field_empty`. A year-end 2007
report at spec 6.1:

```text
$ hardmoney validate tests/fixtures/F3XN_320000_v6.1.fec
WARN  line 1 HDR fec_version: Filing must be in the current FEC format (found 6.1, current is 8.5)
WARN  line 8 SB21B payee_city: PAYEE CITY is Missing
WARN  line 8 SB21B payee_state: PAYEE STATE is Missing
WARN  line 8 SB21B payee_zip_code: PAYEE ZIP is Missing
WARN  line 13 SB21B payee_street_1: PAYEE STREET 1 is Missing
WARN  line 13 SB21B payee_zip_code: PAYEE ZIP is Missing
WARN  line 14 SB21B payee_street_1: PAYEE STREET 1 is Missing
WARN  line 14 SB21B payee_zip_code: PAYEE ZIP is Missing
WARN  line 17 SB21B payee_city: PAYEE CITY is Missing
WARN  line 17 SB21B payee_state: PAYEE STATE is Missing
WARN  line 17 SB21B payee_zip_code: PAYEE ZIP is Missing
WARN  line 18 SB21B payee_street_1: PAYEE STREET 1 is Missing
WARN  line 18 SB21B payee_zip_code: PAYEE ZIP is Missing
WARN  line 21 SB21B payee_city: PAYEE CITY is Missing
WARN  line 21 SB21B payee_state: PAYEE STATE is Missing
WARN  line 21 SB21B payee_zip_code: PAYEE ZIP is Missing
ACCEPTABLE: F3XN, 20 body line(s), 0 error(s), 16 warning(s)
      15 warning recommended_field_empty
       1 warning current_format
```

Exit 0 -- and exit 1 with `--strict-warnings`.

The negative side is covered by `tests/fixtures/invalid/`: ten copies of
accepted filings with one or two defects each (a reused transaction id,
an eight-character filer id, a 31-character name, a dangling back
reference, a `DEL` byte and a Windows-1252 `ž`, a blanked amendment
header, impossible dates and `$5,500.00`, a second cover record, a
Schedule A under a Form 24), each pinned to exactly the rules it must
trigger and no others. The README there lists them with the FEC
message number each corresponds to.

## From Rust

`Filing::validate` never fails -- a filing that parsed can always be
checked -- and returns a `Validation`: every `Finding`, in file order.

```rust
use hardmoney::Filing;

let filing = Filing::open("tests/fixtures/invalid/duplicate_tran_id.fec")?;
let report = filing.validate(); // never fails
println!("acceptable: {}", report.is_acceptable());
for f in report.errors() {
    println!("{f}");
    println!(
        "  rule={} severity={} line={} form={} field={:?}",
        f.rule, f.severity, f.line_no, f.form_type, f.field
    );
}
println!("{} error(s), {} warning(s)", report.error_count(), report.warning_count());
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
acceptable: false
ERROR line 5 SB21B transaction_id_number: Tran ID SB21B.4120 is NOT UNIQUE - This one is same as other(s) (first used on line 4)
  rule=duplicate_transaction_id severity=error line=5 form=SB21B field=Some("transaction_id_number")
1 error(s), 0 warning(s)
```

`Validation` gives you `is_acceptable()` (no error-severity findings),
`errors()` and `warnings()` iterators, `error_count()`,
`warning_count()`, `counts_by_rule()`, `iter()`/`IntoIterator`,
`Display` (one finding per line, as the CLI prints them), and with the
`serde` feature `Serialize`. `Finding` is `severity`, `rule`, `line_no`,
`form_type`, `field: Option<&'static str>`, `message`. `Rule` and
`Severity` are small, `Copy`, and ordered (`Warning < Error`):

```rust
use hardmoney::parser::{Rule, Severity};

assert_eq!(Rule::FieldTooLong.severity(), Severity::Error);
assert_eq!(Rule::CurrentFormat.severity(), Severity::Warning);
assert_eq!(Rule::FieldTooLong.fec_message(), "{field} exceeds maximum length of ______");
assert_eq!(Rule::FieldTooLong.to_string(), "field_too_long");
assert_eq!("duplicate_transaction_id".parse::<Rule>()?, Rule::DuplicateTransactionId);
assert!(Severity::Warning < Severity::Error);
# Ok::<(), Box<dyn std::error::Error>>(())
```

If you parsed leniently, validate the `Lenient<Filing>` rather than
unwrapping it first: `Lenient<Filing>::validate` runs `Filing::validate`
and adds one `unrecognized_form_type` warning per skipped line, which is
what the FEC does with a record type it does not know (failing message
#18 -- despite the name, it ignores the record rather than rejecting the
filing). This is what the CLI does.

```rust
use hardmoney::parser::Rule;
use hardmoney::{Filing, ParseOptions};

let bytes = std::fs::read("/tmp/with_junk.fec")?; // a fixture plus one 'ZZZ' line
let lenient = Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT)?;
let report = lenient.validate(); // Filing::validate + one warning per skipped line
for f in report.iter().filter(|f| f.rule == Rule::UnrecognizedFormType) {
    println!("{f}");
}
assert!(report.is_acceptable()); // the FEC ignores such records; it does not reject
# Ok::<(), Box<dyn std::error::Error>>(())
```

```text
WARN  line 5 ZZZ form_type: Unrecognized Form Type / Record Ignored ('ZZZ': unknown form type)
```

(`/tmp/with_junk.fec` is the fixture-plus-one-bad-line file built in
[Strict vs. Lenient Parsing](./strict-vs-lenient.md).)

## Comparing with the FEC's WebCheck

Everything above is hardmoney's reading of the FEC's rules. The FEC's own
reading is a web service: [WebCheck](https://efoservices.fec.gov/webcheck/),
the validator the Electronic Filing Office runs on every submission,
exposed so filers can check a file first. (FECfile+, the FEC's own
browser tool, has no validator step; its team tells users to download
the `.fec` and run it through WebCheck.) `--oracle webcheck` sends your
file there, prints WebCheck's findings after ours, and diffs the two:

```text
$ hardmoney validate tests/fixtures/invalid/field_too_long.fec --oracle webcheck
ERROR line 3 SA11AI contributor_last_name: CONTRIBUTOR LAST NAME exceeds maximum length of 30 (31 characters)
ERROR line 4 SB21B payee_organization_name: PAYEE ORGANIZATION NAME exceeds maximum length of 200 (201 characters)
NOT ACCEPTABLE: F3XA, 6 body line(s), 2 error(s), 0 warning(s)
       2 error   field_too_long

--- WebCheck (tests/fixtures/invalid/field_too_long.fec) ---
ERROR SA11AI #008 Contributor Last Name {YYYYYYYYYYYYYYYYYYYYYYYYYYYYYYY, DAVID BROCK}: exceeds maximum length of 30
ERROR SB21B #007 Recipient Organization Name {XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX...}: exceeds maximum length of 200
WebCheck NOT ACCEPTABLE (ERRORS): 2 error(s), 0 warning(s), filing type F3XA

--- Diff: hardmoney vs. WebCheck ---
Matched (2):
  ERROR line 3 SA11AI contributor_last_name: CONTRIBUTOR LAST NAME exceeds maximum length of 30 (31 characters)
    ~ ERROR SA11AI #008 Contributor Last Name {YYYYYYYYYYYYYYYYYYYYYYYYYYYYYYY, DAVID BROCK}: exceeds maximum length of 30
  ERROR line 4 SB21B payee_organization_name: PAYEE ORGANIZATION NAME exceeds maximum length of 200 (201 characters)
    ~ ERROR SB21B #007 Recipient Organization Name {XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX...}: exceeds maximum length of 200
Only ours (0):
Only theirs (0):
2 matched, 0 only ours, 0 only theirs
error: validation failed with 2 error(s) and 0 warning(s)
```

WebCheck does not report line numbers. It identifies a record by its
type and a name in braces (the payee, the contributor, a city) and a
field by its 1-based number and the FEC's label -- `#008 Contributor
Last Name` -- so a WebCheck finding prints as `SEVERITY FORM_TYPE #NNN
label {item}: message` where ours prints `SEVERITY line N FORM_TYPE
field: message`. The diff pairs findings by record type, field number
(ours from the bundled spec's column) and message template (ours from
`Rule::fec_message`; a template matches when its words appear in order in
WebCheck's message, so `Tran ID is NOT UNIQUE - This one is same as
other(s)` matches `Tran ID 'SB21B.4120' is NOT UNIQUE - This one is same
as other(s)`). Anything unpaired is listed under `Only ours` or `Only
theirs`; a matched pair whose severities differ is marked `[severity
differs]`. The summary line is `N matched, M only ours, K only theirs`.

The exit status is still decided by *our* errors (and
`--strict-warnings`): the oracle is evidence, not a verdict. Add
`--strict-oracle` to exit 1 on any disagreement -- the mode for a vendor
check that both validators say the same thing. A network or service
failure is reported on stderr and exits 1, since the oracle was asked
for. With `--json`, the document gains `oracle` (WebCheck's verdict,
counts, filing type, committee id and findings) and `oracle_diff`
(`matched` count, `only_ours`, `only_theirs`).

The file leaves your machine. `--oracle` is never on by default.

### What the FEC's validator says about our fixtures

Run on 2026-09-15 against the live service: every one of the accepted
fixtures in `tests/fixtures/` comes back `SUCCESS` with no messages, as
it should. On the ten `invalid/` fixtures, eight diff clean -- the
same field, the same rule. The two that do not are instructive:

- `amendment_missing_ids.fec` (an F3XA whose header has no report id or
  amendment number). We report two HDR errors -- `Amended filing must
  have an ID of the "Original"` and `... an "Amendment Number"` (FEC
  #17, #9) -- and a warning for an embedded quote. WebCheck reports one
  error on the cover: `F3XA #001 Form Type: Header (HDR) inconsistent
  with Orig/Amend status` (#8), and nothing about the quote. Same
  defect, attributed to different lines under different rules; `0
  matched, 3 only ours, 1 only theirs`.
- `multi_form.fec` (a second cover record, F3XN, inside an F3XA). Both
  flag the F3XN's empty treasurer name. We put `Multi-Form Filings are
  NOT Allowed` on the offending F3XN line; WebCheck puts it on the F3XA
  cover and *also* rejects the F3XN as `Schedule does not belong with
  Form F3XA`. `1 matched, 1 only ours, 2 only theirs`.

Two more things the live service taught us, both encoded in the
`webcheck` module. Its wording departs from the FEC's published list in
places -- `Filing Format must be Version 8.5` rather than `Filing must
be in the current FEC format`, `No Match Found for Back-Reference to
Schedule/TranID - SA11AI.9999` rather than `Back-Reference TRAN-ID does
not match Sched TRAN-ID`, `$5,500.00 not a Valid Amount of Expenditure
value` rather than `Invalid Amount format` -- and those observed
alternates are matched too. And it is not always deterministic: the
same eight-character committee id drew `ID# 'C0094412' NOT Correct FEC
ID# Format` on some submissions and `An FEC 'C9xxxxxxx' ID must be used
to file Form 5` on others.

A superseded-format filing shows the deliberate difference described
above. `F3XN_210000_v5.3.fec` is one `current_format` *warning* to us;
WebCheck fails it (`HDR #003 FEC Version#: Filing Format must be Version
8.5`, paired with ours and marked `[severity differs]`) and, having
parsed a 5.3 file with 8.5 column positions, reports two hundred more
misaligned-field messages -- `200 only theirs`, and exit 1 under
`--strict-oracle`.

### Two channels, one credential-free

WebCheck has two entry points. The public **upload** (`POST
/webcheck/services/upload`, `multipart/form-data`) is what the WebCheck
page itself calls; it needs no account and answers at once with the
HTML fragment the page renders, which is what `--oracle webcheck` uses.
Files over 20 MB get their results by e-mail instead, which is reported
as an error rather than an empty report.

The **SOAP service** (`POST /webcheck/services/validate`, operation
`validate(arg0: string, arg1: string, arg2: base64Binary) -> string`,
WSDL at `?wsdl`) is the interface for registered filing vendors and
requires a vendor API key: every request without one, whatever the
arguments, is answered `Error! API Key is invalid` (with a pointer to
Vendor Registration). Pass `--webcheck-api-key` (or `WEBCHECK_API_KEY`)
and optionally `--webcheck-email` to use it. The client sends the key as
`arg0` and the e-mail as `arg1`, decodes the JSON the service returns
inside `<return>`, and follows a results URL if one is given -- but the
WSDL loses the parameter names and nobody on this project holds a key,
so the path past the key check is untested. If you have one and it does
not work, the argument order in `webcheck::soap_envelope` is the first
thing to try.

### From Rust

```rust,no_run
use hardmoney::Filing;
use hardmoney::parser::webcheck::{WebCheck, diff};

let bytes = std::fs::read("filing.fec")?;
let ours = Filing::parse_bytes(&bytes)?.validate();
let theirs = WebCheck::new().submit("filing.fec", &bytes, None)?;
println!("{theirs}");                 // WebCheck's findings and verdict
let d = diff(&ours, &theirs);
println!("{d}");                      // Matched / Only ours / Only theirs
assert!(d.is_empty(), "the two validators disagree");
# Ok::<(), Box<dyn std::error::Error>>(())
```

`WebCheck::submit` returns an `OracleReport` -- `raw` (the response as
received), `result` (`SUCCESS`/`ERRORS`), the counts WebCheck itself
stated, `filing_type`, `committee_id`, and `findings: Vec<OracleFinding>`
(`line_no: Option<u64>`, `severity`, `form_type`, `item`, `field_no`,
`field_label`, `message`). It fails with a `WebCheckError`: `Transport`,
`Http { status, .. }`, `SoapFault { code, string }`, `Rejected {
message }` (the invalid-key answer), `Deferred { message }` (results by
e-mail), or `Unparseable { snippet }`. Parsing never panics on an
unexpected shape; the report also carries `counts_disagree()`, true if
the counts WebCheck stated differ from the findings parsed -- the alarm
for a changed response format. `parse_upload_response` and
`parse_soap_response` are public so the parsing can be tested on
captured responses, which `tests/webcheck_oracle.rs` does; the live
tests there run with `HARDMONEY_NETWORK_TESTS=1 cargo test --test
webcheck_oracle -- --ignored`.

Together with the previous two chapters this completes the loop a filing
vendor or a compliance team needs: parse, fix (`set`), **validate**,
[reconcile](./reconciling.md), [write](./writing-fec.md).
