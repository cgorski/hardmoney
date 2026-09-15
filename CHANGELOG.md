# Changelog

## Unreleased

- **Python bindings** (`python/`, PyPI package `hardmoney`, PyO3 0.29 +
  maturin, one `abi3` wheel per platform for CPython >= 3.9). A
  parser-only wheel -- no tokio/sqlx/axum -- exposing `parse`,
  `parse_file` (streamed), `fetch`, and `Filing` (`header`, `summary`,
  `lines`, `lines_for`, `iter_lines`, `skipped`, `to_fec`,
  `to_fec_string`, `validate`, `reconcile`), `Line` (mapping-style field
  access, `set`, `amount() -> decimal.Decimal` built from the exact
  `Decimal` string, `date() -> datetime.date`), `Validation`/`Finding`,
  `Reconciliation`/`LineCheck`, and the spec helpers `tables`, `layout`,
  `field_spec`, `BUNDLED_SPEC_VERSION`. Errors are `hardmoney.FecError`
  (a `ValueError` with `line_no`) and `hardmoney.UnsupportedForm`;
  unknown fields are `KeyError`; a missing file is `FileNotFoundError`.
  Every class is frozen and a `Line` is a handle into its `Filing`, so
  `line.set(...)` is visible to `filing.to_fec()`. Type stubs
  (`_hardmoney.pyi`, `py.typed`) ship in the wheel and a pytest checks
  them against the compiled module. pytest runs the real fixtures
  (`tests/fixtures/*.fec`): every one parses, round-trips through
  `to_fec`, and validates with no errors; `F3XN_2011831.fec` reconciles.
  New workflow `python.yml` builds wheels on Linux and macOS (artifacts
  only; no PyPI publish). The root crate excludes `/python` from
  `cargo package`. Book: *Python*.
- **Filing discovery: openFEC and the e-file feed** (`hardmoney::fec`,
  behind `fetch`; the openFEC client also needs `serde`). `OpenFec`
  queries `/filings/` (processed metadata: amendment chains,
  `most_recent`, cover-page totals, paper filings with their negative
  file numbers) and `/efile/filings/` (raw, minutes after receipt) with
  a `FilingsQuery`/`EfileQuery` builder and an auto-paginating
  `filings_all`; the key comes from `FEC_API_KEY` or
  `~/fec_api_key.txt`, is redacted from `Debug` and every error, and 429s
  are retried per `Retry-After` (three times, 60 s cap) before
  `FecApiError::RateLimited`. `EfileFeed::poll` parses the FEC's
  e-filing RSS feed, `daily_zip_filings` reads the daily
  `electronic/YYYYMMDD.zip` archives, and `fetch_filing_bytes` downloads
  a raw `.fec` through a shared `Cache` (`~/.cache/hardmoney/filings/`,
  `HARDMONEY_CACHE_DIR`). New commands: `hardmoney filings` (query,
  table or `--json`, then `--fetch DIR`/`--validate`/`--reconcile`/
  `--ingest` per filing) and `hardmoney efile watch|backfill|cache-info|
  cache-clear` (poll the feed with a persistent seen list, `--once`,
  `--exec PROGRAM`; walk daily archives by date range). The `fetch`
  feature now also enables `zip` (for the archives). Book: *Finding
  Filings*.
