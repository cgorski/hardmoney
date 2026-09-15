#!/usr/bin/env python3
"""Make the canonical field vocabulary consistent across the format tables.

The canonical names in ``data/fec-csv-sources/*.csv`` (column 1 of every
row) were inherited from fech-sources, where each table was named on its
own and the same concept ended up spelled differently from one table to the
next (``transaction_id`` on Schedule A, ``transaction_id_number`` on
Schedule B; ``memo_text`` on H4, ``memo_text_description`` on Schedule A;
``col_a_individual_contributions_itemized`` on F3 but
``col_a_individuals_itemized`` on F3X). This script holds the rename map
as data, one family per concept with the rule that picked the winner, and
applies it so the change is reproducible and reviewable.

Rules used to pick a canonical spelling:

* One spelling per concept across every table. Where two names meant the
  same thing on the same kind of record, the shorter FEC-faithful one wins
  (``transaction_id``, ``back_reference_tran_id``, ``memo_text``), or the
  majority spelling where both are equally faithful.
* ZIP fields end in ``_zip_code``; street fields end in ``_street_1`` /
  ``_street_2``; middle names end in ``_middle_name``.
* Cover-page lines keep their ``col_a_`` / ``col_b_`` prefix and are only
  unified where the FEC's own line labels describe the same line. Lines
  whose FEC labels genuinely differ keep their form-specific names. The
  line-number-prefixed tables (F3S, F3PS) are a different convention and
  are left alone.
* Nothing but column 1 changes: no column position, label, or version
  bucket is touched.

Usage::

    python3 scripts/rename_canonical.py            # apply
    python3 scripts/rename_canonical.py --dry-run  # report only
    python3 scripts/rename_canonical.py --appendix book/src/field-names.md
                                                   # regenerate the appendix

The script is idempotent: a rename whose old name is gone and whose new
name is present is reported as already applied.

Guardrails: a rename is refused if the old name is missing, if the new
name already exists in the same table (unless the pair is listed in
``MERGES`` *and* the two rows never both have a position in one version
bucket), or if any resulting name is not ``lower_snake_case``.
"""

from __future__ import annotations

import argparse
import csv
import io
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCES = ROOT / "data" / "fec-csv-sources"
SPEC = ROOT / "data" / "fec-spec" / "spec-8.5.json"

# ---------------------------------------------------------------------------
# The rename map, one family per concept.
# ---------------------------------------------------------------------------

Renames = dict[tuple[str, str], str]


def _family(new: str, tables: str, old: str) -> Renames:
    """Same (old -> new) on several tables."""
    return {(t, old): new for t in tables.split()}


def _cols(table: str, pairs: dict[str, str]) -> Renames:
    """``col_a_``/``col_b_`` pairs on one cover page (both columns)."""
    out: Renames = {}
    for old, new in pairs.items():
        for c in ("col_a_", "col_b_"):
            out[(table, c + old)] = c + new
    return out


