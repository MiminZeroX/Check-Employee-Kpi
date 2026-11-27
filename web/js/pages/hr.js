import { api, get, post, qs, session } from '../api.js';
import {
  esc, icon, crudPage, opt, table, toast, showError, formModal, fmtDate, fmtDateTime, fmtTime, today,
} from '../ui.js';

export const REQ_KIND = { leave: 'ลา', offsite: 'นอกสถานที่', overtime: 'OT' };
export const REQ_STATUS = {
  pending: '<span class="badge warn">รอพิจารณา</span>', approved: '<span class="badge ok">อนุมัติ</span>',
  rejected: '<span class="badge bad">ไม่อนุมัติ</span>', cancelled: '<span class="badge">ยกเลิก</span>',
};
const WEEKDAYS = ['จ', 'อ', 'พ', 'พฤ', 'ศ', 'ส', 'อา'];
const daysLabel = (s) => s.split(',').map((d) => WEEKDAYS[Number(d) - 1]).join(' ');

export function requestPeriod(r) {
  if (r.kind === 'leave') {
    const a = fmtDate(r.start_at); const b = fmtDate(r.end_at);
    return `${a}${a !== b ? ` - ${b}` : ''} ${r.days != null ? `<span class="muted">(${r.days} วัน)</span>` : ''}`;
  }
  return `${fmtDateTime(r.start_at)} - ${fmtTime(r.end_at)} ${r.hours != null ? `<span class="muted">(${r.hours} ชม.)</span>` : ''}`;
}

export const requestColumns = (withEmployee = true) => [
  ...(withEmployee ? [{ key: 'employee_name', label: 'พนักงาน' }, { key: 'department_name', label: 'แผนก', hideSm: true }] : []),
  { label: 'ประเภท', render: (r) => `<span class="badge info">${REQ_KIND[r.kind]}</span> ${esc(r.leave_type_name || r.location || '')}` },
  { label: 'ช่วงเวลา', render: requestPeriod },
  { key: 'reason', label: 'เหตุผล' },
  { label: 'สถานะ', render: (r) => `${REQ_STATUS[r.status] || esc(r.status)}${r.approver_note ? `<div class="small muted">${esc(r.approver_name || '')}: ${esc(r.approver_note)}</div>` : ''}` },
];

/** Form to ask for leave / off-site work / overtime (for oneself, or for an employee when employeeField). */
export async function requestForm({ kind, employeeField = false, onDone }) {
  const types = (await get('/leave-types')).filter((t) => t.is_active).map((t) => ({ value: t.id, label: t.name }));
  const fields = [
    ...(employeeField ? [{ key: 'employee_id', label: 'พนักงาน', type: 'select', required: true, options: opt.employees, numeric: true, full: true }] : []),
    ...(kind === 'leave' ? [
      { key: 'leave_type_id', label: 'ประเภทการลา', type: 'select', required: true, options: types, numeric: true },
      { key: 'half_day', label: 'ช่วงเวลา', type: 'select', required: true, default: 'full', options: [{ value: 'full', label: 'เต็มวัน' }, { value: 'am', label: 'ครึ่งวันเช้า' }, { value: 'pm', label: 'ครึ่งวันบ่าย' }] },
      { key: 'start_date', label: 'ตั้งแต่วันที่', type: 'date', required: true, default: today() },
      { key: 'end_date', label: 'ถึงวันที่', type: 'date', default: today(), help: 'ลาครึ่งวันใช้เฉพาะวันเริ่ม · วันหยุดไม่นับ' },
    ] : kind === 'offsite' ? [
      { key: 'start_date', label: 'ตั้งแต่วันที่', type: 'date', required: true, default: today() },
      { key: 'end_date', label: 'ถึงวันที่', type: 'date', default: today() },
      { key: 'start_time', label: 'เวลาเริ่ม (ว่าง = ตามกะ)', type: 'time' },
      { key: 'end_time', label: 'เวลาสิ้นสุด (ว่าง = ตามกะ)', type: 'time' },
      { key: 'location', label: 'สถานที่', required: true, full: true },
    ] : [
      { key: 'start_date', label: 'วันที่', type: 'date', required: true, default: today() },
      { key: 'start_time', label: 'เวลาเริ่ม (ว่าง = หลังเลิกกะ)', type: 'time' },
      { key: 'hours', label: 'จำนวนชั่วโมง', type: 'number', step: '0.5', min: 0.5, max: 16, required: true },
    ]),
    { key: 'reason', label: 'เหตุผล', type: 'textarea', required: true },
  ];
  formModal({
    title: { leave: 'ขอลา', offsite: 'ขอปฏิบัติงานนอกสถานที่', overtime: 'ขอทำงานล่วงเวลา (OT)' }[kind],
    fields, submitLabel: 'ส่งคำขอ',
    onSubmit: async (v) => {
      const r = await post('/requests', { ...v, kind });
      toast(`ส่งคำขอแล้ว${r.days ? ` (${r.days} วันทำงาน)` : ''}`);
      onDone?.();
    },
  });
}

