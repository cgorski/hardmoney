#!/usr/bin/env python3
"""
Extends the fec-csv-sources format tables (originally topping out at v8.0-8.2)
to cover FEC electronic filing spec versions 8.3, 8.4, and 8.5.

Methodology (documented for auditability):

1. For EVERY form CSV, the top-of-file version bucket (the newest regex
   column) is widened to also match 8.3, 8.4, and 8.5 filings, UNLESS the
   FEC's own published build changelog documents a field-layout change for
   that specific form in that version range (see SPECIAL_CASES below). This
   follows the FEC's own established backward-compatible convention verified
   by inspecting the existing CSVs: previous version bumps (e.g. 7.0 -> 8.0)
   repeatedly reused the same column positions with no field changes, only
   splitting into a new bucket when a field was actually added or moved.

2. Three forms have documented, dated field additions in the FEC's official
   FECFile build changelog
   (https://www.fec.gov/help-candidates-and-committees/filing-reports/fecfile-software/):

   - Form 9 (F9): Build 8.3 (August 15, 2018) added a new "Original Amendment
     Date" field ("FECPrint - A new date field 'Original Amendment Date' has
     been added to Form 9."). New field appended after the last existing
     column, per the FEC's standard append-only convention.
   - Form 99 (F99): Build 8.5.0.0 (September 2, 2025) required categorizing
     every submission (submission_category) and, for two of those categories,
     a monthly/quarterly filing_frequency designator
     (https://www.fec.gov/updates/fec-updates-electronic-filing-specifications/).
     Two fields appended.
   - Schedule C-2 (SchC2): Build 8.5.0.0 (September 2, 2025) allowed listing a
     *registered committee* (rather than only an individual) as a loan
     guarantor. Two fields appended (guarantor_committee_id,
     guarantor_committee_name).

   No official FEC document publishes the exact byte/column position of these
   new fields (the FEC has never published a downloadable field-position spec
   for 8.3-8.5 as of this writing -- confirmed by direct inspection of
   https://www.fec.gov/help-candidates-and-committees/filing-reports/other-filing-software/
   and the FECFileManual PDF, which is a GUI user guide, not a field-position
   spec). Per the FEC's consistently observed convention across every prior
   version bump in this same dataset, new fields are appended as new trailing
   columns rather than inserted or reordered. This script follows that
   convention. This is a best-effort, clearly-flagged extrapolation, NOT a
   value confirmed from an official byte-position specification -- see
   README.md "Assumptions and known limitations" for how this was
   cross-checked against real 2026 sample filings.

All other documented 8.1-8.5 changes recorded in the FEC changelog are either:
  (a) about forms nyt-pyfec (and therefore this port) never parsed at all
      (Form 1, Form 1S, Form 2 administrative/registration forms), or
  (b) changes to allowed *values* of an already-existing field (e.g. new
      Form 1 "committee_type" options in build 8.4), which require no
      change to column position tables, or
  (c) FECCheck validation-rule changes or FECFile/FECPrint software-only
      fixes that do not alter the .fec data file layout at all.
These are left untouched.
"""
import csv
import io
import os
import sys

SRC_DIR = os.path.join(os.path.dirname(__file__), "..", "data", "fec-csv-sources")

NEW_VERSIONS = ["8.5", "8.4", "8.3"]

# form_csv -> (new_bucket_versions, [(canonical_name, LABEL), ...] appended fields)
# new_bucket_versions is the subset of NEW_VERSIONS that gets the NEW bucket
# (with the extra fields). Any of NEW_VERSIONS not listed here is folded into
# the existing top (unchanged-layout) bucket instead.
SPECIAL_CASES = {
    "F9.csv": {
        "new_bucket_versions": ["8.5", "8.4", "8.3"],
        "fields": [("original_amendment_date", "ORIGINAL AMENDMENT DATE")],
    },
    "F99.csv": {
        "new_bucket_versions": ["8.5"],
        "fields": [
            ("submission_category", "SUBMISSION CATEGORY"),
            ("filing_frequency_designation", "FILING FREQUENCY DESIGNATION (MONTHLY/QUARTERLY)"),
        ],
    },
    "SchC2.csv": {
        "new_bucket_versions": ["8.5"],
        "fields": [
            ("guarantor_committee_id", "GUARANTOR COMMITTEE FEC ID"),
            ("guarantor_committee_name", "GUARANTOR COMMITTEE NAME"),
        ],
    },
}


