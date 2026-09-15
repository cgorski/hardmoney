# The browser UI

`hardmoney serve --ui` adds two pages to the REST API server, at `/ui`:

* a **filing workbench** for checking one `.fec` file: drop it or fetch
  it by filing id, read the header and cover page, see every validation
  finding and every cover-page line that does not reconcile with its
  schedules, edit fields, re-check, and download the corrected file;
* a **data browser** over the tables the server's namespace holds:
  candidates, committees, their ingested filings, Schedule A and B
  transactions, independent expenditures for or against a candidate, and
  the `/schema` load dashboard.

It is a QA tool for filers, filing vendors, and analysts, and a way to
look at loaded data without writing SQL. It is not a filing system: it
does not talk to the FEC's upload service, and the file it writes is the
same canonical output `hardmoney write` produces (see
[Writing `.fec` files](./writing-fec.md)).

The UI is embedded in the `hardmoney` binary. There is no separate build,
no Node toolchain, and nothing is loaded from a CDN, so it works on a
network that blocks third-party hosts.

## Starting it

```bash
hardmoney serve --ui --schema ui_smoke --bind 127.0.0.1:18099
```

```text
2026-09-15T12:15:54.947620Z  INFO hardmoney::api: hardmoney API listening bind=127.0.0.1:18099 cors="permissive" api_key=false ui=true
```

Open `http://127.0.0.1:18099/ui/`. Everything `serve` requires still
applies: a database URL, a namespace whose migrations are current, and
optionally `--api-key`, `--cors-origin`, and `--timeout-secs` (see
[Hardening the API](./api-hardening.md)). The workbench needs no data in
the database; the data browser shows whatever the namespace holds.

`--ui` also changes one default: the request body cap rises from 64 KB to
32 MB, because the workbench uploads whole filings. A filing larger than
that is refused with a 413 and a message pointing at the CLI, which
streams files of any size (`hardmoney parse`, `validate`, `reconcile`,
`write`).

## The workbench

Load a filing by dropping a `.fec` file on the page, choosing one, or
typing an FEC filing id (the number in an FEC.gov filing URL; the server
downloads it from `docquery.fec.gov`). The page then shows:

* a summary strip: form, spec version, amendment status, filer, body line
  count, and badges for validation errors, warnings, and cover-page lines
  that do not reconcile;
* the **header** and **cover page** as key/value cards. Blank cover fields
  are hidden by default; every value is an input, and changing one is an
  edit;
* a **records** table with a tab per table (`SchA 2,626`, `SchB 211`,
  and so on). Only the rows in view are in the page, so a filing with
  100,000 lines scrolls without stalling. Click a column header to sort,
  type in the filter box to keep matching rows, and use the column
  chooser to show or hide fields (by default, fields that are blank on
  every row are hidden). Hover or focus a column header for the FEC's
  specification of that field: description, type, maximum length,
  required level, and rule. Click a cell (or press Enter on it) to edit
  it; Escape cancels;
* a **validation** panel listing findings grouped by severity, each with
  its rule name and a link to the offending line;
* a **reconcile** panel for F3X, F3, and F3P: every cover-page line with
  its reported value, the value the schedules or the other cover lines
  imply, the difference, and the rule. Mismatching lines are highlighted;
  a toggle shows all lines or only the mismatches. Clicking a field name
  jumps to it on the cover card;
* an **edits** panel listing every changed field with its before and
  after value and a per-edit revert.

Each edit updates the document in the page and, after a short pause,
re-runs validation and reconciliation on the server. A toast reports what
changed ("errors 0 → 1", "cover lines off 1 → 0"). **Download .fec**
writes the edited document and saves it as `<form>-<committee>.fec`; the
toast tells you how many validation errors the written file still has.

Amounts are displayed from the exact decimal strings the parser produced
(`20,000.00`); the page never converts a dollar value to a floating-point
number. Edits are stored as the raw string you typed, so `19999.00` stays
`19999.00`.

Nothing is stored on the server between requests. Closing the tab
discards unsaved edits.

## The data browser

`/ui/data` searches candidates (`?q=`, cycle, state, office) and
committees (`?q=`, cycle, type) through the JSON API. A committee page
shows the committee's master record, its ingested filings (with the
amendment chain fields from [Amendments](./amendments.md)), its Schedule
A contributions and Schedule B disbursements with the same filters the
API accepts, and its independent expenditures. A candidate page shows the
candidate's record and the independent expenditures for and against them,
with support and oppose totals added exactly from the API's decimal
strings for the rows on the current page.

Every list is paginated with the API's own `limit` (25 to 500) and
`offset`, and "Next" is enabled while a page comes back full. An empty
result says which bulk file would fill it. `/ui/data/schema` shows the
namespace, migration state, and the latest load per source and cycle,
with a reminder that other namespaces in the same database are not
visible to this server.

## API key

When the server runs with `--api-key`, the shell and its assets still
load without the key (they are static files), but the first request the
page makes gets a 401, and the page asks for the key. It is kept in the
tab's `sessionStorage`, sent as `X-Api-Key` on every request, and gone
when the tab closes. The **API key** button in the top bar changes it.

## Themes and accessibility

The UI follows the system light/dark preference and has a toggle in the
top bar, remembered in `localStorage`. Colours meet WCAG AA contrast in
both themes. The records table is a keyboard grid with one tab stop:
arrow keys move between cells and column headers, Home/End and
PageUp/PageDown jump, Enter edits, Escape cancels, Enter on a header
sorts. Tabs, findings, and dialogs are reachable by keyboard, and status
changes are announced.

