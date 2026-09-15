#!/usr/bin/env python3
"""Independent oracle: compare hardmoney's parse of every corpus filing
against the `fecfile` Python library (MIT; independently maintained
column mappings in `fecfile/mappings.json`).

Both libraries descend from the fech-sources canonical field names, so a
disagreement is almost always one of:

* a column-position bug in one library's tables (the interesting kind);
* a version bucket one library covers and the other does not;
* a canonical-name difference (hardmoney renames upstream collisions and
  unifies spellings, see `NOTICE`), which this script bridges by *learning*
  aliases from column co-location -- a hardmoney field and a fecfile field
  that sit at the same column in the workbook-verified 8.5 layout (or, for
  fields that no longer exist in 8.5, in most other versions) are the same
  field. Learning rather than hand-writing the table keeps the oracle
  honest when names change and lets a field that moved in one version
  surface as a value disagreement instead of vanishing behind an alias;
* a wire-normalisation difference (whitespace, wrapping quotes, encoding).

Every disagreement is classified. Classes listed in `EXPLAINED` have been
investigated against the FEC's own artefacts (the v8.5 specification
workbook and the per-version column listings in
`data/fec-csv-sources/headers/`) and documented in
`tests/fixtures/ORACLE_NOTES.md`; the script exits non-zero if any
*unexplained* disagreement remains, which is what `tests/parser_oracle.rs`
asserts when `HARDMONEY_ORACLE_TESTS=1`.

Usage (from the repository root, with both libraries in the venv):

    tmp/venv/bin/python tests/oracle_fecfile.py [-v] [--max-mb 20] [--show N] [FILE...]
"""

from __future__ import annotations

import argparse
import csv
import glob
import os
import re
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass

import hardmoney
from fecfile import fecparser
from fecfile.cache import FecParserMissingMappingError, getMapping

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

CORPUS_GLOBS = [
    "tests/fixtures/*.fec",
    "tmp/agent-misc/f3scan/*.fec",
    "tmp/agent-misc/samples/*.fec",
    "tmp/agent-misc/filings/*.fec",
    "tmp/competitors/*/test*/**/*.fec",
]

# A representative wire token per hardmoney table, used to look up the
# fecfile mapping for that table when learning aliases.
TABLE_TOKENS: dict[str, str] = {
    "HDR": "HDR", "F1": "F1N", "F13": "F13N", "F132": "F132", "F133": "F133", "F1M": "F1MN",
    "F1S": "F1S", "F2": "F2N", "F2S": "F2S", "F24": "F24N", "F3": "F3N", "F3L": "F3LN",
    "F3P": "F3PN", "F3P31": "F3P31", "F3PS": "F3PS", "F3PZ1": "F3PZ1", "F3PZ2": "F3PZ2",
    "F3S": "F3S", "F3X": "F3XN", "F3Z": "F3Z", "F3Z1": "F3Z1", "F3Z2": "F3Z2", "F4": "F4N",
    "F5": "F5N", "F56": "F56", "F57": "F57", "F6": "F6N", "F65": "F65", "F7": "F7N",
    "F76": "F76", "F8": "F8N", "F82": "F8II", "F83": "F8III", "F9": "F9N", "F91": "F91",
    "F92": "F92", "F93": "F93", "F94": "F94", "F99": "F99", "F10": "F10", "F105": "F105",
    "H1": "H1", "H2": "H2", "H3": "H3", "H4": "H4", "H5": "H5", "H6": "H6",
    "SchA": "SA11AI", "SchA3L": "SA3L", "SchB": "SB21B", "SchC": "SC/10", "SchC1": "SC1",
    "SchC2": "SC2", "SchD": "SD10", "SchE": "SE", "SchF": "SF", "SchI": "SI", "SchL": "SL",
    "TEXT": "TEXT",
}
ELECTRONIC_VERSIONS = [
    "3.0", "5.0", "5.1", "5.2", "5.3", "6.1", "6.2", "6.3", "6.4", "7.0",
    "8.0", "8.1", "8.2", "8.3", "8.4", "8.5",
]


def fecfile_mapping(token: str, version: str) -> list[str] | None:
    try:
        return getMapping(fecparser.mappings, token, version)
    except FecParserMissingMappingError:
        return None


