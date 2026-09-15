"""10. Validate one filing and print the findings grouped by severity.

Shows: ``Filing.validate()`` and the ``Validation`` / ``Finding`` objects.
``severity`` is ``"error"`` (the FEC would reject the filing) or
``"warning"`` (reported, but accepted); ``rule`` is a stable snake_case
name you can switch on; ``line_no``, ``form_type``, ``field``, and
``message`` say where and what.

Run:
    python examples/10_validate_by_severity.py [path/to/filing.fec]

Defaults to tests/fixtures/invalid/bad_dates_and_amounts.fec, a copy of a
real filing with five deliberate defects.
"""

from __future__ import annotations

import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "invalid" / "bad_dates_and_amounts.fec"


def main(argv: Sequence[str]) -> int:
    path = Path(argv[0]) if argv else DEFAULT
    # lenient=True so a filing with an unparseable line still gets validated;
    # such lines surface as unrecognized_form_type warnings.
    filing = hardmoney.parse_file(path, lenient=True)
    v = filing.validate()

    verdict = "acceptable (no errors)" if v.is_acceptable else "would be REJECTED by the FEC"
    print(f"{path.name}: {len(v.errors)} error(s), {len(v.warnings)} warning(s); {verdict}\n")

    by_severity: dict[str, list[hardmoney.Finding]] = defaultdict(list)
    for finding in v:
        by_severity[finding.severity].append(finding)

    for severity in ("error", "warning"):
        findings = by_severity.get(severity, [])
        if not findings:
            continue
        print(f"{severity.upper()}S ({len(findings)}):")
        rules = Counter(f.rule for f in findings)
        for rule, n in rules.most_common():
            print(f"  {rule} x{n}")
        for f in findings:
            where = f"line {f.line_no} {f.form_type}" + (f" {f.field}" if f.field else "")
            print(f"    {where}: {f.message}")
        print()

    # str(v) is the same list in the one-line-per-finding format the CLI
    # (`hardmoney validate`) prints, e.g. "ERROR line 2 F3XA date_signed: ...".
    return 0 if v.is_acceptable else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
