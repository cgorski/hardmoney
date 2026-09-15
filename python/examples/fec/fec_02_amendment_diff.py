"""FEC 02. What changed between a report and its amendment.

Shows: two real filings of the same report (Abundant Future, Q2 2026:
original 1996410, amendment 2011821) compared the way an analyst reads an
amendment: cover-page lines that moved, with the delta; other cover fields
that changed; transactions added, removed, or edited, matched on
``transaction_id`` (the FEC requires it to be stable across amendments);
whether each version balances. Given one filing id, the amendment's header
names the original (``amends_filing``) and both are fetched.

Run:
    python examples/fec/fec_02_amendment_diff.py [original.fec amendment.fec | amendment_id]
    (default tests/fixtures/F3XN_1996410.fec and tests/fixtures/F3XA_2011821.fec)
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Sequence

import hardmoney

FIXTURES = Path(__file__).resolve().parents[3] / "tests" / "fixtures"


def who(line: hardmoney.Line) -> str:
    name = next((line[f] for f in line.keys() if f.endswith(("organization_name", "_last_name")) and line[f]), "")
    amt = next((line[f] for f in ("contribution_amount", "expenditure_amount", "balance_at_close_this_period") if f in line), "")
    return f"{line.form_type:7} {name[:32]:32} {amt}"


def balances(filing: hardmoney.Filing) -> str:
    try:
        r = filing.reconcile()
    except hardmoney.UnsupportedForm:
        return "n/a"
    return "balances" if r.balances else "; ".join(f"{c.line} off by {c.delta:+}" for c in r.mismatches())


def compare(orig: hardmoney.Filing, amend: hardmoney.Filing) -> None:
    a, b = orig.summary, amend.summary
    print(f"original:  {orig.form_type} {a.get('report_code')} {a.get('coverage_from_date')}..{a.get('coverage_through_date')}, "
          f"{len(orig.lines)} line(s), signed {a.get('date_signed')}, {balances(orig)}")
    print(f"amendment: {amend.form_type}, amendment {amend.header['report_number']} of filing {amend.amends_filing}, "
          f"{len(amend.lines)} line(s), signed {b.get('date_signed')}, {balances(amend)}")
    changed = [k for k in a.keys() if k in b and a[k] != b[k]]
    print("\ncover-page lines that changed:")
    for k in changed:
        if k.startswith("col_") and a.amount(k) is not None and b.amount(k) is not None:
            print(f"  {k:42} {a.amount(k)!s:>14} -> {b.amount(k)!s:>14}  ({b.amount(k) - a.amount(k):+})")
    print("other cover fields that changed: " + (", ".join(k for k in changed if not k.startswith("col_")) or "none"))

    before = {l["transaction_id"]: l for l in orig.iter_lines() if l.get("transaction_id")}
    after = {l["transaction_id"]: l for l in amend.iter_lines() if l.get("transaction_id")}
    edited = sorted(t for t in before.keys() & after.keys() if before[t].to_dict() != after[t].to_dict())
    print(f"\ntransactions: {len(before)} -> {len(after)}; {len(after.keys() - before.keys())} added, "
          f"{len(before.keys() - after.keys())} removed, {len(edited)} edited")
    for t in sorted(after.keys() - before.keys()):
        print(f"  + {t:18} {who(after[t])}")
    for t in sorted(before.keys() - after.keys()):
        print(f"  - {t:18} {who(before[t])}")
    for t in edited:
        x, y = before[t].to_dict(), after[t].to_dict()
        print(f"  ~ {t:18} " + "; ".join(f"{k}: {x[k]!r} -> {y[k]!r}" for k in x if x[k] != y.get(k)))


def main(argv: Sequence[str]) -> int:
    if len(argv) == 1 and argv[0].isdigit():
        amend = hardmoney.fetch(int(argv[0]))
        if amend.amends_filing is None:
            print(f"filing {argv[0]} is not an amendment (header report id {amend.header['report_id']!r})")
            return 1
        orig = hardmoney.fetch(amend.amends_filing)
    else:
        paths = argv if len(argv) == 2 else [FIXTURES / "F3XN_1996410.fec", FIXTURES / "F3XA_2011821.fec"]
        orig, amend = (hardmoney.parse_file(p, lenient=True) for p in paths)
    compare(orig, amend)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
