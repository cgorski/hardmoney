"""31. Poll the e-file feed once and process the new filings.

Shows: ``hardmoney efile watch --once --json`` polls the FEC's electronic
filing RSS feed, prints one JSON object per filing not seen before (JSON
Lines), and records the ids it has seen under the cache directory so the
next poll reports only newer ones. Python reads the lines, picks the
filings it cares about, downloads them with ``hardmoney.fetch``, and
validates and reconciles each. Run it from cron or a loop with a
persistent cache directory and you have a monitor.

Needs the ``hardmoney`` CLI (on PATH, at ``$HARDMONEY_BIN``, or built in
the repository) and network access; no API key. The cache directory
comes from ``HARDMONEY_CACHE_DIR`` (the CLI's own variable), so point it
somewhere disposable to try this without touching your real seen-list.
In the test suite it runs only when ``HARDMONEY_NETWORK_TESTS`` is set.

Run:
    HARDMONEY_CACHE_DIR=/tmp/hm-demo python examples/31_efile_watch.py [form_type] [max_to_process]

Defaults to F3X and 3.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any, Iterator, Optional, Sequence

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


def poll_once(cli: str, form_type: str) -> Iterator[dict[str, Any]]:
    """Every filing in the feed not seen before, newest last."""
    cmd = [cli, "efile", "watch", "--once", "--json", "--form-type", form_type]
    result = subprocess.run(cmd, check=True, capture_output=True, text=True)
    for line in result.stdout.splitlines():
        if line.strip():
            yield json.loads(line)


def process(entry: dict[str, Any]) -> str:
    filing = hardmoney.fetch(int(entry["filing_id"]))
    v = filing.validate()
    try:
        r = filing.reconcile()
        recon = "balances" if r.balances else f"{len(r.mismatches())} cover line(s) disagree"
    except hardmoney.UnsupportedForm:
        recon = "no cover-page rules"
    return (f"{len(filing.lines)} line(s), {len(v.errors)} error(s), {len(v.warnings)} warning(s), {recon}")


def main(argv: Sequence[str]) -> int:
    cli = hardmoney_cli()
    if cli is None:
        print("hardmoney CLI not found (set HARDMONEY_BIN or cargo build --all-features)")
        return 0
    form_type = argv[0] if argv else "F3X"
    limit = int(argv[1]) if len(argv) > 1 else 3

    new = list(poll_once(cli, form_type))
    cache = os.environ.get("HARDMONEY_CACHE_DIR", "the default cache dir")
    print(f"{len(new)} new {form_type} filing(s) in the feed (seen-list in {cache})\n")
    for entry in new[:limit]:
        print(f"{entry['filing_id']} {entry['form_type']:5} {entry['committee_id']} "
              f"{entry.get('report_type', '')!s:20} {entry['committee_name'][:40]}")
        try:
            print(f"    {process(entry)}")
        except hardmoney.FecError as e:
            print(f"    failed: {e}")
    if len(new) > limit:
        print(f"... {len(new) - limit} more not processed (raise the limit)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
