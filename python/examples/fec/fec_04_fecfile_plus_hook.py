"""FEC 04. One function FECfile+ could call from a Django view or a Celery task.

Shows: ``check_dot_fec(data) -> dict`` over the bytes FECfile+'s
``dot_fec_composer`` produces. It returns the whole-file validation in the
shape of ``fecfile_validate.ValidationResult`` (``errors`` / ``warnings``
with a ``path`` and a ``message``), the Column A schedule sums keyed the
way ``reports/form_3x/summary.py`` keys them (``line_11ai``, ``line_21b``,
including the lines that module stubs to zero: 18a, 21ai, 21aii, 30ai,
30aii), and any cover-page line that disagrees with its schedules. No
Django import; amounts are strings so they survive ``JsonResponse``.

    # web_services/summary/tasks.py, or a DRF view
    report = check_dot_fec(compose_dot_fec(report_id).encode("utf-8"))
    if report["errors"]: ...

Run:
    python examples/fec/fec_04_fecfile_plus_hook.py [filing.fec]   (default tests/fixtures/F3XA_2011814.fec)
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any, Sequence

import hardmoney

DEFAULT = Path(__file__).resolve().parents[3] / "tests" / "fixtures" / "F3XA_2011814.fec"


def line_key(label: str) -> str:
    """FEC line label -> summary.py key: 11(a)(i) -> line_11ai, 30(b) -> line_30b."""
    return "line_" + label.replace("(", "").replace(")", "")


def check_dot_fec(data: bytes) -> dict[str, Any]:
    """Validate and total a .fec the way the FEC's acceptance step and summary calculator would."""
    try:
        filing = hardmoney.parse(data, lenient=True)
    except hardmoney.FecError as e:
        return {"errors": [{"path": f"line {e.line_no}" if e.line_no else "file", "message": str(e)}], "warnings": []}
    v = filing.validate()
    out: dict[str, Any] = {
        "form_type": filing.form_type,
        "fec_version": filing.version,
        "errors": [{"path": f"{f.form_type}.{f.field or ''}".rstrip("."), "message": f.message} for f in v.errors],
        "warnings": [{"path": f"{f.form_type}.{f.field or ''}".rstrip("."), "message": f.message} for f in v.warnings],
        "column_a": {},
        "cover_disagrees": [],
    }
    try:
        r = filing.reconcile()
    except hardmoney.UnsupportedForm:
        return out
    for c in r.checks:
        if c.column == "A" and "sum of" in c.rule:  # the get_line("SA11AI", ...) sums, not the arithmetic
            out["column_a"][line_key(c.line)] = str(c.expected)
    out["cover_disagrees"] = [
        {"line": c.line, "column": c.column, "reported": str(c.reported), "expected": str(c.expected), "rule": c.rule}
        for c in r.mismatches()
    ]
    return out


def main(argv: Sequence[str]) -> int:
    path = Path(argv[0]) if argv else DEFAULT
    report = check_dot_fec(path.read_bytes())
    print(json.dumps(report, indent=2))
    return 0 if not report["errors"] else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
