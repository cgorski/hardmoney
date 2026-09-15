# The FEC's Postgres dump files

The FEC publishes four tables from its own database as Postgres dump
files: committee history, independent expenditures, itemized receipts,
and itemized disbursements, refreshed every weekend. hardmoney loads
them into your own Postgres database. This chapter starts with the
three commands that do it and what they say back, then explains the
choices they make, and ends with the reference material: what is inside
each archive, the FEC's processed columns, and the lower-level
`bulk-restore-dump` commands.

## Run these three commands

With Postgres installed and a database created (the
[tutorial](./tutorial-dumps.md) covers that from nothing, on macOS and
Ubuntu):

```bash
export DATABASE_URL=postgres://you@localhost/fec   # your login name, your database
hardmoney dumps check
hardmoney dumps import all-small
```

`check` looks at your computer and the database and prints one line per
thing an import needs, `✓`, `!`, or `✗`, each with a fix. `import
all-small` downloads the two small files (14 MB and 43 MB), shows a
plan with disk and time estimates, asks for a `y`, and loads them.
About a minute later you have `dump_committee_history` (262,276 rows in
September 2026) and `dump_schedule_e` (549,525 rows) to query:

```text
$ psql "$DATABASE_URL" -c "SELECT count(*) FROM dump_schedule_e;"
 count
--------
 549525
(1 row)
```

The words a beginner meets along the way, once each:

- A **dump** is a file Postgres can turn back into a table, rows and
  all. `pg_restore`, which comes with Postgres, does the turning;
  hardmoney runs it for you.
- A **schema** is a named folder of tables inside a database. The FEC's
  files insist on landing in one called `disclosure`; hardmoney creates
  it.
- A **namespace** is hardmoney's word for the schema where its own
  tables and its friendly views live. The default is `public`. The
  `disclosure` tables are shared by every namespace; the views over
  them are made per namespace.
- A **view** is a saved query that behaves like a table.
  `dump_schedule_e` and `independent_expenditures` are views over
  `disclosure.fec_fitem_sched_e`.
- A **partition** is a separate table holding one slice of a bigger
  one. The FEC splits its two large tables into one partition per
  two-year election cycle, which is what makes them loadable a cycle at
  a time.
- An **index** is a lookup structure that makes searching a table by a
  column fast, at the cost of disk and load time.

## The `dumps` commands

| command | what it does |
|---|---|
| `hardmoney dumps` | Where am I: the four files with today's sizes from fec.gov, which are downloaded, which are in the database, and what to run next. |
| `hardmoney dumps check [--for WHAT] [--cycles ...]` | Preflight: `pg_restore` present and version 15 or newer; a database URL; the connection; server version; the `disclosure` schema exists or can be created; `pg_trgm` and `btree_gin` available; free disk where downloads go and where Postgres keeps its data, against what the import needs; the namespace set up (offers to run `schema-init`). |
| `hardmoney dumps import WHAT [--cycles Y,Y] [--yes] [--explain] [--dump-file PATH] [--jobs N]` | The import. Runs the checks, prints the plan, asks, downloads with a progress bar, restores with a heartbeat line every 30 seconds, adds hardmoney's indexes where the FEC's were skipped, then prints rows, time, and three commands to try. |
| `hardmoney dumps status [--offline]` | What is imported (rows, indexes, cycles, when, from which fec.gov file), what is downloaded, and whether fec.gov has a newer file. |
| `hardmoney dumps update [--yes]` | Re-imports whatever is imported when fec.gov has a newer file. Prints a cron line. |
| `hardmoney dumps remove WHAT [--yes] [--keep-file]` | Drops the table and deletes the download, after asking. |

