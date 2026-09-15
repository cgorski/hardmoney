"""05. Lenient parsing: keep going past lines the parser cannot interpret.

Shows: ``parse(data, lenient=True)`` and ``Filing.skipped``. A strict
parse raises ``FecError`` at the first body line with an unknown form type
(or one with no column layout for the filing's spec version). A lenient
parse records such lines in ``filing.skipped`` and carries on, and
``validate()`` reports each of them as an ``unrecognized_form_type``
warning.

The fixtures were all accepted by the FEC, so none of them has such a
line. This example appends two bad lines to the file's bytes before
parsing, so there is something to skip.

Run:
    python examples/05_lenient_parsing.py [path/to/filing.fec]

Defaults to tests/fixtures/F3XA_2011827.fec when no path is given.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "F3XA_2011827.fec"
FS = b"\x1c"  # the FEC field separator


def main(argv: Sequence[str]) -> int:
    path = Path(argv[0]) if argv else DEFAULT
    data = path.read_bytes()
    if not data.endswith((b"\n", b"\r\n")):
        data += b"\r\n"
    # A made-up form type, then a line with no columns after its token.
    data += b"ZZZ" + FS + b"C00944124" + FS + b"garbage\r\n"
    data += b"SX99" + FS + b"C00944124\r\n"

    print("strict parse:")
    try:
        hardmoney.parse(data)
    except hardmoney.FecError as e:
        print(f"  FecError at line {e.line_no}: {e}")

    print("\nlenient parse:")
    filing = hardmoney.parse(data, lenient=True)
    print(f"  {len(filing.lines)} line(s) parsed, {len(filing.skipped)} skipped")
    for skipped in filing.skipped:
        print(f"  line {skipped['line_no']:>3} {skipped['form_type']:6} {skipped['reason']}")

    print("\nvalidate() reports each skipped line as a warning:")
    for finding in filing.validate().warnings:
        if finding.rule == "unrecognized_form_type":
            print(f"  {finding}")

    # The lines that did parse are intact, and to_fec() writes only them.
    original = hardmoney.parse_file(path)
    same = [l.to_dict() for l in filing.lines] == [l.to_dict() for l in original.lines]
    print(f"\nparsed lines identical to the original file's: {same}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
