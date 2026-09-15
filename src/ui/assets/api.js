// Fetch wrapper: sends the API key, turns error bodies into ApiError, and
// lets the shell handle a 401 by asking for a key and retrying once.

const KEY_STORAGE = 'hardmoney.apiKey';

export class ApiError extends Error {
  constructor(status, message) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

export function getApiKey() {
  return sessionStorage.getItem(KEY_STORAGE) || '';
}

export function setApiKey(key) {
  if (key) sessionStorage.setItem(KEY_STORAGE, key);
  else sessionStorage.removeItem(KEY_STORAGE);
}

let unauthorizedHandler = null;

/**
 * Registers the function called on a 401. It should resolve to true once
 * a new key has been stored (the request is then retried once) or false
 * to give up.
 */
export function onUnauthorized(handler) {
  unauthorizedHandler = handler;
}

async function errorMessage(res) {
  const text = await res.text().catch(() => '');
  try {
    const json = JSON.parse(text);
    if (json && typeof json.error === 'string') return json.error;
  } catch {
    // Not JSON: axum's extractor rejections are plain text.
  }
  return text || `${res.status} ${res.statusText}`;
}

/**
 * Performs a request. `body` may be a string, ArrayBuffer, Blob, or a
 * plain object (sent as JSON). Returns the parsed JSON body, or the raw
 * Response when `raw` is set.
 */
export async function request(path, { method = 'GET', body, headers = {}, raw = false } = {}, retried = false) {
  const h = new Headers(headers);
  const key = getApiKey();
  if (key) h.set('X-Api-Key', key);
  let payload = body;
  if (body !== undefined && body !== null && typeof body === 'object' && !(body instanceof ArrayBuffer) && !(body instanceof Blob) && !ArrayBuffer.isView(body)) {
    payload = JSON.stringify(body);
    if (!h.has('Content-Type')) h.set('Content-Type', 'application/json');
  }
  let res;
  try {
    res = await fetch(path, { method, body: payload, headers: h });
  } catch (e) {
    throw new ApiError(0, `Could not reach the server: ${e.message}`);
  }
  if (res.status === 401 && !retried && unauthorizedHandler) {
    const again = await unauthorizedHandler();
    if (again) return request(path, { method, body, headers, raw }, true);
  }
  if (!res.ok) throw new ApiError(res.status, await errorMessage(res));
  return raw ? res : res.json();
}

function query(params) {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(params || {})) {
    if (v === undefined || v === null || v === '') continue;
    q.set(k, String(v));
  }
  const s = q.toString();
  return s ? `?${s}` : '';
}

export const api = {
  get: (path, params) => request(`${path}${query(params)}`),
  postBytes: (path, bytes) => request(path, {
    method: 'POST',
    body: bytes,
    headers: { 'Content-Type': 'application/octet-stream' },
  }),
  postJson: (path, obj) => request(path, { method: 'POST', body: obj }),
  postJsonRaw: (path, obj) => request(path, { method: 'POST', body: obj, raw: true }),
};

const specCache = new Map();

/** Layout + FEC field specs for a table, cached per (table, version). */
export async function tableSpec(table, version) {
  const k = `${table}@${version}`;
  if (!specCache.has(k)) {
    specCache.set(k, api.get(`/tools/spec/${encodeURIComponent(table)}`, { version }).then((spec) => {
      const byName = new Map();
      for (const f of spec.fields) byName.set(f.name, f);
      spec.byName = byName;
      return spec;
    }).catch((e) => {
      specCache.delete(k);
      throw e;
    }));
  }
  return specCache.get(k);
}
