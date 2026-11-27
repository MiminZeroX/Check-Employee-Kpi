import { api, get, post, put, qs, download, session, live as liveState, onLive } from '../api.js';
import {
  esc, icon, crudPage, opt, table, toast, showError, formModal, modal, renderFields, readFields, fmtDateTime, ago,
  fmtBytes, fmtDur, stateBadge, STATES, confirmBox, daysAgo, today, fmtTime,
} from '../ui.js';
import { cctvTile, bindFullscreen, checkStreams } from './live.js';


// ---------------------------------------------------------------- cameras

export async function cameras(el) {
  await crudPage(el, {
    title: 'Camera Management',
    desc: 'กล้อง IP / RTSP / ONVIF ทุกยี่ห้อ · ภาพสดส่งผ่าน go2rtc (WebRTC) โดยผ่านการยืนยันตัวตนของระบบ แยกจากการประมวลผล AI',
    endpoint: '/cameras',
    addLabel: 'เพิ่มกล้อง',
    deleteLabel: 'ลบกล้องนี้? โต๊ะทำงานที่ผูกกับกล้องจะถูกลบด้วย (ประวัติกิจกรรมยังอยู่)',
    columns: [
      { label: 'กล้อง', render: (r) => `<b>${esc(r.name)}</b><div class="small muted mono">${esc(r.rtsp_url)}</div>` },
      { label: 'ตำแหน่ง', hideSm: true, render: (r) => esc([r.branch_name, r.floor && `ชั้น ${r.floor}`, r.location].filter(Boolean).join(' · ') || '-') },
      { key: 'brand', label: 'ยี่ห้อ', hideSm: true },
      { label: 'สถานะ', render: (r) => `<span class="badge ${r.status === 'online' ? 'ok' : r.status === 'offline' ? 'bad' : ''}"><span class="dot"></span>${{ online: 'ออนไลน์', offline: 'ออฟไลน์' }[r.status] || 'ไม่ทราบ'}</span><div class="small muted">${r.last_seen_at ? ago(r.last_seen_at) : ''}</div>` },
      { label: 'สุขภาพสตรีม', hideSm: true, render: (r) => {
        const h = r.health;
        if (!h) return '<span class="muted small">-</span>';
        const err = { 'frozen picture': 'ภาพค้าง (กำลังต่อใหม่)', 'cannot open stream': 'เปิดสตรีมไม่ได้', 'stream ended': 'สตรีมหลุด (กำลังต่อใหม่)', 'no frames': 'ไม่มีภาพ' }[h.error] || h.error;
        // values come from a device: force them to numbers before they reach the page
        const num = (v) => (Number.isFinite(Number(v)) && v !== null ? Number(v) : '-');
        return `<div class="small">${h.error ? `<span class="badge bad">${esc(err)}</span><br>` : ''}AI ${num(h.fps)} fps${h.boosted === true ? ' <span class="badge info">เร่ง</span>' : ''} · กล้อง ${num(h.input_fps)} fps<br><span class="muted">หน่วง ${num(h.latency_ms)} ms · ต่อใหม่ ${num(h.reconnects)} ครั้ง</span></div>`;
      } },
      { label: 'AI', render: (r) => (r.ai_enabled ? `<span class="badge info">เปิด · ${r.ai_fps} fps · ${r.ai_width}px</span>` : '<span class="badge">ปิด</span>') },
      { key: 'workstation_count', label: 'โต๊ะ', align: 'r' },
    ],
    headExtra: `<button class="btn" data-discover>${icon('refresh')} ค้นหากล้องในเครือข่าย (ONVIF)</button>`,
    extraActions: (r) => `<button class="btn sm" data-act="${r.id}" data-action="view">ดูภาพ</button><button class="btn sm" data-act="${r.id}" data-action="masks">ปิดบัง${r.privacy_masks.length ? ` (${r.privacy_masks.length})` : ''}</button>`,
    onAction: async (action, row, reload) => {
      await checkStreams();
      if (action === 'view') {
        const m = modal({ title: row.name, wide: true, body: `<div class="cctv n1">${cctvTile(row)}</div>` });
        bindFullscreen(m.body);
        return;
      }
      if (action === 'masks') {
        const masks = row.privacy_masks.map((p) => p.map((q) => [...q]));
        const save = () => put(`/cameras/${row.id}`, { ...row, privacy_masks: masks, onvif_password: '' });
        const m = modal({
          title: `พื้นที่ปิดบังความเป็นส่วนตัว: ${row.name}`,
          body: `<p class="small muted" style="margin-top:0">พื้นที่สีดำจะถูกปิดในภาพนิ่งที่เซิร์ฟเวอร์ และทับบนภาพสดในหน้าเว็บ (เช่น ประตูห้องน้ำ หน้าต่างอาคารข้างเคียง) · เพื่อปิดในสตรีมด้วย ควรตั้ง Privacy Mask ในตัวกล้องด้วย</p><div data-list></div>`,
          foot: `<button class="btn primary" data-add-mask>${icon('plus')} วาดพื้นที่ใหม่</button>`,
        });
        const list = () => {
          m.body.querySelector('[data-list]').innerHTML = masks.length ? masks.map((p, i) => `<div class="row" style="justify-content:space-between;padding:6px 0;border-bottom:1px solid var(--line)"><span>พื้นที่ ${i + 1} · ${p.length} จุด</span><button class="btn sm danger" data-rm="${i}">ลบ</button></div>`).join('') : '<div class="empty">ยังไม่มีพื้นที่ปิดบัง</div>';
        };
        m.body.addEventListener('click', async (e) => {
          const b = e.target.closest('[data-rm]'); if (!b) return;
          masks.splice(Number(b.dataset.rm), 1);
          try { await save(); list(); toast('ลบพื้นที่แล้ว'); reload(); } catch (err) { showError(err); }
        });
        m.el.querySelector('[data-add-mask]').addEventListener('click', async () => {
          const pts = await roiEditor({ name: row.name, roi: [] }, row, { title: 'วาดพื้นที่ปิดบัง', hint: 'คลิกวางจุดรอบพื้นที่ที่ต้องปิด (อย่างน้อย 3 จุด)', others: masks, color: '#ef5b5b' });
          masks.push(pts);
          try { await save(); list(); toast('บันทึกพื้นที่ปิดบังแล้ว'); reload(); } catch (err) { showError(err); }
        });
        list();
      }
    },
    fields: [
      { key: 'name', label: 'ชื่อกล้อง', required: true },
      { key: 'brand', label: 'ยี่ห้อ', placeholder: 'Hikvision, Dahua, Uniview...' },
      { key: 'rtsp_url', label: 'RTSP URL', required: true, full: true, placeholder: 'rtsp://user:pass@192.168.1.64:554/Streaming/Channels/102', help: 'แนะนำให้ใช้ sub-stream ความละเอียดต่ำสำหรับ AI · รหัสผ่านจะถูกซ่อน (****) หากไม่แก้จะใช้ค่าเดิม' },
      { key: 'onvif_host', label: 'ONVIF host (ถ้ามี)', placeholder: '192.168.1.64:80' },
      { key: 'branch_id', label: 'สาขา', type: 'select', options: opt.branches, numeric: true },
      { key: 'floor', label: 'ชั้น' },
      { key: 'location', label: 'จุดติดตั้ง' },
      { key: 'ai_enabled', label: 'เปิดใช้ AI Detection กับกล้องนี้', type: 'checkbox', default: true },
      { key: 'ptz_supported', label: 'กล้องรองรับ PTZ', type: 'checkbox' },
      { key: 'onvif_user', label: 'ONVIF ชื่อผู้ใช้', help: 'ใช้ควบคุม PTZ' },
      { key: 'onvif_password', label: 'ONVIF รหัสผ่าน', type: 'password', help: 'เข้ารหัสเก็บในฐานข้อมูล · เว้นว่าง = ใช้ค่าเดิม' },
      { key: 'playback_url', label: 'URL ภาพย้อนหลังจาก NVR', full: true, placeholder: 'rtsp://user:pass@NVR:554/Streaming/tracks/101?starttime={start}&endtime={end}',
        help: '{start} {end} = เวลา UTC แบบ 20260131T080000Z · {start_local} {end_local} = เวลาท้องถิ่น · รหัสผ่านถูกเข้ารหัสและซ่อน' },
      { key: 'ai_fps', label: 'FPS สำหรับ AI', type: 'number', step: '0.5', min: 0.2, max: 15, default: 2, help: '1-3 fps เพียงพอและประหยัด CPU' },
      { key: 'ai_width', label: 'ความกว้างภาพสำหรับ AI (px)', type: 'number', min: 320, max: 1920, default: 960 },
    ],
    toForm: (r) => ({ ...r, onvif_password: '' }),
  });
  el.querySelector('[data-discover]').addEventListener('click', async (e) => {
    const b = e.currentTarget; b.disabled = true; b.textContent = 'กำลังค้นหา (3 วินาที)...';
    try {
      const found = await post('/cameras/discover');
      const m = modal({
        title: `พบกล้อง ONVIF ${found.length} ตัว`, wide: true,
        body: found.length ? table([
          { label: 'กล้อง', render: (c) => `<b>${esc(c.name || '-')}</b><div class="small muted">${esc(c.hardware || '')}</div>` },
          { key: 'ip', label: 'IP' }, { key: 'xaddr', label: 'ONVIF', render: (c) => `<span class="mono small">${esc(c.xaddr)}</span>` },
        ], found, { actions: (c) => `<button class="btn sm primary" data-pick="${esc(c.onvif_host)}">เพิ่ม</button>` })
          : '<div class="empty">ไม่พบกล้องในเครือข่ายเดียวกับเซิร์ฟเวอร์ (ถ้าใช้ Docker ต้องเปิด host network) · เพิ่มด้วย RTSP URL ได้ตามปกติ</div>',
      });
      m.body.addEventListener('click', (ev) => {
        const p = ev.target.closest('[data-pick]'); if (!p) return;
        const c = found.find((x) => x.onvif_host === p.dataset.pick);
        m.close();
        formModal({
          title: 'เพิ่มกล้องที่ค้นพบ',
          fields: [
            { key: 'name', label: 'ชื่อกล้อง', required: true, default: c.name || '' },
            { key: 'rtsp_url', label: 'RTSP URL', required: true, full: true, default: `rtsp://user:pass@${c.ip}:554/Streaming/Channels/102` },
            { key: 'onvif_host', label: 'ONVIF host', default: c.onvif_host }, { key: 'brand', label: 'ยี่ห้อ', default: (c.hardware || '').split(' ')[0] },
            { key: 'onvif_user', label: 'ONVIF ชื่อผู้ใช้' }, { key: 'onvif_password', label: 'ONVIF รหัสผ่าน', type: 'password' },
          ],
          onSubmit: async (v) => { await post('/cameras', { ...v, ai_enabled: true, ai_fps: 2, ai_width: 960 }); toast('เพิ่มกล้องแล้ว'); location.reload(); },
        });
      });
    } catch (err) { showError(err); } finally { b.disabled = false; b.innerHTML = `${icon('refresh')} ค้นหากล้องในเครือข่าย (ONVIF)`; }
  });
}

