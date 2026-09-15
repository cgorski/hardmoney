// A virtualised, sortable, filterable, editable table of ParsedLines.
// Only the rows in the scroll window are in the DOM, so 100k rows render
// without freezing the page. Rows are the document's own objects: an edit
// writes through `onEdit`, which updates `row.fields` and the table then
// re-reads it.
//
// Keyboard: the table is one tab stop (a roving tabindex, WAI-ARIA grid
// pattern). Arrow keys move between cells and column headers, Home/End go
// to the first/last column, PageUp/PageDown move a screen of rows,
// Ctrl+Home/Ctrl+End go to the first/last row. Enter or F2 edits a cell,
// Enter commits, Escape cancels. Enter or Space on a header sorts by it.
// Because rows outside the window do not exist in the DOM, the cell that
// carries tabindex="0" is the active cell when it is rendered, otherwise
// the nearest rendered cell in the same column.

import { escapeHtml, formatMoney, compareDecimals, isAmountSpec, isNumericSpec, labelize, formatCount } from './format.js';

const ROW_H = 30;
const OVERSCAN = 8;
const HEADER_ROW = -1;

export class RecordsTable {
  /**
   * @param {HTMLElement} container
   * @param {object} opts
   * @param {(row, field, before, after) => void} opts.onEdit
   * @param {(field) => object|undefined} opts.getSpec  field name -> FieldInfo (with .spec)
   * @param {(row, field) => boolean} opts.isEdited
   * @param {(lineNo) => boolean} opts.lineHasError
   * @param {(html, anchor) => void} opts.showTooltip
   * @param {() => void} opts.hideTooltip
   * @param {() => void} opts.hideTooltipSoon  delayed hide, cancelled if the pointer reaches the tooltip
   */
  constructor(container, opts) {
    this.container = container;
    this.opts = opts;
    this.rows = [];
    this.view = [];
    this.fields = [];
    this.visible = new Set();
    this.sort = null; // { field, dir: 1 | -1 }
    this.filter = '';
    this.highlighted = null;
    this.editing = null;
    this.pendingRefresh = false;
    this.active = { row: HEADER_ROW, col: 0 }; // roving tabindex position
    this.build();
  }

  build() {
    this.container.innerHTML = `
      <div class="rt">
        <div class="rt-toolbar">
          <label class="field">
            <span class="muted">Filter</span>
            <input type="search" class="rt-filter" placeholder="Text in any shown column" autocomplete="off">
          </label>
          <details class="rt-cols">
            <summary>Columns</summary>
            <div class="rt-columns"></div>
          </details>
          <span class="rt-status" role="status" aria-live="polite"></span>
        </div>
        <div class="rt-scroll" tabindex="-1">
          <table role="grid" aria-label="Records" aria-rowcount="1">
            <thead><tr aria-rowindex="1"></tr></thead>
            <tbody></tbody>
          </table>
        </div>
      </div>`;
    this.el = {
      filter: this.container.querySelector('.rt-filter'),
      details: this.container.querySelector('.rt-cols'),
      cols: this.container.querySelector('.rt-columns'),
      status: this.container.querySelector('.rt-status'),
      scroll: this.container.querySelector('.rt-scroll'),
      table: this.container.querySelector('table'),
      thead: this.container.querySelector('thead tr'),
      tbody: this.container.querySelector('tbody'),
    };
    let t = null;
    this.el.filter.addEventListener('input', () => {
      clearTimeout(t);
      t = setTimeout(() => { this.filter = this.el.filter.value.trim().toLowerCase(); this.applyView(); }, 150);
    });
    // The column chooser is a popover: Escape closes it and returns focus,
    // and it closes when focus leaves it.
    this.el.details.addEventListener('keydown', (ev) => {
      if (ev.key === 'Escape' && this.el.details.open) {
        ev.preventDefault();
        ev.stopPropagation();
        this.el.details.open = false;
        this.el.details.querySelector('summary').focus();
      }
    });
    this.el.details.addEventListener('focusout', (ev) => {
      if (this.el.details.open && !this.el.details.contains(ev.relatedTarget)) this.el.details.open = false;
    });
    this.el.scroll.addEventListener('scroll', () => this.renderWindow());
    this.el.scroll.addEventListener('keydown', (ev) => this.onGridKey(ev));
    this.el.tbody.addEventListener('click', (ev) => {
      const td = ev.target.closest('td.cell');
      if (td && !this.editing) this.beginEdit(td);
    });
    this.el.tbody.addEventListener('focusin', (ev) => {
      const td = ev.target.closest('td.cell');
      if (td) this.setActive(this.indexOf(td), this.colOf(td));
    });
    this.el.thead.addEventListener('click', (ev) => {
      const th = ev.target.closest('th[data-field]');
      if (th) this.toggleSort(th.dataset.field);
    });
    for (const type of ['mouseover', 'focusin']) {
      this.el.thead.addEventListener(type, (ev) => {
        const th = ev.target.closest('th[data-field]');
        if (!th) return;
        if (type === 'focusin') this.setActive(HEADER_ROW, this.colOf(th));
        this.showSpec(th);
      });
    }
    this.el.thead.addEventListener('mouseleave', () => this.opts.hideTooltipSoon());
    this.el.thead.addEventListener('focusout', () => this.opts.hideTooltip());
  }

