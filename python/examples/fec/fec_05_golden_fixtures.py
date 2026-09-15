"""FEC 05. Golden .fec fixtures from a transaction list, with the cover page derived.

Shows: a spec-8.5 Form 3X built from ``(form type, amount, memo)`` tuples
with ``hardmoney.layout()``, its cover page filled by fixed point
(``reconcile()`` says what each line should be given the lines set so
far, ``Line.set()`` writes it, and a few passes reach a cover that
balances on all 69 checks), written with ``to_fec()``, and re-parsed to
prove it validates clean. The default transactions are the ones
FECfile+'s ``test_calculate_summary_column_a`` asserts against, memo SA15
and SE entries included, so the ``.json`` beside the ``.fec`` is the
Column A that calculator should produce.

Run:
    python examples/fec/fec_05_golden_fixtures.py [out_dir]   (default: a temp directory)
"""

from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path
from typing import Sequence

import hardmoney

FS, COMMITTEE = "\x1c", "C00123456"
FECFILE_PLUS_TEST_SET = [  # fecfiler/web_services/summary/tests/utils.py, public domain
    ("SA11AI", "10000.23", False), ("SA11B", "444.44", False), ("SA11C", "555.55", False), ("SA12", "1212.12", False),
    ("SA13", "1313.13", False), ("SA14", "1414.14", False), ("SA15", "1234.56", False), ("SA15", "891.23", False),
    ("SA15", "10000.23", True), ("SA16", "16", False), ("SA17", "200.50", False), ("SA17", "-1", False),
    ("SA17", "800.50", False), ("SB21B", "150", False), ("SB22", "22", False), ("SB23", "14", False),
    ("SB26", "44", False), ("SB27", "31", False), ("SB28A", "101.50", False), ("SB28B", "201.50", False),
    ("SB28C", "301.50", False), ("SB29", "201.50", False), ("SB30B", "102.25", False),
    ("SE", "65", False), ("SE", "76", False), ("SE", "10", False), ("SE", "57", True),
]
COVER = {"form_type": "F3XN", "committee_name": "Example PAC", "street_1": "PO BOX 1", "city": "ALEXANDRIA", "state": "VA",
         "zip_code": "22313", "report_code": "Q1", "coverage_from_date": "20260101", "coverage_through_date": "20260331",
         "treasurer_last_name": "Doe", "treasurer_first_name": "Pat", "date_signed": "20260415"}
PAYEE = {"entity_type": "ORG", "payee_organization_name": "Acme Print", "payee_street_1": "2 Main St", "payee_city": "Springfield",
         "payee_state": "VA", "payee_zip_code": "22150", "expenditure_purpose_descrip": "Printing", "category_code": "001"}
TEMPLATES = {
    "SchA": {"entity_type": "IND", "contributor_last_name": "Smith", "contributor_first_name": "Jane", "contributor_street_1": "1 Main St",
             "contributor_city": "Springfield", "contributor_state": "VA", "contributor_zip_code": "22150", "contribution_date": "20260315"},
    "SchB": {**PAYEE, "expenditure_date": "20260316"},
    "SchE": {**PAYEE, "election_code": "G2026", "dissemination_date": "20260316", "support_oppose_code": "S", "candidate_office": "H",
             "candidate_id_number": "H0VA01001", "candidate_last_name": "Roe", "candidate_first_name": "Rae", "candidate_state": "VA",
             "candidate_district": "01", "calendar_y_t_d_per_election_office": "151.00", "completing_last_name": "Doe",
             "completing_first_name": "Pat", "date_signed": "20260415"},
}


def record(table: str, values: dict[str, str]) -> str:
    layout = hardmoney.layout(table, "8.5")
    cells = [""] * (max(col for _, col in layout) + 1)
    for name, col in layout:
        cells[col] = values.get(name, "")
    return FS.join(cells)


def build(transactions: list[tuple[str, str, bool]]) -> hardmoney.Filing:
    rows = [FS.join(["HDR", "FEC", "8.5", "hardmoney golden", hardmoney.__version__, "", "", ""]),
            record("F3X", {**COVER, "filer_committee_id_number": COMMITTEE})]
    for i, (token, amount, memo) in enumerate(transactions, 1):
        table = {"SA": "SchA", "SB": "SchB", "SE": "SchE"}[token[:2]]
        rows.append(record(table, {**TEMPLATES[table], "form_type": token, "filer_committee_id_number": COMMITTEE,
                                   "transaction_id": f"{token}.{i}", "memo_code": "X" if memo else "",
                                   "contribution_amount" if table == "SchA" else "expenditure_amount": amount}))
    filing = hardmoney.parse("\r\n".join(rows) + "\r\n")
    for _ in range(12):  # each pass settles one more layer of formulas: 11ai -> 11aiii -> 11d -> 19 -> ... -> 8
        pending = [c for c in filing.reconcile().checks if c.reported is None or not c.matches]
        if not pending:
            break
        for c in pending:
            filing.summary.set(c.field, str(c.expected))
    return filing


def main(argv: Sequence[str]) -> int:
    out = Path(argv[0]) if argv else Path(tempfile.mkdtemp()) / "golden"
    out.mkdir(parents=True, exist_ok=True)
    (out / "f3x_fecfile_plus_test_set.fec").write_bytes(build(FECFILE_PLUS_TEST_SET).to_fec())
    again = hardmoney.parse_file(out / "f3x_fecfile_plus_test_set.fec")
    r, v = again.reconcile(), again.validate()
    column_a = {c.line: str(c.expected) for c in r.checks if c.column == "A"}
    (out / "f3x_fecfile_plus_test_set.json").write_text(json.dumps(column_a, indent=2))
    print(f"wrote {out}/f3x_fecfile_plus_test_set.fec: {again.form_type} v{again.version}, {len(again.lines)} body line(s)")
    print(f"re-parsed: {len(v.errors)} error(s), {len(v.warnings)} warning(s); reconcile balances: {r.balances} ({len(r)} checks)")
    for line in ("11(a)(i)", "15", "17", "21(b)", "24", "6(c)", "7", "8"):
        print(f"  Column A {line:9} {column_a[line]:>10}")
    return 0 if v.is_acceptable and r.balances else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
