"""Spec helpers: tables, per-version layouts, and the FEC's field specs."""

from __future__ import annotations

import pytest

import hardmoney


def test_bundled_spec_version() -> None:
    assert hardmoney.BUNDLED_SPEC_VERSION == "8.5"
    assert hardmoney.__version__ == "2.0.0"


def test_tables() -> None:
    tables = hardmoney.tables()
    assert isinstance(tables, list)
    assert len(tables) >= 50
    assert tables == sorted(tables)
    for expected in ("F3X", "F3", "F3P", "SchA", "SchB", "SchE", "TEXT", "HDR"):
        assert expected in tables


def test_layout_columns_are_the_wire_positions() -> None:
    # Verified against the generated tables: `contribution_amount` is
    # column 20 in the 8.x Schedule A layout and 15 in the 5.x one.
    layout = hardmoney.layout("SchA", "8.5")
    assert isinstance(layout, list)
    assert ("contribution_amount", 20) in layout
    assert layout[0] == ("form_type", 0)
    assert ("filer_committee_id_number", 1) in layout
    columns = [c for _, c in layout]
    assert len(set(columns)) == len(columns), "one field per column"
    assert ("contribution_amount", 15) in hardmoney.layout("SchA", "5.3")
    # Table names are case-insensitive; the version keeps one minor digit.
    assert hardmoney.layout("scha", "8.5.0.1") == layout
    assert hardmoney.layout("SchA", "3.00") == hardmoney.layout("SchA", "3.0")


def test_layout_matches_a_parsed_line() -> None:
    # A parsed line's keys are exactly the layout's field names, in order.
    text = "HDR\x1cFEC\x1c8.5\x1cX\x1c1\nF3XN\x1cC00123456\nSA11AI\x1cC00123456"
    filing = hardmoney.parse(text)
    assert filing.lines[0].keys() == [name for name, _ in hardmoney.layout("SchA", "8.5")]
    assert filing.summary.keys() == [name for name, _ in hardmoney.layout("F3X", "8.5")]


def test_layout_errors() -> None:
    with pytest.raises(ValueError, match="unknown table"):
        hardmoney.layout("SchZ", "8.5")
    with pytest.raises(ValueError, match="not an FEC spec version"):
        hardmoney.layout("SchA", "eight")
    with pytest.raises(hardmoney.FecError, match="no column layout") as info:
        hardmoney.layout("SchA", "9.9")
    assert info.value.line_no is None


def test_field_spec() -> None:
    spec = hardmoney.field_spec("SchA", "contribution_amount")
    assert spec is not None
    assert spec["column"] == 20
    assert spec["kind"] == "amount"
    assert spec["max_len"] == 12
    assert spec["required"] in {"none", "error", "warning", "conditional"}
    assert spec["description"].startswith("CONTRIBUTION AMOUNT")
    assert isinstance(spec["forms"], list) and "F3X" in spec["forms"]
    assert isinstance(spec["allowed_values"], list)
    assert set(spec) == {
        "column",
        "description",
        "kind",
        "max_len",
        "required",
        "condition",
        "sample",
        "value_reference",
        "rule",
        "forms",
        "allowed_values",
        "pattern",
    }

    filer_id = hardmoney.field_spec("F3X", "filer_committee_id_number")
    assert filer_id is not None
    assert filer_id["required"] == "error"
    assert filer_id["pattern"] is not None  # the FEC publishes the committee-id regex
    assert filer_id["pattern"].startswith("^[C|P]")

    form_type = hardmoney.field_spec("F3X", "form_type")
    assert form_type is not None
    assert form_type["allowed_values"] == ["F3XA", "F3XN", "F3XT"]

    entity = hardmoney.field_spec("SchA", "entity_type")
    assert entity is not None
    assert entity["required"] == "error"
    assert entity["rule"] == "[CAN|CCM|COM|IND|ORG|PAC|PTY]"

    conditional = [
        hardmoney.field_spec("SchA", name)
        for name, _ in hardmoney.layout("SchA", "8.5")
        if (s := hardmoney.field_spec("SchA", name)) and s["required"] == "conditional"
    ]
    assert conditional and all(s["condition"] for s in conditional)

    assert hardmoney.field_spec("SchA", "no_such_field") is None
    with pytest.raises(ValueError, match="unknown table"):
        hardmoney.field_spec("SchZ", "x")


def test_layout_and_spec_agree_at_the_bundled_version() -> None:
    for table in ("F3X", "SchA", "SchB", "SchE"):
        for name, column in hardmoney.layout(table, hardmoney.BUNDLED_SPEC_VERSION):
            spec = hardmoney.field_spec(table, name)
            if spec is not None:
                assert spec["column"] == column, (table, name)
