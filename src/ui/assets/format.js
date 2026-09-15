// Formatting helpers. Money is handled as decimal strings end to end:
// nothing here converts an amount to a binary float.

/** Escapes text for insertion into HTML. */
export function escapeHtml(s) {
  return String(s ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

const DECIMAL = /^([+-])?(\d*)(?:\.(\d*))?$/;

/**
 * Splits a decimal string into { sign, int, frac } or returns null when it
 * is not `[-]digits[.digits]`. Blank is null.
 */
export function splitDecimal(s) {
  const t = String(s ?? '').trim();
  if (t === '') return null;
  const m = DECIMAL.exec(t);
  if (!m) return null;
  const int = m[2] === '' ? '0' : m[2].replace(/^0+(?=\d)/, '');
  const frac = m[3] ?? '';
  if (m[2] === '' && frac === '') return null;
  return { sign: m[1] === '-' ? '-' : '', int, frac };
}

const GROUPER = new Intl.NumberFormat('en-US', { useGrouping: true, maximumFractionDigits: 0 });

/**
 * Formats a decimal string as money: thousands separators on the integer
 * part, at least two fraction digits, the sign kept. The digits themselves
 * are never converted to a float; grouping is applied to the integer part
 * as a BigInt. Anything that is not a decimal string comes back unchanged.
 */
export function formatMoney(s) {
  const d = splitDecimal(s);
  if (!d) return String(s ?? '');
  const grouped = GROUPER.format(BigInt(d.int));
  const frac = d.frac.length < 2 ? d.frac.padEnd(2, '0') : d.frac;
  return `${d.sign}${grouped}.${frac}`;
}

/** Adds decimal strings exactly. Non-decimal inputs count as zero. */
export function sumDecimals(values) {
  let scale = 2;
  const parts = [];
  for (const v of values) {
    const d = splitDecimal(v);
    if (!d) continue;
    scale = Math.max(scale, d.frac.length);
    parts.push(d);
  }
  let total = 0n;
  for (const d of parts) {
    const digits = d.int + d.frac.padEnd(scale, '0');
    const n = BigInt(digits);
    total += d.sign === '-' ? -n : n;
  }
  return bigIntToDecimal(total, scale);
}

function bigIntToDecimal(n, scale) {
  const neg = n < 0n;
  let digits = (neg ? -n : n).toString().padStart(scale + 1, '0');
  const int = digits.slice(0, digits.length - scale);
  const frac = digits.slice(digits.length - scale);
  return `${neg ? '-' : ''}${int}${scale > 0 ? '.' + frac : ''}`;
}

/** Numeric comparison of two decimal strings; non-decimals sort last. */
export function compareDecimals(a, b) {
  const da = splitDecimal(a);
  const db = splitDecimal(b);
  if (!da && !db) return String(a).localeCompare(String(b));
  if (!da) return 1;
  if (!db) return -1;
  const scale = Math.max(da.frac.length, db.frac.length);
  const na = BigInt(da.int + da.frac.padEnd(scale, '0')) * (da.sign === '-' ? -1n : 1n);
  const nb = BigInt(db.int + db.frac.padEnd(scale, '0')) * (db.sign === '-' ? -1n : 1n);
  return na < nb ? -1 : na > nb ? 1 : 0;
}

/** `YYYYMMDD` -> `YYYY-MM-DD`; ISO timestamps are cut to the date; else unchanged. */
export function formatDate(s) {
  const t = String(s ?? '').trim();
  if (/^\d{8}$/.test(t)) return `${t.slice(0, 4)}-${t.slice(4, 6)}-${t.slice(6, 8)}`;
  if (/^\d{4}-\d{2}-\d{2}T/.test(t)) return t.slice(0, 10);
  return t;
}

/** `contributor_last_name` -> `Contributor last name`. */
export function labelize(name) {
  const s = String(name ?? '').replaceAll('_', ' ');
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/** Thousands-grouped integer count. */
export function formatCount(n) {
  return GROUPER.format(n);
}

/** Bytes -> "12.3 KB" style. */
export function formatBytes(n) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/** Whether a field spec describes a money column. */
export function isAmountSpec(spec) {
  return spec?.kind === 'amount';
}

/** Whether a field spec describes a digits-only column (dates included). */
export function isNumericSpec(spec) {
  return spec?.kind === 'numeric';
}
