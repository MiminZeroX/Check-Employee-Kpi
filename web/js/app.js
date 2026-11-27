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
    { path: 'integrations', label: 'Integrations / Webhook', icon: 'link', perm: 'webhooks.manage', page: ['business', 'integrations'] },
  ] },
  { group: 'บุคลากร', items: [
    { path: 'employees', label: 'Employee Management', icon: 'users', perm: 'employees.view', page: ['people', 'employees'] },
    { path: 'attendance', label: 'Attendance Management', icon: 'clock', perm: 'attendance.view', page: ['people', 'attendance'] },
    { path: 'departments', label: 'Department Management', icon: 'building', perm: 'employees.view', page: ['people', 'departments'] },
    { path: 'org-chart', label: 'ผังองค์กร', icon: 'org', perm: 'employees.view', page: ['hr', 'orgChart'] },
    { path: 'approvals', label: 'คำขอและการอนุมัติ', icon: 'check', perm: ['requests.approve', 'attendance.view'], page: ['hr', 'approvals'] },
    { path: 'shifts', label: 'กะ / วันหยุด / การลา', icon: 'calendar', perm: 'hr.manage', page: ['hr', 'shifts'] },
  ] },
  { group: 'วิเคราะห์และผลงาน', items: [
    { path: 'analytics', label: 'Activity Analytics', icon: 'chart', perm: 'analytics.view', page: ['analytics', 'analytics'] },
    { path: 'tasks', label: 'Task Management', icon: 'tasks', perm: 'tasks.view', page: ['work', 'tasks'] },
    { path: 'projects', label: 'Project Management', icon: 'folder', perm: ['projects.view'], page: ['business', 'projects'] },
    { path: 'crm', label: 'CRM Management', icon: 'crm', perm: 'crm.view', page: ['work', 'crm'] },
    { path: 'kpi', label: 'KPI & Performance', icon: 'kpi', perm: 'kpi.view', page: ['work', 'kpi'] },
    { path: 'wellness', label: 'Health & Wellness', icon: 'heart', perm: 'wellness.view', page: ['analytics', 'wellness'] },
    { path: 'reports', label: 'Report Center', icon: 'report', perm: 'reports.view', page: ['analytics', 'reports'] },
    { path: 'notifications', label: 'Notification Center', icon: 'bell', perm: 'notifications.view', page: ['analytics', 'notifications'] },
    { path: 'documents', label: 'Document Management', icon: 'report', perm: 'documents.view', page: ['business', 'documents'] },
  ] },
  { group: 'ERP', items: [
    { path: 'payroll', label: 'เงินเดือน', icon: 'money', perm: ['payroll.view', 'payroll.manage'], page: ['erp', 'payroll'] },
    { path: 'expenses', label: 'ใบเบิกค่าใช้จ่าย', icon: 'receipt', perm: ['expenses.approve', 'expenses.manage'], page: ['erp', 'expenses'] },
    { path: 'accounting', label: 'บัญชี', icon: 'book', perm: ['accounting.view', 'accounting.manage'], page: ['erp', 'accounting'] },
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
  { path: 'my-payslips', label: 'สลิปเงินเดือน', icon: 'money', self: true, page: ['erp', 'myPayslips'] },
  { path: 'my-expenses', label: 'เบิกค่าใช้จ่าย', icon: 'receipt', self: true, page: ['erp', 'myExpenses'] },
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

// Day & Night switch. With no explicit choice the page follows the device; choosing the
// same mode as the device goes back to "follow the device".
const sysDark = window.matchMedia('(prefers-color-scheme: dark)');
function isNight() {
  const t = document.documentElement.dataset.theme;
  return t ? t === 'dark' : sysDark.matches;
}
function dayNightLabel() {
  const auto = !document.documentElement.dataset.theme;
  return `${isNight() ? 'Night (โหมดกลางคืน)' : 'Day (โหมดกลางวัน)'}${auto ? ' · ตามเครื่อง' : ''} · คลิกเพื่อสลับ`;
}
export function dayNightSwitch() {
  const stars = [[12, 7], [20, 18], [9, 21], [27, 10], [31, 22]].map(([x, y], i) => `<i class="dn-star" style="left:${x}px;top:${y}px;animation-delay:${i * 0.4}s"></i>`).join('');
  return `<button type="button" class="daynight" data-theme-toggle role="switch" aria-checked="${isNight()}" aria-label="${dayNightLabel()}" title="${dayNightLabel()}">
    <span class="dn-stars" aria-hidden="true">${stars}</span>
    <span class="dn-clouds" aria-hidden="true"><i></i><i></i><i></i></span>
    <span class="dn-knob" aria-hidden="true"><i></i><i></i><i></i></span></button>`;
}
function syncSwitches() {
  document.querySelectorAll('[data-theme-toggle]').forEach((b) => {
    b.setAttribute('aria-checked', String(isNight()));
    b.title = dayNightLabel(); b.setAttribute('aria-label', dayNightLabel());
  });
}
function toggleTheme() {
  const wantNight = !isNight();
  if (wantNight === sysDark.matches) delete document.documentElement.dataset.theme; // matches the device: follow it again
  else document.documentElement.dataset.theme = wantNight ? 'dark' : 'light';
  try {
    if (document.documentElement.dataset.theme) localStorage.setItem('so-theme', document.documentElement.dataset.theme);
    else localStorage.removeItem('so-theme');
  } catch { /* ignore */ }
  syncSwitches();
}
sysDark.addEventListener('change', syncSwitches);
document.addEventListener('click', (e) => { if (e.target.closest('[data-theme-toggle]')) toggleTheme(); });

function renderLogin() {
  disconnectLive();
  root.innerHTML = `<div class="login"><div class="login-theme">${dayNightSwitch()}</div><div class="panel">
    <div class="brand" style="padding:0"><div class="brand-mark">SO</div><div><b>Smart Office</b><span>Monitoring & Workforce</span></div></div>
    <h1>เข้าสู่ระบบ</h1><div class="muted small">${esc(session.config.org_name || '')}</div>
    <form data-pw-form>
      <div class="field"><label for="u">ชื่อผู้ใช้</label><input class="input" id="u" name="u" autocomplete="username" required></div>
      <div class="field"><label for="p">รหัสผ่าน</label><input class="input" id="p" name="p" type="password" autocomplete="current-password" required></div>
      <button class="btn primary" style="justify-content:center;height:40px">เข้าสู่ระบบ</button>
    </form>
    <form data-mfa-form hidden>
      <p class="small muted" style="margin:0">เปิดแอปยืนยันตัวตน (เช่น Google Authenticator) แล้วใส่รหัส 6 หลัก</p>
      <div class="field"><label for="otp">รหัสยืนยัน</label><input class="input" id="otp" name="otp" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9 ]{6,7}" maxlength="7" required></div>
      <button class="btn primary" style="justify-content:center;height:40px">ยืนยัน</button>
      <button type="button" class="btn ghost" data-back style="justify-content:center">ย้อนกลับ</button>
    </form>
    <div data-sso hidden><div class="login-or small muted">หรือ</div><a class="btn" href="/api/auth/oidc/start" style="justify-content:center;height:40px;width:100%">${icon('shield')}<span data-sso-label></span></a></div>
    ${session.config.monitoring_notice ? `<p class="notice" style="margin-top:18px">${icon('shield').replace('<svg', '<svg style="width:14px;height:14px;vertical-align:-2px;margin-right:4px"')}${esc(session.config.monitoring_notice)}</p>` : ''}
  </div></div>`;
  const form = root.querySelector('[data-pw-form]');
  const mfaForm = root.querySelector('[data-mfa-form]');
  let mfaToken = null;
  const done = (r) => {
    session.save(r.token, r.user);
    const target = homePath();
    if (location.hash === target) route(); else location.hash = target;
  };
  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    try {
      const r = await post('/auth/login', { username: form.u.value, password: form.p.value });
      if (r.mfa_required) {
        mfaToken = r.mfa_token;
        form.hidden = true; mfaForm.hidden = false; mfaForm.otp.value = ''; mfaForm.otp.focus();
        return;
      }
      done(r);
    } catch (err) { showError(err); }
  });
  mfaForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    try { done(await post('/auth/mfa/verify', { mfa_token: mfaToken, code: mfaForm.otp.value })); } catch (err) {
      showError(err);
      if (/เข้าสู่ระบบใหม่/.test(err.message)) mfaForm.querySelector('[data-back]').click();
    }
  });
  mfaForm.querySelector('[data-back]').addEventListener('click', () => {
    mfaToken = null; mfaForm.hidden = true; form.hidden = false; form.p.value = ''; form.p.focus();
  });
  get('/auth/oidc').then((o) => {
    if (!o.enabled) return;
    root.querySelector('[data-sso-label]').textContent = o.label;
    root.querySelector('[data-sso]').hidden = false;
  }).catch(() => {});
  if (ssoError) { toast(ssoError, 'err'); ssoError = ''; }
  form.u.focus();
}

