"""18. Fix a field with ``Line.set`` and write the filing back with ``to_fec()``.

Shows: the edit-validate-write loop. A ``Line`` is a handle into its
``Filing``, so ``set`` on any handle is what ``to_fec()`` emits. The
example takes a filing with a too-long field, trims it to the maximum the
FEC's spec allows (from ``field_spec``), confirms with ``validate()`` that
the error is gone, writes the bytes, and re-parses the output to verify.

Run:
    python examples/18_fix_field_and_write.py [in.fec] [out.fec]

Defaults to tests/fixtures/invalid/field_too_long.fec and writes to a
temporary directory when no output path is given.
"""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path
from typing import Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "invalid" / "field_too_long.fec"


def line_at(filing: hardmoney.Filing, line_no: int) -> hardmoney.Line:
    if line_no == 2:
        return filing.summary
    for line in filing.iter_lines():
        if line.line_no == line_no:
            return line
    raise LookupError(f"no parsed line {line_no}")


def main(argv: Sequence[str]) -> int:
    src = Path(argv[0]) if argv else DEFAULT
    out = Path(argv[1]) if len(argv) > 1 else Path(tempfile.mkdtemp()) / f"{src.stem}.fixed.fec"

    filing = hardmoney.parse_file(src)
    before = filing.validate()
    print(f"{src.name}: {len(before.errors)} error(s) before\n")

    fixed = 0
    for finding in before.errors:
        if finding.rule != "field_too_long" or finding.field is None:
            continue
        line = line_at(filing, finding.line_no)
        spec = hardmoney.field_spec(line.table, finding.field)
        if spec is None or spec["max_len"] is None:
            continue
        old = line[finding.field]
        new = old[: spec["max_len"]].rstrip()
        line.set(finding.field, new)  # the parser's trimming applies to set() too
        fixed += 1
        print(f"line {finding.line_no} {finding.field}: {len(old)} -> {len(new)} chars "
              f"(max {spec['max_len']}); now {new[:40]!r}")

    after = filing.validate()
    print(f"\n{fixed} field(s) fixed; {len(after.errors)} error(s) after; acceptable: {after.is_acceptable}")

    data = filing.to_fec()  # canonical .fec bytes (CRLF, full record width)
    out.write_bytes(data)
    print(f"wrote {len(data)} bytes to {out}")

    again = hardmoney.parse_file(out)
    print(f"re-parsed: {again.form_type}, {len(again.lines)} line(s), "
          f"{len(again.validate().errors)} error(s)")
    for finding in before.errors:
        if finding.field:
            print(f"  line {finding.line_no} {finding.field} = {line_at(again, finding.line_no)[finding.field][:40]!r}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