def hardmoney_layout(table: str, version: str) -> dict[str, int] | None:
    try:
        return dict(hardmoney.layout(table, version))
    except Exception:
        return None


def learn_aliases() -> dict[str, dict[str, str]]:
    """hardmoney name -> fecfile name per table, from column co-location.

    The 8.5 layout is checked column-by-column against the FEC's workbook
    by `tests/parser_oracle.rs`, so a pairing seen there outweighs any
    number of pairings in older versions. hardmoney fields absent from 8.5
    (historical columns) take the majority pairing over the versions where
    both libraries have a layout."""
    out: dict[str, dict[str, str]] = {}
    for table, token in TABLE_TOKENS.items():
        votes: dict[str, Counter] = defaultdict(Counter)
        for v in ELECTRONIC_VERSIONS:
            hm = hardmoney_layout(table, v)
            ff = fecfile_mapping(token, v)
            if hm is None or ff is None:
                continue
            weight = 1000 if v == "8.5" else 1
            for name, col in hm.items():
                if col < len(ff) and ff[col]:
                    votes[name][ff[col]] += weight
        out[table] = {n: c.most_common(1)[0][0] for n, c in votes.items() if c}
    return out


ALIASES = learn_aliases()
# The header is a struct in hardmoney (`Header`), not a layout, so its one
# differently-spelled key cannot be learned from columns.
ALIASES.setdefault("HDR", {})["fec_version_raw"] = "fec_version"


def alias(table: str, name: str) -> str:
    return ALIASES.get(table, {}).get(name, name)


# ---------------------------------------------------------------------------
# Explained disagreement classes. Key: (class, table, field-or-'*').
# Value: who is right and where the evidence is (details in ORACLE_NOTES.md).
# ---------------------------------------------------------------------------
EXPLAINED: dict[tuple[str, str, str], str] = {
    # -- fecfile is wrong ------------------------------------------------------
    ("only_hardmoney", "F24", "treasurer_name"):
        "3.x-5.x col 9 NAME/TREASURER exists (headers/3.csv, 5.2.csv); fecfile leaves the column unnamed.",
    ("only_hardmoney", "F57", "payee_street_1"):
        "3.x col 5 is STREET 1 and col 6 STREET 2 (headers/3.csv); fecfile puts payee_street_2 at col 5 and names col 6 nothing.",
    ("value", "F57", "payee_street_2"): "same F57 3.x defect: fecfile reads STREET 1 as payee_street_2.",
    ("only_hardmoney", "F5", "individual_occupation"):
        "5.3 col 12 INDOCC (headers/5.3.csv); fech-sources had it at 18 (NOTICE), fecfile inherited that.",
    ("only_hardmoney", "F3S", "*"):
        "3.x-5.x cols 26/29/30 (headers/3.csv, 5.2.csv); fech-sources had label text where the positions belong (NOTICE), fecfile inherited the gap.",
    ("only_hardmoney", "H1", "form_type"):
        "fecfile's H1 3.x mapping names column 1 `ballot_local_candidates` instead of form_type; FEC lists FORM TYPE (headers/3.csv).",
    ("only_hardmoney", "SchI", "transaction_id"):
        "SchI 6.x-7.0 col 3 TRANSACTION ID NUMBER (headers/6.1.csv-7.0.csv); fecfile leaves the column unnamed.",
    ("only_hardmoney", "F3L", "state_of_election"):
        "F3L col 14 STATE OF ELECTION (v8.5 workbook, headers/6.4.csv-8.5.csv); fecfile leaves the column unnamed.",
    ("only_hardmoney", "*", "amended_cd"):
        "3.x-5.x AMENDED CD column (headers/3.csv, 5.2.csv); fecfile leaves it unnamed on most tables.",
    ("fecfile_duplicate_name", "*", "*"):
        "fecfile maps two columns to one name and its dict keeps the later one; hardmoney renamed the pair (NOTICE). The value shown is the other column's.",
    ("only_hardmoney", "F2S", "*"):
        "fecfile has no F2S table: its `(^f2$)|(^f2[^4])` regex parses F2S lines with the F2 layout (FEC lists F2S since 6.1: headers/6.1.csv-8.5.csv).",
    ("only_fecfile", "F2S", "*"): "same: fecfile mis-parses F2S with the F2 layout.",
    ("value", "F2S", "*"): "same: fecfile mis-parses F2S with the F2 layout.",
    ("file", "*", "fecfile_missing_mapping"):
        "fecfile has no mapping for the token at this version (F3Z 8.2-8.5, SchI 8.0-8.4, ...) although the FEC lists it (headers/*.csv).",
    ("file", "*", "fecfile_short_line"):
        "fecfile drops any record with fewer than two fields; hardmoney parses it (a bare token is a valid, empty record).",
    ("encoding", "*", "*"):
        "fecfile decodes non-UTF-8 lines as ISO-8859-1; hardmoney as Windows-1252, where 0x80-0x9F are the curly quotes and dashes Windows software writes. cp1252 is right.",
    ("line_number", "*", "*"):
        "fecfile parses each line on its own but drops lines with <2 fields and mis-numbers after a [BEGINTEXT] block; hardmoney reports the physical line.",
    # -- differences by design ---------------------------------------------------
    ("only_hardmoney", "TEXT", "form_type"):
        "hardmoney names every column the FEC lists; fecfile leaves 3.x-5.x TEXT col 2 (FORM TYPE) unnamed in some buckets.",
    ("file", "*", "hardmoney_paper_version"):
        "hardmoney parses electronic filings only (spec 3.x-8.x); P* paper-conversion versions are rejected with UnknownElectronicHeaderVersion by design.",
    ("file", "*", "hardmoney_deprecated_header"):
        "'/*'-style pre-3.0 headers are rejected with DeprecatedHeaderFormat by design (fecfile reads the key=value header).",
    ("file", "*", "hardmoney_unknown_version"):
        "e.g. `180.5`: rejected by hardmoney; fecfile has no mapping for it either.",
    ("file", "*", "hardmoney_no_cover_line"):
        "empty file or header only: hardmoney fails with MissingFormLine; fecfile yields nothing.",
}