FAMILIES: list[tuple[str, Renames]] = [
    (
        # Transaction id: `transaction_id` on 17 tables, `transaction_id_number`
        # on 16 (the FEC labels both "TRANSACTION ID NUMBER"). Shortest wins.
        # SchI carried both -- `transaction_id_number` in the 6.x-8.4 bucket
        # and `transaction_id` ("TRAN ID") in the 3.x/5.x bucket -- and is a
        # merge into one name.
        "transaction id -> transaction_id",
        _family(
            "transaction_id",
            "F132 F133 F3P31 F57 H4 H6 SchB SchC SchC1 SchC2 SchD SchE SchF SchI SchL TEXT",
            "transaction_id_number",
        ),
    ),
    (
        # Back reference: `back_reference_tran_id_number` was already uniform,
        # but pairs with `transaction_id` only if it also drops `_number`
        # (the FEC's own H3/F94 label is "BACK REFERENCE TRAN ID"). TEXT
        # alone said `back_reference_sched_form_name`.
        "back reference -> back_reference_tran_id / back_reference_sched_name",
        {
            **_family(
                "back_reference_tran_id",
                "F132 F133 F92 F93 F94 H3 H4 H6 SchA SchA3L SchB SchC1 SchC2 SchE SchF TEXT",
                "back_reference_tran_id_number",
            ),
            ("TEXT", "back_reference_sched_form_name"): "back_reference_sched_name",
        },
    ),
    (
        # Memo text: `memo_text` (H4, H6, SchA3L) vs `memo_text_description`
        # (8 tables) for the same "MEMO TEXT/DESCRIPTION" column. Shortest
        # wins; it is also what the FEC's own API calls the field.
        "memo -> memo_text",
        _family("memo_text", "F132 F133 F3P31 SchA SchB SchC SchE SchF", "memo_text_description"),
    ),
    (
        # ZIP: every ZIP field is `*_zip_code` except two.
        "zip -> *_zip_code",
        _family("contributor_zip_code", "F132 F133", "contributor_zip"),
    ),
    (
        # Street: every street field is `*_street_1` / `*_street_2` except
        # the Schedule A conduit block.
        "street -> *_street_1 / *_street_2",
        {
            ("SchA", "conduit_street1"): "conduit_street_1",
            ("SchA", "conduit_street2"): "conduit_street_2",
            ("SchA3L", "conduit_street1"): "conduit_street_1",
            ("SchA3L", "conduit_street2"): "conduit_street_2",
        },
    ),
    (
        # Election date/state on cover pages. "DATE OF ELECTION" was
        # `election_date` on F3/F3L/F5/F7 and `date_of_election` on F3P/F3X;
        # the noun-first form matches `election_code`, `contribution_date`,
        # `coverage_from_date`. "STATE OF ELECTION" (the report's election)
        # is `state_of_election` on F3/F3P/F3X; F5 and F7 called the same
        # field `election_state`, which on F3/F3L is a *different* field
        # ("ELECTION STATE", the candidate's state, next to ELECTION
        # DISTRICT).
        "election date/state -> election_date / state_of_election",
        {
            ("F3P", "date_of_election"): "election_date",
            ("F3X", "date_of_election"): "election_date",
            ("F5", "election_state"): "state_of_election",
            ("F7", "election_state"): "state_of_election",
        },
    ),
    (
        # Candidate id without a role prefix: `candidate_id_number` on eight
        # tables ("CANDIDATE ID NUMBER", "S/O CANDIDATE ID NUMBER"); F10 said
        # `candidate_id`; the pre-6.x candidate block on F56/F65 said
        # `candidate_id` and on F3P31/H4/H6/SchD `fec_candidate_id_number`
        # for the same "FEC CANDIDATE ID NUMBER"; F2S said
        # `filer_candidate_id_number` for the column F2 calls
        # `candidate_id_number` (both "FILER CANDIDATE ID NUMBER").
        "candidate id -> candidate_id_number",
        {
            ("F10", "candidate_id"): "candidate_id_number",
            ("F56", "candidate_id"): "candidate_id_number",
            ("F65", "candidate_id"): "candidate_id_number",
            ("F3P31", "fec_candidate_id_number"): "candidate_id_number",
            ("H4", "fec_candidate_id_number"): "candidate_id_number",
            ("H6", "fec_candidate_id_number"): "candidate_id_number",
            ("SchD", "fec_candidate_id_number"): "candidate_id_number",
            ("F2S", "filer_candidate_id_number"): "candidate_id_number",
        },
    ),
    (
        # Committee id: `*_committee_id_number` everywhere (filer, lender,
        # creditor, subordinate, designating, affiliated, authorized, payee
        # on SchF) except two "cmtte"/"cmte" abbreviations.
        "committee id -> *_committee_id_number / *_committee_type",
        {
            ("F57", "payee_cmtte_fec_id_number"): "payee_committee_id_number",
            ("SchE", "payee_cmtte_fec_id_number"): "payee_committee_id_number",
            ("F1S", "joint_fund_participant_cmte_type"): "joint_fund_participant_committee_type",
        },
    ),
    (
        # Names: every middle name is `*_middle_name` except one FEC
        # abbreviation ("LENDER CANDIDATE MIDDLE NM"). The whole-name
        # "as signed" fields on Schedule E follow the table's own
        # `completing_*` block and F5's `notary_name`.
        "names -> *_middle_name, notary_name, completing_name",
        {
            ("SchC", "lender_candidate_middle_nm"): "lender_candidate_middle_name",
            ("SchE", "ind_name_notary"): "notary_name",
            ("SchE", "ind_name_as_signed"): "completing_name",
        },
    ),
    (
        # Purpose description: the FEC label is "EXPENDITURE PURPOSE DESCRIP"
        # on every schedule and `expenditure_purpose_descrip` on five tables
        # (as is `contribution_purpose_descrip`); H4/H6 spelled it out.
        "purpose description -> *_purpose_descrip",
        _family("expenditure_purpose_descrip", "H4 H6", "expenditure_purpose_description"),
    ),
    (
        # Codes: "AMENDED CD" is `amended_cd` on eight tables; F57 alone
        # said `amended_code`. F5's pre-6.x "RPTPGI" is the field the FEC
        # itself relabelled "ELECTION CODE {was RPTPGI}". F92's pre-6.x
        # "TRANS CODE" is F3P31's "TRANSACTION CODE".
        "codes -> amended_cd, election_code, transaction_code",
        {
            ("F57", "amended_code"): "amended_cd",
            ("F5", "report_pgi"): "election_code",
            ("F92", "transaction_type"): "transaction_code",
        },
    ),
    (
        # Single-table oddities that name a concept another table already
        # names: the SI/SL account reference ("Reference to SI or SL system
        # code that identifies the Account") is `reference_code` on SchA and
        # SchA3L; H3's event name is the key into H2's `activity_event_name`
        # ("ACTIVITY/EVENT NAME" / "EVENT/ACTIVITY ID/NAME"); the F5/F9
        # yes/no for qualified non-profit status.
        "shared concepts -> reference_code, activity_event_name, qualified_nonprofit",
        {
            ("SchB", "reference_to_si_or_sl_system_code_that_identifies_the_account"): "reference_code",
            ("H3", "event_activity_name"): "activity_event_name",
            ("F9", "qualified_non_profit"): "qualified_nonprofit",
        },
    ),
    # --- Cover pages -------------------------------------------------------
    # The rule: unify only where the FEC's line labels describe the same
    # line. Six tables (F3, F3Z, F3Z1, F3Z2, F3PZ1, F3PZ2) use the F3
    # vocabulary for contributions/loans/repayments/refunds, F3P and F3X
    # each had their own; the majority spelling wins for shared lines.
    (
        # F3 11(a)(i)-(iii): "Individuals Itemized" / "Individuals
        # Unitemized" / "Individual Contribution Total" are the F3X/F3P
        # names. 23 "Cash Beginning Reporting Period" is the same line as
        # F3X 6(b) / F3P 6 "Cash on Hand Beginning Period". 25 "Subtotals"
        # is F3X/F3P/F4 "Subtotal".
        "cover F3 -> F3X/F3P spellings",
        {
            **_cols(
                "F3",
                {
                    "individual_contributions_itemized": "individuals_itemized",
                    "individual_contributions_unitemized": "individuals_unitemized",
                    "total_individual_contributions": "individual_contribution_total",
                },
            ),
            ("F3", "col_a_cash_beginning_reporting_period"): "col_a_cash_on_hand_beginning_period",
            ("F3", "col_a_subtotals"): "col_a_subtotal",
        },
    ),
    (
        # F3Z / F3Z1 / F3Z2 (F3's per-committee schedules) use the F3 line
        # numbers; align them with F3.
        "cover F3Z* -> F3 spellings",
        {
            ("F3Z", "col_a_individual_contributions_itemized"): "col_a_individuals_itemized",
            ("F3Z", "col_a_cash_beginning_reporting_period"): "col_a_cash_on_hand_beginning_period",
            **{
                (t, "col_a_" + old): "col_a_" + new
                for t in ("F3Z1", "F3Z2")
                for old, new in {
                    "loan_repayments_by_candidate": "candidate_loan_repayments",
                    "loan_repayments_other": "other_loan_repayments",
                    "loan_repayments_total": "total_loan_repayments",
                    "refunds_individuals": "refunds_to_individuals",
                    "refunds_parties": "refunds_to_party_committees",
                    "refunds_pacs": "refunds_to_other_committees",
                    "refunds_total": "total_refunds",
                    "cash_beginning_reporting_period": "cash_on_hand_beginning_period",
                }.items()
            },
        },
    ),
    (
        # F3PZ1 / F3PZ2 (F3P's per-committee schedules) use the F3P line
        # numbers: 27(c) is "Total Loan Repayments Made" (F3P/F3X name),
        # 6/10 are F3P's summary-block cash lines, 20(d) is "Total Offsets
        # to Expenditures".
        "cover F3PZ* -> F3P/F3 spellings",
        {
            (t, "col_a_" + old): "col_a_" + new
            for t in ("F3PZ1", "F3PZ2")
            for old, new in {
                "loan_repayments_by_candidate": "candidate_loan_repayments",
                "loan_repayments_other": "other_loan_repayments",
                "loan_repayments_total": "total_loan_repayments_made",
                "refunds_individuals": "refunds_to_individuals",
                "refunds_parties": "refunds_to_party_committees",
                "refunds_pacs": "refunds_to_other_committees",
                "refunds_total": "total_refunds",
                "cash_beginning_reporting_period": "cash_on_hand_beginning_period",
                "cash_on_hand_close": "cash_on_hand_close_of_period",
                "total_offset": "total_offsets_to_expenditures",
            }.items()
        },
    ),
    (
        # F3P: 17(b)-(d), 19(a), 24, 27(a)-(b), 28(a)-(d) carry the same
        # labels as the F3-family lines and take those names; 20(a)-(c)
        # ("Operating", "Fundraising", "Legal and Accounting" *offsets*)
        # take F3PZ1's unambiguous names; 26 takes F3PZ1's shorter name.
        # Three Column B names disagreed with their own Column A
        # (17(e), 20(d), 27(a)).
        "cover F3P -> F3/F3PZ spellings",
        {
            **_cols(
                "F3P",
                {
                    "political_party_committees_receipts": "political_party_contributions",
                    "other_political_committees_pacs": "pac_contributions",
                    "the_candidate": "candidate_contributions",
                    "received_from_or_guaranteed_by_cand": "candidate_loans",
                    "operating": "offset_to_operating_expenditures",
                    "fundraising": "offset_to_fundraising_expenditures",
                    "legal_and_accounting": "offset_to_legal_expenditures",
                    "transfers_to_other_authorized_committees": "transfers_to_authorized",
                    "exempt_legal_accounting_disbursement": "exempt_legal_disbursements",
                    "other_repayments": "other_loan_repayments",
                    "individuals": "refunds_to_individuals",
                    "political_party_committees_refunds": "refunds_to_party_committees",
                    "other_political_committees": "refunds_to_other_committees",
                    "total_contributions_refunds": "total_refunds",
                },
            ),
            ("F3P", "col_a_made_or_guaranteed_by_candidate"): "col_a_candidate_loan_repayments",
            ("F3P", "col_b_made_or_guaranteed_by_the_candidate"): "col_b_candidate_loan_repayments",
            ("F3P", "col_b_total_contributions_other_than_loans"): "col_b_total_contributions",
            ("F3P", "col_b_total_offsets_to_operating_expenditures"): "col_b_total_offsets_to_expenditures",
        },
    ),
    (
        # F3X 11(b)/(c) "Political Party Committees" / "Other Political
        # Committees (PACs)" are F3's 11(b)/(c).
        "cover F3X -> F3 spellings",
        _cols(
            "F3X",
            {
                "political_party_committees": "political_party_contributions",
                "other_political_committees_pacs": "pac_contributions",
            },
        ),
    ),
    (
        # F4 6(b) is the same line as F3X 6(b); Column B 12(b) had a typo.
        "cover F4",
        {
            ("F4", "col_a_cash_on_hand_beginning_reporting_period"): "col_a_cash_on_hand_beginning_period",
            ("F4", "col_b_prior_expendiutres_subject_to_limits"): "col_b_prior_expenditures_subject_to_limits",
        },
    ),
    (
        # Schedule L line 9 "SUBTOTAL" is Schedule I's line 9 `col_a_subtotal`.
        "cover SchL -> SchI spelling",
        _cols("SchL", {"subtotal_period": "subtotal"}),
    ),
]

