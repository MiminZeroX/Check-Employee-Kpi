// API client, session and live WebSocket.

const store = {
  get(k) { try { return localStorage.getItem(k); } catch { return null; } },
  set(k, v) { try { v == null ? localStorage.removeItem(k) : localStorage.setItem(k, v); } catch { /* ignore */ } },
};

export const session = {
  token: store.get('so-token'),
  user: JSON.parse(store.get('so-user') || 'null'),
  config: {},
  save(token, user) {
    this.token = token; this.user = user;
    store.set('so-token', token); store.set('so-user', JSON.stringify(user));
  },
  clear() {
    this.token = null; this.user = null;
    store.set('so-token', null); store.set('so-user', null);
  },
  is(...roles) { return !!this.user && (this.user.role === 'admin' || roles.includes(this.user.role)); },
};

export class ApiError extends Error {
  constructor(status, message) { super(message); this.status = status; }
}

export async function api(path, { method = 'GET', body, raw = false } = {}) {
  const headers = {};
  if (session.token) headers.Authorization = `Bearer ${session.token}`;
  if (body !== undefined) headers['Content-Type'] = 'application/json';
  const res = await fetch(`/api${path}`, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  if (res.status === 401 && path !== '/auth/login') {
    session.clear();
    location.hash = '#login';
    throw new ApiError(401, 'กรุณาเข้าสู่ระบบอีกครั้ง');
  }
  if (!res.ok) {
    let msg = res.statusText;
    try { msg = (await res.json()).error || msg; } catch { /* not json */ }
    if (res.status === 403) msg = 'ไม่มีสิทธิ์เข้าถึงส่วนนี้';
    throw new ApiError(res.status, msg);
  }
  if (raw) return res;
  const ct = res.headers.get('content-type') || '';
  return ct.includes('json') ? res.json() : res.text();
}

export const get = (p) => api(p);
export const post = (p, body = {}) => api(p, { method: 'POST', body });
export const put = (p, body = {}) => api(p, { method: 'PUT', body });
export const del = (p) => api(p, { method: 'DELETE' });

export function qs(obj) {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(obj)) if (v !== undefined && v !== null && v !== '') p.set(k, v);
  const s = p.toString();
  return s ? `?${s}` : '';
}

/** Downloads an authenticated file response. */
export async function download(path, fallbackName) {
  const res = await api(path, { raw: true });
  const cd = res.headers.get('content-disposition') || '';
  const name = (cd.match(/filename="([^"]+)"/) || [])[1] || fallbackName;
  const blob = await res.blob();
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = name;
  document.body.appendChild(a);
  a.click();
  setTimeout(() => { URL.revokeObjectURL(a.href); a.remove(); }, 1000);
}

/** Opens an authenticated HTML page (e.g. the A4 report) in a new tab. */
export async function openHtml(path) {
  // open the tab synchronously (inside the click) so popup blockers allow it
  const win = window.open('', '_blank');
  if (!win) throw new Error('เบราว์เซอร์บล็อกหน้าต่างใหม่ กรุณาอนุญาต Pop-up สำหรับเว็บไซต์นี้');
  win.document.write('<p style="font-family:sans-serif;padding:24px">กำลังสร้างรายงาน...</p>');
  try {
    const html = await api(path);
    const url = URL.createObjectURL(new Blob([html], { type: 'text/html;charset=utf-8' }));
    win.location.href = url;
  } catch (e) { win.close(); throw e; }
}

// ---------------------------------------------------------------- live updates

const listeners = new Set();
export const live = { desks: new Map(), connected: false };
let ws = null;
let retry = 0;

export function onLive(fn) { listeners.add(fn); return () => listeners.delete(fn); }

function emit(msg) { for (const fn of listeners) { try { fn(msg); } catch (e) { console.error(e); } } }

export function connectLive() {
  if (!session.token || session.user?.role === 'employee' || ws) return;
  const proto = location.protocol === 'https:' ? 'wss' : 'ws';
  ws = new WebSocket(`${proto}://${location.host}/api/ws?token=${encodeURIComponent(session.token)}`);
  ws.onopen = () => { retry = 0; live.connected = true; emit({ type: 'connection', connected: true }); };
  ws.onmessage = (ev) => {
    let msg;
    try { msg = JSON.parse(ev.data); } catch { return; }
    if (msg.type === 'live') for (const d of msg.desks) live.desks.set(d.workstation_id, d);
    emit(msg);
  };
  ws.onclose = () => {
    ws = null; live.connected = false; emit({ type: 'connection', connected: false });
    if (!session.token) return;
    retry = Math.min(retry + 1, 6);
    setTimeout(connectLive, 1000 * 2 ** retry);
  };
}

export function disconnectLive() { if (ws) { const w = ws; ws = null; w.onclose = null; w.close(); } }
