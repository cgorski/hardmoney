# Finding filings: openFEC and the e-file feed

Everything else in this book starts from a `.fec` file you already have,
or a filing id you already know. This chapter is about the step before
that: discovering which filings exist (for a committee, a cycle, a form
type, or "since the last time I looked") and getting their raw bytes onto
disk, where `parse`, `validate`, `reconcile`, `export`, and
`bulk-load-filing` can work on them.

There are two routes, and they answer different questions:

| Route | Command | Source | What it knows | How fresh |
|---|---|---|---|---|
| openFEC | `hardmoney filings` | `api.open.fec.gov/v1/filings/` | The FEC's processed view: amendment chains, `most_recent`, cover-page totals, paper filings too. | Hours to days after receipt. |
| E-file feed | `hardmoney efile watch` / `backfill` | `efilingapps.fec.gov` RSS and the daily zips at `fec.gov/files/bulk-downloads/electronic/` | Only what was e-filed, exactly as it arrived, including filings the FEC will later reject. | Minutes (feed); next day (zips). |

Both routes hand the filings they find to the same set of actions
(`--fetch`/`--out`, `--validate`, `--reconcile`, `--ingest`), and both go
through the same download cache, so a filing is fetched from
`docquery.fec.gov` once no matter how many times or ways you ask for it.

In the library these are the `hardmoney::fec` module: `OpenFec`,
`FilingsQuery`, `EfileFeed`, `daily_zip_filings`, `Cache`, and
`fetch_filing_bytes`. The module is enabled by the `fetch` feature; the
openFEC client additionally needs `serde` (it decodes JSON). The parser-only
build (`--no-default-features --features fetch`) gets the cache, the feed,
the daily zips, and `fetch_filing_bytes`, but not `OpenFec`.

## The API key

openFEC requires a key. It is free and instant at
<https://api.data.gov/signup/>. hardmoney looks for it in two places, in
order:

1. the environment variable `FEC_API_KEY`;
2. the file `~/fec_api_key.txt` (first line, whitespace trimmed).

If neither has one, every `filings` command fails before making a request:

```text
$ hardmoney filings --committee C00554709
error: no openFEC API key found (looked in FEC_API_KEY, /Users/chris.gorski/fec_api_key.txt); set FEC_API_KEY or save the key in ~/fec_api_key.txt -- a free key is at https://api.data.gov/signup/
```

The key is a secret and hardmoney treats it as one: it never appears in
output, logs, `Debug` formatting (`ApiKey` prints as `ApiKey(REDACTED)`),
or error messages. When an error carries the request URL, the key in it is
replaced:

```text
error: could not decode the JSON from https://api.open.fec.gov/v1/filings/?api_key=REDACTED&committee_id=C00554709&sort=-receipt_date&per_page=100&page=1: ...
```

Do not commit `fec_api_key.txt` to a repository; it lives in your home
directory, not the project, for exactly that reason.

The e-file feed and the daily zips need no key.

## Rate limits

openFEC allows 1,000 requests per hour per key. A `filings` query is
one request per page of up to 100 records, so a typical committee is one
or two requests; a cycle-wide `--form-type F24` query without `--limit` can
be hundreds.

When the API answers HTTP 429, hardmoney reads its `Retry-After` header,
sleeps that long (or 1 s, 2 s, 4 s if the header is absent), and retries,
up to three times. Two things stop it from silently hanging: a single
wait is capped at 60 seconds, and after the third retry it gives up. In
either case the error is typed
(`FecApiError::RateLimited { retry_after }`) and printed as:

```text
error: openFEC rate limit reached (1,000 requests per hour per key); the server asks to retry after 3600 second(s)
```

Library users can change the budget with `OpenFec::retry_policy`.

## `hardmoney filings`: asking openFEC

The query flags map one-to-one onto openFEC's `/filings/` parameters:

