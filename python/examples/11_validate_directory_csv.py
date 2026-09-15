"""11. Validate every filing in a directory and write a CSV report.

Shows: walking a directory of ``.fec`` files, validating each, and
flattening the findings into rows with the standard-library ``csv``
module. A filing that cannot be parsed at all becomes one row with
severity ``parse_error`` rather than stopping the run.

Run:
    python examples/11_validate_directory_csv.py [directory] [report.csv]

Defaults to tests/fixtures (searched recursively, so the deliberately
broken copies under invalid/ are included) and prints the CSV to stdout
when no report path is given.
"""

from __future__ import annotations

import csv
import sys
from pathlib import Path
from typing import Iterator, Sequence, TextIO

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT_DIR = REPO / "tests" / "fixtures"
COLUMNS = ["file", "form_type", "version", "severity", "rule", "line_no", "record", "field", "message"]


def rows_for(path: Path, root: Path) -> Iterator[dict[str, object]]:
    rel = str(path.relative_to(root))
    try:
        filing = hardmoney.parse_file(path, lenient=True)
    except hardmoney.FecError as e:
        yield {"file": rel, "form_type": "", "version": "", "severity": "parse_error",
               "rule": "", "line_no": e.line_no or "", "record": "", "field": "", "message": str(e)}
        return
    for f in filing.validate():
        yield {"file": rel, "form_type": filing.form_type, "version": filing.version,
               "severity": f.severity, "rule": f.rule, "line_no": f.line_no,
               "record": f.form_type, "field": f.field or "", "message": f.message}


def write_report(root: Path, out: TextIO) -> tuple[int, int, int]:
    """Write the CSV; return (files, errors, warnings)."""
    writer = csv.DictWriter(out, fieldnames=COLUMNS, lineterminator="\n")
    writer.writeheader()
    files = errors = warnings = 0
    for path in sorted(root.rglob("*.fec")):
        files += 1
        for row in rows_for(path, root):
            writer.writerow(row)
            if row["severity"] == "error":
                errors += 1
            elif row["severity"] == "warning":
                warnings += 1
    return files, errors, warnings


def main(argv: Sequence[str]) -> int:
    root = Path(argv[0]) if argv else DEFAULT_DIR
    if len(argv) > 1:
        with open(argv[1], "w", newline="", encoding="utf-8") as out:
            files, errors, warnings = write_report(root, out)
        print(f"{files} file(s) under {root}: {errors} error(s), {warnings} warning(s) -> {argv[1]}")
    else:
        files, errors, warnings = write_report(root, sys.stdout)
        print(f"# {files} file(s) under {root}: {errors} error(s), {warnings} warning(s)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