// ---------------------------------------------------------------- AI detection

function roiEditor(ws, camera, { title, hint, others = [], color = '#22c3e6' } = {}) {
  let pts = (ws.roi || []).map((p) => [...p]);
  const snap = camera ? `/video/api/frame.jpeg?src=${encodeURIComponent(camera.stream_name)}&t=${Date.now()}` : '';
  const m = modal({
    title: title || `พื้นที่ตรวจจับ: ${ws.name}`, wide: true,
    body: `<p class="small muted" style="margin-top:0">${esc(hint || 'คลิกบนภาพเพื่อวางจุดรอบบริเวณโต๊ะและเก้าอี้ (อย่างน้อย 3 จุด) ระบบจะตรวจเฉพาะบุคคลที่อยู่ในพื้นที่นี้')}</p>
      <div class="roi-editor">${snap ? `<img src="${esc(snap)}" alt="">` : ''}<svg viewBox="0 0 1000 562.5" preserveAspectRatio="none"></svg></div>`,
    foot: '<button class="btn" data-undo>ย้อนจุดล่าสุด</button><button class="btn" data-clear>ล้าง</button><span class="spacer"></span><button class="btn" data-close>ยกเลิก</button><button class="btn primary" data-save>บันทึกพื้นที่</button>',
  });
  const box = m.body.querySelector('.roi-editor');
  box.querySelector('img')?.addEventListener('error', (e) => e.target.remove());
  const svg = box.querySelector('svg');
  const poly = (ps) => ps.map(([x, y]) => `${x * 1000},${y * 562.5}`).join(' ');
  const draw = () => {
    svg.innerHTML = `${others.map((o) => `<polygon points="${poly(o)}" fill="rgba(0,0,0,.55)" stroke="#888" stroke-width="2"/>`).join('')}
      <polygon points="${poly(pts)}" fill="${color}33" stroke="${color}" stroke-width="3"/>${pts.map(([x, y]) => `<circle cx="${x * 1000}" cy="${y * 562.5}" r="7" fill="${color}"/>`).join('')}`;
  };
  box.addEventListener('click', (e) => {
    const r = box.getBoundingClientRect();
    pts.push([+((e.clientX - r.left) / r.width).toFixed(4), +((e.clientY - r.top) / r.height).toFixed(4)]);
    draw();
  });
  m.el.querySelector('[data-undo]').addEventListener('click', () => { pts.pop(); draw(); });
  m.el.querySelector('[data-clear]').addEventListener('click', () => { pts = []; draw(); });
  draw();
  return new Promise((resolve) => {
    m.el.querySelector('[data-save]').addEventListener('click', () => {
      if (pts.length < 3) { toast('ต้องมีอย่างน้อย 3 จุด', 'warn'); return; }
      m.close(); resolve(pts);
    });
  });
}

