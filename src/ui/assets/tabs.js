// WAI-ARIA tabs with a roving tabindex: one tab stop for the whole list,
// Left/Right/Home/End move focus, Enter/Space (or a click) select. Used by
// the workbench (main tabs, table tabs) and the committee page so the
// three tab strips behave the same way.

/**
 * Wires a `[role="tablist"]` whose children are `[role="tab"]` buttons.
 * Each tab's key is `tab.dataset[key]`; `onSelect(key)` runs when one is
 * chosen. `panel`, if given, is marked `role="tabpanel"` and labelled by
 * the selected tab. Returns `{ select(key) }`.
 */
export function wireTablist(list, { key = 'tab', panel = null, onSelect }) {
  const tabs = () => Array.from(list.querySelectorAll('[role="tab"]'));
  const idFor = (tab) => `${list.id || 'tabs'}-${tab.dataset[key]}`;
  for (const tab of tabs()) {
    if (!tab.id) tab.id = idFor(tab);
    tab.tabIndex = -1;
    if (panel) tab.setAttribute('aria-controls', panel.id);
  }
  if (panel) panel.setAttribute('role', 'tabpanel');

  function select(k, { focus = false } = {}) {
    let chosen = null;
    for (const tab of tabs()) {
      const on = tab.dataset[key] === k;
      tab.setAttribute('aria-selected', String(on));
      tab.tabIndex = on ? 0 : -1;
      if (on) chosen = tab;
    }
    if (!chosen) return;
    if (panel) panel.setAttribute('aria-labelledby', chosen.id);
    if (focus) chosen.focus();
    onSelect(k);
  }

  list.addEventListener('click', (e) => {
    const tab = e.target.closest('[role="tab"]');
    if (tab && list.contains(tab)) select(tab.dataset[key]);
  });
  list.addEventListener('keydown', (e) => {
    const all = tabs();
    const i = all.indexOf(document.activeElement);
    if (i < 0) return;
    let next = null;
    if (e.key === 'ArrowRight') next = all[(i + 1) % all.length];
    else if (e.key === 'ArrowLeft') next = all[(i - 1 + all.length) % all.length];
    else if (e.key === 'Home') next = all[0];
    else if (e.key === 'End') next = all[all.length - 1];
    else return;
    e.preventDefault();
    next.focus();
  });
  return { select };
}