| Flag | openFEC parameter |
|---|---|
| `--committee C00554709` | `committee_id` |
| `--candidate H4CA11114` | `candidate_id` |
| `--cycle 2026` | `cycle` (validated: even, 1976-2100) |
| `--form-type F3X` | `form_type` (base type, no `N`/`A`/`T` suffix) |
| `--report-type Q2` | `report_type` |
| `--most-recent` | `most_recent=true` |
| `--since` / `--until` | `min_receipt_date` / `max_receipt_date` |

Results come newest first (`sort=-receipt_date`), 100 per page, and
`--limit N` stops after N records without fetching pages it does not need.
With no filter at all the command refuses to run unless you give a
`--limit`, because "every filing the FEC has" is half a million requests.

Here is the amendment chain from the [Amendments](./amendments.md)
chapter, as openFEC reports it:

```text
$ hardmoney filings --committee C00554709 --cycle 2016 --form-type F3 --report-type 12G
file_number  form  report  coverage                receipt_date  amend  most_recent  total_receipts  committee
1151343      F3    12G     2016-10-01..2016-10-19  2017-03-04    2      yes                25270.16  C00554709
1131084      F3    12G     2016-10-01..2016-10-19  2016-12-08    1      no                 25270.16  C00554709
1118027      F3    12G     2016-10-01..2016-10-19  2016-10-27    0      no                 25270.16  C00554709
```

`amend` is openFEC's `amendment_version` (0 = original), and `most_recent`
is its verdict on which version is current. `--json` prints every field
openFEC returns, under openFEC's own names, with dates as ISO strings and
money as decimal strings (never floats):

```text
$ hardmoney filings --committee C00554709 --json --limit 1
[
  {
    "file_number": 1993971,
    "fec_file_id": "FEC-1993971",
    "form_type": "F3",
    "form_category": "REPORT",
    "report_type": "Q2",
    "report_type_full": "JULY QUARTERLY",
    "report_year": 2026,
    "cycle": 2026,
    "committee_id": "C00554709",
    "committee_name": "MARK DESAULNIER FOR CONGRESS",
    "committee_type": "H",
    "candidate_id": null,
    "candidate_name": null,
    "amendment_indicator": "N",
    "amendment_chain": [
      1993971
    ],
    "amendment_version": 0,
    "most_recent": true,
    "most_recent_file_number": 1993971,
    "previous_file_number": 1993971,
    "is_amended": false,
    "receipt_date": "2026-07-15T00:00:00",
    "coverage_start_date": "2026-05-14",
    "coverage_end_date": "2026-06-30",
    "update_date": "2026-07-15",
    "fec_url": "https://docquery.fec.gov/dcdev/posted/1993971.fec",
    "csv_url": "https://docquery.fec.gov/csv/971/1993971.csv",
...
```

### Paper filings have negative numbers

