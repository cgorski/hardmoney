"""15. ``Relation``: which cover lines must equal their schedule and which
are floors, and why (the $200 itemization threshold).

Shows: ``LineCheck.relation``, ``matches``, ``delta``, and ``violation``.

Federal law requires itemizing a receipt or disbursement on a schedule
only once the aggregate with that person exceeds $200 in the cycle
(11 CFR 104.3). Smaller items are still counted in the cover total, so
for those lines (operating expenditures, offsets, other receipts, refunds
to individuals) the schedule sum is a floor: ``relation == "at_least"``,
and a cover total above the itemized sum is normal. Lines whose
transactions must always be itemized (committee contributions, transfers,
loans, independent expenditures) are ``"equal"``. Formula lines (this
line = that line + the other) are ``"equal"`` too.

Run:
    python examples/15_relations_and_threshold.py [path/to/filing.fec]

Defaults to tests/fixtures/F3XA_2011821.fec, whose operating expenditures
(line 21(b)) exceed the itemized Schedule B sum by $79.00 of unitemized
spending.
"""

from __future__ import annotations

import sys
from decimal import Decimal
from pathlib import Path
from typing import Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "F3XA_2011821.fec"


def show(c: hardmoney.LineCheck) -> None:
    verdict = "ok" if c.matches else f"VIOLATION {c.violation}"
    print(f"  {c.column} {c.line:9} {c.relation:8} reported {c.reported!s:>11} "
          f"expected {c.expected!s:>11} delta {c.delta!s:>9}  {verdict}")


def main(argv: Sequence[str]) -> int:
    path = Path(argv[0]) if argv else DEFAULT
    filing = hardmoney.parse_file(path)
    r = filing.reconcile()
    print(f"{path.name}: {r.form}, {len(r)} checks, balances: {r.balances}\n")

    floors = [c for c in r.checks if c.relation == "at_least"]
    equals = [c for c in r.checks if c.relation == "equal"]
    print(f"{len(floors)} floor check(s) ('at_least'), {len(equals)} exact check(s) ('equal')\n")

    print("floors with a schedule (column A): cover total >= itemized sum")
    for c in floors:
        if c.column == "A":
            show(c)
            if c.reported is not None and c.delta > 0:
                print(f"    -> {c.delta} reported on the cover but not itemized: items under the "
                      f"$200 aggregate threshold. matches={c.matches}, violation={c.violation}")

    print("\nexact checks against a schedule (column A, non-zero):")
    for c in equals:
        if c.column == "A" and c.lines_summed and c.expected != 0:
            show(c)

    # Undershooting a floor is a discrepancy: itemized more than was reported.
    opex = r.line("A", "21(b)")
    if opex is not None and opex.reported is not None:
        below = opex.expected - Decimal("100.00")
        print(f"\nsetting 21(b) below its itemized sum ({opex.expected} -> {below}) in memory:")
        filing.summary.set(opex.field, str(below))
        again = filing.reconcile().line("A", "21(b)")
        if again is not None:
            show(again)
            print(f"    -> delta {again.delta} is negative; violation is the shortfall, {again.violation}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