// ---------------------------------------------------------------- approvals

export async function approvals(el) {
  el.innerHTML = `<div class="page-head"><div><h1>คำขอและการอนุมัติ</h1><p>ลา · ปฏิบัติงานนอกสถานที่ · ทำงานล่วงเวลา (หัวหน้างานเห็นเฉพาะแผนกที่ดูแล)</p></div><span class="spacer"></span>
    ${['leave', 'offsite', 'overtime'].map((k) => `<button class="btn" data-new="${k}">${icon('plus')} ${REQ_KIND[k]}แทนพนักงาน</button>`).join('')}</div>
    <div data-corr></div>
    <div class="panel" style="margin-bottom:16px"><div class="panel-head"><h2>รออนุมัติ</h2><span class="sub" data-count></span></div><div data-inbox></div></div>
    <div class="panel"><div class="panel-head"><h2>คำขอทั้งหมด</h2><span class="spacer"></span>
      <select class="input" data-status style="width:150px"><option value="">ทุกสถานะ</option>${Object.keys(REQ_STATUS).map((s) => `<option value="${s}">${REQ_STATUS[s].replace(/<[^>]+>/g, '')}</option>`).join('')}</select>
      <select class="input" data-kind style="width:150px"><option value="">ทุกประเภท</option>${Object.entries(REQ_KIND).map(([k, v]) => `<option value="${k}">${v}</option>`).join('')}</select></div><div data-all></div></div>`;
  const load = async () => {
    try {
      const inbox = await get('/approvals');
      el.querySelector('[data-count]').textContent = `${inbox.requests.length} รายการ`;
      el.querySelector('[data-corr]').innerHTML = inbox.corrections ? `<p class="notice" style="margin-top:0">มีคำขอแก้ไขข้อมูลกิจกรรมรอพิจารณา ${inbox.corrections} รายการ · <a href="#privacy">ไปที่ Privacy &amp; Data</a></p>` : '';
      el.querySelector('[data-inbox]').innerHTML = table(requestColumns(), inbox.requests, {
        empty: 'ไม่มีคำขอรออนุมัติ',
        actions: (r) => `<button class="btn sm primary" data-ok="${r.id}">อนุมัติ</button><button class="btn sm danger" data-no="${r.id}">ไม่อนุมัติ</button>`,
      });
      const all = await get(`/requests${qs({ status: el.querySelector('[data-status]').value, kind: el.querySelector('[data-kind]').value })}`);
      el.querySelector('[data-all]').innerHTML = table(requestColumns(), all, { empty: 'ไม่มีคำขอ' });
    } catch (e) { showError(e); }
  };
  el.addEventListener('click', (e) => {
    const b = e.target.closest('button'); if (!b) return;
    if (b.dataset.new) { requestForm({ kind: b.dataset.new, employeeField: true, onDone: load }); return; }
    const id = b.dataset.ok || b.dataset.no; if (!id) return;
    const approve = !!b.dataset.ok;
    formModal({
      title: approve ? 'อนุมัติคำขอ' : 'ไม่อนุมัติคำขอ',
      fields: [{ key: 'note', label: approve ? 'หมายเหตุ (ถ้ามี)' : 'เหตุผลที่ไม่อนุมัติ', type: 'textarea', required: !approve }],
      submitLabel: approve ? 'อนุมัติ' : 'ยืนยันไม่อนุมัติ',
      onSubmit: async (v) => { await post(`/requests/${id}/decide`, { approve, ...v }); toast('บันทึกผลแล้ว และแจ้งพนักงานแล้ว'); load(); },
    });
  });
  el.querySelector('[data-status]').addEventListener('change', load);
  el.querySelector('[data-kind]').addEventListener('change', load);
  await load();
}