let ssoError = '';

// ---------------------------------------------------------------- browser notifications (employees)
// Polls the employee's own notifications (wellness alerts, approvals, tickets) and shows new ones
// as system notifications. Needs the user's permission and HTTPS (or localhost).
let notifTimer = null;
const NOTIF_KEY = () => `so-notif-seen-${session.user?.id}`;
const notifSupported = () => 'Notification' in window && window.isSecureContext;
async function pollNotifications(first = false) {
  if (!session.token || !session.user?.employee_id) return;
  let rows;
  try { rows = await get('/me/notifications'); } catch { return; }
  let seen = 0;
  try { seen = Number(localStorage.getItem(NOTIF_KEY())) || 0; } catch { /* ignore */ }
  const maxId = rows.reduce((m, n) => Math.max(m, n.id), seen);
  try { localStorage.setItem(NOTIF_KEY(), String(maxId)); } catch { /* ignore */ }
  if (first && !seen) return; // first run on this browser: do not replay old items
  const fresh = rows.filter((n) => n.id > seen && !n.is_read).reverse();
  for (const n of fresh.slice(-3)) {
    if (notifSupported() && Notification.permission === 'granted' && document.visibilityState !== 'visible') {
      try {
        const sys = new Notification(n.title, { body: n.body || '', tag: `so-${n.id}` });
        sys.onclick = () => { window.focus(); location.hash = '#me'; sys.close(); };
      } catch { /* some browsers only allow notifications from a service worker */ }
    } else {
      toast(`${n.title}${n.body ? ` · ${n.body}` : ''}`);
    }
  }
}
function startNotifier() {
  clearInterval(notifTimer); notifTimer = null;
  if (!session.user?.employee_id) return;
  pollNotifications(true);
  notifTimer = setInterval(() => pollNotifications(), 60000);
}

