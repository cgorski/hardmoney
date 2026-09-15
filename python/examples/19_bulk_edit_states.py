"""19. Bulk edit: upper-case every state code, re-validate, and write.

Shows: iterating every line of every table and editing the fields whose
names end in ``_state`` (``contributor_state`` on Schedule A,
``payee_state`` on Schedule B, ``state`` on the cover, ...) through
``Line.keys()`` and ``Line.set``. The FEC's state field is a two-letter
code, and a filer's software occasionally emits ``va`` or ``Va``.

The fixtures were all accepted by the FEC and already upper-case, so with
no arguments the example first lower-cases the states of one fixture in
memory (labelled as such) and then runs the fixer on that.

Run:
    python examples/19_bulk_edit_states.py [in.fec] [out.fec]
"""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path
from typing import Iterator, Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEMO_SOURCE = REPO / "tests" / "fixtures" / "F3XN_2011835.fec"


def every_line(filing: hardmoney.Filing) -> Iterator[hardmoney.Line]:
    yield filing.summary
    yield from filing.iter_lines()


def state_fields(line: hardmoney.Line) -> list[str]:
    return [name for name in line.keys() if name == "state" or name.endswith("_state")]


def upper_case_states(filing: hardmoney.Filing) -> list[tuple[int, str, str, str]]:
    """Fix every state code in place; return (line_no, field, old, new) per change."""
    changes = []
    for line in every_line(filing):
        for name in state_fields(line):
            value = line[name]
            if value and value != value.upper():
                line.set(name, value.upper())
                changes.append((line.line_no, name, value, value.upper()))
    return changes


def demo_input() -> bytes:
    """A copy of a real filing with its state codes lower-cased."""
    filing = hardmoney.parse_file(DEMO_SOURCE)
    for line in every_line(filing):
        for name in state_fields(line):
            if line[name]:
                line.set(name, line[name].lower())
    return filing.to_fec()


def main(argv: Sequence[str]) -> int:
    if argv:
        src = Path(argv[0])
        filing = hardmoney.parse_file(src)
        label = src.name
    else:
        filing = hardmoney.parse(demo_input())
        label = f"{DEMO_SOURCE.name} with its states lower-cased (demo input)"
    out = Path(argv[1]) if len(argv) > 1 else Path(tempfile.mkdtemp()) / "states-fixed.fec"

    print(f"{label}: {filing.form_type}, {len(filing.lines)} body line(s)")
    print(f"validation before: {len(filing.validate().errors)} error(s), "
          f"{len(filing.validate().warnings)} warning(s)\n")

    changes = upper_case_states(filing)
    for line_no, name, old, new in changes:
        print(f"  line {line_no:>3} {name:20} {old!r} -> {new!r}")
    print(f"\n{len(changes)} field(s) changed")

    v = filing.validate()
    print(f"validation after:  {len(v.errors)} error(s), {len(v.warnings)} warning(s)")
    out.write_bytes(filing.to_fec())
    print(f"wrote {out}")

    still_lower = [c for c in upper_case_states(hardmoney.parse_file(out))]
    print(f"re-parsed output has {len(still_lower)} lower-case state(s) left")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
