"""``hardmoney.pytest_plugin`` through pytest's own ``pytester`` fixture.

Each test writes a small test file into a temporary project and runs
pytest on it in-process. The plugin is normally loaded through the
``pytest11`` entry point the wheel declares; when the package under test
was installed without it, the runs pass ``-p hardmoney.pytest_plugin``.
"""

from __future__ import annotations

import importlib.metadata
from pathlib import Path

import pytest

from conftest import FIXTURES

pytest_plugins = "pytester"

GOLDEN = FIXTURES / "golden"


def _entry_point_installed() -> bool:
    eps = importlib.metadata.entry_points()
    group = eps.select(group="pytest11") if hasattr(eps, "select") else eps.get("pytest11", [])
    return any(ep.name == "hardmoney" for ep in group)


PLUGIN_ARGS: list[str] = [] if _entry_point_installed() else ["-p", "hardmoney.pytest_plugin"]


def run(pytester: pytest.Pytester, *args: str) -> pytest.RunResult:
    return pytester.runpytest(*PLUGIN_ARGS, *args)


def test_marker_is_registered(pytester: pytest.Pytester) -> None:
    result = run(pytester, "--markers")
    result.stdout.fnmatch_lines(["@pytest.mark.hardmoney: *"])


def test_assert_fec_acceptable_passes_and_fails_with_findings_listed(pytester: pytest.Pytester) -> None:
    pytester.makepyfile(
        f"""
        from pathlib import Path
        GOLDEN = Path({str(GOLDEN)!r})

        def test_clean(assert_fec_acceptable):
            filing = assert_fec_acceptable(GOLDEN / "f3x.fec")
            assert filing.form_type == "F3XN"

        def test_bytes_and_str(assert_fec_acceptable):
            data = (GOLDEN / "f3.fec").read_bytes()
            assert_fec_acceptable(data)
            assert_fec_acceptable(data.decode("utf-8"))

        def test_duplicate(assert_fec_acceptable):
            assert_fec_acceptable(GOLDEN / "f3x_duplicate_transaction_id.fec")

        def test_not_a_filing(assert_fec_acceptable):
            assert_fec_acceptable(b"garbage")
        """
    )
    result = run(pytester, "-v")
    result.assert_outcomes(passed=2, failed=2)
    result.stdout.fnmatch_lines(
        [
            "*f3x_duplicate_transaction_id.fec (F3XN v8.5) has 1 error(s):",
            "*ERROR line 4 SA11B transaction_id: Tran ID SA11AI.1 is NOT UNIQUE*",
            "*does not parse as a .fec*",
        ]
    )


def test_allow_warnings_false_fails_on_warnings(pytester: pytest.Pytester) -> None:
    pytester.makepyfile(
        f"""
        from pathlib import Path
        import hardmoney
        import pytest

        @pytest.fixture
        def with_a_warning():
            # The golden F3X with a recommended field blanked: one warning, no error.
            filing = hardmoney.parse_file(Path({str(GOLDEN / "f3x.fec")!r}))
            filing.lines[0].set("contributor_street_1", "")
            return filing.to_fec()

        def test_default(assert_fec_acceptable, with_a_warning):
            assert_fec_acceptable(with_a_warning)

        def test_strict(assert_fec_acceptable, with_a_warning):
            assert_fec_acceptable(with_a_warning, allow_warnings=False)
        """
    )
    result = run(pytester)
    result.assert_outcomes(passed=1, failed=1)
    result.stdout.fnmatch_lines(
        ["*the filing (F3XN v8.5) has 1 finding(s):*", "*WARN  line 3 SA11AI contributor_street_1: *"]
    )


