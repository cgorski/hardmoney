# Accessibility of the browser UI

A federal agency cannot deploy a web tool for staff or filers unless it
conforms to Section 508, which incorporates WCAG 2.1 Level AA. This
chapter records how `hardmoney serve --ui` was reviewed against those
criteria, what was found and fixed, what the automated scan and the
keyboard walkthrough showed afterwards, and what is not yet verified.

The wording here is deliberate. "Reviewed against" means a criterion was
checked by reading the code and by the automated and keyboard tests
described below. The UI has not yet been tested with a screen reader by a
screen-reader user, nor in Firefox or Safari, so this chapter does not
claim conformance; it claims the review described and shows its results.

## Scope

Everything the binary serves under `/ui`: the shell (`index.html`), the
stylesheet, and the ES modules for the filing workbench and the data
browser (`src/ui/assets/`). Not in scope: the JSON API (machine
interface), the CLI, and the book itself (an mdBook site with its own
theme).

Review date: 2026-09-15, against the assets at commit state of hardmoney
3.0.1 plus the fixes listed below. Standards: WCAG 2.1 Level A and AA
success criteria that apply to a single-page application; the Revised 508
Standards (36 CFR 1194) point at the same criteria for web content.

## Method

1. **Static review** of each asset against every applicable success
   criterion (the table below), reading the HTML the scripts emit as well
   as the shell.
2. **Contrast computation.** `tmp/agent-508/contrast.py` parses the CSS
   custom properties out of `app.css` for both themes and applies the
   WCAG relative-luminance formula to every foreground/background pair
   the stylesheet actually uses: 22 text pairs at the 4.5:1 threshold and
   13 non-text pairs (control borders, the focus ring against each surface
   it can sit on, the state marks, toast edges) at 3:1, plus the
   decorative card border listed for information. 36 pairs per theme, 72
   in all.
3. **Automated scan.** axe-core 4.10.2 (`axe.min.js` from cdnjs, saved
   locally and injected over the DevTools protocol, never served by the
   app) run by headless Google Chrome 153 against the live server with
   `tests/fixtures/F3XA_2011827.fec` loaded. Rule set: `wcag2a`,
   `wcag2aa`, `wcag21a`, `wcag21aa`, `best-practice`. Seventeen scenes:
   empty workbench, loaded workbench (records, validation, reconcile,
   edits tabs), the state after an edit, the API-key dialog, the data
   browser search, schema, and committee pages, each in the light theme
   and the state-bearing ones in the dark theme too.
4. **Keyboard walkthrough**, scripted through the same DevTools session
   with real key events: tab order from the top of the page, choosing a
   file, switching the main tabs and the table tabs, entering the records
   grid, moving with arrow, Home, End, PageUp/PageDown, sorting from a
   header, editing a cell and committing, surviving the automatic re-check
   that follows an edit, the header tooltip (appear on focus, dismiss with
   Escape), the API-key dialog (open, Tab cycle, Escape, focus return),
   downloading, closing the filing, the committee tabs, and reflow at a
   320 CSS-pixel viewport.
5. **Regression tests** in `tests/ui_routes.rs` (string-level) that fail
   if the shell loses its `lang`, title, skip link, single `<main>`,
   labelled controls, or live regions; if any asset introduces a positive
   `tabindex`, an unnamed image, or `outline: none`; or if the grid, tabs,
   tooltip, or theme toggle lose the attributes the review verified.

The driver scripts and both result files are under `tmp/agent-508/`
(`audit.mjs`, `results-before.json`, `results-after.json`,
`contrast-after.txt`). They need Chrome and Node but no npm packages.

## What was found and fixed

Before the fixes, axe reported one violation across the seventeen scenes
(`empty-table-header`, the unnamed "Revert" column of the edits table) and
six colour-contrast items it could not decide. The keyboard walkthrough,
and the static review behind it, found the defects that mattered:

