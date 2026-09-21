# Changelog

## Unreleased

- **CORS allow-list mode now works for browser clients.** With
  `--cors-origin`, the server answers a preflight with
  `Access-Control-Allow-Headers: content-type, x-api-key` and exposes
  `Content-Disposition` and `X-Hardmoney-Validation-Errors`, so a page at
  an allowed origin can send the API key and a JSON body and read what
  `POST /tools/write` returns; before, every such request failed the
  browser's preflight. New `ApiConfig::validate` (and `ApiConfigError`)
  rejects an origin a browser could never match -- a trailing slash, a
  path, uppercase, a missing scheme, `*` -- and `serve` (the library
  function and the CLI) runs it before binding, so a mistyped
  `--cors-origin` is a startup error naming the entry instead of a silent
  browser failure. `api::router` no longer panics on `*`; it skips and
  logs an entry `validate` would reject.
- **`GET /tools/fetch/{id}` never downloads more than the body cap.** The
  filing is refused with 413 from the document store's `Content-Length`
  before any of it is read, or as soon as the cap is passed for a body of
  unknown length; before, the whole filing (up to 135 MB) was read into
  memory and only then compared with the cap. At most four fetches run at
  once per server (`tools::MAX_CONCURRENT_FETCHES`); further ones wait
  their turn within the request timeout. Library: new
  `fec::download_filing_bytes_capped` and `FecApiError::TooLarge`;
  `download_filing_bytes`, `fetch_filing_bytes*`, and `Filing::fetch_bytes`
  are unchanged and still unbounded.
- **`dumps import` keeps the database password off `pg_restore`'s command
  line.** The URL's password is passed to the child in `PGPASSWORD`
  (percent-decoded) and `-d` receives the URL without it, so it is no
  longer readable by every user on the machine through `ps` for the
  hours a restore runs. `dump::pg_restore_command` and `--explain` show
  the command exactly as it runs, with no password; new
  `preflight::strip_password`.
- **The CLI reads `.env` from the current directory only.** It used to
  search every parent directory as well, so a `.env` in the home
  directory or a checkout above the working folder could repoint
  `DATABASE_URL` or the `HARDMONEY_*` FEC endpoints without notice. The
  documentation always said "the current directory"; the code now does.
- Tests: the Postgres integration tests fail, rather than silently pass,
  when `HARDMONEY_TEST_DATABASE_URL` is unset under GitHub Actions, and
  the crates.io publish workflow now runs them against a Postgres service
  (it ran the suite without a database before). CI workflows pin every
  action to a commit, grant `contents: read` by default, build with
  `--locked`, and cancel superseded pull-request runs.
- **A local Postgres in one command.** `docker-compose.yml` and
  `scripts/dev-postgres.sh up|test|psql|url|down` bring up the same
  container the CI test job uses (Postgres 18, or `PG_VERSION=15` for the
  FEC's major; role, password, and database identical, so the test URL is
  the same string) on host port 5433, leaving an existing Postgres on
  5432 alone; `test` runs `cargo test --all-features` against it.
- **Validator: 3x faster on large filings, same findings.** The
  per-field classifiers that decide what a column is (state, ZIP, date,
  district, office, election code, check-box, year, phone) and what a
  blank one is judged by read the FEC workbook's prose for every field of
  every line; that prose is constant, so it is now read once per column
  (`FieldProfile`), and the current-format decision once per filing.
  Validating the 135 MB presidential filing drops from 5.7 s to 1.9 s
  (parse 0.7 s). `validate --json` output is byte-identical on all 61
  fixtures and on that filing. New `benches/validate.rs` times
  `validate`, `reconcile`, and `review` on parsed fixtures
  (`HARDMONEY_BENCH_FILING` adds one of your own).
- **Writer: three round-trip gaps closed, found by fuzzing.** (1) A
  `[BEGINTEXT]` block after a body line: the parser splices it into that
  line's `text` with its line breaks (and any ASCII-28, since block lines
  are not split on the delimiter); the writer wrote the value back inline
  with both turned into spaces. Now any record whose `text` contains
  either is written with a block after it, exactly as read; other text
  stays inline. (2) A value that itself begins and ends with `"` -- what
  the wire cell `""-""` parses to, seen in a real Schedule B purpose --
  lost its own quotes on re-parse, because the parser strips one wrapping
  pair as a wire convention and the writer did not add one back; it now
  does, in both formats, and the property tests no longer exclude such
  values. (3) In a comma-delimited filing an ASCII-28 inside a quoted
  value is data (the delimiter is decided by the header line alone); the
  writer replaced it with a space. Also fixed: a spec 3.x-5.x Form 99 with
  text had its text hoisted into a block those formats do not have, which
  the parser then read as a bad record; pre-6.0 text is always inline.
  (4) A comma-delimited header whose first cell was `"/*..."` or
  `"<BOM>..."` on the wire parses (the quotes hide the two markers the
  parser acts on at byte 0: the pre-3.0 comment header it rejects, and a
  UTF-8 byte-order mark it strips) but was written bare, so the output
  was rejected or lost its first character; the cell is written quoted
  again -- decided on the encoded bytes, since a quoted BOM read from a
  Windows-1252 file is three Latin characters that encode back to the
  same three bytes.
  None of the corpus fixtures' canonical output changes.
- **Releases from CI, gated.** A `v*` tag now publishes the wheels and
  sdist to PyPI from the workflow that built and attested them, after the
  maintainer approves the run in the `pypi` GitHub environment (which
  admits only `v*` tags); the crates.io publish workflow likewise waits
  for approval in the `crates-io` environment (which admits only `main`)
  and can be rehearsed with its `dry_run` input. The default branch has a
  ruleset forbidding force-pushes, deletion, and merge commits.