  /** Names the grid for assistive technology (the table tab it shows). */
  setLabel(text) {
    this.el.table.setAttribute('aria-label', text);
  }

  /** Replaces the rows shown. `fields` is the layout's field list (in order). */
  setRows(rows, fields) {
    this.rows = rows;
    this.fields = fields;
    this.sort = null;
    this.highlighted = null;
    this.active = { row: HEADER_ROW, col: 0 };
    // Default to the columns that carry a value somewhere, so a 45-column
    // schedule opens on the ones that matter; the chooser adds the rest.
    const used = new Set(['form_type']);
    for (const r of rows) {
      for (const f of fields) if (!used.has(f) && r.fields[f]) used.add(f);
      if (used.size === fields.length) break;
    }
    this.visible = new Set(fields.filter((f) => used.has(f)));
    this.renderColumnChooser();
    this.renderHeader();
    this.applyView();
  }

  renderColumnChooser() {
    const c = this.el.cols;
    c.innerHTML = `<div class="rt-columns-actions" role="group" aria-label="Show columns">
        <button type="button" class="btn btn-small" data-all>All</button>
        <button type="button" class="btn btn-small" data-none>None</button>
        <button type="button" class="btn btn-small" data-used>Non-blank</button>
      </div>`;
    for (const f of this.fields) {
      const label = document.createElement('label');
      const cb = document.createElement('input');
      cb.type = 'checkbox';
      cb.checked = this.visible.has(f);
      cb.dataset.field = f;
      cb.addEventListener('change', () => {
        if (cb.checked) this.visible.add(f); else this.visible.delete(f);
        this.renderHeader();
        this.renderWindow(true);
      });
      label.append(cb, ` ${f}`);
      c.appendChild(label);
    }
    const setAll = (pred) => {
      this.visible = new Set(this.fields.filter(pred));
      for (const cb of c.querySelectorAll('input[type="checkbox"]')) cb.checked = this.visible.has(cb.dataset.field);
      this.renderHeader();
      this.renderWindow(true);
    };
    c.querySelector('[data-all]').addEventListener('click', () => setAll(() => true));
    c.querySelector('[data-none]').addEventListener('click', () => setAll((f) => f === 'form_type'));
    c.querySelector('[data-used]').addEventListener('click', () => {
      const used = new Set(['form_type']);
      for (const r of this.rows) for (const f of this.fields) if (r.fields[f]) used.add(f);
      setAll((f) => used.has(f));
    });
  }

  shownFields() {
    return this.fields.filter((f) => this.visible.has(f));
  }

