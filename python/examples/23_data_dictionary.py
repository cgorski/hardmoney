"""23. Generate a Markdown data dictionary for one table.

Shows: ``hardmoney.field_spec(table, name)`` for every field of a table's
layout at the bundled spec version. Each spec carries the FEC's own
description, type, maximum length, required level, rule text, allowed
values, and pattern. Fields the current spec does not document (they
exist in the layout, so the parser reads them) are marked as such.

Run:
    python examples/23_data_dictionary.py [table] > SchA.md

Defaults to SchA.
"""

from __future__ import annotations

import sys
from typing import Any, Optional, Sequence

import hardmoney

REQUIRED = {"error": "yes", "warning": "recommended", "conditional": "conditional", "none": ""}


def cell(text: Optional[str], limit: int = 60) -> str:
    if not text:
        return ""
    flat = " ".join(str(text).split()).replace("|", "\\|")
    return flat if len(flat) <= limit else flat[: limit - 3] + "..."


def row(name: str, col: int, spec: Optional[dict[str, Any]]) -> str:
    if spec is None:
        return f"| {col} | `{name}` | | | | (not in the {hardmoney.BUNDLED_SPEC_VERSION} spec) | |"
    kind = spec["kind"] + (f"({spec['max_len']})" if spec["max_len"] is not None else "")
    constraint = spec["rule"] or spec["condition"]
    if spec["allowed_values"]:
        constraint = "one of " + ", ".join(spec["allowed_values"])
    elif spec["pattern"]:
        constraint = f"pattern `{spec['pattern']}`"
    return (f"| {col} | `{name}` | {kind} | {REQUIRED.get(spec['required'], spec['required'])} "
            f"| {cell(spec['description'])} | {cell(constraint, 70)} | {cell(spec['sample'], 20)} |")


def main(argv: Sequence[str]) -> int:
    table = argv[0] if argv else "SchA"
    version = hardmoney.BUNDLED_SPEC_VERSION
    layout = hardmoney.layout(table, version)

    print(f"# {table} (FEC spec {version})\n")
    print(f"{len(layout)} fields, in column order. Column numbers are 0-based positions "
          f"in the `.fec` record.\n")
    print("| col | field | type | required | description | rule / values | sample |")
    print("|---|---|---|---|---|---|---|")
    documented = 0
    for name, col in layout:
        spec = hardmoney.field_spec(table, name)
        documented += spec is not None
        print(row(name, col, spec))
    print(f"\n{documented} of {len(layout)} fields documented in the {version} spec.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
