"""34. A pytest pattern for projects that generate ``.fec`` files.

Shows: how a filing-software project asserts that every ``.fec`` its
build produces would be accepted by the FEC, with the failure message
listing the findings. Copy the fixture and the two tests into your own
``conftest.py`` / ``test_fec_output.py`` and point ``FEC_OUTPUT_DIR`` at
the directory your code writes to.

As a pytest module::

    FEC_OUTPUT_DIR=build/fec pytest examples/34_pytest_fixture_pattern.py

Without the variable it runs over the repository's fixtures. As a plain
script it applies the same assertions to the paths given and reports
pass/fail without pytest.

Run:
    python examples/34_pytest_fixture_pattern.py [file.fec ...]
"""

from __future__ import annotations

import os
import sys
from pathlib import Path
from typing import Sequence

import pytest

import hardmoney

REPO = Path(__file__).resolve().parents[2]
OUTPUT_DIR = Path(os.environ.get("FEC_OUTPUT_DIR", REPO / "tests" / "fixtures"))
FEC_FILES = sorted(OUTPUT_DIR.glob("*.fec"))


# --- the reusable part -----------------------------------------------------

def assert_validates_clean(filing: hardmoney.Filing, *, warnings_ok: bool = True) -> None:
    """Fail with every finding listed when the FEC would reject the filing."""
    v = filing.validate()
    bad = v.errors if warnings_ok else v.findings
    assert not bad, f"{len(bad)} finding(s):\n" + "\n".join(str(f) for f in bad)


def assert_cover_page_balances(filing: hardmoney.Filing) -> None:
    """Fail listing the cover lines that disagree with the schedules."""
    try:
        r = filing.reconcile()
    except hardmoney.UnsupportedForm:
        return  # F1, F2, F24, F99, ...: nothing to reconcile
    assert r.balances, "\n".join(str(c) for c in r.mismatches())


@pytest.fixture(params=FEC_FILES, ids=lambda p: p.name)
def generated_filing(request: pytest.FixtureRequest) -> hardmoney.Filing:
    """One parsed filing per .fec file the build produced."""
    return hardmoney.parse_file(request.param)


def test_generated_filing_is_acceptable(generated_filing: hardmoney.Filing) -> None:
    assert_validates_clean(generated_filing)


def test_generated_cover_page_balances(generated_filing: hardmoney.Filing) -> None:
    assert_cover_page_balances(generated_filing)


# --- script mode -----------------------------------------------------------

def main(argv: Sequence[str]) -> int:
    paths = [Path(a) for a in argv] or FEC_FILES
    failures = 0
    for path in paths:
        filing = hardmoney.parse_file(path)
        try:
            assert_validates_clean(filing)
            assert_cover_page_balances(filing)
        except AssertionError as e:
            failures += 1
            print(f"FAIL {path.name}\n     " + str(e).replace("\n", "\n     "))
        else:
            print(f"ok   {path.name}")
    print(f"\n{len(paths) - failures} of {len(paths)} file(s) acceptable and balanced")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
