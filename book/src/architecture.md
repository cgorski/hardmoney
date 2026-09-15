# How data flows through hardmoney

This chapter is a map. Each diagram names the real types, functions,
commands, and tables involved, so you can go from a box to the code
(`src/`) or to the chapter that explains it. Read it once before the
tutorials, or come back to it when you want to know where a particular
piece of FEC data enters and where it ends up.

## Overview

```mermaid
flowchart LR
    subgraph Sources[FEC sources]
        Docquery["docquery.fec.gov posted/ID.fec"]
        Rss["e-file RSS feed"]
        Daily["electronic/YYYYMMDD.zip"]
        BulkZip["bulk-downloads/CYCLE/*.zip"]
        Dumps["data-dump pg_dump archives"]
        OpenFec["openFEC API"]
        WebCheck["WebCheck validator"]
    end
    subgraph Hm[hardmoney]
        Parser["parser: Filing, to_fec"]
        Validate["Filing::validate"]
        Reconcile["Filing::reconcile"]
        Export["export: CSV, JSONL, Parquet, SQLite"]
        Loader["bulk::loader"]
        Restore["bulk::dump (pg_restore)"]
        Ingest["bulk::ingest"]
        Pg[("Postgres: namespaces + disclosure")]
        Api["api: Axum REST"]
        Ui["ui: /ui workbench"]
        Cli["hardmoney CLI"]
        Py["hardmoney Python package"]
    end
    subgraph Who[Consumers]
        Journalists
        Researchers
        Filers["filers and vendors"]
        FecStaff["FEC staff"]
        Scripts
    end
    Docquery & Rss & Daily --> Parser
    OpenFec -->|"names filings"| Docquery
    BulkZip --> Loader
    Dumps --> Restore
    Parser --> Validate & Reconcile & Export & Ingest & Py
    Validate <-->|"--oracle"| WebCheck
    Loader & Restore & Ingest --> Pg
    Pg --> Api
    Api --> Ui & Cli & Journalists
    Export --> Researchers
    Validate --> Filers
    Reconcile --> FecStaff
    Cli & Py --> Scripts
```

Every box on the left is an FEC endpoint: six supply bytes or rows, and
WebCheck receives a filing and returns the FEC's own findings.
Everything in the middle is a module of the crate (`src/parser`,
`src/fec`, `src/export`, `src/bulk`, `src/db`, `src/api`, `src/ui`,
`src/cli`, and the PyO3 crate in `python/`). Individual filings flow
through the parser and out to the validator, reconciler, exporter, or
Postgres ingester; the FEC's aggregated products (bulk zips and `pg_dump`
archives) bypass the parser and go straight into Postgres.

### Where each kind of FEC data enters and which command handles it

| Source (URL pattern) | Command | Table or output |
|---|---|---|
| `docquery.fec.gov/dcdev/posted/<id>.fec` | `parse`, `validate`, `reconcile`, `write`, `export` (a file); `bulk-load-filing <id>` and `Filing::fetch` download it | `Filing` in memory; `filings` + `schedule_e_lines` when ingested |
| `efilingapps.fec.gov/rss/generate?preDefinedFilingType=ALL` | `efile watch` | cache (`~/.cache/hardmoney/filings/<id>.fec`), then `--validate`, `--reconcile`, `--ingest`, `--out`, `--exec` |
| `fec.gov/files/bulk-downloads/electronic/YYYYMMDD.zip` | `efile backfill --from --to` | same actions as `efile watch`, one archive per day |
| `api.open.fec.gov/v1/filings/` and `/efile/filings/` | `filings --committee ... --fetch/--validate/--reconcile/--ingest` | filing metadata, then the same per-filing actions |
| `fec.gov/files/bulk-downloads/<cycle>/{cn,cm,ccl,indiv,oth,pas2,oppexp,weball,webl,webk}<yy>.zip` | `bulk-load <source> --cycle`, `bulk-load-all --cycle` | `candidates`, `committees`, `candidate_committee_links`, `schedule_a`, `committee_to_committee_transactions`, `committee_to_candidate_transactions`, `disbursements`, `candidate_summary`, `house_senate_summary`, `pac_party_summary`, plus a `loads` row |
| `fec.gov/files/bulk-downloads/data-dump/schedules/{fec_fitem_sched_a,fec_fitem_sched_b,fec_fitem_sched_e,ofec_committee_history}.dump` | `bulk-restore-dump <name> [--cycles]`, `bulk-dump-index` | `disclosure.*` tables; `independent_expenditures` and `dump_*` views in each namespace |
| `efoservices.fec.gov/webcheck/services/upload` | `validate --oracle webcheck` | the FEC's findings, diffed against ours |
| the spec workbook and fecfile-validate schemas (build time) | `scripts/distill_fec_spec.py`, then `cargo build` | `Table`, `Layout`, `FieldSpec` statics; `spec tables/fields/export/diff` |