def normalise(v: str | None) -> str:
    if v is None:
        return ""
    v = v.strip()
    if len(v) >= 2 and v[0] == '"' and v[-1] == '"':
        v = v[1:-1].strip()
    return re.sub(r"\s+", " ", v).casefold()


def is_encoding_difference(hv: str, fv: str) -> bool:
    """True when the two strings are the same bytes read as cp1252 (hardmoney)
    vs. latin-1 (fecfile)."""
    try:
        return normalise(hv.encode("cp1252").decode("latin-1")) == normalise(fv)
    except UnicodeEncodeError:
        return False


@dataclass
class Disagreement:
    kind: str
    file: str
    line_no: int
    form_type: str
    table: str
    field: str
    hardmoney: str
    fecfile: str
    our_col: int | None
    their_col: int | None

    def key(self) -> tuple[str, str, str]:
        return (self.kind, self.table, self.field)

    def explained(self) -> str | None:
        for k in (self.key(), (self.kind, self.table, "*"), (self.kind, "*", self.field), (self.kind, "*", "*")):
            if k in EXPLAINED:
                return EXPLAINED[k]
        return None

    def __str__(self) -> str:
        hv = self.hardmoney if len(self.hardmoney) < 80 else self.hardmoney[:77] + "..."
        fv = self.fecfile if len(self.fecfile) < 80 else self.fecfile[:77] + "..."
        return (
            f"{self.kind:22} {os.path.relpath(self.file, ROOT)}:{self.line_no} {self.form_type} "
            f"[{self.table}] {self.field}: hardmoney={hv!r} (col {self.our_col}) "
            f"fecfile={fv!r} (col {self.their_col})"
        )


# ---------------------------------------------------------------------------
# fecfile side: per-physical-line parse with as_strings, catching per-line
# mapping failures so one unknown token does not abort the whole file.
# ---------------------------------------------------------------------------
@dataclass
class FfLine:
    line_no: int
    form: str
    fields: dict[str, str]
    mapping: list[str]


def fecfile_decode(raw: bytes) -> str:
    try:
        return raw.decode("utf-8")
    except UnicodeDecodeError:
        return raw.decode("ISO-8859-1")