def test_assert_fec_balances(pytester: pytest.Pytester) -> None:
    pytester.makepyfile(
        f"""
        from decimal import Decimal
        from pathlib import Path
        import pytest
        GOLDEN = Path({str(GOLDEN)!r})

        def test_balanced(assert_fec_balances):
            r = assert_fec_balances(GOLDEN / "f3x.fec")
            assert r.balances and r.form == "F3X"

        def test_off_by_one_cent_fails(assert_fec_balances):
            assert_fec_balances(GOLDEN / "f3x_cover_off_by_one_cent.fec")

        def test_off_by_one_cent_within_tolerance(assert_fec_balances):
            assert_fec_balances(GOLDEN / "f3x_cover_off_by_one_cent.fec", tolerance=Decimal("0.01"))
            assert_fec_balances(GOLDEN / "f3x_cover_off_by_one_cent.fec", tolerance="0.01")

        def test_nothing_to_reconcile(assert_fec_balances):
            assert assert_fec_balances(GOLDEN / "f24.fec") is None

        def test_float_tolerance_rejected(assert_fec_balances):
            with pytest.raises(TypeError):
                assert_fec_balances(GOLDEN / "f3x.fec", tolerance=0.01)
        """
    )
    result = run(pytester, "-v")
    result.assert_outcomes(passed=4, failed=1)
    result.stdout.fnmatch_lines(
        [
            "*2 of * cover line(s) disagree with the schedules (tolerance 0):",
            "*DIFF col A line 11(a)(i) *",
            "*DIFF col A line 11(a)(iii) *",
        ]
    )


def test_fec_filing_factory(pytester: pytest.Pytester) -> None:
    pytester.makepyfile(
        f"""
        from pathlib import Path
        import hardmoney
        GOLDEN = Path({str(GOLDEN)!r})

        def test_factory(fec_filing):
            filing = fec_filing(GOLDEN / "f99.fec")
            assert isinstance(filing, hardmoney.Filing)
            assert filing.form_type == "F99"
            assert fec_filing(filing.to_fec()).summary["text"] == filing.summary["text"]
        """
    )
    run(pytester).assert_outcomes(passed=1)


def test_fixtures_option_parametrises_over_every_fec_file(pytester: pytest.Pytester) -> None:
    pytester.makepyfile(
        """
        from pathlib import Path
        import pytest

        def test_each(fec_fixture, assert_fec_acceptable, assert_fec_balances):
            assert isinstance(fec_fixture, Path)
            if fec_fixture.name in {"f3x_duplicate_transaction_id.fec", "f3x_missing_required_field.fec"}:
                pytest.xfail("deliberately broken fixture")
            assert_fec_acceptable(fec_fixture)
            if "off_by_one_cent" not in fec_fixture.name:
                assert_fec_balances(fec_fixture)
        """
    )
    n = len(list(GOLDEN.glob("*.fec")))
    assert n == 9
    result = run(pytester, "-v", f"--hardmoney-fixtures={GOLDEN}")
    result.assert_outcomes(passed=n - 2, xfailed=2)
    result.stdout.fnmatch_lines(["*test_each[[]f3x.fec[]] PASSED*", "*test_each[[]f99.fec[]] PASSED*"])


def test_fixtures_option_absent_skips(pytester: pytest.Pytester) -> None:
    pytester.makepyfile(
        """
        def test_each(fec_fixture):
            raise AssertionError("must not run")
        """
    )
    result = run(pytester, "-rs")
    result.assert_outcomes(skipped=1)


def test_fixtures_option_rejects_a_missing_directory(pytester: pytest.Pytester) -> None:
    pytester.makepyfile(
        """
        def test_each(fec_fixture):
            pass
        """
    )
    result = run(pytester, "--hardmoney-fixtures=/nonexistent/dir")
    assert result.ret == pytest.ExitCode.USAGE_ERROR
    result.stderr.fnmatch_lines(["*--hardmoney-fixtures: /nonexistent/dir is not a directory*"])


def test_marker_is_applied_to_tests_using_the_fixtures(pytester: pytest.Pytester) -> None:
    pytester.makepyfile(
        f"""
        from pathlib import Path
        GOLDEN = Path({str(GOLDEN)!r})

        def test_uses_plugin(assert_fec_acceptable):
            assert_fec_acceptable(GOLDEN / "f1m.fec")

        def test_plain():
            pass
        """
    )
    only_marked = run(pytester, "-m", "hardmoney", "-v")
    only_marked.assert_outcomes(passed=1, deselected=1)
    only_marked.stdout.fnmatch_lines(["*test_uses_plugin PASSED*"])
    unmarked = run(pytester, "-m", "not hardmoney", "-v")
    unmarked.assert_outcomes(passed=1, deselected=1)
    unmarked.stdout.fnmatch_lines(["*test_plain PASSED*"])