## A filing's life inside the parser

```mermaid
flowchart TD
    Bytes["bytes: file, docquery download, or cache"]
    Decode["decode: UTF-8, else Windows-1252"]
    Split["split records: ASCII-28 (spec 6.0+) or CSV (3.x to 5.x)"]
    Hdr["Header::from_fields (line 1)"]
    Ver["SpecVersion"]
    Cover["cover line (line 2)"]
    Body["body lines"]
    Dispatch["form::table_for_form_type gives a Table"]
    Layout["Table::layout(SpecVersion) gives a static Layout"]
    Cells["ParsedLine::from_cells"]
    Text["[BEGINTEXT] ... [ENDTEXT] block"]
    Attach["attach_free_text: spliced into the preceding record's text field"]
    Acc["BodyAccumulator: push_body_line, skip_or_fail per ParseOptions"]
    Len["Lenient#lt;Filing#gt;"]
    Filing["Filing: header, summary, lines"]
    Parts["(Filing, Vec#lt;SkippedLine#gt;)"]
    Out[".fec bytes: CRLF, full column count, Windows-1252 or UTF-8"]
    Data["data/fec-csv-sources/*.csv and data/fec-spec/spec-8.5.json"]
    Build["build.rs"]
    Statics["Layout, FieldDef, FieldSpec statics in tables.rs"]

    Bytes --> Decode --> Split
    Split --> Hdr --> Ver
    Split --> Cover --> Dispatch
    Split --> Body --> Dispatch
    Split --> Text --> Attach --> Acc
    Ver --> Layout
    Dispatch --> Layout --> Cells --> Acc
    Acc --> Len
    Len -->|"into_strict()"| Filing
    Len -->|"into_parts()"| Parts
    Filing -->|"to_fec()"| Out
    Out -.->|"parse again: same header, cover, lines"| Bytes
    Data --> Build --> Statics --> Layout
```

`Filing::parse_bytes` decodes once for the whole file, reads the header
and cover line, and then runs every body line through the same two
lookups: the form-type token in column 0 picks a `Table`, and the
filing's `SpecVersion` picks that table's `Layout`. A `ParsedLine` keeps
a reference to the layout it was parsed with, which is what lets
`Filing::to_fec` write every field back at the right column. Under
`ParseOptions::STRICT` the first unparseable body line fails the parse;
under `ParseOptions::LENIENT` it is recorded as a `SkippedLine` and the
`Lenient<Filing>` must be opened with `into_parts()` or `into_strict()`.
`FilingReader` (and `Filing::open`, built on it) follow the same steps
one record at a time for filings too large to hold in memory.

## Build-time schema pipeline

```mermaid
flowchart LR
    Csv["data/fec-csv-sources/*.csv: column positions, every version since 2001"]
    Xlsx["data/fec-spec/FEC_EFO_Format_Specifications_v8.5.xlsx"]
    Schemas["fecgov/fecfile-validate schema/*.json: enum and pattern"]
    Distill["scripts/distill_fec_spec.py"]
    Json["data/fec-spec/spec-8.5.json"]
    Build["build.rs"]
    Gen["$OUT_DIR/tables.rs"]
    Tables["Table enum, Layout per version bucket, Field constants, FieldSpec rows, BUNDLED_SPEC_VERSION"]
    Parser["parser: Table::layout, ParsedLine"]
    Validate["validate: per-field rules from FieldSpec"]
    Reconcile["reconcile: cover-page field names"]
    SpecCli["hardmoney spec tables, fields, export, diff"]
    Parquet["export: Parquet types from FieldKind (Decimal128, Date32, Utf8)"]
    PySpec["Python: tables(), layout(), field_spec()"]
    Drift["spec-drift.yml, Mondays: re-distil against upstream develop and diff"]

    Xlsx & Schemas --> Distill --> Json
    Csv & Json --> Build --> Gen
    Gen -->|"include! in src/parser/tables.rs"| Tables
    Tables --> Parser & Validate & Reconcile & SpecCli & Parquet & PySpec
    Drift -.->|"fails CI when the JSON is stale"| Json
```

