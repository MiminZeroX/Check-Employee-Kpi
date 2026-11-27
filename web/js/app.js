import { session, post, get, del, connectLive, disconnectLive, onLive, live } from './api.js';
import { esc, icon, toast, showError, clearCache, ROLES } from './ui.js';


// The 22 admin menus from the project spec, grouped. `roles` = who may open it (admin always can).
export const MENU = [
  { group: 'ภาพรวม', items: [
    { path: 'dashboard', label: 'Executive Dashboard', icon: 'dashboard', perm: 'dashboard.view', page: ['dashboard', 'dashboard'] },
    { path: 'live', label: 'Live CCTV Monitoring', icon: 'cctv', perm: 'live.view', page: ['live', 'live'] },
    { path: 'floor', label: 'Office Floor Plan', icon: 'floor', perm: 'live.view', page: ['live', 'floor'] },
  ] },
  { group: 'กล้องและ AI', items: [
    { path: 'cameras', label: 'Camera Management', icon: 'camera', perm: 'cameras.manage', page: ['admin', 'cameras'] },
    { path: 'ai', label: 'AI Detection Management', icon: 'ai', perm: 'ai.manage', page: ['admin', 'ai'] },
    { path: 'devices', label: 'Device Integration', icon: 'device', perm: 'devices.view', page: ['admin', 'devices'] },
  ] },
  { group: 'บุคลากร', items: [
    { path: 'employees', label: 'Employee Management', icon: 'users', perm: 'employees.view', page: ['people', 'employees'] },
    { path: 'attendance', label: 'Attendance Management', icon: 'clock', perm: 'attendance.view', page: ['people', 'attendance'] },
    { path: 'departments', label: 'Department Management', icon: 'building', perm: 'employees.view', page: ['people', 'departments'] },
  ] },
  { group: 'วิเคราะห์และผลงาน', items: [
    { path: 'analytics', label: 'Activity Analytics', icon: 'chart', perm: 'analytics.view', page: ['analytics', 'analytics'] },
    { path: 'tasks', label: 'Task & Project', icon: 'tasks', perm: 'tasks.view', page: ['work', 'tasks'] },
    { path: 'crm', label: 'CRM Management', icon: 'crm', perm: 'crm.view', page: ['work', 'crm'] },
    { path: 'kpi', label: 'KPI & Performance', icon: 'kpi', perm: 'kpi.view', page: ['work', 'kpi'] },
    { path: 'wellness', label: 'Health & Wellness', icon: 'heart', perm: 'wellness.view', page: ['analytics', 'wellness'] },
    { path: 'reports', label: 'Report Center', icon: 'report', perm: 'reports.view', page: ['analytics', 'reports'] },
    { path: 'notifications', label: 'Notification Center', icon: 'bell', perm: 'notifications.view', page: ['analytics', 'notifications'] },
  ] },
  { group: 'ระบบ', items: [
    { path: 'users', label: 'User & Role', icon: 'lock', perm: 'users.manage', page: ['admin', 'users'] },
    { path: 'audit', label: 'Audit Logs', icon: 'log', perm: 'audit.view', page: ['admin', 'audit'] },
    { path: 'privacy', label: 'Privacy & Data', icon: 'shield', perm: 'privacy.review', page: ['admin', 'privacy'] },
    { path: 'system-settings', label: 'System Settings', icon: 'gear', perm: 'settings.manage', page: ['admin', 'settings'] },
    { path: 'system', label: 'Server & Resources', icon: 'server', perm: 'system.view', page: ['admin', 'system'] },
    { path: 'backup', label: 'Backup & Restore', icon: 'backup', perm: 'backup.manage', page: ['admin', 'backup'] },
  ] },
];

const EMPLOYEE_MENU = [{ group: 'ของฉัน', items: [
  { path: 'me', label: 'ข้อมูลของฉัน', icon: 'me', self: true, page: ['me', 'me'] },
  { path: 'my-tasks', label: 'งานของฉัน', icon: 'tasks', self: true, page: ['me', 'myTasks'] },
] }];

const root = document.getElementById('root');
let cleanup = null;
let navSeq = 0;
let collapsed = new Set();
try { collapsed = new Set(JSON.parse(localStorage.getItem('so-nav') || '[]')); } catch { /* ignore */ }

/** A menu item is visible when the user holds its permission; "self" pages need a linked employee record. */
export function allowed(i) {
  if (i.self) return !!session.user?.employee_id;
  return session.can(...[].concat(i.perm));
}

function menuFor() {
  const staff = MENU.map((g) => ({ ...g, items: g.items.filter(allowed) })).filter((g) => g.items.length);
  const mine = EMPLOYEE_MENU.map((g) => ({ ...g, items: g.items.filter(allowed) })).filter((g) => g.items.length);
  return [...mine, ...staff];
}

