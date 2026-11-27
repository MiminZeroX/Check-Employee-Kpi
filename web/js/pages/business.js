import { api, get, post, qs, session, download } from '../api.js';
import {
  esc, icon, crudPage, opt, table, toast, showError, formModal, modal, fmtDate, fmtDateTime, ago, confirmBox,
} from '../ui.js';
import { taskColumns } from './work.js';

const baht = (n) => `฿${Number(n || 0).toLocaleString('th-TH', { maximumFractionDigits: 0 })}`;
const PROJECT_STATUS = { planning: 'วางแผน', active: 'กำลังดำเนินการ', on_hold: 'พักไว้', done: 'เสร็จสิ้น', cancelled: 'ยกเลิก' };
export const STAGES = [
  ['lead', 'ผู้สนใจ'], ['qualified', 'คัดกรองแล้ว'], ['proposal', 'เสนอราคา'], ['negotiation', 'เจรจา'], ['won', 'ปิดการขาย'], ['lost', 'ไม่สำเร็จ'],
];
const PRIORITY = { low: 'ต่ำ', normal: 'ปกติ', high: 'สูง', urgent: 'ด่วนมาก' };
const TICKET_STATUS = { open: 'เปิด', in_progress: 'กำลังดำเนินการ', waiting: 'รอลูกค้า', resolved: 'แก้ไขแล้ว', closed: 'ปิด' };

export function slaBadge(t) {
  if (t.resolved_at) {
    const ok = !t.due_at || new Date(t.resolved_at) <= new Date(t.due_at);
    return `<span class="badge ${ok ? 'ok' : 'warn'}">${ok ? 'ทัน SLA' : 'เกิน SLA'}</span>`;
  }
  if (!t.due_at) return '-';
  const left = (new Date(t.due_at) - Date.now()) / 3600e3;
  if (left < 0) return `<span class="badge bad">เกินกำหนด ${Math.ceil(-left)} ชม.</span>`;
  return `<span class="badge ${left < 4 ? 'warn' : 'info'}">เหลือ ${left < 1 ? `${Math.round(left * 60)} นาที` : `${Math.floor(left)} ชม.`}</span>`;
}

export const ticketColumns = [
  { label: '#', render: (t) => `#${t.id}` },
  { label: 'หัวข้อ', render: (t) => `<b>${esc(t.subject)}</b><div class="small muted">${esc([t.customer_name, t.contact_name].filter(Boolean).join(' · '))}</div>` },
  { key: 'assignee_name', label: 'ผู้รับผิดชอบ', hideSm: true },
  { label: 'ความเร่งด่วน', render: (t) => `<span class="badge ${t.priority === 'urgent' ? 'bad' : t.priority === 'high' ? 'warn' : ''}">${PRIORITY[t.priority]}</span>` },
  { label: 'สถานะ', render: (t) => `<span class="badge ${['resolved', 'closed'].includes(t.status) ? 'ok' : 'info'}">${TICKET_STATUS[t.status]}</span>` },
  { label: 'SLA', render: slaBadge },
  { label: 'เปิดเมื่อ', hideSm: true, render: (t) => ago(t.created_at) },
];

// ---------------------------------------------------------------- projects

