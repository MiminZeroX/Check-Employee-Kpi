import { get, qs, live as liveState, onLive } from '../api.js';
import { esc, icon, donut, timeline, fmtDur, fmtDate, ago, today, STATES, STATE_KEYS } from '../ui.js';
import { cctvTile, drawOverlays, bindFullscreen, checkStreams } from './live.js';

const AT_DESK = ['focused', 'present', 'phone', 'looking_away'];

export async function dashboard(el) {
  const [ov, cams, ws, tl] = await Promise.all([
    get('/dashboard/overview'), get('/cameras'), get('/workstations'), get(`/activity/timeline${qs({ date: today() })}`), checkStreams(),
  ]);
  const atDeskNow = () => [...liveState.desks.values()].filter((d) => AT_DESK.includes(d.state)).length;
  const secs = ov.today_seconds;
  const totalDesk = AT_DESK.reduce((a, k) => a + (secs[k] || 0), 0);

  el.innerHTML = `
  <div class="page-head"><div><h1>Executive Dashboard</h1><p>ภาพรวมวันที่ ${fmtDate(today())}</p></div><span class="spacer"></span>
    <a class="btn" href="#/live">${icon('cctv')} Live CCTV</a><a class="btn primary" href="#/reports">${icon('print')} Export A4 Report</a></div>

  <div class="grid g-4" style="margin-bottom:16px">
    <div class="panel stat ${ov.cameras.online < ov.cameras.total ? 'warn' : 'ok'}"><div class="ic">${icon('camera')}</div><div><b>${ov.cameras.online}/${ov.cameras.total}</b><span>กล้องออนไลน์ · AI ${ov.cameras.ai_enabled} ตัว</span></div></div>
    <div class="panel stat"><div class="ic">${icon('clock')}</div><div><b>${ov.scanned_today}/${ov.employees}</b><span>สแกนเข้างานวันนี้</span></div></div>
    <div class="panel stat ok"><div class="ic">${icon('users')}</div><div><b data-atdesk>${atDeskNow()}/${ov.workstations}</b><span>อยู่ที่โต๊ะขณะนี้</span></div></div>
    <div class="panel stat ${ov.pending_corrections ? 'violet' : ''}"><div class="ic">${icon('heart')}</div><div><b>${ov.wellness_alerts_today}</b><span>แจ้งเตือนสุขภาพวันนี้ · รอตรวจสอบ ${ov.pending_corrections}</span></div></div>
  </div>

  <div class="grid g-main" style="margin-bottom:16px">
    <div class="panel"><div class="panel-head"><h2>Live CCTV</h2><span class="sub">${cams.length} กล้อง</span><span class="spacer"></span><a class="btn sm" href="#/live">ดูทั้งหมด</a></div>
      <div class="panel-body" data-grid>${cams.length ? `<div class="cctv n4">${cams.slice(0, 4).map((c) => cctvTile(c)).join('')}${'<div class="tile empty-slot">ว่าง</div>'.repeat(Math.max(0, 4 - cams.length))}</div>` : '<div class="empty">ยังไม่มีกล้อง <a href="#/cameras">เพิ่มกล้อง</a></div>'}</div></div>
    <div class="stack">
      <div class="panel"><div class="panel-head"><h2>กิจกรรมวันนี้ (ทุกคน)</h2></div>
        <div class="panel-body"><div class="row" style="gap:18px;align-items:center;flex-wrap:nowrap">${donut(secs, 140)}
          <div style="flex:1;min-width:0">${STATE_KEYS.map((k) => `<div class="row small" style="justify-content:space-between;flex-wrap:nowrap"><span><i style="display:inline-block;width:9px;height:9px;border-radius:2px;background:${STATES[k].color};margin-right:6px"></i>${esc(STATES[k].long)}</span><b>${fmtDur(secs[k])}</b></div>`).join('')}
          <div class="small muted" style="margin-top:6px">อยู่ที่โต๊ะรวม ${fmtDur(totalDesk)}</div></div></div>
          <p class="notice" style="margin:12px 0 0">ข้อมูลจากกล้องเป็นข้อมูลประกอบ ไม่ใช้ตัดสินผลงานโดยอัตโนมัติ</p></div></div>
      <div class="panel"><div class="panel-head"><h2>เหตุการณ์ล่าสุด</h2><span class="spacer"></span><a class="btn sm" href="#/notifications">ทั้งหมด</a></div>
        <div class="panel-body flush">${ov.recent_notifications.length ? ov.recent_notifications.map((n) => `<div style="padding:10px 16px;border-bottom:1px solid var(--line)">
          <div class="row" style="flex-wrap:nowrap"><b class="small">${esc(n.title)}</b><span class="spacer"></span><span class="small muted nowrap">${ago(n.created_at)}</span></div>
          <div class="small muted">${esc(n.employee_name || '')} ${esc(n.body || '')}</div></div>`).join('') : '<div class="empty">ไม่มีเหตุการณ์</div>'}</div></div>
    </div>
  </div>

  <div class="panel"><div class="panel-head"><h2>Activity Timeline วันนี้</h2><span class="sub">แถบสีคือช่วงกิจกรรม · เส้นตั้งคือเวลาสแกนเข้า-ออก</span><span class="spacer"></span><a class="btn sm" href="#/analytics">วิเคราะห์เพิ่มเติม</a></div>
    <div class="panel-body">${timeline(tl)}</div></div>`;

  const grid = el.querySelector('[data-grid]');
  bindFullscreen(grid);
  drawOverlays(grid, ws);
  const off = onLive((m) => {
    if (m.type === 'live') {
      drawOverlays(grid, ws);
      el.querySelector('[data-atdesk]').textContent = `${atDeskNow()}/${ov.workstations}`;
    }
  });
  return () => off();
}

