import { get, post, qs, download, openHtml, session } from '../api.js';
import {
  esc, icon, table, opt, timeline, stateBar, legend, rangePicker, fmtDur, fmtHours, fmtTime, fmtDate, fmtDateTime,
  ago, daysAgo, today, modal, showError, stateBadge,
} from '../ui.js';

const secsOf = (r) => ({ focused: r.focused_sec, present: r.present_sec, phone: r.phone_sec, looking_away: r.looking_away_sec, away: r.away_sec, unknown: r.unknown_sec });
const atDesk = (r) => r.focused_sec + r.present_sec + r.phone_sec + r.looking_away_sec;

async function employeeDetail(row, from, to) {
  const [days, events] = await Promise.all([
    get(`/activity/daily${qs({ employee_id: row.employee_id, from, to })}`),
    get(`/activity/events${qs({ employee_id: row.employee_id, from: to, to, limit: 200 })}`),
  ]);
  modal({
    title: `${row.name} · ${fmtDate(from)} - ${fmtDate(to)}`,
    wide: true,
    body: `<h3 style="margin-bottom:8px">สรุปรายวัน</h3>${table([
      { label: 'วันที่', render: (d) => fmtDate(d.work_date) },
      { label: 'สแกนเข้า', render: (d) => fmtTime(d.first_scan_at) }, { label: 'ถึงโต๊ะ', render: (d) => fmtTime(d.first_desk_at) },
      { label: 'สแกน→โต๊ะ', align: 'r', render: (d) => (d.scan_to_desk_sec != null ? fmtDur(d.scan_to_desk_sec) : '-') },
      { label: 'อยู่ที่โต๊ะ', align: 'r', render: (d) => fmtHours(atDesk(d)) },
      { label: 'สัดส่วน', render: (d) => stateBar(secsOf(d)) },
      { label: 'มือถือ', align: 'r', render: (d) => `${d.phone_count} ครั้ง` },
      { label: 'นั่งนานสุด', align: 'r', render: (d) => fmtDur(d.longest_sitting_sec) },
    ], days)}
    <h3 style="margin:18px 0 8px">เหตุการณ์ของวันที่ ${fmtDate(to)}</h3>${table([
      { label: 'เวลา', render: (e) => `${fmtTime(e.started_at)} - ${fmtTime(e.ended_at)}` }, { label: 'สถานะ', render: (e) => stateBadge(e.state) },
      { label: 'ระยะเวลา', align: 'r', render: (e) => fmtDur(e.duration_sec) }, { label: 'ความมั่นใจ AI', align: 'r', render: (e) => `${Math.round(e.confidence * 100)}%` },
      { key: 'workstation_name', label: 'โต๊ะ' }, { label: 'ตรวจสอบ', render: (e) => (e.review_status === 'auto' ? '-' : `<span class="badge warn">${esc(e.review_status)}</span>`) },
    ], events, { empty: 'ไม่มีเหตุการณ์' })}`,
  });
}