- **Amendment-chain resolution for ingested filings** (migration
  `0003_amendment_chains.sql`). `filings` gains `report_id`,
  `report_code`, `coverage_from`, `coverage_through`, `amendment_number`
  (written at ingest) and the openFEC-semantics chain columns
  `amendment_version`, `amendment_chain`, `most_recent`,
  `most_recent_filing_id`, `previous_filing_id`, `chain_unresolved`,
  recomputed by a set-based resolver in the ingest's own transaction --
  so the result is the same whichever order an original and its
  amendments arrive in. An amendment whose header names no ingested
  original stands alone with `chain_unresolved = true`. New view
  `filings_current` (one row per report: its latest version); new
  `db::resolve_amendment_chain` / `db::resolve_all_amendment_chains`;
  `bulk::ingest::{ChainResolution, ingest_filing_bytes_with,
  ingest_filing_with}` and `IngestReport::chain_rows_resolved`;
  `bulk-load-filing --no-resolve` for batch loads. `GET /filings/{id}`
  gains `amendment_indicator`, `amendment_version`, `amendment_chain`,
  `most_recent`, `most_recent_file_number`, `previous_file_number`,
  `chain_unresolved`, `report_type`, `coverage_start_date`,
  `coverage_end_date`, `fec_url` (openFEC's names); new `GET
  /filings?committee_id=&most_recent=&form_type=&limit=&offset=`. Rows
  ingested before the migration have NULL chain columns until
  `resolve_all_amendment_chains` runs. Book: *Amendments*.
- **`hardmoney export`** (and the `export` module, feature `export`, on
  by default): one table per record type from a filing to CSV, JSON
  Lines, Parquet (`Decimal128` money and `Date32` dates typed from the
  bundled `FieldSpec`, zstd), or SQLite (bundled; one database, one table
  per `Table`, plus a `filings` table). Streams through `FilingReader`,
  so a 135 MB filing exports in constant memory. Book: *Exporting a
  Filing*.
- **`hardmoney validate --oracle webcheck`** (and
  `hardmoney::parser::webcheck`, behind `fetch`): submits the file to the
  FEC's own WebCheck validator, prints its findings after ours (`ERROR
  SB21B #020 Date of Expenditure {Election CFO}: ...` -- WebCheck gives
  no line numbers), and diffs the two by record type, field number and
  message template: `N matched, M only ours, K only theirs`. Exit status
  is unchanged unless `--strict-oracle` (exit 1 on any disagreement);
  `--json` gains `oracle` and `oracle_diff`. Uses WebCheck's public,
  credential-free upload channel (what the WebCheck page itself calls);
  `--webcheck-api-key`/`WEBCHECK_API_KEY` (and `--webcheck-email`)
  switch to the vendor SOAP service, which rejects every request without
  a key and is untested past that check. `WebCheck::submit ->
  OracleReport { findings: Vec<OracleFinding>, .. }`, `diff(&Validation,
  &OracleReport) -> OracleDiff`, `WebCheckError { Transport, Http,
  SoapFault, Rejected, Deferred, Unparseable }`; hand-rolled base64,
  multipart and response scanning, no new dependencies. Against the live
  service (2026-09-15) every accepted fixture passes and eight of the
  ten `tests/fixtures/invalid/` files diff clean; the two that do not
  (header amendment fields, a second cover record) are attributed to
  different lines by the two validators and are written up in the book.
  Book: *Validating a Filing* -- "Comparing with the FEC's WebCheck".

## 2.0.0 — 2026-09-15

The first release with users. Everything below describes the crate as it
is; earlier tags were development snapshots.

### Parser

- Field values are preserved as filed. The parser trims surrounding ASCII
  whitespace and removes one pair of wrapping double quotes (some vendors
  quote every field, and the FEC strips them); it changes nothing else.
  Form-type tokens, entity types, and memo flags are interpreted
  case-insensitively where they are used. `raw_form_type` is upper-cased;
  the `form_type` field keeps the spelling on the wire.
- The format is data compiled by `build.rs`: `data/fec-csv-sources/*.csv`
  gives the column layout of every table in every spec version since
  2001, and `data/fec-spec/spec-8.5.json` (distilled from the FEC's
  Electronic Filing Specification workbook by
  `scripts/distill_fec_spec.py`, with `enum`/`pattern` constraints merged
  from `fecgov/fecfile-validate`) gives each field's type, maximum length,
  required level, sample, and rule text. The generator rejects a table
  where two fields share a column, one field has two columns, a position
  cell is not a number, or two version buckets overlap.
- `SpecVersion` (numeric, ordered, `FromStr`/`Display`), `Layout` and
  `FieldDef` (a table's columns at one version), `FieldSpec`, `FieldKind`,
  `Requirement`.
- `ParsedLine` holds a `&'static Layout` and a flat value slice. Read with
  `get` (`Some("")` for blank, `None` for a field this version lacks),
  `get_non_empty`, `iter`, `field_names`, `to_cells`; write with `set`
  (`FecError::UnknownField` for a name the table lacks); build with
  `from_cells` or `from_pairs`.
- Compile-time-checked field access: one `Field<Marker>` constant per
  field per table (`tables::sch_a::CONTRIBUTION_AMOUNT`), a zero-sized
  marker type per table (`tables::markers::SchA`), and
  `Typed<'_, T>` views (`line.typed::<SchA>()`,
  `filing.summary_as::<F3X>()`) whose `get`/`money`/`date`/`string` accept
  only that table's fields. `TypedView` implementations (`ScheduleA`,
  `ScheduleB`, `ScheduleE`, `Form3XSummary`) are written against these
  constants.
- `Header` is a struct: `record_type`, `ef_type`, `fec_version_raw`,
  `version`, `soft_name`, `soft_ver`, `name_delim` (3.x-5.x), `report_id`,
  `report_number`, `comment`; `original_filing_id()`,
  `amendment_number()`, `to_fields()`.
