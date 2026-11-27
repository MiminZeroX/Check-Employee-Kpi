import { get, post, qs, session } from '../api.js';
import { esc, icon, crudPage, opt, table, fmtTime, fmtDur, fmtDateTime, today, toast, showError, formModal } from '../ui.js';

export async function employees(el) {
  const canEdit = session.is('hr');
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
      { label: 'เวลางาน', hideSm: true, render: (r) => `${r.work_start.slice(0, 5)}-${r.work_end.slice(0, 5)}` },
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
      { key: 'work_start', label: 'เวลาเริ่มงาน', type: 'time', default: '08:30' },
      { key: 'work_end', label: 'เวลาเลิกงาน', type: 'time', default: '17:30' },
      { key: 'monitoring_consent', label: 'พนักงานรับทราบและยินยอมการวิเคราะห์ข้อมูลตามนโยบาย PDPA ขององค์กร', type: 'checkbox', full: true },
    ],
  });
}

export async function attendance(el) {
  el.innerHTML = `<div class="page-head"><div><h1>Attendance Management</h1><p>เปรียบเทียบเวลาสแกนนิ้ว/สแกนหน้า กับเวลาที่มาถึงโต๊ะทำงานจริง</p></div><span class="spacer"></span>
    <input type="date" class="input" data-date value="${today()}" style="width:160px">
    <select class="input" data-dept style="width:180px"><option value="">ทุกแผนก</option></select>
    ${session.is('hr') ? `<button class="btn" data-manual>${icon('plus')} บันทึกเวลาแทน</button>` : ''}</div>
    <div class="grid g-4" style="margin-bottom:16px" data-stats></div>
    <div class="tabs"><button class="active" data-tab="daily">สรุปรายวัน</button><button data-tab="logs">ประวัติการสแกน</button>${session.is('hr') ? '<button data-tab="unmatched">รหัสที่ยังไม่จับคู่</button>' : ''}</div>
    <div class="panel" data-body></div>`;
  const depts = await opt.departments();
  el.querySelector('[data-dept]').insertAdjacentHTML('beforeend', depts.map((d) => `<option value="${d.value}">${esc(d.label)}</option>`).join(''));
  let tab = 'daily';
  const body = el.querySelector('[data-body]');
  const statuses = { on_time: '<span class="badge ok">ตรงเวลา</span>', late: '<span class="badge warn">มาสาย</span>', absent: '<span class="badge">ไม่พบการสแกน</span>' };
  const load = async () => {
    const date = el.querySelector('[data-date]').value;
    const dept = el.querySelector('[data-dept]').value;
    try {
      if (tab === 'daily') {
        const d = await get(`/attendance/daily${qs({ date, department_id: dept })}`);
        const s2d = d.rows.filter((r) => r.scan_to_desk_sec != null).map((r) => r.scan_to_desk_sec);
        el.querySelector('[data-stats]').innerHTML = `
          <div class="panel stat ok"><div class="ic">${icon('users')}</div><div><b>${d.present}/${d.rows.length}</b><span>มาทำงาน</span></div></div>
          <div class="panel stat warn"><div class="ic">${icon('clock')}</div><div><b>${d.late}</b><span>มาสาย</span></div></div>
          <div class="panel stat"><div class="ic">${icon('floor')}</div><div><b>${s2d.length ? fmtDur(s2d.reduce((a, b) => a + b, 0) / s2d.length) : '-'}</b><span>สแกน → ถึงโต๊ะ (เฉลี่ย)</span></div></div>
          <div class="panel stat violet"><div class="ic">${icon('chart')}</div><div><b>${s2d.length ? fmtDur(Math.max(...s2d)) : '-'}</b><span>สแกน → ถึงโต๊ะ (นานสุด)</span></div></div>`;
        body.innerHTML = table([
          { key: 'employee_code', label: 'รหัส' }, { key: 'name', label: 'ชื่อ' }, { key: 'department', label: 'แผนก', hideSm: true },
          { label: 'เริ่มงาน', render: (r) => r.work_start.slice(0, 5) },
          { label: 'สแกนเข้า', render: (r) => fmtTime(r.first_scan_at) },
          { label: 'ถึงโต๊ะ', render: (r) => fmtTime(r.first_desk_at) },
          { label: 'สแกน→โต๊ะ', align: 'r', render: (r) => (r.scan_to_desk_sec != null ? fmtDur(r.scan_to_desk_sec) : '-') },
          { label: 'สแกนออก', render: (r) => (r.last_scan_at && r.last_scan_at !== r.first_scan_at ? fmtTime(r.last_scan_at) : '-') },
          { label: 'สาย', align: 'r', render: (r) => (r.late_min ? `${r.late_min} นาที` : '-') },
          { label: 'สถานะ', render: (r) => statuses[r.status] },
        ], d.rows);
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
  const canEdit = session.is('hr');
  el.innerHTML = '<div data-dept></div><div style="height:24px"></div><div data-branch></div>';
  await crudPage(el.querySelector('[data-dept]'), {
    title: 'Department Management', desc: 'แผนกและโครงสร้างองค์กร', endpoint: '/departments', canEdit, addLabel: 'เพิ่มแผนก',
    columns: [{ key: 'name', label: 'แผนก' }, { key: 'branch_name', label: 'สาขา' }, { key: 'employee_count', label: 'พนักงาน', align: 'r' }],
    fields: [{ key: 'name', label: 'ชื่อแผนก', required: true }, { key: 'branch_id', label: 'สาขา', type: 'select', options: opt.branches, numeric: true }],
  });
  await crudPage(el.querySelector('[data-branch]'), {
    title: 'สาขา / อาคาร', endpoint: '/branches', canEdit, canDelete: session.is(), addLabel: 'เพิ่มสาขา',
    columns: [{ key: 'name', label: 'สาขา' }, { key: 'address', label: 'ที่อยู่' }],
    fields: [{ key: 'name', label: 'ชื่อสาขา', required: true }, { key: 'address', label: 'ที่อยู่', full: true }],
  });
}
