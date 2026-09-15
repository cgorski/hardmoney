"""09. A running total over a very large filing.

Shows: ``parse_file`` (which streams the file from disk, so peak memory is
the parsed lines rather than the file plus its decoded text) and
``Filing.iter_lines(["SchA"])`` to walk one schedule of a filing with
hundreds of thousands of lines, keeping a ``Decimal`` running total per
form-type token.

The 135 MB presidential filing this was written against (FEC filing
2010101, 704,651 body lines) is not in the repository. Pass any large
filing's path; without one, the example looks for the file under
``tmp/agent-misc/filings/`` and exits quietly if it is absent.

Run:
    python examples/09_stream_large_filing.py [path/to/large.fec]
"""

from __future__ import annotations

import sys
import time
from collections import Counter
from decimal import Decimal
from pathlib import Path
from typing import Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tmp" / "agent-misc" / "filings" / "2010101.fec"
PROGRESS_EVERY = 100_000


def main(argv: Sequence[str]) -> int:
    path = Path(argv[0]) if argv else DEFAULT
    if not path.exists():
        print(f"{path} not present; nothing to stream (pass a filing path to run this example)")
        return 0

    size_mb = path.stat().st_size / 1_000_000
    started = time.perf_counter()
    filing = hardmoney.parse_file(path)
    parsed_at = time.perf_counter()
    print(f"{path.name}: {size_mb:.0f} MB, {filing.form_type} v{filing.version}, "
          f"{len(filing.lines):,} body lines, parsed in {parsed_at - started:.2f}s")
    print(f"committee: {filing.summary['committee_name']}\n")

    totals: Counter[str] = Counter()
    counts: Counter[str] = Counter()
    memos = 0
    seen = 0
    for line in filing.iter_lines(["SchA"]):
        seen += 1
        if seen % PROGRESS_EVERY == 0:
            running = sum(totals.values(), Decimal("0"))
            print(f"  ... {seen:>9,} Schedule A lines, running total {running:>16,}")
        if line.is_memo:
            memos += 1
            continue
        amount = line.amount("contribution_amount")
        if amount is None:
            continue
        totals[line.form_type] += amount
        counts[line.form_type] += 1

    print(f"\n{seen:,} Schedule A line(s) ({memos:,} memo) in {time.perf_counter() - parsed_at:.2f}s")
    print(f"{'token':8} {'lines':>9} {'total':>18}")
    for token, total in totals.most_common():
        print(f"{token:8} {counts[token]:9,} {total:18,}")

    # Itemized individual contributions: SA11AI on Form 3X/3, SA17A on Form 3P.
    token = "SA17A" if filing.base_form_type == "F3P" else "SA11AI"
    cover = filing.summary.amount("col_a_individuals_itemized")
    print(f"\n{token} total {totals[token]} vs cover page itemized individuals {cover}: "
          f"{'equal' if totals[token] == cover else 'different'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
