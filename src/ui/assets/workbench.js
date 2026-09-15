// Filing workbench: load a .fec (file or filing id), inspect header, cover
// page, reconciliation, and validation, edit fields, re-check, download.

import { api, tableSpec } from './api.js';
import { RecordsTable } from './records-table.js';
import { wireTablist } from './tabs.js';
import { escapeHtml, formatMoney, formatCount, formatBytes, isAmountSpec } from './format.js';

const RECHECK_DELAY_MS = 600;
const MAX_FINDINGS_SHOWN = 500;

const state = {
  doc: null,
  source: '',
  edits: new Map(), // `${line_no}:${field}` -> { line_no, table, field, before, after, row }
  specs: new Map(), // table -> TableSpec
  tab: 'records',
  table: null,
  errorLines: new Set(),
  records: null,
  recheckTimer: null,
  checking: false,
  showBlankCover: false,
  onlyMismatches: true,
};

let ui = null; // { toast, reportError, setTitle, focusHeading, showTooltip, hideTooltip, hideTooltipSoon }
let root = null;
let mainTabs = null; // { select } from wireTablist

export function renderWorkbench(main, helpers) {
  ui = helpers;
  root = main;
  if (state.doc) renderLoaded();
  else renderEmpty();
}

function title() {
  const d = state.doc;
  if (!d) return 'Filing workbench';
  const committee = d.summary.fields.filer_committee_id_number || d.summary.fields.candidate_id_number || '';
  return `${d.form_type}${committee ? ' ' + committee : ''} · Filing workbench`;
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

function renderEmpty() {
  root.innerHTML = `
    <h1>Filing workbench</h1>
    <p class="muted">Parse a <code>.fec</code> electronic filing, check it against the FEC's acceptance rules and its own cover-page arithmetic, fix fields, and download the result. Nothing is stored on the server.</p>
    <div class="dropzone" id="dropzone">
      <p><strong>Drop a .fec file here</strong></p>
      <p class="or">or</p>
      <p class="file-pick"><input type="file" id="file-input" accept=".fec,text/plain,application/octet-stream" class="visually-hidden">
      <label class="btn" for="file-input">Choose a file</label></p>
      <form class="fetch-row" id="fetch-form" novalidate>
        <label for="filing-id">Filing id</label>
        <input type="text" id="filing-id" inputmode="numeric" pattern="[0-9]+" placeholder="2011827" required autocomplete="off" aria-describedby="fetch-help">
        <button type="submit" class="btn btn-primary">Fetch from the FEC</button>
      </form>
      <p id="fetch-help" class="muted">The id is the number in an FEC.gov filing URL. The server downloads the file from docquery.fec.gov.</p>
    </div>
    <p id="load-status" class="muted" role="status" aria-live="polite"></p>`;
  ui.setTitle(title());

  const zone = root.querySelector('#dropzone');
  const input = root.querySelector('#file-input');
  input.addEventListener('change', () => { if (input.files[0]) loadFile(input.files[0]); });
  for (const ev of ['dragenter', 'dragover']) {
    zone.addEventListener(ev, (e) => { e.preventDefault(); zone.classList.add('active'); });
  }
  for (const ev of ['dragleave', 'drop']) {
    zone.addEventListener(ev, (e) => { e.preventDefault(); zone.classList.remove('active'); });
  }
  zone.addEventListener('drop', (e) => {
    const f = e.dataTransfer?.files?.[0];
    if (f) loadFile(f);
  });
  const idInput = root.querySelector('#filing-id');
  root.querySelector('#fetch-form').addEventListener('submit', (e) => {
    e.preventDefault();
    const id = idInput.value.trim();
    if (!/^[0-9]+$/.test(id)) {
      fieldError(idInput, id ? `"${id}" is not a filing id. Enter digits only, like 2011827.` : 'Enter a filing id, like 2011827.');
      return;
    }
    fieldError(idInput, null);
    loadId(id);
  });
  idInput.addEventListener('input', () => fieldError(idInput, null));
}

/**
 * Marks a field invalid (or valid again when `message` is null) and puts
 * the message in the live status line, which the field also references
 * through aria-describedby (WCAG 3.3.1, 3.3.3).
 */
function fieldError(input, message) {
  const el = root.querySelector('#load-status');
  const ids = (input.getAttribute('aria-describedby') || '').split(/\s+/).filter((s) => s && s !== 'load-status');
  if (message) {
    input.setAttribute('aria-invalid', 'true');
    input.setAttribute('aria-describedby', [...ids, 'load-status'].join(' '));
    if (el) { el.textContent = message; el.classList.add('error-text'); }
    input.focus();
  } else {
    input.removeAttribute('aria-invalid');
    input.setAttribute('aria-describedby', ids.join(' '));
    if (el && el.classList.contains('error-text')) { el.textContent = ''; el.classList.remove('error-text'); }
  }
}

function setLoadStatus(text) {
  const el = root.querySelector('#load-status');
  if (el) { el.textContent = text; el.classList.remove('error-text'); }
}

async function loadFile(file) {
  setLoadStatus(`Parsing ${file.name} (${formatBytes(file.size)})…`);
  try {
    const bytes = await file.arrayBuffer();
    const doc = await api.postBytes('/tools/parse', bytes);
    accept(doc, file.name);
  } catch (e) {
    setLoadStatus('');
    ui.reportError(e);
  }
}

async function loadId(id) {
  setLoadStatus(`Fetching filing ${id} from the FEC…`);
  try {
    const doc = await api.get(`/tools/fetch/${encodeURIComponent(id)}`);
    accept(doc, `filing ${id}`);
  } catch (e) {
    // Tie the failure to the field that caused it as well as toasting it.
    const input = root.querySelector('#filing-id');
    if (input) fieldError(input, e.message || String(e));
    else setLoadStatus('');
    ui.reportError(e);
  }
}

function accept(doc, source) {
  state.doc = doc;
  state.source = source;
  state.edits.clear();
  state.specs.clear();
  state.tab = 'records';
  state.table = null;
  state.records = null;
  recomputeErrorLines();
  renderLoaded();
  ui.focusHeading(root);
  ui.toast(`Parsed ${source}: ${formatCount(doc.line_count)} lines, ${doc.validation.findings.filter((f) => f.severity === 'error').length} validation errors.`, 'ok');
}

function reset() {
  state.doc = null;
  state.edits.clear();
  state.records = null;
  renderEmpty();
  ui.focusHeading(root);
}

// ---------------------------------------------------------------------------
// Loaded view
// ---------------------------------------------------------------------------

function counts() {
  const v = state.doc.validation.findings;
  const errors = v.filter((f) => f.severity === 'error').length;
  const warnings = v.length - errors;
  const r = state.doc.reconciliation;
  const mismatches = r ? r.checks.filter((c) => !checkMatches(c)).length : null;
  return { errors, warnings, mismatches };
}

function checkMatches(c) {
  // Mirrors LineCheck::matches: delta == 0, or delta >= 0 for a floor.
  const d = c.delta;
  if (/^-?0(\.0*)?$/.test(d)) return true;
  return c.relation === 'at_least' && !d.startsWith('-');
}

function recomputeErrorLines() {
  state.errorLines = new Set(state.doc.validation.findings.filter((f) => f.severity === 'error').map((f) => f.line_no));
}

function tableOrder() {
  const seen = new Map();
  for (const l of state.doc.lines) seen.set(l.table, (seen.get(l.table) || 0) + 1);
  return Array.from(seen.entries());
}

function renderLoaded() {
  const d = state.doc;
  const committee = d.summary.fields.filer_committee_id_number || d.summary.fields.candidate_id_number || '';
  const name = d.summary.fields.committee_name || d.summary.fields.candidate_name || '';
  ui.setTitle(title());
  root.innerHTML = `
    <div class="card-header">
      <h1>Filing workbench</h1>
      <span class="spacer"></span>
      <button type="button" class="btn" id="download-btn">Download .fec</button>
      <button type="button" class="btn" id="reset-edits-btn" ${state.edits.size ? '' : 'disabled'}>Revert edits</button>
      <button type="button" class="btn btn-quiet" id="close-btn">Close filing</button>
    </div>
    <div class="card">
      <div class="summary-strip">
        <span class="item"><span class="label">Source</span> <span>${escapeHtml(state.source)}</span></span>
        <span class="item"><span class="label">Form</span> <span class="mono">${escapeHtml(d.form_type)}</span>${d.is_amendment ? ' <span class="badge">amendment' + (d.amends_filing ? ` of ${escapeHtml(String(d.amends_filing))}` : '') + '</span>' : ''}</span>
        <span class="item"><span class="label">Version</span> <span class="mono">${escapeHtml(d.version)}</span></span>
        <span class="item"><span class="label">Filer</span> <span class="mono">${escapeHtml(committee)}</span> <span>${escapeHtml(name)}</span></span>
        <span class="item"><span class="label">Body lines</span> <span>${formatCount(d.line_count)}</span></span>
        <span class="item" id="status-badges"></span>
      </div>
      ${d.skipped.length ? `<p class="banner banner-warn">${d.skipped.length} body line(s) could not be parsed and are not shown: ${escapeHtml(d.skipped.slice(0, 5).map((s) => `line ${s.line_no} '${s.raw_form_type}' (${s.reason})`).join('; '))}${d.skipped.length > 5 ? '; …' : ''}. They are kept out of the written file.</p>` : ''}
    </div>
    <div class="grid-2" id="top-cards">
      <section class="card" aria-labelledby="hdr-title">
        <div class="card-header"><h2 id="hdr-title">Header</h2><span class="muted">line 1</span></div>
        <dl class="kv" id="header-kv"></dl>
      </section>
      <section class="card" aria-labelledby="cover-title">
        <div class="card-header">
          <h2 id="cover-title">Cover page</h2>
          <span class="muted mono">${escapeHtml(d.summary.table)} · line 2</span>
          <span class="spacer"></span>
          <label class="muted"><input type="checkbox" id="show-blank-cover" ${state.showBlankCover ? 'checked' : ''}> show blank fields</label>
        </div>
        <dl class="kv" id="cover-kv"></dl>
      </section>
    </div>
    <section class="card" aria-label="Details">
      <div class="tabs" role="tablist" id="main-tabs" aria-label="Details">
        <button type="button" role="tab" data-tab="records">Records <span class="count">${formatCount(d.line_count)}</span></button>
        <button type="button" role="tab" data-tab="validation">Validation <span class="count" id="tab-validation-count"></span></button>
        <button type="button" role="tab" data-tab="reconcile">Reconcile <span class="count" id="tab-reconcile-count"></span></button>
        <button type="button" role="tab" data-tab="edits">Edits <span class="count" id="tab-edits-count">${state.edits.size}</span></button>
      </div>
      <div id="panel"></div>
    </section>`;

  root.querySelector('#download-btn').addEventListener('click', download);
  root.querySelector('#reset-edits-btn').addEventListener('click', revertAll);
  root.querySelector('#close-btn').addEventListener('click', reset);
  root.querySelector('#show-blank-cover').addEventListener('change', (e) => {
    state.showBlankCover = e.target.checked;
    renderCover();
  });
  mainTabs = wireTablist(root.querySelector('#main-tabs'), {
    panel: root.querySelector('#panel'),
    onSelect: (tab) => {
      state.tab = tab;
      const panel = root.querySelector('#panel');
      panel.innerHTML = '';
      if (tab === 'records') renderRecords();
      else if (tab === 'validation') renderValidation();
      else if (tab === 'reconcile') renderReconcile();
      else renderEdits();
    },
  });

  renderHeader();
  renderCover();
  updateBadges();
  selectTab(state.tab);
}

function updateBadges() {
  const { errors, warnings, mismatches } = counts();
  const badges = [];
  badges.push(errors ? `<span class="badge badge-error">${errors} error${errors === 1 ? '' : 's'}</span>` : '<span class="badge badge-ok">no validation errors</span>');
  if (warnings) badges.push(`<span class="badge badge-warn">${warnings} warning${warnings === 1 ? '' : 's'}</span>`);
  if (mismatches !== null) {
    badges.push(mismatches ? `<span class="badge badge-error">${mismatches} cover line${mismatches === 1 ? '' : 's'} off</span>` : '<span class="badge badge-ok">cover page reconciles</span>');
  }
  if (state.edits.size) badges.push(`<span class="badge">${state.edits.size} edit${state.edits.size === 1 ? '' : 's'}</span>`);
  root.querySelector('#status-badges').innerHTML = badges.join(' ');
  root.querySelector('#tab-validation-count').textContent = `${errors} / ${warnings}`;
  root.querySelector('#tab-reconcile-count').textContent = mismatches === null ? 'n/a' : String(mismatches);
  root.querySelector('#tab-edits-count').textContent = String(state.edits.size);
  root.querySelector('#reset-edits-btn').disabled = state.edits.size === 0;
}

// ---------------------------------------------------------------------------
// Header and cover cards (editable key/value lists)
// ---------------------------------------------------------------------------

const HEADER_FIELDS = ['record_type', 'ef_type', 'fec_version_raw', 'soft_name', 'soft_ver', 'name_delim', 'report_id', 'report_number', 'comment'];

function renderHeader() {
  const dl = root.querySelector('#header-kv');
  dl.innerHTML = '';
  const h = state.doc.header;
  for (const f of HEADER_FIELDS) {
    if (f === 'name_delim' && h.name_delim === null) continue;
    const dt = document.createElement('dt');
    dt.textContent = f;
    const dd = document.createElement('dd');
    const input = kvInput(f, h[f] ?? '', 1, 'HDR', (before, after) => {
      h[f] = after;
      recordEdit({ line_no: 1, table: 'HDR', field: f, before, after, row: null });
    });
    dd.appendChild(input);
    dl.append(dt, dd);
  }
}

function coverSpecFor(field) {
  return state.specs.get(state.doc.summary.table)?.byName.get(field);
}

function renderCover() {
  const dl = root.querySelector('#cover-kv');
  dl.innerHTML = '';
  const s = state.doc.summary;
  const spec = state.specs.get(s.table);
  if (!spec) {
    tableSpec(s.table, state.doc.version).then((sp) => { state.specs.set(s.table, sp); renderCover(); }).catch(() => {});
  }
  for (const [f, v] of Object.entries(s.fields)) {
    const edited = state.edits.has(`2:${f}`);
    if (!state.showBlankCover && v === '' && !edited) continue;
    const info = spec?.byName.get(f);
    const dt = document.createElement('dt');
    dt.textContent = f;
    if (info?.spec?.description) dt.title = info.spec.description;
    const dd = document.createElement('dd');
    const input = kvInput(f, v, 2, s.table, (before, after) => {
      s.fields[f] = after;
      recordEdit({ line_no: 2, table: s.table, field: f, before, after, row: s });
    });
    input.id = `cover-${f}`;
    if (isAmountSpec(info?.spec)) {
      input.classList.add('amount');
      input.title = v === '' ? '' : formatMoney(v);
    }
    dd.appendChild(input);
    dl.append(dt, dd);
  }
}

function kvInput(field, value, lineNo, table, commit) {
  const input = document.createElement('input');
  input.type = 'text';
  input.value = value;
  input.autocomplete = 'off';
  // The edited state is in the name as well as the colour (WCAG 1.4.1).
  const mark = () => {
    const edited = state.edits.has(`${lineNo}:${field}`);
    input.classList.toggle('edited', edited);
    input.setAttribute('aria-label', `${field}, line ${lineNo}${edited ? ', edited' : ''}`);
  };
  mark();
  input.addEventListener('change', () => {
    const after = input.value.trim();
    const before = value;
    input.value = after;
    if (after === before) return;
    commit(before, after);
    value = after;
    mark();
  });
  return input;
}

// ---------------------------------------------------------------------------
// Edits
// ---------------------------------------------------------------------------

function recordEdit({ line_no, table, field, before, after, row }) {
  const key = `${line_no}:${field}`;
  const existing = state.edits.get(key);
  const original = existing ? existing.before : before;
  if (after === original) state.edits.delete(key);
  else state.edits.set(key, { line_no, table, field, before: original, after, row });
  updateBadges();
  if (state.tab === 'edits') renderEdits();
  scheduleRecheck();
}

function isEdited(row, field) {
  return state.edits.has(`${row.line_no}:${field}`);
}

function revertEdit(key) {
  const e = state.edits.get(key);
  if (!e) return;
  if (e.line_no === 1) state.doc.header[e.field] = e.before;
  else e.row.fields[e.field] = e.before;
  state.edits.delete(key);
  afterRevert();
}

function revertAll() {
  for (const key of Array.from(state.edits.keys())) {
    const e = state.edits.get(key);
    if (e.line_no === 1) state.doc.header[e.field] = e.before;
    else e.row.fields[e.field] = e.before;
    state.edits.delete(key);
  }
  afterRevert();
}

function afterRevert() {
  renderHeader();
  renderCover();
  updateBadges();
  state.records?.refresh();
  if (state.tab === 'edits') renderEdits();
  scheduleRecheck();
}

function toDocument() {
  const d = state.doc;
  return {
    version: d.version,
    header: d.header,
    summary: { table: d.summary.table, line_no: d.summary.line_no, fields: d.summary.fields },
    lines: d.lines.map((l) => ({ table: l.table, line_no: l.line_no, fields: l.fields })),
  };
}

function scheduleRecheck() {
  clearTimeout(state.recheckTimer);
  state.recheckTimer = setTimeout(recheck, RECHECK_DELAY_MS);
}

async function recheck() {
  if (state.checking || !state.doc) { scheduleRecheck(); return; }
  state.checking = true;
  const before = counts();
  try {
    const doc = toDocument();
    const validation = await api.postJson('/tools/validate', doc);
    state.doc.validation = validation;
    if (state.doc.reconciliation) {
      state.doc.reconciliation = await api.postJson('/tools/reconcile', doc);
    }
    recomputeErrorLines();
    const after = counts();
    updateBadges();
    if (state.tab === 'validation') renderValidation();
    if (state.tab === 'reconcile') renderReconcile();
    state.records?.refresh();
    const parts = [];
    if (after.errors !== before.errors) parts.push(`errors ${before.errors} → ${after.errors}`);
    if (after.warnings !== before.warnings) parts.push(`warnings ${before.warnings} → ${after.warnings}`);
    if (after.mismatches !== null && after.mismatches !== before.mismatches) parts.push(`cover lines off ${before.mismatches} → ${after.mismatches}`);
    ui.toast(parts.length ? `Re-checked: ${parts.join(', ')}.` : 'Re-checked: no change in findings.', 'info');
  } catch (e) {
    ui.reportError(e);
  } finally {
    state.checking = false;
  }
}

async function download() {
  try {
    const res = await api.postJsonRaw('/tools/write', toDocument());
    const blob = await res.blob();
    const disposition = res.headers.get('Content-Disposition') || '';
    const m = /filename="([^"]+)"/.exec(disposition);
    const filename = m ? m[1] : 'filing.fec';
    const errors = res.headers.get('X-Hardmoney-Validation-Errors');
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 10_000);
    const n = Number(errors || 0);
    ui.toast(n ? `Wrote ${filename} with ${n} validation error${n === 1 ? '' : 's'}; the FEC would reject it.` : `Wrote ${filename}; no validation errors.`, n ? 'error' : 'ok');
  } catch (e) {
    ui.reportError(e);
  }
}

