import { get, post, session, live as liveState, onLive } from '../api.js';
import { esc, icon, STATES, fmtDur, ago, legend, nowStamp, modal, formModal, showError, toast } from '../ui.js';

let streamCheck = null;
let streamOk = false;
/**
 * Opens the video session (HttpOnly cookie for /video) and checks that the stream server answers.
 * Video is always proxied by our server: the browser never talks to go2rtc directly.
 */
export function checkStreams() {
  if (!streamCheck) {
    streamCheck = (async () => {
      try {
        await post('/video/session');
        streamOk = (await get('/video/status')).available;
      } catch { streamOk = false; }
      if (!streamOk) streamCheck = null; // retry on the next page view
      return streamOk;
    })();
  }
  return streamCheck;
}

/** One CCTV tile: WebRTC/MSE stream (via the /video proxy) + anonymous AI detection-zone overlay. */
export function cctvTile(cam, { stream = true } = {}) {
  const src = `/video/stream.html?src=${encodeURIComponent(cam.stream_name)}&mode=webrtc,mse&media=video`;
  const online = cam.status === 'online';
  return `<div class="tile" data-cam="${cam.id}">
    <div class="placeholder">${icon('camera').replace('<svg', '<svg style="width:28px;height:28px;display:block;margin:0 auto 6px"')}${esc(cam.name)}<br>${!streamOk ? (window.SO_DEMO ? 'เดโม: ไม่มีภาพจากกล้องจริง<br>กรอบสีคือสถานะจาก AI' : 'เชื่อมต่อเซิร์ฟเวอร์ภาพ (go2rtc) ไม่ได้') : online ? 'กำลังเชื่อมต่อ...' : 'ไม่มีสัญญาณ'}</div>
    ${stream && streamOk ? `<iframe src="${esc(src)}" loading="lazy" allow="autoplay" title="${esc(cam.name)}"></iframe>` : ''}
    <svg class="overlay" viewBox="0 0 1000 562.5" preserveAspectRatio="none"></svg>
    ${(cam.privacy_masks || []).length ? `<svg class="overlay masks" viewBox="0 0 1000 562.5" preserveAspectRatio="none" aria-hidden="true">${cam.privacy_masks.map((p) => `<polygon points="${p.map(([x, y]) => `${x * 1000},${y * 562.5}`).join(' ')}" fill="#000"/>`).join('')}</svg>` : ''}
    <div class="cap"><span class="rec ${online ? '' : 'off'}"></span>${esc(cam.name)}<span class="spacer"></span>${cam.ai_enabled ? '<span class="badge info" style="padding:0 6px">AI</span>' : ''}</div>
    <div class="tile-tools">
      ${cam.playback_url && session.can('playback.view') ? `<button class="btn sm" data-playback="${cam.id}" title="ภาพย้อนหลัง">${icon('clock')}</button>` : ''}
      <button class="btn sm icon" data-full title="เต็มจอ">${icon('expand')}</button></div>
    ${cam.ptz_supported && session.can('cameras.manage') ? `<div class="ptz" data-ptz="${cam.id}" aria-label="ควบคุมกล้อง PTZ">
      <button class="btn sm" data-dir="0,1,0" aria-label="ขึ้น">▲</button><button class="btn sm" data-dir="-1,0,0" aria-label="ซ้าย">◀</button>
      <button class="btn sm" data-dir="1,0,0" aria-label="ขวา">▶</button><button class="btn sm" data-dir="0,-1,0" aria-label="ลง">▼</button>
      <button class="btn sm" data-dir="0,0,1" aria-label="ซูมเข้า">+</button><button class="btn sm" data-dir="0,0,-1" aria-label="ซูมออก">−</button></div>` : ''}
    <div class="stamp" data-stamp></div>
  </div>`;
}

