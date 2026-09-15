"""hardmoney through the interface of the FEC's ``fecfile-validate`` package.

FECfile+ (``fecgov/fecfile-web-api``) validates one record at a time with
``fecfile_validate.validate(schema_name, data) -> ValidationResult``, whose
``errors`` and ``warnings`` are lists of ``ValidationError(message, path)``.
It has no check of a whole ``.fec`` file. This module gives a Django view,
a Celery task, or a test the same two classes over hardmoney's whole-file
validator, plus a per-record entry point and FECfile+'s ``line_*``-keyed
Column A summary::

    from hardmoney.compat import fecfile_validate as fv

    result = fv.validate_file(compose_dot_fec(report_id))
    assert result.errors == [], [e.message for e in result.errors]

    column_a = fv.summary_column_a(dot_fec_bytes)     # {"line_11ai": Decimal("10000.23"), ...}

What is mirrored and what is added
----------------------------------

``ValidationError`` has fecfile-validate's two attributes, ``message`` and
``path``, with the same meaning (``path`` is the property name inside one
record, ``""`` when the finding is not about a field). hardmoney adds
``validator`` (the rule name, like jsonschema's ``validator`` attribute),
``instance`` (the field's value as filed), ``severity``, ``line_no``, and
``form_type``, because a whole-file check needs to say which record it
means. ``ValidationResult`` has ``errors`` and ``warnings`` plus an
``is_acceptable`` property. Nothing else from jsonschema's error object
(``schema``, ``schema_path``, ``validator_value``) has a meaning here and
none is provided.

Record-level validation is emulated
-----------------------------------

hardmoney validates filings, not records, and its Python package has no
record constructor. :func:`validate_record` therefore serialises the record
to one ``.fec`` line at the columns ``hardmoney.layout`` gives, wraps it in
a minimal header and cover page, parses and validates the two-line filing,
and keeps only the findings on the record's line. Cross-record rules that a
single record cannot satisfy (a memo's back-reference, duplicate
transaction ids) are dropped. A record that is itself a cover page (an
``F3XN``, an ``F24N``) is validated as the cover instead.
"""

from __future__ import annotations

import datetime
import os
from decimal import Decimal
from typing import Mapping, Optional, Union

import hardmoney

__all__ = [
    "FecData",
    "ValidationError",
    "ValidationResult",
    "line_key",
    "load_filing",
    "summary_column_a",
    "validate_file",
    "validate_record",
]

FecData = Union[bytes, bytearray, str, "os.PathLike[str]"]
"""What the functions here accept: ``.fec`` content as ``bytes`` or ``str``,
or a path as any :class:`os.PathLike` (a :class:`pathlib.Path`). A plain
``str`` is always content, never a path, because that is what FECfile+'s
``.fec`` composer returns."""

_FS = "\x1c"


class ValidationError:
    """One finding, shaped like ``fecfile_validate.validate.ValidationError``.

    ``message`` and ``path`` are positional, as in fecfile-validate. The
    keyword-only attributes are hardmoney's additions and default to
    ``None`` (``severity`` to ``"error"``).
    """

    __slots__ = ("message", "path", "validator", "instance", "severity", "line_no", "form_type")

    def __init__(
        self,
        message: str,
        path: str,
        *,
        validator: Optional[str] = None,
        instance: Optional[str] = None,
        severity: str = "error",
        line_no: Optional[int] = None,
        form_type: Optional[str] = None,
    ) -> None:
        self.message: str = message
        """A complete sentence, worded after the FEC's own message."""
        self.path: str = path
        """The canonical field name the finding is about, or ``""``."""
        self.validator: Optional[str] = validator
        """hardmoney's rule name, e.g. ``"required_field_empty"``."""
        self.instance: Optional[str] = instance
        """The field's value as filed, when the finding is about a field."""
        self.severity: str = severity
        """``"error"`` (the FEC rejects the filing) or ``"warning"``."""
        self.line_no: Optional[int] = line_no
        """1-based physical line in the ``.fec`` (1 = ``HDR``, 2 = cover)."""
        self.form_type: Optional[str] = form_type
        """The record's form-type token, upper-cased (``"SA11AI"``)."""

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, ValidationError):
            return NotImplemented
        return all(getattr(self, s) == getattr(other, s) for s in self.__slots__)

    def __hash__(self) -> int:
        return hash(tuple(getattr(self, s) for s in self.__slots__))

    def __repr__(self) -> str:
        where = f" line {self.line_no}" if self.line_no is not None else ""
        return f"<ValidationError {self.severity}{where} {self.path!r}: {self.message!r}>"


class ValidationResult:
    """``errors`` and ``warnings`` lists, as ``fecfile_validate.validate`` returns.

    Both default to new empty lists (fecfile-validate shares one mutable
    default list between instances; this class does not).
    """

    def __init__(
        self,
        errors: Optional[list[ValidationError]] = None,
        warnings: Optional[list[ValidationError]] = None,
    ) -> None:
        self.errors: list[ValidationError] = list(errors or [])
        """Findings the FEC would reject the filing for."""
        self.warnings: list[ValidationError] = list(warnings or [])
        """Findings the FEC reports but accepts."""

    @property
    def is_acceptable(self) -> bool:
        """True when ``errors`` is empty."""
        return not self.errors

    def __repr__(self) -> str:
        return f"<ValidationResult {len(self.errors)} error(s), {len(self.warnings)} warning(s)>"


