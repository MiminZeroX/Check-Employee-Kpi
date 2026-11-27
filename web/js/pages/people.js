import { get, post, qs, session } from '../api.js';
import { esc, icon, crudPage, opt, table, fmtTime, fmtDur, fmtDate, fmtDateTime, today, daysAgo, toast, showError, formModal } from '../ui.js';

export async function employees(el) {
  const canEdit = session.can('employees.manage');
  await crudPage(el, {
    title: 'Employee Management',
    desc: 'ข้อมูลพนักงาน รหัสเครื่องสแกน เวลาทำงาน และการยินยอมให้วิเคราะห์ข้อมูล',
    endpoint: '/employees',
    canEdit,
    addLabel: 'เพิ่มพนักงาน',
    deleteLabel: 'ปิดการใช้งานพนักงานคนนี้? (ประวัติยังคงอยู่)',
    columns: [
      { key: 'employee_code', label: 'รหัส' },
      { label: 'ชื่อ-สกุล', render: (r) => `${esc(r.first_name)} ${esc(r.last_name)}${r.nickname ? ` <span class="muted">(${esc(r.nickname)})</span>` : ''}` },
      { key: 'position', label: 'ตำแหน่ง', hideSm: true },
      { key: 'department_name', label: 'แผนก' },
      { key: 'attendance_ref', label: 'รหัสเครื่องสแกน', hideSm: true },
      { label: 'กะ', hideSm: true, render: (r) => esc(r.shift_name || 'กะเริ่มต้น') },
      { label: 'ยินยอม', render: (r) => (r.monitoring_consent ? '<span class="badge ok">ยินยอม</span>' : '<span class="badge warn">ยังไม่บันทึก</span>') },
      { label: 'สถานะ', render: (r) => (r.status === 'active' ? '<span class="badge ok">ทำงาน</span>' : '<span class="badge">ปิดใช้งาน</span>') },
    ],
    query: () => '',
    fields: [
      { key: 'employee_code', label: 'รหัสพนักงาน', required: true },
      { key: 'attendance_ref', label: 'รหัสในเครื่องสแกนนิ้ว/หน้า', help: 'User ID / PIN ของพนักงานในเครื่องสแกน' },
      { key: 'first_name', label: 'ชื่อ', required: true },
      { key: 'last_name', label: 'นามสกุล' },
      { key: 'nickname', label: 'ชื่อเล่น' },
      { key: 'position', label: 'ตำแหน่ง', help: 'ใช้จับคู่ตัวชี้วัด KPI ตามตำแหน่ง' },
      { key: 'department_id', label: 'แผนก', type: 'select', options: opt.departments, numeric: true },
      { key: 'status', label: 'สถานะ', type: 'select', required: true, options: [{ value: 'active', label: 'ทำงาน' }, { value: 'inactive', label: 'ปิดใช้งาน' }], default: 'active' },
      { key: 'email', label: 'อีเมล', type: 'email' },
      { key: 'phone', label: 'โทรศัพท์' },
      { key: 'shift_id', label: 'กะการทำงาน', type: 'select', options: async () => (await get('/shifts')).map((s) => ({ value: s.id, label: s.name })), numeric: true, placeholder: '- ใช้กะเริ่มต้นของบริษัท -' },
      { key: 'work_start', label: 'เวลาเริ่มงาน (ใช้เมื่อไม่มีกะ)', type: 'time', default: '08:30' },
      { key: 'work_end', label: 'เวลาเลิกงาน (ใช้เมื่อไม่มีกะ)', type: 'time', default: '17:30' },
      { key: 'monitoring_consent', label: 'พนักงานรับทราบและยินยอมการวิเคราะห์ข้อมูลตามนโยบาย PDPA ขององค์กร', type: 'checkbox', full: true },
    ],
  });
}