- **Fuzzing.** `fuzz/` holds four cargo-fuzz targets -- `parse_bytes`,
  `stream_vs_eager`, `roundtrip`, `validate_reconcile` -- with a `.fec`
  token dictionary and a seed script over the fixtures. `fuzz.yml` runs
  each nightly for ten minutes, keeps the evolved corpus in the Actions
  cache, and uploads crashes; CI type-checks the targets on stable on
  every push. The two documented parser contracts (eager and streaming
  decode differently on a file that mixes UTF-8 and Windows-1252 lines;
  a file whose header delimiter disagrees with its version is written in
  the version's format) are encoded in the targets rather than reported.
- **Blocking work leaves the async runtime.** The `/tools/*` handlers
  parse, validate, reconcile, and write a filing on tokio's blocking pool
  instead of a runtime worker, so one 32 MiB upload no longer stalls every
  other request (and the database pool's housekeeping) on that worker; a
  worker failure is a 500 (`ToolsError::Internal`) rather than a
  misreported 502. The CLI does the same for every `ureq` call it makes
  while a Postgres pool may be open -- `efile watch`/`backfill` polls and
  downloads, `filings` paging, downloads, `--reconcile-chain`, and the
  parse/validate/reconcile and `--exec` hook in `apply`, and
  `bulk-load-filing`'s download -- and `dumps` preflight runs its
  `pg_restore --version` and `df` children off the runtime. `bulk-load`
  from a URL no longer sends a HEAD request on every load: the download's
  own `ETag`/`Last-Modified` are recorded (a HEAD is made only for
  `--if-changed`, off the runtime), and the download goes through the
  crate's shared HTTP agent, so it carries the `hardmoney/<version>`
  User-Agent, has a connect timeout, and turns a non-2xx answer into an
  error naming the URL instead of trying to unzip an HTML error page.
  Tests: a `/tools/parse` of a 2 MB filing on a single-threaded runtime
  must let a concurrent timer fire (it fires ~370 times; the old inline
  handler let it fire 0 times); staging from a URL records the response
  headers and sends the User-Agent; a 404 is a typed error.
- **Fixtures are protected as bytes.** A new `.gitattributes` marks
  `tests/fixtures/**` and `*.fec` as never line-ending-converted (git's
  binary heuristic only happened to protect the FS-delimited ones; a
  Windows checkout with `core.autocrlf=true` would have rewritten the
  comma-delimited CRLF filings), stores all source as LF, and
  `tests/fixtures/MANIFEST.sha256` (`scripts/fixture_manifest.py`) records
  the committed SHA-256 of all 90 fixture files. New
  `tests/corpus_shape.rs` fails on any byte difference, pins the corpus's
  coverage of the line-ending and delimiter regimes the FEC produces
  (CRLF and LF, FS and comma, non-ASCII, `[BEGINTEXT]`), and checks that
  every fixture parses identically with CRLF and with LF line endings,
  strictly and leniently. CI checks the manifest and that no tracked text
  file carries CRLF. The 24 `data/fec-csv-sources/*.csv` files that
  carried CRLF or mixed line endings (upstream fech-sources rows were
  CRLF, rows appended here were LF) are normalised to LF; the generated
  format tables are byte-identical. Dependabot's two cargo entries are
  merged into one covering `/` and `/python`, so a dependency bump
  updates both lockfiles in one pull request (as two entries, every such
  PR failed `--locked` on the lockfile it did not touch). Book:
  `mermaid-init.js` refreshed to the file mdbook-mermaid 0.17.1 writes;
  the vendored copy looked up theme buttons by their mdBook 0.4 ids, so
  switching theme did not re-render diagrams.

## 3.2.0 — 2026-09-16

- **Every FEC address is configurable, from one place.** New
  `hardmoney::fec::Endpoints` holds the six bases the crate requests
  (`www.fec.gov`, openFEC, `docquery.fec.gov`, `efilingapps.fec.gov`, the
  e-file RSS URL, WebCheck), each defaulting to production and overridable
  by environment variable (`HARDMONEY_FEC_WWW_BASE`,
  `HARDMONEY_OPENFEC_BASE`, `HARDMONEY_DOCQUERY_BASE`,
  `HARDMONEY_EFILINGAPPS_BASE`, `HARDMONEY_EFILE_RSS_URL`,
  `HARDMONEY_WEBCHECK_ENDPOINT`), by builder (`with_*`), or by CLI flag
  (`--fec-www-base`, `--openfec-base`, `--docquery-base`,
  `--efile-rss-url`, `--webcheck-endpoint` on `filings`, `lag`, `efile
  watch`/`backfill`, `bulk-load`, `bulk-load-all`, `bulk-restore-dump`,
  `bulk-dump-info`, `bulk-load-filing`, `dumps check`/`import`/`status`/
  `update`, `validate`, and `serve`). A malformed override fails before
  any request with a message naming the variable; nothing falls back to
  production silently. Links the FEC hands out for `docquery.fec.gov` (RSS
  items, openFEC `fec_url`) are rebased onto the configured document
  store, so a mirror works for feed- and API-driven downloads; `serve`
  builds the `fec_url` in `/filings` responses from `--docquery-base` as
  a bound SQL parameter and `/tools/fetch` downloads from it. Library
  additions: `Endpoints`, `Url`, `EndpointError`, `fetch_filing_bytes_with`,
  `resolve_fec_url_with`, `download_filing_bytes`, `daily_zip_filings_with`,
  `parse_feed_with`, `EfileFeed::with_endpoints`, `OpenFec::with_endpoints`,
  `WebCheck::from_env`/`with_endpoints`, `bulk::source::download_url_with`,
  `BulkSource::doc_url`, `DumpSource::url`, `dump::remote_info_with`,
  `dump::download_with`, `RestoreOptions::endpoints`, `ApiConfig::endpoints`,
  and `FecApiError::Endpoint`/`DumpError::Endpoint`. `OpenFec::from_env`,
  `fec::fetch_filing_bytes`, `resolve_fec_url`, `daily_zip_filings`,
  `dump::remote_info`, `dump::download`, and `Filing::fetch` now read the
  variables. Changed: `BulkSource.doc_url` is now the path field
  `doc_path` plus a `doc_url(&Endpoints)` method, and `DumpSource.url` is
  now `file_name` plus a `url(&Endpoints)` method; `Filing::fetch_bytes`
  requests `https://docquery.fec.gov` directly instead of `http://` and
  following the redirect. The `bulk` feature now implies `fetch`. Python:
  `hardmoney.fetch` honours `HARDMONEY_DOCQUERY_BASE`. Book: the security
  chapter's network table gains an override column and a section on
  mirrors and proxies, with what `ureq` 3 does with the `*_PROXY`
  variables.
- **`hardmoney review`: the RAD-style checks that lead to an RFAI,
  measured against the FEC's letters.** New `hardmoney::parser::review`
  with `Filing::review()` / `review_with(&ReviewOptions)` and
  `ReportChain::review()`, producing a `Review` of `Observation`s
  (`concern`, `line_no`, `transaction_id`, `amount`, `detail`,
  `rfai_request_type`) with a per-concern summary; thirteen `Concern`s,
  each documented with its basis in 11 CFR or the form instructions:
  employer/occupation blank over $200 (best-efforts language reported
  separately), contributions over the limit for the recipient and cycle
  (`CONTRIBUTION_LIMITS`, 2019-2026, per election for candidates with
  redesignation memos netted, per year for PACs and parties), receipts
  dated after the period, missing treasurer signature, earmarks counted
  twice, memo entries without a parent, cover lines `reconcile` flags,
  unexplained negative entries, unitemized totals inconsistent with
  itemization, duplicate transaction ids and same-contributor repeats,
  and, across a `ReportChain`, cash carry-forward, Column B, aggregates
  crossing $200, and contributions repeated from earlier reports. CLI:
  `hardmoney review <file|id>... [--recipient] [--with-prior] [--json]`,
  exit 0 always, and `--eval --cycle YYYY [--sample N] [--rescore FILE]`,
  which joins openFEC's `FRQ` letters to the reports they name by
  committee and coverage period, reviews lettered and clean reports, and
  prints precision, recall, F1, and per-concern lift. Measured on 2024
  (98 lettered, 96 clean; 257 requests): precision 51% and recall 37% at
  report level, precision 80% against committees with no letter; two
  rules were narrowed by the result (pre-period dates, negative children
  of counted parents). Python: `Filing.review(recipient=None)`,
  `hardmoney.Review`, `hardmoney.Observation`. Book: "Reviewing a
  filing", with the concern table, limits, method, and numbers.

## 3.1.1 — 2026-09-15 (Python package only)

- `hardmoney.compat.fecfile_validate.validate_file` and `load_filing`
  accept a `str` path to an existing file (a single-line `str` naming a
  file is read; a multi-line `str` is still `.fec` content, as FECfile+'s
  composer returns). Previously `validate_file("report.fec")` parsed the
  file name as content and reported "filing has no cover/summary line".

## 3.1.0 — 2026-09-15

- **Release hygiene for institutional users.** `SECURITY.md` (supported
  versions, disclosure address, response commitment, the full list of
  hosts each CLI command contacts, secret handling); a "Support and
  security" section in the README with the point of contact and release
  cadence; a book chapter for security reviewers
  (`book/src/security-and-provenance.md`: network behaviour, data
  handling, dependency policy, SBOM, build provenance and how to verify
  it, reproducibility, licences, data provenance, release process);
  issue templates for validator and reconcile disagreements and bugs.
  Supply chain: `deny.toml` (permissive-licence allow-list with three
  documented crate-scoped exceptions, RustSec advisories, crates.io-only
  sources, OpenSSL banned) and a `supply-chain.yml` workflow running
  `cargo audit`, `cargo deny`, and a CycloneDX SBOM (crate, extension
  crate, wheel) on every push and weekly; Dependabot for Cargo, pip, and
  Actions. CI now tests the Postgres integration suite on PG 15 (the
  FEC's version) as well as 18. Python: wheels are built for Windows
  x86_64, Linux aarch64, and macOS Intel in addition to Linux x86_64 and
  Apple silicon, with `--locked`; tests run on Windows; `v*` tags
  produce build provenance attestations (`gh attestation verify`) and a
  PyPI publish job gated on the `PYPI_PUBLISH` variable and
  `PYPI_API_TOKEN` secret. `pyproject.toml` gains a `Security` URL. The
  README's validator count is corrected from 32 to 41 rules.
- **The spec as JSON Schema and CSV, and a published drift report.**
  `hardmoney spec export --format json-schema` renders the compiled FEC
  format as one JSON Schema (draft 2020-12) per table, as a `$defs`
  bundle or one document with `--table`, at any version with a layout
  (`--version 7.0`): a record is an object keyed by canonical field name,
  every value a string, with `maxLength` from the FEC's type (`AMT-n`
  allows a sign and a point, as the FEC does), `enum` and `pattern` from
  its code lists and formats (blank always allowed; emitted only at the
  version the workbook describes, since accepted 3.00 filings violate the
  8.5 district pattern), `required` for `X (error)` fields, and an
  `x-fec` object per field carrying the workbook row (1-based `column`,
  `type` code, `required_level`, `sample`, `value_reference`, `rule`,
  `forms`) plus `x-hardmoney` document metadata. The rendering is
  `hardmoney::parser::jsonschema::{table_schema, bundle}` (feature
  `serde`). `--format csv` writes one flat row per table, version bucket,
  and field with the spec columns joined (7,361 rows). `--table` and
  `--version` also filter the JSON export. `tests/spec_export.rs` checks
  every record of every fixture against its table's schema (in-crate,
  and with the Python `jsonschema` package under
  `HARDMONEY_ORACLE_TESTS=1`). `scripts/distill_fec_spec.py
  --diff-report PATH` writes a Markdown report of what changed against
  the checked-in spec; the weekly `spec-drift.yml` job now uploads that
  report, the regenerated JSON, the JSON Schema bundle, and the CSV as a
  `spec-drift-<date>` artifact and writes the report to the step summary
  (the deliverable fecfile-validate#302 asks for). New book chapter
  *The spec as data* (`book/src/spec-as-data.md`).
- **Golden fixture pack** (`tests/fixtures/golden/`): nine spec-8.5
  `.fec` files generated deterministically by
  `examples/golden_fixtures.rs` from `ParsedLine::from_pairs` and
  `Filing::from_parts`, with every cover page filled to the fixed point of
  `Filing::reconcile` so each Column A and Column B line balances. F3X
  (Schedules A, B, C, D, E, F, H3, H4, built from FECfile+'s public-domain
  `test_calculate_summary_column_a` transaction set), F3, F3P, F24, F1M,
  F99 (with a `[BEGINTEXT]` block), and three broken F3X variants (a
  cover total one cent off, a duplicate transaction id, a blank required
  field) with their intended findings. Beside each file, the exact
  `hardmoney validate --json` and `hardmoney reconcile --json --all`
  output; `f3x.expected_column_a.json`, every Column A line keyed the way
  FECfile+'s `calculate_summary_column_a` keys them, matching that test's
  values on the lines it computes and carrying the H3/H4-derived values
  for the five lines it stubs to zero; and a `MANIFEST.json` with SHA-256,
  provenance, and expected outcomes per file. `tests/golden_fixtures.rs`
  regenerates the pack in memory and byte-compares it with the committed
  files, re-validates and re-reconciles each, and compares the sidecars
  with the CLI's output. All nine were submitted to WebCheck on 2026-09-15
  (six `SUCCESS` with no findings; the broken ones fail as intended); the
  pack's `README.md` records the verdicts and why the committee ids are
  those of committees terminated decades ago (WebCheck checks every id
  against the FEC registry).
- **`hardmoney.compat.fecfile_validate` and a pytest plugin.** The
  Python package gains `hardmoney.compat.fecfile_validate`, which presents
  whole-file validation in the shape of the FEC's `fecfile-validate`
  package (`validate_file(data) -> ValidationResult` with `.errors` and
  `.warnings` of `ValidationError(message, path)`, plus `validator`,
  `instance`, `severity`, `line_no`, `form_type`), an emulated per-record
  `validate_record(form_type, fields)` (the record is serialised to one
  `.fec` line under a minimal header and cover and findings filtered to
  that line; FECfile+'s field spellings and non-wire keys are accepted),
  and `summary_column_a(data) -> dict[str, Decimal]` keyed `line_11ai`
  ... as FECfile+'s `summary.py` keys them. `hardmoney.pytest_plugin`,
  registered through the `pytest11` entry point, adds a `hardmoney`
  marker and the `fec_filing`, `assert_fec_acceptable`,
  `assert_fec_balances`, and `fec_fixture` fixtures with a
  `--hardmoney-fixtures=DIR` option. New book chapter *Using hardmoney
  from FECfile+ and other Python projects*
  (`book/src/python-fecfile-plus.md`).
- **Section 508 review of the browser UI.** `hardmoney serve --ui` was
  reviewed against WCAG 2.1 Level AA with axe-core 4.10.2 in headless
  Chrome over seventeen application states, computed contrast ratios for
  every colour pair in both themes, and a scripted keyboard-only
  walkthrough; the result is the new book chapter *Accessibility of the
  browser UI* (`book/src/accessibility.md`, with a conformance table and
  the known limitations). Fixes in `src/ui/assets/`: the file chooser is
  reachable by keyboard; the records grid is one tab stop with a roving
  tabindex (arrows, Home/End, PageUp/PageDown, Enter to edit, Enter on a
  header to sort), reports `aria-rowcount`/`aria-rowindex`, keeps focus
  across virtualised redraws, and never redraws under an open editor;
  focus moves to the heading after loading or closing a filing and after
  navigation; the page title changes per view; all three tab strips share
  one `tabs.js` with `aria-controls`/`tabpanel`; the header tooltip is
  linked with `aria-describedby`, hoverable, and dismissed by Escape; the
  theme toggle keeps one label and reports `aria-pressed`; toasts feed two
  persistent live regions and pause while hovered; a rejected filing id
  marks the field `aria-invalid` with the message in its description;
  mismatch rows, edited cells, error lines, the selected tab, and the
  pressed toggle carry text or a mark as well as colour; a
  `--control-border` token puts every control boundary at 3:1 or better;
  the layout reflows at 320 px. New string-level tests in
  `tests/ui_routes.rs` guard the shell's `lang`, title, skip link, single
  `main`, labelled controls, live regions, the absence of positive
  `tabindex` and `outline: none`, and the widget attributes the review
  verified.
- **Column B across a chain of reports.** `ReportChain` in
  `hardmoney::parser::reconcile` takes a report and the committee's
  earlier reports (any order) and checks what one file cannot: every
  Column B schedule line against the sum of that schedule over the
  reports in the period, with the same `Relation` as Column A, and cash
  on hand carried forward (6(b)/23/6 against the prior report's 8/27/10;
  Form 3X 6(a) against the last report of the prior year when the chain
  has one). The period is typed (`PeriodBasis::YearToDate` for Form 3X,
  `CycleToDate` for Forms 3 and 3P) and follows the FEC's form
  instructions and FECfile+'s `calculate_summary_column_b` /
  `calculate_cash_on_hand_fields`. `ReportChain::new` rejects another
  form, another filer, a report that does not end before the current one
  begins, and overlapping reports (an original with its amendment), each
  as a typed `ChainError`; `gaps()` names uncovered days. `LineCheck`
  gains `reports_summed` (1 for every existing check). CLI: `hardmoney
  reconcile FILE --with-prior FILE...` appends the chain checks and a
  `chain` object to `--json`; `hardmoney filings --reconcile-chain`
  orders a committee's reports by coverage and prints one line per report
  (Column A, Column B over the chain, cash carried). Verified on the
  Republican Party of Minnesota's twelve 2025-2026 reports and a national
  committee's seven 2026 monthlies (52 MB); five of the Minnesota files
  are fixtures in `tests/fixtures/chain/` with a provenance README, and a
  known $200 Column A discrepancy is shown propagating into Column B of
  every later report. Book: "Column B and prior reports" in
  `reconciling.md`.
- **Processing lag and `fec_url` resolution.** `hardmoney lag ID...`
  joins openFEC's `/efile/filings/` (receipt time), `/filings/` (summary
  processed), and `/operations-log/` (`summary_data_complete_date`,
  `transaction_data_complete_date`, keyed by beginning image number) to
  say how far the FEC's processed data is behind each filing, three
  requests per fifty ids; `--committee C --cycle 2026` measures a whole
  committee in a handful of requests and prints the distribution (median,
  p90, max, pending past 30 and 60 days); `--counts` compares the raw
  filing's Schedule A/B/E line counts with the rows openFEC has processed
  for the filing's page range. Library: `OpenFec::operations_log`,
  `processing_status`, `committee_processing_status`, `processed_rows`,
  `resolve_fec_url`; `OperationsLogQuery`, `OperationsLogRecord`,
  `ProcessingStatus`, `ProcessingStage`, `ScheduleEndpoint`; repeated
  `file_number=` on `FilingsQuery` and `EfileQuery`.
  `hardmoney::fec::resolve_fec_url(id)` asks openFEC for the filing's
  `fec_url` and falls back to the `docquery.fec.gov` template, and
  `fetch_filing_bytes` uses it when given no URL (openFEC#6717 is
  inventorying docquery for retirement). REST: `GET
  /filings/{id}/processing` asks openFEC live; 503 with guidance when the
  server has no key, no database row needed. Measured 2026-09-15 on the
  fifty oldest Form 3X/3 reports in the RSS feed (received 2026-09-08):
  summary lag 0 days for all, transactions loaded for 42 within 6 days
  (median 1, p90 6), 8 pending at 7 days. Book: "Processing lag" in
  `discovery.md`. New openFEC fixtures under `tests/fixtures/openfec/`.

## 3.0.1 — 2026-09-15

- **For FEC staff** (`book/src/for-fec-staff.md`): a chapter arranged by
  role (Reports Analysis Division first-pass review and amendment diffs,
  Electronic Filing Office vendor conformance with the WebCheck diff,
  FECfile+ integration points and golden fixtures, Data division dump
  restores and raw-vs-processed comparison, audit-style cycle
  reconciliation), each backed by a runnable script under
  `python/examples/fec/` with captured output and a pytest module. New
  fixtures: `F3XN_1996410.fec` (the original of the existing
  `F3XA_2011821.fec`, a real original/amendment pair) and
  `tests/fixtures/rad/F3XA_2011912.fec` (a real accepted report with a
  $200 line 11(c) discrepancy).
- **Python documentation reorganised.** The book now has a top-level
  *Python* part (Getting started with Python, Python cookbook, Python API
  reference, For FEC staff) ahead of the Rust chapters, with part headers
  (*Tutorials*, *Filings*, *Data in Postgres*, *Serving*, *Reference*)
  grouping the rest. The API reference (`book/src/python-api.md`) is
  generated from the type stub `_hardmoney.pyi` by
  `scripts/gen_python_api.py` (signatures, docstrings, cross-links;
  standard library only) and checked for staleness in the book workflow.
  The stub gains docstrings for `FecError.line_no` and the `__len__`,
  `__iter__`, `__str__`, and `__repr__` methods, so the reference and
  `help()` describe them. `python/README.md` (the PyPI page) is rewritten
  to stand alone with a fuller quick start and absolute links to every
  Python page; `pyproject.toml` adds `API reference`, `Cookbook`,
  `For FEC staff`, `Source`, and `Issues` URLs. Installation, Quick start,
  and the Introduction gain Python sections.

## 3.0.0 — 2026-09-15

- **Breaking: one canonical field vocabulary across all 59 tables.** The
  field names in `data/fec-csv-sources/*.csv` spelled the same concept
  differently from one table to the next (`transaction_id` on Schedule A
  but `transaction_id_number` on B, C, D, E, F, H4, H6 and TEXT;
  `memo_text_description` on Schedule A but `memo_text` on H4;
  `col_a_individual_contributions_itemized` on Form 3 but
  `col_a_individuals_itemized` on 3X and 3P; `date_of_election` on 3X but
  `election_date` on 3). 159 names on 39 tables are renamed so each
  concept has one spelling everywhere: `transaction_id`,
  `back_reference_tran_id`, `back_reference_sched_name`, `memo_text`,
  `*_zip_code`, `*_street_1`/`_2`, `*_middle_name`, `election_date`,
  `state_of_election`, `candidate_id_number`, `payee_committee_id_number`,
  `expenditure_purpose_descrip`, `amended_cd`, and on the cover pages of
  F3, F3Z*, F3P, F3PZ*, F3X, F4 and Schedule L the F3-family spellings
  for lines whose FEC labels describe the same line
  (`col_a_political_party_contributions`, `col_a_pac_contributions`,
  `col_a_candidate_contributions`, `col_a_candidate_loans`,
  `col_a_refunds_to_*`, `col_a_total_refunds`,
  `col_a_cash_on_hand_beginning_period`, ...). No column position or
  version bucket changed, so parsing is byte-for-byte the same; only the
  keys differ. The generated constants follow (`sch_b::TRANSACTION_ID`,
  `sch_a::MEMO_TEXT`), as do the typed views (`ScheduleA::memo_text`,
  `ScheduleB::memo_text`, `ScheduleE::memo_text`), the reconcile rule
  tables, `LineCheck.field`, validation findings' `field`, exports'
  column headers, and the JSON stored in `schedule_e_lines.raw` (re-ingest
  before `bulk-dump-compare` on filings ingested with 2.x). The full map,
  one family per concept with the rule that picked the winner, is
  `scripts/rename_canonical.py` (the script that applied it; refuses
  collisions and touches only column 1); `NOTICE` lists every family;
  the new book chapter *Field names* documents the rules, the concept
  table, and every field of every table with its FEC label; a new test
  in `tests/format_table_integrity.rs` enforces the rules on any future
  table change.
- **Correctness and hardening audit of everything outside the parser**
  (`db`, `api`, `bulk`, `export`, `fec`, `ui`, `cli`, Python bindings).
  Fixed: concurrent ingests of amendments to one original could leave
  two rows `most_recent` (each chain `UPDATE` ran against a snapshot
  without the other's uncommitted row); the resolver now takes a
  transaction-scoped advisory lock on the chain's original id
  (`db::chain_lock`), with a test that reproduces the race.
  `GET /filings/{id}/schedule-e` is a 404 for an id that was never
  ingested, like the parent route (it was `[]`).
  `GET /independent-expenditures` pages deterministically (`sub_id` is
  the tiebreaker). The request log span redacts `?api_key=` so
  `RUST_LOG=debug` never writes the key. `POST /tools/{write,validate,
  reconcile}` bound a document by the size of the filing it would
  *write* (sum of the records' layout widths) rather than by its JSON
  size, 413 past the body cap; a record whose `table` disagrees with its
  form-type token, or a schedule record without one, is a 400 that says
  what disagrees (it was "no format table for form type 'SCHA'").
  `fec`: `ureq`'s `BadUri`/`RequireHttpsOnly` errors, which quote the
  request URI, are redacted before they reach an error message.
  `bulk::dump`: a resumed download whose 206 carries a different ETag
  than the saved partial restarts from zero instead of splicing two
  files. `dumps check` measures free space for a relative cache dir that
  does not exist yet; `dumps remove --keep-file` says the file was kept
  rather than "not downloaded". `efile watch` sleeps on the runtime, not
  the thread. `query` percent-encodes path ids and rejects a negative
  `--offset` up front. New `hardmoney bulk-resolve-chains`: the missing
  CLI half of `bulk-load-filing --no-resolve` (and the fix-up after
  migration `0003` on a namespace that already held filings). Book: the
  CLI reference is re-captured from 2.2.0 (`validate --oracle`,
  `bulk-restore-dump --cycles/--no-indexes/--dump-file/--jobs`, the three
  `bulk-dump-*` commands were missing); *API hardening* says 64 KiB / 32
  MiB with `--ui` (it said 64 KB); *REST API*, *Amendments*, and *Web UI*
  document the behaviours above. Tech debt: one `cli::shared` module
  replaces the two copies of the column renderer and the dump cache
  default; `bulk::filing_id_from_path` is the same four lines as the
  export one.

- **Parser correctness audit against an independent oracle.** Every
  corpus filing (141 real filings, 435,726 records, 19.5 million field
  values) was parsed with both hardmoney and the `fecfile` Python library
  and compared field by field (`tests/oracle_fecfile.py`, run by
  `tests/parser_oracle.rs` under `HARDMONEY_ORACLE_TESTS=1`); every
  disagreement was traced to the FEC's own column listings or v8.5
  workbook and is documented in `tests/fixtures/ORACLE_NOTES.md`. Found
  and fixed in hardmoney: **comma-delimited (spec 3.x-5.x) filings are
  now parsed one record per physical line** -- a stray unbalanced `"` at
  the end of a record (every Schedule H4 line of a real Aristotle 3.00
  filing, now `tests/fixtures/quirks/`) used to open a CSV quote that
  silently swallowed the remaining 113 records into one field; `line_no`
  on that path is now the physical line (it was off by the blank lines
  skipped before each record, and the cover was always reported as line
  2) on both the eager and streaming parsers. Table data (each recorded
  in `NOTICE`): Schedule C-2 in 3.x-5.x had a phantom `transaction_id`
  column that shifted every guarantor field one column right; Form 5 had
  no layout for spec 6.1 and Form 3Z none before 6.4 although the FEC
  lists both; Form 3L column 14 (STATE OF ELECTION, the only v8.5
  workbook column no layout named) is now `state_of_election`; and the
  3.x-5.x AMENDED CD column, which real filings populate, is named
  `amended_cd` on the 21 tables where it was unnamed. A leading UTF-8 BOM
  no longer leaks into `header.record_type`. The writer keeps a bare `\r`
  inside a value (the parser does), so `parse(write(parse(f)))` is exact
  on every corpus filing including the Windows-1252 ones. `ParsedLine::set`
  updates `raw_form_type` when the *column-0* field is set (`rec_type`
  on TEXT), not whenever a field happens to be called `form_type`;
  `Layout::token_field` names that column. `build.rs` and the parser
  modules in scope have no unchecked arithmetic, indexing, or `unwrap`
  left outside tests. New `tests/parser_adversarial.rs` (proptest):
  arbitrary and filing-shaped bytes, truncation at every offset, injected
  delimiters/quotes/NULs/invalid UTF-8/`[BEGINTEXT]`, a 5 MB single line,
  and random valid filings over every table and version never panic and
  round-trip through the writer and `FilingReader`; every `FecError`
  variant is provoked. Docs corrected: `Filing::parse` lists its
  delimiter choice and every error; `Header::from_fields` its failure
  cases; `normalize_field` cites the FEC's rule #30 for why a
  quote-wrapped field is never data; `decode` explains the UTF-8-first
  choice; `is_allowed_top_level_form` says it takes a *base* form.

- **Reconcile and validate audited against 98 freshly fetched FEC-accepted
  reports.** The Schedule H3-H6 rules (F3X 18(a), 18(b), 21(a), 30(a)),
  written from the spec text, were checked against 45 state-party Form 3X
  reports carrying 3-648 H4 and up to 33 H3 records each: every one
  balances, confirming 18(a) = sum of H3 `transferred_amount` (each
  event-type record repeats the transfer total; summing that would
  double-count) and 21(a)(i)/(ii) = H4 federal/nonfederal shares with
  memo entries excluded (4,065 memos in the sample; no report carries an
  `SB21A`). Four lines are now floors (`>=`) rather than identities, per
  the FEC's own Form 3X/3/3P instructions ("aggregating in excess of
  $200"): F3X 24 (independent expenditures; Schedule E's unitemized
  "Line (b)" has no electronic field) and 30(b) (100%-federal election
  activity; seven of the 45 reports exceed their `SB30B` sum), F3 11(d)
  and F3P 17(d) (contributions from the candidate). The corpus now stands
  at 124 periodic reports, 102 balancing every Column A rule; the 22
  others are 17 truncated third-party samples and 5 single-line filer
  discrepancies, each written up in `tests/fixtures/ORACLE_NOTES.md`
  along with a twelve-cent gap on one NRSC line 12. `Relation`'s docs
  now tabulate the exact-versus-floor derivation for every line;
  `LineCheck::reported_unparseable` says what it does (blank is zero, not
  garbage); the module documents why `Decimal` saturation cannot hide a
  discrepancy. Six fixtures added (all FEC-accepted, spec 8.5): three
  state parties with H2-H4 (Georgia GOP, Republican Party of Virginia,
  New Hampshire Democrats), a House amendment with 58 Schedule C loans
  and 3 Schedule D debts, a joint fundraising committee with 106 `SB22`
  transfers, and a 2026 presidential amendment. **Validate:** three false
  positives found on accepted filings and fixed -- `non_numeric` on the
  `NUM-5` allocation percentages of Schedules H1/H2 (`0.49`, on 16 of 45
  party reports; a decimal point is numeric), `field_too_long` on
  `AMT-12` amounts written in 13 characters (ActBlue's accepted
  `2280311229.59`; the bound is on digits), and `illegal_character` on a
  2001 report's `á`/`í` (demoted to a warning on superseded formats, as
  `required_field_empty` is). Nine rules added from the FEC's message
  list, each data-driven from the workbook: `invalid_year` (#36),
  `invalid_district` (#39, replacing the pattern warning; demoted on old
  formats), `invalid_event_type` (#45, H3 codes read from the value
  reference), `invalid_zip_code` (W30), `invalid_phone_number` (W31),
  `invalid_office_code` (W32), `invalid_election_code` (W5),
  `invalid_checkbox` (W43), `address_in_second_line` (W28).
  `header_inconsistent_with_amendment_status` is an error, as the FEC's
  #8 is; `invalid_allowed_value` is an error where the workbook says
  "Error if Coded incorrectly" (report codes, #22). Conditional
  requirements whose condition sits in the workbook's rule cell ("Used
  if CCM, PAC or PTY", "Used if CAN or CCM" on Schedule A's donor
  columns) are now evaluated, matching WebCheck's live behaviour.
  `Rule::demoted_on_superseded_format` names the demoted set. Every FEC
  message without a rule is tabulated in the module docs with the reason
  (#22/#38 need a report-code list the workbook elides; #37 cannot be
  enforced on `A/N-15` free text that accepted filings fill with
  `Prime -1`). Across 364 corpus files every FEC-accepted one reports
  zero errors. **WebCheck:** `OracleReport::is_acceptable` treats the
  service's `WARNINGS` verdict ("PASSED validation with Warnings") as
  accepted, as the FEC does; WebCheck's `is Required, but field is
  Empty` wording for a blank `X (warning)` column is a live alternate for
  `recommended_field_empty`; a live test pins five 2026 fixtures against
  the service. `webcheck.rs` slices are `get`-based throughout; no `as`
  cast remains in the three modules.

## 2.2.0 — 2026-09-15

- **`hardmoney dumps`: a guided import of the FEC's Postgres dump files**
  for people who have not used Postgres or a terminal much. `hardmoney
  dumps` alone is the "where am I" screen (the four files with today's
  sizes from fec.gov, which are downloaded, which are in the database,
  what to run next). `dumps check` runs the prerequisites, one `✓`/`!`/`✗`
  line each with a plain fix: `pg_restore` on `PATH` and version 15 or
  newer (the archives are written by Postgres 15), a database URL (or
  the `createdb`/`export DATABASE_URL` recipe when there is none), the
  connection (refused, missing database, unknown user, bad password,
  each explained), server version, the `disclosure` schema exists or is
  creatable, `pg_trgm`/`btree_gin` available, free disk where downloads
  go and at the server's data directory (tolerating `SHOW
  data_directory` being denied or the server being remote) against what
  the import needs, and the namespace's `schema-init` (offered). `dumps
  import committees | independent-expenditures | receipts |
  disbursements | all-small [--cycles] [--yes] [--explain]
  [--dump-file]` prints a plan in words (bytes to download, rows, table,
  database, disk, and time ranges scaled from the FEC's own published
  timings), asks, downloads with a progress bar, restores with a
  heartbeat line every 30 seconds, adds hardmoney's indexes for a
  cycle-selective restore (`receipts`/`disbursements` default to the
  current cycle, data only), and ends with rows, time, `hardmoney query`
  and `psql` commands to try, and where the data lives; `--explain`
  prints the exact `pg_restore` command and SQL and exits. `dumps
  status` (rows, indexes, cycles, last import and its source file,
  downloads, and whether fec.gov has a newer file), `dumps update
  [--yes]` (re-import what has a newer file; prints a cron line), and
  `dumps remove` (drop the table and the download, after confirming).
  Every command takes `--json`; every error says what to do next before
  the raw message. Library: `hardmoney::bulk::preflight` (`run ->
  Preflight { checks: Vec<Check { name, status, detail, fix }> }` with
  `Display`, the `check_*` functions, `parse_pg_version`,
  `free_disk_space`/`judge_space`, `estimate -> ImportNeeds`,
  `explain_connect_error`, `redact_url`) and, in `bulk::dump`,
  `plan_restore_offline`, `pg_restore_command`, `drop_restored`,
  `evict_cached`, `RemoteDump::is_newer_than`,
  `RestoreRecord::source_version`. Book: *The FEC's Postgres dump files*
  now opens with the three commands and the vocabulary; new tutorial
  *Importing the FEC's database dumps, from nothing* with the real
  session; `dumps` in the CLI reference.
- **The FEC's Postgres dumps, first-class** (`hardmoney::bulk::dump`).
  `bulk-restore-dump` gains `--cycles 2024,2026` (restore only those
  two-year periods of `schedule_a_full` / `schedule_b_full`: the parent
  plus the named child tables, discovered from `pg_restore --list` so
  new cycles need no code change; a cycle the archive lacks is an error
  before anything is dropped; lifts `--allow-large`), `--no-indexes`
  (`--section=pre-data --section=data`, the FEC's 5 h instead of 35 h
  for Schedule A), `--dump-file PATH` (restore an archive you already
  have; its table of contents is checked first), and `--jobs`.
  Downloads resume: a dropped connection keeps `<name>.dump.partial`,
  the next run sends `Range` plus `If-Range` with the saved ETag, and
  the file is renamed to `.dump` only when its length matches
  `Content-Length`. New `bulk-dump-info` (the four dumps' current size
  and `Last-Modified` from `HEAD`, cache state, database state with
  estimated rows, index count, and cycles present, and the restores
  recorded in the namespace), `bulk-dump-index <name> [--cycles]`
  (hardmoney's few indexes per restored table: unique `sub_id`,
  committee id plus date, trigram on the name column when `pg_trgm` is
  available, `s_o_cand_id` and `file_num` on Schedule E; idempotent),
  and `bulk-dump-compare <filing_id>` (an ingested filing's raw Schedule
  E lines against the FEC's processed `fec_fitem_sched_e` rows for the
  same `file_num`: counts, totals, transaction ids on one side only,
  amount mismatches, and whether the filing postdates the dump). Every
  restore is recorded in `loads` (source `dump:<name>`, one row per
  cycle, mode `restore` / `restore-data-only`). `db::ensure_views` now
  also creates `dump_schedule_a`, `dump_schedule_b`, `dump_schedule_e`,
  and `dump_committee_history` (the FEC's column names plus `cycle`)
  for whichever dump tables exist, and tolerates a table dropped by a
  concurrent restore; new `db::ensure_dump_views`. Library: `DumpToc`,
  `TocEntry`, `read_toc`, `partition_table`, `partition_cycle`,
  `RestoreOptions`, `restore_with`, `plan_restore`, `download`,
  `cached`, `remote_info`, `create_indexes`, `table_state`,
  `restore_history`, `compare_filing`, and `DumpError` (converts into
  `BulkError`; `restore` keeps its 2.x signature). `DumpSource` is now
  `#[non_exhaustive]` with `partitioned_by_cycle` and `indexes`. Book:
  *The FEC's Postgres dump files* (what the real archives contain, the
  80 processed Schedule E columns against the raw `.fec` fields, the
  per-cycle child tables, the FEC's own timings, and that the Schedule E
  dump has no Form 24 rows).
- **Browser UI** (`hardmoney serve --ui`, roadmap §5). A filing
  workbench at `/ui`: drop a `.fec` or fetch one by filing id, read the
  header and cover page, see validation findings grouped by severity with
  links to the offending line, see the cover-page reconciliation with
  mismatching lines highlighted, edit any field (records table, cover
  card, header) with debounced re-validation and re-reconciliation, review
  the edits as a before/after list, and download the written `.fec`. The
  records table renders only the rows in view, so 100k-line filings
  scroll without stalling; column headers show the FEC's field spec. A
  data browser at `/ui/data` over the JSON API: candidate and committee
  search, committee pages (ingested filings with amendment chain,
  Schedule A, Schedule B, independent expenditures), candidate pages with
  support/oppose totals added exactly from decimal strings, and a
  `/schema` dashboard. Light and dark themes (system preference plus a
  persisted toggle), WCAG AA contrast, keyboard-navigable grid, no CDN
  dependencies; assets are embedded in the binary with `rust-embed` and
  served with `ETag` revalidation and a same-origin CSP. New
  database-free routes, on with `--ui` and behind the API key like every
  other route: `POST /tools/parse`, `GET /tools/fetch/{id}`, `POST
  /tools/validate`, `POST /tools/reconcile`, `POST /tools/write` (with
  `Content-Disposition` and `X-Hardmoney-Validation-Errors`), and `GET
  /tools/spec/{table}?version=`. `ApiConfig` gains `ui: bool` and
  `ApiConfig::ui(bool)`, which raises `max_body_bytes` from 64 KiB
  (`DEFAULT_MAX_BODY_BYTES`) to 32 MiB (`UI_MAX_BODY_BYTES`) unless set
  explicitly; in CORS allow-list mode `POST` is allowed when the UI is
  on. New `Filing::from_parts(header, summary, lines)` constructor, the
  only way to build a `Filing` outside the crate; it derives the form
  fields exactly as parsing does and rejects a line whose token does not
  dispatch to its table or whose layout is not the header's version. New
  module `hardmoney::ui`, new `api::routes::tools`, new test
  `tests/ui_routes.rs` (no database needed). Book: *The browser UI*.

## 2.1.0 — 2026-09-15

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
