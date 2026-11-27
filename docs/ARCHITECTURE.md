# สถาปัตยกรรมระบบ (Architecture)

```
 กล้อง IP / RTSP / ONVIF                      เครื่องสแกนนิ้ว / สแกนหน้า
 (Hikvision, Dahua, ...)                      (ZKTeco ADMS, Hikvision ISAPI, REST)
        │ RTSP (เปิดครั้งเดียวต่อกล้อง)                     │ HTTP push
        ▼                                                    │
 ┌──────────────┐  WebRTC/MSE  ┌──────────────────┐          │
 │   go2rtc     │─────────────▶│  เบราว์เซอร์แอดมิน │          │
 │ (restreamer) │              │  web/ (Vanilla JS)│          │
 └──────┬───────┘              └────────┬─────────┘          │
        │ RTSP restream                 │ REST + WebSocket   │
        ▼                               ▼                    ▼
 ┌──────────────┐  segments +   ┌──────────────────────────────────┐    ┌─────────┐
 │  AI worker   │  heartbeat    │  API server (Rust / Axum / SQLx) │───▶│  MySQL  │
 │ Python + ONNX│──────────────▶│  auth · RBAC · analytics · KPI   │    └─────────┘
 │ YOLOX + YuNet│  (ไม่มีภาพ)    │  reports · audit · summaries     │
 └──────────────┘               └──────────────────────────────────┘
```

## ส่วนประกอบ

| ส่วน | เทคโนโลยี | หน้าที่ |
|---|---|---|
| `server/` | Rust, Axum 0.8, SQLx 0.8, Tokio | REST API, WebSocket, RBAC, สรุปรายวัน, KPI, รายงาน A4, รับข้อมูลจากอุปกรณ์, เสิร์ฟหน้าเว็บ |
| `web/` | HTML/CSS/ES modules ไม่มี build step | หน้าแอดมิน 22 เมนู + หน้าพนักงาน, Dark/Light mode, รองรับมือถือ |
| `ai-worker/` | Python, OpenCV, ONNX Runtime | อ่านภาพ → ตรวจคน/มือถือ → ทิศทางใบหน้า → สถานะของแต่ละโต๊ะ → ส่งเป็นช่วงเวลา |
| go2rtc | Go (MIT) | แปลง RTSP เป็น WebRTC ให้ดูบนเว็บ และ restream ให้ AI worker โดยเปิดกล้องเพียงครั้งเดียว |
| MySQL 8 | | เก็บข้อมูลทั้งหมด (ไม่เก็บวิดีโอ) |

แยก **video streaming** (go2rtc) ออกจาก **AI inference** (ai-worker) ตามข้อกำหนด ทำให้ขยายได้อิสระ:
เพิ่ม AI worker หลายเครื่อง (เช่น ต่อสาขาหรือแบบ Edge) โดยแต่ละเครื่องมี device key ของตัวเอง

## การตรวจจับกิจกรรม

ต่อเฟรม (ค่าเริ่มต้น 2 fps ต่อกล้อง ปรับได้ต่อกล้อง):

1. YOLOX ตรวจหา `person` และ `cell phone` (COCO)
2. เลือกบุคคลที่จุดกึ่งกลางอยู่ในพื้นที่ ROI ของโต๊ะ
3. ไม่มีบุคคล → `away` · มีแต่ความมั่นใจต่ำ → `unknown`
4. มีมือถืออยู่ในกรอบของบุคคล → `phone`
5. YuNet หาใบหน้าในครึ่งบนของบุคคล แล้วประเมินมุม yaw จากตำแหน่งจมูกเทียบกับดวงตา
   - ต่างจากมุมที่ปรับเทียบไว้ (`screen_yaw_deg`) ไม่เกิน `yaw_tolerance_deg` → `focused`
   - เกิน → `looking_away` · ไม่เห็นใบหน้า (เช่น หันหลังให้กล้อง) → `present`
