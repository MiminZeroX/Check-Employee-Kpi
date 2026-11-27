# Check-Employee-KPI · AI Smart Office Monitoring & Workforce Management

ระบบบริหารจัดการสำนักงานอัจฉริยะ: ใช้กล้อง CCTV + AI วิเคราะห์การใช้งานโต๊ะทำงาน (มองหน้าจอ / ใช้มือถือ / มองทางอื่น / ไม่อยู่โต๊ะ) เป็นนาที
เชื่อมกับเครื่องสแกนนิ้ว/สแกนหน้าเพื่อดูว่า **สแกนเข้างานแล้วถึงโต๊ะกี่โมง** มีหน้าแอดมินแบบ VMS (คล้าย Hikvision) ดูกล้องบนเว็บ
ระบบงาน/CRM, KPI ถ่วงน้ำหนัก, ฟีเจอร์ส่งเสริมสุขภาพ และ **รายงาน KPI รายบุคคลขนาด A4** เก็บข้อมูลใน MySQL

> ข้อกำหนดตั้งต้น: [docs/SPEC.md](docs/SPEC.md) · สถานะแต่ละหัวข้อ: [docs/ROADMAP.md](docs/ROADMAP.md)

## หลักการสำคัญ

- **ข้อมูลกล้องเป็นข้อมูลประกอบ ไม่ใช่คะแนน** — KPI คิดจากผลงาน (งานเสร็จ ตรงเวลา คุณภาพ ความพึงพอใจลูกค้า การมาตรงเวลา ผลประเมิน) ระบบไม่อนุญาตให้ใช้ข้อมูลกล้องเป็นตัวชี้วัด
- **ไม่เก็บภาพ ไม่ใช้ face recognition** — AI ส่งเฉพาะสถานะ + เวลา ระบุบุคคลจากการจับคู่โต๊ะกับพนักงาน
- **พนักงานตรวจสอบและขอแก้ข้อมูลของตนเองได้** โดยมี HR/หัวหน้างานพิจารณา ทุกการกระทำอยู่ใน Audit log
- รายละเอียด PDPA: [docs/PRIVACY.md](docs/PRIVACY.md)

## สถาปัตยกรรม

```
กล้อง RTSP ──▶ go2rtc ──WebRTC──▶ หน้าเว็บแอดมิน (web/)
                 │ RTSP                    ▲ REST + WebSocket
                 ▼                         │
           AI worker (Python, ONNX) ──▶ API server (Rust/Axum) ──▶ MySQL
                                           ▲
          เครื่องสแกน ZKTeco / Hikvision / REST ─┘
```

| โฟลเดอร์ | เนื้อหา |
|---|---|
| `server/` | API server ภาษา **Rust** (Axum + SQLx) — ใช้ RAM ประมาณ 11 MB, migrations ของ MySQL |
| `web/` | หน้าแอดมิน 22 เมนู + หน้าพนักงาน (HTML/JS ล้วน ไม่ต้อง build) |
| `ai-worker/` | ตัวตรวจจับ AI (OpenCV + ONNX Runtime, YOLOX + YuNet) + โหมดจำลอง |
| `deploy/` | Docker Compose: MySQL, go2rtc, server, ai-worker |
| `scripts/seed_demo.py` | สร้างข้อมูลตัวอย่างผ่าน API จริง |
| `docs/` | สเปก สถาปัตยกรรม API ความเป็นส่วนตัว open source roadmap |

รายละเอียด: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) · [docs/API.md](docs/API.md) · [docs/OPEN_SOURCE.md](docs/OPEN_SOURCE.md)

## เมนูหน้าแอดมิน (22 เมนู)

| กลุ่ม | เมนู |
|---|---|
| ภาพรวม | Executive Dashboard · Live CCTV Monitoring (1-64 ช่อง) · Office Floor Plan |
| กล้องและ AI | Camera Management · AI Detection Management (วาด ROI, ปรับเทียบทิศจอ) · Device Integration |
| บุคลากร | Employee · Attendance (สแกน → ถึงโต๊ะ) · Department/สาขา |
| วิเคราะห์และผลงาน | Activity Analytics (timeline) · Task & Project · CRM · KPI & Performance · Health & Wellness · Report Center (A4/PDF/CSV) · Notification Center |
| ระบบ | User & Role · Audit Logs · Privacy & Data (คำขอแก้ไข) · System Settings · Server & Resources · Backup & Restore |

บทบาท: ผู้ดูแลระบบ · ฝ่ายบุคคล · หัวหน้างาน · ผู้ชม · พนักงาน (เห็นเฉพาะข้อมูลตนเอง, ขอแก้ไขข้อมูล, อัปเดตงาน, ดูรายงาน A4 ของตนเอง)

