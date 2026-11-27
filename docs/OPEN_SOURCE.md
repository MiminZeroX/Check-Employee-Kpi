# Open Source ที่ใช้และที่พิจารณา

## ใช้ในระบบ

| โครงการ | License | ใช้ทำอะไร |
|---|---|---|
| [go2rtc](https://github.com/AlexxIT/go2rtc) | MIT | Restream RTSP → WebRTC/MSE สำหรับหน้าเว็บ และ RTSP ให้ AI worker |
| [YOLOX](https://github.com/Megvii-BaseDetection/YOLOX) (ผ่าน OpenCV Zoo) | Apache-2.0 | ตรวจจับบุคคลและโทรศัพท์มือถือ |
| [YuNet](https://github.com/opencv/opencv_zoo/tree/main/models/face_detection_yunet) | MIT | ตรวจใบหน้า + landmark เพื่อประเมินทิศทางการมอง |
| [ONNX Runtime](https://github.com/microsoft/onnxruntime) | MIT | รันโมเดล (รองรับ OpenVINO / CUDA) |
| [OpenCV](https://github.com/opencv/opencv) | Apache-2.0 | อ่านสตรีม ประมวลผลภาพ |
| Axum, Tokio, SQLx, Serde, jsonwebtoken, argon2 | MIT / Apache-2.0 | API server (Rust) |
| MySQL | GPL-2.0 (ใช้เป็นบริการแยก) | ฐานข้อมูล |

ทุก license ข้างต้นอนุญาตให้ใช้ในเชิงพาณิชย์ได้

## พิจารณาแล้ว

| โครงการ | License | ผลการพิจารณา |
|---|---|---|
| [Frigate NVR](https://github.com/blakeblackshear/frigate) | MIT | NVR + object detection ที่ดีมาก เหมาะถ้าต้องการ **บันทึกวิดีโอย้อนหลัง** สามารถใช้คู่กันได้ (Frigate ก็ใช้ go2rtc ภายใน) แต่ไม่มีแนวคิดโต๊ะ/พนักงาน/ทิศทางการมอง จึงไม่ใช้เป็นแกนหลัก |
| ZoneMinder, Shinobi | GPL / มีเงื่อนไข | VMS ที่หนักกว่าและ license จำกัดกว่า |
| MediaMTX | MIT | ทางเลือกแทน go2rtc |
| Ultralytics YOLOv8/YOLO11 | **AGPL-3.0** | แม่นยำกว่าเล็กน้อย ระบบรองรับรูปแบบโมเดลนี้ แต่ต้องซื้อ license หรือเปิดซอร์สทั้งระบบหากใช้เชิงพาณิชย์ |
| MediaPipe Face Mesh | Apache-2.0 | ประเมินทิศทางศีรษะได้ละเอียดกว่า แต่หนักกว่า YuNet อาจใช้ในอนาคต |
| Odoo / ERPNext / Twenty CRM | LGPL / GPL / AGPL | CRM/HR ครบกว่า แต่หนักและผูกกับสถาปัตยกรรมของตัวเอง ระบบนี้มี CRM/Task พื้นฐานในตัว และเชื่อมต่อระบบภายนอกผ่าน REST API ได้ |

## โปรโตคอลเครื่องสแกน

- **ZKTeco** — ADMS / Push protocol (`/iclock/cdata`) เครื่องรุ่นที่มีเมนู Cloud Server Setting ส่งข้อมูลเข้ามาเองโดยไม่ต้องใช้ SDK
- **Hikvision** — ISAPI event push (`AccessControllerEvent`) ผ่าน HTTP Listening
- **อื่น ๆ** — ส่ง JSON ผ่าน `/api/ingest/attendance` หรือเขียนสคริปต์ดึงจาก SDK/ฐานข้อมูลของเครื่องแล้วส่งเข้ามา
