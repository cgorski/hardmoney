// Data browser over the JSON API: candidates, committees, their filings
// and transactions, independent expenditures, and the /schema dashboard.

import { api } from './api.js';
import { escapeHtml, formatMoney, formatDate, formatCount, sumDecimals, labelize } from './format.js';

const LIMITS = [25, 50, 100, 500];
const MAX_LIMIT = 500;

let ui = null;

/** Entry point: `parts` is the path below /ui/data split on '/'. */
export function renderData(main, parts, helpers) {
  ui = helpers;
  const [kind, id] = parts;
  if (kind === 'candidates' && id) renderCandidate(main, decodeURIComponent(id));
  else if (kind === 'committees' && id) renderCommittee(main, decodeURIComponent(id));
  else if (kind === 'schema') renderSchema(main);
  else renderSearch(main);
}

// ---------------------------------------------------------------------------
// Shared pieces
// ---------------------------------------------------------------------------

function errorBanner(e) {
  const hint = e.status === 503 ? ' The server says this data is not loaded.' : '';
  return `<p class="banner banner-error" role="alert">${escapeHtml(e.message)}${hint}</p>`;
}

/** For an entity card: a 404 is shown in place (the bulk table may simply not be loaded), other errors are reported too. */
function entityError(card, e, what) {
  if (e.status === 404) {
    card.innerHTML = `<p class="banner">${escapeHtml(e.message)}. The ${what} bulk file may not be loaded into this namespace; data below comes from other tables.</p>`;
    return;
  }
  card.innerHTML = errorBanner(e);
  ui.reportError(e);
}

function table(columns, rows, { empty = 'No rows.' } = {}) {
  if (!rows.length) return `<p class="muted">${escapeHtml(empty)}</p>`;
  return `<div class="table-scroll"><table class="plain">
    <thead><tr>${columns.map((c) => `<th scope="col" class="${c.amount ? 'amount' : ''}">${escapeHtml(c.label)}</th>`).join('')}</tr></thead>
    <tbody>${rows.map((r) => `<tr>${columns.map((c) => `<td class="${c.amount ? 'amount' : ''} ${c.mono ? 'mono' : ''}">${c.render ? c.render(r) : escapeHtml(r[c.key] ?? '')}</td>`).join('')}</tr>`).join('')}</tbody>
  </table></div>`;
}

const money = (key) => (r) => (r[key] === null || r[key] === undefined ? '' : formatMoney(r[key]));
const date = (key) => (r) => escapeHtml(formatDate(r[key] ?? ''));
const link = (href, text) => `<a href="${escapeHtml(href)}">${escapeHtml(text)}</a>`;

/**
 * A paginated, filterable list backed by one API route. `fields` are the
 * filter inputs; `columns` the table columns. Renders into `host`.
 */