export async function projects(el) {
  const page = await crudPage(el, {
    title: 'Project Management', desc: 'โครงการ ความคืบหน้า และงานที่ล่าช้า · คลิก "งาน" เพื่อดูงานในโครงการ', endpoint: '/projects',
    canEdit: session.can('projects.manage'), addLabel: 'สร้างโครงการ',
    columns: [
      { label: 'โครงการ', render: (p) => `<b>${esc(p.name)}</b>${p.code ? ` <span class="muted mono">${esc(p.code)}</span>` : ''}<div class="small muted">${esc(p.customer_name || '')}</div>` },
      { key: 'owner_name', label: 'ผู้รับผิดชอบ', hideSm: true },
      { label: 'สถานะ', render: (p) => `<span class="badge ${p.status === 'active' ? 'info' : p.status === 'done' ? 'ok' : ''}">${PROJECT_STATUS[p.status]}</span>` },
      { label: 'กำหนดเสร็จ', render: (p) => fmtDate(p.due_date) },
      { label: 'ความคืบหน้า', render: (p) => {
        const pct = p.tasks_total ? Math.round((p.tasks_done * 100) / p.tasks_total) : 0;
        return `<div class="meter" style="min-width:110px"><span style="width:${pct}%"></span></div><div class="small muted">${p.tasks_done}/${p.tasks_total} งาน · ${pct}%</div>`;
      } },
      { label: 'ล่าช้า', align: 'r', render: (p) => (p.tasks_overdue ? `<span class="badge bad">${p.tasks_overdue}</span>` : '0') },
      { label: 'งบประมาณ', align: 'r', hideSm: true, render: (p) => (p.budget != null ? baht(p.budget) : '-') },
    ],
    extraActions: (p) => `<button class="btn sm" data-act="${p.id}" data-action="tasks">งาน</button>`,
    onAction: async (action, p) => {
      const tasks = await get(`/tasks?project_id=${p.id}`);
      modal({ title: `งานในโครงการ ${p.name}`, wide: true, body: table(taskColumns(), tasks, { empty: 'ยังไม่มีงาน' }) });
    },
    fields: [
      { key: 'name', label: 'ชื่อโครงการ', required: true }, { key: 'code', label: 'รหัสโครงการ' },
      { key: 'customer_id', label: 'ลูกค้า', type: 'select', options: opt.customers, numeric: true },
      { key: 'owner_employee_id', label: 'ผู้จัดการโครงการ', type: 'select', options: opt.employees, numeric: true },
      { key: 'status', label: 'สถานะ', type: 'select', required: true, default: 'active', options: Object.entries(PROJECT_STATUS).map(([value, label]) => ({ value, label })) },
      { key: 'budget', label: 'งบประมาณ (บาท)', type: 'number', min: 0 },
      { key: 'start_date', label: 'วันเริ่ม', type: 'date' }, { key: 'due_date', label: 'กำหนดเสร็จ', type: 'date' },
      { key: 'description', label: 'รายละเอียด', type: 'textarea' },
    ],
  });
  return page;
}

// ---------------------------------------------------------------- CRM