# Renames whose new name already exists in the table on rows that never
# share a version bucket with the renamed row (so the two rows describe
# the same field in different eras, exactly as build.rs already allows).
MERGES: frozenset[tuple[str, str]] = frozenset({("SchI", "transaction_id_number")})

SNAKE = re.compile(r"^[a-z0-9_]+$")


def renames() -> Renames:
    out: Renames = {}
    for _, fam in FAMILIES:
        for key, new in fam.items():
            if key in out:
                raise SystemExit(f"duplicate rename key {key}")
            out[key] = new
    return out


# ---------------------------------------------------------------------------
# CSV access. Files are handled as bytes and split on b"\n" only; the sole
# mutation is the prefix of a row before its first comma.
# ---------------------------------------------------------------------------


class Table:
    def __init__(self, path: Path) -> None:
        self.path = path
        self.name = path.stem
        self.raw = path.read_bytes()
        self.lines = self.raw.split(b"\n")
        header = next(csv.reader([self.lines[0].decode("utf-8", "replace")]))
        # Column index of every version-bucket pattern cell.
        self.buckets = [
            (i, c.strip()) for i, c in enumerate(header) if c.strip() and c.strip() != "canonical"
        ]

    def col1(self, line: bytes) -> str:
        return line.split(b",", 1)[0].decode("ascii", "replace").strip()

    def rows(self, name: str) -> list[int]:
        return [i for i, l in enumerate(self.lines[1:], 1) if self.col1(l) == name]

    def names(self) -> set[str]:
        return {self.col1(l) for l in self.lines[1:] if l.strip()}

    def positions(self, line_no: int) -> dict[str, str]:
        """bucket pattern -> position cell (only non-blank, non-zero)."""
        cells = next(csv.reader([self.lines[line_no].decode("utf-8", "replace")]))
        out = {}
        for i, pat in self.buckets:
            cell = cells[i].strip() if i < len(cells) else ""
            if cell and cell not in ("0", "0.0"):
                out[pat] = cell
        return out

    def rename_row(self, line_no: int, new: str) -> None:
        line = self.lines[line_no]
        _, rest = line.split(b",", 1)
        self.lines[line_no] = new.encode("ascii") + b"," + rest

    def write(self) -> None:
        self.path.write_bytes(b"\n".join(self.lines))


