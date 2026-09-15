"""Every script in ``python/examples/`` runs against the fixtures and prints
what its docstring promises.

Each example is run as a subprocess (``python examples/NN_*.py args``) so
``sys.argv`` handling, exit codes, and imports are exercised the way a
reader would hit them. Examples needing an optional library skip when it
is not installed; those needing the ``hardmoney`` CLI skip when no build
is found; those needing the network run only with
``HARDMONEY_NETWORK_TESTS`` set.
"""

from __future__ import annotations

import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Optional, Sequence

import pytest

import hardmoney
from conftest import FIXTURES, INVALID

EXAMPLES = Path(__file__).resolve().parents[1] / "examples"
REPO = Path(__file__).resolve().parents[2]
NETWORK = bool(os.environ.get("HARDMONEY_NETWORK_TESTS"))


def _cli() -> Optional[str]:
    env = os.environ.get("HARDMONEY_BIN")
    if env:
        return env
    found = shutil.which("hardmoney")
    if found:
        return found
    builds = [p for p in (REPO / "target" / prof / "hardmoney" for prof in ("debug", "release")) if p.exists()]
    return str(max(builds, key=lambda p: p.stat().st_mtime)) if builds else None


def _has(module: str) -> bool:
    return importlib.util.find_spec(module) is not None


def _example(prefix: str) -> Path:
    [path] = EXAMPLES.glob(f"{prefix}_*.py")
    return path


def run(prefix: str, *args: object, env: Optional[dict[str, str]] = None,
        timeout: float = 120) -> subprocess.CompletedProcess[str]:
    full_env = dict(os.environ)
    if env:
        full_env.update(env)
    return subprocess.run(
        [sys.executable, str(_example(prefix)), *map(str, args)],
        capture_output=True, text=True, timeout=timeout, env=full_env, cwd=EXAMPLES.parent,
    )


def ok(prefix: str, *args: object, expect: Sequence[str] = (), **kwargs: object) -> str:
    result = run(prefix, *args, **kwargs)  # type: ignore[arg-type]
    assert result.returncode == 0, f"exit {result.returncode}\n{result.stdout}\n{result.stderr}"
    for text in expect:
        assert text in result.stdout, f"{text!r} not in output:\n{result.stdout}"
    return result.stdout


def test_every_example_has_a_docstring_and_main() -> None:
    scripts = sorted(EXAMPLES.glob("*.py"))
    assert len(scripts) == 35
    numbers = [int(p.name[:2]) for p in scripts]
    assert numbers == list(range(1, 36))
    for script in scripts:
        source = script.read_text(encoding="utf-8")
        assert source.startswith('"""'), script.name
        assert "def main(argv" in source, script.name
        assert 'if __name__ == "__main__":' in source, script.name
        assert "Run:" in source, script.name
        # Money is never a float. Example 24 converts once, deliberately, to
        # show the damage; every other script must not call float() at all.
        if not script.name.startswith("24_"):
            assert "float(" not in source, script.name


# --- parsing -----------------------------------------------------------------

def test_01_parse_and_print_header() -> None:
    ok("01", FIXTURES / "F3XN_2011831.fec",
       expect=["form type:       F3XN (base F3X)", "soft_name", "col_a_total_receipts                   15245.52",
               "coverage_from_date                     2026-08-01"])
    # A form with a different cover page still prints (fields it lacks are skipped).
    ok("01", FIXTURES / "F99_2011828.fec", expect=["form type:       F99"])


def test_02_count_lines_by_table() -> None:
    out = ok("02", FIXTURES / "F3XA_2011821.fec", expect=["26 body lines", "SA11AI          4"])
    assert "SchA          5      1" in out  # five lines, one memo


def test_03_schedule_a_to_dicts() -> None:
    ok("03", FIXTURES / "F3XN_2011831.fec",
       expect=["132 Schedule A record(s)", "Decimal('380.00')", "datetime.date(2026, 8, 31)",
               "amount is Decimal, date is date", "total of non-memo contributions: 13736.02"])


def test_04_top_donors() -> None:
    ok("04", FIXTURES / "F3A_2011812.fec", 3,
       expect=["58 distinct donor(s)", "MIZUSAWA, BERT / SELF", "equal: True"])


def test_05_lenient_parsing() -> None:
    ok("05", FIXTURES / "F3XA_2011827.fec",
       expect=["FecError at line 9", "2 skipped", "unknown form type",
               "Unrecognized Form Type", "identical to the original file's: True"])


