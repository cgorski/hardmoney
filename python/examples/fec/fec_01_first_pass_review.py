"""FEC 01. First-pass review of one report, in the order an analyst reads it.

Shows: ``validate()`` and ``reconcile()`` on one filing, reported as the
Reports Analysis Division's opening questions: would the file be accepted;
which cover-page lines disagree with their schedules and by how much; do
memo entries account for the gap, or is one transaction on the cover and
not on the schedule; which fields the FEC asks for are blank; are any
transaction ids duplicated or dangling. The default is a real, accepted
state-party amendment whose line 11(c) is $200 above its Schedule A.

Run:
    python examples/fec/fec_01_first_pass_review.py [filing.fec | filing_id]
    (default tests/fixtures/rad/F3XA_2011912.fec; an id is fetched from docquery.fec.gov)
"""

from __future__ import annotations

import sys
from decimal import Decimal
from pathlib import Path
from typing import Optional, Sequence

import hardmoney

DEFAULT = Path(__file__).resolve().parents[3] / "tests" / "fixtures" / "rad" / "F3XA_2011912.fec"
BLANK = {"required_field_empty", "conditionally_required_field_empty", "recommended_field_empty"}
IDS = {"duplicate_transaction_id", "back_reference_not_found"}


def amount(line: hardmoney.Line) -> Optional[Decimal]:
    return next((line.amount(f) for f in ("contribution_amount", "expenditure_amount") if f in line), None)


def explain(filing: hardmoney.Filing, c: hardmoney.LineCheck) -> None:
    if "sum of" not in c.rule:
        print("      a formula over other cover lines: the cover page is inconsistent with itself")
        return
    key = c.line.replace("(", "").replace(")", "").upper()  # 11(a)(i) -> 11AI, as in SA11AI
    lines = [l for l in filing.iter_lines() if l.form_type[2:] == key]
    memos = sum((amount(l) or Decimal(0) for l in lines if l.is_memo), Decimal(0))
    gap = abs(c.delta)
    print(f"      {len(lines)} schedule line(s) carry this line number; memo entries among them total {memos}"
          + ("; the gap equals the memo entries: the filer counted them in the total" if memos == gap else ""))
    hits = [f"line {l.line_no} {l['transaction_id']}" for l in lines if not l.is_memo and amount(l) == gap]
    print(f"      {', '.join(hits)} is exactly {gap}" if hits else
          f"      the {'cover page carries' if c.delta > 0 else 'schedule itemizes'} {gap} that the other side does not")


def review(filing: hardmoney.Filing) -> None:
    cover, hdr = filing.summary, filing.header
    amends = f"; amends {filing.amends_filing}, amendment {hdr['report_number']}" if filing.is_amendment else ""
    print(f"{filing.form_type} v{filing.version}: {cover.get('committee_name')} ({cover.get('filer_committee_id_number')})\n"
          f"report {cover.get('report_code')}, {cover.get('coverage_from_date')} to {cover.get('coverage_through_date')}{amends}\n"
          f"software: {hdr['soft_name']} {hdr['soft_ver']}; {len(filing.lines)} body line(s), {len(filing.skipped)} skipped\n")
    v = filing.validate()
    print(f"acceptance rules: {len(v.errors)} error(s), {len(v.warnings)} warning(s); "
          f"the FEC would {'accept' if v.is_acceptable else 'REJECT'} this file")
    for title, rules in (("fields the FEC asks for that are blank", BLANK), ("transaction id problems", IDS)):
        hits = [f for f in v.findings if f.rule in rules]
        print(f"{title}: {len(hits)}")
        for f in hits[:8]:
            print(f"  {f.severity:7} line {f.line_no} {f.form_type} {f.field}: {f.message}")
    try:
        r = filing.reconcile()
    except hardmoney.UnsupportedForm as e:
        print(f"\ncover page vs schedules: {e}")
        return
    print(f"\ncover page vs schedules ({r.form}, {len(r)} checks): {len(r.mismatches())} line(s) disagree")
    for c in r.mismatches():
        print(f"  col {c.column} line {c.line:<10} reported {c.reported!s:>12} expected {c.expected!s:>12} delta {c.delta:+}  {c.rule}")
        explain(filing, c)


def main(argv: Sequence[str]) -> int:
    arg = argv[0] if argv else str(DEFAULT)
    review(hardmoney.fetch(int(arg)) if arg.isdigit() else hardmoney.parse_file(arg, lenient=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