function pagedList(host, { path, fixed = {}, fields = [], columns, title, empty, onRows }) {
  const state = { limit: 50, offset: 0, filters: {}, rows: [] };
  host.innerHTML = `
    ${title ? `<h3>${escapeHtml(title)}</h3>` : ''}
    <form class="field-row pl-filters">
      ${fields.map((f) => `<label class="field"><span class="muted">${escapeHtml(f.label)}</span><input type="${f.type || 'text'}" name="${escapeHtml(f.name)}" placeholder="${escapeHtml(f.placeholder || '')}" ${f.pattern ? `pattern="${f.pattern}"` : ''}></label>`).join('')}
      <label class="field"><span class="muted">Rows per page</span><select name="limit">${LIMITS.map((l) => `<option value="${l}" ${l === state.limit ? 'selected' : ''}>${l}</option>`).join('')}</select></label>
      <button type="submit" class="btn btn-primary">Search</button>
    </form>
    <div class="pager">
      <button type="button" class="btn btn-small" data-prev disabled>Previous</button>
      <button type="button" class="btn btn-small" data-next disabled>Next</button>
      <span class="status" aria-live="polite"></span>
    </div>
    <div class="pl-rows"></div>`;
  const form = host.querySelector('.pl-filters');
  const status = host.querySelector('.status');
  const rowsEl = host.querySelector('.pl-rows');
  const prev = host.querySelector('[data-prev]');
  const next = host.querySelector('[data-next]');

  async function load() {
    status.textContent = 'Loading…';
    rowsEl.setAttribute('aria-busy', 'true');
    try {
      const rows = await api.get(path, { ...fixed, ...state.filters, limit: state.limit, offset: state.offset });
      state.rows = rows;
      rowsEl.innerHTML = table(columns, rows, { empty });
      const from = rows.length ? state.offset + 1 : 0;
      status.textContent = rows.length ? `Rows ${formatCount(from)}–${formatCount(state.offset + rows.length)}${rows.length === state.limit ? ' (more may follow)' : ''}` : 'No rows';
      prev.disabled = state.offset === 0;
      next.disabled = rows.length < state.limit;
      onRows?.(rows, state);
    } catch (e) {
      rowsEl.innerHTML = errorBanner(e);
      status.textContent = '';
      ui.reportError(e);
    } finally {
      rowsEl.removeAttribute('aria-busy');
    }
  }
  form.addEventListener('submit', (e) => {
    e.preventDefault();
    const data = new FormData(form);
    state.filters = {};
    for (const f of fields) {
      const v = String(data.get(f.name) || '').trim();
      if (v) state.filters[f.name] = f.upper ? v.toUpperCase() : v;
    }
    state.limit = Math.min(MAX_LIMIT, Number(data.get('limit')) || 50);
    state.offset = 0;
    load();
  });
  prev.addEventListener('click', () => { state.offset = Math.max(0, state.offset - state.limit); load(); });
  next.addEventListener('click', () => { state.offset += state.limit; load(); });
  load();
  return { reload: load, state };
}

const CYCLE_FIELD = { name: 'cycle', label: 'Cycle', placeholder: '2026', type: 'text', pattern: '\\d{4}' };

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

function renderSearch(main) {
  main.innerHTML = `
    <h1>Data browser</h1>
    <p class="muted">Reads the tables this server's namespace holds. Amounts are shown exactly as the database stores them. See <a href="/ui/data/schema">what is loaded</a>.</p>
    <div class="search-forms">
      <section class="card" aria-labelledby="cand-title"><h2 id="cand-title">Candidates</h2><div id="cand-list"></div></section>
      <section class="card" aria-labelledby="cmte-title"><h2 id="cmte-title">Committees</h2><div id="cmte-list"></div></section>
    </div>`;
  pagedList(main.querySelector('#cand-list'), {
    path: '/candidates',
    fields: [
      { name: 'q', label: 'Name contains', placeholder: 'SMITH' },
      CYCLE_FIELD,
      { name: 'state', label: 'State', placeholder: 'OR', upper: true },
      { name: 'office', label: 'Office', placeholder: 'H, S, or P', upper: true },
    ],
    columns: [
      { label: 'ID', mono: true, render: (r) => link(`/ui/data/candidates/${encodeURIComponent(r.cand_id)}`, r.cand_id) },
      { label: 'Name', key: 'cand_name' },
      { label: 'Cycle', key: 'cycle' },
      { label: 'Party', key: 'cand_pty_affiliation' },
      { label: 'Office', render: (r) => escapeHtml([r.cand_office, r.cand_office_st, r.cand_office_district].filter(Boolean).join(' ')) },
      { label: 'Status', key: 'cand_status' },
      { label: 'Principal committee', mono: true, render: (r) => (r.cand_pcc ? link(`/ui/data/committees/${encodeURIComponent(r.cand_pcc)}`, r.cand_pcc) : '') },
    ],
    empty: 'No candidates match. Is the candidates bulk file loaded for this cycle?',
  });
  pagedList(main.querySelector('#cmte-list'), {
    path: '/committees',
    fields: [
      { name: 'q', label: 'Name contains', placeholder: 'REVIVE' },
      CYCLE_FIELD,
      { name: 'cmte_tp', label: 'Type', placeholder: 'H, S, P, Q, …', upper: true },
    ],
    columns: [
      { label: 'ID', mono: true, render: (r) => link(`/ui/data/committees/${encodeURIComponent(r.cmte_id)}`, r.cmte_id) },
      { label: 'Name', key: 'cmte_nm' },
      { label: 'Cycle', key: 'cycle' },
      { label: 'Type', key: 'cmte_tp' },
      { label: 'Designation', key: 'cmte_dsgn' },
      { label: 'Party', key: 'cmte_pty_affiliation' },
      { label: 'Location', render: (r) => escapeHtml([r.cmte_city, r.cmte_st].filter(Boolean).join(', ')) },
    ],
    empty: 'No committees match. Is the committees bulk file loaded for this cycle?',
  });
}

