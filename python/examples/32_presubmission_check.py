"""32. A pre-submission check function for a web application.

Shows: one pure function, ``check_fec_bytes(data) -> dict``, that a
Django (or any other) view can call on an uploaded ``.fec`` before the
filer submits it to the FEC. It returns the validation errors and
warnings, and, for the periodic reports (F3X, F3, F3P), the cover-page
lines that do not agree with the schedules. No Django import; the
function only needs ``hardmoney`` and returns JSON-ready data (amounts
as strings, so they stay exact through ``JsonResponse``).

Where it plugs in::

    # views.py
    from django.http import JsonResponse
    from .fec_check import check_fec_bytes

    def precheck(request):
        upload = request.FILES["filing"]
        report = check_fec_bytes(upload.read())
        return JsonResponse(report, status=200 if report["acceptable"] else 422)

Run:
    python examples/32_presubmission_check.py [path/to/filing.fec]

Defaults to tests/fixtures/invalid/bad_dates_and_amounts.fec.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any, Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "invalid" / "bad_dates_and_amounts.fec"


def _finding(f: hardmoney.Finding) -> dict[str, Any]:
    return {"rule": f.rule, "line": f.line_no, "record": f.form_type, "field": f.field, "message": f.message}


def _mismatch(c: hardmoney.LineCheck) -> dict[str, Any]:
    return {
        "column": c.column,
        "line": c.line,
        "field": c.field,
        "reported": None if c.reported is None else str(c.reported),
        "expected": str(c.expected),
        "delta": str(c.delta),
        "relation": c.relation,
        "rule": c.rule,
    }


def check_fec_bytes(data: bytes) -> dict[str, Any]:
    """Validate and reconcile an uploaded ``.fec``; never raises for bad input.

    ``acceptable`` mirrors the FEC's decision (no error-severity findings).
    ``reconciliation`` is ``None`` for forms without cover-page rules.
    """
    try:
        filing = hardmoney.parse(data, lenient=True)
    except hardmoney.FecError as e:
        return {
            "parsed": False,
            "acceptable": False,
            "error": str(e),
            "line": e.line_no,
            "errors": [],
            "warnings": [],
            "reconciliation": None,
        }

    v = filing.validate()
    report: dict[str, Any] = {
        "parsed": True,
        "form_type": filing.form_type,
        "version": filing.version,
        "committee_id": filing.summary.get("filer_committee_id_number"),
        "body_lines": len(filing.lines),
        "skipped_lines": len(filing.skipped),
        "acceptable": v.is_acceptable,
        "errors": [_finding(f) for f in v.errors],
        "warnings": [_finding(f) for f in v.warnings],
        "reconciliation": None,
    }
    try:
        r = filing.reconcile()
    except hardmoney.UnsupportedForm:
        return report
    report["reconciliation"] = {
        "form": r.form,
        "checks": len(r),
        "balances": r.balances,
        "mismatches": [_mismatch(c) for c in r.mismatches()],
    }
    return report


def main(argv: Sequence[str]) -> int:
    path = Path(argv[0]) if argv else DEFAULT
    report = check_fec_bytes(path.read_bytes())
    print(json.dumps(report, indent=2))
    return 0 if report["acceptable"] else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
