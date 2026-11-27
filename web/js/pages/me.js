import { get, post, qs, openHtml, session } from '../api.js';
import {
  esc, icon, table, opt, stateBar, stateBadge, rangePicker, fmtDur, fmtHours, fmtTime, fmtDate, fmtDateTime, ago,
  daysAgo, today, formModal, toast, showError, STATES,
} from '../ui.js';
import { taskColumns } from './work.js';
import { requestColumns, requestForm } from './hr.js';

const secsOf = (r) => ({ focused: r.focused_sec, present: r.present_sec, phone: r.phone_sec, looking_away: r.looking_away_sec, away: r.away_sec, unknown: r.unknown_sec });

export async function me(el) {
  el.innerHTML = `<div class="page-head"><div><h1>สวัสดี ${esc(session.user.name)}</h1><p>ข้อมูลของคุณที่ระบบบันทึก · หากพบข้อมูลคลาดเคลื่อน กด "ขอแก้ไข" ได้ทันที</p></div><span class="spacer"></span>
    <div data-range></div><button class="btn primary" data-a4>${icon('print')} รายงาน A4 ของฉัน</button></div>
    ${session.config.monitoring_notice ? `<p class="notice">${esc(session.config.monitoring_notice)}</p>` : ''}
    <div class="grid g-4" style="margin:16px 0" data-cards></div>
    <div class="grid g-2" style="margin-bottom:16px">
      <div class="panel"><div class="panel-head"><h2>KPI ของฉัน</h2></div><div data-kpi></div></div>
      <div class="panel"><div class="panel-head"><h2>การแจ้งเตือน</h2></div><div data-notif></div></div></div>
    <div class="panel" style="margin-bottom:16px"><div class="panel-head"><h2>การลาและคำขอ</h2><span class="spacer"></span>
      <button class="btn sm" data-req="leave">${icon('plus')} ขอลา</button><button class="btn sm" data-req="overtime">${icon('plus')} ขอ OT</button><button class="btn sm" data-req="offsite">${icon('plus')} นอกสถานที่</button></div>
      <div data-balance></div><div data-requests></div></div>
    <div class="panel" style="margin-bottom:16px"><div class="panel-head"><h2>เวลาทำงานรายวัน</h2><span class="sub">ตามกะ วันหยุด และการลาที่อนุมัติแล้ว</span></div><div data-sessions></div></div>
    <div class="panel" style="margin-bottom:16px"><div class="panel-head"><h2>สรุปกิจกรรมรายวัน</h2></div><div data-daily></div></div>
    <div class="panel" style="margin-bottom:16px"><div class="panel-head"><h2>เหตุการณ์ที่ระบบตรวจพบ</h2><span class="spacer"></span><input type="date" class="input" data-day value="${today()}" style="width:160px"></div><div data-events></div></div>
    <div class="panel"><div class="panel-head"><h2>คำขอแก้ไขข้อมูลของฉัน</h2></div><div data-corr></div></div>`;
  const range = rangePicker(el.querySelector('[data-range]'), () => { loadRange(); loadRequests(); }, { from: daysAgo(29), to: today() });
  let events = [];

  const loadRange = async () => {
    const { from, to } = range.get();
    try {
      const [s, days, k] = await Promise.all([get(`/me/summary${qs({ from, to })}`), get(`/me/daily${qs({ from, to })}`), get(`/kpi/scores${qs({ from, to })}`)]);
      const r = s.summary;
      const atDesk = r ? r.focused_sec + r.present_sec + r.phone_sec + r.looking_away_sec : 0;
      const kp = k.rows[0];
      el.querySelector('[data-cards]').innerHTML = `
        <div class="panel stat"><div class="ic">${icon('kpi')}</div><div><b>${kp?.total != null ? kp.total.toFixed(1) : '-'}</b><span>คะแนน KPI · ${esc(kp?.grade || 'ข้อมูลไม่พอ')}</span></div></div>
        <div class="panel stat ok"><div class="ic">${icon('clock')}</div><div><b>${days.filter((d) => d.first_scan_at).length} วัน</b><span>มาทำงาน</span></div></div>
        <div class="panel stat"><div class="ic">${icon('floor')}</div><div><b>${fmtHours(atDesk)}</b><span>อยู่ที่โต๊ะรวม</span></div></div>
        <div class="panel stat warn"><div class="ic">${icon('heart')}</div><div><b>${r ? fmtDur(r.longest_sitting_sec) : '-'}</b><span>นั่งต่อเนื่องนานสุด · ควรพักทุก 50-60 นาที</span></div></div>`;
      el.querySelector('[data-kpi]').innerHTML = kp ? table([
        { key: 'name', label: 'ตัวชี้วัด' }, { label: 'น้ำหนัก', align: 'r', render: (i) => `${i.weight}%` },
        { label: 'คะแนน', align: 'r', render: (i) => (i.score == null ? '-' : i.score.toFixed(1)) }, { key: 'detail', label: 'รายละเอียด' },
      ], kp.items) : '<div class="empty">ไม่มีข้อมูล</div>';
      el.querySelector('[data-daily]').innerHTML = table([
        { label: 'วันที่', render: (d) => fmtDate(d.work_date) }, { label: 'สแกนเข้า', render: (d) => fmtTime(d.first_scan_at) },
        { label: 'ถึงโต๊ะ', render: (d) => fmtTime(d.first_desk_at) }, { label: 'สแกนออก', render: (d) => fmtTime(d.last_scan_at) },
        { label: 'อยู่ที่โต๊ะ', align: 'r', render: (d) => fmtHours(d.focused_sec + d.present_sec + d.phone_sec + d.looking_away_sec) },
        { label: 'สัดส่วน', render: (d) => stateBar(secsOf(d)) },
      ], days);
    } catch (e) { showError(e); }
  };
  const loadDay = async () => {
    const d = el.querySelector('[data-day]').value;
    events = await get(`/me/events${qs({ from: d, to: d, limit: 500 })}`);
    el.querySelector('[data-events]').innerHTML = table([
      { label: 'เวลา', render: (e) => `${fmtTime(e.started_at)} - ${fmtTime(e.ended_at)}` }, { label: 'สถานะ', render: (e) => stateBadge(e.state) },
      { label: 'ระยะเวลา', align: 'r', render: (e) => fmtDur(e.duration_sec) }, { label: 'ความมั่นใจ', align: 'r', render: (e) => `${Math.round(e.confidence * 100)}%` },
      { label: 'ตรวจสอบ', render: (e) => (e.review_status === 'auto' ? '' : `<span class="badge warn">${esc({ disputed: 'รอพิจารณา', corrected: 'แก้ไขแล้ว', rejected: 'ลบแล้ว' }[e.review_status] || e.review_status)}</span>`) },
    ], events, { empty: 'ไม่มีเหตุการณ์ในวันนี้', actions: (e) => (e.review_status === 'auto' ? `<button class="btn sm" data-fix="${e.id}">ขอแก้ไข</button>` : '') });
  };
  const loadRequests = async () => {
    const { from, to } = range.get();
    const [bal, reqs, sess] = await Promise.all([get('/leave-balances'), get('/requests?mine=true'), get(`/me/work-sessions${qs({ from, to })}`)]);
    el.querySelector('[data-balance]').innerHTML = `<div class="row" style="padding:12px 16px;gap:10px">${bal.rows.map((b) => `<div class="badge" style="padding:6px 12px">${esc(b.name)}: <b>&nbsp;${b.remaining == null ? 'ไม่จำกัด' : `เหลือ ${b.remaining} วัน`}</b>&nbsp;<span class="muted">(ใช้ ${b.used}${b.pending ? ` · รอ ${b.pending}` : ''})</span></div>`).join('')}</div>`;
    el.querySelector('[data-requests]').innerHTML = table(requestColumns(false), reqs, {
      empty: 'ยังไม่มีคำขอ',
      actions: (r) => (r.status === 'pending' ? `<button class="btn sm" data-cancel="${r.id}">ยกเลิก</button>` : ''),
    });
    const ST = { present: 'มาทำงาน', leave: 'ลา', offsite: 'นอกสถานที่', holiday: 'วันหยุด', dayoff: 'วันหยุดประจำ', absent: 'ขาดงาน', not_yet: 'ยังไม่ถึงเวลา' };
    el.querySelector('[data-sessions]').innerHTML = table([
      { label: 'วันที่', render: (r) => fmtDate(r.work_date) }, { label: 'กะ', render: (r) => esc(r.shift_name || '-') },
      { label: 'เข้า-ออก', render: (r) => `${fmtTime(r.check_in)} - ${fmtTime(r.check_out)}` },
      { label: 'ทำงาน', align: 'r', render: (r) => (r.worked_sec ? fmtDur(r.worked_sec) : '-') },
      { label: 'สาย', align: 'r', render: (r) => (r.late_min ? `${r.late_min} นาที` : '-') },
      { label: 'OT', align: 'r', render: (r) => (r.overtime_min ? fmtDur(r.overtime_min * 60) : '-') },
      { label: 'สถานะ', render: (r) => esc(ST[r.status] || r.status) },
    ], sess, { empty: 'ไม่มีข้อมูล' });
  };
  el.addEventListener('click', async (e) => {
    const b = e.target.closest('button'); if (!b) return;
    try {
      if (b.dataset.req) requestForm({ kind: b.dataset.req, onDone: loadRequests });
      if (b.dataset.cancel) { await post(`/requests/${b.dataset.cancel}/cancel`); toast('ยกเลิกคำขอแล้ว'); loadRequests(); }
    } catch (err) { showError(err); }
  });
  const loadOther = async () => {
    const [corr, notif] = await Promise.all([get('/me/corrections'), get('/me/notifications')]);
    el.querySelector('[data-corr]').innerHTML = table([
      { label: 'วันที่ขอ', render: (r) => fmtDateTime(r.created_at) }, { label: 'เหตุการณ์', render: (r) => `${stateBadge(r.event_state)} ${fmtTime(r.event_started_at)}` },
      { key: 'reason', label: 'เหตุผล' }, { label: 'ผล', render: (r) => `<span class="badge ${{ pending: 'warn', approved: 'ok', rejected: 'bad' }[r.status]}">${{ pending: 'รอพิจารณา', approved: 'อนุมัติ', rejected: 'ไม่อนุมัติ' }[r.status]}</span>` },
      { key: 'review_note', label: 'หมายเหตุ' },
    ], corr, { empty: 'ยังไม่มีคำขอ' });
    el.querySelector('[data-notif]').innerHTML = notif.length ? notif.slice(0, 8).map((n) => `<div style="padding:10px 16px;border-bottom:1px solid var(--line)"><div class="row"><b class="small">${esc(n.title)}</b><span class="spacer"></span><span class="small muted">${ago(n.created_at)}</span></div><div class="small muted">${esc(n.body || '')}</div></div>`).join('') : '<div class="empty">ไม่มีการแจ้งเตือน</div>';
  };
  el.querySelector('[data-events]').addEventListener('click', (e) => {
    const b = e.target.closest('[data-fix]'); if (!b) return;
    const ev = events.find((x) => String(x.id) === b.dataset.fix);
    formModal({
      title: `ขอแก้ไข: ${STATES[ev.state]?.long} ${fmtTime(ev.started_at)}-${fmtTime(ev.ended_at)}`,
      fields: [
        { key: 'requested_state', label: 'สถานะที่ถูกต้อง', type: 'select', options: opt.states, placeholder: '- ข้อมูลนี้ไม่ถูกต้อง ให้ลบออก -', full: true },
        { key: 'reason', label: 'เหตุผล เช่น ประชุมกับลูกค้า / ไปห้องเอกสาร', type: 'textarea', required: true },
      ],
      submitLabel: 'ส่งคำขอ',
      onSubmit: async (v) => { await post('/me/corrections', { ...v, activity_event_id: ev.id }); toast('ส่งคำขอแล้ว ฝ่ายบุคคลจะพิจารณา'); loadDay(); loadOther(); },
    });
  });
  el.querySelector('[data-day]').addEventListener('change', () => loadDay().catch(showError));
  el.querySelector('[data-a4]').addEventListener('click', () => {
    const { from, to } = range.get();
    openHtml(`/reports/employee/${session.user.employee_id}${qs({ from, to })}`).catch(showError);
  });
  await Promise.all([loadRange(), loadDay(), loadOther(), loadRequests()]);
}