// ---------------------------------------------------------------------------
// Entity cards
// ---------------------------------------------------------------------------

function entityCard(rows, fields) {
  const latest = rows[0];
  const cycles = rows.map((r) => r.cycle).join(', ');
  return `<dl class="kv">
    ${fields.map(([k, label]) => `<dt>${escapeHtml(label || labelize(k))}</dt><dd>${latest[k] === null || latest[k] === undefined || latest[k] === '' ? '<span class="muted">—</span>' : escapeHtml(String(latest[k]))}</dd>`).join('')}
    <dt>Cycles on file</dt><dd>${escapeHtml(cycles)}</dd>
  </dl>`;
}

const SUPPORT_OPPOSE = { S: 'Support', O: 'Oppose' };

/** Renders support/oppose totals from exact decimal strings. */
function ieTotals(rows) {
  const support = sumDecimals(rows.filter((r) => r.support_oppose_code === 'S').map((r) => r.expenditure_amt));
  const oppose = sumDecimals(rows.filter((r) => r.support_oppose_code === 'O').map((r) => r.expenditure_amt));
  return `<div class="totals">
    <div class="total"><span class="label">Supporting</span><span class="value ok-text">${formatMoney(support)}</span></div>
    <div class="total"><span class="label">Opposing</span><span class="value error-text">${formatMoney(oppose)}</span></div>
    <div class="total"><span class="label">Rows totalled</span><span class="value">${formatCount(rows.length)}</span></div>
  </div>
  <p class="muted">Totals cover the rows on this page (exact decimal addition, no floating point). Raise the page size to total more at once.</p>`;
}

const IE_COLUMNS = [
  { label: 'Date', render: date('expenditure_date') },
  { label: 'Committee', mono: true, render: (r) => (r.cmte_id ? link(`/ui/data/committees/${encodeURIComponent(r.cmte_id)}`, r.cmte_id) : '') },
  { label: 'Committee name', key: 'committee_name' },
  { label: 'For/against', render: (r) => escapeHtml(SUPPORT_OPPOSE[r.support_oppose_code] || r.support_oppose_desc || r.support_oppose_code || '') },
  { label: 'Candidate', render: (r) => (r.candidate_id ? `${link(`/ui/data/candidates/${encodeURIComponent(r.candidate_id)}`, r.candidate_id)} ${escapeHtml(r.candidate_name || '')}` : escapeHtml(r.candidate_name || '')) },
  { label: 'Payee', key: 'payee_name' },
  { label: 'Purpose', key: 'expenditure_description' },
  { label: 'Amount', amount: true, render: money('expenditure_amt') },
];

