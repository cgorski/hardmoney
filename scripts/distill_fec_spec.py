#!/usr/bin/env python3
"""Distils the FEC's own electronic-filing specification into one
machine-readable JSON file, `data/fec-spec/spec-<version>.json`, which
`build.rs` compiles into the crate's `FieldSpec` tables.

Inputs (both US-government works, public domain):

  1. `data/fec-spec/FEC_EFO_Format_Specifications_v8.5.xlsx` -- the FEC's
     "Electronic Filing Specification Requirements, Part II" workbook
     (<https://docquery.fec.gov/formatspecs/FEC_EFO_Format_Specifications.xlsx>).
     One sheet per form/schedule; each row is one column of the record:
     COL SEQ, FIELD DESCRIPTION, TYPE (`A/N-200`, `AMT-12`, `NUM-8`, ...),
     REQUIRED (`X (error)`, `X (warning)`, conditional prose), SAMPLE DATA,
     VALUE REFERENCE, RULE REFERENCE, FIELD-FORM ASSOCIATION.
  2. Optionally, the `schema/*.json` directory of
     <https://github.com/fecgov/fecfile-validate> (pass with `--schemas`).
     Those JSON Schemas embed the same spreadsheet rows as `fec_spec` and
     add machine-readable `enum` and `pattern` constraints that the
     spreadsheet expresses only in prose. They are joined onto the
     spreadsheet rows by (sheet, COL SEQ).

The output is deliberately a *distillation*, not a dump: only the fields
hardmoney's validator and spec exporter use are kept, so a diff between two
spec versions is readable.

Usage:
    scripts/distill_fec_spec.py [--schemas PATH] [--out PATH] [--diff-report PATH]
Requires `openpyxl` (`python3 -m venv .venv && .venv/bin/pip install openpyxl`).

`--diff-report PATH` writes a Markdown summary of how the freshly distilled
spec differs from the checked-in `data/fec-spec/spec-8.5.json` (tables and
fields added or removed, and per-field attribute changes such as rule text,
type, length, required level, code lists, patterns). The weekly drift job
uploads it as an artifact; it is the report fecfile-validate#302 asks for.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys

try:
    import openpyxl
except ImportError:  # pragma: no cover
    sys.exit("openpyxl is required: python3 -m pip install openpyxl")

ROOT = pathlib.Path(__file__).resolve().parent.parent
DEFAULT_XLSX = ROOT / "data" / "fec-spec" / "FEC_EFO_Format_Specifications_v8.5.xlsx"
DEFAULT_OUT = ROOT / "data" / "fec-spec" / "spec-8.5.json"

# Workbook sheet name -> hardmoney table name (matches data/fec-csv-sources/<name>.csv).
SHEET_TO_TABLE = {
    "HDR": "HDR",
    "F1": "F1", "F1S": "F1S", "F1M": "F1M", "F2": "F2", "F2S": "F2S", "F24": "F24",
    "F3": "F3", "F3S": "F3S", "F3Z1": "F3Z1", "F3Z2": "F3Z2", "F3P": "F3P", "F3PS": "F3PS",
    "F3P31": "F3P31", "F3PZ1": "F3PZ1", "F3PZ2": "F3PZ2", "F3X": "F3X", "F3L": "F3L",
    "F4": "F4", "F56": "F56", "F5": "F5", "F57": "F57", "F6": "F6", "F65": "F65",
    "F7": "F7", "F76": "F76", "F9": "F9", "F91": "F91", "F92": "F92", "F93": "F93",
    "F94": "F94", "F13": "F13", "F132": "F132", "F133": "F133", "F99": "F99",
    "Sch A": "SchA", "Sch B": "SchB", "Sch C": "SchC", "Sch C1": "SchC1", "Sch C2": "SchC2",
    "Sch D": "SchD", "Sch E": "SchE", "Sch F": "SchF", "Sch H1": "H1", "Sch H2": "H2",
    "Sch H3": "H3", "Sch H4": "H4", "Sch H5": "H5", "Sch H6": "H6", "Sch L": "SchL",
    "Text": "TEXT",
}

# fecfile-validate schema file -> hardmoney table, for the wire-level schemas
# (the ones whose properties carry `fec_spec.COL_SEQ`).
SCHEMA_TO_TABLE = {
    "HDR.json": "HDR", "F1M.json": "F1M", "F24.json": "F24", "F3.json": "F3",
    "F3X.json": "F3X", "F99.json": "F99", "SchA.json": "SchA", "SchB.json": "SchB",
    "SchC.json": "SchC", "SchC1.json": "SchC1", "SchC2.json": "SchC2", "SchD.json": "SchD",
    "SchE.json": "SchE", "SchF.json": "SchF", "Text.json": "TEXT",
}

TYPE_RE = re.compile(r"^\s*(A/N|A|NUM|N|AMT)\s*-\s*(\d+)\s*$", re.I)
# The FEC schemas' generic "any printable ASCII up to N" pattern carries no
# information beyond the spreadsheet TYPE, so it is dropped.
GENERIC_PATTERN_RE = re.compile(r"^\^\[ -~\]\{0,\d+\}\$$")


def clean(cell) -> str | None:
    if cell is None:
        return None
    s = str(cell).replace("\r", "").strip()
    return s or None


def parse_type(raw: str | None) -> tuple[str, int | None]:
    """`A/N-200` -> ("alphanumeric", 200); `AMT-12` -> ("amount", 12)."""
    if not raw:
        return ("unknown", None)
    m = TYPE_RE.match(raw)
    if not m:
        return ("unknown", None)
    code, n = m.group(1).upper(), int(m.group(2))
    kind = {"A/N": "alphanumeric", "A": "alpha", "NUM": "numeric", "N": "numeric", "AMT": "amount"}[code]
    return (kind, n)


def parse_required(raw: str | None) -> dict:
    """The REQUIRED column: blank, `X (error)`, `X (warning)`, bare `X`, or
    conditional prose. Returns {"level": "none|error|warning|conditional",
    "condition": str|None}."""
    if not raw:
        return {"level": "none"}
    s = " ".join(raw.split())
    low = s.lower()
    if low in ("x (error)", "x(error)"):
        return {"level": "error"}
    if low in ("x (warning)", "x(warning)"):
        return {"level": "warning"}
    if low == "x":
        # A bare X in the workbook marks check-box fields whose only rule is
        # "X = Yes"; the FEC does not fail a filing over them.
        return {"level": "none"}
    return {"level": "conditional", "condition": s}


def forms_from_association(raw: str | None) -> list[str]:
    if not raw:
        return []
    return [p.strip() for p in raw.split("|") if p.strip()]


def read_sheet(ws) -> list[dict]:
    rows = list(ws.iter_rows(values_only=True))
    header_idx = next(
        (
            i
            for i, r in enumerate(rows)
            if r and any(isinstance(c, str) and c.replace("\n", " ").strip().upper().startswith("COL") for c in r)
        ),
        None,
    )
    if header_idx is None:
        return []
    header = [clean(c).upper().replace("\n", " ") if clean(c) else "" for c in rows[header_idx]]
    header = [" ".join(h.split()) for h in header]

    def col(name_prefix: str) -> int | None:
        for i, h in enumerate(header):
            if h.startswith(name_prefix):
                return i
        return None

    c_seq, c_desc, c_type, c_req = col("COL"), col("FIELD DESC"), col("TYPE"), col("REQUIRED")
    c_sample, c_valref, c_rule, c_forms = col("SAMPLE"), col("VALUE REF"), col("RULE REF"), col("FIELD-FORM")

    out = []
    for r in rows[header_idx + 1 :]:
        if not r or c_seq is None or c_seq >= len(r):
            continue
        seq = r[c_seq]
        if isinstance(seq, float) and seq.is_integer():
            seq = int(seq)
        if not isinstance(seq, int):
            continue

        def cell(idx):
            return clean(r[idx]) if idx is not None and idx < len(r) else None

        kind, max_len = parse_type(cell(c_type))
        sample = cell(c_sample)
        out.append(
            {
                "column": seq,
                "description": " ".join((cell(c_desc) or "").split()),
                "kind": kind,
                "max_len": max_len,
                "required": parse_required(cell(c_req)),
                "sample": sample,
                "value_reference": cell(c_valref),
                "rule": cell(c_rule),
                "forms": forms_from_association(cell(c_forms)),
                "allowed_values": [],
                "pattern": None,
            }
        )
    return out


def merge_schemas(tables: dict[str, list[dict]], schema_dir: pathlib.Path) -> int:
    """Joins `enum`/`pattern` from the fecfile-validate wire-level schemas
    onto spreadsheet rows by (table, COL_SEQ). Returns the number of rows
    enriched."""
    enriched = 0
    for fname, table in SCHEMA_TO_TABLE.items():
        path = schema_dir / fname
        if not path.exists() or table not in tables:
            continue
        schema = json.loads(path.read_text())
        by_col = {row["column"]: row for row in tables[table]}
        for prop in schema.get("properties", {}).values():
            if not isinstance(prop, dict):
                continue
            seq = (prop.get("fec_spec") or {}).get("COL_SEQ")
            if not isinstance(seq, int) or seq not in by_col:
                continue
            row = by_col[seq]
            touched = False
            enum = prop.get("enum")
            if isinstance(enum, list):
                vals = sorted({str(v) for v in enum if v is not None and str(v).strip()})
                if vals and vals != row["allowed_values"]:
                    row["allowed_values"] = sorted(set(row["allowed_values"]) | set(vals))
                    touched = True
            pattern = prop.get("pattern")
            if isinstance(pattern, str) and pattern and not GENERIC_PATTERN_RE.match(pattern):
                # FECfile+ stores dates as ISO `YYYY-MM-DD` internally; that
                # pattern does not describe the wire format, so skip it.
                if not (row["kind"] == "numeric" and "-[0-9]{2}-" in pattern):
                    row["pattern"] = pattern
                    touched = True
            enriched += touched
    return enriched


# Per-field attributes the diff report compares, in the order it lists them.
FIELD_ATTRS = (
    "description",
    "kind",
    "max_len",
    "required",
    "sample",
    "value_reference",
    "rule",
    "forms",
    "allowed_values",
    "pattern",
)


def _md_cell(value) -> str:
    """One attribute value as a Markdown table cell: JSON for lists and
    dicts, blank for None, whitespace collapsed, pipes escaped."""
    if value is None:
        return ""
    if isinstance(value, (list, dict)):
        value = json.dumps(value, ensure_ascii=False)
    text = " ".join(str(value).split())
    return "`" + text.replace("|", "\\|").replace("`", "'") + "`" if text else ""


def diff_specs(old: dict, new: dict) -> dict:
    """Compares two distilled spec documents. Fields are keyed by (table,
    column). Returns {"tables_added", "tables_removed", "fields_added",
    "fields_removed", "fields_changed"}; the last maps table -> list of
    (column, description, [(attr, old, new), ...])."""
    old_tables = old.get("tables", {})
    new_tables = new.get("tables", {})
    result = {
        "tables_added": sorted(set(new_tables) - set(old_tables)),
        "tables_removed": sorted(set(old_tables) - set(new_tables)),
        "fields_added": {},
        "fields_removed": {},
        "fields_changed": {},
    }
    for table in sorted(set(old_tables) & set(new_tables)):
        old_by_col = {r["column"]: r for r in old_tables[table]}
        new_by_col = {r["column"]: r for r in new_tables[table]}
        added = [new_by_col[c] for c in sorted(set(new_by_col) - set(old_by_col))]
        removed = [old_by_col[c] for c in sorted(set(old_by_col) - set(new_by_col))]
        if added:
            result["fields_added"][table] = added
        if removed:
            result["fields_removed"][table] = removed
        changed = []
        for col in sorted(set(old_by_col) & set(new_by_col)):
            o, n = old_by_col[col], new_by_col[col]
            attrs = [(a, o.get(a), n.get(a)) for a in FIELD_ATTRS if o.get(a) != n.get(a)]
            if attrs:
                changed.append((col, n.get("description") or o.get("description") or "", attrs))
        if changed:
            result["fields_changed"][table] = changed
    return result


def diff_is_empty(d: dict) -> bool:
    return not (
        d["tables_added"] or d["tables_removed"] or d["fields_added"] or d["fields_removed"] or d["fields_changed"]
    )


def render_diff_report(old: dict, new: dict, baseline_path: pathlib.Path, d: dict) -> str:
    """The Markdown drift report. Always starts with a one-line `Summary:`
    that CI can lift into a step summary."""
    n_fields_added = sum(len(v) for v in d["fields_added"].values())
    n_fields_removed = sum(len(v) for v in d["fields_removed"].values())
    n_fields_changed = sum(len(v) for v in d["fields_changed"].values())
    n_old = sum(len(v) for v in old.get("tables", {}).values())
    n_new = sum(len(v) for v in new.get("tables", {}).values())
    lines = [
        "# FEC spec drift report",
        "",
        f"Baseline: `{baseline_path.name}` (spec {old.get('spec_version')}, "
        f"{len(old.get('tables', {}))} tables, {n_old} fields, "
        f"schemas merged: {old.get('schemas_merged')}).",
        f"Regenerated: spec {new.get('spec_version')} from `{new.get('source_xlsx')}`, "
        f"{len(new.get('tables', {}))} tables, {n_new} fields, "
        f"schemas merged: {new.get('schemas_merged')}.",
        "",
        f"Summary: {len(d['tables_added'])} tables added, {len(d['tables_removed'])} removed; "
        f"{n_fields_added} fields added, {n_fields_removed} removed, {n_fields_changed} changed.",
        "",
    ]
    if diff_is_empty(d):
        lines.append("No differences: the checked-in spec matches the FEC's current sources.")
        lines.append("")
        return "\n".join(lines)

    if d["tables_added"] or d["tables_removed"]:
        lines.append("## Tables")
        lines.append("")
        for t in d["tables_added"]:
            lines.append(f"- added: `{t}` ({len(new['tables'][t])} fields)")
        for t in d["tables_removed"]:
            lines.append(f"- removed: `{t}` ({len(old['tables'][t])} fields)")
        lines.append("")

    tables = sorted(set(d["fields_added"]) | set(d["fields_removed"]) | set(d["fields_changed"]))
    for table in tables:
        lines.append(f"## {table}")
        lines.append("")
        for row in d["fields_added"].get(table, []):
            lines.append(
                f"- added column {row['column']}: {row['description']} "
                f"({row['kind']}-{row['max_len']}, required {row['required'].get('level')})"
            )
        for row in d["fields_removed"].get(table, []):
            lines.append(f"- removed column {row['column']}: {row['description']}")
        if d["fields_added"].get(table) or d["fields_removed"].get(table):
            lines.append("")
        changed = d["fields_changed"].get(table, [])
        if changed:
            lines.append("| Column | Field | Attribute | Checked in | Regenerated |")
            lines.append("|---|---|---|---|---|")
            for col, desc, attrs in changed:
                for attr, o, n in attrs:
                    lines.append(f"| {col} | {desc} | {attr} | {_md_cell(o)} | {_md_cell(n)} |")
            lines.append("")
    return "\n".join(lines)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--xlsx", type=pathlib.Path, default=DEFAULT_XLSX)
    ap.add_argument("--schemas", type=pathlib.Path, default=None, help="fecfile-validate/schema directory")
    ap.add_argument("--out", type=pathlib.Path, default=DEFAULT_OUT)
    ap.add_argument(
        "--diff-report",
        type=pathlib.Path,
        default=None,
        help=f"write a Markdown summary of how the result differs from the checked-in {DEFAULT_OUT.name}",
    )
    args = ap.parse_args()

    # Read the baseline before anything is written, in case --out is the
    # checked-in path itself.
    baseline = None
    if args.diff_report is not None:
        if DEFAULT_OUT.exists():
            baseline = json.loads(DEFAULT_OUT.read_text())
        else:
            print(f"warning: no baseline at {DEFAULT_OUT}; the diff report will show everything as added", file=sys.stderr)
            baseline = {"tables": {}}

    wb = openpyxl.load_workbook(args.xlsx, read_only=True, data_only=True)
    version = None
    for r in wb[wb.sheetnames[0]].iter_rows(values_only=True):
        for c in r:
            if isinstance(c, str):
                m = re.search(r"Version\s+(\d+\.\d+)", c)
                if m:
                    version = m.group(1)
                    break
        if version:
            break
    if not version:
        sys.exit("could not find the spec version on the first sheet")

    tables: dict[str, list[dict]] = {}
    unmapped = []
    for sheet in wb.sheetnames:
        table = SHEET_TO_TABLE.get(sheet)
        if table is None:
            if sheet not in ("SUMMARY OF CHANGES",) and not sheet.startswith("Version"):
                unmapped.append(sheet)
            continue
        rows = read_sheet(wb[sheet])
        if not rows:
            print(f"warning: sheet {sheet!r} yielded no rows", file=sys.stderr)
            continue
        tables[table] = rows
    if unmapped:
        print(f"warning: unmapped sheets: {unmapped}", file=sys.stderr)

    enriched = 0
    if args.schemas:
        enriched = merge_schemas(tables, args.schemas)

    doc = {
        "$comment": "GENERATED by scripts/distill_fec_spec.py from the FEC's Electronic Filing "
        "Specification workbook (public domain). Do not edit by hand.",
        "spec_version": version,
        "source_xlsx": args.xlsx.name,
        "schemas_merged": bool(args.schemas),
        "tables": {t: tables[t] for t in sorted(tables)},
    }
    args.out.write_text(json.dumps(doc, indent=1, ensure_ascii=False) + "\n")
    n_fields = sum(len(v) for v in tables.values())
    print(f"wrote {args.out} ({len(tables)} tables, {n_fields} fields, {enriched} enriched from schemas)")

    if args.diff_report is not None:
        d = diff_specs(baseline, doc)
        report = render_diff_report(baseline, doc, DEFAULT_OUT, d)
        args.diff_report.write_text(report)
        summary = next((l for l in report.splitlines() if l.startswith("Summary:")), "")
        print(f"wrote {args.diff_report}: {summary}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