`WHAT` is `committees`, `independent-expenditures`, `receipts`,
`disbursements`, or `all-small` (the first two). The technical names
(`committee_history`, `schedule_e`, `schedule_a_full`,
`schedule_b_full`) are accepted too. Every command takes `--json` for
scripts, and `--database-url`, `--schema`, `--cache-dir` before or
after the subcommand. Full `--help` text is in the
[CLI reference](./cli-reference.md#dumps).

### What `import` decides for you

- **The small files are loaded whole**, indexes and all, because they
  take seconds either way.
- **`receipts` and `disbursements` are loaded one cycle at a time**,
  the current cycle unless `--cycles` says otherwise, and without the
  FEC's own indexes. Loading a partition with `pg_restore --table`
  never restores indexes, and the FEC's 34 per partition are most of
  the 35 hours its README quotes; hardmoney adds three per partition
  afterwards (unique `sub_id`, committee plus date, and a trigram index
  on the name column when `pg_trgm` is available). Run again with
  `--cycles 2024` to add a cycle; the ones you have are left alone.
- **The whole large file is downloaded even for one cycle.** The FEC
  does not offer per-cycle files. The download resumes if it is
  interrupted and is kept for the next cycle you add.
- **Estimates are the FEC's numbers, scaled.** The plan's download
  times assume 2 to 25 MB/s; restore times take the FEC's published
  rates (about 6 minutes per gigabyte of archive data-only, 44 with the
  FEC's indexes) times the share of the archive a recent cycle is, then
  halve and double them for the range. Disk inside Postgres is about
  five times the archive bytes restored. They are for choosing between
  "before lunch" and "over the weekend", and the plan says so.
- **Confirmation** is asked when there is a terminal to ask on. `--yes`
  skips it; so does `--json`. `dumps remove` always insists on one or
  the other.
- **The namespace is set up if it is not.** An import needs `schema-init`
  to have run so the import can be recorded and the views made;
  `import` asks and runs it.
- **Errors say what to do.** Every failure prints a sentence about what
  happened and a "what to do" line before the raw Postgres or
  `pg_restore` message, which is kept underneath for whoever needs it.

### `--explain`: no magic

`hardmoney dumps import receipts --explain` prints the plan and then
the exact statements and `pg_restore` command that would run, and
exits. Captured while writing this (the disk checks failed on the
laptop, and the plan is printed anyway):

```text
What will run, exactly:
  receipts:
    CREATE SCHEMA IF NOT EXISTS disclosure;
    DROP TABLE IF EXISTS disclosure.fec_fitem_sched_a_2025_2026 CASCADE;
    pg_restore --no-owner --no-acl --table=fec_fitem_sched_a --table=fec_fitem_sched_a_2025_2026 -d postgres://chris.gorski@localhost/fec /Users/chris.gorski/.cache/hardmoney/dumps/schedule_a_full.dump
    CREATE UNIQUE INDEX hm_fec_fitem_sched_a_2025_2026_sub_id_uidx ON disclosure.fec_fitem_sched_a_2025_2026 (sub_id);
    CREATE INDEX hm_fec_fitem_sched_a_2025_2026_cmte_dt_idx ON disclosure.fec_fitem_sched_a_2025_2026 (cmte_id, contb_receipt_dt);
    CREATE INDEX hm_fec_fitem_sched_a_2025_2026_contbr_nm_trgm_idx ON disclosure.fec_fitem_sched_a_2025_2026 USING gin (contbr_nm gin_trgm_ops);  -- skipped if pg_trgm is unavailable
    (then hardmoney recreates the view dump_schedule_a in namespace 'public' and records the import in its loads table)
```

That `pg_restore` line is the FEC README's own recipe (name the parent
table and the partitions wanted) with the table names filled in. A
password in the URL is shown as `***`.

### The checks, and what each failure means

| check | passes when | if not |
|---|---|---|
| `pg_restore` | on `PATH`, version 15 or newer (the archives were written by Postgres 15's `pg_dump`) | install the Postgres client tools; the fix line names the package for your OS |
| `database` | `DATABASE_URL` or `--database-url` is a Postgres URL | `createdb fec`, then `export DATABASE_URL=postgres://you@localhost/fec` |
| `connection` | one connection opens | the fix names the cause: Postgres not running, database not created, unknown user, wrong password, wrong host |
| `Postgres version` | server 15 or newer (10 to 14 is a warning) | upgrade the server |
| `disclosure schema` | exists and you can create tables in it, or you can create it | the `GRANT` or `CREATE SCHEMA` for an administrator to run |
| `extensions` | `pg_trgm` and `btree_gin` are available (warning only) | optional: install the contrib package |
| `disk space for downloads` | free space at `--cache-dir` is at least 1.2 times the download | free space, or `--cache-dir` on a bigger disk |
| `disk space for the database` | free space at the server's data directory is at least 1.2 times the estimated table size; a warning if the server will not say where that is (needs superuser) or is on another machine | free space there, or fewer `--cycles` |
| `namespace` | `schema-init` has run (warning only) | `check` and `import` offer to run it |

The same checks are `hardmoney::bulk::preflight::run` in the library,
returning a `Preflight` of typed `Check`s with a `Display`, so a program
can run them too.

---

The rest of this chapter is the reference: what is in the archives,
how the FEC's processed columns relate to the raw `.fec` fields, and
the `bulk-restore-dump` family of commands that `dumps` is built on and
that scripts may prefer.

## What the FEC publishes

The files live under
`https://www.fec.gov/files/bulk-downloads/data-dump/schedules/`, which
redirects to an S3 bucket in `us-gov-west-1`. The FEC's own
[`README.txt`](https://www.fec.gov/files/bulk-downloads/data-dump/schedules/README.txt)
in that directory is the only documentation; the figures below that are
not from hardmoney's own inspection are from it.

| hardmoney name | file | table (schema `disclosure`) | size, 13 Sept 2026 | rows (FEC README, Aug 2020) |
|---|---|---|---|---|
| `schedule_a_full` | `fec_fitem_sched_a.dump` | `fec_fitem_sched_a`: itemized receipts, 1975 to date | 90.18 GB | 362,016,701 |
| `schedule_b_full` | `fec_fitem_sched_b.dump` | `fec_fitem_sched_b`: itemized disbursements | 39.31 GB | 182,544,742 |
| `schedule_e` | `fec_fitem_sched_e.dump` | `fec_fitem_sched_e`: independent expenditures | 43.4 MB | 340,729 (549,525 in the Sept 2026 file) |
| `committee_history` | `ofec_committee_history.dump` | `ofec_committee_history`: one row per committee per cycle, as fec.gov shows it | 14.2 MB | 242,346 |

There are no dumps for Schedules C, D, or F, and no dump of the
`filings` metadata. The README says the files are refreshed weekly on
Saturday; the files hardmoney inspected carried a Sunday `Last-Modified`
(`Sun, 13 Sep 2026 11:03:27 GMT` for Schedule E, 20:53 for Schedule A),
and an FEC staff member describes the job as "our Sunday morning file
generation processes" in
[fecgov/FEC#13168](https://github.com/fecgov/FEC/issues/13168). Plan on
"weekend".

The README's sizing history shows the growth: Schedule A was 33.6 GB in
August 2020, 47.2 GB in February 2021, 90.2 GB in September 2026. The
README's own estimate is that the data doubles every two and a half
years.

S3 serves `Accept-Ranges: bytes`, an `ETag`, and `Last-Modified`, and
answers `Range` requests with `206 Partial Content` through the
redirect. hardmoney's downloader relies on that (below).

### What people have asked the FEC for

The dumps are the only way to get the FEC's *processed* itemized data
in bulk, so they get used despite the format. The open ticket
[fecgov/FEC#13168](https://github.com/fecgov/FEC/issues/13168)
(March 2024, still open, 11 comments) asks the FEC to publish Parquet
alongside the dumps; the FEC replied that it was "open to exploring it"
and that its DBA team was assessing the cost, and the thread has since
stalled on DBA time. The requester now runs a weekly job that unpacks
the Schedule A and B table data straight from the `.dump` files into
Parquet on Hugging Face
([NickCrews/fec-dumps](https://github.com/NickCrews/fec-dumps), all
columns as strings). An older ticket,
[fecgov/FEC#16](https://github.com/fecgov/FEC/issues/16) from 2014,
asked for "Database dump, not just raw csv" in the first place.

## Inside an archive

The archives are `pg_dump --format=custom`, gzip-compressed, and their
table of contents is at the front, so `pg_restore --list` on the 90 GB
file is instant. The complete listing of the September 2026 Schedule E
archive:

```text
;
; Archive created at 2026-09-13 12:01:50 EDT
;     dbname: fec
;     TOC Entries: 8
;     Compression: gzip
;     Dump Version: 1.14-0
;     Format: CUSTOM
;     Integer: 4 bytes
;     Offset: 8 bytes
;     Dumped from database version: 15.10
;     Dumped by pg_dump version: 15.16
;
; Selected TOC Entries:
;
358; 1259 17839 TABLE disclosure fec_fitem_sched_e fec
8792; 0 17839 TABLE DATA disclosure fec_fitem_sched_e fec
8395; 2606 19168 CONSTRAINT disclosure fec_fitem_sched_e fec_fitem_sched_e_pkey fec
8396; 2620 19532 TRIGGER disclosure fec_fitem_sched_e tri_fec_fitem_sched_e fec
```

Four entries: the table, its rows, a primary key on `sub_id`, and a
`BEFORE INSERT` trigger. Things to notice:

- **The schema is hard-coded.** Every DDL statement says
  `disclosure.fec_fitem_sched_e`. The archive contains no `CREATE
  SCHEMA`, so the schema must exist first (hardmoney creates it). A
  restore lands in `disclosure` whatever `--schema` you pass; the dump
  is reference data shared by every hardmoney namespace.
- **The trigger's function is not in the archive.** `tri_fec_fitem_sched_e`
  calls `disclosure.fec_fitem_sched_e_insert()`, which exists only in
  the FEC's database. `pg_restore` therefore always prints one error
  and exits 1 on a complete restore. The FEC's README says to ignore
  it; hardmoney does, and judges success by the tables and rows that
  exist afterwards.
- **Schedule E is one table.** No per-cycle child tables and no
  secondary indexes ("no index", as the README puts it). Schedule A and
  B are different, below.
- **Dumped from Postgres 15**, restorable with any newer `pg_restore`
  (hardmoney's tests use 18). The README asks for 9.6 or later on the
  server.

Restoring the file takes about seven seconds locally. The other small
archive, `ofec_committee_history.dump`, is one table with nine btree
indexes.

### Per-cycle child tables in Schedule A and B

`fec_fitem_sched_a` and `fec_fitem_sched_b` are split by two-year
period using table inheritance, not declarative partitioning: the README
shows `fec_fitem_sched_b_2019_2020` created with
`INHERITS (disclosure.fec_fitem_sched_b)` and a
`CHECK (two_year_transaction_period = ANY (ARRAY[2019, 2020]))`. There
is one child per period from `_1975_1976` onwards, each with its own
primary key, its own `BEFORE INSERT` trigger, and its own set of indexes:
34 per child for Schedule A and 30 for Schedule B, mostly composite
btrees ending in `sub_id` plus GIN indexes on the `tsvector` columns.
Querying the parent reads every child.

The README's own recipe for making the 90 GB file tractable is to name
the parent and the wanted children:

```bash
pg_restore --dbname testdata --no-acl --no-owner \
  --table fec_fitem_sched_a \
  --table fec_fitem_sched_a_2021_2022 fec_fitem_sched_a.dump
```

`pg_restore --table` restores table definitions and data only, never
indexes, constraints, or triggers, which is why the README lists this
under "without index (data only)".

The FEC's own test environment (4 CPUs, 50 GB RAM, 1.75 MB/s network)
gives the scale, for the February 2021 files:

| | with indexes | data only |
|---|---|---|
| disk | 2 TB | 300 GB |
| Schedule A restore | 35 h | 5 h |
| Schedule B restore | 12 h | 1.5 to 2 h |
| Schedule E, committee history | 1 min each | 1 min each |
| download of Schedule A | 150 min to a workstation, 421 min copied on to the server, 200 min with `wget` | |

Those files were roughly half the size of today's.

### Columns: the FEC's processed view of a Schedule E line

`fec_fitem_sched_e` has 80 columns. They are not the raw `.fec` fields;
they are what the FEC's processing pipeline produced from them. The
mapping from the fields hardmoney parses (`data/fec-csv-sources/SchE.csv`)
to the dump's columns, for the ones you will reach for:

| raw `.fec` field (hardmoney) | dump column | notes |
|---|---|---|
| `filer_committee_id_number` | `cmte_id` | plus `cmte_nm`, looked up |
| `transaction_id_number` | `tran_id` | the join key `bulk-dump-compare` uses |
| `back_reference_tran_id_number`, `back_reference_sched_name` | `back_ref_tran_id`, `back_ref_sched_nm` | |
| `entity_type` | `entity_tp`, `entity_tp_desc` | |
| `payee_organization_name` / `payee_last_name`, ... | `pye_nm`, `payee_l_nm`, `payee_f_nm`, `payee_m_nm`, `payee_prefix`, `payee_suffix` | `pye_nm` is the single display name; `payee_name_text` is a `tsvector` of it |
| `payee_street_1` ... `payee_zip_code` | `pye_st1`, `pye_st2`, `pye_city`, `pye_st`, `pye_zip` | |
| `election_code` | `election_tp`, `fec_election_tp_desc` | |
| `dissemination_date`, `disbursement_date` | `dissem_dt`, `exp_dt` | `timestamp`, not `date` |
| `expenditure_amount` | `exp_amt` | `numeric(14,2)` |
| `calendar_y_t_d_per_election_office` | `cal_ytd_ofc_sought` | |
| `expenditure_purpose_descrip` | `exp_desc` | |
| `category_code` | `catg_cd`, `catg_cd_desc` | |
| `support_oppose_code` | `s_o_ind`, `s_o_ind_desc` | |
| `candidate_id_number`, `candidate_name`, ... | `s_o_cand_id`, `s_o_cand_nm`, `s_o_cand_nm_first`, `s_o_cand_nm_last`, `s_o_cand_m_nm`, `s_o_cand_prefix`, `s_o_cand_suffix`, `s_o_cand_office`, `s_o_cand_office_desc`, `s_o_cand_office_st`, `s_o_cand_office_st_desc`, `s_o_cand_office_district` | |
| `completing_last_name` ... | `filer_l_nm`, `filer_f_nm`, `filer_m_nm`, `filer_prefix`, `filer_suffix` | |
| `date_signed`, `ind_name_as_signed` | `indt_sign_dt`, `indt_sign_nm` | |
| `date_notarized`, `ind_name_notary`, `date_notary_commission_expires` | `notary_sign_dt`, `notary_sign_nm`, `notary_commission_exprtn_dt` | |
| `memo_code`, `memo_text_description` | `memo_cd`, `memo_cd_desc`, `memo_text` | |
| `conduit_name` ... `conduit_zip_code` | `conduit_cmte_id`, `conduit_cmte_nm`, `conduit_cmte_st1`, ..., `conduit_cmte_zip` | |
| `form_type` (`SE`) | `schedule_type` (`SE`), `schedule_type_desc`, `line_num` (`24`) | |

Columns with no raw counterpart, added by the FEC's processing:

- `sub_id` (`numeric(19,0)`, the primary key), `link_id`, `orig_sub_id`:
  the FEC's internal transaction identifiers.
- `file_num`: the filing number, which is hardmoney's `filing_id`.
  `filing_form` (`F3X`, `F5`, `F3`, `F3P`), `rpt_tp` (`M7`, `Q2`,
  `30G`, ...), `rpt_yr`, and `election_cycle` (the two-year period;
  in Schedule A and B this column is named `two_year_transaction_period`).
- `image_num` and `pdf_url`
  (`https://docquery.fec.gov/cgi-bin/fecimg/?<image_num>`): the page image.
- `exp_tp` / `exp_tp_desc`: the transaction type code
  (`24E` "INDEPENDENT EXPENDITURE FOR", `24A` against).
- `action_cd` / `action_cd_desc`: `A` ADD, `C` CHANGE, `N` NO CHANGE,
  `T`, from the FEC's amendment processing. In the September 2026 file:
  208,787 `A`, 203,446 `C`, 136,955 `N`.
- `pg_date`: when the row was written to the FEC's Postgres database
  (`DEFAULT now()`). This is the processing timestamp: a filing imaged
  on 8 September 2026 has `pg_date` 11 September. There is no separate
  `load_date`/`update_date` pair in this table.

Two things the Schedule E dump does not contain:

- **Form 24 (24- and 48-hour notice) rows.** `filing_form` is only ever
  `F3X` (543,489 rows), `F5` (5,968), `F3` (50), or `F3P` (18), and
  `rpt_tp` is never `24` or `48`. The FEC's processed Schedule E table
  is built from periodic reports. For notices you need the raw filing
  (`bulk-load-filing`) or the FEC's separate
  `independent_expenditure_<year>.csv`.
- **Anything filed after the archive was taken.** The September 13
  archive's newest `file_num` is 2,011,113; the fixtures in this
  repository, filed on September 14, are 2,011,8xx.

Schedule A adds columns worth knowing about: `clean_contbr_id` (the
contributor id normalised for joins), `is_individual`,
`contributor_name_text` / `contributor_employer_text` /
`contributor_occupation_text` (`tsvector`, what fec.gov's search uses),
`line_number_label`, and the filer's `cmte_tp`, `org_tp`, `cmte_dsgn`
denormalised on to every row. Schedule B has the disbursement
equivalents (`clean_recipient_cmte_id`, `recipient_name_text`,
`disbursement_description_text`, `disbursement_purpose_category`).

The FEC's README shows how fec.gov joins these to committee history:

```sql
SELECT ...
FROM disclosure.fec_fitem_sched_a
LEFT OUTER JOIN disclosure.ofec_committee_history h
  ON fec_fitem_sched_a.cmte_id = h.committee_id
 AND fec_fitem_sched_a.two_year_transaction_period = h.cycle
WHERE fec_fitem_sched_a.two_year_transaction_period IN (2020)
ORDER BY contb_receipt_dt DESC, sub_id DESC
LIMIT 20;
```

Note the join is on `(committee_id, cycle)`: committee history is one
row per committee per cycle, and the receipt's period picks the right
one.

## Restoring with hardmoney

`bulk-restore-dump` needs `pg_restore` on `PATH` and a namespace that
has had `schema-init`. The two small dumps:

```bash
hardmoney bulk-restore-dump schedule_e
hardmoney bulk-restore-dump committee_history
```

```text
restored 549525 rows into disclosure.fec_fitem_sched_e (dump cached at /Users/you/.cache/hardmoney/dumps/schedule_e.dump)
pg_restore reported 3 non-fatal message(s); first: pg_restore: error: could not execute query: ERROR:  function disclosure.fec_fitem_sched_e_insert() does not exist
views over disclosure refreshed in namespace 'public'
```

(Output of the September 2026 file restored while writing this
chapter; the row count changes weekly.) The three messages are the
trigger error, the `Command was:` line under it, and pg_restore's
`errors ignored on restore: 1` summary.

A re-run drops the table first (`CASCADE`, which also removes every
namespace's views over it; they are recreated at the end), so the
second restore is clean rather than a pile of "already exists" errors.

### Choosing cycles: `--cycles`

For the two large dumps, name the two-year periods you want:

```bash
hardmoney bulk-restore-dump schedule_a_full --cycles 2024,2026
```

hardmoney reads the archive's table of contents, checks that
`fec_fitem_sched_a_2023_2024` and `fec_fitem_sched_a_2025_2026` are in
it, drops any earlier copy of those two child tables, and runs
`pg_restore --table fec_fitem_sched_a --table fec_fitem_sched_a_2023_2024
--table fec_fitem_sched_a_2025_2026`. The parent is named only if it is
not already in the database, so running again later with
`--cycles 2022` adds a period without touching the ones you have. A
cycle that is not in the archive is an error before anything is dropped:

```text
error: pg_restore failed: fec_fitem_sched_a.dump has no table for cycle 2030; the archive contains: 1976, 1978, ..., 2026
```

Because `--table` never restores post-data, a cycle-selective restore is
always data only: no primary keys, no triggers, none of the FEC's 34
indexes per child. The report says so and tells you the
`bulk-dump-index` command to run next. `--cycles` on `schedule_e` or
`committee_history` is refused (they are single tables).

`--cycles` also lifts the `--allow-large` requirement, which now guards
only a whole-archive restore of the two large dumps. `--jobs N` passes
`pg_restore --jobs`, which helps when several child tables are being
copied.

### Skipping indexes: `--no-indexes`

For a whole-archive restore, `--no-indexes` runs
`pg_restore --section=pre-data --section=data`, skipping the post-data
section (indexes, primary keys, triggers). Using the FEC's own numbers,
that is the difference between 35 hours and 5 hours for Schedule A, and
between 2 TB and 300 GB of disk. The FEC's indexes are tuned for
fec.gov's search page, with a composite index per filterable column;
hardmoney's own views and API need far fewer, so the intended workflow
is `--no-indexes` (or `--cycles`) followed by:

```bash
hardmoney bulk-dump-index schedule_a_full --cycles 2024,2026
```

```text
indexing disclosure.fec_fitem_sched_a for cycle(s) 2024, 2026 ...
created hm_fec_fitem_sched_a_2023_2024_sub_id_uidx on disclosure.fec_fitem_sched_a_2023_2024 (…s)
created hm_fec_fitem_sched_a_2023_2024_cmte_dt_idx on disclosure.fec_fitem_sched_a_2023_2024 (…s)
created hm_fec_fitem_sched_a_2023_2024_contbr_nm_trgm_idx on disclosure.fec_fitem_sched_a_2023_2024 (…s)
...
2 table(s): 6 created, 0 already present, 0 skipped
```

(Illustrative: the command was run against the Schedule E dump while
writing this, not against Schedule A.) Without `--cycles` it indexes
every child table present. The indexes per dump:

| dump | indexes (`hm_<table>_<suffix>`) |
|---|---|
| `schedule_a_full` | unique `sub_id`; `(cmte_id, contb_receipt_dt)`; trigram on `contbr_nm` |
| `schedule_b_full` | unique `sub_id`; `(cmte_id, disb_dt)`; trigram on `recipient_nm` |
| `schedule_e` | unique `sub_id`; `(cmte_id, exp_dt)`; `s_o_cand_id`; `file_num`; trigram on `pye_nm` |
| `committee_history` | unique `idx`; `(committee_id, cycle)`; trigram on `name` |

The trigram indexes need `pg_trgm` (installed into `public` if it can
be) and are reported as skipped otherwise. The unique index on `sub_id`
is skipped if the archive's primary key is already there (a restore with
indexes). Running the command twice reports the indexes as already
present. On Schedule A, expect the trigram index on a recent child table
to take a long time; it is the one to leave out if you do not search
contributor names.

### From a file you already have: `--dump-file`

The FEC's README assumes you download the archive somewhere and copy it
to the database server. `--dump-file PATH` restores from any
`pg_restore`-readable file without consulting the cache:

```bash
hardmoney bulk-restore-dump schedule_e --dump-file /mnt/dumps/fec_fitem_sched_e.dump
```

hardmoney still checks the table of contents first, and refuses a file
whose `TABLE` entries do not include the expected table ("is it the
schedule_e dump?"). A restore from a file is recorded without a source
URL, ETag, or `Last-Modified`.

### Downloads that resume

The cache is `~/.cache/hardmoney/dumps` (`XDG_CACHE_HOME` honoured;
`--cache-dir` or `HARDMONEY_CACHE_DIR` override). A download writes to
`<name>.dump.partial` and records the server's `ETag`, `Last-Modified`,
and `Content-Length` in `<name>.dump.meta`. If the connection drops,
the partial file is kept and the command exits with:

```text
error: download of https://www.fec.gov/files/bulk-downloads/data-dump/schedules/fec_fitem_sched_a.dump stopped at 31245107200 of 90181919946 bytes; the partial file was kept, re-run to resume
```

Re-running sends `Range: bytes=31245107200-` and
`If-Range: "<the saved ETag>"`. S3 answers `206` and hardmoney appends;
if the FEC has replaced the file since (a new weekly dump), `If-Range`
fails, S3 answers `200` with the whole new file, and hardmoney starts
over rather than splicing two weeks together. The file is renamed from
`.partial` to `.dump` only when its length matches the declared total.
The progress bar shows bytes, rate, and ETA, starting from the resumed
offset.

A cached file that `pg_restore --list` rejects (a damaged download) is
reported with the path to delete; hardmoney does not delete tens of
gigabytes on its own.

### What is where: `bulk-dump-info`

```bash
hardmoney bulk-dump-info
```

```text
dump               table                   remote size  last-modified (UTC)  cached  in database
schedule_e         fec_fitem_sched_e       43.4 MB      2026-09-13 11:03     -       ~549525 rows, 5 index(es)
committee_history  ofec_committee_history  14.1 MB      2026-09-13 11:02     -       -
schedule_a_full    fec_fitem_sched_a       90.1 GB      2026-09-13 20:53     -       -
schedule_b_full    fec_fitem_sched_b       39.3 GB      2026-09-13 15:21     -       -
cache: /Users/you/.cache/hardmoney/dumps
restores recorded in namespace 'public':
restored (UTC)    dump        cycle  mode               rows    source last-modified
2026-09-15 12:58  schedule_e  all    restore-data-only  549525  local file
2026-09-15 12:58  schedule_e  all    restore            549525  local file
```

(Run on 15 September 2026 after the two `--dump-file` restores above,
with an empty cache.) One `HEAD` per dump gives the current size and
date (`--offline` skips them and shows `?`). "cached" shows a complete
file (`43.4 MB`), a partial one awaiting resume (`partial 20.0 MB`),
or a note that the FEC now has a newer file than the cached one (ETag
mismatch). "in database" comes from the catalogs: an estimated row
count (`pg_stat_user_tables.n_live_tup`, summed over child tables), the
index count, and for the split tables the cycles present. Row counts
are estimates because an exact `count(*)` of 360 million rows is not a
status query.

### Restore records

Every restore writes a row to the namespace's `loads` table, the same
table the bulk CSV loader uses (`schema-status` lists it):

| column | value |
|---|---|
| `source` | `dump:schedule_e`, `dump:schedule_a_full`, ... |
| `cycle` | the cycle, one row per child table for `--cycles`; `NULL` for a whole restore |
| `mode` | `restore`, or `restore-data-only` for `--no-indexes` / `--cycles` |
| `row_count` | rows counted after the restore (exact) |
| `source_url`, `source_etag`, `source_last_modified` | from the download; `NULL` for `--dump-file` |
| `hardmoney_version` | |

The record lives in the namespace the command ran in, while the data in
`disclosure` is shared by all of them. `bulk-dump-info` from another
namespace shows the tables but not these rows.

## Views in every namespace

`db::ensure_views` runs after `schema-init`, at `serve` start, and at
the end of every restore. For each dump table that exists it creates a
view in the current namespace, and does nothing for the ones that do
not:

| view | over | columns |
|---|---|---|
| `independent_expenditures` | `fec_fitem_sched_e` | hardmoney's names (`payee_name`, `expenditure_amt`, `candidate_id`, ...); what `/independent-expenditures` reads |
| `dump_schedule_e` | `fec_fitem_sched_e` | the FEC's 80 columns, plus `cycle` = `election_cycle::int` |
| `dump_schedule_a` | `fec_fitem_sched_a` | the FEC's columns, plus `cycle` = `two_year_transaction_period::int` |
| `dump_schedule_b` | `fec_fitem_sched_b` | the FEC's columns, plus `cycle` |
| `dump_committee_history` | `ofec_committee_history` | the FEC's columns (it already has `cycle`) |

The `dump_*` views keep the FEC's names so the README, the API
documentation, and the FEC's sample SQL apply verbatim; `cycle` is
there so the same `WHERE cycle = 2026` works across hardmoney's own
tables and the dumps. Because `fec_fitem_sched_a` is an inheritance
parent, `dump_schedule_a` sees every restored child table, and a
`WHERE cycle = 2024` is answered from the `_2023_2024` child by its
`CHECK` constraint when `constraint_exclusion` is on (the default,
`partition`, covers inheritance children).

The FEC's join, in hardmoney terms:

```sql
SELECT a.contbr_nm, a.contb_receipt_amt, a.contb_receipt_dt, h.name, h.party
FROM dump_schedule_a a
LEFT JOIN dump_committee_history h
  ON h.committee_id = a.cmte_id AND h.cycle = a.two_year_transaction_period
WHERE a.cycle = 2026
ORDER BY a.contb_receipt_dt DESC, a.sub_id DESC
LIMIT 20;
```

## Raw filing against processed rows: `bulk-dump-compare`

hardmoney is in the unusual position of having both the raw `.fec`
lines of a filing (`bulk-load-filing` into `schedule_e_lines`) and the
FEC's processed rows for the same filing (`file_num` in the Schedule E
dump). `bulk-dump-compare` puts them side by side:

```bash
hardmoney bulk-load-filing tests/fixtures/F24N_2011823.fec
hardmoney bulk-dump-compare 2011823
```

```text
filing 2011823 (F24N, C00912865)
                    raw .fec (schedule_e_lines)  FEC dump (fec_fitem_sched_e)
rows                1                            0
total amount        1074900.00                   0.00
matched by tran_id  0                            0
only in raw filing:  SE.4825
only in FEC dump:    (none)
amount mismatches:   (none)
newest file_num in dump: 2011113; this filing is newer than the dump (weekly snapshot lag)
note: the FEC's Schedule E dump holds periodic reports only (F3X, F5, F3, F3P); Form 24 filings never appear in it
```

Rows are matched on the filer's transaction id (`raw ->>
'transaction_id_number'` against `tran_id`); the report lists ids on one
side only and matched ids whose amounts differ. It is SQL over the two
tables and prints in well under a second.

Differences are the point, not a failure. The reasons they occur:

- **Lag.** The dump is a weekend snapshot and the FEC's own processing
  adds days (`pg_date` minus the image date). A filing numbered above
  the dump's newest `file_num` cannot be in it yet.
- **Form 24.** Notices never appear in the dump, so a `F24N` filing
  always compares as raw-only. Compare an `F3X` or `F5` instead.
- **Amendments.** When a committee amends, the FEC's table reflects its
  amendment processing (`action_cd`), not the file you ingested. Compare
  the most recent filing in the chain
  ([Amendments](./amendments.md)).
- **Edits.** The FEC corrects some fields during processing; an amount
  mismatch on a matched id is worth a look at the page image
  (`pdf_url`).

## From Rust

Everything above is `hardmoney::bulk::dump`:

- `read_toc(&path) -> DumpToc`, `DumpToc::partitions("fec_fitem_sched_a")`,
  `partition_table(parent, cycle)`, `partition_cycle(parent, table)`.
- `RestoreOptions::new().cycles([...]).data_only(true).dump_file(Some(path))`
  and `restore_with(&pool, &url, &SCHEDULE_A, &cache_dir, &options) ->
  RestoreReport` (`rows`, `cycles`, `data_only`, `indexes_skipped`,
  `load_ids`, `warnings`). `restore(...)` keeps the 2.x signature for a
  whole-archive restore.
- `download(&source, &cache_dir)` (resumable), `cached(&source, &cache_dir)`,
  `remote_info(&source)` (one `HEAD`).
- `create_indexes(&pool, &SCHEDULE_A, &cycles) -> IndexReport`.
- `table_state(&pool, &source)`, `restore_history(&pool)`.
- `compare_filing(&pool, filing_id) -> CompareReport`.
- For the guided commands: `plan_restore_offline` (the plan without the
  archive, for `--explain`), `pg_restore_command` (the exact argument
  list), `drop_restored`, `evict_cached`, and
  `RemoteDump::is_newer_than` (ETag, then `Last-Modified`).

The checks and estimates are `hardmoney::bulk::preflight`:
`run(&PreflightInput { database_url, namespace, cache_dir, needs })
-> Preflight` (a `Vec<Check { name, status: Pass | Warn | Fail, detail,
fix }>` with `Display`), the individual `check_*` functions,
`parse_pg_version`, `free_disk_space` and `judge_space`, and
`estimate(&source, archive_bytes, download_needed, &cycles, data_only,
today) -> ImportNeeds` (download and database bytes, time ranges).

Errors are `DumpError` (`MissingPartition`, `NotPartitioned`,
`Incomplete`, `LargeDumpNotAllowed`, `TableMissing`,
`FilingNotIngested`, ...) and convert into `BulkError` for callers of
the older API.