- `Filing.amends_filing: Option<u64>`; a missing or malformed reference
  is `None`, not a parse error (`validate` reports it).
- `Table` derives `Display`, `FromStr` (case-insensitive), and `EnumIter`.
- Streaming: `FilingReader<R: BufRead>` yields body lines one at a time
  with a one-record lookahead for `[BEGINTEXT]` blocks; `filter_tables`;
  `Filing::open` and `Filing::open_with` read a path. Peak memory on a
  135 MB, 704,651-line presidential filing: 9.8 MB streaming versus
  1.34 GB for `Filing::parse_bytes`. Output is field-for-field equal to
  the eager parser on every fixture. Criterion bench in `benches/`.

### Writer

- `Filing::to_fec`, `to_fec_string`, `write_fec`. Output is canonical:
  CRLF, every record at its layout's full width, ASCII-28 for spec 6.0+
  and CSV quoting for 3.x-5.x, a Form 99's text as a
  `[BEGINTEXT]`/`[ENDTEXT]` block, Windows-1252 when every character fits
  (the FEC's character set) and UTF-8 otherwise. Re-parsing the output
  gives the same header, cover, and body lines for every fixture; writing
  again gives identical bytes. Property tests over arbitrary values.
- `hardmoney write <file> [--out PATH] [--check] [--lenient]`.

### Reconciliation

- `Filing::reconcile` for Form 3X, 3, and 3P. Column A lines are
  recomputed from schedule sums (memo entries excluded) or from formulas
  over the other reported lines; Column B lines from formulas. Lines whose
  transactions need itemizing only above the $200 aggregate threshold
  (operating expenditures, offsets, other receipts and disbursements,
  refunds to individuals) are checked as floors (`Relation::AtLeast`);
  all others must match exactly. Unitemized lines are inputs. The
  allocation lines from Schedules H3-H6 are computed from the spec's rule
  text. The rule tables are written in a small macro in the FEC's own
  notation. Accepts the `SA11A1` token spelling of 3.x-5.x files.
- Oracle test: agrees with the FEC's FECfile+ summary calculator on every
  Column A line of its test dataset. Corpus: 95 of 109 real reports
  satisfy every rule; the rest are truncated third-party samples or
  genuine filer discrepancies.
- `hardmoney reconcile <file> [--all] [--column A|B] [--tolerance N] [--json]`.

### Validation

- `Filing::validate` applies 32 rules modelled on the FEC's published
  failing and warning messages: structure (HDR first, cover second, one
  form per file, schedules allowed with the form), IDs (format, filer id
  on every line), per-field checks driven by `FieldSpec` (required,
  length, type, legal characters, dates, amounts, allowed values,
  patterns), unique transaction ids, resolvable back-references, state
  and entity codes, F99 text length. `Validation { findings }` with
  `errors()`, `warnings()`, `is_acceptable()`, `Display`. Zero
  error-severity findings across 102 filings the FEC accepted; the
  warnings that remain are the FEC's own (blank addresses, one
  out-of-range date).
- `hardmoney validate <file> [--json] [--strict-warnings]`; exit 1 on any
  error.

### Spec as data

- `hardmoney spec tables`, `spec fields <TABLE> [--version V]`,
  `spec export` (every layout and every spec row as JSON), and
  `spec diff <FROM> <TO>` (fields added, removed, and moved between two
  spec versions).

### Format data corrections

Six defects in the vendored `fech-sources` tables, found by the build-time
checks and recorded in `NOTICE`: F3 6.1-6.3 `election_date` shared a column
with `report_code`; F3X 5.x/3.x `col_b_cash_on_hand_jan_1` read the wrong
column because of an unquoted comma in its label; F3S 5.x had label text in
three position cells so those fields were unreadable; F5 5.3
`individual_occupation` shared a column with `coverage_through_date`; F57
3.x `payee_street_2` shared a column with `payee_street_1`; HDR had
overlapping version buckets and listed `name_delim` for 6.x+.

### API and CLI

- `?limit` outside `1..=500` and negative `?offset` return 400 with a
  message instead of being silently clamped. `hardmoney query --limit`
  is validated the same way.
- Book: chapters on the schema, fidelity, writing, reconciling,
  validating, streaming; CLI reference for every command.
- `CONTRIBUTING.md`.

### Dependencies

`compact_str` (inline small strings for field values), `strum` (enum
derives); dev: `proptest`, `criterion`. `serde_json` with
`preserve_order` so JSON keeps the FEC's column order. Build
dependencies: `csv`, `regex`, `serde_json`.

## 1.1.0 — 2026-09-15

