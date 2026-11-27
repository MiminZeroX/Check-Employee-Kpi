-- Phase 2: complete modules from the specification.

-- ---------------------------------------------------------------- roles & permissions
CREATE TABLE roles (
  name VARCHAR(40) PRIMARY KEY,
  label VARCHAR(100) NOT NULL,
  is_system TINYINT(1) NOT NULL DEFAULT 0
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE role_permissions (
  role VARCHAR(40) NOT NULL,
  permission VARCHAR(60) NOT NULL,
  PRIMARY KEY (role, permission),
  CONSTRAINT fk_rp_role FOREIGN KEY (role) REFERENCES roles(name) ON DELETE CASCADE ON UPDATE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

INSERT INTO roles (name, label, is_system) VALUES
 ('admin', 'ผู้ดูแลระบบ', 1), ('hr', 'ฝ่ายบุคคล', 1), ('manager', 'หัวหน้างาน', 1),
 ('viewer', 'ผู้ชม', 1), ('employee', 'พนักงาน', 1);

INSERT INTO role_permissions (role, permission) VALUES
 ('hr','dashboard.view'),('hr','live.view'),('hr','playback.view'),('hr','employees.view'),('hr','employees.manage'),
 ('hr','attendance.view'),('hr','attendance.manage'),('hr','org.manage'),('hr','analytics.view'),('hr','tasks.view'),
 ('hr','crm.view'),('hr','kpi.view'),('hr','kpi.manage'),('hr','kpi.review'),('hr','wellness.view'),('hr','reports.view'),
 ('hr','notifications.view'),('hr','privacy.review'),('hr','devices.view'),('hr','requests.approve'),('hr','hr.manage'),
 ('hr','documents.view'),('hr','documents.manage'),('hr','projects.view'),
 ('manager','dashboard.view'),('manager','live.view'),('manager','playback.view'),('manager','employees.view'),
 ('manager','attendance.view'),('manager','analytics.view'),('manager','tasks.view'),('manager','tasks.manage'),
 ('manager','crm.view'),('manager','crm.manage'),('manager','kpi.view'),('manager','kpi.review'),('manager','wellness.view'),
 ('manager','reports.view'),('manager','notifications.view'),('manager','privacy.review'),('manager','requests.approve'),
 ('manager','documents.view'),('manager','projects.view'),('manager','projects.manage'),('manager','tickets.manage'),
 ('viewer','dashboard.view'),('viewer','live.view'),('viewer','employees.view'),('viewer','attendance.view'),
 ('viewer','analytics.view'),('viewer','tasks.view'),('viewer','crm.view'),('viewer','notifications.view'),('viewer','projects.view');

ALTER TABLE users
  ADD COLUMN mfa_enabled TINYINT(1) NOT NULL DEFAULT 0,
  ADD COLUMN totp_secret VARCHAR(255) NULL,
  ADD COLUMN oidc_subject VARCHAR(200) NULL,
  ADD COLUMN token_version INT NOT NULL DEFAULT 0,
  ADD UNIQUE KEY uq_user_oidc (oidc_subject),
  ADD CONSTRAINT fk_user_role FOREIGN KEY (role) REFERENCES roles(name) ON UPDATE CASCADE;

-- ---------------------------------------------------------------- organisation
ALTER TABLE departments
  ADD COLUMN parent_id BIGINT NULL,
  ADD COLUMN manager_employee_id BIGINT NULL,
  ADD CONSTRAINT fk_dept_parent FOREIGN KEY (parent_id) REFERENCES departments(id) ON DELETE SET NULL,
  ADD CONSTRAINT fk_dept_manager FOREIGN KEY (manager_employee_id) REFERENCES employees(id) ON DELETE SET NULL;

-- ---------------------------------------------------------------- shifts, holidays, leave
CREATE TABLE shifts (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  name VARCHAR(100) NOT NULL,
  start_time TIME NOT NULL,
  end_time TIME NOT NULL,
  break_start TIME NULL,
  break_end TIME NULL,
  -- ISO weekdays the shift works by default (1 = Monday ... 7 = Sunday)
  work_days VARCHAR(20) NOT NULL DEFAULT '1,2,3,4,5',
  grace_min INT NOT NULL DEFAULT 0,
  is_default TINYINT(1) NOT NULL DEFAULT 0,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

INSERT INTO shifts (name, start_time, end_time, break_start, break_end, work_days, grace_min, is_default)
VALUES ('กะปกติ 08:30-17:30', '08:30:00', '17:30:00', '12:00:00', '13:00:00', '1,2,3,4,5', 5, 1);

ALTER TABLE employees
  ADD COLUMN shift_id BIGINT NULL,
  ADD CONSTRAINT fk_emp_shift FOREIGN KEY (shift_id) REFERENCES shifts(id) ON DELETE SET NULL;

-- per-day override (rotations, swaps); shift_id NULL = day off
CREATE TABLE shift_assignments (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  employee_id BIGINT NOT NULL,
  work_date DATE NOT NULL,
  shift_id BIGINT NULL,
  note VARCHAR(200) NULL,
  UNIQUE KEY uq_sa (employee_id, work_date),
  CONSTRAINT fk_sa_emp FOREIGN KEY (employee_id) REFERENCES employees(id) ON DELETE CASCADE,
  CONSTRAINT fk_sa_shift FOREIGN KEY (shift_id) REFERENCES shifts(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE holidays (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  holiday_date DATE NOT NULL,
  name VARCHAR(200) NOT NULL,
  branch_id BIGINT NULL,
  KEY idx_hol_date (holiday_date),
  CONSTRAINT fk_hol_branch FOREIGN KEY (branch_id) REFERENCES branches(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE leave_types (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  code VARCHAR(40) NOT NULL,
  name VARCHAR(100) NOT NULL,
  annual_quota_days DOUBLE NOT NULL DEFAULT 0,
  paid TINYINT(1) NOT NULL DEFAULT 1,
  is_active TINYINT(1) NOT NULL DEFAULT 1,
  UNIQUE KEY uq_lt_code (code)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

INSERT INTO leave_types (code, name, annual_quota_days, paid) VALUES
 ('sick', 'ลาป่วย', 30, 1), ('personal', 'ลากิจ', 3, 1), ('vacation', 'ลาพักร้อน', 6, 1), ('unpaid', 'ลาไม่รับค่าจ้าง', 0, 0);

-- leave, off-site work and overtime requests share one approval workflow
CREATE TABLE requests (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  employee_id BIGINT NOT NULL,
  -- leave | offsite | overtime
  kind VARCHAR(20) NOT NULL,
  leave_type_id BIGINT NULL,
  start_at DATETIME NOT NULL,
  end_at DATETIME NOT NULL,
  days DOUBLE NULL,
  hours DOUBLE NULL,
  location VARCHAR(200) NULL,
  reason VARCHAR(1000) NOT NULL,
  -- pending | approved | rejected | cancelled
  status VARCHAR(20) NOT NULL DEFAULT 'pending',
  approver_user_id BIGINT NULL,
  approver_note VARCHAR(1000) NULL,
  decided_at DATETIME NULL,
  created_by_user_id BIGINT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_req_emp (employee_id, start_at),
  KEY idx_req_status (status),
  CONSTRAINT fk_req_emp FOREIGN KEY (employee_id) REFERENCES employees(id) ON DELETE CASCADE,
  CONSTRAINT fk_req_lt FOREIGN KEY (leave_type_id) REFERENCES leave_types(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- one row per employee per day: schedule vs. what happened
CREATE TABLE work_sessions (
  employee_id BIGINT NOT NULL,
  work_date DATE NOT NULL,
  -- workday | holiday | dayoff
  day_type VARCHAR(20) NOT NULL DEFAULT 'workday',
  -- present | absent | leave | offsite | holiday | dayoff
  status VARCHAR(20) NOT NULL DEFAULT 'absent',
  shift_id BIGINT NULL,
  scheduled_start DATETIME NULL,
  scheduled_end DATETIME NULL,
  office_entry_at DATETIME NULL,
  check_in DATETIME NULL,
  check_out DATETIME NULL,
  first_desk_at DATETIME NULL,
  worked_sec INT NOT NULL DEFAULT 0,
  desk_sec INT NOT NULL DEFAULT 0,
  break_sec INT NOT NULL DEFAULT 0,
  late_min INT NOT NULL DEFAULT 0,
  early_leave_min INT NOT NULL DEFAULT 0,
  overtime_min INT NOT NULL DEFAULT 0,
  approved_overtime_min INT NOT NULL DEFAULT 0,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (employee_id, work_date),
  KEY idx_ws_date (work_date)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

ALTER TABLE daily_summaries
  ADD COLUMN meeting_sec INT NOT NULL DEFAULT 0,
  ADD COLUMN meeting_count INT NOT NULL DEFAULT 0,
  ADD COLUMN break_sec INT NOT NULL DEFAULT 0;

-- ---------------------------------------------------------------- devices & cameras
-- entrance terminals record "entered the office", office terminals "checked in"
ALTER TABLE devices ADD COLUMN location_role VARCHAR(20) NOT NULL DEFAULT 'office';

ALTER TABLE cameras
  ADD COLUMN privacy_masks TEXT NULL,
  ADD COLUMN playback_url VARCHAR(500) NULL,
  ADD COLUMN onvif_user VARCHAR(80) NULL,
  ADD COLUMN onvif_password VARCHAR(500) NULL;

-- zones other than desks: office area, entrance, meeting room (anonymous people counting)
CREATE TABLE detection_zones (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  camera_id BIGINT NOT NULL,
  name VARCHAR(120) NOT NULL,
  -- area | entrance | meeting_room | restricted
  kind VARCHAR(20) NOT NULL DEFAULT 'area',
  roi TEXT NOT NULL,
  capacity INT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_dz_cam (camera_id),
  CONSTRAINT fk_dz_cam FOREIGN KEY (camera_id) REFERENCES cameras(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE zone_occupancy (
  zone_id BIGINT NOT NULL,
  bucket_start DATETIME NOT NULL,
  max_people INT NOT NULL DEFAULT 0,
  people_sum DOUBLE NOT NULL DEFAULT 0,
  samples INT NOT NULL DEFAULT 0,
  entries INT NOT NULL DEFAULT 0,
  PRIMARY KEY (zone_id, bucket_start),
  KEY idx_zo_time (bucket_start),
  CONSTRAINT fk_zo_zone FOREIGN KEY (zone_id) REFERENCES detection_zones(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- ---------------------------------------------------------------- projects, CRM, service
CREATE TABLE projects (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  code VARCHAR(40) NULL,
  name VARCHAR(200) NOT NULL,
  customer_id BIGINT NULL,
  owner_employee_id BIGINT NULL,
  -- planning | active | on_hold | done | cancelled
  status VARCHAR(20) NOT NULL DEFAULT 'active',
  start_date DATE NULL,
  due_date DATE NULL,
  budget DOUBLE NULL,
  description TEXT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT fk_proj_cust FOREIGN KEY (customer_id) REFERENCES customers(id) ON DELETE SET NULL,
  CONSTRAINT fk_proj_owner FOREIGN KEY (owner_employee_id) REFERENCES employees(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

ALTER TABLE tasks
  ADD COLUMN project_id BIGINT NULL,
  ADD CONSTRAINT fk_task_project FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE SET NULL;

-- move free-text project names into real projects
INSERT INTO projects (name, status)
  SELECT DISTINCT project, 'active' FROM tasks WHERE project IS NOT NULL AND project <> '';
UPDATE tasks t JOIN projects p ON p.name = t.project SET t.project_id = p.id WHERE t.project IS NOT NULL;

CREATE TABLE contacts (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  customer_id BIGINT NOT NULL,
  name VARCHAR(200) NOT NULL,
  position VARCHAR(120) NULL,
  email VARCHAR(200) NULL,
  phone VARCHAR(40) NULL,
  is_primary TINYINT(1) NOT NULL DEFAULT 0,
  notes VARCHAR(1000) NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT fk_contact_cust FOREIGN KEY (customer_id) REFERENCES customers(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE deals (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  title VARCHAR(200) NOT NULL,
  customer_id BIGINT NULL,
  owner_employee_id BIGINT NULL,
  -- lead | qualified | proposal | negotiation | won | lost
  stage VARCHAR(20) NOT NULL DEFAULT 'lead',
  value DOUBLE NOT NULL DEFAULT 0,
  probability INT NOT NULL DEFAULT 10,
  expected_close DATE NULL,
  closed_at DATETIME NULL,
  notes VARCHAR(1000) NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT fk_deal_cust FOREIGN KEY (customer_id) REFERENCES customers(id) ON DELETE SET NULL,
  CONSTRAINT fk_deal_owner FOREIGN KEY (owner_employee_id) REFERENCES employees(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE tickets (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  subject VARCHAR(300) NOT NULL,
  description TEXT NULL,
  customer_id BIGINT NULL,
  contact_id BIGINT NULL,
  assignee_employee_id BIGINT NULL,
  -- low | normal | high | urgent
  priority VARCHAR(10) NOT NULL DEFAULT 'normal',
  -- open | in_progress | waiting | resolved | closed
  status VARCHAR(20) NOT NULL DEFAULT 'open',
  due_at DATETIME NULL,
  resolved_at DATETIME NULL,
  satisfaction TINYINT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_ticket_status (status),
  CONSTRAINT fk_ticket_cust FOREIGN KEY (customer_id) REFERENCES customers(id) ON DELETE SET NULL,
  CONSTRAINT fk_ticket_contact FOREIGN KEY (contact_id) REFERENCES contacts(id) ON DELETE SET NULL,
  CONSTRAINT fk_ticket_emp FOREIGN KEY (assignee_employee_id) REFERENCES employees(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE documents (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  title VARCHAR(200) NOT NULL,
  category VARCHAR(60) NULL,
  -- employee | customer | project | general
  entity VARCHAR(20) NOT NULL DEFAULT 'general',
  entity_id BIGINT NULL,
  file_name VARCHAR(255) NOT NULL,
  stored_name VARCHAR(80) NOT NULL,
  content_type VARCHAR(100) NOT NULL,
  size_bytes BIGINT NOT NULL,
  sha256 CHAR(64) NOT NULL,
  uploaded_by BIGINT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_doc_entity (entity, entity_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE webhooks (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  name VARCHAR(120) NOT NULL,
  url VARCHAR(500) NOT NULL,
  -- comma separated event names, or * for all
  events VARCHAR(500) NOT NULL DEFAULT '*',
  secret VARCHAR(255) NOT NULL,
  is_active TINYINT(1) NOT NULL DEFAULT 1,
  last_status INT NULL,
  last_error VARCHAR(300) NULL,
  last_sent_at DATETIME NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- ---------------------------------------------------------------- KPI & settings
INSERT INTO kpi_metrics (code, name, description, source, weight) VALUES
 ('project_efficiency', 'ประสิทธิภาพการดำเนินโครงการ', 'งานของโครงการที่เสร็จตรงกำหนด / งานของโครงการที่ครบกำหนดในช่วงเวลา', 'project_efficiency', 10);

INSERT INTO system_settings (setting_key, setting_value) VALUES
 ('report_doc_prefix', '"RPT"'),
 ('report_show_activity', 'true'),
 ('ticket_sla_hours', '{"low":72,"normal":48,"high":24,"urgent":4}');