/** Draws each workstation zone coloured by its live state. No faces or identities are drawn. */
export function drawOverlays(container, workstations) {
  container.querySelectorAll('.tile[data-cam]').forEach((tile) => {
    const camId = Number(tile.dataset.cam);
    const svg = tile.querySelector('svg.overlay');
    if (!svg) return;
    svg.innerHTML = workstations.filter((w) => w.camera_id === camId).map((w) => {
      const d = liveState.desks.get(w.id);
      const st = STATES[d?.state] || null;
      const color = st ? st.color : '#5b6b84';
      const pts = (w.roi || []).map(([x, y]) => `${x * 1000},${y * 562.5}`).join(' ');
      const [lx, ly] = w.roi?.[0] || [0, 0];
      const label = `${w.name}${st ? ` · ${st.label}` : ''}${d ? ` ${fmtDur((Date.now() - new Date(d.since)) / 1000)}` : ''}`;
      return `<polygon points="${pts}" fill="${color}" fill-opacity="0.12" stroke="${color}" stroke-width="3"/>
        <rect x="${lx * 1000 + 4}" y="${ly * 562.5 + 4}" width="${Math.min(380, label.length * 13 + 16)}" height="30" rx="4" fill="rgba(5,10,20,.75)"/>
        <text x="${lx * 1000 + 12}" y="${ly * 562.5 + 25}" fill="#fff" font-size="20">${esc(label)}</text>`;
    }).join('');
    const stamp = tile.querySelector('[data-stamp]');
    if (stamp) stamp.textContent = nowStamp();
  });
}

/** PTZ: press and hold a direction to move, release to stop. */
function bindPtz(container) {
  let moving = null;
  const stop = () => {
    if (!moving) return;
    const id = moving; moving = null;
    post(`/cameras/${id}/ptz`, { action: 'stop' }).catch(showError);
  };
  container.addEventListener('pointerdown', (e) => {
    const b = e.target.closest('[data-dir]'); if (!b) return;
    e.preventDefault();
    const id = b.closest('[data-ptz]').dataset.ptz;
    const [pan, tilt, zoom] = b.dataset.dir.split(',').map(Number);
    moving = id;
    post(`/cameras/${id}/ptz`, { action: 'move', pan: pan * 0.5, tilt: tilt * 0.5, zoom: zoom * 0.5 }).catch((err) => { moving = null; showError(err); });
  });
  ['pointerup', 'pointerleave', 'pointercancel'].forEach((ev) => container.addEventListener(ev, stop));
}

/** NVR playback: pick a time, the server opens a temporary stream through the proxy. */
function bindPlayback(container, cams) {
  container.addEventListener('click', (e) => {
    const b = e.target.closest('[data-playback]'); if (!b) return;
    const cam = cams.find((c) => String(c.id) === b.dataset.playback);
    const d = new Date(Date.now() - 3600e3 + (session.config.tz_offset_min ?? 420) * 60e3).toISOString().slice(0, 16);
    formModal({
      title: `ภาพย้อนหลัง: ${cam.name}`,
      fields: [{ key: 'start', label: 'เริ่มที่เวลา', type: 'datetime-local', required: true, default: d },
        { key: 'minutes', label: 'ความยาว (นาที)', type: 'number', min: 1, max: 240, default: 30 }],
      submitLabel: 'เปิดภาพย้อนหลัง',
      onSubmit: async (v) => {
        const r = await post(`/cameras/${cam.id}/playback`, { start: v.start.replace('T', ' ') + ':00', minutes: v.minutes || 30 });
        const m = modal({ title: `ย้อนหลัง ${cam.name} · ${v.start.replace('T', ' ')}`, wide: true,
          body: `<div class="cctv n1"><div class="tile"><iframe src="/video/stream.html?src=${encodeURIComponent(r.stream)}&mode=webrtc,mse&media=video" allow="autoplay" title="ภาพย้อนหลัง"></iframe></div></div>
            <p class="small muted">ลิงก์ภาพย้อนหลังนี้หมดอายุใน 2 ชั่วโมง · การเปิดดูถูกบันทึกใน Audit Log</p>` });
        bindFullscreen(m.body);
        toast('กำลังเปิดภาพย้อนหลังจาก NVR');
      },
    });
  });
}

export function bindFullscreen(container) {
  container.addEventListener('click', (e) => {
    const b = e.target.closest('[data-full]');
    if (!b) return;
    const tile = b.closest('.tile');
    if (document.fullscreenElement) document.exitFullscreen(); else tile.requestFullscreen?.();
  });
  if (!container.dataset.ptzBound) {
    container.dataset.ptzBound = '1';
    bindPtz(container);
  }
  container.addEventListener('dblclick', (e) => {
    const tile = e.target.closest('.tile');
    if (tile && !document.fullscreenElement) tile.requestFullscreen?.();
  });
}

const LAYOUTS = [1, 4, 9, 16, 25, 36, 64];