/** First page this user may open. */
function homePath() {
  const first = menuFor()[0]?.items[0];
  return first ? `#${first.path}` : '#me';
}

function allItems() { return [...MENU, ...EMPLOYEE_MENU].flatMap((g) => g.items); }

// auto (follow the device) -> light -> dark -> auto
const THEMES = { auto: 'ตามเครื่อง (อัตโนมัติ)', light: 'โหมดสว่าง', dark: 'โหมดมืด' };
function currentTheme() { return document.documentElement.dataset.theme || 'auto'; }
function themeTitle() { return `ธีม: ${THEMES[currentTheme()]} · คลิกเพื่อเปลี่ยน`; }
function themeIcon() { return currentTheme() === 'auto' ? 'auto' : currentTheme() === 'light' ? 'sun' : 'moon'; }
function toggleTheme(e) {
  const next = { auto: 'light', light: 'dark', dark: 'auto' }[currentTheme()];
  if (next === 'auto') delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = next;
  try { if (next === 'auto') localStorage.removeItem('so-theme'); else localStorage.setItem('so-theme', next); } catch { /* ignore */ }
  const btn = e?.currentTarget;
  if (btn) { btn.innerHTML = icon(themeIcon()); btn.title = themeTitle(); btn.setAttribute('aria-label', themeTitle()); }
  toast(THEMES[next]);
}

function renderLogin() {
  disconnectLive();
  root.innerHTML = `<div class="login"><div class="panel">
    <div class="brand" style="padding:0"><div class="brand-mark">SO</div><div><b>Smart Office</b><span>Monitoring & Workforce</span></div></div>
    <h1>เข้าสู่ระบบ</h1><div class="muted small">${esc(session.config.org_name || '')}</div>
    <form>
      <div class="field"><label for="u">ชื่อผู้ใช้</label><input class="input" id="u" name="u" autocomplete="username" required></div>
      <div class="field"><label for="p">รหัสผ่าน</label><input class="input" id="p" name="p" type="password" autocomplete="current-password" required></div>
      <button class="btn primary" style="justify-content:center;height:40px">เข้าสู่ระบบ</button>
    </form>
    ${session.config.monitoring_notice ? `<p class="notice" style="margin-top:18px">${icon('shield').replace('<svg', '<svg style="width:14px;height:14px;vertical-align:-2px;margin-right:4px"')}${esc(session.config.monitoring_notice)}</p>` : ''}
  </div></div>`;
  const form = root.querySelector('form');
  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    try {
      const r = await post('/auth/login', { username: form.u.value, password: form.p.value });
      session.save(r.token, r.user);
      const target = homePath();
      if (location.hash === target) route(); else location.hash = target;
    } catch (err) { showError(err); }
  });
  form.u.focus();
}

function renderShell() {
  const groups = menuFor();
  root.innerHTML = `<div class="app">
    <aside class="sidebar">
      <div class="brand"><div class="brand-mark">SO</div><div><b>Smart Office</b><span>${esc(session.config.org_name || 'Monitoring & Workforce')}</span></div></div>
      <nav class="nav">${groups.map((g) => `<div class="nav-group ${collapsed.has(g.group) ? 'collapsed' : ''}" data-group="${esc(g.group)}">
        <button type="button">${esc(g.group)} ${icon('chev')}</button>
        <div class="nav-items">${g.items.map((i) => `<a href="#${i.path}" data-path="${i.path}">${icon(i.icon)}<span>${esc(i.label)}</span>${i.path === 'privacy' ? '<span class="badge warn hidden" data-badge="corrections"></span>' : ''}</a>`).join('')}</div></div>`).join('')}
      </nav>
      <div class="sidebar-foot">v${esc(session.config.version || '')} · ${esc(ROLES[session.user.role] || session.user.role)}</div>
    </aside>
    <div class="main">
      <header class="topbar">
        <button class="btn icon ghost menu-btn" data-menu aria-label="เมนู">${icon('menu')}</button>
        <h1 data-title></h1><span class="spacer"></span>
        <span class="row small muted hide-sm" data-conn ${session.can('dashboard.view', 'live.view') ? '' : 'hidden'}><span class="status-dot"></span><span>Live</span></span>
        <button class="btn icon ghost" data-theme-toggle title="${themeTitle()}" aria-label="${themeTitle()}">${icon(themeIcon())}</button>
        <span class="small hide-sm">${esc(session.user.name)}</span>
        <button class="btn icon ghost" data-pw title="เปลี่ยนรหัสผ่าน">${icon('key')}</button>
        <button class="btn icon ghost" data-logout title="ออกจากระบบ">${icon('logout')}</button>
      </header>
      <main class="content" id="page"></main>
    </div></div>`;
  const app = root.querySelector('.app');
  root.querySelector('[data-menu]').addEventListener('click', () => app.classList.toggle('nav-open'));
  root.querySelector('[data-theme-toggle]').addEventListener('click', toggleTheme);
  root.querySelector('[data-logout]').addEventListener('click', () => {
    del('/video/session').catch(() => {}); // drop the video cookie too
    session.clear(); disconnectLive(); location.hash = '#login'; route();
  });
  root.querySelector('[data-pw]').addEventListener('click', changePassword);
  root.querySelectorAll('.nav-group > button').forEach((b) => b.addEventListener('click', () => {
    const g = b.parentElement; g.classList.toggle('collapsed');
    const name = g.dataset.group;
    if (g.classList.contains('collapsed')) collapsed.add(name); else collapsed.delete(name);
    try { localStorage.setItem('so-nav', JSON.stringify([...collapsed])); } catch { /* ignore */ }
  }));
  root.querySelector('.nav').addEventListener('click', (e) => { if (e.target.closest('a')) app.classList.remove('nav-open'); });
  const conn = root.querySelector('[data-conn] .status-dot');
  const setConn = () => { conn.className = `status-dot ${live.connected ? 'on' : 'off'}`; };
  setConn();
  onLive((m) => {
    if (m.type === 'connection') setConn();
    if (m.type === 'wellness') toast(`แจ้งเตือนสุขภาพ: พนักงานนั่งต่อเนื่อง ${m.minutes} นาที`, 'warn');
    if (m.type === 'camera' && m.status === 'offline') toast(`กล้อง #${m.camera_id} ออฟไลน์`, 'err');
    if (m.type === 'correction') refreshBadges();
  });
  connectLive();
  refreshBadges();
}

