# Importing the FEC's database dumps, from nothing

Who this is for: a journalist, a RAD analyst, a graduate student, or
anyone else who wants the FEC's own processed tables on their own
computer and has never installed Postgres or spent much time in a
terminal. Every command below was run while writing this, on a Mac with
Postgres 18, and the output shown is what came back. Where a line would
differ on your machine (a user name, a path, a date, a row count), it is
pointed out.

What you'll have at the end: the FEC's committee table and its
independent-expenditure table loaded into a Postgres database on your
computer, a first query against each, and the two commands that keep
them current.

## What the FEC publishes, in one paragraph

Alongside its CSV bulk files, the FEC publishes four tables from its own
Postgres database as `pg_dump` files: a "dump" is a file that Postgres
can turn back into a table, rows and all. Two are small (about 14 MB and
43 MB) and load in seconds; two are enormous (90 GB and 39 GB, growing
weekly) and are best loaded one election cycle at a time. The FEC's
own instructions for them are a plain-text README that assumes you
already know Postgres. `hardmoney dumps` is the same job with the
knowledge built in: it checks your computer, tells you what a step will
download and how long it should take, asks before it starts, and tells
you what to type next.

| name you type | what it is | size (September 2026) |
|---|---|---|
| `committees` | one row per committee per two-year election cycle: name, type, party, treasurer, as fec.gov shows it | 14 MB |
| `independent-expenditures` | every independent expenditure the FEC has processed since 1975 (Schedule E: outside groups spending for or against candidates) | 43 MB |
| `receipts` | every itemized receipt since 1975 (Schedule A: who gave money to which committee) | 90 GB |
| `disbursements` | every itemized disbursement since 1975 (Schedule B: what committees spent it on) | 39 GB |

This tutorial imports the two small ones. The same commands, with
`receipts` or `disbursements`, do the large ones; the differences are
covered at the end.

## Step 1: install Postgres and the hardmoney command

Postgres is the database program the data goes into. Installing it also
gives you `pg_restore`, the Postgres tool that reads a dump file, and
`psql`, the tool you type SQL into. hardmoney runs `pg_restore` for you.

macOS, with [Homebrew](https://brew.sh):

```bash
brew install postgresql@18
brew services start postgresql@18
```

Ubuntu or Debian:

```bash
sudo apt install postgresql postgresql-contrib
sudo -u postgres createuser --superuser "$USER"
```

(The second Ubuntu line gives your own login a Postgres account, which
Homebrew does on its own. `postgresql-contrib` carries the `pg_trgm`
extension that makes name searches fast; the import works without it.)

Then install hardmoney as in [Installation](./installation.md):

```bash
cargo install hardmoney
```

Check that both are on your `PATH`:

```text
$ pg_restore --version
pg_restore (PostgreSQL) 18.3
$ hardmoney --version
hardmoney 2.1.0
```

If `pg_restore` is version 14 or older, the FEC's files will not open
(they were written by Postgres 15); install a newer client.

## Step 2: see where you stand

`hardmoney dumps` with nothing after it is the "where am I" screen. Run
it before anything is set up and it says so:

```text
$ hardmoney dumps
The FEC publishes four tables from its own database as Postgres "dump" files,
refreshed every weekend. hardmoney downloads a file and loads it into your
Postgres database, where it becomes an ordinary table you can query with SQL.

  name                      what it holds                                    on fec.gov  downloaded  in database
  committees                one row per committee per cycle                  14.1 MB     no          ?
  independent-expenditures  independent expenditures since 1975, Schedule E  43.4 MB     no          ?
  receipts                  itemized receipts since 1975, Schedule A         90.1 GB     no          ?
  disbursements             itemized disbursements since 1975, Schedule B    39.3 GB     no          ?

Downloads are kept in /Users/chris.gorski/.cache/hardmoney/dumps.

No database is configured yet, so the last column is unknown. To set one up:
  if Postgres is installed on this computer, create a database and tell hardmoney where it is:
    createdb fec
    export DATABASE_URL=postgres://chris.gorski@localhost/fec
  (put the export line in ~/.zshrc or ~/.bashrc, or in a .env file in this folder, so it is remembered)

Next:
  hardmoney dumps check          check that this computer is ready
```

The sizes in the "on fec.gov" column are live: hardmoney asked fec.gov
for them just now. (Offline, it shows approximate sizes marked `~`.)
The suggested `export` line uses your own login name; yours will not
say `chris.gorski`.

## Step 3: create a database and tell hardmoney about it

Do what the screen said:

```bash
createdb fec
export DATABASE_URL=postgres://chris.gorski@localhost/fec
```

`createdb fec` makes an empty database named `fec` on your computer.
The second line is how every hardmoney command finds it: a URL of the
form `postgres://USER@HOST/DBNAME`. It lasts until you close the
terminal, so put it in your shell's start-up file (`~/.zshrc` on a Mac,
`~/.bashrc` on Ubuntu) or in a file named `.env` in the folder you work
from; hardmoney reads `.env` on its own.