async function pipeline(body) {
  const canEdit = session.can('crm.manage');
  const render = async () => {
    const [deals, sum] = await Promise.all([get('/deals'), get('/deals/summary')]);
    body.innerHTML = `<div class="page-head"><div><h1>Sales Pipeline</h1><p>มูลค่าคาดการณ์ (ถ่วงความน่าจะเป็น) ${baht(sum.weighted_pipeline)} · อัตราปิดการขาย ${sum.win_rate == null ? '-' : `${sum.win_rate.toFixed(0)}%`}</p></div>
      <span class="spacer"></span>${canEdit ? `<button class="btn primary" data-new>${icon('plus')} เพิ่มดีล</button>` : ''}</div>
      <div class="kanban">${STAGES.map(([k, label], i) => {
        const s = sum.stages.find((x) => x.stage === k);
        return `<section class="kcol" data-stage="${k}" aria-label="${esc(label)}"><header><b>${esc(label)}</b><span class="small muted">${s.count} · ${baht(s.value)}</span></header>
          ${deals.filter((d) => d.stage === k).map((d) => `<article class="kcard" draggable="${canEdit}" data-id="${d.id}">
            <b>${esc(d.title)}</b><div class="small muted">${esc(d.customer_name || '-')}</div>
            <div class="row small" style="justify-content:space-between"><span>${baht(d.value)}</span><span class="muted">${d.probability}%</span></div>
            <div class="small muted">${esc(d.owner_name || '')}${d.expected_close ? ` · ${fmtDate(d.expected_close)}` : ''}</div>
            ${canEdit ? `<div class="row" style="margin-top:6px;gap:4px">${i > 0 ? `<button class="btn sm" data-move="${d.id}" data-to="${STAGES[i - 1][0]}" aria-label="ย้ายกลับ">‹</button>` : ''}
              ${i < STAGES.length - 1 ? `<button class="btn sm" data-move="${d.id}" data-to="${STAGES[i + 1][0]}" aria-label="ย้ายไปขั้นถัดไป">›</button>` : ''}
              <span class="spacer"></span><button class="btn sm ghost" data-edit-deal="${d.id}" aria-label="แก้ไข">${icon('edit')}</button></div>` : ''}
          </article>`).join('') || '<div class="small muted" style="padding:8px">-</div>'}</section>`;
      }).join('')}</div>`;
    const fields = [
      { key: 'title', label: 'ชื่อดีล', required: true, full: true }, { key: 'customer_id', label: 'ลูกค้า', type: 'select', options: opt.customers, numeric: true },
      { key: 'owner_employee_id', label: 'เจ้าของดีล', type: 'select', options: opt.employees, numeric: true },
      { key: 'stage', label: 'ขั้นตอน', type: 'select', required: true, default: 'lead', options: STAGES.map(([value, label]) => ({ value, label })) },
      { key: 'value', label: 'มูลค่า (บาท)', type: 'number', min: 0 }, { key: 'probability', label: 'โอกาสสำเร็จ (%)', type: 'number', min: 0, max: 100, help: 'ว่าง = ตามขั้นตอน' },
      { key: 'expected_close', label: 'คาดว่าจะปิด', type: 'date' }, { key: 'notes', label: 'บันทึก', type: 'textarea' },
    ];
    body.onclick = async (e) => {
      const b = e.target.closest('button'); if (!b) return;
      try {
        if (b.dataset.move) { await post(`/deals/${b.dataset.move}/stage`, { stage: b.dataset.to }); render(); }
        else if (b.dataset.new !== undefined) formModal({ title: 'เพิ่มดีล', fields, onSubmit: async (v) => { await post('/deals', v); toast('เพิ่มดีลแล้ว'); render(); } });
        else if (b.dataset.editDeal) {
          const d = deals.find((x) => String(x.id) === b.dataset.editDeal);
          formModal({
            title: 'แก้ไขดีล', fields, values: d,
            intro: `<div class="row" style="justify-content:flex-end;margin-bottom:8px"><button type="button" class="btn sm danger" data-del-deal>ลบดีล</button></div>`,
            onSubmit: async (v) => { await api(`/deals/${d.id}`, { method: 'PUT', body: v }); toast('บันทึกแล้ว'); render(); },
          }).then((m) => m.el.querySelector('[data-del-deal]').addEventListener('click', async () => {
            if (await confirmBox('ลบดีลนี้?', 'ลบ')) { await api(`/deals/${d.id}`, { method: 'DELETE' }); m.close(); render(); }
          }));
        }
      } catch (err) { showError(err); }
    };
    // drag & drop between columns (buttons above do the same for keyboard / touch users)
    body.querySelectorAll('.kcard[draggable="true"]').forEach((c) => c.addEventListener('dragstart', (e) => e.dataTransfer.setData('text/plain', c.dataset.id)));
    body.querySelectorAll('.kcol').forEach((col) => {
      col.addEventListener('dragover', (e) => { e.preventDefault(); col.classList.add('over'); });
      col.addEventListener('dragleave', () => col.classList.remove('over'));
      col.addEventListener('drop', async (e) => {
        e.preventDefault(); col.classList.remove('over');
        try { await post(`/deals/${e.dataTransfer.getData('text/plain')}/stage`, { stage: col.dataset.stage }); render(); } catch (err) { showError(err); }
      });
    });
  };
  await render();
}

