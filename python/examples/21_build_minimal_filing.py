"""21. Build a small Form 3X from scratch and write it.

Shows: there is no ``Filing`` constructor (every class is frozen and comes
from the parser), so the way to build a filing is to build the text.
``hardmoney.layout(table, version)`` gives the column of every field, so
a record can be assembled from a ``{field: value}`` dict without knowing
any column numbers; the assembled text is parsed with ``hardmoney.parse``,
checked with ``validate()`` and ``reconcile()``, and written with
``to_fec()``.

The cover total 11(a)(i) is computed from the Schedule A lines with
``Decimal``, and the reconciler confirms it.

Run:
    python examples/21_build_minimal_filing.py [out.fec]

Writes to a temporary directory when no output path is given.
"""

from __future__ import annotations

import sys
import tempfile
from decimal import Decimal
from pathlib import Path
from typing import Mapping, Sequence

import hardmoney

FS = "\x1c"  # the FEC field separator (ASCII 28)
VERSION = hardmoney.BUNDLED_SPEC_VERSION
COMMITTEE = "C00123456"


def record(table: str, values: Mapping[str, str]) -> str:
    """One .fec record: ``values`` placed in the columns ``layout`` says."""
    layout = hardmoney.layout(table, VERSION)
    known = {name for name, _ in layout}
    unknown = set(values) - known
    if unknown:
        raise KeyError(f"{table} has no field(s) {sorted(unknown)}")
    cells = [""] * (max(col for _, col in layout) + 1)
    for name, col in layout:
        cells[col] = values.get(name, "")
    return FS.join(cells)


def main(argv: Sequence[str]) -> int:
    out = Path(argv[0]) if argv else Path(tempfile.mkdtemp()) / "minimal-f3x.fec"

    contributions = [
        {"transaction_id": "A1", "contributor_last_name": "Smith", "contributor_first_name": "Jane",
         "contributor_street_1": "1 Main St", "contributor_city": "Springfield", "contributor_state": "VA",
         "contributor_zip_code": "22150", "contribution_date": "20260315", "contribution_amount": "250.00",
         "contribution_aggregate": "250.00", "contributor_employer": "Acme", "contributor_occupation": "Engineer"},
        {"transaction_id": "A2", "contributor_last_name": "Jones", "contributor_first_name": "Sam",
         "contributor_street_1": "2 Oak Ave", "contributor_city": "Arlington", "contributor_state": "VA",
         "contributor_zip_code": "22201", "contribution_date": "20260320", "contribution_amount": "1000.00",
         "contribution_aggregate": "1000.00", "contributor_employer": "Self", "contributor_occupation": "Consultant"},
    ]
    itemized = sum((Decimal(c["contribution_amount"]) for c in contributions), Decimal("0"))

    header = record("HDR", {"record_type": "HDR", "ef_type": "FEC", "fec_version": VERSION,
                            "soft_name": "example 21", "soft_ver": "1.0"})
    cover = record("F3X", {
        "form_type": "F3XN", "filer_committee_id_number": COMMITTEE, "committee_name": "Example PAC",
        "street_1": "PO Box 1", "city": "Alexandria", "state": "VA", "zip_code": "22313",
        "report_code": "Q1", "coverage_from_date": "20260101", "coverage_through_date": "20260331",
        "treasurer_last_name": "Doe", "treasurer_first_name": "Pat", "date_signed": "20260415",
        # The summary page: the receipts flow from 11(a)(i) up through the
        # subtotals to cash on hand. Every one of these is a reconciler rule.
        "col_a_individuals_itemized": str(itemized),          # 11(a)(i)
        "col_a_individuals_unitemized": "0.00",               # 11(a)(ii)
        "col_a_individual_contribution_total": str(itemized),  # 11(a)(iii)
        "col_a_total_contributions": str(itemized),           # 11(d)
        "col_a_total_receipts_recap": str(itemized),          # 19
        "col_a_total_federal_receipts": str(itemized),        # 20 = 19 - 18(c)
        "col_a_total_contributions_recap": str(itemized),     # 33
        "col_a_net_contributions": str(itemized),             # 35 = 33 - 34
        "col_a_cash_on_hand_beginning_period": "0.00",        # 6(b)
        "col_a_total_receipts": str(itemized),                # 6(c) = 19
        "col_a_subtotal": str(itemized),                      # 6(d) = 6(b) + 6(c)
        "col_a_total_disbursements": "0.00",                  # 7
        "col_a_cash_on_hand_close_of_period": str(itemized),  # 8 = 6(d) - 7
    })
    body = [
        record("SchA", {"form_type": "SA11AI", "filer_committee_id_number": COMMITTEE,
                        "entity_type": "IND", "election_code": "P2026", **c})
        for c in contributions
    ]
    text = "\n".join([header, cover, *body]) + "\n"

    filing = hardmoney.parse(text)
    print(f"built {filing.form_type} v{filing.version} with {len(filing.lines)} body line(s)")
    print(f"cover 11(a)(i) = {filing.summary.amount('col_a_individuals_itemized')}")

    v = filing.validate()
    print(f"validate: {len(v.errors)} error(s), {len(v.warnings)} warning(s); acceptable: {v.is_acceptable}")
    for w in v.warnings[:5]:
        print(f"  {w}")

    r = filing.reconcile()
    print(f"reconcile: balances {r.balances}; "
          f"{', '.join(f'{c.column} {c.line} {c.delta:+}' for c in r.mismatches()) or 'no mismatches'}")

    data = filing.to_fec()
    out.write_bytes(data)
    print(f"wrote {len(data)} bytes to {out}")
    # Note: str.splitlines() treats the FS byte (0x1c) as a line break, so
    # split .fec text on the CRLF the writer emits instead.
    print("first SchA record as written:")
    print("  " + filing.to_fec_string().split("\r\n")[2].replace(FS, "|")[:110] + "...")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
