"""FEC 06. How long each of a committee's reports took to reach the processed data.

Shows: the raw filing on day 0 against the FEC's own processing log.
``hardmoney filings --json --fetch`` finds a committee's reports through
openFEC and downloads each ``.fec``; ``parse_file`` counts the itemized,
non-memo Schedule A and B lines that pass 2 (itemization) has to load;
``/operations-log/`` (joined on ``sub_id``) says when pass 1 (summary)
and pass 2 finished. The gap between receipt and
``transaction_data_complete_date`` is the lag a data user waits through.
Needs the CLI, network, and an openFEC key in ``FEC_API_KEY`` or
``~/fec_api_key.txt`` (read, never printed).

Run:
    python examples/fec/fec_06_processing_lag.py [committee_id] [cycle] [limit]   (default C00140855 2026 8)
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from datetime import date
from pathlib import Path
from typing import Any, Optional, Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[3]


def cli() -> Optional[str]:
    builds = [p for p in (REPO / "target" / prof / "hardmoney" for prof in ("debug", "release")) if p.exists()]
    return os.environ.get("HARDMONEY_BIN") or shutil.which("hardmoney") or (
        str(max(builds, key=lambda p: p.stat().st_mtime)) if builds else None)


def api_key() -> Optional[str]:
    path = Path.home() / "fec_api_key.txt"
    return os.environ.get("FEC_API_KEY") or (path.read_text().strip() if path.exists() else None)


def operations_log(key: str, committee: str, year: int, form_type: str) -> dict[str, dict[str, Any]]:
    query = {"api_key": key, "candidate_committee_id": committee, "report_year": year, "form_type": form_type, "per_page": 100}
    try:
        with urllib.request.urlopen("https://api.open.fec.gov/v1/operations-log/?" + urllib.parse.urlencode(query), timeout=60) as r:
            return {str(row["sub_id"]): row for row in json.load(r)["results"]}
    except urllib.error.HTTPError as e:  # never echo e.url: it carries the key
        raise SystemExit(f"openFEC /operations-log/ answered HTTP {e.code}") from None


def day(value: Optional[str]) -> Optional[date]:
    return date.fromisoformat(value[:10]) if value else None


def main(argv: Sequence[str]) -> int:
    bin_, key = cli(), api_key()
    if bin_ is None or key is None:
        print("needs the hardmoney CLI and an openFEC API key (FEC_API_KEY or ~/fec_api_key.txt)")
        return 0
    committee, cycle = (argv[0] if argv else "C00140855"), int(argv[1]) if len(argv) > 1 else 2026
    limit, form_type, out_dir = int(argv[2]) if len(argv) > 2 else 8, "F3X", Path(tempfile.mkdtemp())
    records = json.loads(subprocess.run(
        [bin_, "filings", "--committee", committee, "--cycle", str(cycle), "--form-type", form_type, "--most-recent",
         "--limit", str(limit), "--json", "--fetch", str(out_dir)], check=True, capture_output=True, text=True).stdout)
    log = {**operations_log(key, committee, cycle - 1, form_type), **operations_log(key, committee, cycle, form_type)}

    print(f"{committee}: {len(records)} most-recent {form_type} report(s) in the {cycle} cycle\n")
    print(f"{'file':>8} {'report':6} {'received':10} {'summary':>8} {'itemized':>9}  {'SchA lines':>10} {'SchB lines':>10}  (non-memo, from the raw .fec)")
    lags: list[int] = []
    for rec in records:
        filing = hardmoney.parse_file(rec["actions"]["saved"], lenient=True)
        sched_a, sched_b = (sum(not l.is_memo for l in filing.lines_for(t)) for t in ("SchA", "SchB"))
        row, received = log.get(str(rec.get("sub_id")), {}), day(rec["receipt_date"])
        done1, done2 = day(row.get("summary_data_complete_date")), day(row.get("transaction_data_complete_date"))
        lag1 = f"+{(done1 - received).days}d" if done1 and received else "pending"
        lag2 = f"+{(done2 - received).days}d" if done2 and received else "pending"
        if done2 and received:
            lags.append((done2 - received).days)
        print(f"{rec['file_number']:>8} {rec['report_type']:6} {received!s:10} {lag1:>8} {lag2:>9}  {sched_a:>10} {sched_b:>10}")
    if lags:
        print(f"\nitemization lag: median {sorted(lags)[len(lags) // 2]} day(s), max {max(lags)}, {len(records) - len(lags)} still pending")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
