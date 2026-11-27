import { get, post, qs, session } from '../api.js';
import { esc, crudPage, opt, table, fmtDate, toast, showError, formModal, rangePicker, daysAgo, today, fillBranches, BRANCH_SELECT } from '../ui.js';

const TASK_STATUS = { todo: 'รอดำเนินการ', in_progress: 'กำลังทำ', done: 'เสร็จแล้ว', cancelled: 'ยกเลิก' };
const TASK_BADGE = { todo: '', in_progress: 'info', done: 'ok', cancelled: 'bad' };
const PRIORITY = { low: 'ต่ำ', normal: 'ปกติ', high: 'สูง' };

export function taskColumns() {
  return [
    { label: 'งาน', render: (r) => `<b>${esc(r.title)}</b>${r.project ? `<div class="small muted">${esc(r.project)}</div>` : ''}` },
    { key: 'assignee_name', label: 'ผู้รับผิดชอบ' },
    { key: 'customer_name', label: 'ลูกค้า', hideSm: true },
    { label: 'ความสำคัญ', hideSm: true, render: (r) => `<span class="badge ${r.priority === 'high' ? 'warn' : ''}">${PRIORITY[r.priority]}</span>` },
    { label: 'กำหนดส่ง', render: (r) => {
      const late = r.due_date && r.status !== 'done' && r.status !== 'cancelled' && r.due_date < today();
      return `<span class="${late ? 'badge bad' : ''}">${fmtDate(r.due_date)}</span>`;
    } },
    { label: 'สถานะ', render: (r) => `<span class="badge ${TASK_BADGE[r.status]}">${TASK_STATUS[r.status]}</span>` },
    { label: 'คุณภาพ', align: 'r', hideSm: true, render: (r) => (r.quality_score ? '★'.repeat(r.quality_score) : '-') },
  ];
}

export async function crm(el) {
  const { crm: crmTabs } = await import('./business.js');
  return crmTabs(el, { customers: customersPage });
}

export async function tasks(el) {
  await crudPage(el, {
    title: 'Task & Project Management',
    desc: 'งานที่มอบหมาย กำหนดส่ง และคะแนนคุณภาพ ซึ่งเป็นข้อมูลหลักของ KPI',
    endpoint: '/tasks',
    canEdit: session.can('tasks.manage'),
    addLabel: 'สร้างงาน',
    columns: taskColumns(),
    fields: [
      { key: 'title', label: 'ชื่องาน', required: true, full: true },
      { key: 'project_id', label: 'โครงการ', type: 'select', options: async () => (await get('/projects')).map((p) => ({ value: p.id, label: p.name })), numeric: true },
      { key: 'assignee_employee_id', label: 'ผู้รับผิดชอบ', type: 'select', options: opt.employees, numeric: true },
      { key: 'customer_id', label: 'ลูกค้า (CRM)', type: 'select', options: opt.customers, numeric: true },
      { key: 'due_date', label: 'กำหนดส่ง', type: 'date' },
      { key: 'priority', label: 'ความสำคัญ', type: 'select', required: true, default: 'normal', options: Object.entries(PRIORITY).map(([value, label]) => ({ value, label })) },
      { key: 'status', label: 'สถานะ', type: 'select', required: true, default: 'todo', options: Object.entries(TASK_STATUS).map(([value, label]) => ({ value, label })) },
      { key: 'quality_score', label: 'คะแนนคุณภาพ (1-5)', type: 'number', min: 1, max: 5, help: 'ให้คะแนนเมื่องานเสร็จ' },
      { key: 'description', label: 'รายละเอียด', type: 'textarea' },
    ],
  });
}