export async function ai(el) {
  el.innerHTML = `<div class="page-head"><div><h1>AI Detection Management</h1><p>กำหนดโต๊ะทำงาน พื้นที่ตรวจจับ (ROI) และค่าความไวของ AI</p></div></div>
    <div class="panel" style="margin-bottom:16px"><div class="panel-head"><h2>ค่าการตรวจจับ</h2><span class="sub">AI worker โหลดค่าใหม่อัตโนมัติทุก 60 วินาที</span><span class="spacer"></span><button class="btn primary sm" data-save-det>บันทึก</button></div>
      <form class="panel-body" data-det></form></div>
    <div data-ws></div>
    <div class="panel" style="margin-top:16px"><div class="panel-body small muted">
      <b>สถานะที่ตรวจจับ:</b> ${Object.values(STATES).map((s) => esc(s.long)).join(' · ')}<br>
      การมองหน้าจอประเมินจากทิศทางใบหน้าเทียบกับค่าปรับเทียบของแต่ละโต๊ะ ให้พนักงานนั่งมองจอตามปกติแล้วกด "ปรับเทียบ" ·
      หากความมั่นใจต่ำกว่าเกณฑ์ ระบบจะบันทึกเป็น "ไม่สามารถระบุได้" แทนการเดา</div></div>`;
  const settings = await get('/settings');
  const detFields = [
    { key: 'min_state_sec', label: 'สถานะต้องคงอยู่อย่างน้อย (วินาที)', type: 'number', min: 1, max: 120, help: 'กันการสลับสถานะถี่เกินไป' },
    { key: 'away_grace_sec', label: 'นับว่าไม่อยู่โต๊ะเมื่อหายไปเกิน (วินาที)', type: 'number', min: 5, max: 600 },
    { key: 'detect_confidence', label: 'เกณฑ์ความมั่นใจขั้นต่ำ (0-1)', type: 'number', step: '0.05', min: 0.1, max: 0.95 },
    { key: 'sitting_alert_min', label: 'แจ้งเตือนเมื่อนั่งต่อเนื่องเกิน (นาที)', type: 'number', min: 10, max: 240 },
  ];
  const det = el.querySelector('[data-det]');
  det.innerHTML = await renderFields(detFields, settings);
  el.querySelector('[data-save-det]').addEventListener('click', async () => {
    try { await put('/settings', readFields(det, detFields)); toast('บันทึกค่าการตรวจจับแล้ว'); } catch (e) { showError(e); }
  });

  const cams = await get('/cameras');
  const page = await crudPage(el.querySelector('[data-ws]'), {
    title: 'โต๊ะทำงานและพื้นที่ตรวจจับ', endpoint: '/workstations', addLabel: 'เพิ่มโต๊ะทำงาน',
    columns: [
      { key: 'name', label: 'โต๊ะ' }, { key: 'camera_name', label: 'กล้อง' },
      { label: 'พนักงาน', render: (r) => esc(r.employee_name || '— ว่าง —') },
      { label: 'สถานะสด', render: (r) => { const d = liveState.desks.get(r.id); return d ? `${stateBadge(d.state)} <span class="small muted">${Math.round(d.confidence * 100)}%</span>` : '<span class="muted small">-</span>'; } },
      { label: 'พื้นที่', render: (r) => `${(r.roi || []).length} จุด` },
      { label: 'ทิศจอ (yaw)', hideSm: true, render: (r) => { const d = liveState.desks.get(r.id); return `${r.screen_yaw_deg.toFixed(0)}° ±${r.yaw_tolerance_deg.toFixed(0)}°${d?.yaw != null ? ` <span class="small muted">(ปัจจุบัน ${d.yaw.toFixed(0)}°)</span>` : ''}`; } },
    ],
    extraActions: (r) => `<button class="btn sm" data-act="${r.id}" data-action="roi">พื้นที่</button><button class="btn sm" data-act="${r.id}" data-action="cal">ปรับเทียบ</button>`,
    onAction: async (action, row, reload) => {
      try {
        if (action === 'roi') {
          await checkStreams();
          const roi = await roiEditor(row, cams.find((c) => c.id === row.camera_id));
          await put(`/workstations/${row.id}`, { camera_id: row.camera_id, name: row.name, employee_id: row.employee_id, roi, screen_yaw_deg: row.screen_yaw_deg, yaw_tolerance_deg: row.yaw_tolerance_deg });
          toast('บันทึกพื้นที่ตรวจจับแล้ว'); reload();
        } else if (action === 'cal') {
          const r = await post(`/workstations/${row.id}/calibrate`);
          toast(`ปรับเทียบแล้ว: ทิศจอ ${r.screen_yaw_deg.toFixed(1)}°`); reload();
        }
      } catch (e) { showError(e); }
    },
    toBody: (v, row) => ({ ...v, roi: row?.roi }),
    fields: [
      { key: 'name', label: 'ชื่อโต๊ะ', required: true },
      { key: 'camera_id', label: 'กล้อง', type: 'select', required: true, options: opt.cameras, numeric: true },
      { key: 'employee_id', label: 'พนักงานประจำโต๊ะ', type: 'select', options: opt.employees, numeric: true, help: 'ระบบระบุบุคคลจากการจับคู่โต๊ะ ไม่ใช้การจดจำใบหน้า' },
      { key: 'yaw_tolerance_deg', label: 'ช่วงองศาที่ถือว่ามองจอ (±)', type: 'number', min: 5, max: 90, default: 30 },
      { key: 'screen_yaw_deg', label: 'ทิศทางหน้าจอ (องศา)', type: 'number', step: '0.5', default: 0, help: 'ใช้ปุ่ม "ปรับเทียบ" แทนการกรอกเองได้' },
    ],
  });
  // ---- people-counting zones (office area, entrance, meeting rooms): anonymous counts only
  const zbox = document.createElement('div'); zbox.style.marginTop = '24px'; el.appendChild(zbox);
  const ZK = { area: 'พื้นที่สำนักงาน', entrance: 'ทางเข้า-ออก', meeting_room: 'ห้องประชุม', restricted: 'พื้นที่หวงห้าม' };
  await crudPage(zbox, {
    title: 'พื้นที่นับคน', desc: 'นับจำนวนคนแบบไม่ระบุตัวตน ใช้ทำ Heatmap การใช้พื้นที่ ความหนาแน่นห้องประชุม และจำนวนคนเข้า-ออก', endpoint: '/zones', addLabel: 'เพิ่มพื้นที่',
    columns: [{ key: 'name', label: 'พื้นที่' }, { label: 'ประเภท', render: (z) => esc(ZK[z.kind] || z.kind) }, { key: 'camera_name', label: 'กล้อง' },
      { label: 'ความจุ', align: 'r', render: (z) => (z.capacity ?? '-') }, { label: 'จุด', align: 'r', render: (z) => z.roi.length }],
    extraActions: (z) => `<button class="btn sm" data-act="${z.id}" data-action="roi">พื้นที่</button>`,
    onAction: async (action, z, reload) => {
      try {
        await checkStreams();
        const roi = await roiEditor({ name: z.name, roi: z.roi }, cams.find((c) => c.id === z.camera_id), { title: `พื้นที่นับคน: ${z.name}`, hint: 'คลิกวางจุดรอบพื้นที่ที่ต้องการนับคน', color: '#f0a63a' });
        await put(`/zones/${z.id}`, { ...z, roi }); toast('บันทึกพื้นที่แล้ว'); reload();
      } catch (e) { showError(e); }
    },
    toBody: (v, row) => ({ ...v, roi: row?.roi }),
    fields: [
      { key: 'name', label: 'ชื่อพื้นที่', required: true }, { key: 'camera_id', label: 'กล้อง', type: 'select', required: true, options: opt.cameras, numeric: true },
      { key: 'kind', label: 'ประเภท', type: 'select', required: true, options: Object.entries(ZK).map(([value, label]) => ({ value, label })) },
      { key: 'capacity', label: 'ความจุ (คน)', type: 'number', min: 1 },
    ],
  });
  const off = onLive((m) => { if (m.type === 'live') page.reload(); });
  return () => off();
}