// Single sign-on returns to "/#sso=<one-time code>" (or "#sso_error=...").
async function finishSso() {
  const h = location.hash;
  if (h.startsWith('#sso_error=')) {
    ssoError = decodeURIComponent(h.slice(11)); history.replaceState(null, '', '/#login');
  } else if (h.startsWith('#sso=')) {
    const code = h.slice(5);
    history.replaceState(null, '', '/#login');
    try { const r = await post('/auth/oidc/exchange', { code }); session.save(r.token, r.user); history.replaceState(null, '', `/#${homePath().slice(1)}`); } catch { ssoError = 'เข้าสู่ระบบด้วยบัญชีองค์กรไม่สำเร็จ กรุณาลองใหม่'; }
  }
}

async function accountSecurity() {
  const { modal, formModal, confirmBox } = await import('./ui.js');
  const st = await get('/auth/mfa');
  const m = modal({
    title: 'ความปลอดภัยของบัญชี',
    body: `<div class="stack">
      <div class="row"><div><b>การยืนยันตัวตนสองขั้นตอน (MFA)</b><div class="small muted">ใช้รหัส 6 หลักจากแอปบนโทรศัพท์ทุกครั้งที่เข้าสู่ระบบด้วยรหัสผ่าน</div></div><span class="spacer"></span>
        ${st.enabled ? '<span class="badge ok">เปิดอยู่</span>' : '<span class="badge">ปิด</span>'}</div>
      <div class="row">${st.enabled ? '<button class="btn danger" data-off>ปิด MFA</button>' : '<button class="btn primary" data-on>เปิด MFA</button>'}
        <button class="btn" data-pw2>${icon('key')}เปลี่ยนรหัสผ่าน</button>
        <button class="btn" data-out>ออกจากระบบทุกอุปกรณ์</button></div>
      ${session.user.employee_id ? `<div class="row" style="border-top:1px solid var(--line);padding-top:14px"><div><b>แจ้งเตือนบนเบราว์เซอร์</b><div class="small muted">${notifSupported()
        ? 'แจ้งเมื่อนั่งนานเกินไป คำขอได้รับอนุมัติ หรือได้รับมอบหมายงาน แม้หน้านี้อยู่เบื้องหลัง'
        : 'ต้องเปิดผ่าน HTTPS และใช้เบราว์เซอร์ที่รองรับ'}</div></div><span class="spacer"></span>
        ${!notifSupported() ? '<span class="badge">ไม่รองรับ</span>' : Notification.permission === 'granted' ? '<span class="badge ok">เปิดอยู่</span>'
          : Notification.permission === 'denied' ? '<span class="badge warn">ถูกบล็อกในเบราว์เซอร์</span>' : '<button class="btn" data-notif>เปิดแจ้งเตือน</button>'}</div>` : ''}</div>`,
  });
  m.body.querySelector('[data-notif]')?.addEventListener('click', async () => {
    const p = await Notification.requestPermission();
    m.close();
    if (p === 'granted') { toast('เปิดการแจ้งเตือนแล้ว'); try { new Notification('Smart Office', { body: 'การแจ้งเตือนพร้อมใช้งาน', tag: 'so-test' }); } catch { /* ignore */ } }
    else toast('ไม่ได้รับอนุญาตให้แจ้งเตือน', 'err');
  });
  m.body.querySelector('[data-pw2]').addEventListener('click', () => { m.close(); changePassword(); });
  m.body.querySelector('[data-out]').addEventListener('click', async () => {
    if (!(await confirmBox('ออกจากระบบทุกอุปกรณ์ รวมถึงเครื่องนี้?', 'ออกทั้งหมด'))) return;
    await post('/auth/logout-all', {}); m.close(); session.clear(); disconnectLive(); location.hash = '#login'; route();
  });
  m.body.querySelector('[data-off]')?.addEventListener('click', () => { m.close(); formModal({
    title: 'ปิดการยืนยันตัวตนสองขั้นตอน',
    fields: [
      { key: 'password', label: 'รหัสผ่าน', type: 'password', required: true, full: true },
      { key: 'code', label: 'รหัส 6 หลักจากแอป', required: true, full: true },
    ],
    submitLabel: 'ปิด MFA',
    onSubmit: async (v) => { await post('/auth/mfa/disable', v); toast('ปิด MFA แล้ว'); },
  }); });
  m.body.querySelector('[data-on]')?.addEventListener('click', () => { m.close(); formModal({
    title: 'เปิดการยืนยันตัวตนสองขั้นตอน',
    intro: 'ยืนยันรหัสผ่านก่อนตั้งค่า',
    fields: [{ key: 'password', label: 'รหัสผ่าน', type: 'password', required: true, full: true }],
    submitLabel: 'ถัดไป',
    onSubmit: async (v) => {
      const s = await post('/auth/mfa/setup', v);
      setTimeout(() => formModal({
        title: 'สแกน QR ด้วยแอปยืนยันตัวตน',
        intro: `<div class="mfa-setup"><img src="${s.qr}" alt="QR code สำหรับแอปยืนยันตัวตน" width="200" height="200">
          <div class="small muted">สแกนด้วย Google Authenticator, Microsoft Authenticator หรือแอปที่รองรับ TOTP<br>หรือพิมพ์รหัสนี้เอง: <code class="mono">${esc(s.secret.replace(/(.{4})/g, '$1 ').trim())}</code></div></div>`,
        fields: [{ key: 'code', label: 'รหัส 6 หลักที่แอปแสดง', required: true, full: true }],
        submitLabel: 'เปิดใช้งาน',
        onSubmit: async (c) => { await post('/auth/mfa/enable', c); toast('เปิด MFA เรียบร้อย ครั้งหน้าจะต้องใส่รหัสจากแอปด้วย'); },
      }), 0);
    },
  }); });
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
        ${dayNightSwitch()}
        <span class="small hide-sm">${esc(session.user.name)}</span>
        <button class="btn icon ghost" data-pw title="ความปลอดภัยของบัญชี (รหัสผ่าน / MFA)" aria-label="ความปลอดภัยของบัญชี">${icon('key')}</button>
        <button class="btn icon ghost" data-logout title="ออกจากระบบ">${icon('logout')}</button>
      </header>
      <main class="content" id="page"></main>
    </div></div>`;
  const app = root.querySelector('.app');
  root.querySelector('[data-menu]').addEventListener('click', () => app.classList.toggle('nav-open'));
  startNotifier();
  root.querySelector('[data-logout]').addEventListener('click', () => {
    clearInterval(notifTimer); notifTimer = null;
    del('/video/session').catch(() => {}); // drop the video cookie too
    session.clear(); disconnectLive(); location.hash = '#login'; route();
  });
  root.querySelector('[data-pw]').addEventListener('click', () => accountSecurity().catch(showError));
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
  await finishSso();
  if (session.token) {
    try { session.save(session.token, await get('/auth/me')); } catch { /* token expired: login page */ }
  }
  window.addEventListener('hashchange', route);
  route();
})();
