# API

Base URL: `http://SERVER:8080/api` · JSON · ผู้ใช้ส่ง `Authorization: Bearer <token>` · อุปกรณ์ส่ง `X-Device-Key: <key>`

สิทธิ์: **A** admin · **H** hr · **M** manager · **V** viewer · **E** employee (เฉพาะข้อมูลตนเอง) · admin เข้าถึงได้ทุกอย่าง

## ผู้ใช้และระบบ

ตัวกรอง `branch_id` (สาขาของแผนก) ใช้ได้กับ `/activity/summary`, `/activity/timeline`, `/kpi/scores`, `/analytics/heatmap`, `/work-sessions`, `/reports/*`

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
| GET | `/reports/summary.csv?from=&to=&department_id=&branch_id=` | HM |
| GET | `/reports/summary.xlsx?from=&to=&department_id=&branch_id=` | HM | Excel 2 ชีต: สรุป KPI + เวลาทำงานรายวัน |
| GET | `/reports/employee/{id}/xlsx?from=&to=` | HM / ตนเอง | Excel รายวันของพนักงาน |
| GET | `/reports/history` | HM |

## ERP

เงินเป็นตัวเลขทศนิยม 2 ตำแหน่ง (คำนวณแบบ exact decimal) · เอกสารที่ออกแล้วไม่ลบ ให้ "ยกเลิก" (เลขที่ยังคงอยู่) · ทุกเอกสารลงบัญชีอัตโนมัติ · ห้ามบันทึก/ยกเลิกรายการในงวดที่ปิดแล้ว (`accounting_lock_date`)

### บัญชี (`accounting.view` / `accounting.manage`)

| Method | Path | หมายเหตุ |
|---|---|---|
| GET/POST/PUT | `/accounting/accounts[/{id}]` | ผังบัญชี · บัญชีที่มี `system_role` ใช้ลงบัญชีอัตโนมัติ |
| GET/POST | `/accounting/journal?from=&to=&source=` | `{entry_date, memo, lines:[{account_id, debit, credit}]}` ต้องสมดุล |
| GET · POST | `/accounting/journal/{id}` · `/{id}/void` | ยกเลิกได้เฉพาะรายการที่บันทึกเอง |
| GET | `/accounting/trial-balance` · `/ledger/{account_id}` · `/profit-loss` · `/balance-sheet?as_of=` | งบทดลอง แยกประเภท กำไรขาดทุน ฐานะการเงิน |
| GET | `/accounting/aging/ar` · `/aging/ap` | อายุลูกหนี้ / เจ้าหนี้ |
| GET | `/accounting/vat-report?year=&month=` · `/wht-report` | ภ.พ.30 · ภ.ง.ด.1 / 3 / 53 |
| GET | `/accounting/financials.xlsx` · `/tax-reports.xlsx` | Excel |
| GET/PUT | `/accounting/settings` | เลขผู้เสียภาษี ที่อยู่ สาขา วันที่ปิดงวด |

### เงินเดือนและใบเบิก

| Method | Path | สิทธิ์ / หมายเหตุ |
|---|---|---|
| GET/PUT | `/payroll/settings` · `/payroll/employees[/{id}]` | `payroll.*` · อัตราประกันสังคม OT ข้อมูลเงินเดือนรายคน |
| GET/POST | `/payroll/runs` | `{year, month, pay_date}` สร้างและคำนวณจากเวลาทำงานจริง |
| POST | `/payroll/runs/{id}/compute` · `/approve` · `/pay` · `/void` | ฉบับร่าง → อนุมัติ (พนักงานเห็นสลิป) → จ่าย (ลงบัญชี) |
| GET | `/payroll/runs/{id}/bank.csv` · `/pnd1.xlsx` · `/sso.xlsx` | ไฟล์โอนธนาคาร ภ.ง.ด.1 สปส.1-10 |
| GET · POST · DELETE | `/payroll/payslips/{id}` · `/{id}/items` · `/payroll/items/{id}` | โบนัส / รายได้อื่น / รายการหักอื่น (ฉบับร่าง) |
| GET | `/payroll/payslips/{id}/print` · `/me/payslips` | สลิป A4 · พนักงานเห็นของตนเองหลังอนุมัติ |
| GET/POST/PUT/DELETE | `/expenses[/{id}]?scope=mine\|review&status=` | ใบเบิก (เจ้าของแก้ได้เฉพาะฉบับร่าง) |
| POST | `/expenses/{id}/submit` · `/decide` · `/pay` | `expenses.approve` (ในสายงาน) · `expenses.manage` |
| GET/POST/PUT | `/expense-categories[/{id}]` | หมวด → บัญชีค่าใช้จ่าย |