export async function live(el) {
  let layout = 4;
  try { layout = Number(localStorage.getItem('so-layout')) || 4; } catch { /* ignore */ }
  let page = 0;
  const [cams, ws, branches] = await Promise.all([get('/cameras'), get('/workstations'), get('/branches'), checkStreams()]);
  el.innerHTML = `<div class="page-head"><div><h1>Live CCTV Monitoring</h1><p>ภาพสดผ่าน WebRTC · กรอบสีแสดงพื้นที่โต๊ะและสถานะที่ AI ตรวจพบ (ไม่ระบุตัวบุคคล)</p></div><span class="spacer"></span>
    <select class="input" data-branch style="width:170px"><option value="">ทุกสาขา</option>${branches.map((b) => `<option value="${b.id}">${esc(b.name)}</option>`).join('')}</select>
    <div class="row" data-layouts>${LAYOUTS.map((n) => `<button class="btn sm ${n === layout ? 'primary' : ''}" data-n="${n}">${n}</button>`).join('')}</div>
    <button class="btn sm" data-prev>‹</button><span class="small muted" data-pageinfo></span><button class="btn sm" data-next>›</button></div>
    <div data-grid></div><div style="margin-top:12px">${legend()}</div>
    ${streamOk || window.SO_DEMO ? '' : '<p class="notice">เซิร์ฟเวอร์เชื่อมต่อ go2rtc ไม่ได้ จึงแสดงเฉพาะสถานะจาก AI · ตรวจสอบว่า go2rtc ทำงานและตั้งค่า GO2RTC_API ของ server แล้ว</p>'}`;
  const grid = el.querySelector('[data-grid]');
  const filtered = () => cams.filter((c) => !el.querySelector('[data-branch]').value || String(c.branch_id) === el.querySelector('[data-branch]').value);
  const draw = () => {
    const list = filtered();
    const pages = Math.max(1, Math.ceil(list.length / layout));
    page = Math.min(page, pages - 1);
    const slice = list.slice(page * layout, page * layout + layout);
    const slots = [...slice.map((c) => cctvTile(c)), ...Array(Math.max(0, Math.min(layout, Math.max(4, slice.length)) - slice.length)).fill('<div class="tile empty-slot">ว่าง</div>')];
    grid.innerHTML = `<div class="cctv n${layout}">${slots.join('')}</div>`;
    el.querySelector('[data-pageinfo]').textContent = `หน้า ${page + 1}/${pages} · ${list.length} กล้อง`;
    drawOverlays(grid, ws);
  };
  el.querySelector('[data-layouts]').addEventListener('click', (e) => {
    const n = Number(e.target.dataset.n);
    if (!n) return;
    layout = n; page = 0;
    try { localStorage.setItem('so-layout', n); } catch { /* ignore */ }
    el.querySelectorAll('[data-n]').forEach((b) => b.classList.toggle('primary', Number(b.dataset.n) === n));
    draw();
  });
  el.querySelector('[data-branch]').addEventListener('change', () => { page = 0; draw(); });
  el.querySelector('[data-prev]').addEventListener('click', () => { page = Math.max(0, page - 1); draw(); });
  el.querySelector('[data-next]').addEventListener('click', () => { page += 1; draw(); });
  bindFullscreen(grid);
  bindPlayback(grid, cams);
  draw();
  const off = onLive((m) => {
    if (m.type === 'live') drawOverlays(grid, ws);
    if (m.type === 'camera') { const c = cams.find((x) => x.id === m.camera_id); if (c) { c.status = m.status; draw(); } }
  });
  const timer = setInterval(() => drawOverlays(grid, ws), 5000);
  return () => { off(); clearInterval(timer); };
}