export async function attendance(el) {
  el.innerHTML = `<div class="page-head"><div><h1>Attendance Management</h1><p>เปรียบเทียบเวลาสแกนนิ้ว/สแกนหน้า กับเวลาที่มาถึงโต๊ะทำงานจริง</p></div><span class="spacer"></span>
    <input type="date" class="input" data-from value="${daysAgo(6)}" style="width:150px" title="ตั้งแต่ (แท็บเวลาทำงาน)">
    <input type="date" class="input" data-date value="${today()}" style="width:160px">
    <select class="input" data-dept style="width:180px"><option value="">ทุกแผนก</option></select>
    ${session.can('attendance.manage') ? `<button class="btn" data-manual>${icon('plus')} บันทึกเวลาแทน</button>` : ''}</div>
    <div class="grid g-4" style="margin-bottom:16px" data-stats></div>
    <div class="tabs"><button class="active" data-tab="daily">สรุปรายวัน</button><button data-tab="sessions">เวลาทำงานช่วงวันที่</button><button data-tab="logs">ประวัติการสแกน</button>${session.can('attendance.manage') ? '<button data-tab="unmatched">รหัสที่ยังไม่จับคู่</button>' : ''}</div>
    <div class="panel" data-body></div>`;
  const depts = await opt.departments();
  el.querySelector('[data-dept]').insertAdjacentHTML('beforeend', depts.map((d) => `<option value="${d.value}">${esc(d.label)}</option>`).join(''));
  let tab = 'daily';
  const body = el.querySelector('[data-body]');
  const statuses = {
    present: (r) => (r.late_min > 0 ? `<span class="badge warn">สาย ${r.late_min} น.</span>` : '<span class="badge ok">มาทำงาน</span>'),
    leave: () => '<span class="badge info">ลา</span>', offsite: () => '<span class="badge info">นอกสถานที่</span>',
    holiday: () => '<span class="badge">วันหยุด</span>', dayoff: () => '<span class="badge">วันหยุดประจำ</span>',
    not_yet: () => '<span class="badge">ยังไม่ถึงเวลา</span>', absent: () => '<span class="badge bad">ขาดงาน</span>',
  };
  const status = (r) => (statuses[r.status] || (() => esc(r.status)))(r);
  const load = async () => {
    const date = el.querySelector('[data-date]').value;
    const dept = el.querySelector('[data-dept]').value;
    try {
      if (tab === 'daily') {
        const d = await get(`/attendance/daily${qs({ date, department_id: dept })}`);
        const s2d = d.rows.filter((r) => r.check_in && r.first_desk_at).map((r) => (new Date(r.first_desk_at) - new Date(r.check_in)) / 1000);
        el.querySelector('[data-stats]').innerHTML = `
          <div class="panel stat ok"><div class="ic">${icon('users')}</div><div><b>${d.present}/${d.rows.filter((r) => r.day_type === 'workday').length}</b><span>มาทำงาน (ของผู้มีกะวันนี้)</span></div></div>
          <div class="panel stat warn"><div class="ic">${icon('clock')}</div><div><b>${d.late}</b><span>มาสาย · ขาดงาน ${d.absent}</span></div></div>
          <div class="panel stat"><div class="ic">${icon('calendar')}</div><div><b>${d.leave + d.offsite}</b><span>ลา ${d.leave} · นอกสถานที่ ${d.offsite}</span></div></div>
          <div class="panel stat violet"><div class="ic">${icon('floor')}</div><div><b>${s2d.length ? fmtDur(s2d.reduce((x, y) => x + y, 0) / s2d.length) : '-'}</b><span>สแกน → ถึงโต๊ะ (เฉลี่ย)</span></div></div>`;
        body.innerHTML = table([
          { key: 'employee_code', label: 'รหัส' }, { key: 'name', label: 'ชื่อ' }, { key: 'department', label: 'แผนก', hideSm: true },
          { label: 'กะ', hideSm: true, render: (r) => (r.scheduled_start ? `${fmtTime(r.scheduled_start)}-${fmtTime(r.scheduled_end)}` : '-') },
          { label: 'เข้าสำนักงาน', hideSm: true, render: (r) => fmtTime(r.office_entry_at) },
          { label: 'สแกนเข้า', render: (r) => fmtTime(r.check_in) },
          { label: 'ถึงโต๊ะ', render: (r) => fmtTime(r.first_desk_at) },
          { label: 'สแกน→โต๊ะ', align: 'r', hideSm: true, render: (r) => (r.check_in && r.first_desk_at ? fmtDur((new Date(r.first_desk_at) - new Date(r.check_in)) / 1000) : '-') },
          { label: 'สแกนออก', render: (r) => fmtTime(r.check_out) },
          { label: 'พัก', align: 'r', hideSm: true, render: (r) => (r.break_sec ? fmtDur(r.break_sec) : '-') },
          { label: 'OT', align: 'r', render: (r) => (r.overtime_min ? `${fmtDur(r.overtime_min * 60)}${r.approved_overtime_min ? ` <span class="small muted">(อนุมัติ ${fmtDur(r.approved_overtime_min * 60)})</span>` : ''}` : '-') },
          { label: 'สถานะ', render: status },
        ], d.rows);
      } else if (tab === 'sessions') {
        el.querySelector('[data-stats]').innerHTML = '';
        const rows = await get(`/work-sessions${qs({ from: el.querySelector('[data-from]').value, to: date, department_id: dept })}`);
        body.innerHTML = table([
          { label: 'วันที่', render: (r) => fmtDate(r.work_date) }, { key: 'name', label: 'พนักงาน' },
          { label: 'กะ', hideSm: true, render: (r) => esc(r.shift_name || '-') },
          { label: 'เข้า-ออก', render: (r) => `${fmtTime(r.check_in)} - ${fmtTime(r.check_out)}` },
          { label: 'ทำงาน', align: 'r', render: (r) => (r.worked_sec ? fmtDur(r.worked_sec) : '-') },
          { label: 'อยู่ที่โต๊ะ', align: 'r', hideSm: true, render: (r) => (r.desk_sec ? fmtDur(r.desk_sec) : '-') },
          { label: 'สาย / กลับก่อน', align: 'r', render: (r) => `${r.late_min || 0} / ${r.early_leave_min || 0} น.` },
          { label: 'OT', align: 'r', render: (r) => (r.overtime_min ? fmtDur(r.overtime_min * 60) : '-') },
          { label: 'สถานะ', render: status },
        ], rows);
      } else if (tab === 'logs') {
        el.querySelector('[data-stats]').innerHTML = '';
        const rows = await get(`/attendance/logs${qs({ from: date, to: date })}`);
        body.innerHTML = table([
          { label: 'เวลา', render: (r) => fmtDateTime(r.scan_time) }, { key: 'employee_name', label: 'พนักงาน', render: (r) => esc(r.employee_name || '— ไม่พบในระบบ —') },
          { key: 'attendance_ref', label: 'รหัสเครื่องสแกน' }, { label: 'ทิศทาง', render: (r) => ({ in: 'เข้า', out: 'ออก' }[r.direction] || '-') },
          { key: 'method', label: 'วิธี' }, { key: 'device_name', label: 'อุปกรณ์', render: (r) => esc(r.device_name || (r.method === 'manual' ? 'บันทึกโดย HR' : '-')) },
        ], rows);
      } else {
        el.querySelector('[data-stats]').innerHTML = '';
        const rows = await get('/attendance/unmatched');
        body.innerHTML = `<div class="panel-body small muted">รหัสจากเครื่องสแกนที่ยังไม่ผูกกับพนักงาน ให้ใส่รหัสนี้ในช่อง "รหัสในเครื่องสแกน" ของพนักงาน แล้วระบบจะเชื่อมประวัติให้อัตโนมัติ</div>${table([
          { key: 'attendance_ref', label: 'รหัสเครื่องสแกน' }, { key: 'count', label: 'จำนวนครั้ง', align: 'r' }, { label: 'สแกนล่าสุด', render: (r) => fmtDateTime(r.last_scan_at) },
        ], rows, { empty: 'ทุกรหัสถูกจับคู่แล้ว' })}`;
      }
    } catch (e) { showError(e); }
  };
  el.querySelector('.tabs').addEventListener('click', (e) => {
    const b = e.target.closest('[data-tab]'); if (!b) return;
    tab = b.dataset.tab;
    el.querySelectorAll('.tabs button').forEach((x) => x.classList.toggle('active', x === b));
    load();
  });
  el.querySelector('[data-date]').addEventListener('change', load);
  el.querySelector('[data-from]').addEventListener('change', load);
  el.querySelector('[data-dept]').addEventListener('change', load);
  el.querySelector('[data-manual]')?.addEventListener('click', () => formModal({
    title: 'บันทึกเวลาเข้า-ออกแทน (กรณีเครื่องสแกนมีปัญหา)',
    fields: [
      { key: 'employee_id', label: 'พนักงาน', type: 'select', required: true, options: opt.employees, numeric: true, full: true },
      { key: 'scan_time', label: 'วันเวลา', type: 'datetime-local', required: true },
      { key: 'direction', label: 'ทิศทาง', type: 'select', required: true, options: [{ value: 'in', label: 'เข้างาน' }, { value: 'out', label: 'ออกงาน' }] },
      { key: 'note', label: 'เหตุผล', type: 'textarea', required: true },
    ],
    onSubmit: async (v) => {
      await post('/attendance/manual', { ...v, scan_time: v.scan_time.replace('T', ' ') + (v.scan_time.length === 16 ? ':00' : '') });
      toast('บันทึกแล้ว (มีบันทึกใน Audit Log)'); setTimeout(load, 500);
    },
  }));
  await load();
}