export async function analytics(el) {
  el.innerHTML = `<div class="page-head"><div><h1>Activity Analytics</h1><p>วิเคราะห์ระยะเวลากิจกรรมรายบุคคล รายแผนก และภาพรวม จากข้อมูลที่ AI ตรวจพบ</p></div><span class="spacer"></span>
    <div data-range></div><select class="input" data-dept style="width:170px"><option value="">ทุกแผนก</option></select>
    ${session.is('hr', 'manager') ? `<button class="btn" data-csv>${icon('download')} CSV</button>` : ''}</div>
    <div class="panel" style="margin-bottom:16px"><div class="panel-head"><h2>สรุปตามพนักงาน</h2><span class="sub">คลิกที่แถวเพื่อดูรายละเอียด</span><span class="spacer"></span>${legend()}</div><div data-summary></div></div>
    <div class="panel"><div class="panel-head"><h2>Timeline รายวัน</h2><span class="spacer"></span><input type="date" class="input" data-day value="${today()}" style="width:160px"></div><div class="panel-body" data-tl></div></div>
    <p class="notice" style="margin-top:16px">การมองออกจากจอ การใช้โทรศัพท์ หรือการไม่อยู่ที่โต๊ะ อาจเป็นส่วนหนึ่งของงาน (ประชุม ติดต่อลูกค้า ค้นเอกสาร) ข้อมูลนี้จึงใช้ดูแนวโน้มและการจัดสรรพื้นที่ ไม่ใช้ตัดสินผลงานรายบุคคลโดยอัตโนมัติ</p>`;
  (await opt.departments()).forEach((d) => el.querySelector('[data-dept]').insertAdjacentHTML('beforeend', `<option value="${d.value}">${esc(d.label)}</option>`));
  const range = rangePicker(el.querySelector('[data-range]'), () => loadSummary());
  let rows = [];
  const loadSummary = async () => {
    const { from, to } = range.get();
    try {
      const d = await get(`/activity/summary${qs({ from, to, department_id: el.querySelector('[data-dept]').value })}`);
      rows = d.rows;
      el.querySelector('[data-summary]').innerHTML = table([
        { key: 'employee_code', label: 'รหัส' }, { key: 'name', label: 'ชื่อ' }, { key: 'department_name', label: 'แผนก', hideSm: true },
        { key: 'days', label: 'วัน', align: 'r' },
        { label: 'อยู่ที่โต๊ะ', align: 'r', render: (r) => fmtHours(atDesk(r)) },
        { label: 'มองหน้าจอ', align: 'r', render: (r) => fmtHours(r.focused_sec) },
        { label: 'มือถือ', align: 'r', hideSm: true, render: (r) => `${fmtDur(r.phone_sec)} · ${r.phone_count} ครั้ง` },
        { label: 'มองทางอื่น', align: 'r', hideSm: true, render: (r) => fmtDur(r.looking_away_sec) },
        { label: 'ไม่อยู่โต๊ะ', align: 'r', hideSm: true, render: (r) => `${fmtDur(r.away_sec)} · ${r.away_count} ครั้ง` },
        { label: 'สแกน→โต๊ะ', align: 'r', hideSm: true, render: (r) => (r.avg_scan_to_desk_sec != null ? fmtDur(r.avg_scan_to_desk_sec) : '-') },
        { label: 'สัดส่วน', render: (r) => stateBar(secsOf(r)) },
      ], rows);
    } catch (e) { showError(e); }
  };
  const loadTimeline = async () => {
    try {
      const d = await get(`/activity/timeline${qs({ date: el.querySelector('[data-day]').value, department_id: el.querySelector('[data-dept]').value })}`);
      el.querySelector('[data-tl]').innerHTML = timeline(d);
    } catch (e) { showError(e); }
  };
  el.querySelector('[data-summary]').addEventListener('click', (e) => {
    const tr = e.target.closest('tr[data-i]');
    if (!tr) return;
    const { from, to } = range.get();
    employeeDetail(rows[Number(tr.dataset.i)], from, to).catch(showError);
  });
  el.querySelector('[data-dept]').addEventListener('change', () => { loadSummary(); loadTimeline(); });
  el.querySelector('[data-day]').addEventListener('change', loadTimeline);
  el.querySelector('[data-csv]')?.addEventListener('click', () => {
    const { from, to } = range.get();
    download(`/reports/summary.csv${qs({ from, to, department_id: el.querySelector('[data-dept]').value })}`, 'summary.csv').catch(showError);
  });
  await Promise.all([loadSummary(), loadTimeline()]);
}