// ---------------------------------------------------------------- shifts, holidays, leave types

export async function shifts(el) {
  el.innerHTML = `<div class="tabs"><button class="active" data-tab="shifts">กะการทำงาน</button><button data-tab="assign">จัดกะรายวัน</button>
    <button data-tab="holidays">วันหยุด</button><button data-tab="leave">ประเภทการลา</button></div><div data-body></div>`;
  const body = el.querySelector('[data-body]');
  const shiftOpts = async () => (await get('/shifts')).map((s) => ({ value: s.id, label: `${s.name} (${s.start_time.slice(0, 5)}-${s.end_time.slice(0, 5)})` }));
  const views = {
    shifts: () => crudPage(body, {
      title: 'กะการทำงาน', desc: 'กำหนดเวลาเข้า-ออก เวลาพัก วันทำงาน และเวลาผ่อนผันการมาสาย · กะข้ามเที่ยงคืนได้', endpoint: '/shifts', addLabel: 'เพิ่มกะ',
      columns: [
        { label: 'กะ', render: (r) => `<b>${esc(r.name)}</b>${r.is_default ? ' <span class="badge info">ค่าเริ่มต้น</span>' : ''}` },
        { label: 'เวลา', render: (r) => `${r.start_time.slice(0, 5)} - ${r.end_time.slice(0, 5)}` },
        { label: 'พัก', render: (r) => (r.break_start ? `${r.break_start.slice(0, 5)} - ${r.break_end?.slice(0, 5) || ''}` : '-') },
        { label: 'วันทำงาน', render: (r) => daysLabel(r.work_days) }, { key: 'grace_min', label: 'ผ่อนผัน (นาที)', align: 'r' },
        { key: 'employee_count', label: 'พนักงาน', align: 'r' },
      ],
      fields: [
        { key: 'name', label: 'ชื่อกะ', required: true, full: true },
        { key: 'start_time', label: 'เวลาเข้างาน', type: 'time', required: true }, { key: 'end_time', label: 'เวลาเลิกงาน', type: 'time', required: true },
        { key: 'break_start', label: 'เริ่มพัก', type: 'time' }, { key: 'break_end', label: 'สิ้นสุดพัก', type: 'time' },
        { key: 'work_days', label: 'วันทำงาน (1=จันทร์ ... 7=อาทิตย์)', default: '1,2,3,4,5', required: true },
        { key: 'grace_min', label: 'ผ่อนผันมาสาย (นาที)', type: 'number', min: 0, max: 120, default: 0 },
        { key: 'is_default', label: 'ใช้เป็นกะเริ่มต้นของพนักงานที่ไม่ได้กำหนดกะ', type: 'checkbox', full: true },
      ],
    }),
    assign: async () => {
      const from = today(); const to = new Date(Date.now() + 13 * 86400e3).toISOString().slice(0, 10);
      body.innerHTML = `<div class="page-head"><div><h1>จัดกะรายวัน</h1><p>กำหนดกะหรือวันหยุดเฉพาะวัน (กะหมุนเวียน สลับวันหยุด) มีผลเหนือกะประจำของพนักงาน</p></div><span class="spacer"></span>
        <button class="btn primary" data-assign>${icon('plus')} จัดกะ</button></div>
        <div class="panel"><div class="toolbar"><input type="date" class="input" data-from value="${from}"><span class="muted">ถึง</span><input type="date" class="input" data-to value="${to}"></div><div data-list></div></div>`;
      const list = async () => {
        const rows = await get(`/shift-assignments${qs({ from: body.querySelector('[data-from]').value, to: body.querySelector('[data-to]').value })}`);
        body.querySelector('[data-list]').innerHTML = table([
          { label: 'วันที่', render: (r) => fmtDate(r.work_date) }, { key: 'employee_name', label: 'พนักงาน' },
          { label: 'กะ', render: (r) => (r.shift_name ? esc(r.shift_name) : '<span class="badge">วันหยุด</span>') }, { key: 'note', label: 'หมายเหตุ' },
        ], rows, { empty: 'ไม่มีการจัดกะพิเศษ ใช้กะประจำของพนักงาน', actions: (r) => `<button class="btn sm ghost danger" data-del="${r.id}">${icon('trash')}</button>` });
      };
      body.querySelector('[data-from]').onchange = list; body.querySelector('[data-to]').onchange = list;
      body.onclick = async (e) => {
        const b = e.target.closest('button'); if (!b) return;
        if (b.dataset.del) { await api(`/shift-assignments/${b.dataset.del}`, { method: 'DELETE' }); list(); return; }
        if (b.dataset.assign === undefined) return;
        const emps = await opt.employees();
        const shiftsList = await shiftOpts();
        const form = await formModal({
          title: 'จัดกะ / กำหนดวันหยุด',
          intro: `<div class="field full" style="margin-bottom:12px"><label>พนักงาน</label><div style="max-height:180px;overflow:auto;border:1px solid var(--line);border-radius:8px;padding:8px">${emps.map((x) => `<label class="check"><input type="checkbox" name="emp" value="${x.value}"> ${esc(x.label)}</label>`).join('')}</div></div>`,
          fields: [
            { key: 'from', label: 'ตั้งแต่', type: 'date', required: true, default: today() }, { key: 'to', label: 'ถึง', type: 'date', required: true, default: today() },
            { key: 'shift_id', label: 'กะ', type: 'select', options: shiftsList, numeric: true, placeholder: '- วันหยุด -' },
            { key: 'weekdays', label: 'เฉพาะวัน (เช่น 6,7 · ว่าง = ทุกวัน)' }, { key: 'note', label: 'หมายเหตุ', full: true },
          ],
          onSubmit: async (v, f) => {
            const ids = [...f.querySelectorAll('input[name=emp]:checked')].map((i) => Number(i.value));
            if (!ids.length) throw new Error('กรุณาเลือกพนักงานอย่างน้อย 1 คน');
            const weekdays = v.weekdays ? v.weekdays.split(',').map((d) => Number(d.trim())).filter(Boolean) : null;
            const r = await post('/shift-assignments', { employee_ids: ids, from: v.from, to: v.to, shift_id: v.shift_id, weekdays, note: v.note });
            toast(`จัดกะแล้ว ${r.assigned} รายการ`); list();
          },
        });
        return form;
      };
      await list();
    },
    holidays: async () => {
      const year = new Date().getFullYear();
      body.innerHTML = `<div class="page-head"><div><h1>วันหยุด</h1><p>วันหยุดของบริษัททั้งหมด หรือเฉพาะสาขา · ระบบไม่นับขาดงาน/มาสายในวันหยุด และนับงานในวันหยุดเป็น OT</p></div><span class="spacer"></span>
        <select class="input" data-year style="width:110px">${[year - 1, year, year + 1].map((y) => `<option ${y === year ? 'selected' : ''}>${y}</option>`).join('')}</select>
        <button class="btn" data-import>นำเข้าวันหยุดราชการ (วันที่คงที่)</button><button class="btn primary" data-add>${icon('plus')} เพิ่มวันหยุด</button></div>
        <p class="notice" style="margin-top:0">วันหยุดทางจันทรคติ (มาฆบูชา วิสาขบูชา อาสาฬหบูชา เข้าพรรษา) และวันหยุดชดเชยเปลี่ยนทุกปี กรุณาเพิ่มตามประกาศของปีนั้น</p>
        <div class="panel" data-list></div>`;
      const y = () => body.querySelector('[data-year]').value;
      const list = async () => {
        const rows = await get(`/holidays?year=${y()}`);
        body.querySelector('[data-list]').innerHTML = table([
          { label: 'วันที่', render: (r) => fmtDate(r.holiday_date) }, { key: 'name', label: 'วันหยุด' }, { label: 'ใช้กับ', render: (r) => esc(r.branch_name || 'ทุกสาขา') },
        ], rows, { empty: 'ยังไม่มีวันหยุด', actions: (r) => `<button class="btn sm ghost danger" data-del="${r.id}">${icon('trash')}</button>` });
      };
      body.querySelector('[data-year]').onchange = list;
      body.onclick = async (e) => {
        const b = e.target.closest('button'); if (!b) return;
        try {
          if (b.dataset.del) { await api(`/holidays/${b.dataset.del}`, { method: 'DELETE' }); list(); }
          else if (b.dataset.import !== undefined) { const r = await post('/holidays/import-fixed', { year: Number(y()) }); toast(`เพิ่ม ${r.added} วัน`); list(); }
          else if (b.dataset.add !== undefined) {
            formModal({
              title: 'เพิ่มวันหยุด',
              fields: [{ key: 'holiday_date', label: 'วันที่', type: 'date', required: true }, { key: 'name', label: 'ชื่อวันหยุด', required: true },
                { key: 'branch_id', label: 'เฉพาะสาขา', type: 'select', options: opt.branches, numeric: true, placeholder: '- ทุกสาขา -' }],
              onSubmit: async (v) => { await post('/holidays', v); toast('เพิ่มวันหยุดแล้ว'); list(); },
            });
          }
        } catch (err) { showError(err); }
      };
      await list();
    },
    leave: () => crudPage(body, {
      title: 'ประเภทการลา', desc: 'โควตาต่อปี (0 = ไม่จำกัด) · ระบบตรวจยอดคงเหลือก่อนส่งคำขอ', endpoint: '/leave-types', addLabel: 'เพิ่มประเภทการลา',
      columns: [{ key: 'code', label: 'รหัส' }, { key: 'name', label: 'ประเภท' }, { label: 'โควตา/ปี', align: 'r', render: (r) => (r.annual_quota_days ? `${r.annual_quota_days} วัน` : 'ไม่จำกัด') },
        { label: 'รับค่าจ้าง', render: (r) => (r.paid ? 'ใช่' : 'ไม่') }, { label: 'ใช้งาน', render: (r) => (r.is_active ? '<span class="badge ok">ใช้งาน</span>' : '<span class="badge">ปิด</span>') }],
      fields: [{ key: 'code', label: 'รหัส', required: true }, { key: 'name', label: 'ชื่อ', required: true },
        { key: 'annual_quota_days', label: 'โควตาต่อปี (วัน)', type: 'number', step: '0.5', min: 0, required: true },
        { key: 'paid', label: 'รับค่าจ้าง', type: 'checkbox', default: true }, { key: 'is_active', label: 'เปิดใช้งาน', type: 'checkbox', default: true }],
    }),
  };
  el.querySelector('.tabs').addEventListener('click', (e) => {
    const b = e.target.closest('[data-tab]'); if (!b) return;
    el.querySelectorAll('.tabs button').forEach((x) => x.classList.toggle('active', x === b));
    body.onclick = null;
    views[b.dataset.tab]();
  });
  await views.shifts();
}

