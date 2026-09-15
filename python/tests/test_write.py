"""Writing: ``to_fec`` round-trips every fixture; edits are written back."""

from __future__ import annotations

from pathlib import Path

import pytest

import hardmoney
from conftest import MINIMAL


def _records(filing: hardmoney.Filing) -> list[tuple[str, str, dict[str, str]]]:
    return [(l.table, l.form_type, l.to_dict()) for l in filing.lines]


def test_to_fec_reparses_equal(fixture_path: Path) -> None:
    filing = hardmoney.parse(fixture_path.read_bytes())
    out = filing.to_fec()
    assert isinstance(out, bytes)
    again = hardmoney.parse(out)
    assert again.header == filing.header
    assert again.version == filing.version
    assert again.form_type == filing.form_type
    assert again.summary.to_dict() == filing.summary.to_dict()
    assert _records(again) == _records(filing)
    # Canonical form is idempotent.
    assert again.to_fec() == out
    # And the text form is the decoded bytes: CRLF, one record per line.
    text = filing.to_fec_string()
    assert text.endswith("\r\n")
    assert text.count("\r\n") >= 2 + len(filing.lines)
    assert hardmoney.parse(text).to_fec() == out


def test_set_is_visible_to_to_fec() -> None:
    filing = hardmoney.parse(MINIMAL)
    line = filing.lines[0]
    line.set("contributor_employer", "  Self-employed  ")
    # Normalised like the parser (trimmed), visible through every handle.
    assert line["contributor_employer"] == "Self-employed"
    assert filing.lines[0]["contributor_employer"] == "Self-employed"
    again = hardmoney.parse(filing.to_fec())
    assert again.lines[0]["contributor_employer"] == "Self-employed"
    assert again.lines[0].to_dict() == line.to_dict()


def test_set_form_type_updates_token() -> None:
    filing = hardmoney.parse(MINIMAL)
    line = filing.lines[0]
    line.set("form_type", "sa11ai")
    assert line["form_type"] == "sa11ai"  # as filed
    assert line.form_type == "SA11AI"  # interpreted
    assert hardmoney.parse(filing.to_fec()).lines[0].form_type == "SA11AI"


def test_set_unknown_field_is_key_error() -> None:
    filing = hardmoney.parse(MINIMAL)
    with pytest.raises(KeyError) as info:
        filing.lines[0].set("no_such_field", "x")
    assert "no_such_field" in str(info.value)
    with pytest.raises(KeyError):
        filing.summary.set("contribution_amount", "1")  # a SchA field, not F3X


def test_cover_edits_round_trip(f3xn_2011831: Path) -> None:
    filing = hardmoney.parse_file(f3xn_2011831)
    filing.summary.set("committee_name", "Renamed Committee")
    again = hardmoney.parse(filing.to_fec())
    assert again.summary["committee_name"] == "Renamed Committee"
    assert _records(again) == _records(filing)


def test_windows_1252_when_representable_else_utf8() -> None:
    filing = hardmoney.parse(MINIMAL)
    filing.summary.set("committee_name", "Jos\u00e9 Mart\u00ednez for Congress")
    out = filing.to_fec()
    assert b"Jos\xe9" in out  # one byte per character
    assert hardmoney.parse(out).summary["committee_name"] == "Jos\u00e9 Mart\u00ednez for Congress"
    filing.summary.set("committee_name", "Jos\u0119")  # outside Windows-1252
    out = filing.to_fec()
    out.decode("utf-8")
    assert hardmoney.parse(out).summary["committee_name"] == "Jos\u0119"
