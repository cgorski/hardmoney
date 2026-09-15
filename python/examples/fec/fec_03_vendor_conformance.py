"""FEC 03. Vendor conformance: a batch of .fec files, grouped by the software that wrote them.

Shows: the HDR record names the filing software (``soft_name``,
``soft_ver``) and ``validate()`` gives the FEC's acceptance verdict, so
one vendor's sample output becomes one table: files, accepted, findings
by rule, every error. With ``--oracle`` each rejected file also goes to
the FEC's own WebCheck through the CLI (``hardmoney validate --oracle
webcheck --json``) and the line gains its matched / only-ours /
only-theirs counts. Files leave your machine only with that flag.

Run:
    python examples/fec/fec_03_vendor_conformance.py [--oracle] [file_or_dir ...]
    (default tests/fixtures and tests/fixtures/invalid)
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Optional, Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[3]
FIXTURES = REPO / "tests" / "fixtures"


def cli() -> Optional[str]:
    builds = [p for p in (REPO / "target" / prof / "hardmoney" for prof in ("debug", "release")) if p.exists()]
    return os.environ.get("HARDMONEY_BIN") or shutil.which("hardmoney") or (
        str(max(builds, key=lambda p: p.stat().st_mtime)) if builds else None)


def webcheck(bin_: str, path: Path) -> str:
    out = subprocess.run([bin_, "validate", str(path), "--oracle", "webcheck", "--json"], capture_output=True, text=True)
    try:
        d = json.loads(out.stdout)["oracle_diff"]
    except (ValueError, KeyError):
        return f"webcheck unavailable: {out.stderr.strip()[:60]}"
    return f"webcheck {d['matched']} matched, {len(d['only_ours'])} only ours, {len(d['only_theirs'])} only theirs"


def main(argv: Sequence[str]) -> int:
    oracle = "--oracle" in argv
    targets = [Path(a) for a in argv if a != "--oracle"] or [FIXTURES, FIXTURES / "invalid"]
    files = sorted(p for t in targets for p in (t.glob("*.fec") if t.is_dir() else [t]))
    bin_ = cli() if oracle else None
    if oracle and bin_ is None:
        print("--oracle needs the hardmoney CLI (HARDMONEY_BIN, PATH, or cargo build --all-features)")
        return 1

    by_software: dict[str, list[tuple[Path, hardmoney.Validation]]] = defaultdict(list)
    for path in files:
        filing = hardmoney.parse_file(path, lenient=True)
        by_software[f"{filing.header['soft_name']} {filing.header['soft_ver']}".strip()].append((path, filing.validate()))
    print(f"{len(files)} file(s) from {len(by_software)} software product(s)\n")
    print(f"{'software':44} {'files':>5} {'accepted':>8} {'errors':>6} {'warnings':>8}")
    for soft, results in sorted(by_software.items(), key=lambda kv: -len(kv[1])):
        print(f"{soft[:44]:44} {len(results):5} {sum(v.is_acceptable for _, v in results):8} "
              f"{sum(len(v.errors) for _, v in results):6} {sum(len(v.warnings) for _, v in results):8}")

    rules: Counter[tuple[str, str]] = Counter((f.severity, f.rule) for results in by_software.values()
                                              for _, v in results for f in v.findings)
    print("\nfindings by rule:")
    for (severity, rule), n in sorted(rules.items(), key=lambda kv: (kv[0][0] != "error", -kv[1])):
        print(f"  {severity:7} {rule:38} {n:4}")

    print("\nfiles the FEC would reject:")
    for soft, results in sorted(by_software.items()):
        for path, v in results:
            if v.errors:
                print(f"  {path.name:34} {soft[:28]:28} {len(v.errors)} error(s)" + (f"; {webcheck(bin_, path)}" if bin_ else ""))
                for f in v.errors[:3]:
                    print(f"      line {f.line_no} {f.form_type} {f.field}: {f.message}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