async function renderCandidate(main, id) {
  main.innerHTML = `<p><a href="/ui/data">← Data browser</a></p><h1>Candidate <span class="mono">${escapeHtml(id)}</span></h1><div id="cand-card" class="card" aria-busy="true">Loading…</div>
    <section class="card" aria-labelledby="ie-title"><h2 id="ie-title">Independent expenditures</h2><div id="ie-totals"></div><div id="ie-list"></div></section>`;
  const card = main.querySelector('#cand-card');
  try {
    const rows = await api.get(`/candidates/${encodeURIComponent(id)}`);
    card.innerHTML = `<h2>${escapeHtml(rows[0].cand_name || id)}</h2>` + entityCard(rows, [
      ['cand_pty_affiliation', 'Party'], ['cand_office', 'Office'], ['cand_office_st', 'State'], ['cand_office_district', 'District'],
      ['cand_election_yr', 'Election year'], ['cand_ici', 'Incumbent/challenger/open'], ['cand_status', 'Status'], ['cand_pcc', 'Principal committee'],
    ]);
    const pcc = rows[0].cand_pcc;
    if (pcc) card.insertAdjacentHTML('beforeend', `<p>${link(`/ui/data/committees/${encodeURIComponent(pcc)}`, `Principal campaign committee ${pcc}`)}</p>`);
  } catch (e) {
    entityError(card, e, 'candidates');
  } finally {
    card.removeAttribute('aria-busy');
  }
  const totals = main.querySelector('#ie-totals');
  pagedList(main.querySelector('#ie-list'), {
    path: '/independent-expenditures',
    fixed: { candidate_id: id },
    fields: [{ name: 'support_oppose_code', label: 'Support (S) or oppose (O)', placeholder: 'S or O', upper: true }],
    columns: IE_COLUMNS,
    empty: 'No independent expenditures for this candidate in the FEC\'s Schedule E dump.',
    onRows: (rows) => { totals.innerHTML = rows.length ? ieTotals(rows) : ''; },
  });
}

