# Tracking Independent Expenditures For or Against a Candidate

**Who this is for:** a watchdog, an opposition-research desk, or campaign
staff who need to know what outside groups -- super PACs above all -- are
spending for or against a specific candidate, and who need to know it
this week, not after the next quarterly report.

**What you'll have at the end:** the FEC's own official Schedule E data
(half a million independent expenditures back to 1975) queryable by
candidate and by support/oppose, plus the exact line items of a 48-hour
notice filed *today*, pulled straight from the FEC's document store --
through the REST API, through SQL, and through six lines of Rust.

The candidate is Sherrod Brown (`S6OH00163`), running for U.S. Senate in
Ohio in 2026; the filing is number 2011823, a Form 24 from a super PAC
called OHIO FLYER PAC. Both are real, and both are fetchable by anyone.

## The vocabulary

An **independent expenditure** (IE) is money a committee spends to
expressly advocate the election or defeat of a clearly identified
candidate, *without* coordinating with that candidate. It is the legal
mechanism by which a super PAC can spend unlimited amounts: the money
never touches the campaign. IEs are reported on **Schedule E**, and every
Schedule E line names the candidate it's about and carries a one-letter
**support/oppose code**: `S` means the spending supports the candidate,
`O` means it opposes them.

Because IEs can land days before an election, the FEC requires fast
notice on top of the normal periodic reports. A **48-hour notice** is
required whenever IEs about a race aggregate $10,000 or more, up to 20
days before the election; inside those last 20 days the threshold drops
to $1,000 and the deadline to **24 hours**. Both are filed on **Form 24**
(`F24`), and the filing's `report_type` field says which: `48` or `24`.
`F24N` is a new notice; `F24A` amends a previous one.

That gives you two sources of Schedule E data with different strengths:

| | FEC's aggregated data (`bulk-restore-dump schedule_e`) | Individual filings (`bulk-load-filing`) |
|---|---|---|
| What it is | The FEC's own `pg_dump` of every Schedule E line it has processed from **periodic reports** (Form 3X, 5, 3, 3P) | The raw `.fec` file of one filing, parsed by hardmoney |
| Coverage | 1975 to present, every filer at once | Whatever filings you ingest |
| Freshness | Weekly, and only after the periodic report containing the line is filed | Minutes after the filing hits `docquery.fec.gov` |
| Includes Form 24 notices? | **No** -- the dump has no `F24` rows at all | Yes |

You want both. The first answers "who has been spending against this
candidate all cycle"; the second answers "what was filed this morning".

## Step 1: a namespace, and the FEC's Schedule E dump

```bash
export DATABASE_URL="postgres://chris.gorski@localhost:5432/hardmoney_v1_test"
hardmoney schema-init --schema tut_ie
```

```text
namespace 'tut_ie': schema at version 2 (2 migration(s) applied now)
```

The FEC publishes a Postgres `pg_dump` archive of its Schedule E table,
`fec_fitem_sched_e.dump`, at
`https://www.fec.gov/files/bulk-downloads/data-dump/schedules/`. On the
day this was written it was 43,440,933 bytes (43 MB) with a
`Last-Modified` of Sunday, 13 September 2026 -- it's refreshed weekly.
`bulk-restore-dump` downloads it (caching under `~/.cache/hardmoney/dumps`),
runs `pg_restore`, and creates a friendly view over it:

```bash
hardmoney bulk-restore-dump --schema tut_ie schedule_e
```

```text
restored 549525 rows into disclosure.fec_fitem_sched_e (dump cached at /Users/you/.cache/hardmoney/dumps/schedule_e.dump)
independent_expenditures view refreshed in namespace 'tut_ie'
```