| Defect | Criteria | Fix |
|---|---|---|
| The file input was `display: none`, so "Choose a file" was not in the tab order; a keyboard user could only load a filing by typing an id | 2.1.1 | The input is visually hidden but focusable; its label is the visible button and shows the focus ring |
| Focus fell to `<body>` after loading a filing, closing it, or navigating between workbench and data browser | 2.4.3, 3.2.x | Focus moves to the view's `<h1>` after each change of view |
| Every visible grid cell and every column header was a tab stop (46 on a small filing) | 2.1.1, 2.4.3 | Roving `tabindex`: the grid is one tab stop; arrows, Home/End, PageUp/PageDown, Ctrl+Home/End move; Enter or F2 edits; Enter/Space on a header sorts |
| The automatic re-check 600 ms after an edit redrew the grid and dropped focus; a redraw while typing discarded the typed text | 2.4.3, 3.2.2 | Redraws restore focus to the active cell and are deferred while a cell editor is open |
| The column-header tooltip had no `aria-describedby`, could not be dismissed with Escape, and hid when the pointer moved onto it | 1.4.13, 4.1.2 | Linked with `aria-describedby`, Escape hides it, hover on the tooltip keeps it |
| The theme toggle changed its label with its state ("Light theme" while `aria-pressed="true"`) | 4.1.2 | One label, "Dark theme", with `aria-pressed` and a pressed style |
| `document.title` was always "hardmoney" | 2.4.2 | Per view: "Filing workbench", "F3XA C00944124 · Filing workbench", "Committee C00944124", and so on |
| Reconcile mismatches were shown by red text on a red background only; edited cells by a blue background only; error lines by a bold red line number only | 1.4.1 | A "Result" column with "off" or "agrees" text; an accent bar plus hidden "(edited)" text; a "!" mark plus hidden "has validation error" text; the selected tab and the pressed toggle have a mark too |
| Control borders were 1.58:1 (light) and 1.75:1 (dark) against their background | 1.4.11 | A `--control-border` token at 4.5:1 and 4.9:1 for inputs, buttons, the dialog, and popovers; `--border` stays for decorative rules |
| Toasts were live regions inserted inside another live region, a pattern that some screen readers announce twice or not at all | 4.1.3 | Two persistent regions in the shell (`role="status"`, `role="alert"`) receive each message; the visible toasts carry no live semantics, and pause their auto-dismiss while hovered or focused |
| A rejected filing id was only reported as a toast | 3.3.1, 3.3.3 | The field gets `aria-invalid` and `aria-describedby` pointing at the status line that holds the message, and focus returns to it |
| At 320 px the cover card overflowed by 91 px (long field names with no break points) and the data browser by 35 px | 1.4.10 | `overflow-wrap: anywhere`, `minmax(0, 1fr)` columns, and `min()` widths for the popovers, dialog, and toasts |
| Tab strips: only the main tabs had arrow keys; the table and committee tabs were separate tab stops with no `aria-controls` or `tabpanel` | 4.1.2, 3.2.4 | One shared `tabs.js` for all three strips |
| Plain tables in a scrolling box with no focusable content could not be scrolled by keyboard | 2.1.1 | The scroll region is focusable and named |
| Small items: the empty `<th>`, `≥` and `←` glyphs, an em dash for blank values, a `title` duplicating the header text, links that open fec.gov in a new tab without saying so | 1.1.1, 1.3.1, 2.4.4 | Text headers, "at least"/"equals", "blank", hidden "(opens on fec.gov in a new tab)" |

After the fixes, on 2026-09-15:

* axe-core: **0 violations** in all 17 scenes (697 passing rule
  instances). Seven items remain in axe's "needs review" list, all
  `color-contrast` on elements axe could not compute a background for (a
  toast over the page, text in the modal dialog over its backdrop, the
  `aria-hidden` sort arrow and back arrow). Each was computed by hand from
  the palette: the lowest is the sort arrow at 5.62:1 (light) and 6.13:1
  (dark).
* Contrast script: **72 of 72 pairs pass** (both themes).
* Keyboard walkthrough: **37 checks, 0 failures**. The grid exposes
  exactly one tab stop; Tab leaves it in one press and Shift+Tab returns.
  Reaching the main tabs from the top of the page takes 126 presses on an
  F3X because the cover card has one input per field; see the known
  limitations.
* Reflow at 320 px: 0 px horizontal overflow on the empty workbench, the
  loaded workbench, and the data browser.
* Console: no errors during either run.

## Conformance table

Status values: **Supports** (reviewed and verified by the methods above),
**Partially supports** (verified with a stated exception), **Not
evaluated** (no verification yet). "Not applicable" criteria (audio,
video, time-based media, CAPTCHA, three-flashes) are omitted.

