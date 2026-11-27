# ผลตรวจความปลอดภัยและมัลแวร์

ตรวจเมื่อ 27 พ.ย. 2569 ทั้งโค้ด library ไฟล์โมเดล AI และการตั้งค่าติดตั้ง

## มัลแวร์ — ไม่พบ

| ตรวจ | ผล |
|---|---|
| ประวัติ git | 1 commit เริ่มต้นของเจ้าของ repo + commit ที่สร้างระบบนี้ ไม่มีไฟล์ binary ใน repo |
| รูปแบบโค้ดอันตราย (`eval`, `exec`, `subprocess`, `base64`, `pickle`, โค้ดถูกซ่อน) | ไม่พบ |
| การเชื่อมต่อออกภายนอก | มีเพียง GitHub (ดาวน์โหลดโมเดล ตรวจ SHA-256) และ Google Fonts (ฟอนต์ในรายงาน A4) ไม่มีการส่งข้อมูลออก |
| รหัสลับฝังในโค้ด | ไม่พบ |
| ไฟล์โมเดล ONNX | ใช้เฉพาะ operator มาตรฐาน ไม่มี custom op / external data · ONNX ไม่สามารถรันโค้ดได้ (ต่างจากไฟล์ .pt/pickle) |
| Library Python (`pip-audit`) | ไม่พบช่องโหว่ |
| Library Rust (`cargo audit`) | 2 รายการ ไม่กระทบระบบนี้ (ดูด้านล่าง) |

`cargo audit`:
- `libsqlite3-sys` (RUSTSEC-2026-0459) — อยู่ใน Cargo.lock เท่านั้น ไม่ถูก compile (ระบบไม่ใช้ SQLite)
- `rsa` Marvin Attack (RUSTSEC-2023-0071) — เป็นการโจมตีฝั่งถอดรหัสด้วย private key แต่ sqlx ใช้เพียง public key ของ MySQL เข้ารหัสรหัสผ่าน จึงไม่กระทบ ยังไม่มีเวอร์ชันแก้
- `chrono` (คำเตือน unmaintained) — ไม่ใช่ช่องโหว่ ติดตามทางเลือกในอนาคต

## ช่องโหว่ที่พบและแก้แล้ว

| # | ระดับ | ปัญหา | การแก้ | ทดสอบ |
|---|---|---|---|---|
| 1 | สูง | go2rtc API (1984) ไม่มีการยืนยันตัวตนและสั่งรันคำสั่งได้ (`exec`) ทุกคนใน LAN ดูกล้องได้ | ไม่เปิดพอร์ต 1984/8554 ออกนอก Docker · ดูภาพผ่าน proxy `/video/*` ของ server ที่ต้อง login (cookie HttpOnly, SameSite=Strict) อนุญาตเฉพาะ player/snapshot/WebSocket · `exec` จำกัดเฉพาะ ffmpeg | ไม่มี cookie → 401, พนักงาน → 403, `/video/api/streams` → 404, PUT → 405 |
| 2 | สูง | ใช้ `JWT_SECRET` ตัวอย่างแล้วปลอม token admin ได้ | server ไม่ยอมเริ่มทำงานถ้า secret เป็นค่าตัวอย่าง | exit 1 พร้อมข้อความ |
| 3 | สูง | ข้อมูลจากอุปกรณ์ (heartbeat) แสดงในหน้าเว็บโดยไม่ escape → XSS | server รับเฉพาะฟิลด์ตัวเลขที่กำหนด, หน้าเว็บแปลงเป็นตัวเลขก่อนแสดง | ส่ง HTML ใน fps → 422, ฟิลด์แปลกถูกทิ้ง |
| 4 | กลาง | login ไม่มีการล็อก, เดาชื่อผู้ใช้จากเวลาตอบได้ | ล็อก 15 นาทีเมื่อผิด 5 ครั้งต่อชื่อผู้ใช้ / 30 ครั้งต่อ IP · ตรวจรหัสกับ hash หลอกเมื่อไม่มีผู้ใช้ | ครั้งที่ 6 → 429, เวลาตอบเท่ากัน |
| 5 | กลาง | ไม่มี security headers | CSP (script เฉพาะจาก server), nosniff, X-Frame-Options, Referrer-Policy, Permissions-Policy · ย้าย inline script ออก | ทุกหน้าไม่มี CSP violation |
| 6 | กลาง | CSV formula injection ใน Excel | ใส่ `'` นำหน้าค่าที่ขึ้นต้นด้วย `= + - @` | `=cmd|calc` → `'=cmd|calc` |
| 7 | กลาง | endpoint ZKTeco (ไม่มี auth ตามโปรโตคอล) ถูกยิงลงทะเบียนเครื่องปลอมได้ไม่จำกัด | จำกัดเครื่องรออนุมัติไม่เกิน 20 | ยิง 30 → บันทึก 20 |
| 8 | กลาง | ดาวน์โหลดโมเดลไม่ตรวจความถูกต้อง | ตรวจ SHA-256 และบังคับ HTTPS | ไฟล์ถูกแก้/ผิด → ปฏิเสธ |
| 9 | ต่ำ | WebSocket ไม่ตรวจบัญชีที่ถูกปิด | ตรวจ `is_active` | |

ที่ตรวจแล้วไม่พบปัญหา: SQL injection (ใช้ parameter binding ทุกคำสั่ง), path traversal, การแยกสิทธิ์ 5 บทบาท,
รหัสผ่าน Argon2, device key เก็บเป็น hash, รหัสผ่านกล้องไม่ถูกส่งไปหน้าเว็บ/ไฟล์สำรอง, container ไม่ได้รันเป็น root