// ---------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------

function selectTab(tab) {
  mainTabs?.select(tab);
}

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

function renderRecords() {
  const panel = root.querySelector('#panel');
  const tables = tableOrder();
  if (!tables.length) {
    panel.innerHTML = '<p class="muted">This filing has no body lines.</p>';
    return;
  }
  if (!state.table || !tables.some(([t]) => t === state.table)) state.table = tables[0][0];
  panel.innerHTML = `
    <div class="tabs" role="tablist" id="table-tabs" aria-label="Tables">
      ${tables.map(([t, n]) => `<button type="button" role="tab" data-table="${escapeHtml(t)}">${escapeHtml(t)} <span class="count">${formatCount(n)}</span></button>`).join('')}
    </div>
    <div id="records-host"></div>`;
  const host = panel.querySelector('#records-host');
  let shown = null;
  const tablist = wireTablist(panel.querySelector('#table-tabs'), {
    key: 'table',
    panel: host,
    onSelect: (t) => {
      if (t === shown) return;
      shown = t;
      state.table = t;
      showTable(t);
    },
  });
  state.records = new RecordsTable(host, {
    onEdit: (row, field, before, after) => {
      row.fields[field] = after;
      recordEdit({ line_no: row.line_no, table: row.table, field, before, after, row });
    },
    getSpec: (field) => state.specs.get(state.table)?.byName.get(field),
    isEdited,
    lineHasError: (lineNo) => state.errorLines.has(lineNo),
    showTooltip: ui.showTooltip,
    hideTooltip: ui.hideTooltip,
    hideTooltipSoon: ui.hideTooltipSoon,
  });
  tablist.select(state.table);
}