  renderHeader() {
    const shown = this.shownFields();
    this.active.col = Math.min(this.active.col, Math.max(0, shown.length - 1));
    const cells = ['<th class="col-line" scope="col">Line</th>'];
    shown.forEach((f, i) => {
      const sorted = this.sort?.field === f;
      const aria = sorted ? (this.sort.dir > 0 ? 'ascending' : 'descending') : 'none';
      const arrow = sorted ? (this.sort.dir > 0 ? '\u25b2' : '\u25bc') : '';
      const stop = this.active.row === HEADER_ROW && this.active.col === i ? 0 : -1;
      cells.push(`<th scope="col" data-field="${escapeHtml(f)}" tabindex="${stop}" aria-sort="${aria}">${escapeHtml(f)}<span class="sort" aria-hidden="true">${arrow}</span></th>`);
    });
    this.el.thead.innerHTML = cells.join('');
    this.ensureTabStop();
  }

  toggleSort(field) {
    if (this.sort?.field === field) {
      this.sort = this.sort.dir > 0 ? { field, dir: -1 } : null;
    } else {
      this.sort = { field, dir: 1 };
    }
    const hadFocus = this.el.thead.contains(document.activeElement);
    this.renderHeader();
    if (hadFocus) this.el.thead.querySelector(`th[data-field="${CSS.escape(field)}"]`)?.focus();
    const dir = this.sort ? (this.sort.dir > 0 ? 'ascending' : 'descending') : 'file order';
    this.applyView(`Sorted by ${field}, ${dir}. `);
  }

  /** Recomputes the filtered + sorted view and redraws. */
  applyView(prefix = '') {
    const shown = this.shownFields();
    let view = this.rows;
    if (this.filter) {
      const q = this.filter;
      view = view.filter((r) => {
        if (String(r.line_no).includes(q)) return true;
        for (const f of shown) {
          const v = r.fields[f];
          if (v && v.toLowerCase().includes(q)) return true;
        }
        return false;
      });
    }
    if (this.sort) {
      const { field, dir } = this.sort;
      const spec = this.opts.getSpec(field)?.spec;
      const numeric = isAmountSpec(spec) || isNumericSpec(spec);
      view = view.slice().sort((a, b) => {
        const va = a.fields[field] ?? '';
        const vb = b.fields[field] ?? '';
        if (va === vb) return a.line_no - b.line_no;
        if (va === '') return 1;
        if (vb === '') return -1;
        const c = numeric ? compareDecimals(va, vb) : va.localeCompare(vb, undefined, { numeric: true, sensitivity: 'base' });
        return c * dir || a.line_no - b.line_no;
      });
    }
    this.view = view;
    if (this.active.row >= view.length) this.active.row = Math.max(HEADER_ROW, view.length - 1);
    this.el.table.setAttribute('aria-rowcount', String(view.length + 1));
    this.el.status.textContent = prefix + (this.filter
      ? `${formatCount(view.length)} of ${formatCount(this.rows.length)} rows match`
      : `${formatCount(this.rows.length)} rows`);
    this.el.scroll.scrollTop = 0;
    this.renderWindow(true);
  }

  /** Draws the rows in the scroll window. */
  renderWindow(force = false) {
    // Never pull the DOM out from under an open editor: a scroll or an
    // app-requested refresh waits and is replayed when the edit finishes.
    if (this.editing) {
      this.pendingRefresh = true;
      return;
    }
    const total = this.view.length;
    const scrollTop = this.el.scroll.scrollTop;
    const height = this.el.scroll.clientHeight || 400;
    const start = Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN);
    const end = Math.min(total, Math.ceil((scrollTop + height) / ROW_H) + OVERSCAN);
    if (!force && this.window && this.window.start === start && this.window.end === end) return;
    this.window = { start, end };