export async function departments(el) {
  const canEdit = session.can('org.manage');
  el.innerHTML = '<div data-dept></div><div style="height:24px"></div><div data-branch></div>';
  await crudPage(el.querySelector('[data-dept]'), {
    title: 'Department Management', desc: 'แผนกและโครงสร้างองค์กร', endpoint: '/departments', canEdit, addLabel: 'เพิ่มแผนก',
    columns: [{ key: 'name', label: 'แผนก' }, { key: 'parent_name', label: 'อยู่ภายใต้' }, { key: 'manager_name', label: 'หัวหน้า' },
      { key: 'branch_name', label: 'สาขา' }, { key: 'employee_count', label: 'พนักงาน', align: 'r' }],
    fields: [{ key: 'name', label: 'ชื่อแผนก', required: true }, { key: 'branch_id', label: 'สาขา', type: 'select', options: opt.branches, numeric: true },
      { key: 'parent_id', label: 'อยู่ภายใต้แผนก', type: 'select', options: opt.departments, numeric: true, placeholder: '- ระดับบนสุด -' },
      { key: 'manager_employee_id', label: 'หัวหน้าแผนก (ผู้อนุมัติคำขอ)', type: 'select', options: opt.employees, numeric: true }],
  });
  await crudPage(el.querySelector('[data-branch]'), {
    title: 'สาขา / อาคาร', endpoint: '/branches', canEdit, canDelete: session.can('org.manage'), addLabel: 'เพิ่มสาขา',
    columns: [{ key: 'name', label: 'สาขา' }, { key: 'address', label: 'ที่อยู่' }],
    fields: [{ key: 'name', label: 'ชื่อสาขา', required: true }, { key: 'address', label: 'ที่อยู่', full: true }],
  });
}