async function showTable(table) {
  const rows = state.doc.lines.filter((l) => l.table === table);
  let fields = rows.length ? Object.keys(rows[0].fields) : [];
  state.records.setRows(rows, fields);
  state.records.setLabel(`${table} records`);
  try {
    if (!state.specs.has(table)) state.specs.set(table, await tableSpec(table, state.doc.version));
    if (state.table === table && state.records) {
      fields = state.specs.get(table).fields.map((f) => f.name);
      state.records.setRows(rows, fields);
    }
  } catch (e) {
    ui.reportError(e);
  }
}

/** Jumps to a line: the cover card for line 2, the records table otherwise. */
function goToLine(lineNo, field) {
  if (lineNo <= 2) {
    if (lineNo === 2 && field && !state.showBlankCover && state.doc.summary.fields[field] === '') {
      state.showBlankCover = true;
      root.querySelector('#show-blank-cover').checked = true;
      renderCover();
    }
    const target = lineNo === 2 && field ? root.querySelector(`#cover-${CSS.escape(field)}`) : root.querySelector(lineNo === 1 ? '#header-kv input' : '#cover-kv input');
    target?.scrollIntoView({ block: 'center' });
    target?.focus();
    return;
  }
  const line = state.doc.lines.find((l) => l.line_no === lineNo);
  if (!line) { ui.toast(`Line ${lineNo} is not among the parsed records.`, 'info'); return; }
  state.table = line.table;
  selectTab('records');
  const tryScroll = () => {
    if (field && state.records && !state.records.visible.has(field)) {
      state.records.visible.add(field);
      state.records.renderHeader();
    }
    state.records?.scrollToLine(lineNo);
  };
  tryScroll();
  setTimeout(tryScroll, 250);
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

function renderValidation() {
  const panel = root.querySelector('#panel');
  const findings = state.doc.validation.findings;
  const errors = findings.filter((f) => f.severity === 'error');
  const warnings = findings.filter((f) => f.severity === 'warning');
  const group = (title, list, cls) => {
    if (!list.length) return '';
    const shown = list.slice(0, MAX_FINDINGS_SHOWN);
    return `
      <h3><span class="badge ${cls}">${title} · ${formatCount(list.length)}</span></h3>
      ${list.length > shown.length ? `<p class="muted">Showing the first ${MAX_FINDINGS_SHOWN}.</p>` : ''}
      <ul class="findings">
        ${shown.map((f) => `<li>
          <span class="where"><button type="button" class="btn-link" data-line="${f.line_no}" data-field="${escapeHtml(f.field || '')}">line ${f.line_no}</button></span>
          <span><span class="mono">${escapeHtml(f.form_type)}${f.field ? ' · ' + escapeHtml(f.field) : ''}</span>: ${escapeHtml(f.message)} <span class="rule">${escapeHtml(f.rule)}</span></span>
        </li>`).join('')}
      </ul>`;
  };
  panel.innerHTML = findings.length
    ? `<p class="muted">Findings from the FEC's acceptance rules. Errors would make the FEC reject the filing; warnings are reported but accepted. Activate a line number to go to the record.</p>
       ${group('Errors', errors, 'badge-error')}${group('Warnings', warnings, 'badge-warn')}`
    : '<p class="banner banner-ok">No findings. The filing passes every rule this validator implements.</p>';
  panel.addEventListener('click', (e) => {
    const b = e.target.closest('button[data-line]');
    if (b) goToLine(Number(b.dataset.line), b.dataset.field || null);
  });
}

// ---------------------------------------------------------------------------
// Reconcile
// ---------------------------------------------------------------------------

function renderReconcile() {
  const panel = root.querySelector('#panel');
  const r = state.doc.reconciliation;
  if (!r) {
    panel.innerHTML = `<p class="banner">${escapeHtml(state.doc.reconcile_error || 'No reconciliation for this form.')}</p>`;
    return;
  }
  const checks = r.checks;
  const mismatches = checks.filter((c) => !checkMatches(c));
  const shown = state.onlyMismatches ? mismatches : checks;
  panel.innerHTML = `
    <div class="card-header">
      <p class="muted">Each cover-page line recomputed from the schedules (memo entries excluded) or from other cover lines, with exact decimal arithmetic. ${mismatches.length ? `<strong class="error-text">${mismatches.length} of ${checks.length} lines disagree.</strong>` : `<strong class="ok-text">All ${checks.length} lines agree.</strong>`}</p>
      <span class="spacer"></span>
      <label class="muted"><input type="checkbox" id="only-mismatches" ${state.onlyMismatches ? 'checked' : ''}> only mismatches</label>
    </div>
    ${shown.length ? `<div class="table-scroll" tabindex="0" role="region" aria-label="Reconciliation checks"><table class="plain">
      <thead><tr><th scope="col">Result</th><th scope="col">Col</th><th scope="col">Line</th><th scope="col">Field</th><th scope="col" class="amount">Reported</th><th scope="col" class="amount">Expected</th><th scope="col" class="amount">Delta</th><th scope="col">Relation</th><th scope="col">Rule</th><th scope="col" class="right">Summed</th></tr></thead>
      <tbody>
        ${shown.map((c) => `<tr class="${checkMatches(c) ? '' : 'mismatch'}">
          <td>${checkMatches(c) ? '<span class="badge badge-ok">agrees</span>' : '<span class="badge badge-error">off</span>'}</td>
          <td>${escapeHtml(c.column)}</td>
          <td class="mono">${escapeHtml(c.line)}</td>
          <td><button type="button" class="btn-link mono" data-field="${escapeHtml(c.field)}">${escapeHtml(c.field)}</button></td>
          <td class="amount">${c.reported === null ? '<span class="muted">blank</span>' : formatMoney(c.reported)}${c.reported_unparseable ? ' <span class="badge badge-warn">unparseable</span>' : ''}</td>
          <td class="amount">${formatMoney(c.expected)}</td>
          <td class="amount">${formatMoney(c.delta)}</td>
          <td>${c.relation === 'at_least' ? 'at least' : 'equals'}</td>
          <td>${escapeHtml(c.rule)}</td>
          <td class="right">${c.lines_summed || ''}</td>
        </tr>`).join('')}
      </tbody></table></div>` : '<p class="banner banner-ok">Every checked line agrees with its rule.</p>'}`;
  panel.querySelector('#only-mismatches').addEventListener('change', (e) => {
    state.onlyMismatches = e.target.checked;
    renderReconcile();
  });
  panel.addEventListener('click', (e) => {
    const b = e.target.closest('button[data-field]');
    if (b) goToLine(2, b.dataset.field);
  });
}

// ---------------------------------------------------------------------------
// Edits
// ---------------------------------------------------------------------------

function renderEdits() {
  const panel = root.querySelector('#panel');
  const edits = Array.from(state.edits.entries()).sort((a, b) => a[1].line_no - b[1].line_no || a[1].field.localeCompare(b[1].field));
  if (!edits.length) {
    panel.innerHTML = '<p class="muted">No edits. Activate a cell in the records table (click it, or press Enter on it), or change a value on the header or cover card. Edits stay in this page until you download.</p>';
    return;
  }
  panel.innerHTML = `
    <p class="muted">${edits.length} field${edits.length === 1 ? '' : 's'} changed from the parsed filing. Download writes them into the <code>.fec</code>.</p>
    <div class="table-scroll" tabindex="0" role="region" aria-label="Edits"><table class="plain edits">
      <thead><tr><th scope="col">Line</th><th scope="col">Table</th><th scope="col">Field</th><th scope="col">Before</th><th scope="col">After</th><th scope="col">Action</th></tr></thead>
      <tbody>${edits.map(([key, e]) => `<tr>
        <td><button type="button" class="btn-link" data-line="${e.line_no}" data-field="${escapeHtml(e.field)}">${e.line_no}</button></td>
        <td class="mono">${escapeHtml(e.table)}</td>
        <td class="mono">${escapeHtml(e.field)}</td>
        <td class="before mono">${e.before === '' ? '<span class="muted">blank</span>' : escapeHtml(e.before)}</td>
        <td class="after mono">${e.after === '' ? '<span class="muted">blank</span>' : escapeHtml(e.after)}</td>
        <td><button type="button" class="btn btn-small" data-revert="${escapeHtml(key)}" aria-label="Revert ${escapeHtml(e.field)} on line ${e.line_no}">Revert</button></td>
      </tr>`).join('')}</tbody>
    </table></div>`;
  panel.addEventListener('click', (e) => {
    const r = e.target.closest('button[data-revert]');
    if (r) { revertEdit(r.dataset.revert); renderEdits(); return; }
    const b = e.target.closest('button[data-line]');
    if (b) goToLine(Number(b.dataset.line), b.dataset.field || null);
  });
}
