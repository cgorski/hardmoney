"""16. Recompute one cover-page line by hand and compare with the reconciler.

Shows: the rule behind ``LineCheck.expected`` for line 11(a)(i), itemized
individual contributions: the ``Decimal`` sum of ``contribution_amount``
over Schedule A lines whose form-type token is ``SA11AI`` (or ``SA11A1``
in pre-6 filings), skipping memo entries. The by-hand sum equals
``expected`` to the cent and the line count equals ``lines_summed``.

The example also sums the memo entries separately, so you can see what
including them would have done to the total.

Run:
    python examples/16_recompute_a_line.py [path/to/filing.fec]

Defaults to tests/fixtures/F3A_2011812.fec (a Form 3 with 41 Schedule A
memo lines: redesignations and reattributions of earlier contributions).
"""

from __future__ import annotations

import sys
from decimal import Decimal
from pathlib import Path
from typing import Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "F3A_2011812.fec"
TOKENS = {"SA11AI", "SA11A1"}


def main(argv: Sequence[str]) -> int:
    path = Path(argv[0]) if argv else DEFAULT
    filing = hardmoney.parse_file(path)

    by_hand = Decimal("0")
    memo_total = Decimal("0")
    counted = memos = 0
    for line in filing.lines_for("SchA"):
        if line.form_type not in TOKENS:
            continue
        amount = line.amount("contribution_amount")
        if amount is None:
            continue
        if line.is_memo:
            memo_total += amount
            memos += 1
        else:
            by_hand += amount
            counted += 1

    # The cover field for 11(a)(i) is named differently on Form 3X and
    # Form 3; the check knows which one it read.
    check = filing.reconcile().line("A", "11(a)(i)")
    assert check is not None
    reported = filing.summary.amount(check.field)
    print(f"{path.name}: {filing.form_type}")
    print(f"  SA11AI lines: {counted} counted, {memos} memo (skipped)")
    print(f"  by-hand sum of non-memo contribution_amount: {by_hand}")
    print(f"  memo entries would have added:               {memo_total}")
    print(f"  cover page 11(a)(i) ({check.field}): {reported}")

    print(f"\nreconciler: {check}")
    print(f"  rule:         {check.rule}")
    print(f"  expected:     {check.expected}  (by hand: {by_hand}, equal: {check.expected == by_hand})")
    print(f"  lines_summed: {check.lines_summed}  (by hand: {counted}, equal: {check.lines_summed == counted})")
    print(f"  reported:     {check.reported}, delta {check.delta}, matches {check.matches}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