def check_merge(t: Table, old_rows: list[int], new_rows: list[int]) -> str | None:
    """Both names may coexist only if no bucket positions both of them."""
    for o in old_rows:
        po = t.positions(o)
        for n in new_rows:
            pn = t.positions(n)
            clash = sorted(set(po) & set(pn))
            if clash:
                return f"bucket(s) {clash} position both rows"
    return None


def apply(dry_run: bool) -> int:
    tables = {p.stem: Table(p) for p in sorted(SOURCES.glob("*.csv"))}
    errors: list[str] = []
    applied = 0
    already = 0
    per_family: list[tuple[str, int, int]] = []

    for label, fam in FAMILIES:
        fam_applied = 0
        fam_already = 0
        for (tname, old), new in fam.items():
            t = tables.get(tname)
            if t is None:
                errors.append(f"{tname}: no such table")
                continue
            if not SNAKE.match(new):
                errors.append(f"{tname}.{old} -> {new!r}: not lower_snake_case")
                continue
            old_rows = t.rows(old)
            new_rows = t.rows(new)
            if not old_rows:
                if new_rows:
                    fam_already += 1
                    continue
                errors.append(f"{tname}.{old}: old name not found")
                continue
            if new_rows:
                if (tname, old) not in MERGES:
                    errors.append(f"{tname}.{old} -> {new}: new name already exists (not a listed merge)")
                    continue
                why = check_merge(t, old_rows, new_rows)
                if why:
                    errors.append(f"{tname}.{old} -> {new}: merge refused, {why}")
                    continue
            for r in old_rows:
                t.rename_row(r, new)
            fam_applied += 1
        per_family.append((label, fam_applied, fam_already))
        applied += fam_applied
        already += fam_already

    if errors:
        print("REFUSED; no files written:", file=sys.stderr)
        for e in errors:
            print(f"  {e}", file=sys.stderr)
        return 1

    touched = 0
    for t in tables.values():
        new_raw = b"\n".join(t.lines)
        if new_raw != t.raw:
            touched += 1
            if not dry_run:
                t.write()

    width = max(len(l) for l, _, _ in per_family)
    for label, a, b in per_family:
        note = f"  ({b} already applied)" if b else ""
        print(f"{label:<{width}}  {a:3d}{note}")
    print(f"{'total':<{width}}  {applied:3d}  renames across {touched} tables"
          + (" (dry run, nothing written)" if dry_run else ""))
    if already and not applied:
        print(f"all {already} renames were already applied")
    return 0