// ---------------------------------------------------------------- devices

function showKey(key, kind) {
  const base = location.origin;
  const howto = kind === 'ai_worker'
    ? `ตั้งค่าใน AI worker:<div class="code">SERVER_URL=${esc(base)}<br>DEVICE_KEY=${esc(key)}</div>`
    : kind === 'hikvision'
      ? `ตั้งค่า HTTP Listening ของเครื่อง Hikvision ให้ส่ง event ไปที่:<div class="code">${esc(base)}/api/ingest/hikvision?key=${esc(key)}</div>`
      : `ส่งข้อมูลการสแกนแบบ JSON ด้วย header <span class="mono">X-Device-Key</span>:<div class="code">POST ${esc(base)}/api/ingest/attendance<br>{"records":[{"attendance_ref":"1001","scan_time":"2026-01-31 08:12:00","direction":"in","method":"face"}]}</div>`;
  modal({ title: 'API Key ของอุปกรณ์', body: `<p class="notice" style="margin-top:0">คัดลอกเก็บไว้ตอนนี้ ระบบจะไม่แสดง Key นี้อีก (สร้างใหม่ได้ด้วยปุ่ม "Key ใหม่")</p><div class="code">${esc(key)}</div><p>${howto}</p>` });
}

export async function devices(el) {
  const KIND = { ai_worker: 'AI Worker', zkteco: 'ZKTeco (ADMS)', hikvision: 'Hikvision Face/Access', generic: 'อื่น ๆ (REST API)' };
  await crudPage(el, {
    title: 'Device Integration', desc: 'AI worker และเครื่องสแกนลายนิ้วมือ / สแกนใบหน้า',
    endpoint: '/devices', canEdit: session.can('devices.manage'), addLabel: 'เพิ่มอุปกรณ์',
    before: `<div class="panel panel-body small" style="margin-bottom:16px"><b>ZKTeco:</b> ตั้งค่าเมนู Cloud Server / ADMS ของเครื่องให้ชี้มาที่ <span class="mono">${esc(location.hostname)}</span> พอร์ต <span class="mono">${esc(location.port || '80')}</span> เครื่องที่ไม่รู้จักจะถูกเพิ่มเป็น "รออนุมัติ" อัตโนมัติ ·
      <b>Hikvision:</b> ใช้ HTTP Listening ส่ง AccessControllerEvent · <b>ระบบอื่น:</b> ส่ง JSON ผ่าน REST API</div>`,
    columns: [
      { key: 'name', label: 'ชื่อ' }, { label: 'ประเภท', render: (r) => esc(KIND[r.kind] || r.kind) }, { key: 'serial_no', label: 'Serial' },
      { label: 'สถานะ', render: (r) => (r.is_enabled ? '<span class="badge ok">เปิดใช้</span>' : '<span class="badge warn">รออนุมัติ / ปิด</span>') },
      { label: 'ติดต่อล่าสุด', render: (r) => (r.last_seen_at ? ago(r.last_seen_at) : '-') },
    ],
    extraActions: (r) => (r.kind !== 'zkteco' ? `<button class="btn sm" data-act="${r.id}" data-action="key">Key ใหม่</button>` : ''),
    onAction: async (action, row) => {
      if (action !== 'key' || !(await confirmBox('สร้าง API Key ใหม่? Key เดิมจะใช้งานไม่ได้ทันที', 'สร้างใหม่', false))) return;
      try { const r = await post(`/devices/${row.id}/rotate-key`); showKey(r.api_key, row.kind); } catch (e) { showError(e); }
    },
    afterSave: (r, created) => { if (created && r.api_key) showKey(r.api_key, document.querySelector('[name=kind]')?.value); },
    fields: [
      { key: 'name', label: 'ชื่ออุปกรณ์', required: true },
      { key: 'kind', label: 'ประเภท', type: 'select', required: true, options: Object.entries(KIND).map(([value, label]) => ({ value, label })) },
      { key: 'serial_no', label: 'Serial Number', help: 'จำเป็นสำหรับ ZKTeco' },
      { key: 'branch_id', label: 'สาขา', type: 'select', options: opt.branches, numeric: true },
      { key: 'is_enabled', label: 'เปิดใช้งาน', type: 'checkbox', default: true },
    ],
  });
}

