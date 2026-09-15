# Who Is Funding a Candidate?

**Who this is for:** a local reporter who has never worked with FEC data
and has a candidate's name, a laptop, and about fifteen minutes.

**What you'll have at the end:** a searchable local copy of every 2026
candidate and committee, a sample of itemized contributions, and answers
to three questions -- who is this candidate's committee, who has given
them $1,000 or more, and who their biggest donors are -- from a web API
*and* from plain SQL, so you can see there's nothing hidden in between.

The candidate in this walkthrough is Roy Cooper, running for U.S. Senate
in North Carolina in 2026. Substitute your own; every step is the same.

## The four words you need

FEC data has its own vocabulary, and four terms cover nearly everything
in this tutorial:

- A **candidate** is a person running for federal office. The FEC gives
  each one an ID that starts with `H` (House), `S` (Senate), or `P`
  (President), then one digit for the election year they first
  registered for, the state, and a serial number: `S6NC00407` is a Senate
  candidate, first registered for a 2026 (`6`) race, in North Carolina.
- A **committee** is the legal entity that actually raises and spends the
  money. Candidates don't receive contributions -- their committees do.
  Every committee ID starts with `C`: `C00913566`. A candidate normally
  has one **principal campaign committee**, and there are also PACs,
  party committees, and super PACs that aren't tied to any one candidate.
- A **cycle** is a two-year election period, named for its even year:
  2025-2026 is "cycle 2026". The FEC publishes one set of bulk files per
  cycle, and every table hardmoney loads is tagged with the cycle it came
  from.