def load_filing(data: FecData, *, lenient: bool = False) -> hardmoney.Filing:
    """Parse ``data`` (see :data:`FecData`) into a :class:`hardmoney.Filing`.

    Raises :class:`hardmoney.FecError` for a filing that cannot be parsed,
    the matching ``OSError`` for an unreadable path, and ``TypeError`` for
    any other type.
    """
    if isinstance(data, os.PathLike):
        return hardmoney.parse_file(os.fspath(data), lenient=lenient)
    if isinstance(data, (bytes, bytearray, str)):
        return hardmoney.parse(data, lenient=lenient)
    raise TypeError(f"expected bytes, str, or os.PathLike, got {type(data).__name__}")


def _errors_for(filing: hardmoney.Filing, findings: list[hardmoney.Finding]) -> list[ValidationError]:
    lines = {line.line_no: line for line in filing.lines}
    lines[filing.summary.line_no] = filing.summary
    out = []
    for f in findings:
        line = lines.get(f.line_no)
        instance = line.get(f.field) if (line is not None and f.field is not None) else None
        out.append(
            ValidationError(
                f.message,
                f.field or "",
                validator=f.rule,
                instance=instance,
                severity=f.severity,
                line_no=f.line_no,
                form_type=f.form_type,
            )
        )
    return out


def validate_file(data: FecData) -> ValidationResult:
    """Validate a whole ``.fec`` against the FEC's acceptance rules.

    Never raises for bad filing content: a file the parser cannot read at
    all (no header, no cover line, an unknown format version) comes back as
    one error with ``validator="parse_error"`` and ``path=""``. Body lines
    the FEC would ignore are parsed leniently and reported as
    ``unrecognized_form_type`` warnings, as ``hardmoney validate`` does.
    """
    try:
        filing = load_filing(data, lenient=True)
    except hardmoney.FecError as e:
        return ValidationResult(
            [ValidationError(str(e), "", validator="parse_error", line_no=e.line_no)], []
        )
    v = filing.validate()
    return ValidationResult(_errors_for(filing, v.errors), _errors_for(filing, v.warnings))


def _wire(value: object) -> str:
    """A record value as it would be written to a ``.fec`` field."""
    if value is None:
        return ""
    if isinstance(value, bool):
        return "X" if value else ""
    if isinstance(value, datetime.datetime):
        return value.date().strftime("%Y%m%d")
    if isinstance(value, datetime.date):
        return value.strftime("%Y%m%d")
    if isinstance(value, float):
        raise TypeError(
            f"{value!r} is a float; pass amounts as Decimal or str so no cents are lost"
        )
    return str(value)


def _canonical(name: str, names: frozenset[str]) -> Optional[str]:
    """hardmoney's field name for a FECfile+ schema property, or ``None``.

    The two vocabularies agree except for three spellings in FECfile+'s
    schemas: ``*_zip`` for ``*_zip_code``, ``back_reference_tran_id_number``
    for ``back_reference_tran_id``, and ``memo_text_description`` for
    ``memo_text``.
    """
    if name in names:
        return name
    if name.endswith("_zip") and name + "_code" in names:
        return name + "_code"
    if name.endswith("_number") and name[: -len("_number")] in names:
        return name[: -len("_number")]
    if name == "memo_text_description" and "memo_text" in names:
        return "memo_text"
    return None


def _record_line(table: str, version: str, form_type: str, fields: Mapping[str, object]) -> str:
    layout = hardmoney.layout(table, version)
    names = frozenset(name for name, _ in layout)
    values: dict[str, str] = {"form_type": form_type}
    for key, value in fields.items():
        canonical = _canonical(key, names)
        if canonical is not None and canonical != "form_type":
            values[canonical] = _wire(value)
    cells = [""] * (max(col for _, col in layout) + 1)
    for name, col in layout:
        cells[col] = values.get(name, "")
    return _FS.join(cells)


def _table_for(form_type: str, version: str) -> str:
    """Let the parser dispatch the token; ``ValueError`` if it cannot."""
    header = _FS.join(["HDR", "FEC", version, "hardmoney", "compat"])
    try:
        return hardmoney.parse(f"{header}\n{form_type}").summary.table
    except hardmoney.FecError as e:
        raise ValueError(f"{form_type!r} is not a form type hardmoney knows at {version}: {e}") from e


def _parent_form(table: str) -> str:
    spec = hardmoney.field_spec(table, "form_type")
    forms = (spec or {}).get("forms") or []
    return "F3X" if not forms or "F3X" in forms else forms[0]