// ---------------------------------------------------------------- users

export async function users(el) {
  el.innerHTML = `<div class="tabs"><button class="active" data-tab="users">ผู้ใช้</button><button data-tab="roles">บทบาทและสิทธิ์</button></div><div data-body></div>`;
  const body = el.querySelector('[data-body]');
  let data = await get('/roles');
  const roleOpts = () => data.roles.map((r) => ({ value: r.name, label: r.label }));
  const roleLabel = (n) => data.roles.find((r) => r.name === n)?.label || n;

  const showUsers = () => crudPage(body, {
    title: 'User & Role Management', desc: 'บัญชีผู้ใช้ · สิทธิ์กำหนดที่แท็บ "บทบาทและสิทธิ์"', endpoint: '/users', addLabel: 'เพิ่มผู้ใช้',
    columns: [
      { key: 'username', label: 'ชื่อผู้ใช้' }, { key: 'display_name', label: 'ชื่อที่แสดง' },
      { label: 'บทบาท', render: (r) => `<span class="badge info">${esc(roleLabel(r.role))}</span>` },
      { label: 'MFA', render: (r) => (r.mfa_enabled ? '<span class="badge ok">เปิด</span>' : '<span class="badge">ปิด</span>') },
      { label: 'สถานะ', render: (r) => (r.is_active ? '<span class="badge ok">ใช้งาน</span>' : '<span class="badge">ระงับ</span>') },
      { label: 'เข้าใช้ล่าสุด', render: (r) => (r.last_login_at ? ago(r.last_login_at) : '-') },
    ],
    fields: [
      { key: 'username', label: 'ชื่อผู้ใช้', required: true, help: 'แก้ไขไม่ได้หลังสร้าง · ถ้าใช้ SSO ให้ตรงกับอีเมลบริษัท' },
      { key: 'display_name', label: 'ชื่อที่แสดง' },
      { key: 'password', label: 'รหัสผ่าน', type: 'password', help: 'อย่างน้อย 8 ตัวอักษร · เว้นว่างเมื่อแก้ไขถ้าไม่ต้องการเปลี่ยน (เปลี่ยนแล้วผู้ใช้จะถูกออกจากทุกอุปกรณ์)' },
      { key: 'role', label: 'บทบาท', type: 'select', required: true, options: roleOpts },
      { key: 'employee_id', label: 'ผูกกับพนักงาน', type: 'select', options: opt.employees, numeric: true, help: 'ผูกแล้วจะเห็นเมนู "ของฉัน" · จำเป็นสำหรับบทบาทพนักงาน' },
      { key: 'is_active', label: 'เปิดใช้งาน', type: 'checkbox', default: true },
      { key: 'mfa_reset', label: 'รีเซ็ต MFA (ให้ผู้ใช้ตั้งค่าใหม่)', type: 'checkbox' },
    ],
    toForm: (r) => ({ ...r, password: '', mfa_reset: false }),
  });

  const showRoles = () => {
    const groups = {};
    for (const p of data.permissions) (groups[p.key.split('.')[0]] ||= []).push(p);
    body.innerHTML = `<div class="page-head"><div><h1>บทบาทและสิทธิ์</h1><p>ติ๊กสิทธิ์ของแต่ละบทบาท มีผลทันทีกับผู้ใช้ทุกคนในบทบาทนั้น · ผู้ดูแลระบบมีทุกสิทธิ์เสมอ</p></div><span class="spacer"></span>
      <button class="btn primary" data-new>${icon('plus')} เพิ่มบทบาท</button></div>
      <div class="panel"><div class="tbl-wrap" style="max-height:none"><table class="tbl"><thead><tr><th>สิทธิ์</th>${data.roles.map((r) => `<th class="r" style="text-align:center">${esc(r.label)}<div class="small muted">${r.users} คน</div></th>`).join('')}</tr></thead>
      <tbody>${Object.entries(groups).map(([, ps]) => ps.map((p) => `<tr><td>${esc(p.label)}<div class="small muted mono">${esc(p.key)}</div></td>${data.roles.map((r) => `<td style="text-align:center"><input type="checkbox" aria-label="${esc(r.label)}: ${esc(p.label)}" data-role="${esc(r.name)}" data-perm="${esc(p.key)}" ${r.permissions.includes(p.key) ? 'checked' : ''} ${r.editable ? '' : 'disabled'}></td>`).join('')}</tr>`).join('')).join('')}</tbody>
      <tfoot><tr><td></td>${data.roles.map((r) => `<td style="text-align:center">${r.editable ? `<button class="btn sm primary" data-save="${esc(r.name)}">บันทึก</button>` : ''}${r.is_system ? '' : ` <button class="btn sm danger" data-del="${esc(r.name)}">ลบ</button>`}</td>`).join('')}</tr></tfoot></table></div></div>`;
    body.onclick = async (e) => {
      const b = e.target.closest('button');
      if (!b) return;
      try {
        if (b.dataset.save) {
          const name = b.dataset.save;
          const perms = [...body.querySelectorAll(`input[data-role="${CSS.escape(name)}"]:checked`)].map((i) => i.dataset.perm);
          await put(`/roles/${encodeURIComponent(name)}`, { label: roleLabel(name), permissions: perms });
          toast(`บันทึกสิทธิ์ของ "${roleLabel(name)}" แล้ว`);
          data = await get('/roles'); showRoles();
        } else if (b.dataset.del) {
          if (!(await confirmBox(`ลบบทบาท "${roleLabel(b.dataset.del)}"?`, 'ลบ'))) return;
          await api(`/roles/${encodeURIComponent(b.dataset.del)}`, { method: 'DELETE' });
          data = await get('/roles'); showRoles();
        } else if (b.dataset.new !== undefined) {
          formModal({
            title: 'เพิ่มบทบาท',
            fields: [
              { key: 'label', label: 'ชื่อบทบาท', required: true, placeholder: 'เช่น หัวหน้าฝ่ายขาย' },
              { key: 'name', label: 'รหัส (a-z, 0-9, _)', required: true, placeholder: 'sales_lead' },
              { key: 'copy', label: 'เริ่มจากสิทธิ์ของ', type: 'select', options: roleOpts },
            ],
            onSubmit: async (v) => {
              const base = data.roles.find((r) => r.name === v.copy);
              await post('/roles', { name: v.name, label: v.label, permissions: base ? base.permissions : [] });
              toast('เพิ่มบทบาทแล้ว'); data = await get('/roles'); showRoles();
            },
          });
        }
      } catch (err) { showError(err); }
    };
  };

  el.querySelector('.tabs').addEventListener('click', (e) => {
    const b = e.target.closest('[data-tab]'); if (!b) return;
    el.querySelectorAll('.tabs button').forEach((x) => x.classList.toggle('active', x === b));
    body.onclick = null;
    if (b.dataset.tab === 'users') showUsers(); else showRoles();
  });
  await showUsers();
}

