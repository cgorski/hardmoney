"""Parsing: every real fixture, both entry points, field access, errors."""

from __future__ import annotations

import datetime
from decimal import Decimal
from pathlib import Path

import pytest

import hardmoney
from conftest import FS, MINIMAL


def test_every_fixture_parses_strictly(fixture_path: Path) -> None:
    filing = hardmoney.parse(fixture_path.read_bytes())
    assert isinstance(filing, hardmoney.Filing)
    assert filing.form_type.startswith("F")
    assert filing.base_form_type
    assert filing.form_type.startswith(filing.base_form_type)
    assert filing.version and filing.version[0].isdigit()
    assert filing.skipped == []
    assert filing.summary.line_no == 2
    assert filing.summary.table == filing.base_form_type
    # Body lines are numbered in file order, after the cover line.
    line_nos = [line.line_no for line in filing.lines]
    assert line_nos == sorted(line_nos)
    assert all(n > 2 for n in line_nos)


def test_parse_file_matches_parse(fixture_path: Path) -> None:
    from_bytes = hardmoney.parse(fixture_path.read_bytes())
    from_path = hardmoney.parse_file(fixture_path)
    assert from_path.header == from_bytes.header
    assert from_path.version == from_bytes.version
    assert from_path.summary.to_dict() == from_bytes.summary.to_dict()
    assert [l.to_dict() for l in from_path.lines] == [l.to_dict() for l in from_bytes.lines]
    # str paths and pathlib paths are both accepted.
    assert hardmoney.parse_file(str(fixture_path)).form_type == from_bytes.form_type


def test_parse_accepts_str_and_bytearray() -> None:
    a = hardmoney.parse(MINIMAL)
    b = hardmoney.parse(MINIMAL.encode())
    c = hardmoney.parse(bytearray(MINIMAL.encode()))
    assert a.to_fec() == b.to_fec() == c.to_fec()


def test_parse_rejects_other_types() -> None:
    with pytest.raises(TypeError, match="bytes or str"):
        hardmoney.parse(123)  # type: ignore[arg-type]


def test_filing_metadata(f3xn_2011831: Path) -> None:
    filing = hardmoney.parse_file(f3xn_2011831)
    assert filing.form_type == "F3XN"
    assert filing.base_form_type == "F3X"
    assert filing.version == "8.5"
    assert filing.is_amendment is False
    assert filing.amends_filing is None
    header = filing.header
    assert header["record_type"] == "HDR"
    assert header["ef_type"] == "FEC"
    assert header["fec_version_raw"] == "8.5"
    assert header["version"] == "8.5"
    assert header["soft_name"]
    # A 6.x+ header has no name_delim column.
    assert "name_delim" not in header
    assert set(header) == {
        "record_type",
        "ef_type",
        "fec_version_raw",
        "version",
        "soft_name",
        "soft_ver",
        "report_id",
        "report_number",
        "comment",
    }
    assert "F3XN" in repr(filing)


def test_amendment_recovers_original_filing_number() -> None:
    path = Path(__file__).resolve().parents[2] / "tests" / "fixtures" / "F3XA_2011827.fec"
    filing = hardmoney.parse_file(path)
    assert filing.is_amendment is True
    assert filing.amends_filing == 1991972
    assert filing.header["report_id"] == "FEC-1991972"


def test_old_format_header_carries_name_delim() -> None:
    path = Path(__file__).resolve().parents[2] / "tests" / "fixtures" / "F3XA_27789_v3.fec"
    filing = hardmoney.parse_file(path)
    assert filing.version == "3.0"
    assert "name_delim" in filing.header


def test_schedule_a_amount_is_exact_decimal(f3xn_2011831: Path) -> None:
    filing = hardmoney.parse_file(f3xn_2011831)
    sched_a = filing.lines_for("SchA")
    assert sched_a, "fixture has Schedule A lines"
    for line in sched_a:
        assert line.table == "SchA"
        amount = line.amount("contribution_amount")
        assert isinstance(amount, Decimal)
        assert amount == Decimal(line["contribution_amount"])
        # Scale 2 always, whatever the filer wrote.
        assert amount.as_tuple().exponent == -2
    total = sum(l.amount("contribution_amount") for l in sched_a if not l.is_memo)
    assert total == Decimal(filing.summary["col_a_individuals_itemized"])


def test_date_is_datetime_date(f3xn_2011831: Path) -> None:
    filing = hardmoney.parse_file(f3xn_2011831)
    cover = filing.summary
    assert cover.date("coverage_from_date") == datetime.date(2026, 8, 1)
    assert isinstance(cover.date("coverage_through_date"), datetime.date)
    line = filing.lines_for("SchA")[0]
    d = line.date("contribution_date")
    assert isinstance(d, datetime.date)
    assert d.strftime("%Y%m%d") == line["contribution_date"]