## Step 4: check

```text
$ hardmoney dumps check
Checking whether this computer can import all-small (needs about 518.6 MB of disk: 57.6 MB to download, about 461.0 MB inside Postgres) ...

✓ pg_restore: version 18.3 is installed (the FEC's files need 15 or newer)
✓ database: fec on localhost:5432 as user chris.gorski
✓ connection: connected to fec on localhost
✓ Postgres version: server is Postgres 18.3
✓ disclosure schema: does not exist yet; it will be created on the first import (the FEC's files always load into this schema)
✓ extensions: pg_trgm is available, btree_gin is available (installed on the first import if needed; they speed up name searches)
✓ disk space for downloads: 89.6 GB free at /Users/chris.gorski/.cache/hardmoney/dumps; this import needs about 57.6 MB
✓ disk space for the database: 89.6 GB free at /opt/homebrew/var/postgresql@18; this import needs about 461.0 MB
! namespace: 'public' has not been set up for hardmoney yet (a namespace is the part of the database hardmoney's own tables and views live in)
  fix: run: hardmoney schema-init --schema public
       (or let `hardmoney dumps import` do it; it asks first)

Set up namespace 'public' for hardmoney now? (runs `hardmoney schema-init`; it only adds hardmoney's own tables) [Y/n] y
✓ namespace: 'public' is now set up.
Everything looks ready.
Next: hardmoney dumps import all-small
```

Each line is one thing the import needs. `✓` is fine, `!` is worth
knowing, `✗` has to be fixed first, and every `!` or `✗` comes with a
`fix:` line saying what to type. The one `!` here is normal on a new
database: a namespace is the part of the database where hardmoney keeps
its own tables (the friendly views, and a record of what was imported
when). Answering `y` creates them; it touches nothing else. The default
namespace is Postgres's `public`, and you never need another one unless
you want to keep two sets of hardmoney tables apart.

Two checks deserve a word. "disclosure schema": the FEC's files insist
on landing in a schema named `disclosure` (a schema is a named folder
of tables inside a database), so hardmoney creates it. "disk space for
the database": hardmoney asked Postgres where it keeps its files and
measured the free space there; on a server you do not administer that
question may be refused, in which case the line becomes a `!` telling
you how much to check for yourself.

If `✗ database` or `✗ connection` appears instead, the `fix:` line
covers the usual causes: Postgres not running (`brew services start
postgresql@18` or `sudo systemctl start postgresql`), the database not
created yet (`createdb fec`), or a login name Postgres does not know.

## Step 5: import the two small files