# ---------------------------------------------------------------------------
# Book appendix: every table's field names with the FEC label.
# ---------------------------------------------------------------------------


def _pos(cell: str) -> int | None:
    m = re.fullmatch(r"(\d+)(?:\.0+)?", cell.strip())
    if not m:
        return None
    return int(m.group(1)) or None


def appendix() -> str:
    spec_tables = json.loads(SPEC.read_text())["tables"]
    out = io.StringIO()
    out.write("## Appendix: every field, by table\n\n")
    out.write(
        "Generated by `scripts/rename_canonical.py --appendix` from "
        "`data/fec-csv-sources/*.csv` and `data/fec-spec/spec-8.5.json`. "
        "The column is the 1-based position in the newest version bucket; "
        "blank means the field is placed only in an older or paper-format "
        "bucket. The label is the FEC's own (spec 8.5 where the table is "
        "still documented, otherwise the label from the format table; blank "
        "where the paper-format listings carry none).\n\n"
    )
    for path in sorted(SOURCES.glob("*.csv")):
        with path.open(newline="", encoding="utf-8", errors="replace") as f:
            rows = list(csv.reader(f))
        header = rows[0]
        buckets = [(i, c.strip()) for i, c in enumerate(header) if c.strip() and c.strip() != "canonical"]
        spec = {r["column"]: r["description"] for r in spec_tables.get(path.stem, [])}
        fields: dict[str, tuple[int | None, str]] = {}
        for row in rows[1:]:
            if not row or not row[0].strip():
                continue
            name = row[0].strip()
            col = None
            label = ""
            for bi, (hi, _) in enumerate(buckets):
                p = _pos(row[hi]) if hi < len(row) else None
                if p is None:
                    continue
                lab = row[hi + 1].strip() if hi + 1 < len(row) and not header[hi + 1].strip() else ""
                if bi == 0 and col is None:
                    col = p
                if not label and lab:
                    label = lab
            prev = fields.get(name)
            if prev is None:
                fields[name] = (col, label)
            else:
                fields[name] = (prev[0] if prev[0] is not None else col, prev[1] or label)
        out.write(f"### {path.stem}\n\n")
        out.write("| Column | Field | FEC label |\n|---:|---|---|\n")
        for name, (col, label) in fields.items():
            fec = spec.get(col, "") if col else ""
            fec = (fec or label).replace("|", "\\|").replace("\r", " ").strip()
            out.write(f"| {col or ''} | `{name}` | {fec} |\n")
        out.write("\n")
    return out.getvalue()


APPENDIX_MARKER = "## Appendix: every field, by table"


def write_appendix(target: Path) -> None:
    text = target.read_text() if target.exists() else ""
    head = text.split(APPENDIX_MARKER, 1)[0]
    target.write_text(head + appendix())
    print(f"wrote appendix to {target}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dry-run", action="store_true", help="report what would change; write nothing")
    ap.add_argument(
        "--appendix",
        metavar="MD",
        type=Path,
        help="replace everything from the appendix heading in MD with a regenerated field list",
    )
    args = ap.parse_args()
    if args.appendix:
        write_appendix(args.appendix)
        return 0
    return apply(args.dry_run)


if __name__ == "__main__":
    sys.exit(main())