def test_06_handle_fec_error() -> None:
    ok("06", FIXTURES / "F3XA_2011827.fec",
       expect=["ok: F3XA with 6 body line(s)", "FecError: no format table for form type 'BOGUS'",
               "line 4: BOGUS|C00944124|X", "(not about one line)", "FileNotFoundError", "caught as ValueError: FecError"])


def test_07_memo_entries() -> None:
    ok("07", FIXTURES / "F3XN_2011834.fec",
       expect=["3 memo(s)", "sum including memos: 20723.50", "sum excluding memos: 10170.37", "match"])


def test_08_old_spec_versions() -> None:
    ok("08", expect=["F3XA spec 3.0", "'name_delim'", "column 15 in 3.0, column 20 in 8.5",
                     "spec 3.0: itemized sum 149408.52 vs cover 11(a)(i) 149408.52 -> True"])


def test_09_stream_large_filing(tmp_path: Path) -> None:
    # The real target is a 135 MB file that is not in the repository; the
    # example must exit cleanly without it and work on any filing.
    ok("09", tmp_path / "absent.fec", expect=["not present"])
    ok("09", FIXTURES / "F3XN_2011831.fec", expect=["132 Schedule A line(s)", "SA11AI", "equal"])


# --- validating --------------------------------------------------------------

def test_10_validate_by_severity() -> None:
    result = run("10", INVALID / "bad_dates_and_amounts.fec")
    assert result.returncode == 1  # errors -> exit 1
    assert "5 error(s), 0 warning(s); would be REJECTED" in result.stdout
    assert "not_a_real_date x2" in result.stdout
    ok("10", FIXTURES / "F3XN_2011831.fec", expect=["acceptable (no errors)"])


def test_11_validate_directory_csv(tmp_path: Path) -> None:
    out = ok("11", FIXTURES, expect=["file,form_type,version,severity,rule,line_no,record,field,message",
                                     "duplicate_tran_id.fec,F3XA,8.5,error,duplicate_transaction_id,5,SB21B"])
    assert out.count("\n") > 100
    report = tmp_path / "report.csv"
    ok("11", FIXTURES, report, expect=["error(s)", str(report)])
    assert report.read_text().startswith("file,form_type")


def test_12_validate_exit_code() -> None:
    ok("12", FIXTURES / "F3XA_2011827.fec", FIXTURES / "F3XN_2011831.fec", expect=["2 of 2 file(s) pass"])
    result = run("12", FIXTURES / "F3XA_2011827.fec", INVALID / "duplicate_tran_id.fec")
    assert result.returncode == 1
    assert "FAIL" in result.stdout and "1 of 2 file(s) pass" in result.stdout
    # --strict fails on warnings too (an 8.0 filing warns about its format).
    result = run("12", "--strict", FIXTURES / "F3A_767339_v8.0.fec")
    assert result.returncode == 1 and "(strict)" in result.stdout


def test_13_explain_finding() -> None:
    ok("13", INVALID / "field_too_long.fec",
       expect=["[field_too_long]", "description: CONTRIBUTOR LAST NAME", "max length 30, required: error",
               "FEC rule:    Required if [IND|CAN]"])


# --- reconciling -------------------------------------------------------------

def test_14_reconcile_mismatches() -> None:
    ok("14", FIXTURES / "F3XN_2011831.fec",
       expect=["69 check(s), 0 mismatch(es); balances: True", "2 mismatch(es); balances: False",
               "11(a)(i)         13836.02       13736.02       100.00", "11(a)(iii)"])
    ok("14", FIXTURES / "F99_2011828.fec", expect=["no reconciliation rules for form F99"])


def test_15_relations_and_threshold() -> None:
    ok("15", FIXTURES / "F3XA_2011821.fec",
       expect=["at_least reported    27326.50 expected    27247.50 delta     79.00  ok",
               "$200 aggregate threshold", "VIOLATION 100.00"])


def test_16_recompute_a_line() -> None:
    ok("16", FIXTURES / "F3A_2011812.fec",
       expect=["96 counted, 41 memo (skipped)", "by-hand sum of non-memo contribution_amount: 44776.25",
               "(by hand: 44776.25, equal: True)", "(by hand: 96, equal: True)"])


def test_17_reconcile_batch() -> None:
    # Fixture counts change as fixtures are added; assert the shape, not the number.
    result = run("17", FIXTURES)
    assert result.returncode == 0, result.stdout + result.stderr
    m = re.search(r"(\d+) reconciled \((\d+) balance\)", result.stdout)
    assert m and m.group(1) == m.group(2), result.stdout
    assert "no line disagrees in any filing" in result.stdout