*(Illustrative: the book's database already had this dump restored, and
re-running the restore would drop and rebuild a table every other
namespace shares, so the command was not re-run. The row count, 549,525,
and the report-year range, 1975 through 2026, are real -- queried from
the restored table -- and the FEC's weekly refresh will change them.)*

Two things about that output. First, the table lands in a schema called
**`disclosure`**, not in `tut_ie`: the FEC's own DDL hard-codes that
name, and hardmoney treats the dump as reference data shared by every
namespace -- restore it once per database. Second, what *is* created in
`tut_ie` is a view, `independent_expenditures`, with readable column
names (`candidate_id` instead of `s_o_cand_id`, `support_oppose_code`
instead of `s_o_ind`). `schema-init` and `serve` recreate that view in
any namespace whenever the table exists, which is why a brand-new
namespace in a database that already has the dump gets it for free:

```bash
psql "$DATABASE_URL" -c "SET search_path TO tut_ie, public;" -c "SELECT count(*) FROM independent_expenditures;"
```

```text
 count
--------
 549525
```

You need `pg_restore` on your `PATH`. The other things worth knowing --
the harmless `pg_restore` warning about a trigger function, `--cache-dir`,
the two *large* dumps that need `--allow-large` -- are in
[Loading Bulk Data](./bulk-etl.md#an-alternative-restoring-the-fecs-own-database-dumps).

While you're here, load the committee and candidate registries too. The
dump's `committee_name` column is `NULL` on every 2026 row, so you'll
want `committees` to put names to IDs:

```bash
hardmoney bulk-load --schema tut_ie committees --cycle 2026
hardmoney bulk-load --schema tut_ie candidates --cycle 2026
```

```text
committees: loaded 20674 rows for cycle 2026 (replace)
candidates: loaded 8557 rows for cycle 2026 (replace)
```

## Step 2: everything spent against the candidate

Start the API and ask for Schedule E lines about Brown with code `O`:

```bash
hardmoney serve --schema tut_ie --bind 127.0.0.1:18092
```

```bash
curl -s "http://127.0.0.1:18092/independent-expenditures?candidate_id=S6OH00163&support_oppose_code=O&limit=3" | python3 -m json.tool
```

```json
[
    {
        "sub_id": 4090820261595915954,
        "cmte_id": "C00687103",
        "committee_name": null,
        "payee_name": "IN PURSUIT OF LLC",
        "candidate_id": "S6OH00163",
        "candidate_name": "BROWN, SHERROD",
        "candidate_office_state": "OH",
        "support_oppose_code": "O",
        "support_oppose_desc": "OPPOSE",
        "expenditure_amt": "250000.00",
        "expenditure_date": "2026-07-13T00:00:00",
        "expenditure_description": "MEDIA PLACEMENT",
        "rpt_yr": 2026,
        "election_cycle": 2026
    },
    {
        "sub_id": 4090820261595915975,
        "cmte_id": "C00687103",
        "committee_name": null,
        "payee_name": "TARGETED VICTORY LLC",
        "candidate_id": "S6OH00163",
        "candidate_name": "BROWN, SHERROD",
        "candidate_office_state": "OH",
        "support_oppose_code": "O",
        "support_oppose_desc": "OPPOSE",
        "expenditure_amt": "2500.00",
        "expenditure_date": "2026-07-13T00:00:00",
        "expenditure_description": "MEDIA PRODUCTION",
        "rpt_yr": 2026,
        "election_cycle": 2026
    },
    {
        "sub_id": 4090820261595915963,
        "cmte_id": "C00687103",
        "committee_name": null,
        "payee_name": "PEOPLE WHO THINK",
        "candidate_id": "S6OH00163",
        "candidate_name": "BROWN, SHERROD",
        "candidate_office_state": "OH",
        "support_oppose_code": "O",
        "support_oppose_desc": "OPPOSE",
        "expenditure_amt": "7750.00",
        "expenditure_date": "2026-07-10T00:00:00",
        "expenditure_description": "MEDIA PRODUCTION",
        "rpt_yr": 2026,
        "election_cycle": 2026
    }
]
```

Newest first. The route's filters are `candidate_id`, `cmte_id`, and
`support_oppose_code` (plus `limit`/`offset`); there is no `cycle`
filter, so a candidate who has run before comes back with every cycle's
spending interleaved by date -- Brown's rows go back to 2006. Use
`election_cycle` in the output to tell them apart, or move to SQL.

`expenditure_amt` is a string (`"250000.00"`) because it's an exact
decimal; the dump's column is `NUMERIC(14,2)`, which is why every value
here carries cents.

Who is doing the spending is a `GROUP BY`, joined to `committees` for the
names the dump leaves blank:

```bash
psql "$DATABASE_URL" -c "SET search_path TO tut_ie, public;" \
  -c "SELECT ie.cmte_id, c.cmte_nm, c.cmte_tp, count(*) AS items, sum(ie.expenditure_amt) AS total
      FROM independent_expenditures ie
      LEFT JOIN committees c ON c.cmte_id = ie.cmte_id AND c.cycle = 2026
      WHERE ie.candidate_id = 'S6OH00163'
        AND ie.support_oppose_code = 'O'
        AND ie.election_cycle = 2026
      GROUP BY 1, 2, 3
      ORDER BY total DESC;"
```

```text
  cmte_id  |                                        cmte_nm                                         | cmte_tp | items |   total
-----------+----------------------------------------------------------------------------------------+---------+-------+------------
 C00912865 | OHIO FLYER PAC                                                                         | O       |     4 | 1664222.00
 C00687103 | AMERICANS FOR PROSPERITY ACTION, INC. (AFP ACTION) DBA CVA ACTION AND DBA LIBRE ACTION | V       |    11 | 1475661.21
 C00571703 | SLF PAC                                                                                | O       |     3 |  416239.01
 C00747121 | RED SENATE                                                                             | O       |     1 |   10002.29
(4 rows)
```

Four committees, $3.57 million against Brown in the 2026 cycle as of the
FEC's 13 September data. `cmte_tp: "O"` is the FEC's code for an
independent-expenditure-only committee -- a super PAC; `V` is a hybrid
PAC with a separate non-contribution account. The biggest spender,
OHIO FLYER PAC, is the filer of the notice in the next step.

For the other side of the ledger, change `'O'` to `'S'`. And for the
long view, this is what the dump holds for Brown across every cycle:

```sql
SELECT election_cycle, support_oppose_code, count(*), sum(expenditure_amt)
FROM independent_expenditures
WHERE candidate_id = 'S6OH00163'
GROUP BY 1, 2 ORDER BY 1 DESC, 2;
```

```text
 election_cycle | support_oppose_code | count |     sum
----------------+---------------------+-------+--------------
           2026 | O                   |    19 |   3566124.51
           2026 | S                   |    43 |     48748.68
           2024 | O                   |   457 | 116057538.16
           2024 | S                   |   366 |  24811282.17
           ...
```

## Step 3: what the dump can't tell you

Look at OHIO FLYER PAC's four rows more closely:

```bash
curl -s "http://127.0.0.1:18092/independent-expenditures?candidate_id=S6OH00163&cmte_id=C00912865" \
  | python3 -c "import json,sys; [print(r['expenditure_date'][:10], r['expenditure_amt'], r['expenditure_description']) for r in json.load(sys.stdin)]"
```

```text
2026-06-26 383754.00 TV ADVERTISING
2026-05-11 313002.00 TV AND INTERNET ADVERTISING
2026-04-27 521670.00 TV AND INTERNET ADVERTISING (ALSO SUPPORTS JON HUSTED)
2026-04-17 445796.00 TV AND INTERNET ADVERTISING (ALSO SUPPORTS JON HUSTED)
```

The newest is 26 June. All four came from one filing (`file_num`
1995269, a Form 3X -- the committee's periodic report), and *every one*
of the 16,142 cycle-2026 rows in the dump has `filing_form = F3X`. The
FEC's Schedule E table is built from periodic reports; the 24- and
48-hour notices that arrive between them are not in it. On 14 September
2026, OHIO FLYER PAC filed a 48-hour notice of a $1,074,900 buy. It is
not in the dump, and won't be until the committee's next periodic report
is filed and processed.

## Step 4: ingest the notice itself

`bulk-load-filing` takes a filing ID, downloads the raw `.fec` file from
`docquery.fec.gov`, parses it with hardmoney's own parser, and stores the
header, the cover line, and every genuine Schedule E line:

```bash
hardmoney bulk-load-filing --schema tut_ie 2011823
```

```text
fetching filing 2011823 from docquery.fec.gov ...
ingested filing 2011823 (F24N): 1 Schedule E line(s), 0 skipped
```

Under half a second. (The same filing ships with the repository as
`tests/fixtures/F24N_2011823.fec`, so `hardmoney bulk-load-filing
--schema tut_ie tests/fixtures/F24N_2011823.fec` gives an identical
result offline -- the ID is read from the filename.) `0 skipped` means
every body line parsed; a non-zero count would be recorded, not hidden,
as explained in [Strict vs. Lenient Parsing](./strict-vs-lenient.md).

The cover page:

```bash
curl -s "http://127.0.0.1:18092/filings/2011823" | python3 -m json.tool
```

```json
{
    "filing_id": 2011823,
    "form_type": "F24N",
    "fec_version": "8.5",
    "committee_id": "C00912865",
    "is_amendment": false,
    "amends_filing_id": null,
    "header": {
        "ef_type": "FEC",
        "fec_version": "8.5",
        "record_type": "HDR",
        "report_id": "",
        "report_number": "",
        "soft_name": "FECFILE",
        "soft_ver": "8.5.1.0(F34)"
    },
    "summary": {
        "city": "ALEXANDRIA",
        "committee_name": "OHIO FLYER PAC",
        "date_signed": "20260914",
        "filer_committee_id_number": "C00912865",
        "form_type": "F24N",
        "original_amendment_date": "",
        "report_type": "48",
        "state": "VA",
        "street_1": "PO BOX 26141",
        "street_2": "",
        "treasurer_first_name": "CHRIS",
        "treasurer_last_name": "MARSTON",
        "treasurer_middle_name": "",
        "treasurer_prefix": "",
        "treasurer_suffix": "",
        "zip_code": "22313"
    },
    "skipped_lines": 0,
    "ingested_at": "2026-09-15T02:44:10.124810Z"
}
```

`form_type: "F24N"` and `report_type: "48"`: a new 48-hour notice, signed
14 September 2026. `is_amendment: false` -- if this were an `F24A`,
`amends_filing_id` would hold the filing it supersedes, and you'd want to
treat the earlier one as replaced.

And the line item:

```bash
curl -s "http://127.0.0.1:18092/filings/2011823/schedule-e" | python3 -m json.tool
```

```json
[
    {
        "line_index": 0,
        "payee_name": "STRATEGIC MEDIA PLACEMENT INC.",
        "expenditure_amt": "1074900.00",
        "expenditure_date": "2026-09-14",
        "support_oppose_code": "O",
        "candidate_id": "S6OH00163",
        "candidate_name": "SHERROD BROWN",
        "candidate_office_state": "OH"
    }
]
```

$1,074,900 against Brown, disseminated 14 September, paid to the same
media buyer as the four June-and-earlier rows in the dump. This single
line is 30% of everything the dump knows was spent against him all cycle,
and it was available within hours of filing.

Who filed it is one more lookup, since the registries are loaded:

```bash
curl -s "http://127.0.0.1:18092/committees/C00912865" | python3 -m json.tool
```

```json
[
    {
        "cmte_id": "C00912865",
        "cycle": 2026,
        "cmte_nm": "OHIO FLYER PAC",
        "tres_nm": "MARSTON, CHRIS",
        "cmte_city": "ALEXANDRIA",
        "cmte_st": "VA",
        "cmte_dsgn": "U",
        "cmte_tp": "O",
        "cmte_pty_affiliation": null,
        "org_tp": null,
        "connected_org_nm": "TEAM HUSTED",
        "cand_id": null
    }
]
```

A super PAC (`cmte_tp: "O"`), unauthorized by any candidate
(`cmte_dsgn: "U"`, `cand_id: null`), with a connected organization named
"TEAM HUSTED" -- Jon Husted being Brown's opponent.

Re-ingesting the same filing ID replaces the previous rows in one
transaction, so the safe habit for an amendment or a re-run is simply to
run `bulk-load-filing` again. To watch for new notices, the FEC's
document store publishes a feed of new filings; each one's numeric ID is
all you need.

## Step 5: the same thing in Rust

Everything `bulk-load-filing` did is available as library calls, and the
Schedule E extraction is short enough to show whole. This needs
`hardmoney` with the `fetch` feature and `rust_decimal` in `Cargo.toml`
(see [the Rust tutorial](./tutorial-rust-library.md#the-cargotoml) for
the exact lines):

```rust
use hardmoney::{Filing, ScheduleE, SupportOppose};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let filing = Filing::fetch(2011823)?;
    println!(
        "{} report_type={:?} from {:?}",
        filing.raw_form_type,
        filing.summary.get("report_type"),
        filing.summary.get("committee_name")
    );

    for e in filing.views::<ScheduleE>() {
        let verb = match e.support_oppose {
            Some(SupportOppose::Support) => "for",
            Some(SupportOppose::Oppose) => "against",
            _ => "(no S/O code)", // SupportOppose is #[non_exhaustive]
        };
        println!(
            "${} {} {} ({}) on {:?}, paid to {}",
            e.expenditure_amount.unwrap_or_default(),
            verb,
            e.candidate_name.as_deref().unwrap_or("?"),
            e.candidate_id_number.as_deref().unwrap_or("?"),
            e.dissemination_date,
            e.payee_name.as_deref().unwrap_or("?"),
        );
    }
    Ok(())
}
```

```text
F24N report_type=Some("48") from Some("OHIO FLYER PAC")
$1074900.00 against SHERROD BROWN (S6OH00163) on Some(2026-09-14), paid to STRATEGIC MEDIA PLACEMENT INC.
```

`filing.views::<ScheduleE>()` yields only lines whose table is
`Table::SchE` -- a Schedule A or B line in the same filing can never be
mistaken for an expenditure -- and each `ScheduleE` has an exact
`Decimal` amount, real `NaiveDate`s, a `candidate_name` assembled from
the split first/last fields the current spec uses, and `support_oppose`
as an enum with the raw code preserved in `Other(String)` if a filer ever
sends something that isn't `S` or `O`. `e.support_oppose_code()` gives
the raw `Option<&str>` back. The types are described in
[Tables and Typed Views](./typed-views.md#codes-as-enums-with-the-unknowns-preserved).

## A third source, for completeness

`bulk-load-all` also fills `committee_to_candidate_transactions` from the
FEC's `pas2` file, and that table has a generated column
`is_independent_expenditure` (true for transaction types `24A` and `24E`).
It's derived from the same periodic reports as the dump, is per-cycle
rather than all-history, and ships whole-dollar amounts, so the dump is
the better aggregated source -- but if you already have a full cycle
loaded, `SELECT ... WHERE cand_id = 'S6OH00163' AND
is_independent_expenditure` is right there.

## Clean up

Stop the server with Ctrl-C, then:

```bash
hardmoney schema-drop tut_ie --yes
```

```text
dropped namespace 'tut_ie'
```

The shared `disclosure` schema is not touched by `schema-drop`; the
restored dump stays available to every other namespace.

## Where to go next

- [The REST API](./rest-api.md#independent-expenditures-from-the-fecs-own-dump)
  for the 503 you'll see if the dump hasn't been restored, and every
  route's filters.
- [Loading Bulk Data](./bulk-etl.md#ingesting-a-single-filing-directly-for-precise-schedule-e-data)
  for `bulk-load-filing`'s `--strict` and `--filing-id` options and the
  library `ingest_filing_bytes` call.
- [Parsing Filings in Rust](./tutorial-rust-library.md) to build a
  pipeline around `Filing::fetch`.
