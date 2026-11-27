import { get, post, live as liveState, onLive } from '../api.js';
import { esc, icon, STATES, fmtDur, ago, legend, nowStamp } from '../ui.js';

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
    <div class="placeholder">${icon('camera').replace('<svg', '<svg style="width:28px;height:28px;display:block;margin:0 auto 6px"')}${esc(cam.name)}<br>${!streamOk ? 'เชื่อมต่อเซิร์ฟเวอร์ภาพ (go2rtc) ไม่ได้' : online ? 'กำลังเชื่อมต่อ...' : 'ไม่มีสัญญาณ'}</div>
    ${stream && streamOk ? `<iframe src="${esc(src)}" loading="lazy" allow="autoplay" title="${esc(cam.name)}"></iframe>` : ''}
    <svg class="overlay" viewBox="0 0 1000 562.5" preserveAspectRatio="none"></svg>
    <div class="cap"><span class="rec ${online ? '' : 'off'}"></span>${esc(cam.name)}<span class="spacer"></span>${cam.ai_enabled ? '<span class="badge info" style="padding:0 6px">AI</span>' : ''}</div>
    <button class="btn sm icon tile-btn" data-full title="เต็มจอ">${icon('expand')}</button>
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

export function bindFullscreen(container) {
  container.addEventListener('click', (e) => {
    const b = e.target.closest('[data-full]');
    if (!b) return;
    const tile = b.closest('.tile');
    if (document.fullscreenElement) document.exitFullscreen(); else tile.requestFullscreen?.();
  });
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
    ${streamOk ? '' : '<p class="notice">เซิร์ฟเวอร์เชื่อมต่อ go2rtc ไม่ได้ จึงแสดงเฉพาะสถานะจาก AI · ตรวจสอบว่า go2rtc ทำงานและตั้งค่า GO2RTC_API ของ server แล้ว</p>'}`;
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
  el.innerHTML = `<div class="page-head"><div><h1>Office Floor Plan</h1><p>สถานะโต๊ะทำงานแบบเรียลไทม์ จัดกลุ่มตามกล้อง / พื้นที่</p></div><span class="spacer"></span>${legend()}</div><div data-floor></div>`;
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
  const off = onLive((m) => { if (m.type === 'live') draw(); });
  const timer = setInterval(draw, 10000);
  return () => { off(); clearInterval(timer); };
}

