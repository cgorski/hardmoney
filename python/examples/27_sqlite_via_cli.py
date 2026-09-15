"""27. SQLite via the CLI, queried with the standard library and exact sums.

Shows: ``hardmoney export --format sqlite`` writes one table per record
type plus a ``filings`` row; every field is ``TEXT`` exactly as filed, so
amounts are strings like ``'3697.50'``. The standard ``sqlite3`` module
sums them exactly if you give it a ``Decimal`` aggregate
(``create_aggregate``) instead of ``SUM(CAST(... AS REAL))``, and
``register_adapter(Decimal, str)`` lets you pass Decimals as parameters.

Requires the ``hardmoney`` CLI (on PATH, at ``$HARDMONEY_BIN``, or built
in the repository); exits 0 with a message when it is missing.

Run:
    python examples/27_sqlite_via_cli.py [path/to/filing.fec] [out.sqlite]

Defaults to tests/fixtures/F3XA_2011821.fec and a temporary file.
"""

from __future__ import annotations

import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
from decimal import Decimal
from pathlib import Path
from typing import Optional, Sequence

REPO = Path(__file__).resolve().parents[2]
DEFAULT = REPO / "tests" / "fixtures" / "F3XA_2011821.fec"


def hardmoney_cli() -> Optional[str]:
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


class DecimalSum:
    """An exact SUM() for TEXT amount columns: ``SELECT decimal_sum(col)``."""

    def __init__(self) -> None:
        self.total = Decimal("0")

    def step(self, value: Optional[str]) -> None:
        if value:  # NULL and '' contribute nothing
            self.total += Decimal(value)

    def finalize(self) -> str:
        return str(self.total)  # back to text; the caller wraps it in Decimal


def connect(db: Path) -> sqlite3.Connection:
    sqlite3.register_adapter(Decimal, str)  # Decimal parameters go in as text
    conn = sqlite3.connect(db)
    conn.create_aggregate("decimal_sum", 1, DecimalSum)
    return conn


def main(argv: Sequence[str]) -> int:
    cli = hardmoney_cli()
    if cli is None:
        print("hardmoney CLI not found (set HARDMONEY_BIN or cargo build --all-features)")
        return 0
    path = Path(argv[0]) if argv else DEFAULT
    db = Path(argv[1]) if len(argv) > 1 else Path(tempfile.mkdtemp()) / f"{path.stem}.sqlite"

    cmd = [cli, "export", str(path), "--format", "sqlite", "--out", str(db), "--include-filing-id"]
    print(subprocess.run(cmd, check=True, capture_output=True, text=True).stdout.rstrip())

    conn = connect(db)
    tables = [r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")]
    print(f"\ntables: {', '.join(tables)}")
    print("filings:", conn.execute("SELECT filing_id, form_type, version, committee_id FROM filings").fetchone())

    print("\nlargest disbursements (amounts are TEXT as filed):")
    for row in conn.execute("""SELECT line_no, payee_organization_name, expenditure_date, expenditure_amount
                               FROM SchB ORDER BY CAST(expenditure_amount AS REAL) DESC LIMIT 3"""):
        print(f"  {row}")

    exact = Decimal(conn.execute("SELECT decimal_sum(expenditure_amount) FROM SchB").fetchone()[0])
    approx = conn.execute("SELECT SUM(CAST(expenditure_amount AS REAL)) FROM SchB").fetchone()[0]
    print(f"\nexact total via decimal_sum: {exact!r}")
    print(f"SUM(CAST(... AS REAL)):      {approx!r} (a float; fine for a glance, not for the books)")

    threshold = Decimal("1000.00")  # a Decimal parameter, adapted to text by register_adapter
    n = conn.execute("SELECT COUNT(*) FROM SchB WHERE CAST(expenditure_amount AS REAL) >= CAST(? AS REAL)",
                     (threshold,)).fetchone()[0]
    print(f"disbursements of at least {threshold}: {n}")
    conn.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