```text
$ hardmoney dumps import all-small
Checking this computer and the database ...

✓ pg_restore: version 18.3 is installed (the FEC's files need 15 or newer)
✓ database: fec on localhost:5432 as user chris.gorski
✓ connection: connected to fec on localhost
✓ Postgres version: server is Postgres 18.3
✓ disclosure schema: does not exist yet; it will be created on the first import (the FEC's files always load into this schema)
✓ extensions: pg_trgm is installed, btree_gin is available (installed on the first import if needed; they speed up name searches)
✓ disk space for downloads: 89.6 GB free at /Users/chris.gorski/.cache/hardmoney/dumps; this import needs about 57.6 MB
✓ disk space for the database: 89.6 GB free at /opt/homebrew/var/postgresql@18; this import needs about 461.0 MB
✓ namespace: 'public' is set up for hardmoney (schema version 3)

Plan
  1. committees: download 14.1 MB from fec.gov (under a minute on a typical connection), then load about 270,000 rows into table disclosure.ofec_committee_history in database fec (about 1 to 2 minutes).
  2. independent-expenditures: download 43.4 MB from fec.gov (under a minute on a typical connection), then load about 550,000 rows into table disclosure.fec_fitem_sched_e in database fec (about 1 to 2 minutes).

Disk: about 518.6 MB in total (57.6 MB of downloads in /Users/chris.gorski/.cache/hardmoney/dumps, about 461.0 MB inside Postgres).
Time: about 1 to 4 minutes in total. These are estimates scaled from the FEC's own published timings;
your connection and hardware decide the real figure.

Continue? [y/N] y
committees: downloading 14.1 MB from fec.gov ...
 [00:00:01] [#####################################>--] 12.84 MiB/13.53 MiB (4.31 MiB/s, ETA 0s)
committees: downloaded to /Users/chris.gorski/.cache/hardmoney/dumps/committee_history.dump (14.1 MB)
committees: loading into disclosure.ofec_committee_history with pg_restore (this is the slow part; a line appears every 30 seconds while it runs) ...
committees: 262,276 rows loaded

independent-expenditures: downloading 43.4 MB from fec.gov ...
 [00:00:03] [#######################################>] 40.83 MiB/41.43 MiB (9.39 MiB/s, ETA 0s)
independent-expenditures: downloaded to /Users/chris.gorski/.cache/hardmoney/dumps/schedule_e.dump (43.4 MB)
independent-expenditures: loading into disclosure.fec_fitem_sched_e with pg_restore (this is the slow part; a line appears every 30 seconds while it runs) ...
independent-expenditures: 549,525 rows loaded

Done in 13 s.
  committees: 262,276 rows in disclosure.ofec_committee_history (5 s)
  independent-expenditures: 549,525 rows in disclosure.fec_fitem_sched_e (7 s)

Try it:
  psql "$DATABASE_URL" -c "SELECT count(*) FROM dump_committee_history;"
  psql "$DATABASE_URL" -c "SELECT cycle, committee_id, name, committee_type, party FROM dump_committee_history WHERE name ILIKE '%actblue%' ORDER BY cycle DESC LIMIT 5;"
  hardmoney query ies --candidate S6OH00163 --limit 5
  psql "$DATABASE_URL" -c "SELECT count(*) FROM dump_schedule_e;"
  psql "$DATABASE_URL" -c "SELECT expenditure_date::date, expenditure_amt, support_oppose_code, candidate_name, cmte_id FROM independent_expenditures ORDER BY expenditure_amt DESC NULLS LAST LIMIT 5;"

Where the data lives:
  table disclosure.ofec_committee_history in database fec on localhost; query it through the view dump_committee_history in namespace 'public'
  downloaded file: /Users/chris.gorski/.cache/hardmoney/dumps/committee_history.dump (14.1 MB); keep it so `hardmoney dumps update` can tell when fec.gov has a newer one, or delete it to free the space
  table disclosure.fec_fitem_sched_e in database fec on localhost; query it through the view dump_schedule_e (or independent_expenditures) in namespace 'public'
  downloaded file: /Users/chris.gorski/.cache/hardmoney/dumps/schedule_e.dump (43.4 MB); keep it so `hardmoney dumps update` can tell when fec.gov has a newer one, or delete it to free the space

The FEC refreshes these files every weekend; `hardmoney dumps update` re-imports what you have when a newer file appears.
```

(The progress bar redraws in place while a download runs and clears
itself when the download finishes; the two bar lines above are its last
redraw before each clear.) The plan comes before anything
happens, and nothing happens until you answer `y`. The estimates come
from the FEC's own published timings scaled to today's file sizes; this
laptop beat them comfortably, which is the usual outcome for the small
files. The "Try it" section is written for the database and namespace
you actually used, so it is safe to copy and paste.

If you would rather not be asked, `--yes` skips the question. If you
want to see what a step will do without doing it, `--explain` prints
the plan plus the exact `pg_restore` command and SQL statements, then
exits.

## Step 6: a first look at the data

The lines under "Try it" are real commands. `psql` is the Postgres
terminal; `-c` runs one statement and prints the result.

```text
$ psql "$DATABASE_URL" -c "SELECT count(*) FROM dump_committee_history;"
 count
--------
 262276
(1 row)
```

