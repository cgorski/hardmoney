"""24. Schedule A as a pandas DataFrame, with money kept exact.

Shows: building a ``DataFrame`` from ``Line.to_dict()`` rows plus typed
``amount``/``date`` columns, and keeping the amount column ``dtype=object``
holding ``decimal.Decimal`` values. pandas will happily infer ``float64``
for a column of Decimals if you let it; then 0.1 + 0.2 != 0.3 and a
groupby sum can drift a cent from the cover page. With Decimals in an
object column, ``sum`` and ``groupby(...).sum()`` return exact Decimals.

Requires pandas (``pip install pandas``); the example exits 0 with a
message when it is not installed.

Run:
    python examples/24_pandas_dataframe.py [path/to/filing.fec]

Defaults to tests/fixtures/F3XN_2011831.fec when no path is given.
"""

from __future__ import annotations

import sys
from decimal import Decimal
from pathlib import Path
from typing import Sequence

import hardmoney

try:
    import pandas as pd
except ImportError:  # pragma: no cover - optional dependency
    pd = None

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "F3XN_2011831.fec"


def schedule_a_frame(filing: hardmoney.Filing) -> "pd.DataFrame":
    rows = []
    for line in filing.lines_for("SchA"):
        row = line.to_dict()  # every field as filed, strings
        row["line_no"] = line.line_no
        row["is_memo"] = line.is_memo
        row["amount"] = line.amount("contribution_amount")  # Decimal or None
        row["date"] = line.date("contribution_date")  # datetime.date or None
        rows.append(row)
    df = pd.DataFrame(rows)
    # Pin the money column to object so pandas never converts it to float64.
    df["amount"] = df["amount"].astype(object)
    return df


def main(argv: Sequence[str]) -> int:
    if pd is None:
        print("pandas is not installed; pip install pandas to run this example")
        return 0
    path = Path(argv[0]) if argv else DEFAULT
    filing = hardmoney.parse_file(path)
    df = schedule_a_frame(filing)

    print(f"{path.name}: DataFrame with {len(df)} rows x {len(df.columns)} columns")
    print(f"amount dtype: {df['amount'].dtype}; first value: {df['amount'].iloc[0]!r}\n")

    cols = ["line_no", "contributor_last_name", "contributor_employer", "contributor_state", "date", "amount"]
    print(df[cols].head(5).to_string(index=False))

    receipts = df[~df["is_memo"]]
    total = receipts["amount"].sum()
    print(f"\nsum of non-memo amounts: {total!r}")
    cover = filing.summary.amount("col_a_individuals_itemized")
    print(f"cover page 11(a)(i):     {cover!r}; equal: {total == cover}")

    print("\nby state (exact Decimal sums):")
    by_state = receipts.groupby("contributor_state")["amount"].agg(["sum", "count"]).sort_values("sum", ascending=False)
    print(by_state.head(8).to_string())

    # What float64 would have done. Same data, one astype away from wrong:
    # the float prints as 13736.02 but the number it holds is not 13736.02.
    as_float = float(receipts["amount"].astype("float64").sum())
    print(f"\nthe same column as float64 sums to {as_float!r}, which is really {Decimal(as_float)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
