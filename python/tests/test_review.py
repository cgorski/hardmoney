"""Review: the RAD-style checks over real filings and the golden pack."""

from __future__ import annotations

from decimal import Decimal
from pathlib import Path

import pytest

import hardmoney
from conftest import FIXTURES, MINIMAL

GOLDEN = FIXTURES / "golden"
RAD = FIXTURES / "rad" / "F3XA_2011912.fec"

# The golden pack is built to be clean. Its Form 3X files carry one
# deliberate -1.00 `SA17` line from FECfile+'s own test transaction set,
# which the negative-amount rule reports; the two defect files add exactly
# their defect.
GOLDEN_EXPECTED = {
    "f3x.fec": {"negative_itemization": 1},
    "f3x_cover_off_by_one_cent.fec": {"cover_not_supported": 2, "negative_itemization": 1},
    "f3x_duplicate_transaction_id.fec": {
        "negative_itemization": 1,
        "duplicate_transaction_id": 1,
    },
    "f3x_missing_required_field.fec": {"negative_itemization": 1},
}


def _counts(review: hardmoney.Review) -> dict[str, int]:
    return {k: v["count"] for k, v in review.summary["by_concern"].items()}


def test_rad_fixture_reports_the_200_dollar_cover_gap() -> None:
    if not RAD.exists():
        pytest.skip(f"{RAD} missing")
    filing = hardmoney.parse_file(RAD)
    r = filing.review()
    assert isinstance(r, hardmoney.Review)
    assert bool(r) is True
    assert len(r) == len(r.observations) == r.summary["observations"] == 25

    cover = r.by_concern("cover_not_supported")
    assert len(cover) == 1
    o = cover[0]
    assert isinstance(o, hardmoney.Observation)
    assert o.concern == "cover_not_supported"
    assert o.line_no == 2
    assert o.transaction_id is None
    assert isinstance(o.amount, Decimal)
    assert o.amount == Decimal("200.00")
    assert "line 11(c)" in o.detail and "2045.00" in o.detail and "1845.00" in o.detail
    assert o.rfai_request_type == 2
    assert o.heuristic is False
    assert str(o).startswith("cover_not_supported line 2 200.00: column A line 11(c)")
    assert repr(o) == "<hardmoney.Observation cover_not_supported line 2>"

    assert r.has("cover_not_supported") is True
    assert r.has("chain_cash_mismatch") is False
    assert r.concerns == [
        "best_efforts_claimed",
        "cover_not_supported",
        "negative_itemization",
        "duplicate_transaction",
    ]
    assert _counts(r) == {
        "best_efforts_claimed": 16,
        "cover_not_supported": 1,
        "negative_itemization": 5,
        "duplicate_transaction": 3,
    }
    assert r.summary["by_concern"]["cover_not_supported"]["amount"] == Decimal("200.00")
    assert isinstance(r.summary["amount_at_issue"], Decimal)
    assert r.summary["amount_at_issue"] == sum(
        (abs(o.amount) for o in r if o.amount is not None), Decimal(0)
    )

    # Heuristic concerns are excluded from strict().
    strict = r.strict()
    assert {o.concern for o in strict} == {"cover_not_supported", "negative_itemization"}
    assert all(o.heuristic is False for o in strict)
    assert all(o.heuristic is True for o in r.by_concern("best_efforts_claimed"))
    assert r.by_concern("best_efforts_claimed")[0].rfai_request_type is None

    # Iteration is in line order; str() ends with the summary line.
    lines = [o.line_no for o in r]
    assert lines == sorted(lines)
    text = str(r)
    assert text.splitlines()[-1] == "25 observation(s) in 4 concern(s); amount at issue 25563.33"
    assert len(text.splitlines()) == 26
    assert repr(r) == "<hardmoney.Review 25 observation(s) in 4 concern(s)>"

    with pytest.raises(ValueError, match="not a review concern"):
        r.by_concern("not_a_concern")


@pytest.mark.parametrize("path", sorted(GOLDEN.glob("*.fec")), ids=lambda p: p.name)
def test_golden_pack_has_only_the_documented_observations(path: Path) -> None:
    r = hardmoney.parse_file(path).review()
    assert _counts(r) == GOLDEN_EXPECTED.get(path.name, {}), str(r)
    if path.name not in GOLDEN_EXPECTED:
        assert len(r) == 0
        assert bool(r) is False
        assert str(r) == "no observations"
        assert r.summary == {
            "observations": 0,
            "amount_at_issue": Decimal("0"),
            "by_concern": {},
        }
    for o in r.by_concern("negative_itemization"):
        assert o.transaction_id == "SA17.12"
        assert o.amount == Decimal("-1.00")


def test_every_fixture_reviews_without_raising(fixture_path: Path) -> None:
    filing = hardmoney.parse(fixture_path.read_bytes(), lenient=True)
    r = filing.review()
    for o in r:
        assert o.concern
        assert o.detail
        assert o.line_no is None or o.line_no >= 1
        assert o.amount is None or isinstance(o.amount, Decimal)
    # An accepted filing whose cover reconciles never gets the cover concern.
    if filing.base_form_type in {"F3X", "F3", "F3P"} and filing.reconcile().balances:
        assert r.has("cover_not_supported") is False, str(r)
    assert r.has("duplicate_transaction_id") is False


def test_recipient_selects_the_limit() -> None:
    # A single $6,000 gift to a Form 3X filer: no limit by default, 1,000
    # over as a PAC, nothing as a state party or an unlimited committee.
    filing = hardmoney.parse(MINIMAL)
    line = next(iter(filing.lines_for("SchA")))
    line.set("contribution_amount", "6000.00")
    line.set("contribution_aggregate", "6000.00")
    assert filing.review().has("over_limit_aggregate") is False
    pac = filing.review(recipient="pac")
    over = pac.by_concern("over_limit_aggregate")
    assert len(over) == 1
    assert over[0].amount == Decimal("1000.00")
    assert over[0].transaction_id == "A1"
    assert "per-calendar-year limit to a PAC" in over[0].detail
    assert filing.review(recipient="state-party").has("over_limit_aggregate") is False
    assert filing.review(recipient="unlimited").has("over_limit_aggregate") is False
    with pytest.raises(ValueError, match="recipient must be one of"):
        filing.review(recipient="charity")


def test_synthetic_employer_missing() -> None:
    filing = hardmoney.parse(MINIMAL)
    line = next(iter(filing.lines_for("SchA")))
    # $250 with a blank employer: the aggregate is over $200.
    r = filing.review()
    assert r.has("employer_occupation_missing") is True
    o = r.by_concern("employer_occupation_missing")[0]
    assert o.line_no == 3
    assert o.amount == Decimal("250.00")
    assert "employer and occupation field is blank" in o.detail
    # Filling both in clears it; "retired" alone is enough.
    line.set("contributor_employer", "Acme")
    line.set("contributor_occupation", "Engineer")
    assert filing.review().has("employer_occupation_missing") is False
    line.set("contributor_employer", "")
    line.set("contributor_occupation", "Retired")
    assert filing.review().has("employer_occupation_missing") is False
    # Best-efforts language is reported separately.
    line.set("contributor_occupation", "Information Requested")
    r = filing.review()
    assert r.has("employer_occupation_missing") is False
    assert r.has("best_efforts_claimed") is True
