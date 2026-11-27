# API

Base URL: `http://SERVER:8080/api` · JSON · ผู้ใช้ส่ง `Authorization: Bearer <token>` · อุปกรณ์ส่ง `X-Device-Key: <key>`

สิทธิ์: **A** admin · **H** hr · **M** manager · **V** viewer · **E** employee (เฉพาะข้อมูลตนเอง) · admin เข้าถึงได้ทุกอย่าง

## ผู้ใช้และระบบ

ทุก `/api` จำกัด `RATE_LIMIT_RPS` ต่อ IP (ค่าเริ่มต้น 20/วินาที, burst 80) เกินแล้วได้ 429

| Method | Path | สิทธิ์ | คำอธิบาย |
|---|---|---|---|
| POST | `/auth/login` | - | `{username, password}` → `{token, user}` หรือ `{mfa_required, mfa_token}` |
| POST | `/auth/mfa/verify` | - | `{mfa_token, code}` → `{token, user}` |
| GET | `/auth/mfa` | ผู้ใช้ | `{enabled}` |
| POST | `/auth/mfa/setup` | ผู้ใช้ | `{password}` → `{secret, uri, qr}` (QR เป็น data URI) |
| POST | `/auth/mfa/enable` · `/auth/mfa/disable` | ผู้ใช้ | `{code}` · `{password, code}` |
| GET | `/auth/oidc` | - | `{enabled, label}` |
| GET | `/auth/oidc/start` → `/auth/oidc/callback` | - | redirect ไปผู้ให้บริการ SSO แล้วกลับมาที่ `/#sso=<code>` |
| POST | `/auth/oidc/exchange` | - | `{code}` → `{token, user}` (code ใช้ได้ครั้งเดียว 60 วินาที) |
| GET | `/auth/me` | ทุกคน | ข้อมูลผู้ใช้ปัจจุบัน |
| POST | `/auth/password` | ทุกคน | `{current, new}` |
| GET/POST | `/users` · PUT/DELETE `/users/{id}` | A | จัดการผู้ใช้และบทบาท |
| GET | `/public/config` | - | ชื่อองค์กร, URL go2rtc, timezone |
| GET | `/dashboard/overview` | HMV | ตัวเลขภาพรวมวันนี้ |
| GET | `/live` | HMV | สถานะสดของทุกโต๊ะ |
| GET | `/ws?token=` | HMV | WebSocket: `live`, `camera`, `events`, `attendance`, `wellness`, `correction` |
| GET/PUT | `/settings` | HMV / A | การตั้งค่าระบบ |
| GET | `/audit?q=&limit=&offset=` | A | Audit log |
| GET | `/system/resources` | A | RAM/CPU, สถานะฐานข้อมูล, ขนาดตาราง |
| GET | `/system/backup` | A | ส่งออกการตั้งค่าเป็น JSON |

## องค์กร กล้อง และอุปกรณ์

| Method | Path | สิทธิ์ |
|---|---|---|
| GET/POST, PUT/DELETE `{id}` | `/branches`, `/departments` | อ่าน: ทุกคน · เขียน: H |
| GET/POST, GET/PUT/DELETE `{id}` | `/employees?q=&department_id=&status=` | อ่าน: HMV · เขียน: H (DELETE = ปิดใช้งาน) |
| GET/POST, PUT/DELETE `{id}` | `/cameras` | อ่าน: HMV · เขียน: A |
| GET/POST, PUT/DELETE `{id}` | `/workstations?camera_id=` | อ่าน: HMV · เขียน: A |
| POST | `/workstations/{id}/calibrate` | A — ใช้มุมใบหน้าล่าสุดเป็นทิศทางหน้าจอ |
| GET/POST, PUT/DELETE `{id}` | `/devices` | อ่าน: H · เขียน: A (POST คืน `api_key` ครั้งเดียว) |
| POST | `/devices/{id}/rotate-key` | A |

## กิจกรรม เวลาเข้างาน และสุขภาพ

| Method | Path | สิทธิ์ |
|---|---|---|
| GET | `/activity/events?from=&to=&employee_id=&state=&limit=` | HMV |
| GET | `/activity/timeline?date=&department_id=` | HMV |
| GET | `/activity/summary?from=&to=&department_id=` | HMV |
| GET | `/activity/daily?employee_id=&from=&to=` | HMV |
| POST | `/activity/recompute` `{from, to}` | A |
| GET | `/attendance/daily?date=&department_id=` | HMV — สแกนเข้า, ถึงโต๊ะ, สาย |
| GET | `/attendance/logs?from=&to=&employee_id=` | HMV |
| GET | `/attendance/unmatched` | H |
| POST | `/attendance/manual` `{employee_id, scan_time, direction, note}` | H |
| GET | `/wellness/overview?from=&to=` | HM |
| GET | `/notifications` · POST `/notifications/{id}/read` | HMV |
| GET | `/corrections?status=` · POST `/corrections/{id}/review` `{approve, corrected_state?, note}` | HM |
| GET | `/me/summary`, `/me/daily`, `/me/events`, `/me/notifications` | E |
| GET/POST | `/me/corrections` `{activity_event_id, reason, requested_state?}` | E |

## งาน CRM KPI และรายงาน

| Method | Path | สิทธิ์ |
|---|---|---|
| GET/POST, PUT/DELETE `{id}` | `/customers` | อ่าน: HMV · เขียน: HM |
| GET/POST, PUT/DELETE `{id}` | `/tasks?assignee=&status=&customer_id=` | อ่าน: ทุกคน (E เห็นของตนเอง) · เขียน: HM |
| POST | `/tasks/{id}/status` `{status}` | ผู้รับผิดชอบ / HM |
| GET/POST, PUT/DELETE `{id}` | `/kpi/metrics` | อ่าน: ทุกคน · เขียน: H |
| GET | `/kpi/scores?from=&to=&department_id=&employee_id=` | HM (E ได้เฉพาะตนเอง) |
| GET/POST | `/kpi/records` | HM — คะแนนจากผู้ประเมิน |
| GET | `/reports/employee/{id}?from=&to=` | HM / E ตนเอง — HTML A4 |
| GET | `/reports/employee/{id}/csv` | HM / E ตนเอง |
| GET | `/reports/batch?from=&to=&department_id=` | HM — A4 ทุกคนในไฟล์เดียว |
| GET | `/reports/summary.csv?from=&to=&department_id=` | HM |
| GET | `/reports/history` | HM |

## Ingest (อุปกรณ์)

| Method | Path | ผู้ใช้ | Body |
|---|---|---|---|
| GET | `/ingest/config` | AI worker | — กล้อง, ROI, ค่าการตรวจจับ |
| POST | `/ingest/events` | AI worker | `{events:[{workstation_id, state, started_at, ended_at, confidence}]}` |
| POST | `/ingest/heartbeat` | AI worker | `{cameras:[{camera_id, online, fps, error}], desks:[{workstation_id, state, confidence, since, yaw}]}` |
| POST | `/ingest/attendance` | เครื่องสแกน (generic) | `{records:[{attendance_ref, scan_time, direction, method}]}` |
| POST | `/ingest/hikvision?key=` | Hikvision | ISAPI `AccessControllerEvent` (JSON หรือ multipart) |
| GET/POST | `/iclock/cdata?SN=` (ไม่มี `/api`) | ZKTeco ADMS | โปรโตคอล push มาตรฐานของ ZKTeco (ATTLOG) |

`state` ∈ `focused`, `present`, `phone`, `looking_away`, `away`, `unknown`