- **Itemized** contributions are the ones large enough that the committee
  had to report the donor's name, address, employer, and occupation --
  currently anyone whose gifts to a committee add up to more than $200.
  Smaller gifts are reported only as a lump-sum total, so
  "who gave" questions are always about itemized money. The FEC's file of
  itemized individual contributions is called **Schedule A** (after the
  form section they're reported on).

## Step 1: install

Follow [Installation](./installation.md) to build the binary, then put
it on your `PATH` for the rest of this session:

```bash
git clone https://github.com/cgorski/hardmoney.git
cd hardmoney
cargo build --release --all-features
export PATH="$PWD/target/release:$PATH"
hardmoney --version
```

```text
hardmoney 1.0.0
```

You also need a running Postgres and a connection URL with a username in
it. This tutorial uses a local Postgres 18:

```bash
export DATABASE_URL="postgres://chris.gorski@localhost:5432/hardmoney_v1_test"
```

## Step 2: create a namespace and the tables

A **namespace** is a self-contained set of hardmoney tables inside the
database, named by `--schema`. Using one means you can delete this whole
exercise later with a single command. Create it and apply the schema:

```bash
hardmoney schema-init --schema tut_journalist
```

```text
namespace 'tut_journalist': schema at version 2 (2 migration(s) applied now)
```

That's every table the tool uses, created empty. (What the tables are is
covered in [Loading Bulk Data into Postgres](./bulk-etl.md#step-1-apply-the-schema);
you don't need it yet.)

## Step 3: load who the candidates and committees are

Three small files from `fec.gov` tell you who everyone is: every
candidate, every committee, and the links between them (which committee
belongs to which candidate). Each downloads and loads in about a second:

```bash
hardmoney bulk-load --schema tut_journalist candidates --cycle 2026
hardmoney bulk-load --schema tut_journalist committees --cycle 2026
hardmoney bulk-load --schema tut_journalist candidate_committee_links --cycle 2026
```

```text
candidates: loaded 8557 rows for cycle 2026 (replace)
committees: loaded 20674 rows for cycle 2026 (replace)
candidate_committee_links: loaded 8085 rows for cycle 2026 (replace)
```

So: 8,557 registered candidates and 20,674 committees in the 2026 cycle,
as of the FEC's most recent weekly refresh.

## Step 4: load a sample of contributions (without downloading 2 GB)

The itemized-contributions file for a cycle is enormous. On the day this
was written, `indiv26.zip` was **2,186,550,443 bytes** -- 2.19 GB
compressed -- and it will keep growing until the cycle ends. You don't
need all of it to learn the tool. `--limit` reads the first N rows and
then **stops the download**: hardmoney streams the zip file straight from
the FEC's server, and as soon as it has written the 200,000th row it
closes the connection. Nothing else is fetched.

```bash
hardmoney bulk-load --schema tut_journalist schedule_a --cycle 2026 --limit 200000
```

```text
schedule_a: loaded 200000 rows for cycle 2026 (replace)
```

That took twelve seconds. Two honest caveats about what you now have:

- It is the *first* 200,000 rows of the file, not a random sample. In this
  run they covered contributions dated **October 22-24, 2025** across
  2,214 committees. That's a slice, not the whole picture, and any totals
  below are totals *of the slice*.
- hardmoney records it that way. `schema-status` marks the load as a
  sample so nobody mistakes it for the full file later:

```bash
hardmoney schema-status --schema tut_journalist
```

```text
namespace:  tut_journalist
applied:    [1, 2]
pending:    []
loads (latest per source/cycle):
  candidate_committee_links                cycle 2026          8085 rows  replace 2026-09-15 02:40:43Z
  candidates                               cycle 2026          8557 rows  replace 2026-09-15 02:40:40Z
  committees                               cycle 2026         20674 rows  replace 2026-09-15 02:40:41Z
  schedule_a                               cycle 2026        200000 rows  replace 2026-09-15 02:40:55Z (sample, limit 200000)
```

When you're ready for the real thing, drop `--limit` and let it run --
see [the researcher tutorial](./tutorial-researcher.md) for what that
involves.

## Step 5: start the API

```bash
hardmoney serve --schema tut_journalist --bind 127.0.0.1:18091
```

```text
2026-09-15T02:41:47.949070Z  INFO hardmoney::api: hardmoney API listening bind=127.0.0.1:18091 cors="permissive" api_key=false
```

Leave that running and open a second terminal. Everything below is a
`curl` to `http://127.0.0.1:18091`. (The `python3 -m json.tool` on the end
of some commands just pretty-prints the JSON; drop it if you don't have
Python.)

## Step 6: find the candidate

`/candidates?q=` does a case-insensitive substring search on the name.
FEC names are stored **`LAST, FIRST`**, so search on the last name, and
narrow with the state and office (`H`, `S`, or `P`):

```bash
curl -s "http://127.0.0.1:18091/candidates?q=cooper&state=NC" | python3 -m json.tool
```

```json
[
    {
        "cand_id": "S6NC00407",
        "cycle": 2026,
        "cand_name": "COOPER, ROY",
        "cand_pty_affiliation": "DEM",
        "cand_election_yr": "2026",
        "cand_office_st": "NC",
        "cand_office": "S",
        "cand_office_district": "00",
        "cand_ici": "O",
        "cand_status": "C",
        "cand_pcc": "C00913566",
        "cand_city": "RALEIGH",
        "cand_st": "NC",
        "cand_zip": "27603"
    }
]
```

One hit. Reading the fields that matter: `cand_office: "S"` is Senate,
`cand_ici: "O"` means an open seat (as opposed to `I` incumbent or `C`
challenger), `cand_status: "C"` is a statutory candidate (has raised or
spent over the $5,000 threshold), and **`cand_pcc: "C00913566"`** is the
principal campaign committee -- the ID you'll follow the money with.

If you'd searched without the state, you'd have gotten thirteen Coopers
running for something in 2026; `?q=roy%20cooper` would have gotten none,
because the stored name is `COOPER, ROY`. Searching `q=cooper&office=S`
also works.

## Step 7: look up the committee

```bash
curl -s "http://127.0.0.1:18091/committees/C00913566" | python3 -m json.tool
```

```json
[
    {
        "cmte_id": "C00913566",
        "cycle": 2026,
        "cmte_nm": "COOPER FOR NORTH CAROLINA",
        "tres_nm": "FALMLEN, SCOTT",
        "cmte_city": "RALEIGH",
        "cmte_st": "NC",
        "cmte_dsgn": "P",
        "cmte_tp": "S",
        "cmte_pty_affiliation": "DEM",
        "org_tp": null,
        "connected_org_nm": null,
        "cand_id": "S6NC00407"
    }
]
```

`cmte_dsgn: "P"` confirms it's the principal campaign committee,
`cmte_tp: "S"` that it's a Senate committee, and `cand_id` points back at
the candidate. The treasurer's name (`tres_nm`) is the person legally
responsible for the reports -- often the first call for a reporter.

A candidate can have more than one authorized committee (a joint
fundraising committee, say). `/committees?q=cooper%20for%20north` searches
committees by name, and the `candidate_committee_links` table you loaded
lists every committee linked to a candidate ID -- there's a SQL example
for it at the end.

## Step 8: who gave $1,000 or more?

`/schedule-a` is the itemized-contributions route. Filter by the
committee ID and a minimum amount; results come back newest first:

```bash
curl -s "http://127.0.0.1:18091/schedule-a?cmte_id=C00913566&min_amount=1000&limit=2" | python3 -m json.tool
```

```json
[
    {
        "sub_id": 4033120261434754289,
        "cycle": 2026,
        "cmte_id": "C00913566",
        "name": "FOWLER, AMY GOLDMAN",
        "city": "RHINEBECK",
        "state": "NY",
        "zip_code": "125722820",
        "employer": "SOLIL MANAGEMENT LLC",
        "occupation": "AUTHOR",
        "transaction_dt": "10232025",
        "transaction_date": "2025-10-23",
        "transaction_amt": "3500",
        "transaction_tp": "15",
        "entity_tp": "IND",
        "memo_cd": null,
        "memo_text": null,
        "file_num": 1956696
    },
    {
        "sub_id": 4033120261434753222,
        "cycle": 2026,
        "cmte_id": "C00913566",
        "name": "OPPENHEIMER, GREGG",
        "city": "SANTA MONICA",
        "state": "CA",
        "zip_code": "904022802",
        "employer": "SELF-EMPLOYED",
        "occupation": "WRITER",
        "transaction_dt": "10232025",
        "transaction_date": "2025-10-23",
        "transaction_amt": "1000",
        "transaction_tp": "15E",
        "entity_tp": "IND",
        "memo_cd": null,
        "memo_text": "* EARMARKED CONTRIBUTION: SEE BELOW",
        "file_num": 1956696
    }
]
```

(`limit=2` to keep the page short; with `limit=500` the same query
returned all 29 contributions of $1,000 or more in the sample, every one
from an individual.) Things worth knowing when you read these:

- **`transaction_amt` is a string** (`"3500"`), not a JSON number. That's
  deliberate: it's an exact decimal, and a string is the only JSON form
  that guarantees a client won't quietly round it. The FEC's
  contributions file ships whole dollars, so you'll see `"3500"` rather
  than `"3500.00"`.
- **Two date fields.** `transaction_dt` is the FEC's raw text
  (`10232025`, month-day-year with no separators); `transaction_date` is
  the same date parsed, and it's what the route sorts and filters on. See
  [Dates](./dates.md).
- **`transaction_tp`** is the FEC's transaction type: `15` is an ordinary
  contribution from an individual, `15E` an *earmarked* contribution that
  came through an intermediary such as ActBlue or WinRed. `memo_cd` would
  be `"X"` on a memo line -- an entry that itemizes money already counted
  elsewhere and shouldn't be added to totals. There are none in this
  committee's slice.
- **$3,500** is the per-election individual contribution limit for
  2025-26, which is why it recurs. A donor who gave $3,500 for the primary
  and $3,500 for the general shows up twice, at $7,000 total.
- **`file_num`** is the FEC filing this line came from -- the same filing
  ID you'd pass to `hardmoney parse` or look up on `fec.gov`.

The filters available are `cmte_id`, `cycle`, `name`, `employer`,
`occupation`, `state`, `zip_code`, `min_amount`, `max_amount`, `min_date`,
`max_date`, plus `limit`/`offset` -- the full list is in
[The REST API](./rest-api.md#all-routes). `name`, `employer`, and
`occupation` are substring matches, so
`/schedule-a?cmte_id=C00913566&employer=university` works.

## Step 9: the biggest donors -- in plain SQL

The API answers "show me rows"; "who gave the most in total" is a
`GROUP BY`, and for that you go straight to the database. The tables the
API reads are ordinary Postgres tables in the `tut_journalist` schema,
so `psql` sees exactly what `curl` did. Here is the API query from Step 8
as SQL, to show there's no magic:

```bash
psql "$DATABASE_URL" -c "SET search_path TO tut_journalist, public;" \
  -c "SELECT name, city, state, employer, transaction_date, transaction_amt
      FROM schedule_a
      WHERE cmte_id = 'C00913566' AND transaction_amt >= 1000
      ORDER BY transaction_date DESC NULLS LAST, sub_id DESC
      LIMIT 5;"
```

```text
         name          |     city      | state |            employer            | transaction_date | transaction_amt
-----------------------+---------------+-------+--------------------------------+------------------+-----------------
 FOWLER, AMY GOLDMAN   | RHINEBECK     | NY    | SOLIL MANAGEMENT LLC           | 2025-10-23       |            3500
 OPPENHEIMER, GREGG    | SANTA MONICA  | CA    | SELF-EMPLOYED                  | 2025-10-23       |            1000
 ALTMAN, STUART H.     | CHAPEL HILL   | NC    | BRANDEIS UNIVERSITY            | 2025-10-23       |            1000
 STEPHENSON, VICKIE F. | SMITHFIELD    | NC    | STEPHENSON GENERAL CONTRACTORS | 2025-10-23       |            1500
 LEVENTHAL, ALAN       | CHESTNUT HILL | MA    | BEACON CAPITAL PARTNERS        | 2025-10-23       |            3500
(5 rows)
```

Same rows, same order. (The `SET search_path` line is how you tell
`psql` which namespace to look in; it's what `--schema` does for the
`hardmoney` commands.) Now the aggregate:

```bash
psql "$DATABASE_URL" -c "SET search_path TO tut_journalist, public;" \
  -c "SELECT name, state, sum(transaction_amt) AS total, count(*) AS gifts
      FROM schedule_a
      WHERE cmte_id = 'C00913566' AND memo_cd IS NULL
      GROUP BY name, state
      ORDER BY total DESC
      LIMIT 10;"
```

```text
         name          | state | total | gifts
-----------------------+-------+-------+-------
 MICHAELS, LAURIE      | TX    |  7000 |     2
 HORING, JEFFREY       | NY    |  7000 |     2
 FOWLER, AMY GOLDMAN   | NY    |  7000 |     2
 STEPHENSON, VICKIE F. | NC    |  5000 |     2
 CHAVKIN, ARNOLD L     | NY    |  3500 |     1
 LEVENTHAL, ALAN       | MA    |  3500 |     1
 NEWHOUSE, BEN         | NJ    |  3500 |     1
 O'NEILL, SARAH        | NY    |  3000 |     1
 EVERHART, SHARON I.   | NC    |  2000 |     2
 WITTY, JOANNE         | NY    |  1500 |     1
(10 rows)
```

Three donors at the $7,000 maximum (primary plus general). `memo_cd IS
NULL` excludes memo lines so nothing is double-counted; grouping by name
*and* state keeps two different people with the same name apart, though
it won't merge one person who spelled their name two ways -- FEC data
has no donor IDs, and matching people across rows is your judgment call,
not the tool's.

One more that local reporters usually want -- how much of the money is
from in state:

```bash
psql "$DATABASE_URL" -c "SET search_path TO tut_journalist, public;" \
  -c "SELECT state = 'NC' AS in_state, count(*) AS gifts, sum(transaction_amt) AS total
      FROM schedule_a
      WHERE cmte_id = 'C00913566' AND memo_cd IS NULL
      GROUP BY 1 ORDER BY 1 DESC;"
```

```text
 in_state | gifts | total
----------+-------+-------
 t        |   172 | 28980
 f        |   143 | 59556
```

In this three-day slice: more gifts from North Carolina, but twice the
dollars from out of state. Remember the caveat from Step 4 -- that is a
fact about 315 contributions from one week in October 2025, and the full
file will tell a fuller story.

And the committee-linkage query promised in Step 7, for candidates with
more than one committee:

```sql
SELECT l.cmte_id, c.cmte_nm, l.cmte_dsgn, l.cmte_tp
FROM candidate_committee_links l
JOIN committees c USING (cmte_id, cycle)
WHERE l.cand_id = 'S6NC00407';
```

## Step 10: clean up

Stop the server with Ctrl-C, then drop the namespace -- every table, every
row, gone in one command:

```bash
hardmoney schema-drop tut_journalist --yes
```

```text
dropped namespace 'tut_journalist'
```

## What you did, and where to go next

You built a local, queryable copy of the FEC's candidate and committee
registries and a sample of itemized contributions, found a candidate by
name, followed `cand_pcc` to the committee, and pulled their largest
donors through both an HTTP API and plain SQL. No API key was needed: the
FEC's bulk files are public downloads, and hardmoney talks to `fec.gov`
directly.

- To load the *whole* contributions file and keep it current week to
  week, read [Loading a Full Election Cycle](./tutorial-researcher.md).
- To see money spent *against* (or for) a candidate by outside groups,
  which is not in Schedule A at all, read
  [Tracking Independent Expenditures](./tutorial-independent-expenditures.md).
- For every route and filter the API offers, see
  [The REST API](./rest-api.md); to put a key on it before anyone else
  can reach it, see [Hardening the API](./api-hardening.md).