def validate_record(
    form_type: str,
    fields: Mapping[str, object],
    *,
    version: str = hardmoney.BUNDLED_SPEC_VERSION,
) -> ValidationResult:
    """Validate one record, the way ``fecfile_validate.validate(schema, data)`` does.

    ``form_type`` is the record's line token (``"SA11AI"``, ``"SB21B"``,
    ``"H4"``, or a cover token such as ``"F3XN"``); ``fields`` maps field
    names to values. Names are hardmoney's canonical ``lower_snake_case``
    names; the three spellings FECfile+'s schemas use differently
    (``*_zip``, ``back_reference_tran_id_number``, ``memo_text_description``)
    are accepted, and keys that are not ``.fec`` fields at all
    (``transaction_type_identifier``, ``aggregation_group``) are ignored.
    Values may be ``str``, ``Decimal``, ``int``, ``date``, ``bool``
    (``True`` writes ``X``, for ``memo_code``), or ``None`` (blank); a
    ``float`` raises ``TypeError``.

    The record is serialised to one ``.fec`` line under a minimal header
    and cover page, the filing is validated, and only findings on the
    record's line are returned; ``back_reference_not_found`` is dropped
    because one record cannot carry its parent. Raises ``ValueError`` for
    a token hardmoney does not know.
    """
    table = _table_for(form_type, version)
    record = _record_line(table, version, form_type, fields)
    filer_id = _wire(fields.get("filer_committee_id_number")) or "C00000000"
    header = _FS.join(["HDR", "FEC", version, "hardmoney", "compat", "", "", ""])
    parent = _FS.join([_parent_form(table) + "N", filer_id])
    filing = hardmoney.parse(f"{header}\n{parent}\n{record}\n")
    findings = [f for f in filing.validate() if f.line_no == 3]
    if any(f.rule == "multiple_forms" for f in findings):
        # The record is a cover page: validate it as one.
        filing = hardmoney.parse(f"{header}\n{record}\n")
        findings = [f for f in filing.validate() if f.line_no == 2]
    findings = [f for f in findings if f.rule != "back_reference_not_found"]
    errors = [f for f in findings if f.severity == "error"]
    warnings = [f for f in findings if f.severity == "warning"]
    return ValidationResult(_errors_for(filing, errors), _errors_for(filing, warnings))


def line_key(label: str) -> str:
    """The FEC line label as FECfile+'s ``summary.py`` keys it:
    ``11(a)(i)`` -> ``line_11ai``, ``6(b)`` -> ``line_6b``, ``38`` -> ``line_38``."""
    return "line_" + label.replace("(", "").replace(")", "")


# Cover lines with no rule of their own: they are inputs to other lines'
# formulas and have no schedule to sum. FECfile+ reads them from its own
# tables (SA11AII transactions, cash_on_hand_yearly); a filed .fec carries
# them only on the cover. Mirrors the `{ input }` Column A rules in
# `src/parser/reconcile.rs`; the golden F3X test guards against drift.
_INPUT_LINES: dict[str, tuple[tuple[str, str], ...]] = {
    "F3X": (("11(a)(ii)", "col_a_individuals_unitemized"), ("6(b)", "col_a_cash_on_hand_beginning_period")),
    "F3": (("11(a)(ii)", "col_a_individuals_unitemized"), ("23", "col_a_cash_on_hand_beginning_period")),
    "F3P": (("17(a)(ii)", "col_a_individuals_unitemized"), ("6", "col_a_cash_on_hand_beginning_period")),
}


def summary_column_a(data: FecData) -> dict[str, Decimal]:
    """Column A as FECfile+'s ``calculate_summary_column_a`` would compute it.

    Every Column A line of a Form 3X, 3, or 3P, keyed ``line_11ai``,
    ``line_21b``, ``line_6c`` and so on, with exact :class:`decimal.Decimal`
    values: schedule-sourced lines are the sum of the matching non-memo
    schedule lines (the itemization-threshold lines too, so a cover total
    that legitimately exceeds its itemized sum is *not* what comes back);
    lines with no schedule (unitemized individuals, cash on hand at the
    start of the period) are the cover's values; formula lines are
    evaluated over those, not over the cover as filed. The five lines
    FECfile+ stubs to zero (``line_18c``, ``line_21ai``, ``line_21aii``,
    ``line_30ai``, ``line_30aii``) carry the values Schedules H3-H6 imply.

    Raises :class:`hardmoney.UnsupportedForm` for any other form and
    :class:`hardmoney.FecError` for a filing that does not parse.
    """
    filing = load_filing(data)
    r = filing.reconcile()
    # Fixed point: write each line's implied value onto a private copy of
    # the cover until every formula is evaluated over implied values.
    for _ in range(32):
        pending = [c for c in r.checks if c.column == "A" and (c.reported is None or c.reported != c.expected)]
        if not pending:
            break
        for c in pending:
            filing.summary.set(c.field, str(c.expected))
        r = filing.reconcile()
    else:
        raise RuntimeError("Column A did not converge; this is a bug in hardmoney")
    out = {line_key(c.line): c.expected for c in r.checks if c.column == "A"}
    for label, field in _INPUT_LINES[r.form]:
        out[line_key(label)] = filing.summary.amount(field) or Decimal("0.00")
    return out