export async function wellness(el) {
  el.innerHTML = `<div class="page-head"><div><h1>Employee Health & Wellness</h1><p>ส่งเสริมการพัก ลดการนั่งนาน และดูแลภาระงาน · ไม่ใช้ภาพจากกล้องวินิจฉัยสุขภาพหรืออารมณ์</p></div><span class="spacer"></span><div data-range></div></div>
    <div class="grid g-3" style="margin-bottom:16px">
      <div class="panel panel-body"><h3>หลัก 20-20-20</h3><p class="small muted" style="margin:6px 0 0">ทุก 20 นาที ให้มองไกล 20 ฟุต (6 เมตร) นาน 20 วินาที เพื่อลดอาการตาล้าจากหน้าจอ</p></div>
      <div class="panel panel-body"><h3>ลุกเดินทุก 50-60 นาที</h3><p class="small muted" style="margin:6px 0 0">ระบบแจ้งเตือนอัตโนมัติเมื่อนั่งต่อเนื่องเกินเวลาที่ตั้งไว้ (ตั้งค่าได้ที่ System Settings)</p></div>
      <div class="panel panel-body"><h3>Work-Life Balance</h3><p class="small muted" style="margin:6px 0 0">ติดตามชั่วโมงทำงานล่วงเวลาจากเวลาสแกนออก เพื่อกระจายภาระงานอย่างเหมาะสม</p></div>
    </div>
    <div class="panel" data-body></div>`;
  const LEVEL = { good: '<span class="badge ok">ปกติ</span>', watch: '<span class="badge warn">ควรติดตาม</span>', attention: '<span class="badge bad">ควรพูดคุย</span>' };
  const load = async () => {
    const { from, to } = range.get();
    try {
      const d = await get(`/wellness/overview${qs({ from, to })}`);
      el.querySelector('[data-body]').innerHTML = table([
        { key: 'employee_code', label: 'รหัส' }, { key: 'name', label: 'ชื่อ' }, { key: 'department', label: 'แผนก', hideSm: true },
        { key: 'days', label: 'วันทำงาน', align: 'r' },
        { label: 'นั่งต่อเนื่องนานสุด', align: 'r', render: (r) => fmtDur(r.longest_sitting_sec) },
        { label: 'เฉลี่ยนั่งนานสุด/วัน', align: 'r', hideSm: true, render: (r) => (r.avg_longest_sitting_sec ? fmtDur(r.avg_longest_sitting_sec) : '-') },
        { key: 'wellness_alerts', label: 'แจ้งเตือนให้พัก', align: 'r' },
        { label: 'ทำงานเกินเวลา (รวม)', align: 'r', render: (r) => fmtDur(r.overtime_min * 60) },
        { label: 'ระดับ', render: (r) => LEVEL[r.level] },
      ], d.rows);
    } catch (e) { showError(e); }
  };
  const range = rangePicker(el.querySelector('[data-range]'), load, { from: daysAgo(29), to: today() });
  await load();
}

