"""30. Find a committee's filings with the CLI and validate each one.

Shows: ``hardmoney filings --committee ... --json --fetch DIR`` asks
openFEC for the filings, downloads each raw ``.fec`` into ``DIR`` (cache
first), and prints a JSON array in openFEC's field names with an
``actions.saved`` path per record. Python then parses each saved file
and runs ``validate()`` and ``reconcile()`` on it.

Needs the ``hardmoney`` CLI (on PATH, at ``$HARDMONEY_BIN``, or built in
the repository), an openFEC API key in ``FEC_API_KEY`` or
``~/fec_api_key.txt``, and network access. In the test suite it runs only
when ``HARDMONEY_NETWORK_TESTS`` is set.

Run:
    python examples/30_filings_search_and_validate.py [committee_id] [limit] [dir]

Defaults to C00140855 (FirstEnergy PAC), 3 filings, and a temp directory.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Optional, Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]


def hardmoney_cli() -> Optional[str]:
    env = os.environ.get("HARDMONEY_BIN")
    if env:
        return env
    found = shutil.which("hardmoney")
    if found:
        return found
    # A checkout may have both a debug and a release build; take the newest.
    builds = [p for p in (REPO / "target" / prof / "hardmoney" for prof in ("debug", "release")) if p.exists()]
    if builds:
        return str(max(builds, key=lambda p: p.stat().st_mtime))
    return None


def have_api_key() -> bool:
    return bool(os.environ.get("FEC_API_KEY")) or (Path.home() / "fec_api_key.txt").exists()


def search(cli: str, committee: str, limit: int, out_dir: Path) -> list[dict[str, Any]]:
    cmd = [cli, "filings", "--committee", committee, "--form-type", "F3X", "--most-recent",
           "--limit", str(limit), "--json", "--fetch", str(out_dir)]
    result = subprocess.run(cmd, check=True, capture_output=True, text=True)
    return json.loads(result.stdout)


def main(argv: Sequence[str]) -> int:
    cli = hardmoney_cli()
    if cli is None:
        print("hardmoney CLI not found (set HARDMONEY_BIN or cargo build --all-features)")
        return 0
    if not have_api_key():
        print("no openFEC API key: set FEC_API_KEY or write it to ~/fec_api_key.txt")
        return 0
    committee = argv[0] if argv else "C00140855"
    limit = int(argv[1]) if len(argv) > 1 else 3
    out_dir = Path(argv[2]) if len(argv) > 2 else Path(tempfile.mkdtemp()) / "filings"

    records = search(cli, committee, limit, out_dir)
    print(f"{len(records)} filing(s) for {committee}\n")
    print(f"{'file_number':>11} {'report':6} {'coverage':23} {'receipts':>12}  errors warnings  reconcile")
    for rec in records:
        saved = rec.get("actions", {}).get("saved")
        if not saved:
            print(f"{rec['file_number']:>11} {rec.get('report_type', ''):6} (not fetched: {rec.get('actions')})")
            continue
        filing = hardmoney.parse_file(saved, lenient=True)
        v = filing.validate()
        try:
            r = filing.reconcile()
            recon = "balances" if r.balances else f"{len(r.mismatches())} line(s) off"
        except hardmoney.UnsupportedForm:
            recon = "n/a"
        coverage = f"{rec.get('coverage_start_date')} to {rec.get('coverage_end_date')}"
        print(f"{rec['file_number']:>11} {rec.get('report_type', ''):6} {coverage:23} "
              f"{filing.summary.amount('col_a_total_receipts')!s:>12}  {len(v.errors):6} {len(v.warnings):8}  {recon}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