def fecfile_parse(data: bytes):
    """Returns (header dict, version, {line_no: FfLine}, {line_no: reason}, f99_texts)."""
    lines = data.split(b"\n")
    header = None
    version = None
    body: dict[int, FfLine] = {}
    failures: dict[int, str] = {}
    f99_texts: list[tuple[int, str]] = []  # (line_no of [BEGINTEXT], text)
    text_section = False
    text_start = 0
    f99_text = ""
    for i, raw in enumerate(lines, 1):
        line = fecfile_decode(raw)
        if version is None:
            if line.startswith("/*"):
                return None, "legacy", body, failures, f99_texts
            if not line.strip():
                continue
            fields = fecparser.fields_from_line(line)
            if len(fields) < 3:
                return None, None, body, failures, f99_texts
            version = fields[2] if fields[1] == "FEC" else fields[1]
            try:
                header = fecparser.parse_line(line, version, 0, as_strings=True)
            except FecParserMissingMappingError as e:
                failures[i] = f"fecfile_missing_mapping: {e}"
            continue
        stripped = line.strip().upper()
        if stripped in ("[BEGINTEXT]", "[BEGIN TEXT]"):
            text_section = True
            text_start = i
            f99_text = ""
            continue
        if stripped in ("[ENDTEXT]", "[END TEXT]"):
            text_section = False
            f99_texts.append((text_start, f99_text))
            continue
        if text_section:
            f99_text = line if f99_text == "" else f99_text + "\n" + line
            continue
        if not line.strip():
            continue
        try:
            fields = fecparser.fields_from_line(
                line, use_ascii_28=not (version[0] in fecparser.comma_versions)
            )
        except csv.Error as e:
            failures[i] = f"fecfile_csv_error: {e}"
            continue
        if len(fields) < 2:
            failures[i] = "fecfile_short_line"
            continue
        form = fields[0].strip()
        mapping = fecfile_mapping(form, version)
        if mapping is None:
            failures[i] = "fecfile_missing_mapping"
            continue
        parsed = {mapping[k]: (fields[k] if k < len(fields) else "") for k in range(len(mapping))}
        body[i] = FfLine(i, form, parsed, mapping)
    return header, version, body, failures, f99_texts


# ---------------------------------------------------------------------------
# Comparison
# ---------------------------------------------------------------------------
class Report:
    def __init__(self) -> None:
        self.files = 0
        self.files_compared = 0
        self.lines_compared = 0
        self.fields_compared = 0
        self.disagreements: list[Disagreement] = []

    def add(self, d: Disagreement) -> None:
        self.disagreements.append(d)


def compare_record(
    rep: Report,
    path: str,
    line_no: int,
    table: str,
    version: str,
    hm_form: str,
    hm_fields: dict[str, str],
    ff: FfLine,
) -> None:
    rep.lines_compared += 1
    our_cols = hardmoney_layout(table, version) or {}
    # fecfile's dict keeps the *last* column for a duplicated name.
    their_cols: dict[str, int] = {}
    dup_names: set[str] = set()
    for i, n in enumerate(ff.mapping):
        if not n:
            continue
        if n in their_cols:
            dup_names.add(n)
        their_cols[n] = i
    seen_ff: set[str] = set()
    for name, hv in hm_fields.items():
        ff_name = alias(table, name)
        our_col = our_cols.get(name)
        if ff_name not in ff.fields:
            if normalise(hv):
                rep.add(Disagreement("only_hardmoney", path, line_no, hm_form, table, name, hv, "", our_col, None))
            continue
        seen_ff.add(ff_name)
        fv = ff.fields[ff_name]
        rep.fields_compared += 1
        if normalise(hv) != normalise(fv):
            if ff_name in dup_names and our_col != their_cols.get(ff_name):
                kind = "fecfile_duplicate_name"
            elif is_encoding_difference(hv, fv):
                kind = "encoding"
            else:
                kind = "value"
            rep.add(Disagreement(kind, path, line_no, hm_form, table, name, hv, fv, our_col, their_cols.get(ff_name)))
    for ff_name, fv in ff.fields.items():
        if ff_name in seen_ff or not ff_name:
            continue
        if normalise(fv):
            rep.add(
                Disagreement("only_fecfile", path, line_no, hm_form, table, ff_name, "", fv, None, their_cols.get(ff_name))
            )


