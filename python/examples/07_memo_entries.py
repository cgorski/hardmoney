"""07. Memo entries, and why they are excluded from totals.

Shows: ``Line.is_memo`` (true when ``memo_code`` is ``X``), the back
reference that ties a memo to the transaction it annotates, and the
double counting you get by summing memos into a total.

A memo entry is informational: it itemizes something already counted in
another line (a joint fundraising committee's transfer broken down by
original donor, a credit card payment broken down by vendor, an earmark's
conduit and recipient). The cover-page totals exclude them, so any
recomputed total must too. Forgetting this is the most common reason a
recomputed total does not match the cover page.

Run:
    python examples/07_memo_entries.py [path/to/filing.fec]

Defaults to tests/fixtures/F3XN_2011834.fec when no path is given.
"""

from __future__ import annotations

import sys
from decimal import Decimal
from pathlib import Path
from typing import Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "F3XN_2011834.fec"


def name_of(line: hardmoney.Line) -> str:
    return (line.get("contributor_organization_name")
            or line.get("contributor_last_name")
            or line.get("contributor_name")
            or "")


def main(argv: Sequence[str]) -> int:
    path = Path(argv[0]) if argv else DEFAULT
    filing = hardmoney.parse_file(path)
    sched_a = filing.lines_for("SchA")
    memos = [l for l in sched_a if l.is_memo]
    print(f"{path.name}: {len(sched_a)} Schedule A line(s), {len(memos)} memo(s)\n")

    print(f"{'line':>4} {'token':6} {'memo':4} {'amount':>10}  {'back ref':14} name / memo text")
    for line in sched_a:
        flag = "X" if line.is_memo else ""
        print(f"{line.line_no:4} {line.form_type:6} {flag:4} {line.amount('contribution_amount')!s:>10}  "
              f"{line['back_reference_tran_id']:14} {name_of(line)}"
              + (f" ({line['memo_text']})" if line['memo_text'] else ""))

    # Sum one form-type token both ways.
    tokens = sorted({l.form_type for l in memos}) or sorted({l.form_type for l in sched_a})
    for token in tokens:
        lines = [l for l in sched_a if l.form_type == token]
        with_memos = sum((l.amount("contribution_amount") or Decimal(0) for l in lines), Decimal("0"))
        without = sum((l.amount("contribution_amount") or Decimal(0) for l in lines if not l.is_memo), Decimal("0"))
        print(f"\n{token}: {len(lines)} line(s)")
        print(f"  sum including memos: {with_memos}")
        print(f"  sum excluding memos: {without}")

    if filing.base_form_type in ("F3X", "F3", "F3P"):
        r = filing.reconcile()
        for token in tokens:
            checks = [c for c in r.checks if c.column == "A" and token in c.rule]
            for c in checks:
                print(f"\ncover line {c.line} ({c.field}) reported {c.reported}; "
                      f"expected {c.expected} from {c.lines_summed} non-memo line(s): "
                      f"{'match' if c.matches else 'MISMATCH'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
