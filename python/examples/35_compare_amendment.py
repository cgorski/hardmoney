"""35. Compare two versions of a report: an original and its amendment.

Shows: diffing two filings field by field. The cover pages are compared
by field name; the schedules are matched on transaction id (the FEC
requires ids to be stable across amendments of a report), giving the
transactions added, removed, and changed. ``Filing.amends_filing`` and
``is_amendment`` come from the amendment's header.

Two fixture-repository details worth knowing: Schedule A calls the id
``transaction_id`` while Schedule B, C, D, E and TEXT call it
``transaction_id_number``; and ``str.splitlines()`` treats the FEC's
field separator (0x1c) as a line break, so split ``.fec`` text on the
CRLF the writer emits.

The repository has no original/amendment pair, so with one path (or
none) the example derives an amendment from that filing: one Schedule A
line removed, one added, one amount changed by $5.00 with the cover
page adjusted to match, and the header marked ``FEC-<id>`` amendment 1.

Run:
    python examples/35_compare_amendment.py original.fec amendment.fec
    python examples/35_compare_amendment.py [original.fec]

Defaults to tests/fixtures/F3XN_2011831.fec.
"""

from __future__ import annotations

import re
import sys
from decimal import Decimal
from pathlib import Path
from typing import Optional, Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "F3XN_2011831.fec"
FS = "\x1c"

# The Form 3X Column A lines that a change to 11(a)(i) flows into.
F3X_RECEIPT_CHAIN = [
    "col_a_individuals_itemized", "col_a_individual_contribution_total", "col_a_total_contributions",
    "col_a_total_receipts_recap", "col_a_total_federal_receipts", "col_a_total_receipts", "col_a_subtotal",
    "col_a_cash_on_hand_close_of_period", "col_a_total_contributions_recap", "col_a_net_contributions",
]


def transaction_id(line: hardmoney.Line) -> Optional[str]:
    return line.get("transaction_id") or line.get("transaction_id_number") or None


def describe(line: hardmoney.Line) -> str:
    who = (line.get("contributor_organization_name") or line.get("payee_organization_name")
           or line.get("contributor_last_name") or line.get("payee_last_name") or "")
    amount = line.amount("contribution_amount") if "contribution_amount" in line else (
        line.amount("expenditure_amount") if "expenditure_amount" in line else None)
    return f"{line.form_type} {who} {amount if amount is not None else ''}".rstrip()


def compare(original: hardmoney.Filing, amendment: hardmoney.Filing) -> None:
    print(f"original:  {original.form_type} v{original.version}, {len(original.lines)} line(s), "
          f"report id {original.header['report_id']!r}")
    print(f"amendment: {amendment.form_type} v{amendment.version}, {len(amendment.lines)} line(s), "
          f"report id {amendment.header['report_id']!r}, is_amendment={amendment.is_amendment}, "
          f"amends_filing={amendment.amends_filing}")

    print("\ncover page fields that changed:")
    a, b = original.summary, amendment.summary
    changed = [(k, a[k], b[k]) for k in a.keys() if k in b and a[k] != b[k]]
    for name, old, new in changed:
        delta = ""
        if a.amount(name) is not None and b.amount(name) is not None:
            delta = f"  (delta {b.amount(name) - a.amount(name):+})"
        print(f"  {name:40} {old!r} -> {new!r}{delta}")
    if not changed:
        print("  none")

    before = {transaction_id(l): l for l in original.iter_lines() if transaction_id(l)}
    after = {transaction_id(l): l for l in amendment.iter_lines() if transaction_id(l)}
    removed = sorted(set(before) - set(after))
    added = sorted(set(after) - set(before))
    modified = [t for t in sorted(set(before) & set(after)) if before[t].to_dict() != after[t].to_dict()]

    print(f"\ntransactions: {len(before)} before, {len(after)} after; "
          f"{len(removed)} removed, {len(added)} added, {len(modified)} changed")
    for t in removed:
        print(f"  - {t:20} {describe(before[t])}")
    for t in added:
        print(f"  + {t:20} {describe(after[t])}")
    for t in modified:
        x, y = before[t].to_dict(), after[t].to_dict()
        diffs = ", ".join(f"{k}: {x[k]!r} -> {y[k]!r}" for k in x if x[k] != y.get(k))
        print(f"  ~ {t:20} {diffs}")


def synthesize_amendment(original_path: Path) -> hardmoney.Filing:
    """An amendment of ``original_path`` with a few realistic changes."""
    f = hardmoney.parse_file(original_path)
    filing_id = int(re.findall(r"\d{4,}", original_path.stem)[-1])
    sched_a = f.lines_for("SchA")
    removed, template, changed = sched_a[0], sched_a[1], sched_a[2]

    # One amount corrected by $5.00, and the cover page adjusted to match.
    changed.set("contribution_amount", str(changed.amount("contribution_amount") + Decimal("5.00")))
    for field in F3X_RECEIPT_CHAIN:
        if field in f.summary and f.summary.amount(field) is not None:
            f.summary.set(field, str(f.summary.amount(field) + Decimal("5.00")))
    f.summary.set("form_type", f.base_form_type + "A")

    # Structural changes need the text: drop one line, add one with the same
    # amount (so totals hold), and mark the header as amendment 1 of FEC-<id>.
    rows = f.to_fec_string().split("\r\n")
    header = rows[0].split(FS)
    header[5], header[6] = f"FEC-{filing_id}", "1"
    rows[0] = FS.join(header)
    del rows[removed.line_no - 1]  # rows[0] is physical line 1 (HDR)
    new_cells = template.to_dict()
    new_cells.update({"transaction_id": "NEW0001", "contributor_last_name": "Newdonor",
                      "contributor_first_name": "Nora", "contribution_amount": removed["contribution_amount"]})
    layout = hardmoney.layout("SchA", f.version)
    cells = [""] * (max(c for _, c in layout) + 1)
    for name, col in layout:
        cells[col] = new_cells.get(name, "")
    rows.insert(len(rows) - 1, FS.join(cells))  # before the trailing empty element
    return hardmoney.parse("\r\n".join(rows))


def main(argv: Sequence[str]) -> int:
    if len(argv) >= 2:
        original = hardmoney.parse_file(argv[0])
        amendment = hardmoney.parse_file(argv[1])
    else:
        path = Path(argv[0]) if argv else DEFAULT
        original = hardmoney.parse_file(path)
        amendment = synthesize_amendment(path)
        print(f"(amendment derived from {path.name} for demonstration)\n")
    compare(original, amendment)
    try:
        r = amendment.reconcile()
        print(f"\namendment reconciles: {r.balances}")
    except hardmoney.UnsupportedForm:
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
