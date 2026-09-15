"""26. Schedule A as a polars DataFrame with a ``pl.Decimal`` column.

Shows: polars has a real decimal type. A column built from Python
``Decimal`` values is ``Decimal(precision=38, scale=2)`` and ``sum``,
``group_by(...).agg(pl.col(...).sum())`` stay exact. Dates go in as
``pl.Date``. If you exported Parquet with the CLI (example 25),
``pl.read_parquet`` gives the same types with no conversion at all.

Requires polars (``pip install polars``); exits 0 with a message when it
is not installed.

Run:
    python examples/26_polars_dataframe.py [path/to/filing.fec]

Defaults to tests/fixtures/F3A_2011812.fec when no path is given.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Sequence

import hardmoney

try:
    import polars as pl
except ImportError:  # pragma: no cover - optional dependency
    pl = None

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "F3A_2011812.fec"


def schedule_a_frame(filing: hardmoney.Filing) -> "pl.DataFrame":
    lines = filing.lines_for("SchA")
    return pl.DataFrame(
        {
            "line_no": [l.line_no for l in lines],
            "form_type": [l.form_type for l in lines],
            "last_name": [l["contributor_last_name"] for l in lines],
            "employer": [l["contributor_employer"] for l in lines],
            "state": [l["contributor_state"] for l in lines],
            "date": [l.date("contribution_date") for l in lines],
            "amount": [l.amount("contribution_amount") for l in lines],
            "is_memo": [l.is_memo for l in lines],
        },
        schema_overrides={"amount": pl.Decimal(scale=2), "date": pl.Date},
    )


def main(argv: Sequence[str]) -> int:
    if pl is None:
        print("polars is not installed; pip install polars to run this example")
        return 0
    path = Path(argv[0]) if argv else DEFAULT
    filing = hardmoney.parse_file(path)
    df = schedule_a_frame(filing)

    print(f"{path.name}: {df.height} rows; schema:")
    for name, dtype in df.schema.items():
        print(f"  {name:10} {dtype}")

    receipts = df.filter(~pl.col("is_memo") & (pl.col("form_type") == "SA11AI"))
    total = receipts["amount"].sum()
    cover = filing.summary.amount("col_a_individuals_itemized")
    print(f"\nsum of non-memo SA11AI amounts: {total} ({type(total).__name__}); cover 11(a)(i): {cover}; "
          f"equal: {total == cover}")

    print("\ntop employers by exact total:")
    print(
        receipts.group_by("employer")
        .agg(pl.col("amount").sum().alias("total"), pl.len().alias("n"))
        .sort("total", descending=True)
        .head(5)
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
