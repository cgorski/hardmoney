"""Type stubs for the ``hardmoney._hardmoney`` extension module.

These mirror the Rust implementation in ``python/src/`` exactly; the
``hardmoney`` package re-exports every name here.
"""

from __future__ import annotations

import datetime
import decimal
import os
from typing import Any, Iterator, Optional, Sequence, TypeVar, Union, final, overload

_T = TypeVar("_T")

__version__: str
"""The package version (same as the Rust crate's)."""

BUNDLED_SPEC_VERSION: str
"""The FEC spec version whose field specifications are bundled (``field_spec``
describes fields at this version), e.g. ``"8.5"``."""

class FecError(ValueError):
    """A filing could not be parsed, written, or interpreted.

    ``line_no`` is the 1-based physical line the error refers to, or ``None``
    when the error is not about one line (a bad header, a missing cover line).
    """

    line_no: Optional[int]
    """The 1-based physical line the error is about, or ``None``. Always
    present, even on an exception constructed from Python."""

class UnsupportedForm(FecError):
    """The cover form has no reconciliation rule table (only F3X, F3, and F3P do)."""

def parse(data: Union[bytes, bytearray, str], *, lenient: bool = False) -> Filing:
    """Parse a filing from ``bytes`` (UTF-8, falling back to Windows-1252) or ``str``.

    Strict by default: the first body line that cannot be parsed raises
    :class:`FecError`. With ``lenient=True`` such lines are recorded in
    :attr:`Filing.skipped` instead. Raises ``TypeError`` for any other type.
    """

def parse_file(path: Union[str, os.PathLike[str]], *, lenient: bool = False) -> Filing:
    """Parse a ``.fec`` file from disk by streaming it.

    Peak memory is the parsed lines alone rather than the file plus its
    decoded text. A missing or unreadable file raises the matching
    ``OSError`` (``FileNotFoundError``, ...); a malformed one raises
    :class:`FecError`.
    """

def fetch(filing_id: int) -> Filing:
    """Download a filing from ``docquery.fec.gov/dcdev/posted/<filing_id>.fec``
    and parse it strictly. Network failures raise :class:`FecError`."""

def tables() -> list[str]:
    """Every table hardmoney knows, by FEC-style name (``"F3X"``, ``"SchA"``,
    ``"TEXT"``), in name order."""

def layout(table: str, version: str) -> list[tuple[str, int]]:
    """The column layout of ``table`` at spec ``version`` as ``(field, column)``
    pairs in table order, ``column`` being 0-based.

    Raises ``ValueError`` for an unknown table or a malformed version, and
    :class:`FecError` when the bundled data has no layout for that table at
    that version.
    """

def field_spec(table: str, name: str) -> Optional[dict[str, Any]]:
    """The FEC's specification of one field of ``table`` at
    :data:`BUNDLED_SPEC_VERSION`, or ``None`` if the current spec does not
    document that field. Raises ``ValueError`` for an unknown table.

    Keys: ``column`` (int), ``description`` (str), ``kind`` (``"alpha"``,
    ``"alpha_numeric"``, ``"numeric"``, ``"amount"``, ``"unknown"``),
    ``max_len`` (int | None), ``required`` (``"none"``, ``"error"``,
    ``"warning"``, ``"conditional"``), ``condition`` (str | None),
    ``sample`` (str | None), ``value_reference`` (str | None), ``rule``
    (str | None), ``forms`` (list[str]), ``allowed_values`` (list[str]),
    ``pattern`` (str | None).
    """

@final
class Filing:
    """A parsed FEC electronic filing: header, cover line, and every body line.

    Obtain one with :func:`parse`, :func:`parse_file`, or :func:`fetch`.
    """

    @property
    def form_type(self) -> str:
        """The top-level form type as filed, upper-cased, e.g. ``"F3XA"``."""

    @property
    def base_form_type(self) -> str:
        """``form_type`` with any amendment/new/termination designator
        stripped, e.g. ``"F3X"``."""

    @property
    def version(self) -> str:
        """The FEC spec version every line was parsed with, e.g. ``"8.5"``."""

    @property
    def is_amendment(self) -> bool:
        """True when the form type designates an amendment."""

    @property
    def amends_filing(self) -> Optional[int]:
        """The filing number this filing amends (from the header's ``FEC-<n>``
        report id), or ``None``."""

    @property
    def header(self) -> dict[str, str]:
        """The ``HDR`` record keyed like the Rust ``Header`` struct:
        ``record_type``, ``ef_type``, ``fec_version_raw``, ``version``,
        ``soft_name``, ``soft_ver``, ``report_id``, ``report_number``,
        ``comment``, and -- on spec 3.x-5.x filings only -- ``name_delim``."""

    @property
    def summary(self) -> Line:
        """The cover/summary line (row 2 of the file)."""

    @property
    def lines(self) -> list[Line]:
        """Every body line in file order. Builds a new list of handles on each
        access; prefer :meth:`iter_lines` or :meth:`lines_for` in a loop over
        a large filing."""

    @property
    def skipped(self) -> list[dict[str, Any]]:
        """Body lines a lenient parse could not interpret, each a dict with
        ``line_no`` (int), ``form_type`` (str), and ``reason`` (str). Always
        empty after a strict parse."""

    def lines_for(self, table: str) -> list[Line]:
        """The body lines belonging to one table, e.g. ``"SchA"``. Raises
        ``ValueError`` for a table name hardmoney does not know."""

    def iter_lines(self, tables: Optional[Sequence[str]] = None) -> Iterator[Line]:
        """Iterate body lines, optionally restricted to the given tables.
        (v1 materialises the selection; the interface is the streaming one.)"""

    def to_fec(self) -> bytes:
        """The filing in canonical ``.fec`` form: Windows-1252 when every
        character is representable (the FEC's character set), else UTF-8."""

    def to_fec_string(self) -> str:
        """The filing in canonical ``.fec`` form as text (CRLF line endings,
        full record width, one wrapping quote pair and padding removed)."""

    def validate(self) -> Validation:
        """Check the filing against the FEC's acceptance rules. Never raises.
        Lines skipped by a lenient parse appear as ``unrecognized_form_type``
        warnings."""

    def reconcile(self) -> Reconciliation:
        """Recompute every cover-page line from the schedules and other cover
        lines. Raises :class:`UnsupportedForm` unless the cover is F3X, F3,
        or F3P."""

    def __repr__(self) -> str:
        """``<hardmoney.Filing F3XN v8.5 (144 body lines)>``."""