async function refreshBadges() {
  const b = root.querySelector('[data-badge="corrections"]');
  if (!b) return;
  try {
    const rows = await get('/corrections?status=pending');
    b.textContent = rows.length; b.classList.toggle('hidden', !rows.length);
  } catch { /* no permission */ }
}

async function changePassword() {
  const { formModal } = await import('./ui.js');
  formModal({
    title: 'เปลี่ยนรหัสผ่าน',
    fields: [
      { key: 'current', label: 'รหัสผ่านปัจจุบัน', type: 'password', required: true, full: true },
      { key: 'new', label: 'รหัสผ่านใหม่ (อย่างน้อย 8 ตัวอักษร)', type: 'password', required: true, full: true },
    ],
    onSubmit: async (v) => {
      const r = await post('/auth/password', v);
      if (r.token) session.save(r.token, session.user); // other sessions are signed out
      toast('เปลี่ยนรหัสผ่านเรียบร้อย อุปกรณ์อื่นถูกออกจากระบบแล้ว');
    },
  });
}

async function route() {
  const path = (location.hash.replace(/^#\/?/, '') || '').split('?')[0];
  if (!session.token) { renderLogin(); return; }
  if (path === 'login' || !path) { location.hash = homePath(); return; }
  if (!root.querySelector('.app')) renderShell();
  const item = allItems().find((i) => i.path === path);
  const page = document.getElementById('page');
  if (typeof cleanup === 'function') { try { cleanup(); } catch { /* ignore */ } }
  cleanup = null;
  clearCache();
  root.querySelectorAll('.nav a').forEach((a) => a.classList.toggle('active', a.dataset.path === path));
  if (!item || !allowed(item)) {
    page.innerHTML = '<div class="empty">ไม่พบหน้านี้ หรือไม่มีสิทธิ์เข้าถึง</div>';
    return;
  }
  root.querySelector('[data-title]').textContent = [...MENU, ...EMPLOYEE_MENU].find((g) => g.items.includes(item))?.group || '';
  document.title = `${item.label} · Smart Office`;
  page.innerHTML = '<div class="empty">กำลังโหลด...</div>';
  const seq = ++navSeq;
  try {
    const mod = await import(`./pages/${item.page[0]}.js`);
    const c = await mod[item.page[1]](page);
    // a newer navigation started while this page was loading: discard this one
    if (seq !== navSeq) { if (typeof c === 'function') c(); return; }
    cleanup = c;
  } catch (e) {
    if (seq !== navSeq) return;
    showError(e);
    page.innerHTML = `<div class="empty">${esc(e.message)}</div>`;
  }
}

(async () => {
  try { session.config = await get('/public/config'); } catch { /* offline */ }
  if (session.token) {
    try { session.save(session.token, await get('/auth/me')); } catch { /* token expired: login page */ }
  }
  window.addEventListener('hashchange', route);
  route();
})();