| Criterion | Level | Status | Note |
|---|---|---|---|
| 1.1.1 Non-text content | A | Supports | No images in the UI; the favicon is not in the document. Glyphs (sort arrows, the dismiss "×", the back arrow) are `aria-hidden` and carry text or an `aria-label`. Enforced by `images_and_svgs_are_named_or_hidden`. |
| 1.3.1 Info and relationships | A | Supports | Landmarks `header`, `nav[aria-label]`, `main`, a named `section` for notifications. Real tables with `th scope="col"`; the grid has row headers (`th scope="row"`) for line numbers. `dl` for key/value cards. Every input has a `<label>` or an `aria-label`; tabs use `tablist`/`tab`/`tabpanel` with `aria-controls`. Headings run h1, h2, h3 in order. |
| 1.3.2 Meaningful sequence | A | Supports | DOM order matches visual order; layout uses flex/grid without `order`. |
| 1.3.3 Sensory characteristics | A | Supports | Instructions refer to names ("Activate a line number"), not to position or colour. |
| 1.3.4 Orientation | AA | Supports | No orientation lock. |
| 1.3.5 Identify input purpose | AA | Supports | The only personal-data fields are search filters, which have no autocomplete token in the spec; `autocomplete="off"` on the key field and grid editors is intentional. |
| 1.4.1 Use of colour | A | Supports | See the fixes table: mismatch rows, edited cells, error lines, selected tabs, and the pressed toggle all carry a non-colour cue. Links in body text are underlined. |
| 1.4.3 Contrast (minimum) | AA | Supports | 22 text pairs per theme computed; lowest 5.26:1 (error text on the highlighted row, dark). |
| 1.4.4 Resize text | AA | Supports | All sizes are in rem/em from a 15 px root; the layout reflows at 200 % zoom (see 1.4.10). |
| 1.4.5 Images of text | AA | Supports | None. |
| 1.4.10 Reflow | AA | Supports | 0 px overflow at 320 px on the three views tested. The records grid and plain tables scroll horizontally inside their own region, which the criterion allows for data tables. |
| 1.4.11 Non-text contrast | AA | Supports | Control borders 4.54:1 / 4.88:1; focus ring at least 3.98:1 against every surface it is drawn on; state marks use the accent at 6.25:1 or better. Disabled controls are exempt. |
| 1.4.12 Text spacing | AA | Partially supports | Cards, forms, and plain tables reflow. Grid cells are a fixed 30 px high and 180 px wide so 100,000 rows can be virtualised; with wider letter spacing more long values truncate with an ellipsis. The full value is one keypress away (Enter opens it in an editor; `title` shows it on hover). |
| 1.4.13 Content on hover or focus | AA | Supports | The field tooltip is dismissible (Escape), hoverable, and persistent until the pointer or focus leaves. |
| 2.1.1 Keyboard | A | Supports | Every action in the walkthrough was completed with keys alone: load, tabs, grid navigation, edit, sort, revert, validate/reconcile (automatic), API key, download, close. Drag-and-drop has the file chooser and the fetch form as alternatives. |
| 2.1.2 No keyboard trap | A | Supports | The dialog is a native `<dialog>` with `showModal()`: Tab cycles through the dialog and the browser UI and never reaches the inert page; Escape closes it. The grid is left with one Tab. |
| 2.1.4 Character key shortcuts | A | Supports | No single-character shortcuts. |
| 2.2.1 Timing adjustable | A | Partially supports | Information and success toasts disappear after 6 s. The same text is announced through a live region when it appears, stays while hovered or focused, and the state it reports (counts, badges) remains on the page. Error toasts stay until dismissed. |
| 2.2.2 Pause, stop, hide | A | Supports | The only animation is a 150 ms toast entrance, disabled under `prefers-reduced-motion`. |
| 2.4.1 Bypass blocks | A | Supports | Skip link to `main`, which is focusable; landmarks throughout. |
| 2.4.2 Page titled | A | Supports | Title changes per view and names the loaded filing. |
| 2.4.3 Focus order | A | Supports | No positive `tabindex` (tested). Focus moves to the new heading on a change of view and is restored to the active cell after grid redraws. |
| 2.4.4 Link purpose (in context) | A | Supports | Link text is the id or name it leads to, in a labelled column; external links say they open on fec.gov in a new tab. |
| 2.4.5 Multiple ways | AA | Supports | Two views, both in the top navigation from every page. |
| 2.4.6 Headings and labels | AA | Supports | Field labels are the canonical FEC field names; the tooltip gives the FEC description. |
| 2.4.7 Focus visible | AA | Supports | A 3 px `:focus-visible` outline everywhere; no `outline: none` (tested). Inset inside grid cells so the sticky header cannot cover it. |
| 2.5.1 Pointer gestures | A | Supports | No multipoint or path gestures. |
| 2.5.2 Pointer cancellation | A | Supports | Actions fire on click, not on pointer-down. |
| 2.5.3 Label in name | A | Supports | Visible labels are contained in accessible names ("Filter", "form_type, line 2, edited"). |
| 2.5.4 Motion actuation | A | Supports | None. |
| 3.1.1 Language of page | A | Supports | `<html lang="en">` (tested). |
| 3.1.2 Language of parts | AA | Supports | Filing data is displayed as filed; the UI has no marked foreign-language passages. |
| 3.2.1 On focus | A | Supports | Focusing a header shows a tooltip; nothing changes context on focus. |
| 3.2.2 On input | A | Supports | Filters apply on submit; the page-size select does not submit; checkboxes redraw the same panel. Edits re-run checks on the server and report the result without moving focus. |
| 3.2.3 Consistent navigation | AA | Supports | One top bar on every view. |
| 3.2.4 Consistent identification | AA | Supports | One `tabs.js` for every tab strip; one `.btn` style; one toast and one dialog pattern. |
| 3.3.1 Error identification | A | Supports | A rejected filing id or a failed fetch marks the field `aria-invalid` and puts the message in the status line the field is described by. Server errors are announced as alerts with the server's message. |
| 3.3.2 Labels or instructions | A | Supports | Every input has a label; the fetch form has help text via `aria-describedby`. |
| 3.3.3 Error suggestion | AA | Supports | The id message says what is expected ("Enter digits only, like 2011827"); validation findings carry the FEC rule and a "line N" link to the field. |
| 3.3.4 Error prevention (legal, financial, data) | AA | Supports | Nothing is submitted to the FEC. Edits are reversible per field and in bulk, listed in the edits tab, and the download toast states how many validation errors the written file still has. |
| 4.1.1 Parsing | A | Supports | Unique ids (`duplicate-id` rules pass); well-formed templates. |
| 4.1.2 Name, role, value | A | Supports | Grid with `aria-rowcount`/`aria-rowindex` and `aria-sort`; tabs with `aria-selected`/`aria-controls`; toggle with `aria-pressed`; dialog with `aria-labelledby`, `aria-describedby`, `aria-modal`; column chooser is a native `details`/`summary` that closes on Escape and on focus leaving. |
| 4.1.3 Status messages | AA | Supports | Persistent `role="status"` and `role="alert"` regions; row counts, page status, and the load status are `role="status"`; "validated: 0 errors" style results are announced without moving focus. |