async function customersPage(el) {
  const SAT = (n) => (n ? `${'★'.repeat(n)}<span class="muted">${'★'.repeat(5 - n)}</span>` : '-');
  await crudPage(el, {
    title: 'CRM Management',
    desc: 'ลูกค้า ผู้ติดต่อ ผู้ดูแล และความพึงพอใจของลูกค้า',
    endpoint: '/customers',
    canEdit: session.can('crm.manage'),
    addLabel: 'เพิ่มลูกค้า',
    columns: [
      { label: 'ลูกค้า', render: (r) => `<b>${esc(r.name)}</b>${r.company ? `<div class="small muted">${esc(r.company)}</div>` : ''}` },
      { label: 'ติดต่อ', hideSm: true, render: (r) => `${esc(r.email || '')}<div class="small muted">${esc(r.phone || '')}</div>` },
      { key: 'owner_name', label: 'ผู้ดูแล' },
      { label: 'สถานะ', render: (r) => `<span class="badge ${r.status === 'active' ? 'ok' : r.status === 'lead' ? 'info' : ''}">${{ lead: 'ผู้สนใจ', active: 'ลูกค้าปัจจุบัน', inactive: 'ไม่เคลื่อนไหว' }[r.status]}</span>` },
      { label: 'ความพึงพอใจ', render: (r) => SAT(r.satisfaction) },
      { key: 'open_tasks', label: 'งานค้าง', align: 'r' },
    ],
    fields: [
      { key: 'name', label: 'ชื่อลูกค้า / ผู้ติดต่อ', required: true },
      { key: 'company', label: 'บริษัท' },
      { key: 'email', label: 'อีเมล', type: 'email' },
      { key: 'phone', label: 'โทรศัพท์' },
      { key: 'status', label: 'สถานะ', type: 'select', required: true, default: 'lead', options: [{ value: 'lead', label: 'ผู้สนใจ (Lead)' }, { value: 'active', label: 'ลูกค้าปัจจุบัน' }, { value: 'inactive', label: 'ไม่เคลื่อนไหว' }] },
      { key: 'owner_employee_id', label: 'พนักงานผู้ดูแล', type: 'select', options: opt.employees, numeric: true },
      { key: 'satisfaction', label: 'ความพึงพอใจ (1-5)', type: 'number', min: 1, max: 5 },
      { key: 'notes', label: 'บันทึก', type: 'textarea' },
    ],
  });
}

const SOURCES = {
  manual: 'กรอกคะแนนโดยผู้ประเมิน',
  task_completion: 'อัตราการปิดงาน (อัตโนมัติ)',
  task_on_time: 'ส่งงานตรงเวลา (อัตโนมัติ)',
  task_quality: 'คุณภาพผลงาน (อัตโนมัติ)',
  attendance_punctuality: 'มาทำงานตรงเวลา (อัตโนมัติ)',
  customer_satisfaction: 'ความพึงพอใจลูกค้า (อัตโนมัติ)',
};

function gradeBadge(t) {
  if (t == null) return '<span class="badge">ข้อมูลไม่พอ</span>';
  const k = t >= 80 ? 'ok' : t >= 70 ? 'info' : t >= 60 ? 'warn' : 'bad';
  return `<span class="badge ${k}">${t.toFixed(1)}</span>`;
}