Nothing about the FEC format is hand-written in Rust. `build.rs` reads
the per-table CSVs (which column each field occupies in each spec
version) and the distilled `spec-8.5.json` (type, length, required
level, rule text, allowed values, regex per field) and emits
`$OUT_DIR/tables.rs`, which `src/parser/tables.rs` includes. The parser,
validator, reconciler, `spec` command, Parquet exporter, and Python
package all read the same statics, so a format change is a data change
followed by a rebuild. The weekly `spec-drift.yml` job re-runs the
distillation against the FEC's current fecfile-validate schemas and
fails when the checked-in JSON differs.

## Validate and reconcile

```mermaid
flowchart TD
    F["Filing"]
    V["Filing::validate()"]
    S["structure: check_header, check_cover (HDR first, cover second, FEC type, amendment ids, filer id)"]
    P["per field: check_body driven by FieldSpec (required, length, charset, dates, amounts, enums, patterns)"]
    X["cross-record: check_transaction_ids (duplicate tran ids, back references not found)"]
    Val["Validation: Vec of Finding (Rule, Severity, line_no, field, message)"]
    R["Filing::reconcile()"]
    SS["Source::Schedules: sum one amount over listed tokens, memo lines excluded, Relation Equal or AtLeast"]
    FM["Source::Formula: signed sum of other reported cover lines"]
    Rec["Reconciliation: Vec of LineCheck (reported, expected, delta)"]
    Oracle["hardmoney validate --oracle webcheck"]
    Wc["WebCheck::submit: POST efoservices.fec.gov/webcheck/services/upload"]
    Diff["webcheck::diff: matched, only ours, only theirs"]

    F --> V & R
    V --> S & P & X --> Val
    R --> SS & FM --> Rec
    Val --> Oracle --> Wc --> Diff
    Val --> Diff
```

`validate()` never fails: it walks the header, cover line, and body,
applying structural rules, the per-field rules the bundled `FieldSpec`
rows describe, and the cross-line transaction-id checks, and returns a
`Validation` whose `Finding`s carry the FEC's own message wording.
`reconcile()` handles Forms 3X, 3, and 3P: each cover-page `LineRule`
is either a `Source::Schedules` sum (memo entries excluded, compared as
`Equal` or `AtLeast`) or a `Source::Formula` over other reported lines,
and each produces a `LineCheck`. With `--oracle webcheck` the CLI also
uploads the file to the FEC's WebCheck and prints the diff between the
two sets of findings.

## Postgres side

```mermaid
flowchart LR
    subgraph In[Inputs]
        Zip["bulk-downloads/CYCLE/indiv26.zip, cm26.zip, ..."]
        Fec[".fec filing: path or filing id"]
        Dump["data-dump/schedules/*.dump"]
    end
    subgraph Writers
        Loader["bulk-load: loader stages CSV, parses date twins, COPY in one transaction, writes a loads row"]
        Ingest["bulk-load-filing: ingest, lenient parse, filings + schedule_e_lines"]
        Resolve["db::resolve_amendment_chain: most_recent, amendment_chain, previous_filing_id"]
        Restore["bulk-restore-dump: pg_restore --table per cycle partition, data-only; bulk-dump-index runs create_indexes"]
    end
    subgraph Pg[Postgres]
        Ns["namespace schema: candidates, committees, schedule_a, disbursements, ..., loads"]
        Filings["filings, schedule_e_lines, filings_current view"]
        Disc["disclosure.fec_fitem_sched_a, _b, _e, ofec_committee_history"]
        Views["per-namespace views: independent_expenditures, dump_schedule_a, dump_schedule_b, dump_schedule_e, dump_committee_history"]
    end
    Mig["schema-init: db::migrate runs migrations/*.sql, then ensure_views"]
    Conn["db::connect: search_path = namespace,public"]
    Api["serve: /candidates, /committees, /schedule-a, /disbursements, /independent-expenditures, /filings, /filings/{filing_id}/schedule-e"]
    Query["hardmoney query: same router in-process, or --api-url"]
    Ui["/ui browser"]

    Zip --> Loader --> Ns
    Fec --> Ingest --> Filings
    Ingest --> Resolve --> Filings
    Dump --> Restore --> Disc --> Views
    Mig --> Ns & Filings & Views
    Conn --> Ns
    Ns & Filings & Views --> Api
    Api --> Query & Ui
```

Three writers, one reader. The `loader` streams a bulk zip into a
staged CSV (adding a parsed `DATE` twin for each raw date column and the
`cycle`), then in one transaction deletes the cycle's old rows (`replace`
mode), `COPY`s the file in, and records the load in `loads`. `ingest`
parses a single `.fec` leniently, stores the header and cover as JSONB
plus the Schedule E lines as typed rows, and re-resolves the filing's
amendment chain so `filings_current` is right as soon as it commits.
`pg_restore` puts the FEC's own dumps into the fixed `disclosure` schema,
and `ensure_views` exposes them inside each namespace. Every hardmoney
table is unqualified, so `db::connect`'s `search_path` decides which
namespace a load, a migration, or a query sees. The Axum router reads all
of it; `hardmoney query` dispatches to the same router in-process, and
the browser UI calls it over HTTP.