async function renderCommittee(main, id) {
  main.innerHTML = `<p><a href="/ui/data">← Data browser</a></p><h1>Committee <span class="mono">${escapeHtml(id)}</span></h1><div id="cmte-card" class="card" aria-busy="true">Loading…</div>
    <section class="card" aria-label="Committee data">
      <div class="tabs" role="tablist" id="cmte-tabs">
        <button role="tab" data-tab="filings" aria-selected="true">Filings</button>
        <button role="tab" data-tab="contributions" aria-selected="false">Contributions (Schedule A)</button>
        <button role="tab" data-tab="disbursements" aria-selected="false">Disbursements (Schedule B)</button>
        <button role="tab" data-tab="ie" aria-selected="false">Independent expenditures</button>
      </div>
      <div id="cmte-panel"></div>
    </section>`;
  const card = main.querySelector('#cmte-card');
  try {
    const rows = await api.get(`/committees/${encodeURIComponent(id)}`);
    card.innerHTML = `<h2>${escapeHtml(rows[0].cmte_nm || id)}</h2>` + entityCard(rows, [
      ['tres_nm', 'Treasurer'], ['cmte_tp', 'Type'], ['cmte_dsgn', 'Designation'], ['cmte_pty_affiliation', 'Party'],
      ['org_tp', 'Organization type'], ['connected_org_nm', 'Connected organization'], ['cmte_city', 'City'], ['cmte_st', 'State'],
    ]);
    if (rows[0].cand_id) card.insertAdjacentHTML('beforeend', `<p>${link(`/ui/data/candidates/${encodeURIComponent(rows[0].cand_id)}`, `Candidate ${rows[0].cand_id}`)}</p>`);
  } catch (e) {
    entityError(card, e, 'committees');
  } finally {
    card.removeAttribute('aria-busy');
  }
  const panel = main.querySelector('#cmte-panel');
  const show = (tab) => {
    for (const b of main.querySelectorAll('#cmte-tabs [role="tab"]')) b.setAttribute('aria-selected', String(b.dataset.tab === tab));
    panel.innerHTML = '';
    if (tab === 'filings') {
      pagedList(panel, {
        path: '/filings',
        fixed: { committee_id: id },
        fields: [
          { name: 'form_type', label: 'Form', placeholder: 'F3X', upper: true },
          { name: 'most_recent', label: 'Most recent only (true/false)', placeholder: 'true' },
        ],
        columns: [
          { label: 'Filing', mono: true, render: (r) => `<a href="${escapeHtml(r.fec_url)}" target="_blank" rel="noopener">${escapeHtml(String(r.filing_id))}</a>` },
          { label: 'Form', key: 'form_type', mono: true },
          { label: 'Report', key: 'report_type' },
          { label: 'Coverage', render: (r) => escapeHtml([r.coverage_start_date, r.coverage_end_date].filter(Boolean).join(' – ')) },
          { label: 'Amendment', render: (r) => escapeHtml(r.is_amendment ? `A${r.amendment_version ?? ''}${r.amends_filing_id ? ` of ${r.amends_filing_id}` : ''}` : 'original') },
          { label: 'Current', render: (r) => (r.most_recent === null ? '<span class="muted">unresolved</span>' : r.most_recent ? 'yes' : 'superseded') },
          { label: 'Version', key: 'fec_version', mono: true },
          { label: 'Ingested', render: date('ingested_at') },
        ],
        empty: 'No ingested filings for this committee. `hardmoney bulk-load-filing` adds them.',
      });
    } else if (tab === 'contributions') {
      pagedList(panel, {
        path: '/schedule-a',
        fixed: { cmte_id: id },
        fields: [
          { name: 'name', label: 'Contributor contains' }, CYCLE_FIELD,
          { name: 'employer', label: 'Employer contains' }, { name: 'occupation', label: 'Occupation contains' },
          { name: 'state', label: 'State', upper: true }, { name: 'zip_code', label: 'ZIP starts with' },
          { name: 'min_amount', label: 'Min amount', placeholder: '200.00' }, { name: 'max_amount', label: 'Max amount' },
          { name: 'min_date', label: 'From (YYYY-MM-DD)', type: 'date' }, { name: 'max_date', label: 'To (YYYY-MM-DD)', type: 'date' },
        ],
        columns: [
          { label: 'Date', render: (r) => escapeHtml(r.transaction_date || r.transaction_dt || '') },
          { label: 'Contributor', key: 'name' },
          { label: 'City', key: 'city' }, { label: 'State', key: 'state' }, { label: 'ZIP', key: 'zip_code', mono: true },
          { label: 'Employer', key: 'employer' }, { label: 'Occupation', key: 'occupation' },
          { label: 'Type', key: 'transaction_tp', mono: true }, { label: 'Entity', key: 'entity_tp', mono: true },
          { label: 'Memo', render: (r) => escapeHtml([r.memo_cd, r.memo_text].filter(Boolean).join(' ')) },
          { label: 'Amount', amount: true, render: money('transaction_amt') },
        ],
        empty: 'No Schedule A rows. Load the individual-contributions bulk file (`indiv`) for the cycle.',
      });
    } else if (tab === 'disbursements') {
      pagedList(panel, {
        path: '/disbursements',
        fixed: { cmte_id: id },
        fields: [
          { name: 'name', label: 'Payee contains' }, CYCLE_FIELD,
          { name: 'purpose', label: 'Purpose contains' }, { name: 'city', label: 'City' }, { name: 'state', label: 'State', upper: true },
          { name: 'min_amount', label: 'Min amount' }, { name: 'max_amount', label: 'Max amount' },
          { name: 'min_date', label: 'From (YYYY-MM-DD)', type: 'date' }, { name: 'max_date', label: 'To (YYYY-MM-DD)', type: 'date' },
        ],
        columns: [
          { label: 'Date', render: (r) => escapeHtml(r.transaction_date || r.transaction_dt || '') },
          { label: 'Payee', key: 'name' },
          { label: 'City', key: 'city' }, { label: 'State', key: 'state' },
          { label: 'Purpose', key: 'purpose' }, { label: 'Category', render: (r) => escapeHtml(r.category_desc || r.category || '') },
          { label: 'Memo', render: (r) => escapeHtml([r.memo_cd, r.memo_text].filter(Boolean).join(' ')) },
          { label: 'Amount', amount: true, render: money('transaction_amt') },
        ],
        empty: 'No disbursement rows. Load the operating-expenditures bulk file (`oppexp`) for the cycle.',
      });
    } else {
      const totals = document.createElement('div');
      const list = document.createElement('div');
      panel.append(totals, list);
      pagedList(list, {
        path: '/independent-expenditures',
        fixed: { cmte_id: id },
        fields: [{ name: 'support_oppose_code', label: 'Support (S) or oppose (O)', upper: true }],
        columns: IE_COLUMNS,
        empty: 'No independent expenditures by this committee in the FEC\'s Schedule E dump.',
        onRows: (rows) => { totals.innerHTML = rows.length ? ieTotals(rows) : ''; },
      });
    }
  };
  main.querySelector('#cmte-tabs').addEventListener('click', (e) => {
    const b = e.target.closest('[role="tab"]');
    if (b) show(b.dataset.tab);
  });
  show('filings');
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

async function renderSchema(main) {
  main.innerHTML = `<p><a href="/ui/data">← Data browser</a></p><h1>What is loaded</h1><div id="schema" class="card" aria-busy="true">Loading…</div>`;
  const host = main.querySelector('#schema');
  try {
    const s = await api.get('/schema');
    host.innerHTML = `
      <dl class="kv">
        <dt>hardmoney version</dt><dd>${escapeHtml(s.hardmoney_version)}</dd>
        <dt>Namespace</dt><dd>${escapeHtml(s.namespace)}</dd>
        <dt>Migrations applied</dt><dd>${escapeHtml(s.migrations_applied.join(', ') || 'none')}</dd>
        <dt>Migrations pending</dt><dd>${s.migrations_pending.length ? `<span class="error-text">${escapeHtml(s.migrations_pending.join(', '))}</span>` : 'none'}</dd>
        <dt>Independent expenditures</dt><dd>${s.independent_expenditures_available ? 'available (FEC Schedule E dump restored)' : '<span class="warn-text">not available</span>: run <code>hardmoney bulk-restore-dump schedule_e</code>'}</dd>
      </dl>
      <p class="muted">A namespace is one Postgres schema holding a complete, isolated set of hardmoney tables. This server was started with <code>--schema ${escapeHtml(s.namespace)}</code>; other namespaces in the same database are not visible here.</p>
      <h2>Latest load per source and cycle</h2>
      ${table([
        { label: 'Source', key: 'source', mono: true },
        { label: 'Cycle', key: 'cycle' },
        { label: 'Loaded', render: (r) => escapeHtml(String(r.loaded_at).replace('T', ' ').slice(0, 19)) },
        { label: 'Mode', key: 'mode' },
        { label: 'Rows', render: (r) => escapeHtml(formatCount(r.row_count)) },
        { label: 'Row limit', render: (r) => (r.row_limit === null ? '' : escapeHtml(formatCount(r.row_limit))) },
        { label: 'Dates nulled', render: (r) => escapeHtml(formatCount(r.dates_nulled)) },
        { label: 'Source last modified', render: (r) => escapeHtml(r.source_last_modified ? String(r.source_last_modified).slice(0, 10) : '') },
        { label: 'By version', key: 'hardmoney_version', mono: true },
      ], s.loads, { empty: 'No bulk loads recorded in this namespace yet.' })}`;
  } catch (e) {
    host.innerHTML = errorBanner(e);
    ui.reportError(e);
  } finally {
    host.removeAttribute('aria-busy');
  }
}
