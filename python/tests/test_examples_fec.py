"""The scripts in ``python/examples/fec/`` (the "For FEC staff" chapter) run
against the fixtures and print what their docstrings promise.

Each is run as a subprocess, as a reader would run it. Scripts that need
the ``hardmoney`` CLI skip when no build is found; those that need the
network (and an openFEC key) run only with ``HARDMONEY_NETWORK_TESTS=1``.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Optional, Sequence

import pytest

import hardmoney
from conftest import FIXTURES, INVALID

EXAMPLES = Path(__file__).resolve().parents[1] / "examples" / "fec"
REPO = Path(__file__).resolve().parents[2]
RAD = FIXTURES / "rad"
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


def _has_api_key() -> bool:
    return bool(os.environ.get("FEC_API_KEY")) or (Path.home() / "fec_api_key.txt").exists()


def _example(number: str) -> Path:
    [path] = EXAMPLES.glob(f"fec_{number}_*.py")
    return path


def run(number: str, *args: object, timeout: float = 300) -> subprocess.CompletedProcess[str]:
    return subprocess.run([sys.executable, str(_example(number)), *map(str, args)],
                          capture_output=True, text=True, timeout=timeout, cwd=EXAMPLES.parents[1])


def ok(number: str, *args: object, expect: Sequence[str] = (), **kwargs: object) -> str:
    result = run(number, *args, **kwargs)  # type: ignore[arg-type]
    assert result.returncode == 0, f"exit {result.returncode}\n{result.stdout}\n{result.stderr}"
    for text in expect:
        assert text in result.stdout, f"{text!r} not in output:\n{result.stdout}"
    return result.stdout


def test_every_fec_example_has_a_docstring_main_and_no_floats() -> None:
    scripts = sorted(EXAMPLES.glob("fec_*.py"))
    assert [int(p.name[4:6]) for p in scripts] == list(range(1, 8)), [p.name for p in scripts]
    for script in scripts:
        source = script.read_text(encoding="utf-8")
        assert source.startswith('"""FEC '), script.name
        for needle in ("def main(argv", 'if __name__ == "__main__":', "Run:", "import hardmoney"):
            assert needle in source, (script.name, needle)
        assert "float(" not in source, script.name  # money is Decimal (Line.amount) or text, never a float
        assert len(source.splitlines()) <= 100, f"{script.name} is meant to fit on a screen"


def test_rad_fixture_is_real_and_does_not_balance() -> None:
    filing = hardmoney.parse_file(RAD / "F3XA_2011912.fec")
    assert filing.form_type == "F3XA" and filing.amends_filing == 1986128
    assert filing.validate().is_acceptable
    [bad] = filing.reconcile().mismatches()
    assert (bad.column, bad.line, str(bad.delta)) == ("A", "11(c)", "200.00")


# --- Reports Analysis Division ------------------------------------------------

def test_fec_01_first_pass_review() -> None:
    ok("01", expect=["REPUBLICAN PARTY OF MINNESOTA - FEDERAL (C00001313)", "amends 1986128, amendment 1",
                     "0 error(s), 0 warning(s); the FEC would accept this file",
                     "1 line(s) disagree", "line 11(c)      reported      2045.00 expected      1845.00 delta +200.00",
                     "5 schedule line(s) carry this line number", "the cover page carries 200.00"])
    ok("01", FIXTURES / "F3XA_2011814.fec",
       expect=["4 warning(s)", "fields the FEC asks for that are blank: 4", "SA11B donor_committee_fec_id", "0 line(s) disagree"])
    ok("01", INVALID / "duplicate_tran_id.fec",
       expect=["the FEC would REJECT this file", "transaction id problems: 1", "SB21B.4120 is NOT UNIQUE"])
    ok("01", FIXTURES / "F99_2011828.fec", expect=["no reconciliation rules for form F99"])


def test_fec_02_amendment_diff() -> None:
    ok("02", expect=["original:  F3XN Q2 20260514..20260630, 24 line(s)", "amendment 1 of filing 1996410, 26 line(s)",
                     "col_a_debts_by", "21177.00 ->       36177.50  (+15000.50)", "other cover fields that changed: form_type, date_signed",
                     "transactions: 24 -> 26; 2 added, 0 removed, 8 edited", "+ PD138", "David Binder Research",
                     "~ PD76", "'5717.00' -> '5717.50'"])
    # Both real files reconcile.
    assert hardmoney.parse_file(FIXTURES / "F3XN_1996410.fec").reconcile().balances
    result = run("02", FIXTURES / "F3XN_1996410.fec", FIXTURES / "F3XA_2011821.fec")
    assert result.returncode == 0 and "2 added" in result.stdout


@pytest.mark.skipif(not NETWORK, reason="set HARDMONEY_NETWORK_TESTS=1 to run network examples")
def test_fec_02_amendment_diff_by_id_fetches_both() -> None:
    ok("02", 2011821, expect=["amendment 1 of filing 1996410", "+ PD138"])
    assert run("02", 1996410).returncode == 1  # not an amendment