export async function myTasks(el) {
  el.innerHTML = `<div class="page-head"><div><h1>งานของฉัน</h1><p>อัปเดตสถานะงานและ Ticket ที่ได้รับมอบหมาย</p></div></div>
    <div class="panel" style="margin-bottom:16px"><div class="panel-head"><h2>งาน</h2></div><div data-body></div></div>
    <div class="panel"><div class="panel-head"><h2>Ticket บริการลูกค้า</h2></div><div data-tickets></div></div>`;
  const { ticketColumns } = await import('./business.js');
  const TS = { open: 'เปิด', in_progress: 'กำลังดำเนินการ', waiting: 'รอลูกค้า', resolved: 'แก้ไขแล้ว', closed: 'ปิด' };
  const load = async () => {
    const [rows, tks] = await Promise.all([get('/tasks'), get('/tickets?mine=true')]);
    el.querySelector('[data-body]').innerHTML = table(taskColumns(), rows, {
      empty: 'ไม่มีงาน',
      actions: (r) => (r.status === 'done' || r.status === 'cancelled' ? '' : `<select class="input" data-status="${r.id}" aria-label="สถานะงาน" style="height:28px">${['todo', 'in_progress', 'done'].map((s) => `<option value="${s}" ${s === r.status ? 'selected' : ''}>${{ todo: 'รอดำเนินการ', in_progress: 'กำลังทำ', done: 'เสร็จแล้ว' }[s]}</option>`).join('')}</select>`),
    });
    el.querySelector('[data-tickets]').innerHTML = table(ticketColumns, tks, {
      empty: 'ไม่มี Ticket',
      actions: (t) => (t.status === 'closed' ? '' : `<select class="input" data-ticket="${t.id}" aria-label="สถานะ Ticket" style="height:28px">${Object.entries(TS).map(([k, v]) => `<option value="${k}" ${k === t.status ? 'selected' : ''}>${v}</option>`).join('')}</select>`),
    });
  };
  el.addEventListener('change', async (e) => {
    const s = e.target.closest('[data-status]'); const t = e.target.closest('[data-ticket]');
    try {
      if (s) { await post(`/tasks/${s.dataset.status}/status`, { status: s.value }); toast('อัปเดตสถานะแล้ว'); load(); }
      if (t) { await post(`/tickets/${t.dataset.ticket}/status`, { status: t.value }); toast('อัปเดต Ticket แล้ว'); load(); }
    } catch (err) { showError(err); }
  });
  await load();
}
