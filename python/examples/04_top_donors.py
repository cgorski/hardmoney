"""04. Top donors by name and employer, with exact Decimal totals.

Shows: aggregating ``Decimal`` amounts in a ``collections.Counter`` (its
values can be any type that supports ``+=``), ``most_common``, and summing
Decimals with ``sum(..., Decimal("0"))``. Memo entries are excluded because
they are not part of the reported totals (see example 07).

Why not floats: 132 contributions summed in binary floating point can land
a cent away from the cover page. ``Decimal`` sums land exactly on it.

Run:
    python examples/04_top_donors.py [path/to/filing.fec] [N]

Defaults to tests/fixtures/F3A_2011812.fec and N=10.
"""

from __future__ import annotations

import sys
from collections import Counter
from decimal import Decimal
from pathlib import Path
from typing import Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "F3A_2011812.fec"
# The form-type tokens for itemized individual contributions: line 11(a)(i)
# on Form 3X and Form 3 (SA11A1 is the pre-6.x spelling), 17(a)(i) on 3P.
INDIVIDUAL_TOKENS = {"SA11AI", "SA11A1", "SA17A"}


def donor_key(line: hardmoney.Line) -> tuple[str, str]:
    """(name, employer); organizations have no employer of their own."""
    org = line.get("contributor_organization_name", "")
    if org:
        return (org, "")
    last = line.get("contributor_last_name") or line.get("contributor_name", "")
    first = line.get("contributor_first_name", "")
    name = f"{last}, {first}".strip(", ")
    return (name, line["contributor_employer"])


def main(argv: Sequence[str]) -> int:
    path = Path(argv[0]) if argv else DEFAULT
    top_n = int(argv[1]) if len(argv) > 1 else 10
    filing = hardmoney.parse_file(path)

    totals: Counter[tuple[str, str]] = Counter()
    count: Counter[tuple[str, str]] = Counter()
    individuals = Decimal("0")
    for line in filing.lines_for("SchA"):
        if line.is_memo:
            continue
        amount = line.amount("contribution_amount")
        if amount is None:  # blank or not a valid FEC amount
            continue
        totals[donor_key(line)] += amount
        count[donor_key(line)] += 1
        if line.form_type in INDIVIDUAL_TOKENS:
            individuals += amount

    grand_total = sum(totals.values(), Decimal("0"))
    print(f"{path.name}: {sum(count.values())} non-memo Schedule A line(s), "
          f"{len(totals)} distinct donor(s), total {grand_total}\n")
    print(f"{'rank':>4}  {'total':>12}  {'n':>3}  donor / employer")
    for rank, ((name, employer), total) in enumerate(totals.most_common(top_n), start=1):
        who = f"{name} / {employer}" if employer else name
        print(f"{rank:4}  {total:>12}  {count[(name, employer)]:3}  {who}")

    # The exactness check: the Decimal sum of the itemized individual lines
    # equals the cover page's 11(a)(i) (17(a)(i) on Form 3P) to the cent.
    # Form 3X and Form 3P call the field col_a_individuals_itemized, Form 3
    # col_a_individual_contributions_itemized.
    itemized = None
    for field in ("col_a_individuals_itemized", "col_a_individual_contributions_itemized"):
        if field in filing.summary:
            itemized = filing.summary.amount(field)
    if itemized is not None:
        print(f"\nitemized individuals: Decimal sum {individuals}, cover page {itemized}, "
              f"equal: {individuals == itemized}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