def test_blank_amount_and_date_are_none() -> None:
    filing = hardmoney.parse(MINIMAL)
    line = filing.lines[0]
    assert line["contribution_aggregate"] == ""
    assert line.amount("contribution_aggregate") is None
    assert line.date("contribution_aggregate") is None
    # Not a valid amount / date: None, not an exception.
    line.set("contribution_amount", "$5,500.00")
    assert line.amount("contribution_amount") is None
    line.set("contribution_date", "20261301")
    assert line.date("contribution_date") is None


def test_line_mapping_protocol() -> None:
    filing = hardmoney.parse(MINIMAL)
    line = filing.lines[0]
    assert line.form_type == "SA11AI"
    assert line.table == "SchA"
    assert line.line_no == 3
    assert line.is_memo is False
    assert line["contributor_last_name"] == "Smith"
    assert "contributor_last_name" in line
    assert "no_such_field" not in line
    assert line.get("no_such_field") is None
    assert line.get("no_such_field", "dflt") == "dflt"
    # A blank field is "", not the default.
    assert line.get("contributor_middle_name", "dflt") == ""
    keys = line.keys()
    assert keys[0] == "form_type"
    assert keys == [k for k, _ in line.items()]
    assert line.to_dict() == dict(line.items())
    assert list(line.to_dict()) == keys
    with pytest.raises(KeyError):
        line["no_such_field"]
    with pytest.raises(KeyError):
        line.amount("no_such_field")
    with pytest.raises(KeyError):
        line.date("no_such_field")
    assert "SA11AI" in repr(line)


def test_lines_for_and_iter_lines(f3xn_2011831: Path) -> None:
    filing = hardmoney.parse_file(f3xn_2011831)
    tables = {line.table for line in filing.lines}
    assert "SchA" in tables
    by_table = {t: filing.lines_for(t) for t in tables}
    assert sum(len(v) for v in by_table.values()) == len(filing.lines)
    assert [l.line_no for l in filing.iter_lines()] == [l.line_no for l in filing.lines]
    only_a = list(filing.iter_lines(["SchA"]))
    assert [l.line_no for l in only_a] == [l.line_no for l in by_table["SchA"]]
    # Table names are case-insensitive, like the Rust `Table: FromStr`.
    assert len(filing.lines_for("scha")) == len(by_table["SchA"])
    assert filing.lines_for("SchE") == []
    with pytest.raises(ValueError, match="unknown table"):
        filing.lines_for("SchZ")
    with pytest.raises(ValueError, match="unknown table"):
        list(filing.iter_lines(["SchZ"]))


def test_memo_flag_is_case_insensitive() -> None:
    filing = hardmoney.parse(MINIMAL)
    line = filing.lines[0]
    line.set("memo_code", "x")
    assert line.is_memo is True
    line.set("memo_code", "")
    assert line.is_memo is False


def test_garbage_raises_fec_error_with_line_no() -> None:
    with pytest.raises(hardmoney.FecError) as info:
        hardmoney.parse(b"garbage")
    assert isinstance(info.value, ValueError)
    assert info.value.line_no is None  # not about one line

    unknown_line = MINIMAL + f"ZZZ{FS}C00123456\n"
    with pytest.raises(hardmoney.FecError) as info:
        hardmoney.parse(unknown_line)
    assert info.value.line_no == 4
    assert "ZZZ" in str(info.value)
    assert "line 4" in str(info.value)

    with pytest.raises(hardmoney.FecError, match="deprecated header"):
        hardmoney.parse(b"/* old */")

    # A FecError constructed from Python still has the attribute.
    assert hardmoney.FecError("x").line_no is None


def test_lenient_parse_records_skipped_lines() -> None:
    text = MINIMAL + f"ZZZ{FS}C00123456\nSB21B{FS}C00123456{FS}B1\n"
    filing = hardmoney.parse(text, lenient=True)
    assert [l.form_type for l in filing.lines] == ["SA11AI", "SB21B"]
    assert filing.skipped == [{"line_no": 4, "form_type": "ZZZ", "reason": "unknown form type"}]
    # Skipped lines surface as warnings in validate(), not as errors.
    v = filing.validate()
    rules = [f.rule for f in v.warnings]
    assert "unrecognized_form_type" in rules
    assert any(f.line_no == 4 and f.form_type == "ZZZ" for f in v.warnings)


def test_missing_file_is_an_os_error(tmp_path: Path) -> None:
    with pytest.raises(FileNotFoundError):
        hardmoney.parse_file(tmp_path / "nope.fec")


def test_malformed_file_is_a_fec_error(tmp_path: Path) -> None:
    bad = tmp_path / "bad.fec"
    bad.write_bytes(b"HDR\x1cFEC\x1c8.5\x1cX\x1c1\n")
    with pytest.raises(hardmoney.FecError, match="no cover/summary line"):
        hardmoney.parse_file(bad)