def classify_hardmoney_error(e: Exception, data: bytes) -> str:
    msg = str(e)
    if "deprecated header" in msg:
        return "hardmoney_deprecated_header"
    if "malformed FEC version" in msg:
        # Paper-conversion files put `P<ver>` in the header's second field
        # (`HDR|P3.4|...`), where an electronic filing has `FEC`.
        first = fecfile_decode(data.split(b"\n", 1)[0])
        fields = fecparser.fields_from_line(first)
        if len(fields) > 1 and fields[1].strip().upper().startswith("P"):
            return "hardmoney_paper_version"
        return "hardmoney_unknown_version"
    if "no cover/summary line" in msg:
        return "hardmoney_no_cover_line"
    return f"hardmoney_error: {msg}"


def compare_file(rep: Report, path: str, verbose: bool) -> None:
    rep.files += 1
    with open(path, "rb") as fh:
        data = fh.read()
    rel = os.path.relpath(path, ROOT)

    try:
        filing = hardmoney.parse(data, lenient=True)
    except hardmoney.FecError as e:
        rep.add(Disagreement("file", path, e.line_no or 0, "", "*", classify_hardmoney_error(e, data), "", str(e), None, None))
        return

    header, version, ff_body, ff_failures, f99_texts = fecfile_parse(data)
    if version in (None, "legacy") or header is None:
        rep.add(Disagreement("file", path, 1, "", "*", "fecfile_no_header", "", str(version), None, None))
        return
    rep.files_compared += 1
    hm_version = filing.version

    # Header: fecfile parses the HDR line with its HDR mapping.
    hm_header = {k: str(v) for k, v in filing.header.items() if k != "version"}
    hdr_mapping = fecfile_mapping("HDR", version) or []
    compare_record(rep, path, 1, "HDR", hm_version, "HDR", hm_header, FfLine(1, "HDR", header, hdr_mapping))

    # Cover + body, aligned by physical line number (both sides report it).
    hm_lines = [filing.summary] + list(filing.lines)
    hm_by_no = {line.line_no: line for line in hm_lines}
    hm_line_nos = sorted(hm_by_no)
    for line in hm_lines:
        no = line.line_no
        fields = dict(line.items())
        ff = ff_body.get(no)
        if ff is None:
            reason = ff_failures.get(no)
            if reason is None:
                rep.add(Disagreement("line_number", path, no, line.form_type, line.table, "*", str(no), "", None, None))
            else:
                rep.add(Disagreement("file", path, no, line.form_type, line.table, reason.split(":")[0], "", reason, None, None))
            continue
        # A [BEGINTEXT] block spliced into `text`: fecfile yields it apart.
        block = None
        if fields.get("text") and not ff.fields.get("text", "").strip():
            following = [i for i in hm_line_nos if i > no]
            next_record = following[0] if following else float("inf")
            for start, txt in f99_texts:
                if no < start < next_record:
                    block = txt
                    break
        if block is not None:
            rep.fields_compared += 1
            if normalise(fields["text"]) != normalise(block):
                kind = "encoding" if is_encoding_difference(fields["text"], block) else "text_block"
                rep.add(Disagreement(kind, path, no, line.form_type, line.table, "text", fields["text"], block, None, None))
            fields = {k: v for k, v in fields.items() if k != "text"}
            ff = FfLine(ff.line_no, ff.form, {k: v for k, v in ff.fields.items() if k != "text"}, ff.mapping)
        compare_record(rep, path, no, line.table, hm_version, line.form_type, fields, ff)

    skipped_nos = {s["line_no"] for s in filing.skipped}
    for s in filing.skipped:
        if s["line_no"] in ff_body:
            rep.add(Disagreement("file", path, s["line_no"], s["form_type"], "*", "hardmoney_skipped", s["reason"], ff_body[s["line_no"]].form, None, None))
    for no, ff in ff_body.items():
        if no not in hm_by_no and no not in skipped_nos:
            rep.add(Disagreement("line_number", path, no, ff.form, "*", "*", "", str(no), None, None))
    if verbose:
        print(f"  {rel}: {len(hm_lines)} records, version {hm_version}")


