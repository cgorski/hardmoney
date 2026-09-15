"""12. A pre-submission / CI check that fails on validation errors.

Shows: the exit-code contract a build step or pre-commit hook wants. Every
path on the command line is validated; the script prints one summary line
per file, the findings for any file that fails, and exits 1 if any file
has an error-severity finding (the FEC would reject it). ``--strict``
also fails on warnings. A file that does not parse fails too.

Wire it up like any other linter:

    python examples/12_validate_exit_code.py build/*.fec || exit 1

Run:
    python examples/12_validate_exit_code.py [--strict] file.fec [file.fec ...]

Defaults to one good and one bad fixture when no paths are given, which
exits 1.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Sequence

import hardmoney

REPO = Path(__file__).resolve().parents[2]
DEFAULTS = [
    REPO / "tests" / "fixtures" / "F3XA_2011827.fec",
    REPO / "tests" / "fixtures" / "invalid" / "duplicate_tran_id.fec",
]


def check(path: Path, strict: bool) -> bool:
    """Print a verdict for one file; return True when it passes."""
    try:
        filing = hardmoney.parse_file(path, lenient=True)
    except (hardmoney.FecError, OSError) as e:
        print(f"FAIL {path}: cannot parse: {e}")
        return False
    v = filing.validate()
    failing = v.errors + (v.warnings if strict else [])
    status = "FAIL" if failing else "ok  "
    print(f"{status} {path}: {filing.form_type} v{filing.version}, "
          f"{len(v.errors)} error(s), {len(v.warnings)} warning(s)")
    for f in failing:
        print(f"       {f}")
    return not failing


def main(argv: Sequence[str]) -> int:
    args = list(argv)
    strict = "--strict" in args
    if strict:
        args.remove("--strict")
    paths = [Path(a) for a in args] or DEFAULTS

    results = [check(p, strict) for p in paths]
    failed = results.count(False)
    print(f"\n{len(results) - failed} of {len(results)} file(s) pass" + (" (strict)" if strict else ""))
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