# --- editing and writing -----------------------------------------------------

def test_18_fix_field_and_write(tmp_path: Path) -> None:
    out = tmp_path / "fixed.fec"
    ok("18", INVALID / "field_too_long.fec", out,
       expect=["2 error(s) before", "31 -> 30 chars (max 30)", "0 error(s) after; acceptable: True", "re-parsed: F3XA"])
    assert hardmoney.parse_file(out).validate().is_acceptable


def test_19_bulk_edit_states(tmp_path: Path) -> None:
    out = tmp_path / "states.fec"
    ok("19", expect=["demo input", "'ky' -> 'KY'", "10 field(s) changed", "0 lower-case state(s) left"])
    # On a real, clean fixture nothing changes and the output still parses.
    ok("19", FIXTURES / "F3XN_2011835.fec", out, expect=["0 field(s) changed"])
    assert hardmoney.parse_file(out).form_type == "F3XN"


def test_20_round_trip_check() -> None:
    result = run("20")
    assert result.returncode == 0, result.stdout + result.stderr
    m = re.search(r"(\d+) of (\d+) filing\(s\) round-trip field for field", result.stdout)
    assert m and m.group(1) == m.group(2) and int(m.group(1)) >= 25, result.stdout
    ok("20", FIXTURES / "F3XA_27789_v3.fec", expect=["ok   F3XA_27789_v3.fec", "1 of 1"])


def test_21_build_minimal_filing(tmp_path: Path) -> None:
    out = tmp_path / "built.fec"
    ok("21", out, expect=["built F3XN v8.5 with 2 body line(s)", "cover 11(a)(i) = 1250.00",
                          "0 error(s), 0 warning(s); acceptable: True", "balances True"])
    built = hardmoney.parse_file(out)
    assert built.reconcile().balances and built.validate().is_acceptable


# --- spec --------------------------------------------------------------------

def test_22_spec_tables_and_layouts() -> None:
    ok("22", expect=["bundled spec version: 8.5", "59 tables", "removed in 8.5: contribution_purpose_code (col 22)",
                     "no column layout for table SchA at spec version 9.9"])


def test_23_data_dictionary() -> None:
    out = ok("23", "SchA", expect=["# SchA (FEC spec 8.5)", "| 20 | `contribution_amount` | amount(12) |",
                                   "45 of 45 fields documented"])
    assert out.count("\n|") >= 46  # header separator + 45 rows
    ok("23", "F3X", expect=["# F3X (FEC spec 8.5)"])


# --- interop -----------------------------------------------------------------

@pytest.mark.skipif(not _has("pandas"), reason="pandas not installed")
def test_24_pandas_dataframe() -> None:
    ok("24", FIXTURES / "F3XN_2011831.fec",
       expect=["amount dtype: object", "sum of non-memo amounts: Decimal('13736.02')", "equal: True",
               "which is really 13736.02000000000043655745685100555419921875"])


@pytest.mark.skipif(not _has("pyarrow"), reason="pyarrow not installed")
@pytest.mark.skipif(_cli() is None, reason="hardmoney CLI not built")
def test_25_parquet_via_cli(tmp_path: Path) -> None:
    out = ok("25", FIXTURES / "F3XA_2011821.fec", tmp_path / "pq",
             expect=["decimal128(38, 2)", "date32[day]", "pyarrow sum of expenditure_amount: Decimal('27247.50')"])
    if _has("duckdb"):
        assert "DuckDB SUM: Decimal('27247.50') as DECIMAL(38,2)" in out


@pytest.mark.skipif(not _has("polars"), reason="polars not installed")
def test_26_polars_dataframe() -> None:
    ok("26", FIXTURES / "F3A_2011812.fec",
       expect=["Decimal(precision=38, scale=2)", "44776.25 (Decimal)", "equal: True", "RETIRED"])


@pytest.mark.skipif(_cli() is None, reason="hardmoney CLI not built")
def test_27_sqlite_via_cli(tmp_path: Path) -> None:
    ok("27", FIXTURES / "F3XA_2011821.fec", tmp_path / "x.sqlite",
       expect=["tables: F3X, SchA, SchB, SchD, SchE, TEXT, filings", "exact total via decimal_sum: Decimal('27247.50')",
               "disbursements of at least 1000.00: 4"])


