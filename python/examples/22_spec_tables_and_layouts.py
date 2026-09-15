"""22. The spec from Python: tables, a version's layout, and a diff of two versions.

Shows: ``hardmoney.tables()``, ``hardmoney.layout(table, version)``, and
``BUNDLED_SPEC_VERSION``. A layout is the list of ``(field, column)``
pairs the parser uses for a table at one spec version. Comparing two
versions' layouts as sets tells you which fields were added, removed, or
moved, which is what you need to know before running one query over
filings from different years.

Run:
    python examples/22_spec_tables_and_layouts.py [table] [old_version] [new_version]

Defaults to SchA, 7.0, and 8.5.
"""

from __future__ import annotations

import sys
from typing import Sequence

import hardmoney


def diff_layouts(table: str, old: str, new: str) -> None:
    a = dict(hardmoney.layout(table, old))
    b = dict(hardmoney.layout(table, new))
    added = sorted(set(b) - set(a), key=b.__getitem__)
    removed = sorted(set(a) - set(b), key=a.__getitem__)
    moved = [(name, a[name], b[name]) for name in a if name in b and a[name] != b[name]]

    print(f"{table}: {len(a)} fields in {old}, {len(b)} in {new}")
    print(f"  added in {new}:   {', '.join(f'{n} (col {b[n]})' for n in added) or 'none'}")
    print(f"  removed in {new}: {', '.join(f'{n} (col {a[n]})' for n in removed) or 'none'}")
    if moved:
        print(f"  moved: {len(moved)} field(s), e.g. " +
              ", ".join(f"{n} {c1}->{c2}" for n, c1, c2 in moved[:4]))
    else:
        print("  moved: none")


def main(argv: Sequence[str]) -> int:
    table = argv[0] if argv else "SchA"
    old = argv[1] if len(argv) > 1 else "7.0"
    new = argv[2] if len(argv) > 2 else hardmoney.BUNDLED_SPEC_VERSION

    tables = hardmoney.tables()
    print(f"bundled spec version: {hardmoney.BUNDLED_SPEC_VERSION}")
    print(f"{len(tables)} tables: {' '.join(tables)}\n")

    layout = hardmoney.layout(table, new)
    print(f"{table} layout at {new} ({len(layout)} fields), first 12:")
    for name, col in layout[:12]:
        print(f"  {col:3} {name}")
    print("  ...\n")

    diff_layouts(table, old, new)
    print()
    diff_layouts(table, "5.3", new)

    print("\nversions that have no layout raise FecError:")
    try:
        hardmoney.layout(table, "9.9")
    except hardmoney.FecError as e:
        print(f"  {e}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
