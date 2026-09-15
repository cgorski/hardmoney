// Shell: theme, client-side routing under /ui, toasts, the API-key dialog,
// and the shared field-spec tooltip.

import { getApiKey, setApiKey, onUnauthorized, ApiError } from './api.js';
import { renderWorkbench } from './workbench.js';
import { renderData } from './data.js';
import { escapeHtml } from './format.js';

const THEME_STORAGE = 'hardmoney.theme';

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

function systemTheme() {
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

function currentTheme() {
  return localStorage.getItem(THEME_STORAGE) || systemTheme();
}

function applyTheme(theme, persist) {
  document.documentElement.dataset.theme = theme;
  if (persist) localStorage.setItem(THEME_STORAGE, theme);
  const btn = document.getElementById('theme-toggle');
  const dark = theme === 'dark';
  btn.textContent = dark ? 'Light theme' : 'Dark theme';
  btn.setAttribute('aria-pressed', String(dark));
}

// ---------------------------------------------------------------------------
// Toasts
// ---------------------------------------------------------------------------

/** Shows a message. kind: 'error' | 'ok' | 'info'. Errors stay until dismissed. */
export function toast(message, kind = 'info') {
  const host = document.getElementById('toasts');
  const el = document.createElement('div');
  el.className = `toast toast-${kind}`;
  el.setAttribute('role', kind === 'error' ? 'alert' : 'status');
  el.innerHTML = `<div class="msg">${escapeHtml(message)}</div><button type="button" class="btn btn-small" aria-label="Dismiss">×</button>`;
  el.querySelector('button').addEventListener('click', () => el.remove());
  host.appendChild(el);
  if (kind !== 'error') setTimeout(() => el.remove(), 6000);
  while (host.children.length > 6) host.firstElementChild.remove();
}

/** Reports a caught error to the user, with the API's message when it has one. */
export function reportError(e) {
  if (e instanceof ApiError) toast(e.message, 'error');
  else toast(e?.message || String(e), 'error');
  console.error(e);
}

// ---------------------------------------------------------------------------
// API key dialog
// ---------------------------------------------------------------------------

/** Opens the key dialog; resolves to true if a key was saved. */
export function promptApiKey() {
  const dialog = document.getElementById('api-key-dialog');
  const input = document.getElementById('api-key-input');
  input.value = getApiKey();
  return new Promise((resolve) => {
    // Tracked here rather than through `dialog.returnValue`: a form with
    // method="dialog" overwrites the return value with the submitter's.
    let saved = false;
    const onClose = () => {
      dialog.removeEventListener('close', onClose);
      resolve(saved);
    };
    dialog.addEventListener('close', onClose);
    dialog.querySelector('[data-action="cancel"]').onclick = () => dialog.close();
    document.getElementById('api-key-form').onsubmit = () => {
      setApiKey(input.value.trim());
      saved = true;
    };
    dialog.showModal();
    input.focus();
  });
}

// ---------------------------------------------------------------------------
// Tooltip (shared)
// ---------------------------------------------------------------------------

const tip = () => document.getElementById('tooltip');

export function showTooltip(html, anchor) {
  const t = tip();
  t.innerHTML = html;
  t.hidden = false;
  const r = anchor.getBoundingClientRect();
  const margin = 8;
  let left = r.left;
  let top = r.bottom + margin;
  const w = t.offsetWidth;
  const h = t.offsetHeight;
  if (left + w > window.innerWidth - margin) left = Math.max(margin, window.innerWidth - w - margin);
  if (top + h > window.innerHeight - margin) top = Math.max(margin, r.top - h - margin);
  t.style.left = `${left}px`;
  t.style.top = `${top}px`;
}

export function hideTooltip() {
  tip().hidden = true;
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

const BASE = '/ui';

/** The path below /ui, without a trailing slash: '' for the workbench, 'data/...' for the browser. */
function routePath() {
  let p = window.location.pathname;
  if (p.startsWith(BASE)) p = p.slice(BASE.length);
  return p.replace(/^\/+|\/+$/g, '');
}

export function navigate(path, { replace = false } = {}) {
  const url = `${BASE}/${path.replace(/^\/+/, '')}`;
  if (replace) history.replaceState(null, '', url);
  else history.pushState(null, '', url);
  render();
}

let current = null;

function render() {
  const main = document.getElementById('main');
  const path = routePath();
  const section = path.split('/')[0] || 'workbench';
  for (const a of document.querySelectorAll('[data-nav]')) {
    if (a.dataset.nav === section) a.setAttribute('aria-current', 'page');
    else a.removeAttribute('aria-current');
  }
  if (section === 'data') {
    current = 'data';
    renderData(main, path.split('/').slice(1), { toast, reportError, navigate });
  } else {
    // The workbench keeps its document across renders so a back/forward
    // does not lose a loaded filing.
    if (current !== 'workbench') {
      renderWorkbench(main, { toast, reportError, showTooltip, hideTooltip });
    }
    current = 'workbench';
  }
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

function boot() {
  applyTheme(currentTheme(), false);
  document.getElementById('theme-toggle').addEventListener('click', () => {
    applyTheme(currentTheme() === 'dark' ? 'light' : 'dark', true);
  });
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
    if (!localStorage.getItem(THEME_STORAGE)) applyTheme(systemTheme(), false);
  });

  document.getElementById('api-key-button').addEventListener('click', () => {
    promptApiKey().then((saved) => { if (saved) toast('API key saved for this tab.', 'ok'); });
  });
  onUnauthorized(async () => {
    toast('The server requires an API key.', 'info');
    return promptApiKey();
  });

  // Same-origin links under /ui go through the client-side router.
  document.addEventListener('click', (ev) => {
    const a = ev.target.closest('a[href]');
    if (!a || ev.defaultPrevented || ev.button !== 0 || ev.metaKey || ev.ctrlKey || ev.shiftKey || ev.altKey) return;
    const url = new URL(a.href, window.location.href);
    if (url.origin !== window.location.origin || !url.pathname.startsWith(BASE)) return;
    if (a.hasAttribute('download') || a.target === '_blank') return;
    ev.preventDefault();
    if (url.pathname + url.search !== window.location.pathname + window.location.search) {
      history.pushState(null, '', url.pathname + url.search);
    }
    render();
  });
  window.addEventListener('popstate', render);
  document.addEventListener('scroll', hideTooltip, true);
  render();
}

boot();
