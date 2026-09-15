"""29. Fetch a filing from the FEC by id and inspect it.

Shows: ``hardmoney.fetch(filing_id)`` downloads
``https://docquery.fec.gov/dcdev/posted/<id>.fec`` and parses it
strictly. Network failures raise ``FecError``. Nothing is cached; for a
cache-first download loop use the CLI (examples 30 and 31).

This example needs network access. In the test suite it runs only when
``HARDMONEY_NETWORK_TESTS`` is set.

Run:
    python examples/29_fetch_filing.py [filing_id]

Defaults to 2011831 (a September 2026 monthly Form 3X, 32 KB).
"""

from __future__ import annotations

import sys
from typing import Sequence

import hardmoney

DEFAULT_ID = 2011831


def main(argv: Sequence[str]) -> int:
    filing_id = int(argv[0]) if argv else DEFAULT_ID
    try:
        filing = hardmoney.fetch(filing_id)
    except hardmoney.FecError as e:
        print(f"could not fetch filing {filing_id}: {e}")
        return 1

    cover = filing.summary
    print(f"filing {filing_id}: {filing.form_type} v{filing.version}, {len(filing.lines)} body line(s)")
    print(f"  committee: {cover.get('committee_name')} ({cover.get('filer_committee_id_number')})")
    if "coverage_from_date" in cover:
        print(f"  coverage:  {cover.date('coverage_from_date')} to {cover.date('coverage_through_date')}")
    if "col_a_total_receipts" in cover:
        print(f"  receipts:  {cover.amount('col_a_total_receipts')}; "
              f"disbursements: {cover.amount('col_a_total_disbursements')}")

    v = filing.validate()
    print(f"  validate:  {len(v.errors)} error(s), {len(v.warnings)} warning(s)")
    try:
        r = filing.reconcile()
        print(f"  reconcile: {'balances' if r.balances else f'{len(r.mismatches())} line(s) disagree'}")
    except hardmoney.UnsupportedForm:
        print(f"  reconcile: not applicable to {filing.base_form_type}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