async function tickets(body) {
  await crudPage(body, {
    title: 'Ticket & Service', desc: 'งานบริการลูกค้า กำหนดเวลาตอบสนอง (SLA) ตามความเร่งด่วน ตั้งค่าได้ที่ System Settings', endpoint: '/tickets',
    canEdit: session.can('tickets.manage', 'crm.manage'), canDelete: session.can('tickets.manage'), addLabel: 'เปิด Ticket',
    columns: ticketColumns,
    fields: [
      { key: 'subject', label: 'หัวข้อ', required: true, full: true },
      { key: 'customer_id', label: 'ลูกค้า', type: 'select', options: opt.customers, numeric: true },
      { key: 'contact_id', label: 'ผู้ติดต่อ', type: 'select', options: async () => (await get('/contacts')).map((c) => ({ value: c.id, label: `${c.name} (${c.customer_name})` })), numeric: true },
      { key: 'assignee_employee_id', label: 'ผู้รับผิดชอบ', type: 'select', options: opt.employees, numeric: true },
      { key: 'priority', label: 'ความเร่งด่วน', type: 'select', required: true, default: 'normal', options: Object.entries(PRIORITY).map(([value, label]) => ({ value, label })) },
      { key: 'status', label: 'สถานะ', type: 'select', required: true, default: 'open', options: Object.entries(TICKET_STATUS).map(([value, label]) => ({ value, label })) },
      { key: 'satisfaction', label: 'ความพึงพอใจของลูกค้า (1-5)', type: 'number', min: 1, max: 5 },
      { key: 'description', label: 'รายละเอียด', type: 'textarea' },
    ],
  });
}

async function contacts(body) {
  await crudPage(body, {
    title: 'ผู้ติดต่อ', desc: 'บุคคลติดต่อของลูกค้าแต่ละราย', endpoint: '/contacts', canEdit: session.can('crm.manage'), addLabel: 'เพิ่มผู้ติดต่อ',
    columns: [
      { label: 'ชื่อ', render: (c) => `<b>${esc(c.name)}</b>${c.is_primary ? ' <span class="badge info">หลัก</span>' : ''}<div class="small muted">${esc(c.position || '')}</div>` },
      { key: 'customer_name', label: 'ลูกค้า' }, { key: 'email', label: 'อีเมล' }, { key: 'phone', label: 'โทรศัพท์' },
    ],
    fields: [
      { key: 'customer_id', label: 'ลูกค้า', type: 'select', required: true, options: opt.customers, numeric: true },
      { key: 'name', label: 'ชื่อ', required: true }, { key: 'position', label: 'ตำแหน่ง' },
      { key: 'email', label: 'อีเมล', type: 'email' }, { key: 'phone', label: 'โทรศัพท์' },
      { key: 'is_primary', label: 'ผู้ติดต่อหลัก', type: 'checkbox' }, { key: 'notes', label: 'บันทึก', type: 'textarea' },
    ],
  });
}

export async function crm(el, { customers }) {
  el.innerHTML = `<div class="tabs"><button class="active" data-tab="customers">ลูกค้า</button><button data-tab="contacts">ผู้ติดต่อ</button>
    <button data-tab="pipeline">Sales Pipeline</button><button data-tab="tickets">Ticket</button></div><div data-body></div>`;
  const body = el.querySelector('[data-body]');
  const views = { customers: () => customers(body), contacts: () => contacts(body), pipeline: () => pipeline(body), tickets: () => tickets(body) };
  el.querySelector('.tabs').addEventListener('click', (e) => {
    const b = e.target.closest('[data-tab]'); if (!b) return;
    el.querySelectorAll('.tabs button').forEach((x) => x.classList.toggle('active', x === b));
    body.onclick = null;
    views[b.dataset.tab]().catch(showError);
  });
  await views.customers();
}

// ---------------------------------------------------------------- documents

const ENTITY = { general: 'เอกสารทั่วไป', employee: 'พนักงาน', customer: 'ลูกค้า', project: 'โครงการ', expense: 'ใบเสร็จใบเบิก' };
const size = (b) => (b > 1 << 20 ? `${(b / (1 << 20)).toFixed(1)} MB` : `${Math.ceil(b / 1024)} KB`);