// ---------------------------------------------------------------- audit

export async function audit(el) {
  el.innerHTML = `<div class="page-head"><div><h1>Audit Logs</h1><p>บันทึกการเข้าถึงและการเปลี่ยนแปลงข้อมูลทั้งหมด</p></div></div>
    <div class="panel"><div class="toolbar"><input class="input" data-q placeholder="ค้นหาผู้ใช้ / การกระทำ / ประเภทข้อมูล" style="width:300px"></div><div data-body></div>
    <div class="panel-body"><button class="btn sm" data-more>โหลดเพิ่ม</button></div></div>`;
  let rows = []; let offset = 0;
  const load = async (reset) => {
    if (reset) { rows = []; offset = 0; }
    const more = await get(`/audit${qs({ limit: 200, offset, q: el.querySelector('[data-q]').value })}`);
    rows = rows.concat(more); offset += more.length;
    el.querySelector('[data-more]').classList.toggle('hidden', more.length < 200);
    el.querySelector('[data-body]').innerHTML = table([
      { label: 'เวลา', render: (r) => fmtDateTime(r.created_at) }, { key: 'username', label: 'ผู้ใช้' },
      { label: 'การกระทำ', render: (r) => `<span class="badge ${r.action.includes('fail') ? 'bad' : ''}">${esc(r.action)}</span>` },
      { label: 'ข้อมูล', render: (r) => `${esc(r.entity || '')} <span class="muted">${esc(r.entity_id || '')}</span>` },
      { label: 'รายละเอียด', hideSm: true, render: (r) => `<span class="mono">${esc((r.detail || '').slice(0, 140))}</span>` },
    ], rows);
  };
  let t;
  el.querySelector('[data-q]').addEventListener('input', () => { clearTimeout(t); t = setTimeout(() => load(true).catch(showError), 300); });
  el.querySelector('[data-more]').addEventListener('click', () => load(false).catch(showError));
  await load(true);
}

// ---------------------------------------------------------------- privacy