### Added
- `hardmoney query <candidates|candidate|committees|committee|contributions|disbursements|ies|filing|filing-ies|schema>`:
  search loaded data from the terminal without curl. Runs the REST API's
  own router in-process against the database (identical results), or hits
  a running server with `--api-url` / `--api-key`. Table output by default,
  `--json` for raw.

### Fixed
- `build.rs` now also watches `src/` and `data/` (a `rerun-if-changed`
  for migrations alone stopped Cargo from rebuilding on source edits).

## 1.0.1 — 2026-09-15

### Fixed
- Migration 0002 installed `pg_trgm` into the *current namespace* (first on
  `search_path`) instead of `public`. On a database where the extension was
  not preinstalled, the second namespace to migrate failed with `operator
  class "gin_trgm_ops" does not exist`, and dropping the first namespace
  took the extension with it. The extension is now pinned to `public` and
  the operator class is schema-qualified. Databases migrated by 1.0.0 are
  fine if 0002 succeeded; if it failed, `hardmoney schema-init` will retry
  it.
- Added `build.rs` with `rerun-if-changed=migrations` so an edited
  migration always triggers a rebuild (`sqlx::migrate!` embeds the SQL at
  compile time; without this Cargo could ship stale SQL).
- `bulk-restore-dump` also pins its `pg_trgm`/`btree_gin` extensions to
  `public`.

## 1.0.0 — 2026-09-15

First stable release. Breaking changes from 0.1 throughout; 0.1 is not
supported.

### Parser
- `ParsedLine.table` is a `Table` enum (59 variants); `ParsedLine.line_no`
  added; `Filing.summary` is a `ParsedLine`.
- `TypedView` trait with `ParsedLine::view::<V>()` and
  `Filing::views::<V>()`. Views refuse lines from the wrong table
  (`TypedViewError::WrongTable`), fixing a bug where every schedule line
  converted "successfully" to `ScheduleE`.
- `EntityType` and `SupportOppose` code enums with `Other(String)`.
- Strict/lenient parsing: `ParseOptions`, `Lenient<T>` (`#[must_use]`),
  `SkippedLine`. Errors carry `line_no`.
- Dispatch covers 20 more form-type patterns: F1/F1S/F1M/F2/F2S,
  F3Z/F3Z1/F3Z2/F3P31/F3PZ1/F3PZ2, F8/F8II/F8III, F10/F105, Schedule I.
  Every Form 1/1M/2 filing on the FEC's live feed (~26% of volume) failed
  to parse before this. New `F2S.csv` table; `SchI.csv` bucket widened.
- `parse_fec_date` requires exactly eight digits; `parse_money` uses
  checked arithmetic; unterminated `[BEGINTEXT]` is an error; no
  input-reachable panics.
- `once_cell` replaced by `std::sync::LazyLock`.

### Database and bulk ETL
- Schema is `sqlx::migrate!` migrations (`migrations/`). `schema-init`,
  `schema-status`, `schema-list`, `schema-drop`.
- Namespaces: `--schema` / `HARDMONEY_SCHEMA` isolates a full set of
  tables per Postgres schema.
- `bulk-load` defaults to `--mode replace` (transactional delete + COPY);
  `--mode append`, `--yes` guard, `--if-changed` (ETag/Last-Modified).
  Every load recorded in `loads`.
- `--limit` now stops the download instead of draining the whole file.
- Raw `*_dt` text columns keep the FEC's string; new `*_date DATE` twins
  parsed at load from both FEC formats (`MMDDYYYY`, `MM/DD/YYYY`).
- Trigram GIN indexes (`pg_trgm`, guarded) for substring search; dead
  btree name indexes removed.
- `bulk-load-filing`: correct filing id from filenames, lenient by
  default with `--strict`, `skipped_lines` stored.
- `bulk-restore-dump`: exits 0 on success, clean re-restore, refreshes
  views, XDG cache dir.
- `Cycle` newtype validates cycles at every boundary.
- Fixed `role "anonymous"` on URLs without a username (sqlx 0.9 × whoami).
- Loader tolerates Windows-1252 bytes; unique staging paths.

### API
- `GET /schema`; hardening (`--api-key`, `--cors-origin`,
  `--timeout-secs`, statement timeout, body cap, graceful shutdown);
  database errors never echoed; odd `?cycle=` → 400; missing view → 503.
- Richer filters on `/schedule-a` and `/disbursements`; `transaction_date`
  in responses; `skipped_lines`/`ingested_at` on `/filings/{id}`.

### Project
- MSRV 1.94. CI: fmt, clippy, docs, MSRV, Postgres 18 integration tests,
  package. Manual, gated publish workflow.

## 0.1.0 — 2026-09-14

Initial release.
