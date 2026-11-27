# Roadmap และสถานะการพัฒนา

✅ ทำแล้ว · 🟡 ทำบางส่วน · ⏳ ยังไม่ทำ (เฟสถัดไป)

## Phase 1 — MVP (ปัจจุบัน)

| หัวข้อในสเปก | สถานะ | หมายเหตุ |
|---|---|---|
| 2. AI ตรวจจับ: มองจอ / มือถือ / มองที่อื่น / ไม่อยู่โต๊ะ | ✅ | YOLOX + YuNet, debounce, confidence, สถานะ Unknown |
| 2. นั่งต่อเนื่อง, เดินเข้า-ออก, กลับมาที่โต๊ะ | ✅ | คำนวณจากช่วงเวลา (longest sitting, away count) |
| 2. พูดคุย/ประชุมกับเพื่อนร่วมงาน | ⏳ | ต้องตรวจหลายบุคคลในโซนเดียวกัน |
| 3. เพิ่ม/ลบ/แก้กล้อง, RTSP, หลายยี่ห้อ, Online/Offline | ✅ | |
| 3. ภาพสดบนเว็บ 1/4/9/16/25/36/64 ช่อง, Full screen | ✅ | ผ่าน go2rtc (WebRTC/MSE) |
| 3. ROI, ตำแหน่งโต๊ะ, เปิด/ปิด AI, FPS/ความละเอียด | ✅ | วาด ROI บนภาพจริง + ปรับเทียบทิศจอ |
| 3. หลายอาคาร/ชั้น/สาขา | ✅ | branch + floor + location |
| 3. PTZ, ONVIF discovery | ⏳ | มี flag `ptz_supported` แล้ว |
| 3. ดูภาพย้อนหลัง (Timeline playback) | ⏳ | แนะนำใช้ NVR เดิม หรือ Frigate คู่กับระบบ |
| 4. เชื่อมเครื่องสแกน (API/Webhook/Push) | ✅ | ZKTeco ADMS, Hikvision ISAPI, REST JSON, บันทึกแทนโดย HR |
| 4. เวลาสแกน → ถึงโต๊ะ, มาสาย, เวลาออก, OT | ✅ | |
| 4. ลา วันหยุด กะ งานนอกสถานที่ | ⏳ | |
| 5. Employee / Department / Org / Task / Project / Customer / KPI / Wellness / Notification | ✅ | Project เป็นฟิลด์ของงาน |
| 5. Leave, Shift, Contact, Sales Pipeline, Ticket, Document, Approval Workflow | ⏳ | |
| 6. Admin Dashboard 22 เมนู, Dark/Light, Responsive, RBAC | ✅ | |
| 7. วิเคราะห์รายบุคคล/แผนก/องค์กร, วัน/สัปดาห์/เดือน/ไตรมาส/ปี | ✅ | สาขา: กรองที่หน้า Live / แผนก |
| 7. KPI ถ่วงน้ำหนักตามตำแหน่ง ไม่ใช้ข้อมูลกล้องเป็นคะแนน | ✅ | |
| 8. รายงาน A4 / PDF (print), CSV, Print preview, โลโก้/หัว/ท้าย/เลขที่เอกสาร | ✅ | |
| 8. Excel (.xlsx) แท้ | 🟡 | CSV แบบ UTF-8 BOM เปิดใน Excel ได้ |
| 9. Wellness: แจ้งเตือนนั่งนาน, 20-20-20, OT, Work-Life Balance, พนักงานแก้ข้อมูลเอง | ✅ | |
| 11. MySQL: index, retention, migration, backup | ✅ | partition ⏳ (ดู ARCHITECTURE) |
| 12. RBAC, Audit, Session, Retention, พนักงานตรวจสอบข้อมูล | ✅ | |
| 12. MFA, SSO, API rate limit, ปิดบังใบหน้าในภาพสด | ⏳ | login มีหน่วงเวลาเมื่อผิด |
| 13. Modular, On-prem/Hybrid, Edge AI | ✅ | AI worker หลายตัว/หลายสาขาได้ |
| 10. Rust สำหรับ production | 🟡 | API server เป็น Rust แล้ว · AI worker เป็น Python (ONNX Runtime ทำงานใน C++ อยู่แล้ว) |

## Phase 2 — Production hardening

1. ทดสอบกับกล้องจริงในสำนักงาน เก็บชุดข้อมูลตัวอย่าง วัด precision/recall ของแต่ละสถานะ และปรับเกณฑ์ต่อมุมกล้อง
2. HTTPS (Caddy) + MFA (TOTP) + SSO (OIDC / Azure AD / Google Workspace)
3. Leave / Shift / Holiday เพื่อคำนวณการมาสายและ OT ให้ตรงตารางงาน
4. Privacy mask (เบลอใบหน้า) ในภาพสดสำหรับบทบาท viewer
5. ส่งออก .xlsx, ตั้งเวลาส่งรายงานทางอีเมล
6. Partition ตาราง `activity_events` รายเดือน + archive

## Phase 3 — Scale

1. ย้าย AI worker เป็น Rust (`ort` + `opencv` crates) หรือใช้ OpenVINO/TensorRT
2. Dashboard รวมหลายสาขา, Edge sync แบบ offline-first
3. PTZ / ONVIF discovery / playback ผ่าน NVR API (Hikvision ISAPI)
4. ตรวจการประชุม/พูดคุย, heatmap การใช้พื้นที่