export async function privacy(el) {
  el.innerHTML = `<div class="page-head"><div><h1>Privacy & Data Management</h1><p>PDPA · Privacy by Design · ตรวจสอบโดยมนุษย์ก่อนนำข้อมูลไปใช้</p></div></div>
    <div class="grid g-3" style="margin-bottom:16px" data-cards></div>
    <div class="panel" style="margin-bottom:16px"><div class="panel-head"><h2>คำขอแก้ไขข้อมูลจากพนักงาน</h2><span class="spacer"></span>
      <select class="input" data-status style="width:150px"><option value="pending">รอพิจารณา</option><option value="approved">อนุมัติแล้ว</option><option value="rejected">ไม่อนุมัติ</option><option value="">ทั้งหมด</option></select></div><div data-corr></div></div>
    <div class="panel"><div class="panel-head"><h2>หลักการที่ระบบใช้</h2></div><div class="panel-body small"><ul style="margin:0;padding-left:18px;line-height:1.9">
      <li>ระบุบุคคลจากการจับคู่โต๊ะทำงานกับพนักงาน ไม่ใช้การจดจำใบหน้า และไม่เก็บข้อมูลชีวมิติซ้ำ (ใช้ผลยืนยันตัวตนจากเครื่องสแกนเท่านั้น)</li>
      <li>AI worker ไม่บันทึกภาพหรือวิดีโอ ส่งเฉพาะสถานะ เวลา และค่าความมั่นใจ (metadata) เข้าฐานข้อมูล</li>
      <li>ข้อมูลกิจกรรมจากกล้องไม่ถูกนำมาคิดคะแนน KPI และไม่ใช้วินิจฉัยสุขภาพ อารมณ์ หรือโรค</li>
      <li>พนักงานดูข้อมูลของตนเองและขอแก้ไขได้ ผู้ตรวจสอบต้องพิจารณาก่อนข้อมูลถูกแก้ไข และทุกการกระทำถูกบันทึกใน Audit Log</li>
      <li>ข้อมูลกิจกรรมถูกลบอัตโนมัติเมื่อเกินระยะเวลาเก็บรักษาที่กำหนด</li></ul></div></div>`;
  const [settings, emps] = await Promise.all([get('/settings'), get('/employees?status=active')]);
  const noConsent = emps.filter((e) => !e.monitoring_consent);
  el.querySelector('[data-cards]').innerHTML = `
    <div class="panel stat"><div class="ic">${icon('shield')}</div><div><b>${settings.retention_days} วัน</b><span>ระยะเวลาเก็บข้อมูลกิจกรรม</span></div></div>
    <div class="panel stat ${noConsent.length ? 'warn' : 'ok'}"><div class="ic">${icon('users')}</div><div><b>${emps.length - noConsent.length}/${emps.length}</b><span>พนักงานที่บันทึกการยินยอมแล้ว</span></div></div>
    <div class="panel panel-body small"><b>ข้อความแจ้งพนักงาน</b><div class="muted" style="margin-top:4px">${esc(settings.monitoring_notice)}</div>${session.can('settings.manage') ? '<a href="#system-settings" class="small">แก้ไขที่ System Settings</a>' : ''}</div>`;
  const load = async () => {
    const st = el.querySelector('[data-status]').value;
    const rows = await get(`/corrections${qs({ status: st })}`);
    el.querySelector('[data-corr]').innerHTML = table([
      { label: 'วันที่ขอ', render: (r) => fmtDateTime(r.created_at) }, { key: 'employee_name', label: 'พนักงาน' },
      { label: 'เหตุการณ์', render: (r) => `${stateBadge(r.event_state)} <span class="small muted">${fmtDateTime(r.event_started_at)} - ${fmtTime(r.event_ended_at)}</span>` },
      { label: 'ขอแก้เป็น', render: (r) => (r.requested_state ? stateBadge(r.requested_state) : '<span class="muted">ลบข้อมูลนี้</span>') },
      { key: 'reason', label: 'เหตุผล' },
      { label: 'สถานะ', render: (r) => `<span class="badge ${{ pending: 'warn', approved: 'ok', rejected: 'bad' }[r.status]}">${{ pending: 'รอพิจารณา', approved: 'อนุมัติ', rejected: 'ไม่อนุมัติ' }[r.status]}</span>${r.review_note ? `<div class="small muted">${esc(r.review_note)}</div>` : ''}` },
    ], rows, {
      empty: 'ไม่มีคำขอ',
      actions: (r) => (r.status === 'pending' ? `<button class="btn sm primary" data-ok="${r.id}">อนุมัติ</button><button class="btn sm danger" data-no="${r.id}">ไม่อนุมัติ</button>` : ''),
    });
    el.querySelector('[data-corr]').onclick = (e) => {
      const b = e.target.closest('button'); if (!b) return;
      const row = rows.find((x) => String(x.id) === (b.dataset.ok || b.dataset.no));
      const approve = !!b.dataset.ok;
      formModal({
        title: approve ? 'อนุมัติคำขอแก้ไข' : 'ไม่อนุมัติคำขอ',
        fields: [
          ...(approve ? [{ key: 'corrected_state', label: 'แก้สถานะเป็น', type: 'select', options: opt.states, default: row.requested_state, placeholder: '- ลบเหตุการณ์นี้ออก -', full: true }] : []),
          { key: 'note', label: 'หมายเหตุถึงพนักงาน', type: 'textarea', required: !approve },
        ],
        submitLabel: approve ? 'อนุมัติ' : 'ยืนยันไม่อนุมัติ',
        onSubmit: async (v) => { await post(`/corrections/${row.id}/review`, { approve, ...v }); toast('บันทึกผลการพิจารณาแล้ว'); load(); },
      });
    };
  };
  el.querySelector('[data-status]').addEventListener('change', () => load().catch(showError));
  await load();
}

// ---------------------------------------------------------------- settings

export async function settings(el) {
  const s = await get('/settings');
  const fields = [
    { key: 'org_name', label: 'ชื่อองค์กร', full: true },
    { key: 'report_header', label: 'หัวรายงาน A4', full: true },
    { key: 'report_footer', label: 'ท้ายรายงาน A4', full: true },
    { key: 'logo_url', label: 'URL โลโก้องค์กร (สำหรับรายงาน)', full: true, emptyAsNull: false },
    { key: 'monitoring_notice', label: 'ข้อความแจ้งพนักงานเรื่องการใช้กล้อง (แสดงที่หน้าเข้าสู่ระบบ)', type: 'textarea' },
    { key: 'retention_days', label: 'เก็บข้อมูลกิจกรรม (วัน)', type: 'number', min: 7, max: 3650 },
    { key: 'sitting_alert_min', label: 'แจ้งเตือนเมื่อนั่งต่อเนื่องเกิน (นาที)', type: 'number', min: 10, max: 240 },
    { key: 'min_state_sec', label: 'สถานะต้องคงอยู่อย่างน้อย (วินาที)', type: 'number', min: 1, max: 120 },
    { key: 'away_grace_sec', label: 'นับว่าไม่อยู่โต๊ะเมื่อหายไปเกิน (วินาที)', type: 'number', min: 5, max: 600 },
    { key: 'detect_confidence', label: 'เกณฑ์ความมั่นใจของ AI (0-1)', type: 'number', step: '0.05', min: 0.1, max: 0.95 },
  ];
  el.innerHTML = `<div class="page-head"><div><h1>System Settings</h1><p>ข้อมูลองค์กร รูปแบบรายงาน การเก็บรักษาข้อมูล และค่าการตรวจจับ</p></div><span class="spacer"></span><button class="btn primary" data-save>บันทึก</button></div>
    <form class="panel panel-body" data-form></form>`;
  const form = el.querySelector('[data-form]');
  form.innerHTML = await renderFields(fields, s);
  el.querySelector('[data-save]').addEventListener('click', async () => {
    const v = readFields(form, fields);
    for (const k of Object.keys(v)) if (v[k] === null) v[k] = fields.find((f) => f.key === k).type === 'number' ? s[k] : '';
    try { await put('/settings', v); toast('บันทึกการตั้งค่าแล้ว'); session.config.org_name = v.org_name; } catch (e) { showError(e); }
  });
}