ใบเสร็จแนบผ่าน `POST /documents` ด้วย `entity=expense&entity_id={claim}` (เจ้าของใบเบิกแนบได้เอง)

### ขาย สินค้า คลัง

| Method | Path | หมายเหตุ |
|---|---|---|
| GET/POST/PUT | `/products[/{id}]` · `/products/{id}/stock` | สินค้า/บริการ ต้นทุนเฉลี่ย บัตรสต็อก ยอดยกมา |
| GET/POST/PUT | `/warehouses[/{id}]` | คลังหลายแห่ง |
| GET · POST | `/inventory/stock?warehouse_id=&low=` · `/inventory/adjust` · `/inventory/transfer` | สต็อกคงเหลือ ตรวจนับ โอนคลัง |
| GET/POST | `/sales/docs?doc_type=&status=` | `{doc_type: quotation\|invoice, customer_id, vat_mode, wht_rate, discount, lines}` |
| GET/PUT/DELETE | `/sales/docs/{id}` | แก้ไข/ลบได้เฉพาะใบเสนอราคาที่เปิดอยู่หรือใบแจ้งหนี้ฉบับร่าง |
| POST | `/sales/docs/{id}/convert` · `/issue` · `/status` · `/void` | ใบเสนอราคา → ใบแจ้งหนี้ → ออกใบกำกับภาษี (ให้เลขที่ ตัดสต็อก ลงบัญชี) |
| POST | `/sales/docs/{id}/payments` · `/sales/payments/{id}/void` | `{amount, wht_amount, method, account_id}` ออกใบเสร็จ RC |
| GET | `/sales/docs/{id}/print` · `/sales/summary` | A4 ต้นฉบับ + สำเนา |

### จัดซื้อ

| Method | Path | หมายเหตุ |
|---|---|---|
| GET/POST/PUT | `/suppliers[/{id}]` | นิติบุคคล (ภ.ง.ด.53) / บุคคลธรรมดา (ภ.ง.ด.3) |
| GET/POST/PUT | `/purchasing/orders[/{id}]` | ใบสั่งซื้อ |
| POST · GET | `/purchasing/orders/{id}/approve` · `/cancel` · `/print` | ผู้สร้างอนุมัติเองไม่ได้ (ยกเว้น admin) |
| GET/POST | `/purchasing/bills` | รับสินค้า + ใบกำกับผู้ขาย: สต็อกเข้า ภาษีซื้อ เจ้าหนี้ |
| GET · POST | `/purchasing/bills/{id}` · `/void` · `/payments` | จ่ายชำระ: ใบสำคัญจ่าย PV + 50 ทวิ WT |
| POST · GET | `/purchasing/payments/{id}/void` · `/print` | |

### ทรัพย์สินถาวร (`assets.manage`)

| Method | Path | หมายเหตุ |
|---|---|---|
| GET/POST/PUT | `/assets[/{id}]` | ทะเบียน + ตารางค่าเสื่อมที่เหลือ · `pay_account_id` = บันทึกการซื้อด้วย |
| POST | `/assets/depreciation` · `/assets/depreciation/void` | `{year, month}` คิดย้อนเดือนที่ขาด ลงบัญชีรายการเดียว |
| POST | `/assets/{id}/dispose` | `{date, amount, account_id}` กำไร/ขาดทุนจากการจำหน่าย |
| GET | `/assets/register.xlsx` | |

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
