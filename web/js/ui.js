// Small UI toolkit: escaping, formatting, modal, toast, forms, tables, charts.
import { get, post, put, del, session } from './api.js';

export const esc = (v) => String(v ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

export const STATES = {
  focused: { label: 'มองหน้าจอ', long: 'มองหน้าจอ / ทำงานที่โต๊ะ', color: 'var(--s-focused)' },
  present: { label: 'อยู่ที่โต๊ะ', long: 'อยู่ที่โต๊ะ (ไม่เห็นใบหน้า)', color: 'var(--s-present)' },
  phone: { label: 'ใช้มือถือ', long: 'ใช้โทรศัพท์มือถือ', color: 'var(--s-phone)' },
  looking_away: { label: 'มองทางอื่น', long: 'มองไปทางอื่น', color: 'var(--s-looking_away)' },
  meeting: { label: 'ประชุม/พูดคุย', long: 'ประชุม / พูดคุยกับเพื่อนร่วมงาน', color: 'var(--s-meeting)' },
  away: { label: 'ไม่อยู่โต๊ะ', long: 'ไม่อยู่ที่โต๊ะ', color: 'var(--s-away)' },
  unknown: { label: 'ไม่ระบุ', long: 'ไม่สามารถระบุได้', color: 'var(--s-unknown)' },
};
export const STATE_KEYS = Object.keys(STATES);

export const ROLES = { admin: 'ผู้ดูแลระบบ', hr: 'ฝ่ายบุคคล', manager: 'หัวหน้างาน', viewer: 'ผู้ชม', employee: 'พนักงาน' };

// ---------------------------------------------------------------- formatting

export function fmtDur(sec) {
  sec = Math.max(0, Math.round(sec || 0));
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  if (h) return `${h} ชม. ${m} น.`;
  if (m) return `${m} นาที`;
  return `${sec} วิ.`;
}
export const fmtHours = (sec) => `${((sec || 0) / 3600).toFixed(1)} ชม.`;
// All times are shown in the organisation's time zone (server config), not the viewer's.
const offMs = () => (session.config.tz_offset_min ?? 420) * 60000;
const shifted = (iso) => new Date(new Date(iso).getTime() + offMs());
export function fmtTime(iso) {
  if (!iso) return '-';
  return shifted(iso).toLocaleTimeString('th-TH', { hour: '2-digit', minute: '2-digit', timeZone: 'UTC' });
}
export function fmtDateTime(iso) {
  if (!iso) return '-';
  return shifted(iso).toLocaleString('th-TH', { dateStyle: 'short', timeStyle: 'short', timeZone: 'UTC' });
}
export function fmtDate(d) {
  if (!d) return '-';
  const x = typeof d === 'string' && d.length === 10 ? new Date(`${d}T00:00:00Z`) : shifted(d);
  return x.toLocaleDateString('th-TH', { day: 'numeric', month: 'short', year: '2-digit', timeZone: 'UTC' });
}
export function ago(iso) {
  if (!iso) return '-';
  const s = (Date.now() - new Date(iso).getTime()) / 1000;
  if (s < 60) return 'เมื่อสักครู่';
  if (s < 3600) return `${Math.floor(s / 60)} นาทีที่แล้ว`;
  if (s < 86400) return `${Math.floor(s / 3600)} ชม.ที่แล้ว`;
  return fmtDateTime(iso);
}
export const fmtBytes = (b) => (b > 1 << 30 ? `${(b / (1 << 30)).toFixed(1)} GB` : b > 1 << 20 ? `${(b / (1 << 20)).toFixed(1)} MB` : `${Math.round((b || 0) / 1024)} KB`);

/** Calendar dates in the organisation's time zone. */
const orgNow = () => new Date(Date.now() + offMs());
const ymd = (d) => d.toISOString().slice(0, 10);
export const today = () => ymd(orgNow());
export function daysAgo(n) { const d = orgNow(); d.setUTCDate(d.getUTCDate() - n); return ymd(d); }
export function monthStart(offset = 0) { const d = orgNow(); d.setUTCDate(1); d.setUTCMonth(d.getUTCMonth() + offset); return ymd(d); }
export function monthEnd(offset = 0) { const d = orgNow(); d.setUTCDate(1); d.setUTCMonth(d.getUTCMonth() + offset + 1, 0); return ymd(d); }
export function nowStamp() { return shifted(new Date()).toLocaleString('th-TH', { timeZone: 'UTC' }); }

export function stateBadge(s) {
  const st = STATES[s] || STATES.unknown;
  return `<span class="badge"><span class="dot" style="color:${st.color}"></span>${esc(st.label)}</span>`;
}

// ---------------------------------------------------------------- icons

const P = {
  dashboard: 'M3 13h8V3H3zm10 8h8V11h-8zM3 21h8v-6H3zm10-18v6h8V3z',
  cctv: 'M4 7l12-4 2 6-12 4zM6 13l1 3h4M16 9l3 3-2 2',
  camera: 'M3 7h4l2-3h6l2 3h4v13H3zM12 17a4 4 0 100-8 4 4 0 000 8z',
  ai: 'M12 2v4M12 18v4M4.9 4.9l2.8 2.8M16.3 16.3l2.8 2.8M2 12h4M18 12h4M4.9 19.1l2.8-2.8M16.3 7.7l2.8-2.8M12 15a3 3 0 100-6 3 3 0 000 6z',
  floor: 'M3 3h18v18H3zM3 12h8M11 3v18M15 12h6',
  users: 'M16 11a4 4 0 10-8 0M4 21a8 8 0 0116 0M17 3a4 4 0 010 8',
  clock: 'M12 22a10 10 0 100-20 10 10 0 000 20zM12 6v6l4 2',
  building: 'M4 21V3h10v18M14 8h6v13M8 7h2M8 11h2M8 15h2M17 12h1M17 16h1M2 21h20',
  chart: 'M3 3v18h18M7 15l4-4 3 3 5-6',
  tasks: 'M9 11l2 2 4-4M4 4h16v16H4z',
  crm: 'M20 21v-2a4 4 0 00-4-4H8a4 4 0 00-4 4v2M12 11a4 4 0 100-8 4 4 0 000 8z',
  kpi: 'M12 20V10M18 20V4M6 20v-4',
  heart: 'M20.8 4.6a5.5 5.5 0 00-7.8 0L12 5.7l-1-1.1a5.5 5.5 0 00-7.8 7.8l1 1.1L12 21l7.8-7.5 1-1.1a5.5 5.5 0 000-7.8z',
  report: 'M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8zM14 2v6h6M8 13h8M8 17h5',
  bell: 'M18 8a6 6 0 00-12 0c0 7-3 9-3 9h18s-3-2-3-9M13.7 21a2 2 0 01-3.4 0',
  device: 'M7 2h10v20H7zM11 18h2',
  shield: 'M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z',
  log: 'M4 4h16M4 9h16M4 14h10M4 19h7',
  lock: 'M5 11h14v10H5zM8 11V7a4 4 0 018 0v4',
  gear: 'M12 15a3 3 0 100-6 3 3 0 000 6zM19.4 15a1.7 1.7 0 00.3 1.8l.1.1a2 2 0 11-2.8 2.8l-.1-.1a1.7 1.7 0 00-1.8-.3 1.7 1.7 0 00-1 1.5V21a2 2 0 11-4 0v-.1a1.7 1.7 0 00-1.1-1.5 1.7 1.7 0 00-1.8.3l-.1.1a2 2 0 11-2.8-2.8l.1-.1a1.7 1.7 0 00.3-1.8 1.7 1.7 0 00-1.5-1H3a2 2 0 110-4h.1a1.7 1.7 0 001.5-1.1 1.7 1.7 0 00-.3-1.8l-.1-.1a2 2 0 112.8-2.8l.1.1a1.7 1.7 0 001.8.3H9a1.7 1.7 0 001-1.5V3a2 2 0 114 0v.1a1.7 1.7 0 001 1.5 1.7 1.7 0 001.8-.3l.1-.1a2 2 0 112.8 2.8l-.1.1a1.7 1.7 0 00-.3 1.8V9a1.7 1.7 0 001.5 1H21a2 2 0 110 4h-.1a1.7 1.7 0 00-1.5 1z',
  server: 'M3 4h18v6H3zM3 14h18v6H3zM7 7h.01M7 17h.01',
  backup: 'M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4M7 10l5 5 5-5M12 15V3',
  me: 'M12 12a5 5 0 100-10 5 5 0 000 10zM3 22a9 9 0 0118 0',
  plus: 'M12 5v14M5 12h14',
  edit: 'M11 4H4v16h16v-7M18.5 2.5a2.1 2.1 0 013 3L12 15l-4 1 1-4z',
  trash: 'M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6',
  x: 'M18 6L6 18M6 6l12 12',
  menu: 'M3 6h18M3 12h18M3 18h18',
  sun: 'M12 17a5 5 0 100-10 5 5 0 000 10zM12 1v2M12 21v2M4.2 4.2l1.4 1.4M18.4 18.4l1.4 1.4M1 12h2M21 12h2M4.2 19.8l1.4-1.4M18.4 5.6l1.4-1.4',
  moon: 'M21 12.8A9 9 0 1111.2 3 7 7 0 0021 12.8z',
  auto: 'M12 21a9 9 0 100-18 9 9 0 000 18zM12 3v18',
  logout: 'M9 21H5a2 2 0 01-2-2V5a2 2 0 012-2h4M16 17l5-5-5-5M21 12H9',
  chev: 'M6 9l6 6 6-6',
  expand: 'M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7',
  download: 'M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4M7 10l5 5 5-5M12 15V3',
  print: 'M6 9V2h12v7M6 18H4a2 2 0 01-2-2v-5a2 2 0 012-2h16a2 2 0 012 2v5a2 2 0 01-2 2h-2M6 14h12v8H6z',
  refresh: 'M23 4v6h-6M1 20v-6h6M3.5 9a9 9 0 0114.9-3.4L23 10M1 14l4.6 4.4A9 9 0 0020.5 15',
  key: 'M21 2l-2 2m-7.6 7.6a5.5 5.5 0 11-7.8 7.8 5.5 5.5 0 017.8-7.8zM15.5 7.5l3 3L22 7l-3-3',
  org: 'M12 3v5M5 13V9h14v4M12 8v5M3 13h4v4H3zM10 13h4v4h-4zM17 13h4v4h-4z',
  check: 'M20 6L9 17l-5-5',
  folder: 'M3 6h6l2 2h10v11H3z',
  link: 'M10 14a5 5 0 007 0l3-3a5 5 0 00-7-7l-1 1M14 10a5 5 0 00-7 0l-3 3a5 5 0 007 7l1-1',
  calendar: 'M3 5h18v16H3zM3 10h18M8 3v4M16 3v4',
  money: 'M2 6h20v12H2zM12 9a3 3 0 100 6 3 3 0 000-6M6 12h.01M18 12h.01',
  receipt: 'M5 3h14v18l-3-2-2 2-2-2-2 2-2-2-3 2zM9 8h6M9 12h6',
  book: 'M4 4h11a3 3 0 013 3v13H7a3 3 0 01-3-3zM4 17a3 3 0 013-3h11',
  cart: 'M3 4h2l2.5 11h11L21 7H6.5M9 20a1 1 0 100-2 1 1 0 000 2M18 20a1 1 0 100-2 1 1 0 000 2',
  box: 'M3 7l9-4 9 4-9 4zM3 7v10l9 4 9-4V7M12 11v10',
  asset: 'M4 20V9l8-5 8 5v11M9 20v-6h6v6',
};
export function icon(name) {
  return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="${P[name] || P.dashboard}"/></svg>`;
}

// ---------------------------------------------------------------- toast & modal

export function toast(msg, kind = '') {
  const el = document.createElement('div');
  el.className = `toast ${kind}`;
  el.textContent = msg;
  document.getElementById('toasts').appendChild(el);
  setTimeout(() => el.remove(), kind === 'err' ? 6000 : 3500);
}

export function showError(e) { console.error(e); toast(e.message || String(e), 'err'); }

export function modal({ title, body, foot = '', wide = false, onClose }) {
  const back = document.createElement('div');
  back.className = 'modal-back';
  back.innerHTML = `<div class="modal ${wide ? 'wide' : ''}" role="dialog" aria-modal="true">
    <div class="modal-head"><h2>${esc(title)}</h2><span class="spacer"></span>
    <button class="btn icon ghost" data-close aria-label="ปิด">${icon('x')}</button></div>
    <div class="modal-body"></div>${foot ? `<div class="modal-foot">${foot}</div>` : ''}</div>`;
  const bodyEl = back.querySelector('.modal-body');
  if (typeof body === 'string') bodyEl.innerHTML = body; else if (body) bodyEl.appendChild(body);
  const close = () => { back.remove(); document.removeEventListener('keydown', onKey); onClose?.(); };
  const onKey = (e) => { if (e.key === 'Escape') close(); };
  back.addEventListener('mousedown', (e) => { if (e.target === back) close(); });
  back.querySelectorAll('[data-close]').forEach((b) => b.addEventListener('click', close));
  document.addEventListener('keydown', onKey);
  document.body.appendChild(back);
  return { el: back, body: bodyEl, close };
}

export function confirmBox(message, okLabel = 'ยืนยัน', danger = true) {
  return new Promise((resolve) => {
    const m = modal({
      title: 'ยืนยันการทำรายการ',
      body: `<p style="margin:0">${esc(message)}</p>`,
      foot: `<button class="btn" data-close>ยกเลิก</button><button class="btn ${danger ? 'danger' : 'primary'}" data-ok>${esc(okLabel)}</button>`,
      onClose: () => resolve(false),
    });
    m.el.querySelector('[data-ok]').addEventListener('click', () => { m.el.remove(); resolve(true); });
  });
}

// ---------------------------------------------------------------- forms

async function resolveOptions(f) {
  if (!f.options) return [];
  return typeof f.options === 'function' ? f.options() : f.options;
}

export async function renderFields(fields, values = {}) {
  const parts = [];
  for (const f of fields) {
    const v = values[f.key] ?? f.default ?? '';
    const id = `f-${f.key}`;
    const req = f.required ? 'required' : '';
    const cls = f.full || f.type === 'textarea' ? 'field full' : 'field';
    let input;
    switch (f.type) {
      case 'select': {
        const opts = await resolveOptions(f);
        const blank = f.required ? '' : `<option value="">${esc(f.placeholder || '- ไม่ระบุ -')}</option>`;
        input = `<select class="input" id="${id}" name="${f.key}" ${req}>${blank}${opts
          .map((o) => `<option value="${esc(o.value)}" ${String(o.value) === String(v) ? 'selected' : ''}>${esc(o.label)}</option>`)
          .join('')}</select>`;
        break;
      }
      case 'checkbox':
        parts.push(`<div class="${cls}"><label class="check"><input type="checkbox" name="${f.key}" ${v === true || v === 1 || (v === '' && f.default) ? 'checked' : ''}> ${esc(f.label)}</label>${f.help ? `<span class="help">${esc(f.help)}</span>` : ''}</div>`);
        continue;
      case 'textarea':
        input = `<textarea class="input" id="${id}" name="${f.key}" ${req}>${esc(v)}</textarea>`;
        break;
      default: {
        const val = f.type === 'time' && typeof v === 'string' ? v.slice(0, 5) : v;
        input = `<input class="input" id="${id}" name="${f.key}" type="${f.type || 'text'}" value="${esc(val)}" ${req}
          ${f.step ? `step="${f.step}"` : ''} ${f.min !== undefined ? `min="${f.min}"` : ''} ${f.max !== undefined ? `max="${f.max}"` : ''}
          ${f.placeholder ? `placeholder="${esc(f.placeholder)}"` : ''} autocomplete="off">`;
      }
    }
    parts.push(`<div class="${cls}"><label for="${id}">${esc(f.label)}${f.required ? ' *' : ''}</label>${input}${f.help ? `<span class="help">${esc(f.help)}</span>` : ''}</div>`);
  }
  return `<div class="form-grid">${parts.join('')}</div>`;
}

export function readFields(form, fields) {
  const out = {};
  for (const f of fields) {
    const el = form.elements[f.key];
    if (!el) continue;
    if (f.type === 'checkbox') out[f.key] = el.checked;
    else if (f.type === 'number') out[f.key] = el.value === '' ? null : Number(el.value);
    else if (f.type === 'select' && f.numeric) out[f.key] = el.value === '' ? null : Number(el.value);
    else out[f.key] = el.value === '' ? (f.emptyAsNull === false ? '' : null) : el.value;
  }
  return out;
}

/** Opens a modal form. `onSubmit(values)` may throw to keep the dialog open. */
export async function formModal({ title, fields, values = {}, submitLabel = 'บันทึก', onSubmit, wide = false, intro = '' }) {
  const form = document.createElement('form');
  form.innerHTML = `${intro}${await renderFields(fields, values)}`;
  const m = modal({
    title, body: form, wide,
    foot: `<button type="button" class="btn" data-close>ยกเลิก</button><button type="button" class="btn primary" data-submit>${esc(submitLabel)}</button>`,
  });
  const submit = async () => {
    if (!form.reportValidity()) return;
    const btn = m.el.querySelector('[data-submit]');
    btn.disabled = true;
    try {
      await onSubmit(readFields(form, fields), form);
      m.close();
    } catch (e) { showError(e); } finally { btn.disabled = false; }
  };
  m.el.querySelector('[data-submit]').addEventListener('click', submit);
  form.addEventListener('submit', (e) => { e.preventDefault(); submit(); });
  form.querySelector('input,select,textarea')?.focus();
  return m;
}

// ---------------------------------------------------------------- tables & CRUD

export function table(columns, rows, { empty = 'ไม่มีข้อมูล', actions } = {}) {
  if (!rows.length) return `<div class="empty">${esc(empty)}</div>`;
  const head = columns.map((c) => `<th class="${c.align === 'r' ? 'r' : ''} ${c.hideSm ? 'hide-sm' : ''}">${esc(c.label)}</th>`).join('');
  const body = rows.map((r, i) => `<tr data-i="${i}">${columns
    .map((c) => `<td class="${c.align === 'r' ? 'r' : ''} ${c.hideSm ? 'hide-sm' : ''}">${c.render ? c.render(r) : esc(r[c.key])}</td>`)
    .join('')}${actions ? `<td class="actions">${actions(r)}</td>` : ''}</tr>`).join('');
  return `<div class="tbl-wrap"><table class="tbl"><thead><tr>${head}${actions ? '<th></th>' : ''}</tr></thead><tbody>${body}</tbody></table></div>`;
}

/**
 * Generic list + create/edit/delete page.
 * opts: { title, desc, endpoint, columns, fields, canEdit, canDelete, deleteLabel, search, extraActions, onRow, filters }
 */
export async function crudPage(el, opts) {
  const canEdit = opts.canEdit ?? true;
  const canDelete = opts.canDelete ?? canEdit;
  el.innerHTML = `
    <div class="page-head"><div><h1>${esc(opts.title)}</h1>${opts.desc ? `<p>${esc(opts.desc)}</p>` : ''}</div><span class="spacer"></span>
      ${opts.headExtra || ''}
      ${(opts.canAdd ?? canEdit) && opts.fields ? `<button class="btn primary" data-add>${icon('plus')} ${esc(opts.addLabel || 'เพิ่มข้อมูล')}</button>` : ''}</div>
    ${opts.before || ''}
    <div class="panel"><div class="toolbar"><input class="input" data-search placeholder="ค้นหา..." style="width:240px">
      <span class="muted small" data-count></span><span class="spacer"></span></div><div data-list></div></div>`;
  const list = el.querySelector('[data-list]');
  let rows = [];
  const draw = () => {
    const term = el.querySelector('[data-search]').value.trim().toLowerCase();
    const shown = term ? rows.filter((r) => JSON.stringify(r).toLowerCase().includes(term)) : rows;
    el.querySelector('[data-count]').textContent = `${shown.length} รายการ`;
    list.innerHTML = table(opts.columns, shown, {
      actions: (r) => `${opts.extraActions ? opts.extraActions(r) : ''}${canEdit && opts.fields ? `<button class="btn sm ghost" data-edit="${r.id}" title="แก้ไข">${icon('edit')}</button>` : ''}${canDelete ? `<button class="btn sm ghost danger" data-del="${r.id}" title="ลบ">${icon('trash')}</button>` : ''}`,
    });
  };
  const load = async () => {
    try { rows = await get(opts.endpoint + (opts.query ? opts.query() : '')); draw(); } catch (e) { showError(e); list.innerHTML = `<div class="empty">${esc(e.message)}</div>`; }
  };
  const editForm = (row) => formModal({
    title: row ? `แก้ไข: ${opts.title}` : (opts.addLabel || `เพิ่ม: ${opts.title}`),
    fields: opts.fields, values: row ? (opts.toForm ? opts.toForm(row) : row) : {}, wide: opts.wideForm,
    onSubmit: async (v) => {
      const body = opts.toBody ? opts.toBody(v, row) : v;
      const r = row ? await put(`${opts.endpoint}/${row.id}`, body) : await post(opts.endpoint, body);
      toast('บันทึกเรียบร้อย');
      opts.afterSave?.(r, !row);
      load();
    },
  });
  el.querySelector('[data-search]').addEventListener('input', draw);
  el.querySelector('[data-add]')?.addEventListener('click', () => editForm(null));
  list.addEventListener('click', async (e) => {
    const b = e.target.closest('button');
    if (!b) return;
    const id = b.dataset.edit || b.dataset.del || b.dataset.act;
    const row = rows.find((r) => String(r.id) === String(id));
    if (b.dataset.edit) editForm(row);
    else if (b.dataset.del) {
      if (!(await confirmBox(opts.deleteLabel || 'ต้องการลบรายการนี้หรือไม่?', 'ลบ'))) return;
      try { await del(`${opts.endpoint}/${id}`); toast('ลบเรียบร้อย'); load(); } catch (err) { showError(err); }
    } else if (b.dataset.act && opts.onAction) opts.onAction(b.dataset.action, row, load);
  });
  await load();
  return { reload: load, rows: () => rows };
}

// ---------------------------------------------------------------- option loaders (cached per page view)

let cache = {};
export function clearCache() { cache = {}; }

/** Fills a <select data-branch> with the branches; hides it when there is only one branch. */
export async function fillBranches(sel) {
  if (!sel) return;
  let list = [];
  try { list = await opt.branches(); } catch { /* no permission */ }
  sel.insertAdjacentHTML('beforeend', list.map((b) => `<option value="${b.value}">${esc(b.label)}</option>`).join(''));
  sel.hidden = list.length < 2;
}
export const BRANCH_SELECT = '<select class="input" data-branch style="width:150px" hidden aria-label="สาขา"><option value="">ทุกสาขา</option></select>';
async function cached(key, fn) { if (!cache[key]) cache[key] = fn(); return cache[key]; }
export const opt = {
  employees: () => cached('emp', async () => (await get('/employees?status=active')).map((e) => ({ value: e.id, label: `${e.employee_code} · ${e.first_name} ${e.last_name}` }))),
  departments: () => cached('dept', async () => (await get('/departments')).map((d) => ({ value: d.id, label: d.name }))),
  branches: () => cached('branch', async () => (await get('/branches')).map((b) => ({ value: b.id, label: b.name }))),
  cameras: () => cached('cam', async () => (await get('/cameras')).map((c) => ({ value: c.id, label: c.name }))),
  customers: () => cached('cust', async () => (await get('/customers')).map((c) => ({ value: c.id, label: c.name }))),
  states: () => STATE_KEYS.map((k) => ({ value: k, label: STATES[k].long })),
};

// ---------------------------------------------------------------- charts

export function legend(keys = STATE_KEYS) {
  return `<div class="legend">${keys.map((k) => `<span><i style="background:${STATES[k].color}"></i>${esc(STATES[k].long)}</span>`).join('')}</div>`;
}

export function stateBar(secs) {
  const total = STATE_KEYS.reduce((a, k) => a + (secs[k] || 0), 0);
  if (!total) return '<div class="hbar"></div>';
  return `<div class="hbar" title="${STATE_KEYS.map((k) => `${STATES[k].label}: ${fmtDur(secs[k] || 0)}`).join('\n')}">${STATE_KEYS
    .map((k) => (secs[k] ? `<span style="width:${(secs[k] * 100) / total}%;background:${STATES[k].color}"></span>` : ''))
    .join('')}</div>`;
}

export function donut(secs, size = 150) {
  const total = STATE_KEYS.reduce((a, k) => a + (secs[k] || 0), 0);
  const r = 52; const c = 2 * Math.PI * r;
  let off = 0;
  const arcs = total ? STATE_KEYS.map((k) => {
    const v = secs[k] || 0;
    if (!v) return '';
    const len = (v / total) * c;
    const s = `<circle r="${r}" cx="70" cy="70" fill="none" stroke="${STATES[k].color}" stroke-width="16" stroke-dasharray="${len} ${c - len}" stroke-dashoffset="${-off}"><title>${STATES[k].label} ${fmtDur(v)}</title></circle>`;
    off += len;
    return s;
  }).join('') : `<circle r="${r}" cx="70" cy="70" fill="none" stroke="var(--panel-2)" stroke-width="16"/>`;
  const focus = total ? Math.round(((secs.focused || 0) * 100) / total) : 0;
  return `<svg viewBox="0 0 140 140" width="${size}" height="${size}" role="img" aria-label="สัดส่วนกิจกรรม">
    <g transform="rotate(-90 70 70)">${arcs}</g>
    <text x="70" y="68" text-anchor="middle" fill="var(--text)" font-size="22" font-weight="600">${focus}%</text>
    <text x="70" y="86" text-anchor="middle" fill="var(--text-3)" font-size="10">มองหน้าจอ</text></svg>`;
}

/** Gantt-like day timeline. data = { start, end, employees:[{name, employee_code, segments, scans}] } */
export function timeline(data, { fromHour = 7, toHour = 20 } = {}) {
  if (!data.employees.length) return '<div class="empty">ไม่มีข้อมูลพนักงาน</div>';
  const day0 = new Date(data.start).getTime();
  const t0 = day0 + fromHour * 3600e3;
  const t1 = day0 + toHour * 3600e3;
  const pct = (t) => Math.max(0, Math.min(100, ((new Date(t).getTime() - t0) * 100) / (t1 - t0)));
  const ticks = [];
  for (let h = fromHour; h <= toHour; h += 1) ticks.push(`<span style="left:${((h - fromHour) * 100) / (toHour - fromHour)}%">${String(h).padStart(2, '0')}</span>`);
  const rows = data.employees.map((e) => {
    const segs = e.segments.map((s) => {
      const l = pct(s.start); const w = pct(s.end) - l;
      if (w <= 0) return '';
      return `<div class="tl-seg" style="left:${l}%;width:${w}%;background:${(STATES[s.state] || STATES.unknown).color}" title="${esc(STATES[s.state]?.long || s.state)} ${fmtTime(s.start)}-${fmtTime(s.end)}"></div>`;
    }).join('');
    const scans = e.scans.map((s) => `<div class="tl-scan" style="left:${pct(s.time)}%" title="สแกน${s.direction === 'out' ? 'ออก' : 'เข้า'} ${fmtTime(s.time)} (${esc(s.method)})"></div>`).join('');
    return `<div class="tl-row"><div class="tl-name">${esc(e.name)}<small>${esc(e.employee_code)}${e.department ? ` · ${esc(e.department)}` : ''}</small></div><div class="tl-track">${segs}${scans}</div></div>`;
  }).join('');
  return `<div class="timeline"><div class="tl-axis"><div></div><div>${ticks.join('')}</div></div>${rows}</div>
    <div class="row" style="margin-top:10px">${legend()}<span class="legend"><span><i style="background:var(--text);width:2px"></i>เวลาสแกนนิ้ว/หน้า</span></span></div>`;
}

export function rangePicker(el, onChange, { from = daysAgo(6), to = today() } = {}) {
  el.innerHTML = `<div class="row">
    <select class="input" data-preset style="width:150px">
      <option value="7">7 วันล่าสุด</option><option value="this-month">เดือนนี้</option>
      <option value="last-month">เดือนที่แล้ว</option><option value="30">30 วันล่าสุด</option>
      <option value="90">ไตรมาส (90 วัน)</option><option value="365">รายปี (365 วัน)</option><option value="custom">กำหนดเอง</option></select>
    <input class="input" type="date" data-from value="${from}" style="width:150px">
    <span class="muted">ถึง</span><input class="input" type="date" data-to value="${to}" style="width:150px"></div>`;
  const f = el.querySelector('[data-from]'); const t = el.querySelector('[data-to]');
  const presetFor = { [daysAgo(6)]: '7', [daysAgo(29)]: '30', [daysAgo(89)]: '90', [daysAgo(364)]: '365' };
  el.querySelector('[data-preset]').value = to === today() && presetFor[from] ? presetFor[from] : 'custom';
  el.querySelector('[data-preset]').addEventListener('change', (e) => {
    const v = e.target.value;
    if (v === 'this-month') { f.value = monthStart(0); t.value = today(); }
    else if (v === 'last-month') { f.value = monthStart(-1); t.value = monthEnd(-1); }
    else if (v !== 'custom') { f.value = daysAgo(Number(v) - 1); t.value = today(); }
    onChange(f.value, t.value);
  });
  [f, t].forEach((x) => x.addEventListener('change', () => { el.querySelector('[data-preset]').value = 'custom'; onChange(f.value, t.value); }));
  return { get: () => ({ from: f.value, to: t.value }) };
}

/** Weekday x hour heatmap. cells[w][h] = { people, focus_pct }. */
export function heatmapGrid(cells, { fromHour = 6, toHour = 21 } = {}) {
  const days = ['จ.', 'อ.', 'พ.', 'พฤ.', 'ศ.', 'ส.', 'อา.'];
  const max = Math.max(0.1, ...cells.flat().map((c) => c.people));
  const hours = [];
  for (let h = fromHour; h <= toHour; h += 1) hours.push(h);
  return `<div class="heat" style="grid-template-columns: 34px repeat(${hours.length}, minmax(22px, 1fr))">
    <span></span>${hours.map((h) => `<span class="heat-h">${String(h).padStart(2, '0')}</span>`).join('')}
    ${cells.map((row, w) => `<span class="heat-d">${days[w]}</span>${hours.map((h) => {
      const c = row[h];
      const a = c.people / max;
      return `<span class="heat-c" style="background:color-mix(in srgb, var(--accent) ${Math.round(a * 85)}%, var(--panel-2))" title="${days[w]} ${String(h).padStart(2, '0')}:00 · เฉลี่ย ${c.people} คนอยู่ที่โต๊ะ${c.focus_pct != null ? ` · มองจอ ${c.focus_pct}%` : ''}">${c.people >= max * 0.6 ? c.people : ''}</span>`;
    }).join('')}`).join('')}</div>
    <div class="legend" style="margin-top:8px"><span>จำนวนคนเฉลี่ยที่อยู่ที่โต๊ะในแต่ละชั่วโมง</span><span><i style="background:var(--panel-2)"></i>น้อย</span><span><i style="background:var(--accent)"></i>มาก</span></div>`;
}
