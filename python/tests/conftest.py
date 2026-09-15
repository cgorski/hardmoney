"""Shared fixtures: the real filings in the repository's ``tests/fixtures/``.

Every one of them was accepted by the FEC (spec 3.00 through 8.5), so
they are the truth the bindings are tested against.
"""

from __future__ import annotations

from pathlib import Path

import pytest

FIXTURES = Path(__file__).resolve().parents[2] / "tests" / "fixtures"
INVALID = FIXTURES / "invalid"

# Every real, FEC-accepted filing (the top level of the directory only;
# `invalid/` holds deliberately broken copies and `openfec/` JSON).
FIXTURE_PATHS = sorted(FIXTURES.glob("*.fec"))
INVALID_PATHS = sorted(INVALID.glob("*.fec"))

# A minimal spec-8.5 Form 3X with one Schedule A line, for synthetic
# cases. Complete enough to pass `validate()` with no errors: the cover
# carries report code, coverage dates, treasurer, and date signed (F3X
# columns 9, 13-14, 16-17, 21), and 11(a)(i) matches the one $250 line.
FS = "\x1c"
MINIMAL = (
    f"HDR{FS}FEC{FS}8.5{FS}Vendor{FS}1.0{FS}{FS}{FS}\n"
    f"F3XN{FS}C00123456{FS}Example PAC{FS}{FS}PO BOX 1{FS}{FS}ALEXANDRIA{FS}VA{FS}22313"
    f"{FS}Q1{FS}{FS}{FS}{FS}20260101{FS}20260331{FS}{FS}Doe{FS}Pat{FS}{FS}{FS}{FS}20260415\n"
    f"SA11AI{FS}C00123456{FS}A1{FS}{FS}{FS}IND{FS}{FS}Smith{FS}Jane"
    f"{FS}{FS}{FS}{FS}1 Main St{FS}{FS}Springfield{FS}VA{FS}22150{FS}G2026{FS}{FS}20260315{FS}250.00\n"
)


def _require_fixtures() -> None:
    if not FIXTURE_PATHS:
        pytest.skip(f"no fixtures found under {FIXTURES}")


@pytest.fixture(scope="session")
def fixture_paths() -> list[Path]:
    _require_fixtures()
    return FIXTURE_PATHS


@pytest.fixture(params=FIXTURE_PATHS, ids=lambda p: p.name)
def fixture_path(request: pytest.FixtureRequest) -> Path:
    """Parametrised over every real fixture."""
    return request.param


@pytest.fixture
def f3xn_2011831() -> Path:
    path = FIXTURES / "F3XN_2011831.fec"
    if not path.exists():
        pytest.skip(f"{path} missing")
    return path