export async function reports(el) {
  el.innerHTML = `<div class="page-head"><div><h1>Report Center</h1><p>รายงาน KPI รายบุคคลขนาด A4 พร้อมพิมพ์หรือบันทึกเป็น PDF และส่งออก CSV (เปิดใน Excel ได้)</p></div></div>
    <div class="grid g-2" style="margin-bottom:16px">
      <div class="panel"><div class="panel-head"><h2>${icon('report')} รายงาน KPI รายบุคคล (A4)</h2></div><div class="panel-body stack">
        <div class="field"><label>พนักงาน</label><select class="input" data-emp></select></div>
        <div class="field"><label>ช่วงประเมิน</label><div data-range></div></div>
        <div class="row"><button class="btn primary" data-a4>${icon('print')} เปิดรายงาน A4 / PDF</button><button class="btn" data-csv>${icon('download')} CSV รายวัน</button></div>
        <p class="small muted" style="margin:0">ในหน้ารายงาน กด "พิมพ์ / บันทึกเป็น PDF" แล้วเลือกขนาดกระดาษ A4 · โลโก้ หัว-ท้ายกระดาษ ตั้งค่าได้ที่ System Settings</p></div></div>
      <div class="panel"><div class="panel-head"><h2>${icon('chart')} รายงานสรุปทั้งองค์กร</h2></div><div class="panel-body stack">
        <div class="field"><label>แผนก</label><select class="input" data-dept><option value="">ทุกแผนก</option></select></div>
        <div class="row"><button class="btn" data-sum>${icon('download')} CSV สรุป KPI และกิจกรรม</button><button class="btn" data-all>${icon('print')} เปิดรายงาน A4 ทุกคนในแผนก</button></div>
        <p class="small muted" style="margin:0">ไฟล์ CSV บันทึกแบบ UTF-8 BOM เปิดด้วย Excel ได้ทันทีโดยภาษาไทยไม่เพี้ยน</p></div></div>
    </div>
    <div class="panel"><div class="panel-head"><h2>ประวัติการออกรายงาน</h2></div><div data-history></div></div>`;
  const emps = await opt.employees();
  el.querySelector('[data-emp]').innerHTML = emps.map((e) => `<option value="${e.value}">${esc(e.label)}</option>`).join('');
  (await opt.departments()).forEach((d) => el.querySelector('[data-dept]').insertAdjacentHTML('beforeend', `<option value="${d.value}">${esc(d.label)}</option>`));
  const range = rangePicker(el.querySelector('[data-range]'), () => {}, { from: daysAgo(29), to: today() });
  const history = async () => {
    const rows = await get('/reports/history');
    el.querySelector('[data-history]').innerHTML = table([
      { key: 'doc_no', label: 'เลขที่เอกสาร' }, { key: 'employee_name', label: 'พนักงาน' },
      { label: 'ช่วงประเมิน', render: (r) => `${fmtDate(r.period_start)} - ${fmtDate(r.period_end)}` },
      { key: 'generated_by_name', label: 'ออกโดย' }, { label: 'เวลา', render: (r) => fmtDateTime(r.created_at) },
    ], rows, { empty: 'ยังไม่เคยออกรายงาน' });
  };
  const emp = () => el.querySelector('[data-emp]').value;
  el.querySelector('[data-a4]').addEventListener('click', async () => {
    const { from, to } = range.get();
    try { await openHtml(`/reports/employee/${emp()}${qs({ from, to })}`); history(); } catch (e) { showError(e); }
  });
  el.querySelector('[data-csv]').addEventListener('click', () => {
    const { from, to } = range.get();
    download(`/reports/employee/${emp()}/csv${qs({ from, to })}`, 'employee.csv').catch(showError);
  });
  el.querySelector('[data-sum]').addEventListener('click', () => {
    const { from, to } = range.get();
    download(`/reports/summary.csv${qs({ from, to, department_id: el.querySelector('[data-dept]').value })}`, 'summary.csv').catch(showError);
  });
  el.querySelector('[data-all]').addEventListener('click', async () => {
    const { from, to } = range.get();
    try { await openHtml(`/reports/batch${qs({ from, to, department_id: el.querySelector('[data-dept]').value })}`); history(); } catch (e) { showError(e); }
  });
  await history();
}

export async function notifications(el) {
  el.innerHTML = '<div class="page-head"><div><h1>Notification Center</h1><p>แจ้งเตือนสุขภาพ ผลการตรวจสอบข้อมูล และเหตุการณ์ระบบ</p></div></div><div class="panel" data-body></div>';
  const load = async () => {
    const rows = await get('/notifications');
    el.querySelector('[data-body]').innerHTML = table([
      { label: '', render: (r) => (r.is_read ? '' : '<span class="status-dot on"></span>') },
      { label: 'เวลา', render: (r) => `<span title="${fmtDateTime(r.created_at)}">${ago(r.created_at)}</span>` },
      { label: 'ประเภท', render: (r) => `<span class="badge ${r.kind === 'wellness_sitting' ? 'warn' : 'info'}">${esc({ wellness_sitting: 'สุขภาพ', correction_result: 'แก้ไขข้อมูล' }[r.kind] || r.kind)}</span>` },
      { key: 'employee_name', label: 'พนักงาน' }, { key: 'title', label: 'หัวข้อ' }, { key: 'body', label: 'รายละเอียด', hideSm: true },
    ], rows, { actions: (r) => (r.is_read ? '' : `<button class="btn sm" data-read="${r.id}">อ่านแล้ว</button>`), empty: 'ไม่มีการแจ้งเตือน' });
  };
  el.querySelector('[data-body]').addEventListener('click', async (e) => {
    const b = e.target.closest('[data-read]');
    if (!b) return;
    try { await post(`/notifications/${b.dataset.read}/read`); load(); } catch (err) { showError(err); }
  });
  await load();
}