`dump_committee_history` is a view (a saved query that behaves like a
table) hardmoney created over the FEC's `disclosure.ofec_committee_history`
table, keeping the FEC's column names. One row per committee per
election cycle, so a long-lived committee appears many times:

```text
$ psql "$DATABASE_URL" -c "SELECT cycle, committee_id, name, committee_type, party FROM dump_committee_history WHERE name ILIKE '%actblue%' ORDER BY cycle DESC LIMIT 5;"
 cycle | committee_id |  name   | committee_type | party
-------+--------------+---------+----------------+-------
  2022 | C00401224    | ACTBLUE | V              |
  2020 | C00401224    | ACTBLUE | V              |
  2018 | C00401224    | ACTBLUE | W              |
  2016 | C00401224    | ACTBLUE | W              |
  2014 | C00401224    | ACTBLUE | W              |
(5 rows)
```

Something worth knowing about this particular file: in the September
2026 download, `cycle` runs from 1976 to 2022 only. The FEC's committee
history dump lags its live site by a couple of cycles, so do not expect
2026 rows in it. (`SELECT max(cycle) FROM dump_committee_history;`
tells you what your copy has.)

Independent expenditures come with a hardmoney command as well as a
view. `hardmoney query ies` asks by candidate id; `S6OH00163` is Sherrod
Brown, running for U.S. Senate in Ohio in 2026:

```text
$ hardmoney query ies --candidate S6OH00163 --limit 5
expenditure_date     expenditure_amt  support_oppose_code  candidate_name  committee_name  payee_name
-------------------  ---------------  -------------------  --------------  --------------  -------------------------------
2026-07-25T00:00:00  3679.69          S                    BROWN, SHERROD                  SWITCHBOARD PUBLIC BENEFIT CORP
2026-07-24T00:00:00  3679.69          S                    BROWN, SHERROD                  SWITCHBOARD PUBLIC BENEFIT CORP
2026-07-15T00:00:00  340.11           S                    BROWN, SHERROD                  NEW BLUE INTERACTIVE
2026-07-13T00:00:00  2500.00          O                    BROWN, SHERROD                  TARGETED VICTORY LLC
2026-07-13T00:00:00  250000.00        O                    BROWN, SHERROD                  IN PURSUIT OF LLC
(showing 5 rows; use --limit / --offset 5 for more, or --json)
```

`S` means the spending supported the candidate, `O` that it opposed
them. The same data through SQL, with the five largest independent
expenditures the FEC has ever processed:

```text
$ psql "$DATABASE_URL" -c "SELECT expenditure_date::date, expenditure_amt, support_oppose_code, candidate_name, cmte_id FROM independent_expenditures ORDER BY expenditure_amt DESC NULLS LAST LIMIT 5;"
 expenditure_date | expenditure_amt | support_oppose_code |   candidate_name   |  cmte_id
------------------+-----------------+---------------------+--------------------+-----------
 2020-10-20       |     32857099.95 | S                   | BIDEN, JOSEPH R JR | C00669259
 2020-10-13       |     31456501.02 | S                   | BIDEN, JOSEPH R JR | C00669259
 2024-10-25       |     30033271.32 | O                   | HARRIS, KAMALA     | C00825851
 2020-10-06       |     25456912.00 | O                   | TRUMP, DONALD J.   | C00669259
 2024-10-18       |     24083279.00 | O                   | HARRIS, KAMALA     | C00825851
(5 rows)
```

`independent_expenditures` is the view with hardmoney's column names
(`payee_name`, `expenditure_amt`, `candidate_name`); `dump_schedule_e`
is the same table with the FEC's own names (`pye_nm`, `exp_amt`,
`s_o_cand_nm`) plus a `cycle` column. Use whichever reads better to
you. The two imported tables join on committee id and cycle, which is
how fec.gov itself does it:

```text
$ psql "$DATABASE_URL" -c "SELECT h.name, count(*) AS expenditures, sum(e.exp_amt) AS total FROM dump_schedule_e e JOIN dump_committee_history h ON h.committee_id = e.cmte_id AND h.cycle = e.cycle WHERE e.cycle = 2022 GROUP BY h.name ORDER BY total DESC LIMIT 5;"
             name              | expenditures |    total
-------------------------------+--------------+--------------
 SENATE LEADERSHIP FUND        |          553 | 246008451.30
 CONGRESSIONAL LEADERSHIP FUND |         2421 | 227517708.02
 SMP                           |          349 | 159667170.16
 HOUSE MAJORITY PAC            |         1140 | 145326407.84
 DCCC                          |          963 |  96432377.95
(5 rows)
```

