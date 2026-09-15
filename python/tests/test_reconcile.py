"""Reconciliation: real cover pages balance; a doctored one does not."""

from __future__ import annotations

from decimal import Decimal
from pathlib import Path

import pytest

import hardmoney
from conftest import FIXTURES, MINIMAL

PERIODIC = {"F3X", "F3", "F3P"}


def test_f3xn_2011831_balances(f3xn_2011831: Path) -> None:
    filing = hardmoney.parse_file(f3xn_2011831)
    r = filing.reconcile()
    assert isinstance(r, hardmoney.Reconciliation)
    assert r.form == "F3X"
    assert r.balances is True
    assert r.mismatches() == []
    assert len(r) == len(r.checks) >= 30
    assert {c.column for c in r.checks} == {"A", "B"}

    itemized = r.line("A", "11(a)(i)")
    assert isinstance(itemized, hardmoney.LineCheck)
    assert itemized.line == "11(a)(i)"
    assert itemized.field == "col_a_individuals_itemized"
    assert itemized.column == "A"
    assert itemized.relation == "equal"
    assert itemized.matches is True
    assert itemized.rule.startswith("= sum of SchA.contribution_amount on SA11AI")
    assert itemized.lines_summed == len([l for l in filing.lines_for("SchA") if not l.is_memo])
    assert itemized.reported_unparseable is False

    # Exact decimals throughout, equal to what the cover page says.
    assert isinstance(itemized.reported, Decimal)
    assert isinstance(itemized.expected, Decimal)
    assert isinstance(itemized.delta, Decimal)
    assert isinstance(itemized.violation, Decimal)
    assert itemized.reported == Decimal(filing.summary["col_a_individuals_itemized"])
    assert itemized.expected == sum(
        l.amount("contribution_amount") for l in filing.lines_for("SchA") if not l.is_memo
    )
    assert itemized.delta == itemized.violation == 0

    # Column B is formulas only; lower-case column letters are accepted.
    assert r.line("b", "6(c)") is not None
    assert r.line("A", "99(z)") is None
    with pytest.raises(ValueError, match="column must be"):
        r.line("C", "11(a)(i)")

    text = str(r)
    assert text.splitlines()[-1] == "F3X: every line agrees with its rule"
    assert len(text.splitlines()) == len(r) + 1
    assert all(l.startswith("ok  ") for l in text.splitlines()[:-1])
    assert "0 mismatch(es)" in repr(r)


def test_every_periodic_fixture_reconciles_in_column_a(fixture_path: Path) -> None:
    filing = hardmoney.parse(fixture_path.read_bytes())
    if filing.base_form_type not in PERIODIC:
        with pytest.raises(hardmoney.UnsupportedForm):
            filing.reconcile()
        return
    r = filing.reconcile()
    bad = [str(c) for c in r.mismatches() if c.column == "A"]
    assert bad == [], "\n".join(bad)


def test_synthetic_mismatch_does_not_balance(f3xn_2011831: Path) -> None:
    filing = hardmoney.parse_file(f3xn_2011831)
    reported = Decimal(filing.summary["col_a_individuals_itemized"])
    filing.summary.set("col_a_individuals_itemized", str(reported + Decimal("100.00")))
    r = filing.reconcile()
    assert r.balances is False
    mismatched = {c.line for c in r.mismatches() if c.column == "A"}
    # The schedule sum no longer matches, and the formula 11(a)(iii) =
    # 11(a)(i) + 11(a)(ii) is evaluated over *reported* values, so it
    # breaks too -- which isolates the bad line.
    assert {"11(a)(i)", "11(a)(iii)"} <= mismatched
    c = r.line("A", "11(a)(i)")
    assert c is not None and c.matches is False
    assert c.delta == Decimal("100.00")
    assert c.violation == Decimal("100.00")
    assert str(c).startswith("DIFF col A line 11(a)(i)")
    assert str(r).splitlines()[-1].startswith("F3X: ")
    assert "disagree" in str(r).splitlines()[-1]


def test_floor_lines_tolerate_unitemized_amounts() -> None:
    # Operating expenditures need itemizing only above $200 aggregate, so
    # the cover may exceed the itemized sum without a mismatch.
    filing = hardmoney.parse(MINIMAL)
    filing.summary.set("col_a_other_federal_operating_expenditures", "50.00")
    r = filing.reconcile()
    opex = r.line("A", "21(b)")
    assert opex is not None
    assert opex.relation == "at_least"
    assert opex.reported == Decimal("50.00")
    assert opex.expected == 0
    assert opex.delta == Decimal("50.00")
    assert opex.matches is True
    assert opex.violation == 0


def test_blank_and_unparseable_reported_are_none() -> None:
    filing = hardmoney.parse(MINIMAL)
    r = filing.reconcile()
    c = r.line("A", "11(a)(i)")
    assert c is not None
    assert c.reported is None  # blank on the cover page
    assert c.reported_unparseable is False
    assert c.expected == Decimal("250.00")
    assert c.delta == Decimal("-250.00")
    assert c.matches is False
    assert str(c).split()[:2] == ["DIFF", "col"]
    assert "(blank)" in str(c)

    filing.summary.set("col_a_individuals_itemized", "$250.00")
    c = filing.reconcile().line("A", "11(a)(i)")
    assert c is not None
    assert c.reported is None
    assert c.reported_unparseable is True  # present but not an FEC amount


def test_unsupported_form_is_a_fec_error() -> None:
    f99 = hardmoney.parse_file(FIXTURES / "F99_2011828.fec")
    with pytest.raises(hardmoney.UnsupportedForm) as info:
        f99.reconcile()
    assert isinstance(info.value, hardmoney.FecError)
    assert isinstance(info.value, ValueError)
    assert info.value.line_no is None
    assert "F99" in str(info.value)
    assert "F3X, F3, F3P" in str(info.value)