// ---------------------------------------------------------------- org chart

export async function orgChart(el) {
  const { departments } = await get('/org-chart');
  const byParent = {};
  for (const d of departments) (byParent[d.parent_id ?? 'root'] ||= []).push(d);
  const node = (d) => `<li><div class="org-node">
      <b>${esc(d.name)}</b><span class="small muted">${esc(d.branch || '')} · ${d.employee_count} คน</span>
      <div class="small">${d.manager_name ? `หัวหน้า: ${esc(d.manager_name)}${d.manager_position ? ` <span class="muted">(${esc(d.manager_position)})</span>` : ''}` : '<span class="muted">ยังไม่กำหนดหัวหน้า</span>'}</div>
      <details><summary class="small">สมาชิก</summary><div class="small">${d.members.map((m) => `${esc(m.name)}${m.position ? ` <span class="muted">· ${esc(m.position)}</span>` : ''}`).join('<br>') || '-'}</div></details>
    </div>${byParent[d.id] ? `<ul>${byParent[d.id].map(node).join('')}</ul>` : ''}</li>`;
  const roots = departments.filter((d) => !d.parent_id || !departments.some((x) => x.id === d.parent_id));
  el.innerHTML = `<div class="page-head"><div><h1>ผังองค์กร</h1><p>โครงสร้างแผนก หัวหน้า และสมาชิก · แก้ไขแผนกแม่และหัวหน้าได้ที่ Department Management</p></div><span class="spacer"></span>
    ${session.can('org.manage') ? '<a class="btn" href="#departments">แก้ไขโครงสร้าง</a>' : ''}</div>
    <div class="panel panel-body org-tree"><ul>${roots.map(node).join('') || '<li class="muted">ยังไม่มีแผนก</li>'}</ul></div>`;
}

