"""02. Iterate every body line and count them by table and form type.

Shows: ``Filing.iter_lines()``, ``Line.table`` (the format table a line was
parsed with, e.g. ``SchA``) versus ``Line.form_type`` (the token in column
0, e.g. ``SA11AI``), and ``Line.is_memo``.

Run:
    python examples/02_count_lines_by_table.py [path/to/filing.fec]

Defaults to tests/fixtures/F3XA_2011821.fec when no path is given.
"""

from __future__ import annotations

import sys
from collections import Counter
from pathlib import Path
from typing import Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "F3XA_2011821.fec"


def main(argv: Sequence[str]) -> int:
    path = Path(argv[0]) if argv else DEFAULT
    filing = hardmoney.parse_file(path)

    by_table: Counter[str] = Counter()
    by_form_type: Counter[str] = Counter()
    memos: Counter[str] = Counter()
    for line in filing.iter_lines():
        by_table[line.table] += 1
        by_form_type[line.form_type] += 1
        if line.is_memo:
            memos[line.table] += 1

    print(f"{path.name}: {filing.form_type} v{filing.version}, {len(filing.lines)} body lines\n")
    print(f"{'table':8} {'lines':>6} {'memo':>6}")
    for table, n in by_table.most_common():
        print(f"{table:8} {n:6} {memos[table]:6}")

    print(f"\n{'form type':10} {'lines':>6}")
    for token, n in sorted(by_form_type.items()):
        print(f"{token:10} {n:6}")

    # lines_for(table) is the same selection as filtering iter_lines().
    assert len(filing.lines_for("SchA")) == by_table["SchA"]
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