// ---------------------------------------------------------------- server resources

export async function system(el) {
  el.innerHTML = '<div class="page-head"><div><h1>Server & Resource Monitoring</h1><p>รีเฟรชทุก 5 วินาที</p></div></div><div data-body></div>';
  const draw = async () => {
    const r = await get('/system/resources');
    const memPct = (r.system_memory_used_bytes * 100) / r.system_memory_total_bytes;
    el.querySelector('[data-body]').innerHTML = `<div class="grid g-4" style="margin-bottom:16px">
      <div class="panel stat ok"><div class="ic">${icon('server')}</div><div><b>${fmtBytes(r.process_memory_bytes)}</b><span>RAM ของ API server · CPU ${r.process_cpu_percent.toFixed(1)}%</span></div></div>
      <div class="panel stat"><div class="ic">${icon('chart')}</div><div><b>${memPct.toFixed(0)}%</b><span>RAM เครื่อง ${fmtBytes(r.system_memory_used_bytes)} / ${fmtBytes(r.system_memory_total_bytes)} · ${r.cpu_count} CPU</span><div class="meter" style="margin-top:6px"><span style="width:${memPct}%"></span></div></div></div>
      <div class="panel stat ${r.db_ok ? 'ok' : 'warn'}"><div class="ic">${icon('log')}</div><div><b>${r.db_ok ? 'ปกติ' : 'ผิดปกติ'}</b><span>MySQL · pool ${r.db_pool_size} (ว่าง ${r.db_pool_idle})</span></div></div>
      <div class="panel stat"><div class="ic">${icon('clock')}</div><div><b>${fmtDur(r.uptime_sec)}</b><span>Uptime · WebSocket ${r.websocket_clients} · โต๊ะสด ${r.live_desks}</span></div></div></div>
      <div class="panel"><div class="panel-head"><h2>ขนาดตารางในฐานข้อมูล</h2></div>${table([
        { key: 'name', label: 'ตาราง' }, { label: 'จำนวนแถว (ประมาณ)', align: 'r', render: (t) => (t.rows ?? 0).toLocaleString() }, { label: 'ขนาด', align: 'r', render: (t) => fmtBytes(t.bytes || 0) },
      ], r.tables)}</div>`;
  };
  await draw();
  const timer = setInterval(() => draw().catch(() => {}), 5000);
  return () => clearInterval(timer);
}

// ---------------------------------------------------------------- backup

export async function backup(el) {
  el.innerHTML = `<div class="page-head"><div><h1>Backup & Restore</h1><p>สำรองการตั้งค่าและฐานข้อมูล</p></div></div>
    <div class="grid g-2">
      <div class="panel"><div class="panel-head"><h2>สำรองการตั้งค่า (JSON)</h2></div><div class="panel-body stack">
        <p class="small muted" style="margin:0">ส่งออกพนักงาน แผนก กล้อง โต๊ะทำงาน ตัวชี้วัด KPI และการตั้งค่า (ไม่รวมรหัสผ่านกล้อง)</p>
        <div><button class="btn primary" data-export>${icon('download')} ดาวน์โหลดไฟล์ตั้งค่า</button></div></div></div>
      <div class="panel"><div class="panel-head"><h2>สำรอง / กู้คืนฐานข้อมูลทั้งหมด</h2></div><div class="panel-body small">
        <p style="margin-top:0">ใช้ <span class="mono">mysqldump</span> บนเซิร์ฟเวอร์ (แนะนำตั้ง cron ทุกคืน):</p>
        <div class="code">docker compose exec mysql sh -c 'mysqldump --single-transaction -usmartoffice -p"$MYSQL_PASSWORD" smartoffice' | gzip > backup-$(date +%F).sql.gz</div>
        <p>กู้คืน:</p><div class="code">gunzip -c backup-2026-01-31.sql.gz | docker compose exec -T mysql sh -c 'mysql -usmartoffice -p"$MYSQL_PASSWORD" smartoffice'</div></div></div>
      <div class="panel"><div class="panel-head"><h2>คำนวณสรุปรายวันใหม่</h2></div><div class="panel-body stack">
        <p class="small muted" style="margin:0">ใช้หลังกู้คืนข้อมูล หรือหลังแก้ไขเวลา/โต๊ะทำงานย้อนหลัง</p>
        <div class="row"><input type="date" class="input" data-from value="${daysAgo(30)}"><span class="muted">ถึง</span><input type="date" class="input" data-to value="${today()}"><button class="btn" data-recompute>${icon('refresh')} คำนวณใหม่</button></div></div></div>
    </div>`;
  el.querySelector('[data-export]').addEventListener('click', () => download('/system/backup', 'config.json').catch(showError));
  el.querySelector('[data-recompute]').addEventListener('click', async () => {
    try {
      const r = await post('/activity/recompute', { from: el.querySelector('[data-from]').value, to: el.querySelector('[data-to]').value });
      toast(`คำนวณใหม่เรียบร้อย ${r.rows} รายการ`);
    } catch (e) { showError(e); }
  });
}