export async function documents(el) {
  el.innerHTML = `<div class="page-head"><div><h1>Document Management</h1><p>เอกสารพนักงาน ลูกค้า และโครงการ · pdf, รูปภาพ, Office, csv, txt, zip ไม่เกิน 20 MB · ทุกการเปิดไฟล์ถูกบันทึกใน Audit Log</p></div>
    <span class="spacer"></span><select class="input" data-entity style="width:170px"><option value="">ทุกประเภท</option>${Object.entries(ENTITY).map(([k, v]) => `<option value="${k}">${v}</option>`).join('')}</select>
    ${session.can('documents.manage') ? `<button class="btn primary" data-upload>${icon('plus')} อัปโหลด</button>` : ''}</div><div class="panel" data-list></div>`;
  const owners = {};
  const ownerName = async (entity, id) => {
    if (!id) return '';
    const key = `${entity}:${id}`;
    if (!(key in owners)) {
      const src = { employee: opt.employees, customer: opt.customers, project: async () => (await get('/projects')).map((p) => ({ value: p.id, label: p.name })) }[entity];
      owners[key] = src ? ((await src()).find((o) => o.value === id)?.label || `#${id}`) : '';
    }
    return owners[key];
  };
  const load = async () => {
    const rows = await get(`/documents${qs({ entity: el.querySelector('[data-entity]').value })}`);
    for (const r of rows) r.owner = await ownerName(r.entity, r.entity_id);
    el.querySelector('[data-list]').innerHTML = table([
      { label: 'เอกสาร', render: (d) => `<b>${esc(d.title)}</b><div class="small muted">${esc(d.file_name)} · ${size(d.size_bytes)}</div>` },
      { label: 'เป็นของ', render: (d) => `${esc(ENTITY[d.entity] || d.entity)}${d.owner ? `: ${esc(d.owner)}` : ''}` },
      { key: 'category', label: 'หมวด', hideSm: true }, { key: 'uploaded_by_name', label: 'อัปโหลดโดย', hideSm: true },
      { label: 'เมื่อ', render: (d) => fmtDateTime(d.created_at) },
    ], rows, {
      empty: 'ยังไม่มีเอกสาร',
      actions: (d) => `<button class="btn sm" data-dl="${d.id}">${icon('download')}</button>${session.can('documents.manage') ? `<button class="btn sm ghost danger" data-del="${d.id}">${icon('trash')}</button>` : ''}`,
    });
    el.querySelector('[data-list]').onclick = async (e) => {
      const b = e.target.closest('button'); if (!b) return;
      try {
        if (b.dataset.dl) await download(`/documents/${b.dataset.dl}/download`, 'document');
        if (b.dataset.del && await confirmBox('ลบเอกสารนี้?', 'ลบ')) { await api(`/documents/${b.dataset.del}`, { method: 'DELETE' }); load(); }
      } catch (err) { showError(err); }
    };
  };
  el.querySelector('[data-entity]').onchange = load;
  el.querySelector('[data-upload]')?.addEventListener('click', async () => {
    const m = await formModal({
      title: 'อัปโหลดเอกสาร',
      intro: '<div class="field full" style="margin-bottom:12px"><label for="f-file">ไฟล์ *</label><input class="input" id="f-file" name="file" type="file" required accept=".pdf,.png,.jpg,.jpeg,.docx,.xlsx,.pptx,.doc,.xls,.csv,.txt,.zip" style="padding-top:6px"></div>',
      fields: [
        { key: 'title', label: 'ชื่อเอกสาร (ว่าง = ชื่อไฟล์)' }, { key: 'category', label: 'หมวด', placeholder: 'สัญญา, ใบรับรองแพทย์ ...' },
        { key: 'entity', label: 'เป็นของ', type: 'select', required: true, default: 'general', options: Object.entries(ENTITY).filter(([k]) => k !== 'expense').map(([value, label]) => ({ value, label })) },
        { key: 'entity_id', label: 'พนักงาน / ลูกค้า / โครงการ', type: 'select', options: [] },
      ],
      submitLabel: 'อัปโหลด',
      onSubmit: async (v, form) => {
        const fd = new FormData();
        const f = form.elements.file.files[0];
        if (!f) throw new Error('กรุณาเลือกไฟล์');
        fd.append('title', v.title || ''); fd.append('category', v.category || ''); fd.append('entity', v.entity); fd.append('entity_id', v.entity_id || '');
        fd.append('file', f);
        const res = await fetch('/api/documents', { method: 'POST', headers: { Authorization: `Bearer ${session.token}` }, body: fd });
        if (!res.ok) throw new Error((await res.json().catch(() => ({}))).error || res.statusText);
        toast('อัปโหลดแล้ว'); load();
      },
    });
    const ent = m.el.querySelector('[name=entity]'); const sel = m.el.querySelector('[name=entity_id]');
    ent.addEventListener('change', async () => {
      const src = { employee: opt.employees, customer: opt.customers, project: async () => (await get('/projects')).map((p) => ({ value: p.id, label: p.name })) }[ent.value];
      sel.innerHTML = '<option value="">-</option>' + (src ? (await src()).map((o) => `<option value="${o.value}">${esc(o.label)}</option>`).join('') : '');
    });
  });
  await load();
}