openFEC's `/filings/` includes filings made on paper (`means_filed:
"paper"`), and it numbers them negatively. A chain that began on paper
carries the negative id; C00554709's Form 1 chain starts at `-6899830`.
The table marks these, and there is nothing to download for them: a paper
filing has an image, not a `.fec`. The action flags skip them with a note
rather than failing.

```text
900330            F1                                    2014-01-15    1      no                           C00554709
-6899830 (paper)  F1                                    2014-01-13    0      no                           C00554709
```

### Doing something with what you found

Four flags act on each filing the query returns. All of them download the
raw `.fec` first (from `fec_url`, falling back to
`https://docquery.fec.gov/dcdev/posted/<id>.fec`), through the cache.
Every host on this page is a default: `--openfec-base`,
`--docquery-base`, `--efile-rss-url`, and `--fec-www-base` (or the
`HARDMONEY_*` variables they read) point these commands at a mirror or
the FEC's test environment, and a `docquery.fec.gov` link in a feed item
or a `fec_url` is rebased onto `--docquery-base` before the download
([Security and provenance](./security-and-provenance.md#pointing-hardmoney-at-a-mirror-or-proxy)):

* `--fetch DIR` copies it to `DIR/<id>.fec`;
* `--validate` runs the FEC's acceptance rules (the same check as
  [`hardmoney validate`](./validating.md)) and prints the counts;
* `--reconcile` recomputes the cover page from the schedules (the same
  check as [`hardmoney reconcile`](./reconciling.md)) and prints whether
  it balances;
* `--ingest` loads it into Postgres exactly as
  [`bulk-load-filing`](./bulk-etl.md#ingesting-a-single-filing-directly-for-precise-schedule-e-data)
  would, amendment chain resolved. Needs `--database-url` (or
  `DATABASE_URL`) and honours `--schema`.

```text
$ hardmoney filings --committee C00554709 --cycle 2026 --most-recent --form-type F3 --limit 3 --fetch /tmp/desaulnier --validate --reconcile
file_number  form  report  coverage                receipt_date  amend  most_recent  total_receipts  committee
1993971      F3    Q2      2026-05-14..2026-06-30  2026-07-15    0      yes                52166.00  C00554709
1978418      F3    12P     2026-04-01..2026-05-13  2026-05-21    0      yes                24267.00  C00554709
1964145      F3    Q1      2026-01-01..2026-03-31  2026-04-15    0      yes                84656.17  C00554709
  1993971: saved /tmp/desaulnier/1993971.fec
  1993971: validate ACCEPTABLE: F3N, 0 error(s), 0 warning(s)
  1993971: reconcile F3: balances (50 line(s))
  1978418: saved /tmp/desaulnier/1978418.fec
  1978418: validate ACCEPTABLE: F3N, 0 error(s), 0 warning(s)
  1978418: reconcile F3: balances (50 line(s))
  1964145: saved /tmp/desaulnier/1964145.fec
  1964145: validate ACCEPTABLE: F3N, 0 error(s), 0 warning(s)
  1964145: reconcile F3: balances (50 line(s))
3 filing(s), 0 action failure(s)
```

Ingesting the 2016 chain above into a fresh namespace reproduces
openFEC's chain columns from the raw files alone:

```text
$ hardmoney filings --committee C00554709 --cycle 2016 --form-type F3 --report-type 12G --ingest --database-url postgres://chris.gorski@localhost/hardmoney_fec_demo --schema fec_demo
file_number  form  report  coverage                receipt_date  amend  most_recent  total_receipts  committee
1151343      F3    12G     2016-10-01..2016-10-19  2017-03-04    2      yes                25270.16  C00554709
1131084      F3    12G     2016-10-01..2016-10-19  2016-12-08    1      no                 25270.16  C00554709
1118027      F3    12G     2016-10-01..2016-10-19  2016-10-27    0      no                 25270.16  C00554709
  1151343: ingested F3A: 0 Schedule E line(s), 0 skipped, amendment chain resolved (1 filing(s))
  1131084: ingested F3A: 0 Schedule E line(s), 0 skipped, amendment chain resolved (2 filing(s))
  1118027: ingested F3N: 0 Schedule E line(s), 0 skipped, amendment chain resolved (3 filing(s))
3 filing(s), 0 action failure(s)

$ psql hardmoney_fec_demo -c "select filing_id, form_type, amendment_version, most_recent, amendment_chain from fec_demo.filings order by filing_id"
 filing_id | form_type | amendment_version | most_recent |      amendment_chain
-----------+-----------+-------------------+-------------+---------------------------
   1118027 | F3N       |                 0 | f           | {1118027}
   1131084 | F3A       |                 1 | f           | {1118027,1131084}
   1151343 | F3A       |                 2 | t           | {1118027,1131084,1151343}
(3 rows)
```

A validation with errors, or a report that does not balance, is
information, not a failure: the command exits 1 only when something could
not be done (a download failed, the file would not parse, an ingest
failed). With `--json`, each record gains an `actions` object holding the
same results.

## The download cache

Every raw filing hardmoney downloads lands in

```text
~/.cache/hardmoney/
  filings/<id>.fec        raw filings, exactly as downloaded
  efile/YYYYMMDD.zip      daily e-filing archives (backfill)
  efile-seen.txt          filing ids `efile watch` has already processed
```

(`$XDG_CACHE_HOME/hardmoney` if that variable is set; override the whole
root with `HARDMONEY_CACHE_DIR` or `--cache-dir`. This is the same root
`bulk-restore-dump` keeps its `dumps/` under.) Writes are atomic (a
download interrupted by Ctrl-C never leaves a truncated file a later run
would trust), and a cached filing is never re-downloaded. Filings do not
change once posted, so there is no expiry.

```text
$ hardmoney efile cache-info
cache root:      /Users/chris.gorski/.cache/hardmoney
raw filings:     10 file(s), 149.0 KB  (/Users/chris.gorski/.cache/hardmoney/filings)
daily archives:  1 file(s), 14.8 KB  (/Users/chris.gorski/.cache/hardmoney/efile)
e-file seen ids: 951  (/Users/chris.gorski/.cache/hardmoney/efile-seen.txt)
$ hardmoney efile cache-clear
removed 10 raw filing(s) and 1 daily archive(s) (163.8 KB) from /Users/chris.gorski/.cache/hardmoney
```

`cache-clear` removes filings and archives but keeps the seen list (so a
running `watch` does not re-process a week of filings); `cache-clear
--seen` forgets that too. Nothing else under the root is touched.

## `hardmoney efile watch`: following the feed

The FEC's e-filing system publishes an RSS feed of every electronic
filing received in the last seven days (around a thousand to two
thousand items) within minutes of receipt, at
`https://efilingapps.fec.gov/rss/generate?preDefinedFilingType=ALL`.
Each item names the committee, the form type as filed (`F3XN`, `F3XA`,
`F24N`), the coverage period, and the raw-filing URL.

`watch` polls that feed, records which ids it has handled in
`efile-seen.txt`, and processes only the new ones, oldest first (so an
original is handled before its amendment when both arrive in one poll).
The default is to poll every 300 seconds forever; `--once` polls a single
time and exits, which is what a cron job or a CI check wants.

```text
$ hardmoney efile watch --once --form-type F5 --validate
2011159  F5N   C90022427 2026-09-09T14:30:46Z  PROGRESS NORTH CAROLINA ACTION
  2011159: validate ACCEPTABLE: F5N, 0 error(s), 2 warning(s)
2011160  F5N   C90011172 2026-09-09T14:32:44Z  AFSCME WORKING FAMILIES FUND
  2011160: validate ACCEPTABLE: F5N, 0 error(s), 2 warning(s)
2011407  F5N   C90023565 2026-09-10T19:22:08Z  AMERICA'S CREDIT UNIONS
  2011407: validate ACCEPTABLE: F5N, 0 error(s), 0 warning(s)
2011438  F5N   C90023557 2026-09-10T21:08:48Z  NORTH CAROLINANS AGAINST GUN VIOLENCE ACTION FUND
  2011438: validate ACCEPTABLE: F5N, 0 error(s), 2 warning(s)
2011649  F5N   C90011156 2026-09-12T01:21:21Z  WORKING AMERICA
  2011649: validate ACCEPTABLE: F5N, 0 error(s), 0 warning(s)
2011870  F5N   C90022476 2026-09-14T21:04:12Z  STAND UP FOR OHIO (OHIO ORGANIZING CAMPAIGN)
  2011870: validate ACCEPTABLE: F5N, 0 error(s), 0 warning(s)
2026-09-15T09:36:10Z: 951 item(s) in feed, 951 new, 6 processed, 0 failed

$ hardmoney efile watch --once --form-type F5 --validate
2026-09-15T09:36:10Z: 951 item(s) in feed, 0 new, 0 processed, 0 failed
```

The summary line goes to stderr; the filings to stdout. Things to know:

* Filters: `--form-type F3X,F24` matches a base type and all its
  amendment suffixes (`F3XN`, `F3XA`, `F3XT`); `--form-type F3XA` matches
  only amendments. `--committee C1,C2` restricts by filer. Filtered-out
  items are still recorded as seen, so changing the filter later does not
  replay a week of history.
* The first run sees everything. With an empty seen list, every item in
  the feed is new, up to two thousand filings. If you only want what
  arrives from now on, run `watch --mark-seen` once first.
* Actions are the same as `filings`': `--out DIR`, `--validate`,
  `--reconcile`, `--ingest`. Plus `--exec PROGRAM`, which runs
  `PROGRAM <id> <path-to-cached-fec>` with `HARDMONEY_FILING_ID` and
  `HARDMONEY_FILING_PATH` in its environment, after the other actions.
  This is the hook for anything hardmoney does not do itself. A non-zero
  exit is reported as a failure for that filing.
* `--json` emits one object per new filing (JSON Lines), with the feed's
  fields and an `actions` object:

```text
$ hardmoney efile watch --once --json --form-type F5 --committee C90011156 --validate
{"filing_id":2011649,"form_type":"F5N","committee_id":"C90011156","committee_name":"WORKING AMERICA","published":"2026-09-12T01:21:21Z","url":"https://docquery.fec.gov/dcdev/posted/2011649.fec","report_type":null,"coverage_start":"2026-07-10","coverage_end":"2026-09-10","actions":{"validation":{"form_type":"F5N","acceptable":true,"errors":0,"warnings":0}}}
```

* Failures do not stop a long-running watch. A poll that fails (the
  feed is down) is reported and retried after the interval; a filing that
  fails is reported and the rest continue. `--once` exits 1 if either
  happened, so a scheduler notices.

## `hardmoney efile backfill`: the daily archives

The feed covers seven days. For anything older, the FEC publishes one zip
per day (every e-filing received that day, as `<id>.fec`) at
`https://www.fec.gov/files/bulk-downloads/electronic/YYYYMMDD.zip`, from
2001-02-01 through yesterday. `backfill` walks a date range through them:

```text
$ hardmoney efile backfill --from 20260906 --form-type F3X --validate --reconcile
2026-09-06  2010931  F3XN  794 byte(s)
  2010931: validate ACCEPTABLE: F3XN, 0 error(s), 0 warning(s)
  2010931: reconcile F3X: balances (69 line(s))
2026-09-06: 24 filing(s) in archive, 1 matched
backfill 2026-09-06..2026-09-06: 24 filing(s), 1 matched, 0 failed, 0 day(s) unavailable
```

`--to` extends the range (inclusive; both accept `YYYYMMDD` or
`YYYY-MM-DD`), `--out DIR` writes the matching `.fec` files out, and the
action flags are the same as above. Archives are cached under `efile/`,
so re-running over the same days downloads nothing. A day the FEC has not
published (today, or occasionally a holiday) is reported and skipped, and
the command exits 1 at the end so the gap is not missed:

```text
$ hardmoney efile backfill --from 20260915
2026-09-15: the FEC has no archive for this day (HTTP 404)
backfill 2026-09-15..2026-09-15: 0 filing(s), 0 matched, 0 failed, 1 day(s) unavailable
error: 0 filing(s) could not be processed and 1 day(s) were unavailable
```

The form-type filter reads the cover line's form type straight from the
bytes, so an archive of 2,000 filings is filtered without parsing 2,000
filings.

## Processed vs. raw: which route to use

The two routes look at the same filings from different sides of the
FEC's processing pipeline, and it matters which one you ask.

openFEC's `/filings/` is processed metadata. A filing appears there
after the FEC has loaded it, which is hours for most e-filings and can be
days around a deadline; paper filings appear after they are keyed. In
exchange you get things only the FEC can tell you: `most_recent`, the
full `amendment_chain`, cover-page totals for every filing including
paper ones, and the candidate a committee belongs to. Use it when the
question is "what is the current state of this committee's reporting?"

The RSS feed, the daily zips, and openFEC's `/efile/filings/` are raw.
They show what the e-filing system received, within minutes, with no
judgement attached: a filing the FEC will reject next week is there
today; an amendment is there before anyone has decided it supersedes
anything. There are no paper filings and no totals, only the `.fec`
itself. Use these when the question is "what just came in?" or "give me
every e-filing from these dates", and let hardmoney's own parser,
validator, reconciler, and amendment-chain resolver do the judging.

The library exposes the raw API route too: `OpenFec::efile_filings` with
an `EfileQuery` returns `/efile/filings/` records (`file_number`,
`committee_id`, `form_type`, `receipt_date`, `fec_url`, ...), which is the
same immediacy as the feed with openFEC's filtering and pagination.

## Processing lag

The FEC loads an e-filing in two passes. Pass 1 loads the cover page:
the filing then appears in `/filings/` and `/reports/` with its totals.
Pass 2 loads every itemized transaction: only then do its rows appear in
`/schedules/schedule_a/` and the other schedule endpoints. The FEC's own
`/operations-log/` records when each pass finished
(`summary_data_complete_date`, `transaction_data_complete_date`), keyed
by the report's `sub_id` and its beginning image number rather than its
filing number. `hardmoney lag` joins the three endpoints and answers the
question in the command's name:

```text
$ hardmoney lag 2011831 2006786 2011912 999999999
file_number  form  report  committee  received          in /filings/  summary loaded  transactions loaded
2011831      F3XN  M9      C00140855  2026-09-14 15:50  yes           2026-09-14 +0d  pending (1d so far)
2006786      F3XN  M8      C00140855  2026-08-17 11:43  yes           2026-08-17 +0d  2026-08-31 +14d
2011912      F3XA          C00001313  2026-09-14 19:50  no                            pending (1d so far)
999999999                                               no
4 filing(s): 1 unknown to openFEC, 1 received only, 1 summary loaded, 1 transactions loaded
summary lag median 0 d, p90 0 d; transaction lag median 14 d, p90 14 d, max 14 d; 2 pending (0 past 30 days, 0 past 60 days)
```

`received` is the e-filing system's timestamp from `/efile/filings/`;
`in /filings/` says whether pass 1 is done; the two date columns are the
operations log's completion dates with the lag from receipt in days.
2011912 was received at 7:50 pm Eastern and missed that night's summary
load (openFEC issue #3800 describes the cutoff), so a day later it is
still "received only", and `hardmoney filings --most-recent` still
returns the report it amends. Ids cost three requests per fifty. `--json`
emits one object per filing and a `summary` object.

`--counts` adds what pass 2 has to load: the raw filing (downloaded
cache-first) is parsed and its Schedule A, B, and E lines counted, memo
entries included because the FEC loads those too, against the rows the
schedule endpoints hold for the filing's page range (three more requests
per filing; the count is exact unless the endpoint says otherwise):

```text
$ hardmoney lag 2006786 1986128 --counts
...
  2006786: SchA raw 131 line(s) (131 non-memo); processed 131
  2006786: SchB raw 2 line(s) (2 non-memo); processed 2
  2006786: SchE raw 0 line(s) (0 non-memo); processed 0
  1986128: SchA raw 1056 line(s) (1047 non-memo); processed 1056
  1986128: SchB raw 77 line(s) (77 non-memo); processed 77
  1986128: SchE raw 0 line(s) (0 non-memo); processed 0
```

Sums are not compared: openFEC has no per-filing aggregate, and paging
through a large filing's rows to add them up would cost one request per
hundred rows.

`--committee` measures a whole committee's cycle in a handful of
requests (one `/filings/` page per hundred filings, one operations-log
page per hundred rows) and prints the distribution:

```text
$ hardmoney lag --committee C00010603 --cycle 2026 --form-type F3X
file_number  form  report  committee  received    in /filings/  summary loaded  transactions loaded
2009202      F3X   M8      C00010603  2026-08-20  yes           2026-08-20 +0d  2026-08-26 +6d
2003519      F3X   M6      C00010603  2026-08-01  yes           2026-08-01 +0d  2026-08-19 +18d
...
1923132      F3X   M10     C00010603  2025-10-20  yes           2025-10-20 +0d  2026-01-07 +79d
...
26 filing(s): 0 unknown to openFEC, 0 received only, 0 summary loaded, 26 transactions loaded
summary lag median 0 d, p90 0 d; transaction lag median 5 d, p90 25 d, max 79 d; 0 pending (0 past 30 days, 0 past 60 days)
```

(Receipt times here are dates: `/filings/` records the day, not the
second.)

Measured on 2026-09-15 over the fifty oldest Form 3X/3 reports then in
the RSS feed, all received on 2026-09-08: summary lag 0 days for all
fifty; transactions loaded for 42 of 50 within 6 days (median 1 day,
p90 6 days); 8 still pending after 7 days; none pending past 30 or 60
days. The fifty newest, received that afternoon, were all "received
only", waiting for the nightly summary load. A deadline week looks
different: the 300 F3/F3X reports received around the July 2026
quarterly deadline measured a 13-day median and 26-day p90 for pass 2,
with 2.7% still pending after 60 days (`tmp/agent-fec-deep/REPORT.md`),
and the committee above waited 54 and 79 days for its October and
November 2025 monthlies. The FEC's own guidance is "up to 30 days"
(openFEC issue #5911).

The REST API exposes the same lookup as `GET /filings/{id}/processing`,
asked of openFEC live (the server needs a key in `FEC_API_KEY` or
`~/fec_api_key.txt`; without one the route answers 503 and says so).
The filing need not have been ingested. The body is the
`ProcessingStatus` fields plus `stage` (`received`, `summary_loaded`,
`transactions_loaded`), `summary_lag_days`, `transaction_lag_days`,
`days_pending`, and `as_of`.

The download URL comes from the same place now. `fetch_filing_bytes`
with no URL asks openFEC for the filing's `fec_url` (`/efile/filings/`
first, then `/filings/`; `hardmoney::fec::resolve_fec_url`) and only
falls back to the `docquery.fec.gov` template when there is no key,
no record, or the request fails. The FEC opened an inventory of docquery
for retirement in September 2026 (openFEC issue #6717); when the host
changes, the resolved URL follows it and the template does not.

## In Rust

```rust,no_run
use hardmoney::fec::openfec::{FilingsQuery, OpenFec};
use hardmoney::fec::{Cache, EfileFeed, fetch_filing_bytes};
use hardmoney::{Filing, ParseOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cache = Cache::from_env();

    // Processed: everything current for one committee.
    let api = OpenFec::from_env()?; // FEC_API_KEY or ~/fec_api_key.txt
    let query = FilingsQuery::new()
        .committee_id("C00554709")
        .form_type("F3")
        .most_recent(true);
    for record in api.filings_all(&query) {
        let record = record?;
        let Some(id) = record.filing_id() else { continue }; // paper
        let bytes = fetch_filing_bytes(id, &cache, record.raw_url().as_deref())?;
        let filing = Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT)?;
        println!("{id}: {}", filing.validate().is_acceptable());
    }

    // Raw: whatever the e-filing system got in the last week.
    for item in EfileFeed::new().poll()? {
        if item.matches_form_type("F24") {
            let bytes = fetch_filing_bytes(item.filing_id, &cache, Some(&item.url))?;
            println!("{}: {} bytes", item.filing_id, bytes.len());
        }
    }
    Ok(())
}
```

Every failure is a `hardmoney::fec::FecApiError`: `MissingApiKey`,
`Http { status, url, body_snippet }` (URL redacted), `RateLimited {
retry_after }`, `Transport`, `Json`, `Io`, `InvalidQuery`, `Rss`, `Zip`.