The UI was reviewed against WCAG 2.1 Level AA (the standard Section 508
incorporates) on 2026-09-15 with axe-core, computed contrast ratios for
both themes, and a scripted keyboard-only walkthrough. The method, the
conformance table, the defects fixed, and the known limitations (no
screen-reader or Firefox/Safari testing yet) are in
[Accessibility of the browser UI](./accessibility.md).

## The tools API

The workbench is built on six routes that need no database. They are on
whenever `--ui` is, and they take the API key like every other route. All
errors are `{"error": "..."}`.

| Route | Body | Response |
|---|---|---|
| `POST /tools/parse` | raw `.fec` bytes (`application/octet-stream` or `text/plain`) | the parsed document (below) |
| `GET /tools/fetch/{filing_id}` | none | the same, for a filing downloaded from the FEC (501 in a build without the `fetch` feature) |
| `POST /tools/validate` | a document (JSON) | a `Validation`: `{"findings": [{severity, rule, line_no, form_type, field, message}]}` |
| `POST /tools/reconcile` | a document (JSON) | a `Reconciliation`: `{"form", "checks": [{line, field, column, rule, reported, expected, delta, relation, lines_summed, reported_unparseable}]}`; 400 for a form without rules |
| `POST /tools/write` | a document (JSON) | the `.fec` bytes, `Content-Disposition: attachment; filename="<form>-<committee>.fec"`, `X-Hardmoney-Validation-Errors: N` |
| `GET /tools/spec/{table}?version=8.5` | none | the table's layout at that version with the FEC's field specs |

The parsed document `POST /tools/parse` returns:

```json
{
  "header": { "record_type": "HDR", "ef_type": "FEC", "fec_version_raw": "8.5", "version": "8.5",
              "soft_name": "FECfile", "soft_ver": "8.5.1.0(f34)", "name_delim": null,
              "report_id": "FEC-1991972", "report_number": "1", "comment": "" },
  "version": "8.5",
  "form_type": "F3XA",
  "base_form_type": "F3X",
  "is_amendment": true,
  "amends_filing": 1991972,
  "summary": { "raw_form_type": "F3XA", "table": "F3X", "line_no": 2, "fields": { "form_type": "F3XA", "filer_committee_id_number": "C00944124", "...": "..." } },
  "lines": [ { "raw_form_type": "SA11AI", "table": "SchA", "line_no": 3, "fields": { "...": "..." } } ],
  "skipped": [],
  "line_count": 6,
  "tables": { "SchA": 1, "SchB": 4, "SchE": 1 },
  "validation": { "findings": [] },
  "reconciliation": { "form": "F3X", "checks": [ "..." ] },
  "reconcile_error": null
}
```

Every field value is a string, and every field of the layout is present
(blank as `""`), in the FEC's column order. `reconciliation` is `null`
for a form without rules, with the reason in `reconcile_error`. Body
lines the lenient parser could not interpret are listed in `skipped` and
reported as `unrecognized_form_type` warnings.

The document `validate`, `reconcile`, and `write` accept is a subset of
that: `version`, `header` (any of its columns; `record_type`, `ef_type`,
and `fec_version_raw` default to `HDR`, `FEC`, and `version`), `summary`
(`table` and `fields`), and `lines` (each with `table` and `fields`;
`line_no` is optional and defaults to file order). Unknown keys are
ignored, so the output of `parse` can be edited and sent back as it is.
`summary.fields.form_type` must be the cover line's token as filed
(`F3XN`, not `F3X`), and every body line's `fields.form_type` must be its
token as filed too (`SA11AI`, `SB21B`; a schedule has many tokens and no
default one). A document that names a table or field the layout does not
have, whose `header.fec_version_raw` disagrees with `version`, or whose
record's `table` disagrees with its form-type token (`"table": "SchB"`
with `"form_type": "SA11AI"`) is a 400 whose message names the line and
says what disagrees.

From a shell:

```bash
curl -s -X POST --data-binary @tests/fixtures/F3XA_2011827.fec \
    -H 'Content-Type: application/octet-stream' \
    http://127.0.0.1:8080/tools/parse > doc.json

# Change a value, then write the result.
jq '.summary.fields.col_a_individuals_itemized = "19999.00"' doc.json > edited.json
curl -s -X POST --data-binary @edited.json -H 'Content-Type: application/json' \
    -D - -o F3XA-edited.fec http://127.0.0.1:8080/tools/write | grep -i -e content-disposition -e x-hardmoney
```

```text
content-disposition: attachment; filename="F3XA-C00944124.fec"
x-hardmoney-validation-errors: 0
```

The written file is what `Filing::to_fec` produces: canonical, with the
differences from the input described in [Writing `.fec` files](./writing-fec.md).

## Limits

* Uploads and fetched filings are capped at the server's body limit
  (32 MiB with `--ui`; `ApiConfig::max_body_bytes` in library code). The
  same cap applies to what a JSON document would *write*: `validate`,
  `reconcile`, and `write` sum the layout widths of the document's records
  and answer 413 if the written filing would exceed it, so a small JSON
  body cannot build a huge filing in server memory. The
  JSON the page receives is several times the size of the `.fec`, so a
  filing near the cap takes tens of seconds to load and hundreds of
  megabytes of browser memory. The CLI has no such ceiling.
* Each re-check after an edit sends the whole document back to the
  server. On a filing with tens of thousands of lines that is a few
  megabytes per edit.
* Reconciliation covers F3X, F3, and F3P, the same as `hardmoney
  reconcile`.
* The data browser totals independent expenditures over the rows on the
  current page (up to 500), not over the whole result.
* Uploads are `application/octet-stream` bodies; `multipart/form-data`
  is not accepted.
* The UI was exercised in current Chrome while this chapter was written.
  It uses ES modules, `<dialog>`, and `BigInt`, so browsers older than
  2021 will not run it.
