"""13. Explain a finding with the FEC's own field specification.

Shows: joining a ``Finding`` to ``hardmoney.field_spec(table, field)``.
A finding names a form-type token (``SA11AI``) and a field
(``contributor_last_name``); the spec is keyed by table (``SchA``), so the
example finds the line the finding is about and takes its ``table``. The
spec carries the FEC's description, type, maximum length, required level,
rule text, and any allowed values or pattern, which is what a filer needs
to fix the field.

Run:
    python examples/13_explain_finding.py [path/to/filing.fec]

Defaults to tests/fixtures/invalid/field_too_long.fec.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any, Optional, Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "invalid" / "field_too_long.fec"


def table_of(filing: hardmoney.Filing, finding: hardmoney.Finding) -> Optional[str]:
    """The format table of the line a finding points at."""
    if finding.line_no == 1:
        return "HDR"
    if finding.line_no == 2:
        return filing.summary.table
    for line in filing.iter_lines():
        if line.line_no == finding.line_no:
            return line.table
    return None  # a skipped (unparseable) line


def value_of(filing: hardmoney.Filing, finding: hardmoney.Finding) -> Optional[str]:
    if finding.field is None:
        return None
    if finding.line_no == 2:
        return filing.summary.get(finding.field)
    for line in filing.iter_lines():
        if line.line_no == finding.line_no:
            return line.get(finding.field)
    return None


def explain(finding: hardmoney.Finding, spec: Optional[dict[str, Any]], value: Optional[str]) -> None:
    print(f"{finding.severity.upper()} line {finding.line_no} {finding.form_type} "
          f"{finding.field or ''} [{finding.rule}]")
    print(f"  message:     {finding.message}")
    if value is not None:
        shown = value if len(value) <= 60 else value[:57] + "..."
        print(f"  value:       {shown!r} ({len(value)} chars)")
    if spec is None:
        print("  spec:        (the current spec does not document this field)")
        return
    print(f"  description: {spec['description']}")
    print(f"  type:        {spec['kind']}, max length {spec['max_len']}, required: {spec['required']}")
    if spec["condition"]:
        print(f"  condition:   {spec['condition']}")
    if spec["rule"]:
        print(f"  FEC rule:    {' '.join(spec['rule'].split())}")
    if spec["allowed_values"]:
        print(f"  allowed:     {', '.join(spec['allowed_values'])}")
    if spec["pattern"]:
        print(f"  pattern:     {spec['pattern']}")
    if spec["sample"]:
        print(f"  sample:      {spec['sample']}")


def main(argv: Sequence[str]) -> int:
    path = Path(argv[0]) if argv else DEFAULT
    filing = hardmoney.parse_file(path, lenient=True)
    v = filing.validate()
    print(f"{path.name}: {len(v.errors)} error(s), {len(v.warnings)} warning(s)\n")
    for finding in v.errors:
        table = table_of(filing, finding)
        spec = hardmoney.field_spec(table, finding.field) if table and finding.field else None
        explain(finding, spec, value_of(filing, finding))
        print()
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
