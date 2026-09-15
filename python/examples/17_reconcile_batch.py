"""17. Reconcile every report in a directory and summarise the disagreements.

Shows: a batch over ``Filing.reconcile()``, catching ``UnsupportedForm``
for forms that have no cover-page rules (F1, F2, F24, F99, ...), and a
``Counter`` of which (column, line) pairs disagree most often. Handy for
a first look at a committee's history or a day's worth of filings.

Run:
    python examples/17_reconcile_batch.py [directory]

Defaults to tests/fixtures. Every FEC-accepted fixture balances; point it
at a directory of downloaded filings to see mismatches.
"""

from __future__ import annotations

import sys
from collections import Counter
from pathlib import Path
from typing import Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT_DIR = REPO / "tests" / "fixtures"


def main(argv: Sequence[str]) -> int:
    root = Path(argv[0]) if argv else DEFAULT_DIR
    paths = sorted(root.glob("*.fec"))

    reconciled = balanced = unsupported = failed = 0
    disagreeing: Counter[tuple[str, str, str]] = Counter()
    print(f"{'file':28} {'form':5} {'checks':>6} {'diff':>4}  mismatched lines")
    for path in paths:
        try:
            filing = hardmoney.parse_file(path, lenient=True)
            r = filing.reconcile()
        except hardmoney.UnsupportedForm:
            unsupported += 1
            continue
        except hardmoney.FecError as e:
            failed += 1
            print(f"{path.name:28} parse error: {e}")
            continue
        reconciled += 1
        bad = r.mismatches()
        if not bad:
            balanced += 1
        for c in bad:
            disagreeing[(r.form, c.column, c.line)] += 1
        detail = ", ".join(f"{c.column} {c.line} ({c.delta:+})" for c in bad)
        print(f"{path.name:28} {r.form:5} {len(r):6} {len(bad):4}  {detail}")

    print(f"\n{len(paths)} file(s): {reconciled} reconciled ({balanced} balance), "
          f"{unsupported} form(s) without cover-page rules, {failed} unparseable")
    if disagreeing:
        print("\nlines that disagree most often:")
        for (form, column, line), n in disagreeing.most_common(10):
            print(f"  {form} column {column} line {line:10} {n} filing(s)")
    else:
        print("no line disagrees in any filing")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
