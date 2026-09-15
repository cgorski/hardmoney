"""A pytest plugin for projects that produce ``.fec`` files.

Installed with the ``hardmoney`` package and registered through the
``pytest11`` entry point, so it is active in any project that has
hardmoney installed; nothing to configure. It adds:

* the ``hardmoney`` marker, applied automatically to every test that uses
  one of the fixtures below, so ``pytest -m hardmoney`` or
  ``-m "not hardmoney"`` selects or skips them;
* ``fec_filing``, a factory: ``fec_filing(path_or_bytes)`` parses a
  filing;
* ``assert_fec_acceptable(path_or_bytes, *, allow_warnings=True)``: fails
  listing every finding the FEC would reject the filing for (or every
  finding at all with ``allow_warnings=False``);
* ``assert_fec_balances(path_or_bytes, tolerance=Decimal("0"))``: fails
  listing every cover-page line that disagrees with its schedules by more
  than ``tolerance``; forms without a rule table (F1, F24, F99, ...) pass,
  there being nothing to reconcile;
* ``fec_fixture``, parametrised over every ``.fec`` under the directory
  given with ``--hardmoney-fixtures=DIR`` (recursively; use the ``=``
  form, or pytest also reads the directory when inferring ``rootdir``).
  Without the option, tests that use it are skipped.

A Django test that composes a report::

    def test_composed_report_is_acceptable(assert_fec_acceptable, assert_fec_balances):
        dot_fec = compose_dot_fec(report.id)
        assert_fec_acceptable(dot_fec)
        assert_fec_balances(dot_fec)

and a sweep over the golden pack::

    pytest --hardmoney-fixtures=tests/fixtures/golden

    def test_golden(fec_fixture, assert_fec_acceptable):
        if "missing" in fec_fixture.name or "duplicate" in fec_fixture.name:
            pytest.xfail("deliberately broken fixture")
        assert_fec_acceptable(fec_fixture)

``path_or_bytes`` follows :data:`hardmoney.compat.fecfile_validate.FecData`:
``bytes`` or ``str`` is ``.fec`` content, an :class:`os.PathLike` is a file.
"""

from __future__ import annotations

from decimal import Decimal
from pathlib import Path
from typing import Callable, Optional, Protocol

import pytest

import hardmoney
from hardmoney.compat.fecfile_validate import FecData, load_filing

__all__ = [
    "AssertAcceptable",
    "AssertBalances",
    "MARKER",
    "OPTION",
]

MARKER = "hardmoney"
OPTION = "--hardmoney-fixtures"
_FIXTURE_NAMES = frozenset(
    {"fec_filing", "assert_fec_acceptable", "assert_fec_balances", "fec_fixture"}
)


class AssertAcceptable(Protocol):
    """The callable the ``assert_fec_acceptable`` fixture returns."""

    def __call__(self, data: FecData, *, allow_warnings: bool = True) -> hardmoney.Filing: ...


class AssertBalances(Protocol):
    """The callable the ``assert_fec_balances`` fixture returns."""

    def __call__(
        self, data: FecData, tolerance: Decimal = Decimal("0")
    ) -> Optional[hardmoney.Reconciliation]: ...


def pytest_addoption(parser: pytest.Parser) -> None:
    group = parser.getgroup("hardmoney", "FEC electronic filings")
    group.addoption(
        OPTION,
        action="store",
        default=None,
        metavar="DIR",
        help="parametrise the fec_fixture fixture over every .fec under DIR (recursively)",
    )


def pytest_configure(config: pytest.Config) -> None:
    root = config.getoption(OPTION)
    if root is not None and not Path(root).is_dir():
        raise pytest.UsageError(f"{OPTION}: {root} is not a directory")
    config.addinivalue_line(
        "markers",
        f"{MARKER}: tests that read .fec files through hardmoney (applied automatically "
        "to tests using the fec_filing, assert_fec_acceptable, assert_fec_balances, or "
        "fec_fixture fixtures)",
    )