# --- Electronic Filing Office -------------------------------------------------

def test_fec_03_vendor_conformance() -> None:
    out = ok("03", expect=["software", "FECfile 8.5.1.0(f34)", "findings by rule:", "error   duplicate_transaction_id",
                           "files the FEC would reject:", "duplicate_tran_id.fec", "wrong_schedule_for_form.fec"])
    assert "file(s) from" in out and "software product(s)" in out
    # Only clean files: nothing to reject.
    out = ok("03", FIXTURES / "F3XN_2011831.fec", FIXTURES / "F3XA_2011814.fec",
             expect=["2 file(s) from 2 software product(s)", "Campaign Manager 360 1.0", "conditionally_required_field_empty"])
    assert out.rstrip().endswith("files the FEC would reject:")


@pytest.mark.skipif(not NETWORK, reason="set HARDMONEY_NETWORK_TESTS=1 to run network examples")
@pytest.mark.skipif(_cli() is None, reason="hardmoney CLI not built")
def test_fec_03_vendor_conformance_against_webcheck() -> None:
    ok("03", "--oracle", INVALID / "duplicate_tran_id.fec", INVALID / "field_too_long.fec",
       expect=["webcheck 1 matched, 0 only ours, 0 only theirs", "webcheck 2 matched, 0 only ours, 0 only theirs"])


# --- FECfile+ developers ------------------------------------------------------

def test_fec_04_fecfile_plus_hook() -> None:
    report = json.loads(ok("04"))
    assert report["errors"] == [] and len(report["warnings"]) == 4
    assert report["warnings"][0]["path"] == "SA11B.donor_committee_fec_id"
    # The lines FECfile+'s summary.py stubs to zero, computed from Schedules H3 and H4.
    assert report["column_a"]["line_18a"] == "78207.70"
    assert report["column_a"]["line_21ai"] == "28833.61" and report["column_a"]["line_21aii"] == "54233.29"
    assert report["column_a"]["line_11ai"] == "47147.22" and report["cover_disagrees"] == []
    result = run("04", INVALID / "bad_dates_and_amounts.fec")
    assert result.returncode == 1 and len(json.loads(result.stdout)["errors"]) == 5
    report = json.loads(ok("04", RAD / "F3XA_2011912.fec"))
    assert report["cover_disagrees"] == [{"line": "11(c)", "column": "A", "reported": "2045.00", "expected": "1845.00",
                                          "rule": "= sum of SchA.contribution_amount on SA11C"}]
    assert json.loads(ok("04", FIXTURES / "F99_2011828.fec"))["column_a"] == {}


def test_fec_05_golden_fixtures(tmp_path: Path) -> None:
    ok("05", tmp_path, expect=["F3XN v8.5, 27 body line(s)", "0 error(s), 0 warning(s); reconcile balances: True (69 checks)",
                               "11(a)(i)    10000.23", "15           2125.79", "17           1000.00", "21(b)         150.00",
                               "24            151.00"])
    written = hardmoney.parse_file(tmp_path / "f3x_fecfile_plus_test_set.fec")
    assert written.validate().is_acceptable and written.reconcile().balances
    assert sum(l.is_memo for l in written.lines) == 2
    column_a = json.loads((tmp_path / "f3x_fecfile_plus_test_set.json").read_text())
    assert column_a["11(d)"] == "11000.22" and column_a["24"] == "151.00" and column_a["28(d)"] == "604.50"


# --- Data division ------------------------------------------------------------

@pytest.mark.skipif(not NETWORK, reason="set HARDMONEY_NETWORK_TESTS=1 to run network examples")
@pytest.mark.skipif(_cli() is None, reason="hardmoney CLI not built")
@pytest.mark.skipif(not _has_api_key(), reason="no openFEC API key")
def test_fec_06_processing_lag() -> None:
    out = ok("06", "C00140855", 2026, 3, expect=["C00140855: 3 most-recent F3X report(s) in the 2026 cycle", "SchA lines"])
    assert "itemization lag" in out or "pending" in out


# --- Audit and Inspector General ----------------------------------------------

def test_fec_07_cycle_reconcile() -> None:
    ok("07", expect=["C00922229  Abundant Future  (2 report(s))", "F3XA_2011821.fec", "21(b) +79.00",
                     "C00934265  Bert for Senate 2026  (2 report(s))", "17 +2369.90", "balances"])
    # The real, accepted report that does not balance shows its line.
    ok("07", RAD, expect=["REPUBLICAN PARTY OF MINNESOTA - FEDERAL  (1 report(s))", "11(c) +200.00"])


@pytest.mark.skipif(not NETWORK, reason="set HARDMONEY_NETWORK_TESTS=1 to run network examples")
@pytest.mark.skipif(_cli() is None, reason="hardmoney CLI not built")
@pytest.mark.skipif(not _has_api_key(), reason="no openFEC API key")
def test_fec_07_cycle_reconcile_fetches_a_committee() -> None:
    ok("07", "C00140855", 2026, expect=["C00140855  FirstEnergy Corp Political Action Committee", "report(s))"])