@final
class Line:
    """One record of a filing: the cover line or a schedule / sub-form / TEXT line.

    Behaves like a read-mostly mapping from canonical field name to the
    value as filed (trimmed, otherwise verbatim). A ``Line`` is a handle
    into its ``Filing``: :meth:`set` is visible to :meth:`Filing.to_fec`.
    """

    @property
    def table(self) -> str:
        """The format table this line was parsed with, e.g. ``"SchA"``."""

    @property
    def form_type(self) -> str:
        """The form-type token from column 0, upper-cased, e.g. ``"SA11AI"``.
        The ``form_type`` *field* keeps the token exactly as filed."""

    @property
    def line_no(self) -> int:
        """1-based physical line number in the source file (0 if synthetic)."""

    @property
    def is_memo(self) -> bool:
        """True when ``memo_code`` is ``X`` (case-insensitive). Memo entries are
        excluded from every cover-page total."""

    def __getitem__(self, name: str) -> str:
        """The value as filed (``""`` when blank); ``KeyError`` if the field
        does not exist in this filing's layout for the table."""

    def __contains__(self, name: str) -> bool:
        """Whether the layout has the field."""

    @overload
    def get(self, name: str) -> Optional[str]: ...
    @overload
    def get(self, name: str, default: _T) -> Union[str, _T]: ...
    def get(self, name: str, default: Any = None) -> Any:
        """The value, or ``default`` if the field does not exist in the layout.
        A blank field is ``""``, not the default."""

    def keys(self) -> list[str]:
        """Field names in layout (table) order."""

    def items(self) -> list[tuple[str, str]]:
        """``(name, value)`` pairs in layout order, blanks included."""

    def to_dict(self) -> dict[str, str]:
        """Every field as a dict in layout order, blanks included."""

    def set(self, name: str, value: str) -> None:
        """Set a field (trimmed like the parser). ``KeyError`` if the layout has
        no such field. Setting ``form_type`` updates :attr:`form_type` too."""

    def amount(self, name: str) -> Optional[decimal.Decimal]:
        """The field as an exact dollar amount (scale 2), or ``None`` when blank
        or not a valid FEC amount. ``KeyError`` if the field is not in the
        layout."""

    def date(self, name: str) -> Optional[datetime.date]:
        """The field as a ``YYYYMMDD`` date, or ``None`` when blank, zero-filled,
        or not a real date. ``KeyError`` if the field is not in the layout."""

    def __repr__(self) -> str:
        """``<hardmoney.Line SA11AI (SchA) line 3>``: form type, table, and
        physical line number."""

@final
class Validation:
    """The result of :meth:`Filing.validate`: every finding in file order.

    ``len(v)`` is the number of findings, ``str(v)`` prints one finding per
    line in the WebCheck style the CLI uses, and iterating yields
    :class:`Finding` objects.
    """

    @property
    def findings(self) -> list[Finding]:
        """Every finding, in line order."""

    @property
    def errors(self) -> list[Finding]:
        """The error-severity findings: the FEC would reject the filing."""

    @property
    def warnings(self) -> list[Finding]:
        """The warning-severity findings: reported, but the filing is accepted."""

    @property
    def is_acceptable(self) -> bool:
        """True when there are no error-severity findings."""

    def __len__(self) -> int:
        """The number of findings, errors and warnings together."""

    def __iter__(self) -> Iterator[Finding]:
        """Yield every :class:`Finding` in line order (the same list as
        :attr:`findings`)."""

    def __str__(self) -> str:
        """One finding per line in the CLI's WebCheck style, e.g.
        ``ERROR line 2 F3XA date_signed: 20261301 is not a Real Date``.
        Empty when there are no findings."""

    def __repr__(self) -> str:
        """``<hardmoney.Validation 5 error(s), 0 warning(s)>``."""