export async function kpi(el) {
  el.innerHTML = `<div class="page-head"><div><h1>KPI & Performance</h1><p>คะแนนถ่วงน้ำหนักจากผลลัพธ์และคุณภาพของงาน · ข้อมูลจากกล้องไม่ถูกนำมาคิดคะแนน</p></div><span class="spacer"></span><div data-range></div>
    ${BRANCH_SELECT}<select class="input" data-dept style="width:170px"><option value="">ทุกแผนก</option></select></div>
    <div class="tabs"><button class="active" data-tab="scores">คะแนน KPI</button><button data-tab="metrics">ตัวชี้วัดและน้ำหนัก</button><button data-tab="records">ประวัติการประเมิน</button></div>
    <div data-body></div>`;
  (await opt.departments()).forEach((d) => el.querySelector('[data-dept]').insertAdjacentHTML('beforeend', `<option value="${d.value}">${esc(d.label)}</option>`));
  await fillBranches(el.querySelector('[data-branch]'));
  let tab = 'scores';
  const body = el.querySelector('[data-body]');
  const range = rangePicker(el.querySelector('[data-range]'), () => load(), { from: daysAgo(29), to: today() });
  const metrics = await get('/kpi/metrics');
  const manualMetrics = () => metrics.filter((m) => m.source === 'manual' && m.is_active).map((m) => ({ value: m.id, label: m.name }));

  const review = (row) => formModal({
    title: `ประเมิน: ${row.name}`,
    intro: '<p class="notice" style="margin-top:0">คะแนนนี้ควรพิจารณาจากผลงานจริง ประกอบการพูดคุยกับพนักงาน</p>',
    fields: [
      { key: 'metric_id', label: 'ตัวชี้วัด', type: 'select', required: true, options: manualMetrics, numeric: true, full: true },
      { key: 'period_start', label: 'ตั้งแต่', type: 'date', required: true, default: range.get().from },
      { key: 'period_end', label: 'ถึง', type: 'date', required: true, default: range.get().to },
      { key: 'score', label: 'คะแนน (0-100)', type: 'number', min: 0, max: 100, step: '0.5', required: true },
      { key: 'note', label: 'ความเห็น', type: 'textarea' },
    ],
    onSubmit: async (v) => { await post('/kpi/records', { ...v, employee_id: row.employee_id }); toast('บันทึกผลประเมินแล้ว'); load(); },
  });

  let scoreRows = [];
  const load = async () => {
    const { from, to } = range.get();
    const dept = el.querySelector('[data-dept]').value;
    try {
      if (tab === 'scores') {
        const d = await get(`/kpi/scores${qs({ from, to, department_id: dept, branch_id: el.querySelector('[data-branch]').value })}`);
        scoreRows = d.rows;
        body.innerHTML = `<div class="panel">${table([
          { key: 'employee_code', label: 'รหัส' }, { key: 'name', label: 'ชื่อ' }, { key: 'department', label: 'แผนก', hideSm: true },
          ...metrics.filter((m) => m.is_active).map((m) => ({ label: `${m.name} (${m.weight}%)`, align: 'r', hideSm: true, render: (r) => {
            const it = r.items.find((i) => i.metric_id === m.id);
            return it ? `<span title="${esc(it.detail)}">${it.score == null ? '<span class="muted">-</span>' : it.score.toFixed(0)}</span>` : '<span class="muted">n/a</span>';
          } })),
          { label: 'คะแนนรวม', align: 'r', render: (r) => gradeBadge(r.total) },
          { label: 'ระดับ', render: (r) => esc(r.grade || '-') },
        ], scoreRows, { actions: (r) => `${manualMetrics().length ? `<button class="btn sm" data-review="${r.employee_id}">ประเมิน</button>` : ''}` })}</div>`;
      } else if (tab === 'metrics') {
        await crudPage(body, {
          title: 'ตัวชี้วัด (KPI Metrics)', desc: 'กำหนดน้ำหนักตามตำแหน่งงานได้ (เว้นว่าง = ใช้กับทุกตำแหน่ง) คะแนนรวมคิดเฉพาะตัวชี้วัดที่มีข้อมูล',
          endpoint: '/kpi/metrics', canEdit: session.can('kpi.manage'), addLabel: 'เพิ่มตัวชี้วัด',
          columns: [
            { key: 'code', label: 'รหัส' }, { key: 'name', label: 'ชื่อ' }, { label: 'แหล่งข้อมูล', render: (r) => esc(SOURCES[r.source]) },
            { key: 'weight', label: 'น้ำหนัก', align: 'r', render: (r) => `${r.weight}%` }, { key: 'position', label: 'ตำแหน่ง', render: (r) => esc(r.position || 'ทุกตำแหน่ง') },
            { label: 'ใช้งาน', render: (r) => (r.is_active ? '<span class="badge ok">ใช้งาน</span>' : '<span class="badge">ปิด</span>') },
          ],
          fields: [
            { key: 'code', label: 'รหัส', required: true }, { key: 'name', label: 'ชื่อตัวชี้วัด', required: true },
            { key: 'source', label: 'แหล่งข้อมูล', type: 'select', required: true, options: Object.entries(SOURCES).map(([value, label]) => ({ value, label })) },
            { key: 'weight', label: 'น้ำหนัก (%)', type: 'number', min: 0, max: 100, step: '0.5', required: true },
            { key: 'position', label: 'ใช้กับตำแหน่ง', help: 'ตรงกับช่องตำแหน่งของพนักงาน' },
            { key: 'is_active', label: 'เปิดใช้งาน', type: 'checkbox', default: true },
            { key: 'description', label: 'คำอธิบาย', type: 'textarea' },
          ],
        });
      } else {
        const rows = await get(`/kpi/records${qs({ from, to })}`);
        body.innerHTML = `<div class="panel">${table([
          { label: 'ช่วงเวลา', render: (r) => `${fmtDate(r.period_start)} - ${fmtDate(r.period_end)}` },
          { key: 'employee_name', label: 'พนักงาน' }, { key: 'metric_name', label: 'ตัวชี้วัด' },
          { key: 'score', label: 'คะแนน', align: 'r' }, { key: 'note', label: 'ความเห็น' },
        ], rows)}</div>`;
      }
    } catch (e) { showError(e); }
  };
  body.addEventListener('click', (e) => {
    const b = e.target.closest('[data-review]');
    if (b) review(scoreRows.find((r) => String(r.employee_id) === b.dataset.review));
  });
  el.querySelector('.tabs').addEventListener('click', (e) => {
    const b = e.target.closest('[data-tab]'); if (!b) return;
    tab = b.dataset.tab;
    el.querySelectorAll('.tabs button').forEach((x) => x.classList.toggle('active', x === b));
    load();
  });
  el.querySelector('[data-dept]').addEventListener('change', load);
  el.querySelector('[data-branch]').addEventListener('change', load);
  await load();
}