def pytest_collection_modifyitems(items: list[pytest.Item]) -> None:
    for item in items:
        names = getattr(item, "fixturenames", ())
        if any(name in _FIXTURE_NAMES for name in names):
            item.add_marker(MARKER)


def _fixture_paths(config: pytest.Config) -> list[Path]:
    root = config.getoption(OPTION)
    if root is None:
        return []
    return sorted(p for p in Path(root).rglob("*.fec") if p.is_file())


def pytest_generate_tests(metafunc: pytest.Metafunc) -> None:
    if "fec_fixture" not in metafunc.fixturenames:
        return
    paths = _fixture_paths(metafunc.config)
    root = metafunc.config.getoption(OPTION)
    ids = [str(p.relative_to(root)) for p in paths] if root else []
    metafunc.parametrize("fec_fixture", paths, ids=ids)


@pytest.fixture
def fec_fixture(request: pytest.FixtureRequest) -> Path:
    """One ``.fec`` path per file under ``--hardmoney-fixtures DIR``."""
    return request.param


@pytest.fixture
def fec_filing() -> Callable[[FecData], hardmoney.Filing]:
    """Factory: ``fec_filing(path_or_bytes) -> hardmoney.Filing`` (strict parse)."""

    def factory(data: FecData) -> hardmoney.Filing:
        return load_filing(data)

    return factory


def _describe(data: FecData) -> str:
    return "the filing" if isinstance(data, (str, bytes, bytearray)) else str(data)


@pytest.fixture
def assert_fec_acceptable() -> AssertAcceptable:
    """``assert_fec_acceptable(path_or_bytes, *, allow_warnings=True) -> Filing``.

    Fails (``pytest.fail``) when the FEC would reject the filing, with one
    line per finding in ``hardmoney validate`` format; with
    ``allow_warnings=False`` any finding fails the test. A filing that does
    not parse fails with the parser's message. Returns the parsed filing.
    """

    def check(data: FecData, *, allow_warnings: bool = True) -> hardmoney.Filing:
        try:
            filing = load_filing(data, lenient=True)
        except hardmoney.FecError as e:
            pytest.fail(f"{_describe(data)} does not parse as a .fec: {e}", pytrace=False)
        v = filing.validate()
        bad = v.errors if allow_warnings else v.findings
        if bad:
            kind = "error(s)" if allow_warnings else "finding(s)"
            listing = "\n".join(f"  {f}" for f in bad)
            pytest.fail(
                f"{_describe(data)} ({filing.form_type} v{filing.version}) has "
                f"{len(bad)} {kind}:\n{listing}",
                pytrace=False,
            )
        return filing

    return check


@pytest.fixture
def assert_fec_balances() -> AssertBalances:
    """``assert_fec_balances(path_or_bytes, tolerance=Decimal("0")) -> Reconciliation | None``.

    Fails listing every cover-page line whose violation (``|delta|`` for
    an equality, the shortfall for a floor) exceeds ``tolerance``. Returns
    the reconciliation, or ``None`` for a form with no rule table.
    """

    def check(data: FecData, tolerance: Decimal = Decimal("0")) -> Optional[hardmoney.Reconciliation]:
        if isinstance(tolerance, float):
            raise TypeError("tolerance must be a Decimal or str, not a float")
        tolerance = Decimal(tolerance)
        try:
            filing = load_filing(data)
        except hardmoney.FecError as e:
            pytest.fail(f"{_describe(data)} does not parse as a .fec: {e}", pytrace=False)
        try:
            r = filing.reconcile()
        except hardmoney.UnsupportedForm:
            return None
        bad = [c for c in r.checks if c.violation > tolerance]
        if bad:
            listing = "\n".join(f"  {c}" for c in bad)
            pytest.fail(
                f"{_describe(data)} ({filing.form_type}): {len(bad)} of {len(r)} cover line(s) "
                f"disagree with the schedules (tolerance {tolerance}):\n{listing}",
                pytrace=False,
            )
        return r

    return check
