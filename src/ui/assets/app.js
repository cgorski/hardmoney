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
  // A toggle button keeps one label ("Dark theme") and reports its state
  // through aria-pressed; changing the label with the state would make
  // "pressed" ambiguous to a screen reader.
  document.getElementById('theme-toggle').setAttribute('aria-pressed', String(theme === 'dark'));
}

// ---------------------------------------------------------------------------
// Toasts
// ---------------------------------------------------------------------------

const TOAST_MS = 6000;

/**
 * Speaks `message` through the persistent live region for its urgency.
 * The region is emptied first so an identical message is announced again.
 */
export function announce(message, assertive = false) {
  const region = document.getElementById(assertive ? 'announce-alert' : 'announce-status');
  region.textContent = '';
  setTimeout(() => { region.textContent = message; }, 50);
}

/**
 * Shows a message. kind: 'error' | 'ok' | 'info'. Errors stay until
 * dismissed; others go after a few seconds, but not while hovered or
 * focused, and the same text is announced through a live region.
 */
export function toast(message, kind = 'info') {
  const host = document.getElementById('toasts');
  const el = document.createElement('div');
  el.className = `toast toast-${kind}`;
  el.innerHTML = `<div class="msg">${escapeHtml(message)}</div><button type="button" class="btn btn-small" aria-label="Dismiss notification"><span aria-hidden="true">×</span></button>`;
  el.querySelector('button').addEventListener('click', () => el.remove());
  host.appendChild(el);
  announce(message, kind === 'error');
  if (kind !== 'error') {
    let timer = setTimeout(() => el.remove(), TOAST_MS);
    const hold = () => clearTimeout(timer);
    const release = () => { clearTimeout(timer); timer = setTimeout(() => el.remove(), TOAST_MS); };
    el.addEventListener('mouseenter', hold);
    el.addEventListener('focusin', hold);
    el.addEventListener('mouseleave', release);
    el.addEventListener('focusout', release);
  }
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
let tooltipAnchor = null;
let tooltipHideTimer = null;

/**
 * Shows the field-spec tooltip under `anchor` and links it with
 * aria-describedby. WCAG 1.4.13: it stays while the pointer is over it
 * (see the listeners in `boot`) and Escape hides it (the anchor's owner
 * calls `hideTooltip` on Escape).
 */
export function showTooltip(html, anchor) {
  clearTimeout(tooltipHideTimer);
  const t = tip();
  if (tooltipAnchor && tooltipAnchor !== anchor) tooltipAnchor.removeAttribute('aria-describedby');
  tooltipAnchor = anchor;
  anchor.setAttribute('aria-describedby', t.id);
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
  clearTimeout(tooltipHideTimer);
  tip().hidden = true;
  if (tooltipAnchor) tooltipAnchor.removeAttribute('aria-describedby');
  tooltipAnchor = null;
}

/** Hides the tooltip shortly, unless the pointer reaches it first. */
export function hideTooltipSoon() {
  clearTimeout(tooltipHideTimer);
  tooltipHideTimer = setTimeout(hideTooltip, 150);
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
let booted = false;

const SITE = 'hardmoney';

/** Sets the document title for the current view (WCAG 2.4.2). */
export function setTitle(view) {
  document.title = view ? `${view} · ${SITE}` : SITE;
}

/**
 * Moves focus to the view's heading after a client-side navigation or a
 * change of view, so keyboard and screen-reader users land on the new
 * content instead of on the document body.
 */
export function focusHeading(root = document.getElementById('main')) {
  const h = root.querySelector('h1') || root;
  if (!h.hasAttribute('tabindex')) h.setAttribute('tabindex', '-1');
  h.focus({ preventScroll: false });
}

function render() {
  const main = document.getElementById('main');
  const path = routePath();
  const section = path.split('/')[0] || 'workbench';
  for (const a of document.querySelectorAll('[data-nav]')) {
    if (a.dataset.nav === section) a.setAttribute('aria-current', 'page');
    else a.removeAttribute('aria-current');
  }
  const helpers = { toast, reportError, navigate, setTitle, focusHeading, showTooltip, hideTooltip, hideTooltipSoon };
  if (section === 'data') {
    current = 'data';
    renderData(main, path.split('/').slice(1), helpers);
  } else {
    // The workbench keeps its document across renders so a back/forward
    // does not lose a loaded filing.
    if (current !== 'workbench') {
      renderWorkbench(main, helpers);
    }
    current = 'workbench';
  }
  // On the first paint the browser's own focus handling is right; after a
  // navigation inside the app nothing else moves focus, so we do.
  if (booted) focusHeading(main);
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

  // The tooltip must be hoverable (1.4.13): leaving the anchor only
  // schedules the hide, and entering the tooltip cancels it.
  const t = tip();
  t.addEventListener('mouseenter', () => clearTimeout(tooltipHideTimer));
  t.addEventListener('mouseleave', hideTooltip);
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && !t.hidden) hideTooltip(); });
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
  booted = true;
}

boot();