def test_28_json_dump(tmp_path: Path) -> None:
    ok("28", FIXTURES / "F3XA_2011827.fec", expect=['"amends_filing": 1991972', '"contribution_amount": "20000.00"',
                                                    "-> Decimal('20000.00')"])
    out = tmp_path / "f.json"
    ok("28", FIXTURES / "F3XA_2011827.fec", out, expect=["wrote"])
    doc = json.loads(out.read_text())
    assert doc["lines"][0]["typed"]["contribution_amount"] == "20000.00"


# --- FEC data online ---------------------------------------------------------

@pytest.mark.skipif(not NETWORK, reason="set HARDMONEY_NETWORK_TESTS=1 to run network examples")
def test_29_fetch_filing() -> None:
    ok("29", 2011831, expect=["filing 2011831: F3XN v8.5, 144 body line(s)", "reconcile: balances"])


@pytest.mark.skipif(not NETWORK, reason="set HARDMONEY_NETWORK_TESTS=1 to run network examples")
@pytest.mark.skipif(_cli() is None, reason="hardmoney CLI not built")
def test_30_filings_search_and_validate(tmp_path: Path) -> None:
    out = ok("30", "C00140855", 2, tmp_path / "dl", timeout=300)
    if "no openFEC API key" in out:
        pytest.skip("no openFEC API key")
    assert "2 filing(s) for C00140855" in out
    assert "balances" in out


@pytest.mark.skipif(not NETWORK, reason="set HARDMONEY_NETWORK_TESTS=1 to run network examples")
@pytest.mark.skipif(_cli() is None, reason="hardmoney CLI not built")
def test_31_efile_watch(tmp_path: Path) -> None:
    # A fresh cache dir so the run never touches the user's seen-list.
    out = ok("31", "F3X", 1, env={"HARDMONEY_CACHE_DIR": str(tmp_path)}, timeout=300)
    assert "new F3X filing(s) in the feed" in out
    assert (tmp_path / "efile-seen.txt").exists()


# --- integration patterns ----------------------------------------------------

def test_32_presubmission_check(tmp_path: Path) -> None:
    result = run("32", INVALID / "bad_dates_and_amounts.fec")
    assert result.returncode == 1
    report = json.loads(result.stdout)
    assert report["acceptable"] is False and len(report["errors"]) == 5
    assert report["reconciliation"]["balances"] is True
    ok("32", FIXTURES / "F3XN_2011831.fec", expect=['"acceptable": true', '"balances": true'])
    # A form without cover-page rules reports reconciliation as null.
    ok("32", FIXTURES / "F99_2011828.fec", expect=['"reconciliation": null'])
    # Unparseable input is a report, not an exception.
    garbage = tmp_path / "garbage.fec"
    garbage.write_bytes(b"not a filing")
    result = run("32", garbage)
    assert result.returncode == 1
    assert json.loads(result.stdout)["parsed"] is False


def test_33_http_validation_server() -> None:
    ok("33", INVALID / "duplicate_tran_id.fec", expect=["HTTP 422", '"rule": "duplicate_transaction_id"'])
    ok("33", FIXTURES / "F3XN_2011831.fec", expect=["HTTP 200", '"ok": true'])


def test_34_pytest_fixture_pattern() -> None:
    ok("34", FIXTURES / "F3XN_2011831.fec", FIXTURES / "F99_2011828.fec", expect=["2 of 2 file(s) acceptable and balanced"])
    result = run("34", INVALID / "bad_filer_id.fec")
    assert result.returncode == 1 and "FAIL bad_filer_id.fec" in result.stdout
    # And as a pytest module, over the fixtures.
    collected = subprocess.run(
        [sys.executable, "-m", "pytest", "-q", "-p", "no:cacheprovider", str(_example("34"))],
        capture_output=True, text=True, timeout=120, cwd=EXAMPLES.parent,
    )
    assert collected.returncode == 0, collected.stdout + collected.stderr
    assert re.search(r"\d+ passed", collected.stdout), collected.stdout


def test_35_compare_amendment() -> None:
    ok("35", FIXTURES / "F3XN_2011831.fec",
       expect=["is_amendment=True, amends_filing=2011831", "'13736.02' -> '13741.02'  (delta +5.00)",
               "1 removed, 1 added, 1 changed", "- PR12339386106229", "+ NEW0001", "amendment reconciles: True"])
    # Two real filings of the same committee compare too (different periods here).
    ok("35", FIXTURES / "F3A_2011812.fec", FIXTURES / "F3A_2011822.fec", expect=["transactions: 587 before, 87 after"])
