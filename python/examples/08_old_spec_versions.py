"""08. Old spec versions: one set of field names from 3.0 through 8.5.

Shows: parsing a 2001 filing (spec 3.00), ``Filing.version``, the extra
``name_delim`` header key on 3.x-5.x filings, and that canonical field
names are stable across versions even though the column positions move.
The same function that sums ``contribution_amount`` on a 2026 filing sums
it on a 2001 one; you never look at column numbers.

The exception is worth knowing: the FEC split the single
``contributor_name`` field into last/first/middle/prefix/suffix and
``contributor_organization_name`` in spec 5.x, so a 3.0 line has
``contributor_name`` ("Last^First^Prefix^Suffix") and an 8.5 line has
the split fields. ``layout()`` tells you which names a version has.

Run:
    python examples/08_old_spec_versions.py [old.fec] [new.fec]

Defaults to tests/fixtures/F3XA_27789_v3.fec and F3XN_2011831.fec.
"""

from __future__ import annotations

import sys
from decimal import Decimal
from pathlib import Path
from typing import Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
OLD = REPO / "tests" / "fixtures" / "F3XA_27789_v3.fec"
NEW = REPO / "tests" / "fixtures" / "F3XN_2011831.fec"

SHOW = ["contribution_date", "contribution_amount", "contribution_aggregate",
        "contributor_city", "contributor_state", "contributor_employer", "contributor_occupation"]


def itemized_total(filing: hardmoney.Filing) -> Decimal:
    """Version-independent: the field is ``contribution_amount`` everywhere."""
    return sum((l.amount("contribution_amount") or Decimal(0)
                for l in filing.lines_for("SchA") if not l.is_memo), Decimal("0"))


def column_of(table: str, version: str, field: str) -> int:
    return dict(hardmoney.layout(table, version))[field]


def main(argv: Sequence[str]) -> int:
    old = hardmoney.parse_file(Path(argv[0]) if argv else OLD)
    new = hardmoney.parse_file(Path(argv[1]) if len(argv) > 1 else NEW)

    for filing in (old, new):
        print(f"{filing.form_type} spec {filing.version}: header keys {sorted(filing.header)}")
        print(f"  software: {filing.header['soft_name']} {filing.header['soft_ver']}; "
              f"raw version string {filing.header['fec_version_raw']!r}")

    print("\nthe same field names on the first Schedule A line of each:")
    print(f"  {'field':24} {'spec ' + old.version:>28} {'spec ' + new.version:>28}")
    a, b = old.lines_for("SchA")[0], new.lines_for("SchA")[0]
    for field in SHOW:
        print(f"  {field:24} {a[field]!r:>28} {b[field]!r:>28}")

    print("\nbut in different columns:")
    for field in ("contribution_date", "contribution_amount", "contributor_employer"):
        print(f"  {field:24} column {column_of('SchA', old.version, field):2} in {old.version}, "
              f"column {column_of('SchA', new.version, field):2} in {new.version}")

    old_names = {n for n, _ in hardmoney.layout("SchA", old.version)}
    new_names = {n for n, _ in hardmoney.layout("SchA", new.version)}
    print(f"\nSchedule A fields: {len(old_names)} in {old.version}, {len(new_names)} in {new.version}, "
          f"{len(old_names & new_names)} shared")
    print(f"  only in {old.version}: {sorted(old_names - new_names)}")
    print(f"  only in {new.version}: {sorted(new_names - old_names)}")
    print(f"  {old.version} name field: {a.get('contributor_name')!r}")
    print(f"  {new.version} name fields: {b.get('contributor_last_name')!r}, {b.get('contributor_first_name')!r}")

    print("\none function, both versions:")
    for filing in (old, new):
        total = itemized_total(filing)
        cover = filing.summary.amount("col_a_individuals_itemized")
        print(f"  spec {filing.version}: itemized sum {total} vs cover 11(a)(i) {cover} -> {total == cover}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