## Step 7: status, and keeping it current

```text
$ hardmoney dumps status
Database: fec on localhost (namespace 'public')

committees (one row per committee per cycle)
  table:        disclosure.ofec_committee_history (view dump_committee_history)
  in database:  yes, about 262,276 rows, 9 index(es); imported 2026-09-15 14:06 UTC from the fec.gov file dated 2026-09-13
  downloaded:   yes, 14.1 MB at /Users/chris.gorski/.cache/hardmoney/dumps/committee_history.dump (fec.gov file dated 2026-09-13)
  on fec.gov:   14.1 MB, dated 2026-09-13; you have the current file

independent-expenditures (independent expenditures since 1975, Schedule E)
  table:        disclosure.fec_fitem_sched_e (view dump_schedule_e)
  in database:  yes, about 549,525 rows, 1 index(es); imported 2026-09-15 14:06 UTC from the fec.gov file dated 2026-09-13
  downloaded:   yes, 43.4 MB at /Users/chris.gorski/.cache/hardmoney/dumps/schedule_e.dump (fec.gov file dated 2026-09-13)
  on fec.gov:   43.4 MB, dated 2026-09-13; you have the current file

receipts (itemized receipts since 1975, Schedule A)
  table:        disclosure.fec_fitem_sched_a (view dump_schedule_a)
  in database:  no
  downloaded:   no
  on fec.gov:   90.1 GB, dated 2026-09-13

disbursements (itemized disbursements since 1975, Schedule B)
  table:        disclosure.fec_fitem_sched_b (view dump_schedule_b)
  in database:  no
  downloaded:   no
  on fec.gov:   39.3 GB, dated 2026-09-13

Downloads are kept in /Users/chris.gorski/.cache/hardmoney/dumps.
```

An index is a lookup structure that makes searching a table by a
column fast; the committee table came with nine of the FEC's, the
Schedule E table with only its primary key. Row counts under "in
database" say "about" because they come from Postgres's own running
estimate, which is cheap to read and slightly behind the exact count.

The FEC replaces these files every weekend. `hardmoney dumps update`
compares the file you imported from with the one on fec.gov and
re-imports only when they differ:

```text
$ hardmoney dumps update
committees: up to date (fec.gov file dated 2026-09-13)
independent-expenditures: up to date (fec.gov file dated 2026-09-13)

Nothing to do. To check every Monday morning (the FEC refreshes on weekends), add this line with `crontab -e`:
  0 6 * * 1  DATABASE_URL=postgres://chris.gorski@localhost/fec hardmoney dumps update --yes >> $HOME/hardmoney-dumps.log 2>&1
```

When there is something to do, it shows the same kind of plan as
`import` and asks before starting; `--yes` answers for you, which is
what the cron line relies on.

To take a table out again, `hardmoney dumps remove committees` drops
the table and deletes the downloaded file, after asking.

## The large files: receipts and disbursements

`receipts` (Schedule A, 90 GB) and `disbursements` (Schedule B, 39 GB)
work the same way with two differences, both of which the plan spells
out. First, the FEC splits each of these tables into one partition per
two-year election cycle (a partition is a separate table holding one
slice of the data, here `fec_fitem_sched_a_2025_2026` for the 2026
cycle), and `hardmoney dumps import receipts` loads only the current
cycle's partition unless you say `--cycles 2022,2024`. Second, the FEC
does not offer per-cycle files, so the whole 90 GB still has to be
downloaded once; hardmoney keeps it, resumes it if the connection
drops, and reuses it for every cycle you add later.

`--explain` is the way to see the cost before committing. This laptop
did not have room, and the check said so:

```text
$ hardmoney dumps import receipts --explain
Checking this computer and the database ...

✓ pg_restore: version 18.3 is installed (the FEC's files need 15 or newer)
✓ database: fec on localhost:5432 as user chris.gorski
✓ connection: connected to fec on localhost
✓ Postgres version: server is Postgres 18.3
✓ disclosure schema: exists, and you can create tables in it (the FEC's files always load into this schema)
✓ extensions: pg_trgm is installed, btree_gin is installed (installed on the first import if needed; they speed up name searches)
✗ disk space for downloads: only 89.0 GB free at /Users/chris.gorski/.cache/hardmoney/dumps, but this import needs about 90.1 GB
  fix: free some space, point --cache-dir (or HARDMONEY_CACHE_DIR) at a bigger disk, or choose a smaller import (the small files first)
✗ disk space for the database: only 89.0 GB free at /opt/homebrew/var/postgresql@18, but this import needs about 135.2 GB
  fix: free some space on the disk that holds the Postgres data directory, or choose a smaller import (fewer --cycles, or the small files first)
✓ namespace: 'public' is set up for hardmoney (schema version 3)

Plan
  1. receipts (cycle 2026): download the whole 90.1 GB file from fec.gov (the FEC does not offer per-cycle files; about 1 to 13 hours depending on your connection, and an interrupted download resumes), then load only the 2025-2026 rows (tens of millions of rows for a recent cycle) into disclosure.fec_fitem_sched_a_2025_2026 in database fec, without the FEC's own indexes (about 2 to 8 hours), then add hardmoney's 3 indexes per table.

Disk: about 225.4 GB in total (90.1 GB of downloads in /Users/chris.gorski/.cache/hardmoney/dumps, about 135.2 GB inside Postgres).
Time: about 3 to 21 hours in total. These are estimates scaled from the FEC's own published timings;
your connection and hardware decide the real figure.

What will run, exactly:
  receipts:
    CREATE SCHEMA IF NOT EXISTS disclosure;
    DROP TABLE IF EXISTS disclosure.fec_fitem_sched_a_2025_2026 CASCADE;
    pg_restore --no-owner --no-acl --table=fec_fitem_sched_a --table=fec_fitem_sched_a_2025_2026 -d postgres://chris.gorski@localhost/fec /Users/chris.gorski/.cache/hardmoney/dumps/schedule_a_full.dump
    CREATE UNIQUE INDEX hm_fec_fitem_sched_a_2025_2026_sub_id_uidx ON disclosure.fec_fitem_sched_a_2025_2026 (sub_id);
    CREATE INDEX hm_fec_fitem_sched_a_2025_2026_cmte_dt_idx ON disclosure.fec_fitem_sched_a_2025_2026 (cmte_id, contb_receipt_dt);
    CREATE INDEX hm_fec_fitem_sched_a_2025_2026_contbr_nm_trgm_idx ON disclosure.fec_fitem_sched_a_2025_2026 USING gin (contbr_nm gin_trgm_ops);  -- skipped if pg_trgm is unavailable
    (then hardmoney recreates the view dump_schedule_a in namespace 'public' and records the import in its loads table)

Nothing was changed (--explain).
error: 2 check(s) failed, so nothing was changed.
  what to do: fix the items marked ✗ above, then run this command again
```

Three things to take from that. The `pg_restore` line is the whole
trick: the FEC's README does the same thing by hand, listing the parent
table and the partitions wanted. The FEC's own indexes are skipped on
purpose (there are 34 per partition, tuned for fec.gov's search page,
and they are most of the 35 hours the README quotes for a full restore);
hardmoney adds three per partition afterwards, enough for lookups by
committee, date, and contributor name. And the disk figures are
estimates, but they are the right order of magnitude: an external drive
for `--cache-dir` is the usual answer on a laptop.

When you have the room, `hardmoney dumps import receipts` runs it, and
`hardmoney dumps import receipts --cycles 2024` adds another cycle later
without touching the one you have. While `pg_restore` runs, a line every
30 seconds says how long it has been going, so a multi-hour load is
distinguishable from a hung one.

## Where to go next

- [The FEC's Postgres dump files](./pg-dumps.md) has the reference
  material: what is inside each archive, every column of the Schedule E
  table against the raw `.fec` fields, and the lower-level
  `bulk-restore-dump` commands for scripts.
- [Tracking independent expenditures](./tutorial-independent-expenditures.md)
  goes on from the Schedule E table to the REST API and to today's
  48-hour notices.
- Everything you imported is ordinary Postgres. Any SQL client, notebook,
  or BI tool that speaks Postgres can read `disclosure.fec_fitem_sched_e`
  and the views over it.