6. `DeskTracker` กรองสัญญาณรบกวน: สถานะใหม่ต้องคงอยู่ `min_state_sec` (หรือ `away_grace_sec` สำหรับ away)
   ก่อนปิดช่วงเวลาเดิม แล้วส่งเป็น segment `{state, started_at, ended_at, confidence}`

ระบุตัวบุคคลจาก **การจับคู่โต๊ะ ↔ พนักงาน** ไม่ใช้ face recognition

## การคำนวณสรุปรายวัน

Job ทุก 60 วินาทีคำนวณ `daily_summaries` ของวันนี้และวันที่มีข้อมูลเข้ามาใหม่ (dirty dates):

- ระยะเวลาแต่ละสถานะ (ตัดตามขอบวันตาม timezone องค์กร), จำนวนครั้งใช้มือถือ / ลุกจากโต๊ะ
- `first_scan_at` จากเครื่องสแกน, `first_desk_at` = ช่วงแรกที่อยู่ที่โต๊ะหลังสแกน, `scan_to_desk_sec`
- `longest_sitting_sec` = ช่วงนั่งต่อเนื่องนานสุด (พักสั้นกว่า 2 นาทีไม่นับว่าลุก)
- `wellness_alerts` = จำนวนช่วงนั่งต่อเนื่องที่เกินค่าที่ตั้ง

ข้อมูลที่พนักงานขอแก้และได้รับอนุมัติ (`corrected_state`) ถูกใช้แทนค่าจาก AI และช่วงที่ถูกปฏิเสธจะไม่ถูกนับ

## KPI

คะแนน = ค่าเฉลี่ยถ่วงน้ำหนักของตัวชี้วัดที่มีข้อมูล (0-100) จากแหล่ง:
`task_completion`, `task_on_time`, `task_quality`, `attendance_punctuality`, `customer_satisfaction`, `manual`
ตัวชี้วัดกำหนดตามตำแหน่งได้ **ข้อมูลจากกล้องไม่ใช่แหล่งข้อมูลที่อนุญาต** (ตรวจสอบที่ API)

## เวลา

ฐานข้อมูลเก็บเวลาเป็น UTC ทั้งหมด การตัดวัน/แสดงผลใช้ `TZ_OFFSET_MINUTES` ขององค์กร (ไทย = 420)
เวลาจากเครื่องสแกนที่ไม่มี timezone ถือเป็นเวลาท้องถิ่นขององค์กร

## ความปลอดภัย

- รหัสผ่าน Argon2, JWT (HS256) อายุ 12 ชม., ตรวจสถานะบัญชีทุก request
- RBAC 5 บทบาท: admin, hr, manager, viewer, employee (employee เห็นเฉพาะข้อมูลตนเอง)
- อุปกรณ์ใช้ API key (เก็บเป็น SHA-256) ยกเว้น ZKTeco ADMS ที่ระบุด้วย Serial และต้องได้รับอนุมัติก่อน
- รหัสผ่านกล้องไม่ถูกส่งกลับไปที่หน้าเว็บ (แสดงเป็น `****`) และไม่อยู่ในไฟล์สำรองการตั้งค่า
- Audit log ทุกการเปลี่ยนแปลง, การเข้าสู่ระบบ, การออกรายงาน และการส่งออกข้อมูล
- ลบข้อมูลกิจกรรมอัตโนมัติตาม `retention_days`

## การขยายระบบ

| ขนาด | แนะนำ |
|---|---|
| ≤ 8 กล้อง | เครื่องเดียว 4 core / 8 GB, AI worker 1 ตัว, 1-2 fps |
| 8-32 กล้อง | แยก AI worker เป็นเครื่องต่างหาก, ใช้ `onnxruntime-openvino` บน Intel iGPU หรือ GPU |
| หลายสาขา | AI worker + go2rtc ที่แต่ละสาขา (Edge) ส่งเฉพาะ metadata มาที่ server กลาง |

ตาราง `activity_events` มี index ตาม (employee, เวลา), (workstation, เวลา) และเวลา
เมื่อข้อมูลเกินหลายสิบล้านแถว แนะนำทำ RANGE partition ตามเดือนบน `started_at`
