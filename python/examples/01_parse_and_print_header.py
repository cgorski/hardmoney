"""01. Parse a filing and print its header and cover page.

Shows: ``hardmoney.parse_file``, the ``Filing`` metadata properties
(``form_type``, ``base_form_type``, ``version``, ``is_amendment``,
``amends_filing``), the ``header`` dict, and reading cover-page fields
with ``Line.get``, ``Line.amount`` (a ``Decimal``) and ``Line.date``.

Run:
    python examples/01_parse_and_print_header.py [path/to/filing.fec]

Defaults to tests/fixtures/F3XN_2011831.fec when no path is given.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "F3XN_2011831.fec"

# Cover-page fields worth printing when the form has them. Form 3X, 3, and
# 3P share most of these; F1, F2, F24, F99 have different cover pages, so
# every lookup below uses ``get`` and skips fields the layout lacks.
TEXT_FIELDS = [
    "filer_committee_id_number",
    "committee_name",
    "report_code",
    "treasurer_last_name",
    "treasurer_first_name",
]
DATE_FIELDS = ["coverage_from_date", "coverage_through_date", "date_signed"]
AMOUNT_FIELDS = [
    "col_a_cash_on_hand_beginning_period",
    "col_a_total_receipts",
    "col_a_total_disbursements",
    "col_a_cash_on_hand_close_of_period",
]


def main(argv: Sequence[str]) -> int:
    path = Path(argv[0]) if argv else DEFAULT
    filing = hardmoney.parse_file(path)

    print(f"file:            {path.name}")
    print(f"form type:       {filing.form_type} (base {filing.base_form_type})")
    print(f"spec version:    {filing.version}")
    print(f"amendment:       {filing.is_amendment}", end="")
    if filing.amends_filing is not None:
        print(f" (amends filing {filing.amends_filing})", end="")
    print()
    print(f"body lines:      {len(filing.lines)}")

    print("\nheader (HDR record):")
    for key, value in filing.header.items():
        print(f"  {key:16} {value!r}")

    cover = filing.summary
    print(f"\ncover page (line {cover.line_no}, table {cover.table}):")
    for name in TEXT_FIELDS:
        if name in cover:
            print(f"  {name:38} {cover[name]!r}")
    for name in DATE_FIELDS:
        if name in cover:
            # date() is a datetime.date or None (blank / not a real date).
            print(f"  {name:38} {cover.date(name)}")
    for name in AMOUNT_FIELDS:
        if name in cover:
            # amount() is a decimal.Decimal with two places, or None.
            print(f"  {name:38} {cover.amount(name)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
