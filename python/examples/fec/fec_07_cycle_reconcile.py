"""FEC 07. A committee's reports across a cycle: floors, and what a persistent shortfall means.

Shows: ``reconcile()`` over every periodic report in a directory, grouped
by committee. Besides balances / does not, each report's floor lines
(``relation == "at_least"``: the lines the $200 itemization threshold
applies to, such as Form 3X 21(b) operating expenditures and 17 other
receipts) are shown as ``reported - itemized``, the unitemized remainder.
Positive is normal: receipts or disbursements under the threshold. Negative
means the schedule itemizes more than the cover total admits; the FEC's
validator lets that through, and the same line negative report after
report is the pattern an auditor follows up. Given a committee id, the
CLI fetches the cycle's most-recent reports first (network, API key).

Run:
    python examples/fec/fec_07_cycle_reconcile.py [dir | committee_id [cycle]]   (default tests/fixtures)
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from collections import Counter, defaultdict
from decimal import Decimal
from pathlib import Path
from typing import Optional, Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[3]


def cli() -> Optional[str]:
    builds = [p for p in (REPO / "target" / prof / "hardmoney" for prof in ("debug", "release")) if p.exists()]
    return os.environ.get("HARDMONEY_BIN") or shutil.which("hardmoney") or (
        str(max(builds, key=lambda p: p.stat().st_mtime)) if builds else None)


def fetch_cycle(committee: str, cycle: int) -> Path:
    out, bin_ = Path(tempfile.mkdtemp()) / committee, cli()
    if bin_ is None:
        raise SystemExit("fetching needs the hardmoney CLI (HARDMONEY_BIN, PATH, or cargo build --all-features)")
    for form in ("F3X", "F3", "F3P"):
        subprocess.run([bin_, "filings", "--committee", committee, "--cycle", str(cycle), "--form-type", form,
                        "--most-recent", "--fetch", str(out)], check=True, capture_output=True, text=True)
    return out


def main(argv: Sequence[str]) -> int:
    where = argv[0] if argv else str(REPO / "tests" / "fixtures")
    folder = fetch_cycle(where, int(argv[1]) if len(argv) > 1 else 2026) if where.startswith("C") and where[1:].isdigit() else Path(where)

    by_committee: dict[str, list[tuple[Path, hardmoney.Filing, hardmoney.Reconciliation]]] = defaultdict(list)
    for path in sorted(folder.glob("*.fec")):
        filing = hardmoney.parse_file(path, lenient=True)
        try:
            r = filing.reconcile()  # only F3X, F3, and F3P have cover-page rules
        except hardmoney.UnsupportedForm:
            continue
        by_committee[filing.summary["filer_committee_id_number"]].append((path, filing, r))

    for committee, reports in sorted(by_committee.items(), key=lambda kv: -len(kv[1])):
        reports.sort(key=lambda t: t[1].summary.get("coverage_from_date") or "")
        print(f"{committee}  {reports[0][1].summary.get('committee_name')}  ({len(reports)} report(s))")
        shortfalls: Counter[str] = Counter()
        for path, filing, r in reports:
            cover = filing.summary
            floors = {c.line: (c.reported or Decimal(0)) - c.expected for c in r.checks if c.column == "A" and c.relation == "at_least"}
            shortfalls.update(line for line, rem in floors.items() if rem < 0)
            verdict = "balances" if r.balances else "; ".join(f"{c.line} {c.delta:+}" for c in r.mismatches())
            print(f"  {path.name:30} {filing.form_type:5} {cover.get('report_code') or '':4} "
                  f"{cover.get('coverage_from_date')}..{cover.get('coverage_through_date')}  {verdict}")
            print("      unitemized remainder on floor lines: " + (", ".join(
                f"{line} {rem:+}" for line, rem in floors.items() if rem != 0) or "none (every floor line equals its itemized sum)"))
        for line, n in shortfalls.items():
            print(f"  ! line {line} is below its itemized sum in {n} of {len(reports)} report(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