def layout_report() -> None:
    """Static comparison of the two libraries' column tables, independent
    of any filing: for every table and electronic version, which fields sit
    at different columns (after aliasing), which columns only one side
    names, and which (table, version) pairs only one side covers. This is
    the evidence base for `tests/fixtures/ORACLE_NOTES.md`; the corpus run
    only exercises the subset real filings happen to contain."""
    print("== learned aliases (hardmoney name -> fecfile name, where they differ)")
    for table in sorted(ALIASES):
        diffs = {k: v for k, v in ALIASES[table].items() if k != v}
        if diffs:
            print(f"  {table}: " + ", ".join(f"{k}->{v}" for k, v in sorted(diffs.items())))
    print("\n== coverage: (table, version) with a layout on one side only")
    for table, token in TABLE_TOKENS.items():
        for v in ELECTRONIC_VERSIONS:
            hm = hardmoney_layout(table, v)
            ff = fecfile_mapping(token, v)
            if (hm is None) != (ff is None):
                print(f"  {table} {v}: hardmoney={'yes' if hm else 'no'} fecfile={'yes' if ff else 'no'}")
    print("\n== same field, different column (table, version, hardmoney name@col, fecfile name@col)")
    print("== and columns named on one side only")
    for table, token in TABLE_TOKENS.items():
        for v in ELECTRONIC_VERSIONS:
            hm = hardmoney_layout(table, v)
            ff = fecfile_mapping(token, v)
            if hm is None or ff is None:
                continue
            ff_cols: dict[str, list[int]] = defaultdict(list)
            for i, n in enumerate(ff):
                if n:
                    ff_cols[n].append(i)
            hm_cols = {c: n for n, c in hm.items()}
            for name, col in sorted(hm.items(), key=lambda kv: kv[1]):
                ff_name = alias(table, name)
                if ff_name not in ff_cols:
                    print(f"  {table} {v}: {name}@{col + 1} has no fecfile column")
                elif col not in ff_cols[ff_name]:
                    print(f"  {table} {v}: {name}@{col + 1} vs fecfile {ff_name}@{[c + 1 for c in ff_cols[ff_name]]}")
            for i, n in enumerate(ff):
                if n and i not in hm_cols:
                    print(f"  {table} {v}: fecfile {n}@{i + 1} has no hardmoney field")


def corpus(max_mb: float) -> list[str]:
    out: list[str] = []
    for g in CORPUS_GLOBS:
        out.extend(glob.glob(os.path.join(ROOT, g), recursive=True))
    paths = sorted({os.path.abspath(p) for p in out})
    return [p for p in paths if os.path.getsize(p) <= max_mb * 1024 * 1024]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("files", nargs="*")
    ap.add_argument("--verbose", "-v", action="store_true")
    ap.add_argument("--max-mb", type=float, default=20.0)
    ap.add_argument("--show", type=int, default=5, help="examples per class")
    ap.add_argument("--layouts", action="store_true", help="print the static layout comparison and exit")
    args = ap.parse_args(argv)

    if args.layouts:
        layout_report()
        return 0

    files = [os.path.abspath(f) for f in args.files] or corpus(args.max_mb)
    rep = Report()
    for path in files:
        try:
            compare_file(rep, path, args.verbose)
        except Exception as e:  # a crash in either library is itself a finding
            rep.add(Disagreement("crash", path, 0, "", "*", type(e).__name__, "", str(e), None, None))

    by_class: dict[tuple[str, str, str], list[Disagreement]] = defaultdict(list)
    for d in rep.disagreements:
        by_class[d.key()].append(d)

    print(f"files: {rep.files} scanned, {rep.files_compared} compared by both libraries")
    print(f"records compared: {rep.lines_compared}; field values compared: {rep.fields_compared}")
    print(f"disagreements: {len(rep.disagreements)} in {len(by_class)} classes\n")
    unexplained = 0
    for key, ds in sorted(by_class.items(), key=lambda kv: (kv[1][0].explained() is not None, kv[0])):
        why = ds[0].explained()
        tag = "explained  " if why else "UNEXPLAINED"
        if not why:
            unexplained += len(ds)
        files_in = len({d.file for d in ds})
        print(f"[{tag}] {key[0]} {key[1]} {key[2]}: {len(ds)} occurrence(s) in {files_in} file(s)")
        if why:
            print(f"    -> {why}")
        for d in ds[: args.show]:
            print(f"    {d}")
        if len(ds) > args.show:
            print(f"    ... {len(ds) - args.show} more")
    print()
    if unexplained:
        print(f"UNEXPLAINED disagreements: {unexplained}")
        return 1
    print("no unexplained disagreements")
    return 0


if __name__ == "__main__":
    sys.exit(main())
