#!/usr/bin/env python3
"""Regenerates src/parser/format_data.rs from every CSV file in
data/fec-csv-sources/ (excluding the headers/ subdir, which
holds old paper-era listings not used by the electronic parser).

Emits two things that must stay in lockstep:

  * `pub enum Table` -- one variant per format table, so the rest of the
    crate can refer to a table with a type instead of a `&'static str`.
  * `FORM_CSV_DATA` -- the embedded CSV for each variant, in the same
    order as `Table::ALL`.

Run this after adding/removing/renaming a format table CSV.
"""
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
DATA_DIR = ROOT / "data" / "fec-csv-sources"
OUT_FILE = ROOT / "src" / "parser" / "format_data.rs"

csvs = sorted(p.name for p in DATA_DIR.glob("*.csv"))
tables = [name[:-4] for name in csvs]


def variant(table: str) -> str:
    """`F3X` -> `F3X`, `SchA3L` -> `SchA3L`, `TEXT` -> `Text`, `HDR` -> `Hdr`."""
    if table.isupper() and table.isalpha():
        return table.capitalize()
    return re.sub(r"[^A-Za-z0-9]", "", table)


lines = [
    "//! Compile-time embedding of the fec-csv-sources format tables, plus the",
    "//! [`Table`] enum that names them.",
    "//!",
    "//! Source and provenance: these tables originate from the community-",
    "//! maintained fech-sources project (<https://github.com/dwillis/fech-sources>,",
    "//! itself derived from the `sources/` directory shipped with the NYT's Fech",
    "//! Ruby gem, the same lineage nyt-pyfec's own bundled tables come from),",
    "//! current through FEC electronic filing spec 8.5.0.1, with the corrections",
    "//! listed in `NOTICE`. `F2S.csv` is authored locally (fech-sources has no",
    "//! table for it). See `README.md` for full sourcing notes and",
    "//! `tests/real_filings.rs` for validation against real filings.",
    "//!",
    "//! GENERATED FILE -- do not edit by hand. Regenerate with",
    "//! `python3 scripts/generate_format_data.py` after adding, removing, or",
    "//! renaming a CSV in `data/fec-csv-sources/`.",
    "",
    "/// One FEC format table: a form, schedule, or record type whose column",
    "/// layout (per spec version) is described by a bundled CSV.",
    "///",
    "/// This is the type-level identity of a table. [`crate::parser::ParsedLine::table`]",
    "/// carries it so callers can match on a closed set instead of comparing",
    "/// strings, and typed views use it to refuse lines from the wrong table.",
    "///",
    "/// `#[non_exhaustive]` because the FEC adds record types over time.",
    "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]",
    '#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]',
    "#[non_exhaustive]",
    "pub enum Table {",
]
for t in tables:
    lines.append(f"    /// `{t}.csv`")
    lines.append(f"    {variant(t)},")
lines.append("}")
lines.append("")
lines.append("impl Table {")
lines.append("    /// Every table, in the same order as [`FORM_CSV_DATA`].")
lines.append("    pub const ALL: &[Table] = &[")
for t in tables:
    lines.append(f"        Table::{variant(t)},")
lines.append("    ];")
lines.append("")
lines.append("    /// Position of this variant in [`Table::ALL`] (its declaration order).")
lines.append("    pub(crate) const fn index(self) -> usize {")
lines.append("        self as usize")
lines.append("    }")
lines.append("")
lines.append("    /// The table's name as used in the CSV filename and in older")
lines.append("    /// string-based APIs, e.g. `\"SchA\"`, `\"F3X\"`.")
lines.append("    pub const fn as_str(self) -> &'static str {")
lines.append("        match self {")
for t in tables:
    lines.append(f'            Table::{variant(t)} => "{t}",')
lines.append("        }")
lines.append("    }")
lines.append("")
lines.append("    /// Inverse of [`Table::as_str`] (exact, case-sensitive).")
lines.append("    pub fn from_name(name: &str) -> Option<Table> {")
lines.append("        match name {")
for t in tables:
    lines.append(f'            "{t}" => Some(Table::{variant(t)}),')
lines.append("            _ => None,")
lines.append("        }")
lines.append("    }")
lines.append("")
lines.append("    /// The raw CSV format table for this variant.")
lines.append("    pub const fn csv(self) -> &'static str {")
lines.append("        match self {")
for t, name in zip(tables, csvs):
    lines.append(
        f'            Table::{variant(t)} => include_str!("../../data/fec-csv-sources/{name}"),'
    )
lines.append("        }")
lines.append("    }")
lines.append("}")
lines.append("")
lines.append("impl std::fmt::Display for Table {")
lines.append("    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {")
lines.append("        f.write_str(self.as_str())")
lines.append("    }")
lines.append("}")
lines.append("")
lines.append("impl std::str::FromStr for Table {")
lines.append("    type Err = UnknownTable;")
lines.append("")
lines.append("    fn from_str(s: &str) -> Result<Self, Self::Err> {")
lines.append("        Table::from_name(s).ok_or_else(|| UnknownTable(s.to_string()))")
lines.append("    }")
lines.append("}")
lines.append("")
lines.append("/// Error from parsing a `Table` from a string: the name is not a bundled format table.")
lines.append("#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]")
lines.append('#[error("unknown format table \'{0}\'")]')
lines.append("pub struct UnknownTable(pub String);")
lines.append("")
lines.append("/// `(table name, embedded CSV)` for every bundled format table, in the")
lines.append("/// same order as [`Table::ALL`]. Prefer [`Table::csv`] in new code.")
lines.append("pub static FORM_CSV_DATA: &[(&str, &str)] = &[")
for t, name in zip(tables, csvs):
    lines.append(f'    ("{t}", include_str!("../../data/fec-csv-sources/{name}")),')
lines.append("];")
lines.append("")
lines.append("/// Compile-time guard: `Table::ALL` and `FORM_CSV_DATA` were generated")
lines.append("/// from the same directory listing and must have the same length.")
lines.append("const _: () = assert!(Table::ALL.len() == FORM_CSV_DATA.len());")
lines.append("")

OUT_FILE.write_text("\n".join(lines))
# Keep the generated file rustfmt-stable so regenerating never dirties the tree.
import subprocess
subprocess.run(["rustfmt", "--edition", "2024", str(OUT_FILE)], check=True)
print(f"Wrote {len(csvs)} tables to {OUT_FILE}")