// ---------------------------------------------------------------- integrations

export async function integrations(el) {
  const events = await get('/webhooks/events');
  const showSecret = (secret) => modal({
    title: 'Secret สำหรับตรวจลายเซ็น',
    body: `<p class="notice" style="margin-top:0">คัดลอกเก็บไว้ ระบบจะไม่แสดงอีก</p><div class="code">${esc(secret)}</div>
      <p class="small">ตรวจสอบ: <span class="mono">X-SmartOffice-Signature = "sha256=" + HMAC_SHA256(secret, body)</span></p>`,
  });
  await crudPage(el, {
    title: 'Integrations & Webhooks', desc: 'ส่งเหตุการณ์ไปยังระบบ HR / ERP / CRM / แชท แบบเรียลไทม์ พร้อมลายเซ็น HMAC · ระบบภายนอกเรียกข้อมูลได้ผ่าน REST API (ดู docs/API.md)',
    endpoint: '/webhooks', addLabel: 'เพิ่ม Webhook',
    before: `<div class="panel panel-body small" style="margin-bottom:16px"><b>เหตุการณ์:</b> ${events.map((e) => `<span class="badge">${esc(e)}</span>`).join(' ')}</div>`,
    columns: [
      { label: 'ชื่อ', render: (w) => `<b>${esc(w.name)}</b><div class="small muted mono">${esc(w.url)}</div>` },
      { label: 'เหตุการณ์', render: (w) => esc(w.events) },
      { label: 'สถานะ', render: (w) => (w.is_active ? '<span class="badge ok">เปิด</span>' : '<span class="badge">ปิด</span>') },
      { label: 'ส่งล่าสุด', render: (w) => (w.last_sent_at ? `<span class="badge ${w.last_status >= 200 && w.last_status < 300 ? 'ok' : 'bad'}">${w.last_status ?? 'error'}</span> ${ago(w.last_sent_at)}${w.last_error ? `<div class="small muted">${esc(w.last_error)}</div>` : ''}` : '-') },
    ],
    extraActions: (w) => `<button class="btn sm" data-act="${w.id}" data-action="test">ทดสอบ</button><button class="btn sm" data-act="${w.id}" data-action="rotate">Secret ใหม่</button>`,
    onAction: async (action, w, reload) => {
      try {
        if (action === 'test') { const r = await post(`/webhooks/${w.id}/test`); toast(r.status ? `ปลายทางตอบกลับ HTTP ${r.status}` : 'ส่งไม่สำเร็จ', r.status >= 200 && r.status < 300 ? '' : 'err'); }
        if (action === 'rotate' && await confirmBox('สร้าง secret ใหม่? ระบบปลายทางต้องอัปเดตด้วย', 'สร้างใหม่', false)) showSecret((await post(`/webhooks/${w.id}/rotate-secret`)).secret);
        reload();
      } catch (e) { showError(e); }
    },
    afterSave: (r, created) => { if (created && r.secret) showSecret(r.secret); },
    fields: [
      { key: 'name', label: 'ชื่อ', required: true }, { key: 'url', label: 'URL ปลายทาง', required: true, placeholder: 'https://erp.example.com/hooks/smartoffice', full: true },
      { key: 'events', label: 'เหตุการณ์ (คั่นด้วย , หรือ * = ทั้งหมด)', required: true, default: '*', full: true },
      { key: 'is_active', label: 'เปิดใช้งาน', type: 'checkbox', default: true },
    ],
  });
}
