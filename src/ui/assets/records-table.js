// A virtualised, sortable, filterable, editable table of ParsedLines.
// Only the rows in the scroll window are in the DOM, so 100k rows render
// without freezing the page. Rows are the document's own objects: an edit
// writes through `onEdit`, which updates `row.fields` and the table then
// re-reads it.

import { escapeHtml, formatMoney, compareDecimals, isAmountSpec, isNumericSpec, labelize, formatCount } from './format.js';

const ROW_H = 30;
const OVERSCAN = 8;

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
    this.build();
  }

  build() {
    this.container.innerHTML = `
      <div class="rt">
        <div class="rt-toolbar">
          <label class="field">
            <span class="muted">Filter</span>
            <input type="search" class="rt-filter" placeholder="Text in any shown column" aria-label="Filter rows">
          </label>
          <details class="rt-cols">
            <summary>Columns</summary>
            <div class="rt-columns"></div>
          </details>
          <span class="rt-status" aria-live="polite"></span>
        </div>
        <div class="rt-scroll" tabindex="-1">
          <table role="grid">
            <thead><tr></tr></thead>
            <tbody></tbody>
          </table>
        </div>
      </div>`;
    this.el = {
      filter: this.container.querySelector('.rt-filter'),
      cols: this.container.querySelector('.rt-columns'),
      status: this.container.querySelector('.rt-status'),
      scroll: this.container.querySelector('.rt-scroll'),
      thead: this.container.querySelector('thead tr'),
      tbody: this.container.querySelector('tbody'),
    };
    let t = null;
    this.el.filter.addEventListener('input', () => {
      clearTimeout(t);
      t = setTimeout(() => { this.filter = this.el.filter.value.trim().toLowerCase(); this.applyView(); }, 150);
    });
    this.el.scroll.addEventListener('scroll', () => this.renderWindow());
    this.el.tbody.addEventListener('click', (ev) => {
      const td = ev.target.closest('td.editable');
      if (td && !this.editing) this.beginEdit(td);
    });
    this.el.tbody.addEventListener('keydown', (ev) => this.onGridKey(ev));
    this.el.thead.addEventListener('click', (ev) => {
      const th = ev.target.closest('th[data-field]');
      if (th) this.toggleSort(th.dataset.field);
    });
    this.el.thead.addEventListener('keydown', (ev) => {
      const th = ev.target.closest('th[data-field]');
      if (th && (ev.key === 'Enter' || ev.key === ' ')) { ev.preventDefault(); this.toggleSort(th.dataset.field); }
    });
    for (const type of ['mouseover', 'focusin']) {
      this.el.thead.addEventListener(type, (ev) => {
        const th = ev.target.closest('th[data-field]');
        if (th) this.showSpec(th);
      });
    }
    for (const type of ['mouseleave', 'focusout']) {
      this.el.thead.addEventListener(type, () => this.opts.hideTooltip());
    }
  }

  /** Replaces the rows shown. `fields` is the layout's field list (in order). */
  setRows(rows, fields) {
    this.rows = rows;
    this.fields = fields;
    this.sort = null;
    this.highlighted = null;
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
    c.innerHTML = `<div class="rt-columns-actions">
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
    const cells = ['<th class="col-line" scope="col">Line</th>'];
    for (const f of this.shownFields()) {
      const sorted = this.sort?.field === f;
      const aria = sorted ? (this.sort.dir > 0 ? 'ascending' : 'descending') : 'none';
      const arrow = sorted ? (this.sort.dir > 0 ? '▲' : '▼') : '';
      cells.push(`<th scope="col" data-field="${escapeHtml(f)}" tabindex="0" aria-sort="${aria}" title="${escapeHtml(f)}">${escapeHtml(f)}<span class="sort" aria-hidden="true">${arrow}</span></th>`);
    }
    this.el.thead.innerHTML = cells.join('');
  }

  toggleSort(field) {
    if (this.sort?.field === field) {
      this.sort = this.sort.dir > 0 ? { field, dir: -1 } : null;
    } else {
      this.sort = { field, dir: 1 };
    }
    this.renderHeader();
    this.applyView();
  }

  /** Recomputes the filtered + sorted view and redraws. */
  applyView() {
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
    this.el.status.textContent = this.filter
      ? `${formatCount(view.length)} of ${formatCount(this.rows.length)} rows match`
      : `${formatCount(this.rows.length)} rows`;
    this.el.scroll.scrollTop = 0;
    this.renderWindow(true);
  }

  /** Draws the rows in the scroll window. */
  renderWindow(force = false) {
    const total = this.view.length;
    const scrollTop = this.el.scroll.scrollTop;
    const height = this.el.scroll.clientHeight || 400;
    const start = Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN);
    const end = Math.min(total, Math.ceil((scrollTop + height) / ROW_H) + OVERSCAN);
    if (!force && this.window && this.window.start === start && this.window.end === end) return;
    this.window = { start, end };
    if (this.editing) this.cancelEdit();

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
      return;
    }
    tbody.appendChild(this.spacer(start * ROW_H, shown.length + 1));
    for (let i = start; i < end; i++) tbody.appendChild(this.renderRow(this.view[i], i, shown));
    tbody.appendChild(this.spacer((total - end) * ROW_H, shown.length + 1));
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
    if (this.highlighted === row.line_no) tr.classList.add('highlight');
    if (this.opts.lineHasError(row.line_no)) tr.classList.add('has-error');
    const line = document.createElement('td');
    line.className = 'col-line';
    line.textContent = row.line_no;
    tr.appendChild(line);
    for (const f of shown) {
      const td = document.createElement('td');
      td.dataset.field = f;
      td.tabIndex = 0;
      td.className = 'editable';
      this.paintCell(td, row, f);
      tr.appendChild(td);
    }
    return tr;
  }

  paintCell(td, row, field) {
    const v = row.fields[field] ?? '';
    const spec = this.opts.getSpec(field)?.spec;
    td.classList.toggle('amount', isAmountSpec(spec));
    td.classList.toggle('edited', this.opts.isEdited(row, field));
    td.textContent = isAmountSpec(spec) && v !== '' ? formatMoney(v) : v;
    td.title = v.length > 24 ? v : '';
  }

  rowFor(td) {
    const tr = td.closest('tr');
    return this.view[Number(tr?.dataset.index)];
  }

  beginEdit(td) {
    const row = this.rowFor(td);
    if (!row) return;
    const field = td.dataset.field;
    const before = row.fields[field] ?? '';
    const input = document.createElement('input');
    input.type = 'text';
    input.value = before;
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
      td.focus();
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

  onGridKey(ev) {
    if (this.editing) return;
    const td = ev.target.closest('td.editable');
    if (!td) return;
    if (ev.key === 'Enter' || ev.key === 'F2') {
      ev.preventDefault();
      this.beginEdit(td);
      return;
    }
    const moves = { ArrowLeft: [0, -1], ArrowRight: [0, 1], ArrowUp: [-1, 0], ArrowDown: [1, 0] };
    const m = moves[ev.key];
    if (!m) return;
    ev.preventDefault();
    const tr = td.closest('tr');
    const cells = Array.from(tr.querySelectorAll('td.editable'));
    const col = cells.indexOf(td);
    if (m[1] !== 0) {
      const next = cells[col + m[1]];
      if (next) next.focus();
      return;
    }
    const index = Number(tr.dataset.index) + m[0];
    if (index < 0 || index >= this.view.length) return;
    this.focusCell(index, col);
  }

  focusCell(index, col) {
    const top = index * ROW_H;
    const s = this.el.scroll;
    if (top < s.scrollTop) s.scrollTop = top;
    else if (top + ROW_H > s.scrollTop + s.clientHeight) s.scrollTop = top + ROW_H - s.clientHeight;
    this.renderWindow();
    const tr = this.el.tbody.querySelector(`tr[data-index="${index}"]`);
    const cell = tr?.querySelectorAll('td.editable')[col];
    if (cell) cell.focus();
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
    const tr = this.el.tbody.querySelector(`tr[data-index="${index}"]`);
    tr?.querySelector('td.editable')?.focus({ preventScroll: true });
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