## Live feed

```mermaid
sequenceDiagram
    participant W as hardmoney efile watch
    participant Feed as efilingapps.fec.gov RSS
    participant C as Cache (HARDMONEY_CACHE_DIR)
    participant DQ as docquery.fec.gov
    participant P as parser
    participant PG as Postgres
    participant X as --exec PROGRAM
    loop every --interval seconds (or once with --once)
        W->>Feed: EfileFeed::poll()
        Feed-->>W: Vec of FeedItem (filing_id, form_type, committee_id, url)
        W->>C: read_seen(), keep unseen ids, sort ascending
        loop each new item passing --form-type and --committee
            W->>C: fetch_filing_bytes(id, cache, url)
            alt not cached
                C->>DQ: GET item.url, fallback docquery_url(id)
                DQ-->>C: .fec bytes, write_filing (atomic)
            end
            C-->>W: bytes
            opt --out DIR
                W->>W: write DIR/id.fec
            end
            opt --validate or --reconcile
                W->>P: Filing::parse_bytes_with(bytes, LENIENT)
                P-->>W: validate() counts, reconcile() balances
            end
            opt --ingest
                W->>PG: ingest_filing_bytes(pool, id, bytes, LENIENT)
                PG-->>W: IngestReport (schedule_e_lines, skipped, chain_rows_resolved)
            end
            opt --exec
                W->>X: PROGRAM id path with HARDMONEY_FILING_ID and HARDMONEY_FILING_PATH
            end
        end
        W->>C: append_seen(new ids)
    end
    Note over W,DQ: efile backfill --from --to reads electronic/YYYYMMDD.zip through daily_zip_filings instead of the feed, then runs the same actions.
```

`efile watch` polls the FEC's RSS feed, which lists the last seven days
of e-filings within minutes of receipt, and processes each id it has
not seen before, oldest first so amendments follow their originals. The
bytes come through `fetch_filing_bytes`, which reads the cache before
touching the network and stores each download atomically. The actions
(`Actions` in `src/cli/filings.rs`) are shared with `efile backfill` and
`hardmoney filings`, so a filing found through the daily archives or
openFEC is handled identically.

## Python

```mermaid
flowchart LR
    subgraph Pkg["python/hardmoney (pure Python)"]
        Init["__init__.py re-exports"]
        Pyi["_hardmoney.pyi stubs and py.typed"]
    end
    subgraph Ext["_hardmoney extension: PyO3, python/src/*.rs"]
        Fns["parse(), parse_file(), fetch()"]
        PyFiling["Filing: Arc RwLock around hardmoney::Filing plus skipped lines"]
        PyLine["Line: same Arc plus a slot (Summary or Body index)"]
        PyVal["Validation, Finding"]
        PyRec["Reconciliation, LineCheck"]
        PySpec["tables(), layout(), field_spec(), BUNDLED_SPEC_VERSION"]
        Types["money as decimal.Decimal from the Decimal string; dates as datetime.date"]
    end
    subgraph Crate[hardmoney crate]
        RParse["Filing::parse_bytes_with, parse_with, open_with, fetch_bytes"]
        RFiling["Filing and ParsedLine"]
        RVal["Filing::validate"]
        RRec["Filing::reconcile"]
        RWrite["Filing::to_fec"]
        RSpec["Table::ALL, Table::layout, Table::spec"]
    end
    Init --> Fns & PyFiling
    Pyi -.-> Init
    Fns --> RParse --> PyFiling
    PyFiling -->|"lines, lines_for(), summary"| PyLine
    PyLine -->|"get, amount(), date(), set()"| RFiling
    PyLine --> Types
    PyFiling -->|"validate()"| RVal --> PyVal
    PyFiling -->|"reconcile()"| RRec --> PyRec
    PyFiling -->|"to_fec()"| RWrite
    PySpec --> RSpec
```

The Python package is the same Rust code behind a thin PyO3 layer. A
Python `Filing` owns the parsed `hardmoney::Filing` inside an
`Arc<RwLock<_>>`; a `Line` is a handle into that shared filing (the
cover line or a body index), so `line.set(...)` followed by
`filing.to_fec()` writes the edit through. The typed stubs in
`_hardmoney.pyi` describe the surface, and money crosses the boundary as
`decimal.Decimal` built from the exact `Decimal` string, never as a
float. The [Python](./python.md) chapter has the full surface.
