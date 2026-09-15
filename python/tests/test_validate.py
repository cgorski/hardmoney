"""Validation: no errors on any FEC-accepted fixture; the broken copies fail."""

from __future__ import annotations

from pathlib import Path

import pytest

import hardmoney
from conftest import FS, INVALID_PATHS, MINIMAL


def test_every_accepted_fixture_has_no_errors(fixture_path: Path) -> None:
    v = hardmoney.parse(fixture_path.read_bytes()).validate()
    assert isinstance(v, hardmoney.Validation)
    assert v.errors == [], str(v)
    assert v.is_acceptable is True
    assert len(v) == len(v.findings) == len(v.warnings)
    for finding in v:
        assert finding.severity == "warning"
        assert finding.line_no >= 1
        assert finding.form_type
        assert finding.message
        assert finding.rule and finding.rule == finding.rule.lower()
    # str(v) is one finding per line, as the CLI prints.
    text = str(v)
    assert text.count("\n") == len(v)
    for line, finding in zip(text.splitlines(), v):
        assert line.startswith("WARN  line ")
        assert line == str(finding)


def test_clean_synthetic_filing_is_acceptable() -> None:
    v = hardmoney.parse(MINIMAL).validate()
    assert v.is_acceptable
    # A minimal cover page is missing recommended fields, so warnings but
    # never errors; every warning names its field.
    assert all(f.field for f in v.warnings)
    assert "0 error(s)" in repr(v)


def test_finding_fields_and_severity() -> None:
    text = MINIMAL.replace("Smith", "S" * 31)
    v = hardmoney.parse(text).validate()
    assert not v.is_acceptable
    [err] = v.errors
    assert isinstance(err, hardmoney.Finding)
    assert err.severity == "error"
    assert err.rule == "field_too_long"
    assert err.line_no == 3
    assert err.form_type == "SA11AI"
    assert err.field == "contributor_last_name"
    assert "maximum length of 30" in err.message
    assert str(err).startswith("ERROR line 3 SA11AI contributor_last_name: ")
    assert "field_too_long" in repr(err)
    assert [f.rule for f in v.findings if f.severity == "error"] == ["field_too_long"]


def test_header_findings_have_no_field() -> None:
    text = MINIMAL.replace("F3XN", "F3XA")  # amendment with no FEC-<n>
    v = hardmoney.parse(text).validate()
    hdr = [f for f in v.errors if f.form_type == "HDR"]
    assert hdr, str(v)
    assert {f.rule for f in hdr} == {"amendment_needs_original_id", "amendment_needs_number"}
    assert all(f.line_no == 1 for f in hdr)


@pytest.mark.parametrize("path", INVALID_PATHS, ids=lambda p: p.name)
def test_every_invalid_fixture_is_rejected(path: Path) -> None:
    v = hardmoney.parse(path.read_bytes(), lenient=True).validate()
    assert not v.is_acceptable, path.name
    assert len(v.errors) >= 1
    lines = str(v).splitlines()
    assert len(lines) == len(v)
    assert all(l.startswith(("ERROR line ", "WARN  line ")) for l in lines)


def test_unrecognized_form_type_is_a_warning() -> None:
    filing = hardmoney.parse(MINIMAL + f"ZZZ{FS}C00123456\n", lenient=True)
    v = filing.validate()
    [w] = [f for f in v.warnings if f.rule == "unrecognized_form_type"]
    assert w.line_no == 4
    assert w.form_type == "ZZZ"
    assert w.field == "form_type"
    assert v.is_acceptable
