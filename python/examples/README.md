# hardmoney Python examples

One script per task, numbered, each runnable on its own. Every script has
a module docstring saying what it shows and how to run it, a
`main(argv)` that takes a file path (with a sensible default from
`tests/fixtures/`), and prints something readable. Money is always
`decimal.Decimal`; nothing here converts an amount to a float.

The core examples need only the standard library and `hardmoney`. The
interop and online ones need what their docstrings say and exit 0 with
a message when it is missing.

```bash
cd python
pip install hardmoney            # or: maturin develop --release
python examples/01_parse_and_print_header.py ../tests/fixtures/F3XN_2011831.fec
```

`tests/test_examples.py` runs every script against the fixtures.
The book chapter [Python cookbook](https://cgorski.github.io/hardmoney/python-cookbook.html)
walks through them with their output.

## Parsing

| | |
|---|---|
| [01_parse_and_print_header.py](01_parse_and_print_header.py) | `parse_file`, `Filing` metadata, the `header` dict, cover fields with `amount()` and `date()` |
| [02_count_lines_by_table.py](02_count_lines_by_table.py) | `iter_lines()`, `Line.table` vs `Line.form_type`, memo counts |
| [03_schedule_a_to_dicts.py](03_schedule_a_to_dicts.py) | Schedule A as dicts with `Decimal` amounts and `date` objects |
| [04_top_donors.py](04_top_donors.py) | `Counter` of Decimals by donor and employer; the sum equals the cover page |
| [05_lenient_parsing.py](05_lenient_parsing.py) | `lenient=True`, `Filing.skipped`, and the warnings `validate()` adds for them |
| [06_handle_fec_error.py](06_handle_fec_error.py) | `FecError.line_no`, showing the raw offending line, `FileNotFoundError` vs `FecError` |
| [07_memo_entries.py](07_memo_entries.py) | `is_memo`, back references, and why memos are excluded from totals |
| [08_old_spec_versions.py](08_old_spec_versions.py) | a spec 3.0 filing from 2001: same field names, different columns |
| [09_stream_large_filing.py](09_stream_large_filing.py) | a running `Decimal` total over a 135 MB, 704,651-line filing |

## Validating

| | |
|---|---|
| [10_validate_by_severity.py](10_validate_by_severity.py) | `validate()`, findings grouped by severity and rule |
| [11_validate_directory_csv.py](11_validate_directory_csv.py) | a directory of filings to one CSV report with the `csv` module |
| [12_validate_exit_code.py](12_validate_exit_code.py) | a CI / pre-submission check that exits 1 on errors (`--strict` for warnings) |
| [13_explain_finding.py](13_explain_finding.py) | join a finding to `field_spec()`: the FEC's description, type, max length, rule |

## Reconciling

| | |
|---|---|
| [14_reconcile_mismatches.py](14_reconcile_mismatches.py) | `reconcile()`, the mismatches table with `Decimal` deltas |
| [15_relations_and_threshold.py](15_relations_and_threshold.py) | `equal` vs `at_least` and the $200 itemization threshold |
| [16_recompute_a_line.py](16_recompute_a_line.py) | sum SA11AI by hand and match `LineCheck.expected` and `lines_summed` |
| [17_reconcile_batch.py](17_reconcile_batch.py) | reconcile a directory; which lines disagree most often |

## Editing and writing

| | |
|---|---|
| [18_fix_field_and_write.py](18_fix_field_and_write.py) | `Line.set` a too-long field to the spec's max, re-validate, `to_fec()`, re-parse |
| [19_bulk_edit_states.py](19_bulk_edit_states.py) | upper-case every `*_state` field across all tables, re-validate, write |
| [20_round_trip_check.py](20_round_trip_check.py) | `parse(to_fec(parse(f)))` equals `parse(f)` field for field, on every fixture |
| [21_build_minimal_filing.py](21_build_minimal_filing.py) | build a Form 3X from dicts using `layout()`, then validate, reconcile, write |

## Spec

| | |
|---|---|
| [22_spec_tables_and_layouts.py](22_spec_tables_and_layouts.py) | `tables()`, `layout()`, and a set diff of two versions' layouts |
| [23_data_dictionary.py](23_data_dictionary.py) | a Markdown data dictionary for one table from `field_spec()` |

## Interop

| | |
|---|---|
| [24_pandas_dataframe.py](24_pandas_dataframe.py) | a DataFrame with `dtype=object` Decimals (needs pandas) |
| [25_parquet_via_cli.py](25_parquet_via_cli.py) | `hardmoney export --format parquet`, read with pyarrow and DuckDB (needs the CLI, pyarrow) |
| [26_polars_dataframe.py](26_polars_dataframe.py) | a polars DataFrame with a `pl.Decimal` column (needs polars) |
| [27_sqlite_via_cli.py](27_sqlite_via_cli.py) | `hardmoney export --format sqlite`, exact sums with a `Decimal` aggregate in `sqlite3` (needs the CLI) |
| [28_json_dump.py](28_json_dump.py) | `json.dumps(..., default=str)` over `to_dict()` plus typed values |

## FEC data online

| | |
|---|---|
| [29_fetch_filing.py](29_fetch_filing.py) | `hardmoney.fetch(filing_id)` (network) |
| [30_filings_search_and_validate.py](30_filings_search_and_validate.py) | `hardmoney filings --committee ... --json --fetch`, then validate each (CLI, API key, network) |
| [31_efile_watch.py](31_efile_watch.py) | `hardmoney efile watch --once --json`, then fetch and check the new filings (CLI, network) |

## Integration patterns

| | |
|---|---|
| [32_presubmission_check.py](32_presubmission_check.py) | `check_fec_bytes(data) -> dict`: a pure function for a Django view |
| [33_http_validation_server.py](33_http_validation_server.py) | an `http.server` endpoint that validates a POSTed `.fec` (stdlib only) |
| [34_pytest_fixture_pattern.py](34_pytest_fixture_pattern.py) | a pytest fixture asserting your generated `.fec` files validate clean and balance |
| [35_compare_amendment.py](35_compare_amendment.py) | diff an original and its amendment: cover fields, transactions added/removed/changed |

## Finding the CLI

Examples 25, 27, 30, and 31 shell out to the `hardmoney` binary. They
look, in order, at `$HARDMONEY_BIN`, `hardmoney` on `PATH`, and the
newest of `target/debug/hardmoney` and `target/release/hardmoney` in a
repository checkout (`cargo build --all-features`).

## Running the tests

```bash
cd python
pytest -q tests/test_examples.py                         # skips network and missing libraries
HARDMONEY_NETWORK_TESTS=1 pytest -q tests/test_examples.py
```