@final
class Finding:
    """One validation message, tied to a line (and usually a field)."""

    @property
    def severity(self) -> str:
        """``"error"`` (the FEC rejects the filing) or ``"warning"``."""

    @property
    def rule(self) -> str:
        """The rule's stable snake_case name, e.g. ``"required_field_empty"``."""

    @property
    def line_no(self) -> int:
        """1-based physical line in the ``.fec`` file (1 = ``HDR``, 2 = cover)."""

    @property
    def form_type(self) -> str:
        """The record's form-type token, upper-cased (``"HDR"``, ``"SA11AI"``)."""

    @property
    def field(self) -> Optional[str]:
        """The canonical field name when the finding is about one field."""

    @property
    def message(self) -> str:
        """A complete sentence a filer could act on, worded after the FEC's."""

    def __str__(self) -> str:
        """The finding as one line of ``hardmoney validate`` output:
        ``ERROR line 2 F3XA date_signed: 20261301 is not a Real Date``
        (severity, line number, form type, field, message)."""

    def __repr__(self) -> str:
        """``<hardmoney.Finding error not_a_real_date line 2 date_signed>``."""

@final
class Reconciliation:
    """Every cover-page line check for one filing, from :meth:`Filing.reconcile`.

    ``str(r)`` prints one check per line followed by a one-line summary, as
    ``hardmoney reconcile`` does; ``len(r)`` is the number of checks.
    """

    @property
    def form(self) -> str:
        """The cover form the rules came from: ``"F3X"``, ``"F3"``, or ``"F3P"``."""

    @property
    def checks(self) -> list[LineCheck]:
        """Every line check, Column A first, in rule-table order."""

    @property
    def balances(self) -> bool:
        """True when every line agrees exactly with its rule."""

    def mismatches(self) -> list[LineCheck]:
        """The checks that do not match (``delta != 0``, or a floor undershot)."""

    def line(self, column: str, line: str) -> Optional[LineCheck]:
        """The check for one line label in one column (``"A"`` or ``"B"``), e.g.
        ``r.line("A", "11(a)(i)")``, or ``None`` if the form has no such rule
        (or this spec version lacks the line). ``ValueError`` for any other
        column."""

    def __len__(self) -> int:
        """The number of line checks (``len(r.checks)``)."""

    def __str__(self) -> str:
        """Every check as ``hardmoney reconcile`` prints it (one
        :class:`LineCheck` per line, ``ok`` or ``DIFF``), then a summary
        line such as ``F3X: every line agrees with its rule`` or
        ``F3X: 2 of 69 line(s) disagree``."""

    def __repr__(self) -> str:
        """``<hardmoney.Reconciliation F3X 69 check(s), 0 mismatch(es)>``."""

@final
class LineCheck:
    """The outcome of checking one cover-page line."""

    @property
    def line(self) -> str:
        """The FEC's line label, e.g. ``"11(a)(i)"``, ``"6(c)"``, ``"31"``."""

    @property
    def field(self) -> str:
        """The canonical cover-page field holding the reported value."""

    @property
    def column(self) -> str:
        """``"A"`` (this period) or ``"B"`` (year/cycle to date)."""

    @property
    def rule(self) -> str:
        """The rule in the FEC's notation, e.g. ``"= 11ai + 11aii"``."""

    @property
    def reported(self) -> Optional[decimal.Decimal]:
        """The value on the cover page, or ``None`` if blank or not a valid
        amount (:attr:`reported_unparseable` tells the two apart); treated as
        0 in :attr:`delta`."""

    @property
    def expected(self) -> decimal.Decimal:
        """The value the schedules or formula imply."""

    @property
    def delta(self) -> decimal.Decimal:
        """``reported - expected`` (a blank ``reported`` counts as 0)."""

    @property
    def relation(self) -> str:
        """``"equal"`` (must match exactly) or ``"at_least"`` (the itemized sum
        is a floor: sub-$200 items may be reported unitemized)."""

    @property
    def matches(self) -> bool:
        """True when the cover page satisfies the rule."""

    @property
    def violation(self) -> decimal.Decimal:
        """How far the rule is violated: ``|delta|`` for an equality, the
        shortfall for a floor, ``0`` when it matches."""

    @property
    def lines_summed(self) -> int:
        """For schedule sums, how many body lines contributed."""

    @property
    def reported_unparseable(self) -> bool:
        """True when the cover page carries a value that is not a valid FEC
        amount (e.g. ``$5,500.00``). A blank value is ``reported=None`` with
        this ``False``."""

    def __str__(self) -> str:
        """The check as one line of ``hardmoney reconcile`` output:
        ``ok   col A line 11(a)(i)   reported 13736.02 expected 13736.02 delta 0.00  = sum of SchA.contribution_amount on SA11AI/SA11A1``
        (``DIFF`` instead of ``ok`` when it does not match; columns are
        padded for alignment)."""

    def __repr__(self) -> str:
        """``<hardmoney.LineCheck col A line 11(a)(i) ok>`` (``DIFF`` when the
        check fails)."""
