# Check-Employee-KPI · AI Smart Office Monitoring & Workforce Management

ระบบบริหารจัดการสำนักงานอัจฉริยะ: ใช้กล้อง CCTV + AI วิเคราะห์การใช้งานโต๊ะทำงาน (มองหน้าจอ / ใช้มือถือ / มองทางอื่น / ไม่อยู่โต๊ะ) เป็นนาที
เชื่อมกับเครื่องสแกนนิ้ว/สแกนหน้าเพื่อดูว่า **สแกนเข้างานแล้วถึงโต๊ะกี่โมง** มีหน้าแอดมินแบบ VMS (คล้าย Hikvision) ดูกล้องบนเว็บ
ระบบงาน/CRM, KPI ถ่วงน้ำหนัก, ฟีเจอร์ส่งเสริมสุขภาพ และ **รายงาน KPI รายบุคคลขนาด A4** เก็บข้อมูลใน MySQL

> ข้อกำหนดตั้งต้น: [docs/SPEC.md](docs/SPEC.md) · สถานะแต่ละหัวข้อ: [docs/ROADMAP.md](docs/ROADMAP.md)

## หลักการสำคัญ

- **ข้อมูลกล้องเป็นข้อมูลประกอบ ไม่ใช่คะแนน** — KPI คิดจากผลงาน (งานเสร็จ ตรงเวลา คุณภาพ ความพึงพอใจลูกค้า การมาตรงเวลา ผลประเมิน) ระบบไม่อนุญาตให้ใช้ข้อมูลกล้องเป็นตัวชี้วัด
- **ไม่เก็บภาพ ไม่ใช้ face recognition** — AI ส่งเฉพาะสถานะ + เวลา ระบุบุคคลจากการจับคู่โต๊ะกับพนักงาน
- **พนักงานตรวจสอบและขอแก้ข้อมูลของตนเองได้** โดยมี HR/หัวหน้างานพิจารณา ทุกการกระทำอยู่ใน Audit log
- รายละเอียด PDPA: [docs/PRIVACY.md](docs/PRIVACY.md) · ผลตรวจความปลอดภัย/มัลแวร์: [docs/SECURITY.md](docs/SECURITY.md)

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
| `server/` | API server ภาษา **Rust** (Axum + SQLx) — ใช้ RAM ประมาณ 10 MB, migrations ของ MySQL |
| `web/` | หน้าแอดมิน 22 เมนู + หน้าพนักงาน (HTML/JS ล้วน ไม่ต้อง build) |
| `ai-worker/` | ตัวตรวจจับ AI (OpenCV + ONNX Runtime, YOLOX-Nano + YuNet) + โหมดจำลอง |
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
cp .env.example .env          # แก้รหัสผ่าน และ JWT_SECRET (openssl rand -hex 32)
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
# ทดสอบ unit
python3 -m unittest -v test_tracker test_stability
cd ../server && cargo test
# ทดสอบ end-to-end ทุกโมดูล (server ต้องรันอยู่และมีข้อมูลจาก seed_demo.py)
python3 tests/api_test.py          # หรือ --only hr,crm,vms,security,reports
# ชุดทดสอบ SSO ใช้ผู้ให้บริการจำลอง: เริ่ม server ด้วย OIDC_ISSUER=http://127.0.0.1:8099
#   OIDC_CLIENT_ID=smartoffice-test OIDC_REDIRECT_URL=http://localhost:8080/api/auth/oidc/callback RATE_LIMIT_RPS=200
```

ตัวแปรของ server: `DATABASE_URL`, `JWT_SECRET` (≥32 ตัวอักษร), `DATA_KEY`, `ADMIN_USERNAME`, `ADMIN_PASSWORD`, `TZ_OFFSET_MINUTES` (420),
`GO2RTC_API`, `GO2RTC_RTSP`, `BIND_ADDR`, `WEB_DIR`, `DATA_DIR`, `DB_MAX_CONNECTIONS`, `JWT_TTL_HOURS`, `TRUST_PROXY`, `RATE_LIMIT_RPS`,
`MFA_ISSUER`, `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET`, `OIDC_REDIRECT_URL` (ดู `deploy/.env.example`)

ข้อมูลมาก (หลายล้านแถว): รัน `scripts/partition_activity.sql` ครั้งเดียวเพื่อแบ่งตารางกิจกรรมเป็นรายเดือน
หลังจากนั้น server เพิ่มเดือนใหม่และลบเดือนที่เกินระยะเก็บข้อมูลให้อัตโนมัติ

## ทรัพยากรที่ใช้ (ออกแบบให้กิน RAM น้อย · วัดจริง)

| ส่วน | RAM | หมายเหตุ |
|---|---|---|
| API server (Rust, release) | **~10 MB** (สูงสุด ~28 MB ขณะยิง 600 request ใน 1.5 วินาที) | 2 threads, DB pool 5, binary 7.3 MB |
| ฐานข้อมูล MariaDB (`deploy/mariadb-lowmem.cnf`) | **~70 MB** (เดิม 146 MB) | buffer pool 64 MB, ปิด performance_schema |
| AI worker 4 กล้อง (YOLOX-Nano, LOW_MEMORY) | **~145 MB** | ~16 ms/เฟรมบน CPU · YOLOX-S เดิมใช้ 215 MB และ 128 ms |
| go2rtc | ~20-40 MB | |
| **รวมทั้งระบบ** | **ประมาณ 250-300 MB** | รันบนเครื่อง 1 GB RAM ได้ |

เปรียบเทียบโมเดลตรวจจับ (CPU 4 core, ภาพ 960px, ต่อ 1 กล้อง):

| โมเดล | RAM สูงสุด | เวลา/เฟรม |
|---|---|---|
| YOLOX-Nano (ค่าเริ่มต้น) | 97 MB | 16 ms |
| YOLOX-Tiny (จับมือถือได้ดีกว่า) | 124 MB | 37 ms |
| YOLOX-S | 159 MB | 132 ms |

ปรับเพิ่มได้: `AI_MODEL`, `AI_THREADS`, `SERVER_THREADS`, `ai_fps` ต่อกล้อง (1-2 fps เพียงพอ) และ `innodb_buffer_pool_size`

### ความเสถียรเมื่อพนักงานเคลื่อนไหวเร็ว

ไม่จำเป็นต้องใช้ 60-120 fps (กล้อง CCTV ส่งได้ 15-30 fps และ CPU รับไม่ไหว) ระบบใช้วิธีนี้แทน:

- **Adaptive FPS** — ปกติ 2 fps, เร่งเป็น 8 fps อัตโนมัติเมื่อมีการเคลื่อนไหวในพื้นที่โต๊ะ (`AI_BOOST_FPS`)
- **ไม่ค้าง** — อ่านภาพแยก thread เก็บเฉพาะภาพล่าสุด หน่วงจากภาพถึงผลประมาณ 60 ms
- **ไม่ดับ** — timeout, ต่อใหม่อัตโนมัติ, ตรวจจับภาพค้าง, สัญญาณหลุดสั้น ๆ ไม่ตัดข้อมูล
- **แม่นยำเมื่อภาพเบลอ** — ติดตามบุคคลต่อเนื่อง, ใช้การเคลื่อนไหวในพื้นที่โต๊ะยืนยันว่ายังมีคน, โหวตผลหลายภาพ
- หน้า Camera Management แสดง fps จริง, ความหน่วง และจำนวนครั้งที่ต่อใหม่ของแต่ละกล้อง

รายละเอียดและผลทดสอบ: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#การตรวจจับกิจกรรม)

## สถานะ

ทำครบ 8 เฟสตามข้อกำหนด (MVP → Production): AI + VMS, เครื่องสแกน, HR/กะ/ลา/OT, งาน/CRM/Ticket, KPI, รายงาน A4/Excel,
สุขภาพพนักงาน, MFA/SSO/HTTPS และเอกสารโครงการครบ 18 รายการ ([docs/PROJECT_DOCS.md](docs/PROJECT_DOCS.md))
กำลังพัฒนา ERP 4 โมดูล — ดู [docs/ROADMAP.md](docs/ROADMAP.md)

ก่อนใช้งานจริงควรทดสอบความแม่นยำกับมุมกล้องของสำนักงานจริงและดำเนินการตามเช็กลิสต์ใน [docs/PRIVACY.md](docs/PRIVACY.md)
และ [docs/SECURITY.md](docs/SECURITY.md)

## เอกสาร

| เอกสาร | เนื้อหา |
|---|---|
| [PROJECT_DOCS.md](docs/PROJECT_DOCS.md) | SRS, Feasibility, Technology, User Flow, Hardware, Testing, Deployment, Maintenance, Cost |
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | สถาปัตยกรรมและการตรวจจับ |
| [ER_DIAGRAM.md](docs/ER_DIAGRAM.md) | แผนภาพฐานข้อมูล (สร้างจากฐานข้อมูลจริง) |
| [API.md](docs/API.md) | API ทั้งหมด |
| [SECURITY.md](docs/SECURITY.md) · [PRIVACY.md](docs/PRIVACY.md) | ความปลอดภัย, PDPA |
| [SPEC.md](docs/SPEC.md) | ข้อกำหนดเดิมของโครงการ |
