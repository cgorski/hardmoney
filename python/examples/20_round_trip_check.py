"""20. Round-trip check: parse, write, parse again, compare every field.

Shows: the writer's guarantee. ``to_fec()`` produces the canonical form
(CRLF line endings, every record at full width, one wrapping quote pair
and surrounding padding removed), so the bytes usually differ from the
input, but ``parse(to_fec(parse(f)))`` has the same header, cover, and
body lines field for field, and writing it again gives the same bytes.

Run:
    python examples/20_round_trip_check.py [path/to/filing.fec ...]

Defaults to every fixture in tests/fixtures when no path is given.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
FIXTURES = REPO / "tests" / "fixtures"


def records(filing: hardmoney.Filing) -> list[tuple[str, str, dict[str, str]]]:
    return [(l.table, l.form_type, l.to_dict()) for l in filing.iter_lines()]


def round_trip(path: Path) -> bool:
    original = path.read_bytes()
    first = hardmoney.parse(original)
    written = first.to_fec()
    second = hardmoney.parse(written)

    same_header = second.header == first.header and second.version == first.version
    same_cover = second.summary.to_dict() == first.summary.to_dict()
    same_lines = records(second) == records(first)
    idempotent = second.to_fec() == written
    ok = same_header and same_cover and same_lines and idempotent

    bytes_note = "identical bytes" if written == original else f"{len(original)} -> {len(written)} bytes"
    print(f"{'ok  ' if ok else 'FAIL'} {path.name:28} {first.form_type:5} v{first.version:4} "
          f"{len(first.lines):5} line(s)  {bytes_note}")
    if not ok:
        print(f"     header {same_header}, cover {same_cover}, lines {same_lines}, idempotent {idempotent}")
    return ok


def main(argv: Sequence[str]) -> int:
    paths = [Path(a) for a in argv] or sorted(FIXTURES.glob("*.fec"))
    results = [round_trip(p) for p in paths]
    print(f"\n{results.count(True)} of {len(results)} filing(s) round-trip field for field")
    return 0 if all(results) else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
