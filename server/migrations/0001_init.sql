-- AI Smart Office Monitoring & Workforce Management System
-- Initial schema (MySQL 8.x / MariaDB 10.6+). All DATETIME values are stored in UTC.

CREATE TABLE organizations (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  name VARCHAR(200) NOT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE branches (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  organization_id BIGINT NULL,
  name VARCHAR(200) NOT NULL,
  address VARCHAR(500) NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT fk_branch_org FOREIGN KEY (organization_id) REFERENCES organizations(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE departments (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  branch_id BIGINT NULL,
  name VARCHAR(200) NOT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT fk_dept_branch FOREIGN KEY (branch_id) REFERENCES branches(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE employees (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  employee_code VARCHAR(40) NOT NULL,
  first_name VARCHAR(120) NOT NULL,
  last_name VARCHAR(120) NOT NULL DEFAULT '',
  nickname VARCHAR(60) NULL,
  position VARCHAR(120) NULL,
  department_id BIGINT NULL,
  email VARCHAR(200) NULL,
  phone VARCHAR(40) NULL,
  -- the user id / PIN of this person on the fingerprint / face scanner
  attendance_ref VARCHAR(60) NULL,
  work_start TIME NOT NULL DEFAULT '08:30:00',
  work_end TIME NOT NULL DEFAULT '17:30:00',
  monitoring_consent TINYINT(1) NOT NULL DEFAULT 0,
  status VARCHAR(20) NOT NULL DEFAULT 'active',
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_emp_code (employee_code),
  UNIQUE KEY uq_emp_attref (attendance_ref),
  KEY idx_emp_dept (department_id),
  CONSTRAINT fk_emp_dept FOREIGN KEY (department_id) REFERENCES departments(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE users (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  username VARCHAR(80) NOT NULL,
  display_name VARCHAR(200) NOT NULL DEFAULT '',
  password_hash VARCHAR(255) NOT NULL,
  -- admin | hr | manager | viewer | employee
  role VARCHAR(20) NOT NULL DEFAULT 'viewer',
  employee_id BIGINT NULL,
  is_active TINYINT(1) NOT NULL DEFAULT 1,
  last_login_at DATETIME NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_username (username),
  CONSTRAINT fk_user_emp FOREIGN KEY (employee_id) REFERENCES employees(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE cameras (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  name VARCHAR(120) NOT NULL,
  branch_id BIGINT NULL,
  floor VARCHAR(40) NULL,
  location VARCHAR(200) NULL,
  brand VARCHAR(60) NULL,
  rtsp_url VARCHAR(500) NOT NULL,
  onvif_host VARCHAR(200) NULL,
  -- name of the stream inside go2rtc (restreamer for browsers and the AI worker)
  stream_name VARCHAR(80) NOT NULL,
  ai_enabled TINYINT(1) NOT NULL DEFAULT 1,
  ai_fps DOUBLE NOT NULL DEFAULT 2,
  ai_width INT NOT NULL DEFAULT 960,
  ptz_supported TINYINT(1) NOT NULL DEFAULT 0,
  status VARCHAR(20) NOT NULL DEFAULT 'unknown',
  last_seen_at DATETIME NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_cam_stream (stream_name),
  CONSTRAINT fk_cam_branch FOREIGN KEY (branch_id) REFERENCES branches(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE workstations (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  camera_id BIGINT NOT NULL,
  name VARCHAR(120) NOT NULL,
  employee_id BIGINT NULL,
  -- JSON array of [x,y] points, normalised 0..1 relative to the camera frame
  roi TEXT NOT NULL,
  -- head yaw (degrees) observed when the person looks at the monitor; calibrated from the UI
  screen_yaw_deg DOUBLE NOT NULL DEFAULT 0,
  yaw_tolerance_deg DOUBLE NOT NULL DEFAULT 30,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_ws_cam (camera_id),
  KEY idx_ws_emp (employee_id),
  CONSTRAINT fk_ws_cam FOREIGN KEY (camera_id) REFERENCES cameras(id) ON DELETE CASCADE,
  CONSTRAINT fk_ws_emp FOREIGN KEY (employee_id) REFERENCES employees(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE devices (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  name VARCHAR(120) NOT NULL,
  -- ai_worker | zkteco | hikvision | generic
  kind VARCHAR(20) NOT NULL,
  serial_no VARCHAR(80) NULL,
  api_key_hash CHAR(64) NULL,
  branch_id BIGINT NULL,
  is_enabled TINYINT(1) NOT NULL DEFAULT 1,
  last_seen_at DATETIME NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_dev_serial (serial_no),
  UNIQUE KEY uq_dev_key (api_key_hash),
  CONSTRAINT fk_dev_branch FOREIGN KEY (branch_id) REFERENCES branches(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- One row per continuous activity segment observed at a workstation.
-- state: focused | present | phone | looking_away | away | unknown
CREATE TABLE activity_events (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  workstation_id BIGINT NOT NULL,
  camera_id BIGINT NOT NULL,
  employee_id BIGINT NULL,
  state VARCHAR(20) NOT NULL,
  started_at DATETIME(3) NOT NULL,
  ended_at DATETIME(3) NOT NULL,
  duration_sec INT NOT NULL,
  confidence DOUBLE NOT NULL DEFAULT 0,
  -- auto | disputed | corrected | rejected
  review_status VARCHAR(20) NOT NULL DEFAULT 'auto',
  corrected_state VARCHAR(20) NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_ae_emp_time (employee_id, started_at),
  KEY idx_ae_ws_time (workstation_id, started_at),
  KEY idx_ae_time (started_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE attendance_logs (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  employee_id BIGINT NULL,
  attendance_ref VARCHAR(60) NOT NULL,
  device_id BIGINT NULL,
  scan_time DATETIME NOT NULL,
  -- in | out | unknown
  direction VARCHAR(10) NOT NULL DEFAULT 'unknown',
  -- finger | face | card | password | manual | other
  method VARCHAR(20) NOT NULL DEFAULT 'other',
  raw VARCHAR(500) NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_att (attendance_ref, scan_time, device_id),
  KEY idx_att_emp_time (employee_id, scan_time),
  KEY idx_att_time (scan_time)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE daily_summaries (
  employee_id BIGINT NOT NULL,
  work_date DATE NOT NULL,
  first_scan_at DATETIME NULL,
  last_scan_at DATETIME NULL,
  first_desk_at DATETIME NULL,
  scan_to_desk_sec INT NULL,
  focused_sec INT NOT NULL DEFAULT 0,
  present_sec INT NOT NULL DEFAULT 0,
  phone_sec INT NOT NULL DEFAULT 0,
  looking_away_sec INT NOT NULL DEFAULT 0,
  away_sec INT NOT NULL DEFAULT 0,
  unknown_sec INT NOT NULL DEFAULT 0,
  phone_count INT NOT NULL DEFAULT 0,
  away_count INT NOT NULL DEFAULT 0,
  longest_sitting_sec INT NOT NULL DEFAULT 0,
  wellness_alerts INT NOT NULL DEFAULT 0,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (employee_id, work_date),
  KEY idx_ds_date (work_date)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE customers (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  name VARCHAR(200) NOT NULL,
  company VARCHAR(200) NULL,
  email VARCHAR(200) NULL,
  phone VARCHAR(40) NULL,
  -- lead | active | inactive
  status VARCHAR(20) NOT NULL DEFAULT 'lead',
  owner_employee_id BIGINT NULL,
  satisfaction TINYINT NULL,
  notes TEXT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT fk_cust_owner FOREIGN KEY (owner_employee_id) REFERENCES employees(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE tasks (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  title VARCHAR(300) NOT NULL,
  description TEXT NULL,
  project VARCHAR(200) NULL,
  assignee_employee_id BIGINT NULL,
  customer_id BIGINT NULL,
  -- low | normal | high
  priority VARCHAR(10) NOT NULL DEFAULT 'normal',
  -- todo | in_progress | done | cancelled
  status VARCHAR(20) NOT NULL DEFAULT 'todo',
  due_date DATE NULL,
  completed_at DATETIME NULL,
  -- 1..5 rated by the supervisor
  quality_score TINYINT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_task_assignee (assignee_employee_id, status),
  CONSTRAINT fk_task_emp FOREIGN KEY (assignee_employee_id) REFERENCES employees(id) ON DELETE SET NULL,
  CONSTRAINT fk_task_cust FOREIGN KEY (customer_id) REFERENCES customers(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- KPI metrics. Camera activity is intentionally NOT an allowed source:
-- it is shown as context only and never scored automatically.
CREATE TABLE kpi_metrics (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  code VARCHAR(60) NOT NULL,
  name VARCHAR(200) NOT NULL,
  description VARCHAR(500) NULL,
  -- manual | task_completion | task_on_time | task_quality | attendance_punctuality | customer_satisfaction
  source VARCHAR(30) NOT NULL DEFAULT 'manual',
  weight DOUBLE NOT NULL DEFAULT 10,
  -- NULL = applies to every position
  position VARCHAR(120) NULL,
  is_active TINYINT(1) NOT NULL DEFAULT 1,
  UNIQUE KEY uq_kpi_code (code)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE kpi_records (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  employee_id BIGINT NOT NULL,
  metric_id BIGINT NOT NULL,
  period_start DATE NOT NULL,
  period_end DATE NOT NULL,
  score DOUBLE NOT NULL,
  note VARCHAR(500) NULL,
  reviewer_user_id BIGINT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_kpi_rec (employee_id, metric_id, period_start, period_end),
  CONSTRAINT fk_kr_emp FOREIGN KEY (employee_id) REFERENCES employees(id) ON DELETE CASCADE,
  CONSTRAINT fk_kr_metric FOREIGN KEY (metric_id) REFERENCES kpi_metrics(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE reports (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  kind VARCHAR(40) NOT NULL,
  employee_id BIGINT NULL,
  period_start DATE NOT NULL,
  period_end DATE NOT NULL,
  doc_no VARCHAR(40) NULL,
  generated_by BIGINT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_rep_emp (employee_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE notifications (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  employee_id BIGINT NULL,
  kind VARCHAR(40) NOT NULL,
  title VARCHAR(200) NOT NULL,
  body VARCHAR(1000) NULL,
  is_read TINYINT(1) NOT NULL DEFAULT 0,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_notif_emp (employee_id, created_at),
  KEY idx_notif_time (created_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- Employees can dispute an AI-detected segment; a human reviews it.
CREATE TABLE data_corrections (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  activity_event_id BIGINT NOT NULL,
  employee_id BIGINT NOT NULL,
  reason VARCHAR(1000) NOT NULL,
  requested_state VARCHAR(20) NULL,
  -- pending | approved | rejected
  status VARCHAR(20) NOT NULL DEFAULT 'pending',
  reviewer_user_id BIGINT NULL,
  review_note VARCHAR(1000) NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  reviewed_at DATETIME NULL,
  KEY idx_corr_status (status),
  CONSTRAINT fk_corr_event FOREIGN KEY (activity_event_id) REFERENCES activity_events(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE audit_logs (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  user_id BIGINT NULL,
  username VARCHAR(80) NULL,
  action VARCHAR(80) NOT NULL,
  entity VARCHAR(60) NULL,
  entity_id VARCHAR(64) NULL,
  detail TEXT NULL,
  ip VARCHAR(64) NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_audit_time (created_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE system_settings (
  setting_key VARCHAR(80) PRIMARY KEY,
  setting_value TEXT NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

INSERT INTO organizations (name) VALUES ('My Organization');

INSERT INTO system_settings (setting_key, setting_value) VALUES
 ('org_name', '"บริษัท ตัวอย่าง จำกัด"'),
 ('report_header', '"รายงานผลการปฏิบัติงานและ KPI"'),
 ('report_footer', '"เอกสารนี้ใช้ประกอบการประเมินเท่านั้น ต้องผ่านการพิจารณาโดยผู้บังคับบัญชา"'),
 ('logo_url', '""'),
 ('retention_days', '180'),
 ('sitting_alert_min', '50'),
 ('min_state_sec', '5'),
 ('away_grace_sec', '20'),
 ('detect_confidence', '0.45'),
 ('monitoring_notice', '"องค์กรใช้กล้องเพื่อวิเคราะห์การใช้พื้นที่และส่งเสริมสุขภาพ ไม่ใช้ตัดสินผลงานโดยอัตโนมัติ"');

INSERT INTO kpi_metrics (code, name, description, source, weight) VALUES
 ('task_completion', 'อัตราการปิดงาน', 'งานที่เสร็จ / งานที่ได้รับมอบหมายในช่วงเวลา', 'task_completion', 30),
 ('task_on_time', 'ส่งงานตรงเวลา', 'งานที่เสร็จภายในกำหนด / งานที่เสร็จและมีกำหนดส่ง', 'task_on_time', 25),
 ('task_quality', 'คุณภาพของผลงาน', 'คะแนนคุณภาพเฉลี่ยจากผู้บังคับบัญชา (1-5)', 'task_quality', 20),
 ('punctuality', 'การมาทำงานตรงเวลา', 'วันที่สแกนเข้าก่อนเวลาเริ่มงาน / วันที่มาทำงาน', 'attendance_punctuality', 10),
 ('supervisor_review', 'ผลประเมินจากผู้บังคับบัญชา', 'กรอกคะแนน 0-100 โดยผู้ประเมิน', 'manual', 15);
