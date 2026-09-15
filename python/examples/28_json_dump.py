"""28. A filing as JSON, with Decimal and date handled.

Shows: ``json.dumps`` over a structure built from ``Filing.header``,
``Line.to_dict()``, and the typed accessors. ``Line.to_dict()`` is all
strings and serialises as is. When you add ``Decimal`` amounts or
``date`` objects, pass ``default=str``: the ``json`` module calls it for
any value it cannot encode, and ``str(Decimal("380.00"))`` is the exact
``"380.00"`` rather than a float. Never convert an amount to a float.

Run:
    python examples/28_json_dump.py [path/to/filing.fec] [out.json]

Defaults to tests/fixtures/F3XA_2011827.fec and prints to stdout.
"""

from __future__ import annotations

import json
import sys
from decimal import Decimal
from pathlib import Path
from typing import Any, Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "F3XA_2011827.fec"

# Fields whose typed value is worth carrying alongside the raw string.
AMOUNT_FIELDS = ("contribution_amount", "expenditure_amount", "contribution_aggregate")
DATE_FIELDS = ("contribution_date", "expenditure_date", "coverage_from_date", "coverage_through_date")


def line_record(line: hardmoney.Line) -> dict[str, Any]:
    rec: dict[str, Any] = {"line_no": line.line_no, "table": line.table, "is_memo": line.is_memo}
    rec["fields"] = line.to_dict()
    typed: dict[str, Any] = {}
    for name in AMOUNT_FIELDS:
        if name in line:
            typed[name] = line.amount(name)  # Decimal or None
    for name in DATE_FIELDS:
        if name in line:
            typed[name] = line.date(name)  # date or None
    if typed:
        rec["typed"] = typed
    return rec


def filing_document(filing: hardmoney.Filing) -> dict[str, Any]:
    return {
        "form_type": filing.form_type,
        "version": filing.version,
        "is_amendment": filing.is_amendment,
        "amends_filing": filing.amends_filing,
        "header": filing.header,
        "cover": line_record(filing.summary),
        "lines": [line_record(l) for l in filing.iter_lines()],
    }


def main(argv: Sequence[str]) -> int:
    path = Path(argv[0]) if argv else DEFAULT
    doc = filing_document(hardmoney.parse_file(path))
    text = json.dumps(doc, indent=2, default=str)  # default=str handles Decimal and date

    if len(argv) > 1:
        Path(argv[1]).write_text(text, encoding="utf-8")
        print(f"wrote {len(text)} characters to {argv[1]}")
    else:
        # Print the interesting parts rather than all of it.
        print(json.dumps({k: doc[k] for k in ("form_type", "version", "is_amendment", "amends_filing")}, indent=2))
        first = doc["lines"][0]
        print(json.dumps({"line_no": first["line_no"], "table": first["table"], "typed": first["typed"],
                          f"fields (3 of {len(first['fields'])})": dict(list(first["fields"].items())[5:8])},
                         indent=2, default=str))
        print(f"... {len(text)} characters for {len(doc['lines'])} body line(s)")

    # Round trip: json.loads gives strings back; Decimal(...) restores exactness.
    back = json.loads(text)
    amount = back["lines"][0]["typed"].get("contribution_amount") or back["lines"][0]["typed"].get("expenditure_amount")
    print(f"\nfirst typed amount after json.loads: {amount!r} -> {Decimal(amount)!r}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