export async function floor(el) {
  const [ws, cams] = await Promise.all([get('/workstations'), get('/cameras')]);
  el.innerHTML = `<div class="page-head"><div><h1>Office Floor Plan</h1><p>สถานะโต๊ะทำงานแบบเรียลไทม์ จัดกลุ่มตามกล้อง / พื้นที่</p></div><span class="spacer"></span>${legend()}</div>
    <div class="panel" style="margin-bottom:16px"><div class="panel-head"><h2>การใช้พื้นที่วันนี้</h2><span class="sub">นับคนแบบไม่ระบุตัวตน · แท่งคือจำนวนคนเฉลี่ยรายชั่วโมง</span></div><div class="panel-body" data-zones></div></div>
    <div data-floor></div>`;
  const ZK = { area: 'พื้นที่สำนักงาน', entrance: 'ทางเข้า-ออก', meeting_room: 'ห้องประชุม', restricted: 'พื้นที่หวงห้าม' };
  const zoneBox = el.querySelector('[data-zones]');
  const loadZones = async () => {
    try {
      const { zones } = await get('/zones/occupancy');
      zoneBox.innerHTML = zones.length ? `<div class="desk-grid" style="grid-template-columns:repeat(auto-fill,minmax(220px,1fr))">${zones.map((z) => {
        const vals = z.avg_by_hour.slice(6, 22); const mx = Math.max(1, ...vals.map((v) => v || 0));
        const full = z.capacity && z.people_now != null ? z.people_now / z.capacity : null;
        return `<div class="zone-card"><div class="row" style="justify-content:space-between"><b>${esc(z.name)}</b><span class="badge">${esc(ZK[z.kind] || z.kind)}</span></div>
          <div><b style="font-size:20px">${z.people_now ?? '-'}</b> <span class="small muted">คนขณะนี้${z.capacity ? ` / ความจุ ${z.capacity}` : ''}</span>
          ${full != null && full >= 1 ? ' <span class="badge bad">เต็ม</span>' : full != null && full >= 0.8 ? ' <span class="badge warn">ใกล้เต็ม</span>' : ''}</div>
          <div class="zone-bars" title="06:00-21:00">${vals.map((v, i) => `<span style="height:${((v || 0) * 100) / mx}%" title="${String(i + 6).padStart(2, '0')}:00 เฉลี่ย ${v ?? 0} คน"></span>`).join('')}</div>
          <div class="small muted">สูงสุด ${z.peak} คน${z.kind === 'entrance' ? ` · เข้า ${z.entries} ครั้ง` : ''}</div></div>`;
      }).join('')}</div>` : '<div class="empty">ยังไม่ได้กำหนดพื้นที่นับคน (เพิ่มได้ที่ AI Detection Management)</div>';
    } catch (e) { zoneBox.innerHTML = `<div class="empty">${esc(e.message)}</div>`; }
  };
  loadZones();
  const box = el.querySelector('[data-floor]');
  const draw = () => {
    if (!ws.length) { box.innerHTML = '<div class="panel empty">ยังไม่มีการกำหนดโต๊ะทำงาน (ตั้งค่าที่เมนู AI Detection Management)</div>'; return; }
    box.innerHTML = cams.filter((c) => ws.some((w) => w.camera_id === c.id)).map((c) => `
      <div class="panel" style="margin-bottom:16px"><div class="panel-head"><h2>${esc(c.name)}</h2><span class="sub">${esc([c.branch_name, c.floor && `ชั้น ${c.floor}`, c.location].filter(Boolean).join(' · '))}</span>
      <span class="spacer"></span><span class="badge ${c.status === 'online' ? 'ok' : 'bad'}"><span class="dot"></span>${c.status === 'online' ? 'ออนไลน์' : 'ออฟไลน์'}</span></div>
      <div class="panel-body"><div class="desk-grid">${ws.filter((w) => w.camera_id === c.id).map((w) => {
        const d = liveState.desks.get(w.id);
        const st = STATES[d?.state];
        return `<div class="desk" style="border-left-color:${st ? st.color : 'var(--line-2)'}"><b>${esc(w.name)}</b>
          <span>${esc(w.employee_name || 'ยังไม่กำหนดพนักงาน')}</span><br>
          <span>${st ? `${esc(st.label)} · ${fmtDur((Date.now() - new Date(d.since)) / 1000)}` : 'ไม่มีข้อมูลสด'}</span>
          ${d?.sitting_since ? `<br><span>นั่งต่อเนื่อง ${fmtDur((Date.now() - new Date(d.sitting_since)) / 1000)}</span>` : ''}
          ${d ? `<br><span class="small">อัปเดต ${ago(d.updated_at)}</span>` : ''}</div>`;
      }).join('')}</div></div></div>`).join('');
  };
  draw();
  const off = onLive((m) => { if (m.type === 'live') draw(); if (m.type === 'zones') loadZones(); });
  const timer = setInterval(draw, 10000);
  return () => { off(); clearInterval(timer); };
}

