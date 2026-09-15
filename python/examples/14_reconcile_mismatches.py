"""14. Reconcile a Form 3X and print the lines that disagree.

Shows: ``Filing.reconcile()`` and the ``Reconciliation`` / ``LineCheck``
objects. Every cover-page line is recomputed from the schedules (memo
entries excluded) or from other cover lines, with exact ``Decimal``
arithmetic; ``reported``, ``expected``, and ``delta`` are Decimals.
``mismatches()`` is the list of lines whose rule does not hold.

Every fixture in the repository balances (the FEC accepted them), so
when the file balances the example also overstates one line in memory
and reconciles again, to show what a mismatch table looks like. Because
formulas are evaluated over the reported values of their inputs, a wrong
11(a)(i) shows up twice: against Schedule A and in 11(a)(iii) = 11(a)(i)
+ 11(a)(ii). That is what pinpoints the bad line.

Run:
    python examples/14_reconcile_mismatches.py [path/to/filing.fec]

Defaults to tests/fixtures/F3XN_2011831.fec when no path is given.
"""

from __future__ import annotations

import sys
from decimal import Decimal
from pathlib import Path
from typing import Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "F3XN_2011831.fec"


def print_mismatches(r: hardmoney.Reconciliation) -> None:
    print(f"{r.form}: {len(r)} check(s), {len(r.mismatches())} mismatch(es); balances: {r.balances}")
    if not r.mismatches():
        return
    print(f"  {'col':3} {'line':10} {'reported':>14} {'expected':>14} {'delta':>12}  rule")
    for c in r.mismatches():
        reported = "(blank)" if c.reported is None and not c.reported_unparseable else (
            "(unparseable)" if c.reported is None else str(c.reported))
        print(f"  {c.column:3} {c.line:10} {reported:>14} {c.expected!s:>14} {c.delta!s:>12}  {c.rule}")


def main(argv: Sequence[str]) -> int:
    path = Path(argv[0]) if argv else DEFAULT
    filing = hardmoney.parse_file(path)
    try:
        r = filing.reconcile()
    except hardmoney.UnsupportedForm as e:
        print(f"{path.name}: {e}")
        return 0

    print(f"{path.name}: {filing.form_type} v{filing.version}")
    print_mismatches(r)

    if r.balances:
        c = r.line("A", "11(a)(i)")
        if c is not None and c.reported is not None:
            bumped = c.reported + Decimal("100.00")
            print(f"\noverstating column A line 11(a)(i) ({c.field}) by 100.00 in memory: "
                  f"{c.reported} -> {bumped}")
            filing.summary.set(c.field, str(bumped))
            print_mismatches(filing.reconcile())

    # str(r) is the full one-line-per-check listing `hardmoney reconcile` prints.
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