def read_csv_rows(path):
    with open(path, "r", newline="", encoding="utf-8") as f:
        return list(csv.reader(f))


def write_csv_rows(path, rows):
    with open(path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerows(rows)


def widen_top_bucket(header_row, extra_versions):
    """Append extra version strings (as alternation) to the first (index-1)
    regex bucket in the header row, unless already present."""
    top_regex = header_row[1]
    missing = [v for v in extra_versions if v not in top_regex]
    if not missing:
        return header_row
    new_regex = top_regex + "".join(f"|{v}" for v in missing)
    new_header = list(header_row)
    new_header[1] = new_regex
    return new_header


def process_plain(path):
    rows = read_csv_rows(path)
    header = rows[0]
    if not header or header[0] != "canonical":
        print(f"  SKIP (unexpected header): {path}")
        return
    new_header = widen_top_bucket(header, NEW_VERSIONS)
    if new_header == header:
        print(f"  no change needed: {os.path.basename(path)}")
        return
    rows[0] = new_header
    write_csv_rows(path, rows)
    print(f"  widened top bucket: {os.path.basename(path)} -> {new_header[1]}")


def process_special(path, spec):
    rows = read_csv_rows(path)
    header = rows[0]
    if not header or header[0] != "canonical":
        print(f"  SKIP (unexpected header): {path}")
        return

    new_bucket_versions = spec["new_bucket_versions"]
    fold_versions = [v for v in NEW_VERSIONS if v not in new_bucket_versions]

    # 1) fold any non-new-bucket new versions into the existing top bucket
    #    (unchanged layout).
    header = widen_top_bucket(header, fold_versions)

    # 2) insert a brand new bucket (2 columns: regex, label-placeholder) right
    #    after the "canonical" column, ahead of the existing top bucket, for
    #    new_bucket_versions.
    new_bucket_regex = "^" + "|".join(new_bucket_versions)
    header = [header[0], new_bucket_regex, ""] + header[1:]

    new_rows = [header]
    old_ncols = len(rows[0])  # original width, before we inserted 2 cols above
    for row in rows[1:]:
        if not row:
            new_rows.append(row)
            continue
        # pad to original width so index math is stable
        padded = row + [""] * (old_ncols - len(row)) if len(row) < old_ncols else row
        # The new bucket has the SAME layout as the existing top bucket for
        # every pre-existing field (only the newly appended fields below
        # differ) -- so copy columns 1,2 (position, label) into the new
        # leading bucket slot too.
        new_row = [padded[0], padded[1], padded[2]] + list(padded[1:])
        new_rows.append(new_row)

    # 3) append the new fields as new trailing rows, populated ONLY in the
    #    new bucket's two columns; blank everywhere else.
    ncols = len(new_rows[0])
    # determine the last used column position in the (now-shifted) top
    # unchanged bucket, columns index 3 and 4 (regex/label pair right after
    # our newly inserted bucket), to compute next available position.
    max_pos = 0
    for row in new_rows[1:]:
        if len(row) > 3 and row[3]:
            try:
                max_pos = max(max_pos, int(row[3]))
            except ValueError:
                pass
    next_pos = max_pos + 1
    for canonical, label in spec["fields"]:
        new_row = [canonical, str(next_pos), label] + [""] * (ncols - 3)
        new_rows.append(new_row)
        next_pos += 1

    write_csv_rows(path, new_rows)
    print(
        f"  added new bucket '{new_bucket_regex}' with {len(spec['fields'])} new field(s) "
        f"(positions {max_pos + 1}-{next_pos - 1}); folded {fold_versions} into existing bucket: {os.path.basename(path)}"
    )


def main():
    if not os.path.isdir(SRC_DIR):
        print(f"ERROR: source dir not found: {SRC_DIR}", file=sys.stderr)
        sys.exit(1)

    for fname in sorted(os.listdir(SRC_DIR)):
        if not fname.endswith(".csv"):
            continue
        path = os.path.join(SRC_DIR, fname)
        if fname == "HDR.csv":
            # HDR.csv's top bucket is "^[6-8]" which already matches any
            # 8.x version by construction; no change needed or possible in
            # the same way (it's a character-class regex, not enumerated
            # dot-versions).
            print(f"  no change needed (already covers 8.x): {fname}")
            continue
        if fname in SPECIAL_CASES:
            print(f"Processing {fname} (special case)")
            process_special(path, SPECIAL_CASES[fname])
        else:
            process_plain(path)


if __name__ == "__main__":
    main()
