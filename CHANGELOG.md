# Changelog

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
