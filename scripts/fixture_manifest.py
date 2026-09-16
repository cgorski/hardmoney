#!/usr/bin/env python3
"""Regenerate or check tests/fixtures/MANIFEST.sha256.

The manifest records the SHA-256 of every file under tests/fixtures/ as it
is in git's index (what `git add` staged, so a new fixture is included as
soon as it is added), never the working tree: a fixture rewritten on disk
by a checkout with core.autocrlf=true or by an editor must not be able to
launder itself into the manifest. tests/corpus_shape.rs compares the
working tree against it.

    python3 scripts/fixture_manifest.py          # rewrite the manifest
    python3 scripts/fixture_manifest.py --check  # exit 1 if it is stale

Standard library only.
"""

from __future__ import annotations

import hashlib
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "tests" / "fixtures" / "MANIFEST.sha256"

HEADER = """\
# SHA-256 of every file under tests/fixtures/, as committed. Checked by
# tests/corpus_shape.rs: a fixture that changes on disk (a checkout with
# core.autocrlf=true rewriting a CRLF filing, an editor saving it, a
# partial download) fails the suite. Regenerate on purpose only, after
# `git add`-ing a new or intentionally changed fixture:
#   python3 scripts/fixture_manifest.py
# Provenance of the filings themselves: tests/fixtures/ORACLE_NOTES.md,
# chain/README.md, rad/README.md, golden/MANIFEST.json.
"""


def staged_fixtures() -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "--", "tests/fixtures"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return sorted(p for p in out.splitlines() if p and not p.endswith("MANIFEST.sha256"))


def staged_bytes(path: str) -> bytes:
    return subprocess.run(
        ["git", "show", f":{path}"], cwd=ROOT, check=True, capture_output=True
    ).stdout


def render() -> str:
    lines = [HEADER]
    for path in staged_fixtures():
        digest = hashlib.sha256(staged_bytes(path)).hexdigest()
        lines.append(f"{digest}  {path}\n")
    return "".join(lines)


def main(argv: list[str]) -> int:
    text = render()
    if "--check" in argv:
        current = MANIFEST.read_text() if MANIFEST.exists() else ""
        if current == text:
            print(f"{MANIFEST.relative_to(ROOT)}: current ({text.count(chr(10)) - HEADER.count(chr(10))} entries)")
            return 0
        print(f"{MANIFEST.relative_to(ROOT)}: stale; run python3 scripts/fixture_manifest.py", file=sys.stderr)
        return 1
    MANIFEST.write_text(text)
    print(f"wrote {MANIFEST.relative_to(ROOT)} ({text.count(chr(10)) - HEADER.count(chr(10))} entries)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