## Known limitations and exceptions

* **Screen readers and other browsers are not yet verified.** The scan
  and the walkthrough ran in Chrome only. VoiceOver, NVDA, JAWS, Firefox,
  and Safari have not been used. The FECfile+ team tests with a screen
  reader as part of their quarterly audits; the same should be done here
  before any deployment claim, and the results appended to this chapter.
* **Tab distance to the details tabs (2.4.3, advisory).** The cover card
  renders one input per cover-page field, about 120 on an F3X, all
  before the Records/Validation/Reconcile/Edits tabs. Every stop is
  meaningful and the order is logical, so this is not a failure, but a
  keyboard user who wants the tabs has a long way to go. A second skip
  link or a collapsed cover card would help.
* **Grid text spacing (1.4.12).** Fixed-height rows are what makes a
  100,000-line filing usable at all; the trade-off is described in the
  table.
* **Toast timing (2.2.1).** See the table; a setting to keep toasts until
  dismissed would remove the exception.
* **Column headers are focusable cells, not buttons.** Sorting follows the
  WAI-ARIA grid pattern (Enter/Space on the header, `aria-sort` reports
  the state). A screen reader announces "column header, sorted ascending"
  rather than "button"; the discoverability of Enter-to-sort relies on
  the grid role.
* **`aria-modal` on a native `<dialog>`** is redundant in current browsers
  and harmless in older ones; it is there because auditors look for it.

## Reporting an accessibility problem

Open an issue at <https://github.com/cgorski/hardmoney/issues> with the
label `accessibility`, or use the security address in `SECURITY.md` if
the report should not be public. Say which page (`/ui/` or `/ui/data/...`),
which browser and assistive technology, what you did, what happened, and
what you expected. A reproduction with `tests/fixtures/F3XA_2011827.fec`
is ideal because it is what the automated checks use. Reports are triaged
like other defects; fixes land with a regression test in
`tests/ui_routes.rs` where a string-level check can express them, and
with a re-run of `tmp/agent-508/audit.mjs` otherwise.

## Summary in the style of an accessibility conformance report

* **Product:** hardmoney browser UI, `hardmoney serve --ui`, version
  3.0.1 plus the changes in the Unreleased section of `CHANGELOG.md`.
* **Standard:** WCAG 2.1, Levels A and AA (Revised Section 508, 36 CFR
  1194, E205.4 and Chapter 5 by reference).
* **Evaluation methods:** static code review; computed contrast ratios for
  every colour pair in both themes; axe-core 4.10.2 in headless Chrome 153
  over seventeen states of the application; a scripted keyboard-only
  walkthrough of every user action; regression tests.
* **Date:** 2026-09-15.
* **Result:** 43 applicable success criteria reviewed. 41 **Supports**,
  2 **Partially supports** (1.4.12 grid text spacing, 2.2.1 toast
  timing). 0 **Does not support**. The tab-distance note under 2.4.3 is
  advisory, not a failure. 0 axe-core violations after fixes (1 before).
  72 of 72 contrast pairs pass. 37 of 37 keyboard checks pass.
* **Not evaluated:** assistive-technology testing with a screen reader;
  Firefox and Safari; Windows High Contrast beyond the `forced-colors`
  stylesheet fallback.
