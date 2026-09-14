#!/usr/bin/env python3
"""Regenerates src/parser/format_data.rs from every CSV file in
data/fec-csv-sources/ (excluding the headers/ subdir, which
holds old paper-era listings not used by the electronic parser).

Run this after adding/removing/renaming a format table CSV.
"""
import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
DATA_DIR = ROOT / "data" / "fec-csv-sources"
OUT_FILE = ROOT / "src" / "parser" / "format_data.rs"

csvs = sorted(p.name for p in DATA_DIR.glob("*.csv"))

lines = [
    "//! Compile-time embedding of the fec-csv-sources format tables.",
    "//!",
    "//! Source and provenance: these tables originate from the community-",
    "//! maintained fech-sources project (https://github.com/dwillis/fech-sources,",
    "//! itself derived from the `sources/` directory shipped with the NYT's Fech",
    "//! Ruby gem, the same lineage nyt-pyfec's own bundled tables come from),",
    "//! current through FEC electronic filing spec 8.5.0.1. See",
    "//! ../../README.md for full sourcing notes, and",
    "//! ../../tests/real_filings.rs for validation against live 2026",
    "//! sample filings.",
    "//!",
    "//! Regenerate this file with `python3 ../../scripts/generate_format_data.py`",
    "//! after adding, removing, or renaming a CSV in data/fec-csv-sources/.",
    "",
    "pub static FORM_CSV_DATA: &[(&str, &str)] = &[",
]
for name in csvs:
    table = name[:-4]
    lines.append(f'    ("{table}", include_str!("../../data/fec-csv-sources/{name}")),')
lines.append("];")
lines.append("")

OUT_FILE.write_text("\n".join(lines))
print(f"Wrote {len(csvs)} tables to {OUT_FILE}")
