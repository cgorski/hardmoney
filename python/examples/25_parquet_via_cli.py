"""25. Parquet via the CLI, read back with pyarrow (and DuckDB if present).

Shows: ``subprocess.run(["hardmoney", "export", path, "--format", "parquet"])``
writes one ``<Table>.parquet`` per table with real types: amounts are
``decimal128(38, 2)`` and ``YYYYMMDD`` dates are ``date32``, so pyarrow
and DuckDB sum money exactly without ever going through a float. The
Python package itself has no Arrow dependency; the CLI does the typed
export and any Arrow reader consumes it.

Requires the ``hardmoney`` CLI (on PATH, at ``$HARDMONEY_BIN``, or built
in the repository with ``cargo build --all-features``) and pyarrow;
DuckDB is used if installed. Exits 0 with a message when anything is
missing.

Run:
    python examples/25_parquet_via_cli.py [path/to/filing.fec] [out_dir]

Defaults to tests/fixtures/F3XA_2011821.fec and a temporary directory.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Optional, Sequence

try:
    import pyarrow.compute as pc
    import pyarrow.parquet as pq
except ImportError:  # pragma: no cover - optional dependency
    pq = pc = None

try:
    import duckdb
except ImportError:  # pragma: no cover - optional dependency
    duckdb = None

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "F3XA_2011821.fec"


def hardmoney_cli() -> Optional[str]:
    """The CLI: $HARDMONEY_BIN, then PATH, then a build in this repository."""
    env = os.environ.get("HARDMONEY_BIN")
    if env:
        return env
    found = shutil.which("hardmoney")
    if found:
        return found
    # A checkout may have both a debug and a release build; take the newest.
    builds = [p for p in (REPO / "target" / prof / "hardmoney" for prof in ("debug", "release")) if p.exists()]
    if builds:
        return str(max(builds, key=lambda p: p.stat().st_mtime))
    return None


def main(argv: Sequence[str]) -> int:
    if pq is None:
        print("pyarrow is not installed; pip install pyarrow to run this example")
        return 0
    cli = hardmoney_cli()
    if cli is None:
        print("hardmoney CLI not found (set HARDMONEY_BIN or cargo build --all-features)")
        return 0
    path = Path(argv[0]) if argv else DEFAULT
    out = Path(argv[1]) if len(argv) > 1 else Path(tempfile.mkdtemp()) / "parquet"

    cmd = [cli, "export", str(path), "--format", "parquet", "--out", str(out), "--include-filing-id"]
    result = subprocess.run(cmd, check=True, capture_output=True, text=True)
    print(result.stdout.rstrip())

    schb = pq.read_table(out / "SchB.parquet")
    print(f"\nSchB.parquet: {schb.num_rows} rows; column types:")
    for name in ("filing_id", "line_no", "payee_organization_name", "expenditure_date", "expenditure_amount"):
        print(f"  {name:26} {schb.schema.field(name).type}")

    total = pc.sum(schb["expenditure_amount"]).as_py()
    print(f"\npyarrow sum of expenditure_amount: {total!r} ({type(total).__name__})")

    if duckdb is not None:
        rel = duckdb.sql(f"""
            SELECT payee_organization_name AS payee, expenditure_date AS date, expenditure_amount AS amount
            FROM '{out / "SchB.parquet"}' ORDER BY amount DESC LIMIT 3""")
        print("\nDuckDB, top 3 disbursements:")
        print(rel)
        [(dsum, dtype)] = duckdb.sql(f"""
            SELECT SUM(expenditure_amount), typeof(SUM(expenditure_amount))
            FROM '{out / "SchB.parquet"}'""").fetchall()
        print(f"DuckDB SUM: {dsum!r} as {dtype}")
    else:
        print("\n(duckdb not installed; pip install duckdb to query the files with SQL)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