    const hadFocus = this.el.tbody.contains(document.activeElement);
    const shown = this.shownFields();
    const tbody = this.el.tbody;
    tbody.innerHTML = '';
    if (total === 0) {
      const tr = document.createElement('tr');
      const td = document.createElement('td');
      td.className = 'rt-empty';
      td.colSpan = shown.length + 1;
      td.textContent = this.rows.length ? 'No rows match the filter.' : 'No rows.';
      tr.appendChild(td);
      tbody.appendChild(tr);
      this.ensureTabStop();
      if (hadFocus) this.el.thead.querySelector('[tabindex="0"]')?.focus({ preventScroll: true });
      return;
    }
    tbody.appendChild(this.spacer(start * ROW_H, shown.length + 1));
    for (let i = start; i < end; i++) tbody.appendChild(this.renderRow(this.view[i], i, shown));
    tbody.appendChild(this.spacer((total - end) * ROW_H, shown.length + 1));
    this.ensureTabStop();
    if (hadFocus) {
      // The focused cell was just re-created; put focus back on it, or on
      // the scroll region if it scrolled out of the window.
      const cell = this.cellAt(this.active.row, this.active.col);
      if (cell) cell.focus({ preventScroll: true });
      else this.el.scroll.focus({ preventScroll: true });
    }
  }

  spacer(height, span) {
    const tr = document.createElement('tr');
    tr.className = 'rt-spacer';
    tr.setAttribute('aria-hidden', 'true');
    const td = document.createElement('td');
    td.colSpan = span;
    td.style.height = `${height}px`;
    tr.appendChild(td);
    return tr;
  }

  renderRow(row, index, shown) {
    const tr = document.createElement('tr');
    tr.dataset.index = String(index);
    tr.dataset.line = String(row.line_no);
    tr.setAttribute('aria-rowindex', String(index + 2));
    if (this.highlighted === row.line_no) tr.classList.add('highlight');
    const hasError = this.opts.lineHasError(row.line_no);
    if (hasError) tr.classList.add('has-error');
    const line = document.createElement('th');
    line.scope = 'row';
    line.className = 'col-line';
    if (hasError) {
      // Bold red alone would be colour-only (WCAG 1.4.1): add a mark and text.
      const mark = document.createElement('span');
      mark.className = 'err-mark';
      mark.setAttribute('aria-hidden', 'true');
      mark.textContent = '!';
      line.append(mark, String(row.line_no), hiddenText(' has validation error'));
    } else {
      line.textContent = row.line_no;
    }
    tr.appendChild(line);
    shown.forEach((f, i) => {
      const td = document.createElement('td');
      td.dataset.field = f;
      td.tabIndex = index === this.active.row && i === this.active.col ? 0 : -1;
      td.className = 'cell';
      this.paintCell(td, row, f);
      tr.appendChild(td);
    });
    return tr;
  }

  paintCell(td, row, field) {
    const v = row.fields[field] ?? '';
    const spec = this.opts.getSpec(field)?.spec;
    const edited = this.opts.isEdited(row, field);
    td.classList.toggle('amount', isAmountSpec(spec));
    td.classList.toggle('edited', edited);
    td.textContent = isAmountSpec(spec) && v !== '' ? formatMoney(v) : v;
    if (edited) td.appendChild(hiddenText(' (edited)'));
    td.title = v.length > 24 ? v : '';
  }

  // ---- Roving tabindex -----------------------------------------------------

  indexOf(td) {
    return Number(td.closest('tr')?.dataset.index);
  }

  colOf(cell) {
    const tr = cell.closest('tr');
    return Array.from(tr.querySelectorAll(cell.tagName === 'TH' ? 'th[data-field]' : 'td.cell')).indexOf(cell);
  }

  cellAt(row, col) {
    if (row === HEADER_ROW) return this.el.thead.querySelectorAll('th[data-field]')[col] || null;
    const tr = this.el.tbody.querySelector(`tr[data-index="${row}"]`);
    return tr?.querySelectorAll('td.cell')[col] || null;
  }

  /** Makes (row, col) the one tab stop, adjusting rendered cells in place. */
  setActive(row, col) {
    if (Number.isNaN(row) || col < 0) return;
    this.active = { row, col };
    for (const el of this.el.table.querySelectorAll('[tabindex="0"]')) el.tabIndex = -1;
    const cell = this.cellAt(row, col);
    if (cell) cell.tabIndex = 0;
    else this.ensureTabStop();
  }

  /**
   * Guarantees the grid has exactly one tab stop even when the active row
   * is not rendered: the nearest rendered row's cell in the active column,
   * or the header.
   */
  ensureTabStop() {
    if (this.el.table.querySelector('[tabindex="0"]')) return;
    const rows = this.el.tbody.querySelectorAll('tr[data-index]');
    let entry = null;
    if (rows.length) {
      const first = Number(rows[0].dataset.index);
      const last = Number(rows[rows.length - 1].dataset.index);
      const tr = this.active.row < first ? rows[0] : this.active.row > last ? rows[rows.length - 1] : rows[0];
      entry = tr.querySelectorAll('td.cell')[Math.min(this.active.col, tr.querySelectorAll('td.cell').length - 1)];
    }
    if (!entry) entry = this.el.thead.querySelectorAll('th[data-field]')[Math.min(this.active.col, this.shownFields().length - 1)];
    if (entry) entry.tabIndex = 0;
  }

  // ---- Editing -------------------------------------------------------------

  rowFor(td) {
    return this.view[this.indexOf(td)];
  }

  beginEdit(td) {
    const row = this.rowFor(td);
    if (!row) return;
    const field = td.dataset.field;
    const before = row.fields[field] ?? '';
    const input = document.createElement('input');
    input.type = 'text';
    input.value = before;
    input.autocomplete = 'off';
    input.setAttribute('aria-label', `${field} on line ${row.line_no}`);
    td.textContent = '';
    td.appendChild(input);
    this.editing = { td, row, field, before, input };
    input.focus();
    input.select();
    let finished = false;
    const finish = (commit) => {
      if (finished) return;
      finished = true;
      const after = input.value.trim();
      this.editing = null;
      if (!td.isConnected) return;
      if (commit && after !== before) this.opts.onEdit(row, field, before, after);
      this.paintCell(td, row, field);
      td.focus({ preventScroll: true });
      if (this.pendingRefresh) {
        this.pendingRefresh = false;
        this.renderWindow(true);
      }
    };
    input.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter') { ev.preventDefault(); finish(true); }
      else if (ev.key === 'Escape') { ev.preventDefault(); finish(false); }
      else if (ev.key === 'Tab') { finish(true); }
      ev.stopPropagation();
    });
    input.addEventListener('blur', () => finish(true));
  }

  cancelEdit() {
    const e = this.editing;
    this.editing = null;
    if (e && e.td.isConnected) this.paintCell(e.td, e.row, e.field);
  }

  // ---- Keyboard ------------------------------------------------------------

  onGridKey(ev) {
    if (this.editing) return;
    const th = ev.target.closest('th[data-field]');
    const td = ev.target.closest('td.cell');
    const shown = this.shownFields();
    if (!shown.length) return;
    const lastRow = this.view.length - 1;
    const pageRows = Math.max(1, Math.floor((this.el.scroll.clientHeight || 400) / ROW_H) - 1);

    if (th) {
      const col = this.colOf(th);
      if (ev.key === 'Enter' || ev.key === ' ') { ev.preventDefault(); this.toggleSort(th.dataset.field); return; }
      if (ev.key === 'Escape') { this.opts.hideTooltip(); return; }
      let target = null;
      if (ev.key === 'ArrowRight') target = [HEADER_ROW, col + 1];
      else if (ev.key === 'ArrowLeft') target = [HEADER_ROW, col - 1];
      else if (ev.key === 'Home') target = [HEADER_ROW, 0];
      else if (ev.key === 'End') target = [HEADER_ROW, shown.length - 1];
      else if (ev.key === 'ArrowDown' || ev.key === 'PageDown') target = [0, col];
      if (!target) return;
      ev.preventDefault();
      this.focusCell(target[0], target[1]);
      return;
    }

    if (!td) {
      // Focus is on the scroll region itself (the active row scrolled out
      // of the DOM); any arrow key re-enters the grid.
      if (ev.key.startsWith('Arrow') || ev.key === 'Home' || ev.key === 'End') {
        ev.preventDefault();
        const entry = this.el.table.querySelector('[tabindex="0"]');
        entry?.focus({ preventScroll: true });
      }
      return;
    }

    if (ev.key === 'Enter' || ev.key === 'F2') {
      ev.preventDefault();
      this.beginEdit(td);
      return;
    }
    const index = this.indexOf(td);
    const col = this.colOf(td);
    let target = null;
    switch (ev.key) {
      case 'ArrowLeft': target = [index, col - 1]; break;
      case 'ArrowRight': target = [index, col + 1]; break;
      case 'ArrowUp': target = [index === 0 ? HEADER_ROW : index - 1, col]; break;
      case 'ArrowDown': target = [index + 1, col]; break;
      case 'Home': target = ev.ctrlKey ? [0, 0] : [index, 0]; break;
      case 'End': target = ev.ctrlKey ? [lastRow, shown.length - 1] : [index, shown.length - 1]; break;
      case 'PageUp': target = [Math.max(0, index - pageRows), col]; break;
      case 'PageDown': target = [Math.min(lastRow, index + pageRows), col]; break;
      default: return;
    }
    ev.preventDefault();
    this.focusCell(target[0], target[1]);
  }

  /** Moves the roving focus to (row, col), scrolling the row into the window first. */
  focusCell(row, col) {
    const shown = this.shownFields();
    if (col < 0 || col >= shown.length) return;
    if (row !== HEADER_ROW && (row < 0 || row >= this.view.length)) return;
    if (row !== HEADER_ROW) {
      const top = row * ROW_H;
      const s = this.el.scroll;
      // Keep the row clear of the sticky header.
      if (top < s.scrollTop + ROW_H) s.scrollTop = Math.max(0, top - ROW_H);
      else if (top + ROW_H > s.scrollTop + s.clientHeight) s.scrollTop = top + ROW_H - s.clientHeight;
      this.renderWindow();
    }
    this.setActive(row, col);
    this.cellAt(row, col)?.focus({ preventScroll: true });
  }

  /** Scrolls to and highlights a line; clears the filter if it hides the line. */
  scrollToLine(lineNo) {
    let index = this.view.findIndex((r) => r.line_no === lineNo);
    if (index < 0 && this.filter) {
      this.filter = '';
      this.el.filter.value = '';
      this.applyView();
      index = this.view.findIndex((r) => r.line_no === lineNo);
    }
    if (index < 0) return false;
    this.highlighted = lineNo;
    const s = this.el.scroll;
    s.scrollTop = Math.max(0, index * ROW_H - s.clientHeight / 2);
    this.renderWindow(true);
    this.focusCell(index, 0);
    return true;
  }

  /** Redraws the window (after edits or spec load). */
  refresh() {
    this.renderWindow(true);
  }

  showSpec(th) {
    const f = th.dataset.field;
    const info = this.opts.getSpec(f);
    const spec = info?.spec;
    let html = `<div class="tt-name">${escapeHtml(f)}</div>`;
    if (info) html += `<div class="muted">column ${info.column + 1}</div>`;
    if (spec) {
      html += `<div class="tt-desc">${escapeHtml(spec.description || labelize(f))}</div><dl>`;
      html += `<dt>Type</dt><dd>${escapeHtml(spec.kind)}${spec.max_len ? `, max ${spec.max_len}` : ''}</dd>`;
      const req = spec.required?.level ?? 'none';
      html += `<dt>Required</dt><dd>${escapeHtml(req)}${spec.required?.condition ? `: ${escapeHtml(spec.required.condition)}` : ''}</dd>`;
      if (spec.rule) html += `<dt>Rule</dt><dd>${escapeHtml(spec.rule)}</dd>`;
      if (spec.value_reference) html += `<dt>Values</dt><dd>${escapeHtml(spec.value_reference)}</dd>`;
      if (spec.sample) html += `<dt>Sample</dt><dd><code>${escapeHtml(spec.sample)}</code></dd>`;
      html += '</dl>';
    } else {
      html += '<div class="muted">Not in the bundled FEC spec for this table.</div>';
    }
    this.opts.showTooltip(html, th);
  }
}

function hiddenText(text) {
  const span = document.createElement('span');
  span.className = 'visually-hidden';
  span.textContent = text;
  return span;
}