## การยืนยันตัวตนและการเข้าถึง (เฟส 6)

| ระบบ | รายละเอียด |
|---|---|
| **MFA (TOTP)** | ผู้ใช้เปิดเองที่ปุ่มกุญแจ → "ความปลอดภัยของบัญชี" · สแกน QR ด้วย Google/Microsoft Authenticator · ต้องยืนยันรหัสผ่านก่อนตั้งค่า/ปิด · secret เข้ารหัส AES-256-GCM ในฐานข้อมูล · รหัสใช้ซ้ำไม่ได้ (เก็บ time step ล่าสุด) · ผิด 5 ครั้งล็อก 15 นาที · โทเคนรอ MFA อายุ 5 นาที เซ็นด้วยกุญแจแยก ใช้แทน session ไม่ได้ · admin รีเซ็ต MFA ให้ผู้ใช้ได้ |
| **SSO (OpenID Connect)** | Google Workspace, Microsoft Entra ID, Keycloak, Authentik ฯลฯ · Authorization Code + PKCE (S256) + state ผูกกับเบราว์เซอร์ด้วยคุกกี้ + nonce · ตรวจลายเซ็น ID token ด้วย JWKS (RS/PS/ES เท่านั้น), iss, aud, exp · ผูกบัญชีครั้งแรกด้วยอีเมลที่ยืนยันแล้ว (ต้องตรงกับผู้ใช้เพียงคนเดียว) จากนั้นใช้ subject · ไม่สร้างผู้ใช้ใหม่อัตโนมัติ · ส่ง session ให้หน้าเว็บด้วยรหัสใช้ครั้งเดียวอายุ 60 วินาที ไม่ใส่ token ใน URL |
| **Rate limit** | token bucket ต่อ IP ทุก `/api` (ค่าเริ่มต้น 20 req/s, burst 80) ตอบ 429 + Retry-After · login มีล็อกแยกต่อผู้ใช้/ต่อ IP อยู่แล้ว |
| **HTTPS** | `docker compose --profile https up -d` ใช้ Caddy ออกใบรับรองอัตโนมัติ (Let's Encrypt สำหรับโดเมนจริง หรือ CA ภายในสำหรับ LAN) · HSTS · `/iclock` ของเครื่องสแกนยังรับ HTTP ได้ |
| **IP จริงหลัง proxy** | `TRUST_PROXY=1` ใช้ค่า X-Forwarded-For ตัวขวาสุด (ที่ Caddy ใส่) · เปิดเฉพาะเมื่อพอร์ต 8080 ไม่เปิดให้เครื่องอื่นเข้าตรง (`SERVER_PUBLISH=127.0.0.1:8080`) |

ตั้งค่า SSO: ลงทะเบียนแอปที่ผู้ให้บริการ ใส่ Redirect URI = `https://<โดเมน>/api/auth/oidc/callback`
แล้วกำหนด `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET`, `OIDC_REDIRECT_URL` ใน `.env` และใส่อีเมลบริษัทให้ผู้ใช้
(หรือพนักงานที่ผูกกับผู้ใช้) ก่อนเข้าสู่ระบบครั้งแรก · ใช้ HTTPS กับ SSO เสมอ

ใบรับรอง LAN (`TLS_MODE=internal`): คัดลอก root CA จาก `docker compose cp caddy:/data/caddy/pki/authorities/local/root.crt .`
แล้วติดตั้งเป็น Trusted Root ในเครื่องผู้ใช้หนึ่งครั้ง

## สิ่งที่ผู้ติดตั้งต้องทำ

- [ ] สร้าง `JWT_SECRET` ด้วย `openssl rand -hex 32` และตั้งรหัสผ่านฐานข้อมูลใหม่ทั้งหมด
- [ ] ใช้ HTTPS (`--profile https`) โดยเฉพาะถ้าเข้าจากนอกสำนักงาน และแนะนำให้ผู้ดูแลระบบ/HR เปิด MFA
- [ ] firewall: เปิดเฉพาะ 8080 (หรือ 443) และ 8555 ให้ LAN · ห้ามเปิด 1984 / 8554 / 3306
- [ ] ให้เครื่องสแกน ZKTeco อยู่ใน VLAN ภายใน (โปรโตคอล ADMS ไม่มีการยืนยันตัวตน)
- [ ] เปลี่ยนรหัสผ่าน admin หลังติดตั้ง และทบทวนรายชื่อผู้ใช้เป็นระยะ

## ข้อจำกัดที่ยังเหลือ

- token เก็บใน localStorage (มี CSP ช่วยป้องกัน XSS) · เปลี่ยนรหัสผ่าน / ออกจากทุกอุปกรณ์ / ปิดบัญชี / รีเซ็ต MFA ยกเลิก token เดิมทันที
- ยังไม่มีรหัสสำรอง (recovery codes) ของ MFA — ถ้าโทรศัพท์หาย ให้ admin รีเซ็ต MFA
- rate limit เก็บในหน่วยความจำของ server ตัวเดียว (ถ้ารันหลายเครื่องต้องทำที่ proxy)
- Docker image อ้างอิงตาม tag ไม่ได้ pin digest
- รายงาน A4 โหลดฟอนต์จาก Google Fonts (ถ้าไม่ต้องการให้ลบบรรทัด `fonts.googleapis.com` ในรายงาน)
