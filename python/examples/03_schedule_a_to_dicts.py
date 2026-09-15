"""03. Schedule A contributions as a list of typed dicts.

Shows: turning ``Line`` objects into plain dicts with ``decimal.Decimal``
amounts and ``datetime.date`` dates, ready for a CSV writer, a DataFrame,
or JSON. ``Line.to_dict()`` gives every field as a string; this example
picks the fields it wants and converts the two that have better Python
types. Amounts are never floats.

Run:
    python examples/03_schedule_a_to_dicts.py [path/to/filing.fec]

Defaults to tests/fixtures/F3XN_2011831.fec when no path is given.
"""

from __future__ import annotations

import datetime
import sys
from decimal import Decimal
from pathlib import Path
from typing import Any, Optional, Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "F3XN_2011831.fec"


def contributor_name(line: hardmoney.Line) -> str:
    """"Last, First" for individuals, the organization name otherwise.

    Spec 3.x filings have a single ``contributor_name`` field instead, so
    fall back to that when the split fields are not in the layout.
    """
    org = line.get("contributor_organization_name", "")
    if org:
        return org
    last = line.get("contributor_last_name", "")
    first = line.get("contributor_first_name", "")
    if last or first:
        return f"{last}, {first}".strip(", ")
    return line.get("contributor_name", "")


def schedule_a_record(line: hardmoney.Line) -> dict[str, Any]:
    amount: Optional[Decimal] = line.amount("contribution_amount")
    date: Optional[datetime.date] = line.date("contribution_date")
    return {
        "line_no": line.line_no,
        "form_type": line.form_type,
        "transaction_id": line["transaction_id"],
        "entity_type": line["entity_type"],
        "contributor": contributor_name(line),
        "city": line["contributor_city"],
        "state": line["contributor_state"],
        "employer": line["contributor_employer"],
        "occupation": line["contributor_occupation"],
        "date": date,
        "amount": amount,
        "aggregate": line.amount("contribution_aggregate"),
        "memo": line.is_memo,
    }


def main(argv: Sequence[str]) -> int:
    path = Path(argv[0]) if argv else DEFAULT
    filing = hardmoney.parse_file(path)
    records = [schedule_a_record(line) for line in filing.lines_for("SchA")]

    print(f"{len(records)} Schedule A record(s) from {path.name}\n")
    for rec in records[:5]:
        print(rec)

    if records:
        first = records[0]
        print(f"\namount is {type(first['amount']).__name__}, date is {type(first['date']).__name__}")

    # sum() needs a Decimal start value, or an empty list would give int 0.
    total = sum((r["amount"] for r in records if not r["memo"] and r["amount"] is not None), Decimal("0"))
    print(f"total of non-memo contributions: {total}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
