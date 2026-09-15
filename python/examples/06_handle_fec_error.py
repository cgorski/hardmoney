"""06. Handling ``FecError`` and pointing at the offending line.

Shows: the exception hierarchy. ``hardmoney.FecError`` is a ``ValueError``
whose ``line_no`` is the 1-based physical line the error is about, or
``None`` when it is not about one line (a bad header, a missing cover
line). ``parse_file`` on a missing path raises ``FileNotFoundError``, not
``FecError``, because that is what Python code expects to catch.

With ``line_no`` in hand you can show the filer the raw line, which is
what this example does.

Run:
    python examples/06_handle_fec_error.py [path/to/filing.fec]

Defaults to tests/fixtures/F3XA_2011827.fec when no path is given. The
file itself is fine; the example derives broken inputs from it.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Optional, Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "F3XA_2011827.fec"


def raw_line(data: bytes, line_no: int) -> Optional[bytes]:
    """The ``line_no``-th physical line of the file, or None."""
    lines = data.splitlines()
    if 1 <= line_no <= len(lines):
        return lines[line_no - 1]
    return None


def try_parse(label: str, data: bytes) -> Optional[hardmoney.Filing]:
    print(f"{label}:")
    try:
        filing = hardmoney.parse(data)
    except hardmoney.FecError as e:
        print(f"  {type(e).__name__}: {e}")
        if e.line_no is None:
            print("  (not about one line)")
        else:
            bad = raw_line(data, e.line_no)
            shown = bad.decode("utf-8", "replace").replace("\x1c", "|") if bad is not None else "?"
            print(f"  line {e.line_no}: {shown[:70]}")
        return None
    print(f"  ok: {filing.form_type} with {len(filing.lines)} body line(s)")
    return filing


def main(argv: Sequence[str]) -> int:
    path = Path(argv[0]) if argv else DEFAULT
    data = path.read_bytes()
    lines = data.splitlines(keepends=True)

    try_parse("the file as is", data)

    # Body line with an unknown form type: the error names that line.
    broken = b"".join(lines[:3]) + b"BOGUS\x1cC00944124\x1cX\r\n" + b"".join(lines[3:])
    try_parse("\nan unknown form type spliced in as line 4", broken)

    # Header only: no cover line to interpret, so line_no is None.
    try_parse("\nthe header alone", lines[0])

    # Not a .fec file at all.
    try_parse("\nsomething that is not a filing", b"hello, world")

    print("\na missing path:")
    try:
        hardmoney.parse_file(path.with_name("does-not-exist.fec"))
    except FileNotFoundError as e:
        print(f"  FileNotFoundError (an OSError, not a FecError): {e}")

    # FecError is a ValueError, so a broad handler still catches it.
    try:
        hardmoney.parse(b"")
    except ValueError as e:
        print(f"\ncaught as ValueError: {type(e).__name__}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