## ติดตั้งด้วย Docker (แนะนำ)

```bash
cd deploy
cp .env.example .env          # แก้รหัสผ่าน, JWT_SECRET, GO2RTC_PUBLIC_URL = IP ของเซิร์ฟเวอร์
docker compose up -d --build
docker compose logs server    # ถ้าไม่ได้ตั้ง ADMIN_PASSWORD จะแสดงรหัสผ่านที่สุ่มให้
```

เปิด `http://SERVER_IP:8080` แล้ว:

1. **Camera Management** → เพิ่มกล้อง (RTSP URL แนะนำ sub-stream) ระบบลงทะเบียนกับ go2rtc ให้อัตโนมัติ
2. **Employee Management** → เพิ่มพนักงาน พร้อม "รหัสในเครื่องสแกน"
3. **AI Detection Management** → เพิ่มโต๊ะ เลือกกล้องและพนักงาน กด "พื้นที่" วาดกรอบโต๊ะ แล้วให้พนักงานนั่งมองจอ กด "ปรับเทียบ"
4. **Device Integration** → เพิ่ม "AI Worker" คัดลอก key ใส่ `AI_DEVICE_KEY` ใน `.env` แล้ว `docker compose --profile ai up -d`
5. เครื่องสแกน ZKTeco: ตั้ง Cloud Server/ADMS ชี้มาที่ `SERVER_IP:8080` แล้วกดอนุมัติเครื่องในหน้า Device Integration
   Hikvision: ตั้ง HTTP Listening เป็น `http://SERVER_IP:8080/api/ingest/hikvision?key=KEY`

ทดลองโดยไม่มีกล้อง: ตั้ง `AI_SIMULATE=1` ระบบจะจำลองสถานะของทุกโต๊ะ

## พัฒนาบนเครื่อง

```bash
# MySQL/MariaDB: สร้างฐานข้อมูล smartoffice และผู้ใช้ smartoffice/smartoffice
cd server
JWT_SECRET=dev-secret-dev-secret-dev-secret-0123 ADMIN_PASSWORD=admin12345 WEB_DIR=../web cargo run
# อีกหน้าต่าง: ข้อมูลตัวอย่าง 3 สัปดาห์
python3 scripts/seed_demo.py --password admin12345 --days 21
# AI worker โหมดจำลอง (สร้าง key ในหน้า Device Integration ก่อน)
cd ai-worker && SERVER_URL=http://localhost:8080 DEVICE_KEY=... SIMULATE=1 python3 worker.py
# AI worker กับกล้องจริง
pip install -r requirements.txt && sh download_models.sh && SERVER_URL=... DEVICE_KEY=... python3 worker.py
# ทดสอบ
python3 -m unittest -v test_tracker
```

ตัวแปรของ server: `DATABASE_URL`, `JWT_SECRET` (≥32 ตัวอักษร), `ADMIN_USERNAME`, `ADMIN_PASSWORD`, `TZ_OFFSET_MINUTES` (420),
`GO2RTC_API`, `GO2RTC_RTSP`, `GO2RTC_PUBLIC_URL`, `BIND_ADDR`, `WEB_DIR`, `DB_MAX_CONNECTIONS`, `JWT_TTL_HOURS`

## ทรัพยากรที่ใช้ (วัดจริงในการทดสอบ)

| ส่วน | ผลวัด |
|---|---|
| API server (release build) | RAM ~11 MB ขณะทำงาน, สูงสุด ~28 MB ระหว่างยิง 600 request วิเคราะห์/KPI ใน 1.5 วินาที · binary 7.3 MB |
| AI worker ต่อเฟรม (CPU 4 core, YOLOX 640px + YuNet) | ~130-160 ms → 1 worker รองรับประมาณ 6-10 กล้องที่ 1 fps |
| ข้อมูลกิจกรรม | ~1 แถวต่อการเปลี่ยนสถานะ (ประมาณ 50-150 แถว/คน/วัน) |

## สถานะ

เวอร์ชันนี้คือ **Phase 1 (MVP)** ใช้งานได้ครบวงจร: กล้อง → AI → MySQL → วิเคราะห์ → KPI → รายงาน A4
ก่อนใช้งานจริงควรทดสอบความแม่นยำกับมุมกล้องของสำนักงานจริงและดำเนินการตามเช็กลิสต์ใน [docs/PRIVACY.md](docs/PRIVACY.md)
สิ่งที่ยังไม่ทำ (PTZ, playback, Leave/Shift, MFA/SSO, .xlsx ฯลฯ) ดูที่ [docs/ROADMAP.md](docs/ROADMAP.md)
