# ER Diagram

สร้างอัตโนมัติจากฐานข้อมูลจริงด้วย `scripts/gen_er_diagram.py` · 62 ตาราง · 76 foreign key

PK = primary key · FK = foreign key · UK = unique · แสดงเฉพาะคีย์และคอลัมน์หลัก (ดูคอลัมน์ทั้งหมดใน `server/migrations/`)

## องค์กรและผู้ใช้

```mermaid
erDiagram
  organizations {
    bigint id PK
    varchar name
    datetime created_at
  }
  branches {
    bigint id PK
    bigint organization_id FK
    varchar name
    varchar address
    datetime created_at
  }
  departments {
    bigint id PK
    bigint branch_id FK
    varchar name
    datetime created_at
    bigint parent_id FK
    bigint manager_employee_id FK
  }
  employees {
    bigint id PK
    varchar employee_code UK
    varchar first_name
    varchar last_name
    varchar nickname
    varchar position
    bigint department_id FK
    varchar email
    varchar phone
    varchar attendance_ref UK
    time work_start
    time work_end
    bigint shift_id FK
  }
  users {
    bigint id PK
    varchar username UK
    varchar display_name
    varchar email
    varchar password_hash
    varchar role FK
    bigint employee_id FK
    tinyint is_active
    datetime last_login_at
    datetime created_at
    tinyint mfa_enabled
    varchar totp_secret
    varchar oidc_subject UK
  }
  roles {
    varchar name PK
    varchar label
    tinyint is_system
  }
  role_permissions {
    varchar role PK
    varchar permission PK
  }
  system_settings {
    varchar setting_key PK
    text setting_value
  }
  audit_logs {
    bigint id PK
    bigint user_id
    varchar username
    varchar action
    varchar entity
    varchar entity_id
    text detail
    varchar ip
    datetime created_at
  }
  organizations ||--o{ branches : "organization_id"
  branches ||--o{ departments : "branch_id"
  employees ||--o{ departments : "manager_employee_id"
  departments ||--o{ departments : "parent_id"
  departments ||--o{ employees : "department_id"
  roles ||--o{ role_permissions : "role"
  employees ||--o{ users : "employee_id"
  roles ||--o{ users : "role"
```

เชื่อมไปโมดูลอื่น:

- `employees.shift_id` → `shifts` (เวลาทำงานและ HR)

## กล้อง AI และอุปกรณ์

```mermaid
erDiagram
  cameras {
    bigint id PK
    varchar name
    bigint branch_id FK
    varchar floor
    varchar location
    varchar brand
    varchar rtsp_url
    varchar onvif_host
    varchar stream_name UK
    tinyint ai_enabled
    double ai_fps
  }
  workstations {
    bigint id PK
    bigint camera_id FK
    varchar name
    bigint employee_id FK
    text roi
    double screen_yaw_deg
    double yaw_tolerance_deg
    datetime created_at
  }
  devices {
    bigint id PK
    varchar name
    varchar kind
    varchar serial_no UK
    char api_key_hash UK
    bigint branch_id FK
    tinyint is_enabled
    datetime last_seen_at
    datetime created_at
    varchar location_role
  }
  detection_zones {
    bigint id PK
    bigint camera_id FK
    varchar name
    varchar kind
    text roi
    int capacity
    datetime created_at
  }
  zone_occupancy {
    bigint zone_id PK
    datetime bucket_start PK
    int max_people
    double people_sum
    int samples
    int entries
  }
  activity_events {
    bigint id PK
    bigint workstation_id
    bigint camera_id
    bigint employee_id
    varchar state
    datetime started_at
    datetime ended_at
    int duration_sec
    double confidence
  }
  data_corrections {
    bigint id PK
    bigint activity_event_id FK
    bigint employee_id
    varchar reason
    varchar requested_state
    varchar status
    bigint reviewer_user_id
    varchar review_note
    datetime created_at
    datetime reviewed_at
  }
  notifications {
    bigint id PK
    bigint employee_id
    varchar kind
    varchar title
    varchar body
    tinyint is_read
    datetime created_at
  }
  activity_events ||--o{ data_corrections : "activity_event_id"
  cameras ||--o{ detection_zones : "camera_id"
  cameras ||--o{ workstations : "camera_id"
  detection_zones ||--o{ zone_occupancy : "zone_id"
```

เชื่อมไปโมดูลอื่น:

- `cameras.branch_id` → `branches` (องค์กรและผู้ใช้)
- `devices.branch_id` → `branches` (องค์กรและผู้ใช้)
- `workstations.employee_id` → `employees` (องค์กรและผู้ใช้)

## เวลาทำงานและ HR

```mermaid
erDiagram
  attendance_logs {
    bigint id PK
    bigint employee_id
    varchar attendance_ref
    bigint device_id
    datetime scan_time
    varchar direction
    varchar method
    varchar raw
    datetime created_at
  }
  daily_summaries {
    bigint employee_id PK
    date work_date PK
    datetime first_scan_at
    datetime last_scan_at
    datetime first_desk_at
    int scan_to_desk_sec
    int focused_sec
    int present_sec
    int phone_sec
    int looking_away_sec
  }
  work_sessions {
    bigint employee_id PK
    date work_date PK
    varchar day_type
    varchar status
    bigint shift_id
    datetime scheduled_start
    datetime scheduled_end
    datetime office_entry_at
    datetime check_in
    datetime check_out
  }
  shifts {
    bigint id PK
    varchar name
    time start_time
    time end_time
    time break_start
    time break_end
    varchar work_days
    int grace_min
    tinyint is_default
  }
  shift_assignments {
    bigint id PK
    bigint employee_id FK
    date work_date
    bigint shift_id FK
    varchar note
  }
  holidays {
    bigint id PK
    date holiday_date
    varchar name
    bigint branch_id FK
  }
  leave_types {
    bigint id PK
    varchar code UK
    varchar name
    double annual_quota_days
    tinyint paid
    tinyint is_active
  }
  requests {
    bigint id PK
    bigint employee_id FK
    varchar kind
    bigint leave_type_id FK
    datetime start_at
    datetime end_at
    double days
    double hours
    varchar location
    varchar reason
    varchar status
  }
  leave_types ||--o{ requests : "leave_type_id"
  shifts ||--o{ shift_assignments : "shift_id"
```

เชื่อมไปโมดูลอื่น:

- `holidays.branch_id` → `branches` (องค์กรและผู้ใช้)
- `requests.employee_id` → `employees` (องค์กรและผู้ใช้)
- `shift_assignments.employee_id` → `employees` (องค์กรและผู้ใช้)

## งาน KPI และรายงาน

```mermaid
erDiagram
  tasks {
    bigint id PK
    varchar title
    text description
    varchar project
    bigint assignee_employee_id FK
    bigint customer_id FK
    varchar priority
    varchar status
    date due_date
    datetime completed_at
    tinyint quality_score
    bigint project_id FK
  }
  projects {
    bigint id PK
    varchar code
    varchar name
    bigint customer_id FK
    bigint owner_employee_id FK
    varchar status
    date start_date
    date due_date
    double budget
    text description
    datetime created_at
  }
  kpi_metrics {
    bigint id PK
    varchar code UK
    varchar name
    varchar description
    varchar source
    double weight
    varchar position
    tinyint is_active
  }
  kpi_records {
    bigint id PK
    bigint employee_id FK
    bigint metric_id FK
    date period_start
    date period_end
    double score
    varchar note
    bigint reviewer_user_id
    datetime created_at
  }
  reports {
    bigint id PK
    varchar kind
    bigint employee_id
    date period_start
    date period_end
    varchar doc_no
    bigint generated_by
    datetime created_at
  }
  documents {
    bigint id PK
    varchar title
    varchar category
    varchar entity
    bigint entity_id
    varchar file_name
    varchar stored_name
    varchar content_type
    bigint size_bytes
  }
  kpi_metrics ||--o{ kpi_records : "metric_id"
  projects ||--o{ tasks : "project_id"
```

เชื่อมไปโมดูลอื่น:

- `kpi_records.employee_id` → `employees` (องค์กรและผู้ใช้)
- `projects.customer_id` → `customers` (CRM และบริการ)
- `projects.owner_employee_id` → `employees` (องค์กรและผู้ใช้)
- `tasks.assignee_employee_id` → `employees` (องค์กรและผู้ใช้)
- `tasks.customer_id` → `customers` (CRM และบริการ)

## CRM และบริการ

```mermaid
erDiagram
  customers {
    bigint id PK
    varchar name
    varchar company
    varchar email
    varchar phone
    varchar tax_id
    varchar branch_code
    varchar address
    int credit_days
    bigint owner_employee_id FK
  }
  contacts {
    bigint id PK
    bigint customer_id FK
    varchar name
    varchar position
    varchar email
    varchar phone
    tinyint is_primary
    varchar notes
    datetime created_at
  }
  deals {
    bigint id PK
    varchar title
    bigint customer_id FK
    bigint owner_employee_id FK
    varchar stage
    double value
    int probability
    date expected_close
    datetime closed_at
    varchar notes
    datetime created_at
  }
  tickets {
    bigint id PK
    varchar subject
    text description
    bigint customer_id FK
    bigint contact_id FK
    bigint assignee_employee_id FK
    varchar priority
    varchar status
    datetime due_at
    datetime resolved_at
    tinyint satisfaction
    datetime created_at
  }
  webhooks {
    bigint id PK
    varchar name
    varchar url
    varchar events
    varchar secret
    tinyint is_active
    int last_status
    varchar last_error
    datetime last_sent_at
  }
  customers ||--o{ contacts : "customer_id"
  customers ||--o{ deals : "customer_id"
  contacts ||--o{ tickets : "contact_id"
  customers ||--o{ tickets : "customer_id"
```

เชื่อมไปโมดูลอื่น:

- `customers.owner_employee_id` → `employees` (องค์กรและผู้ใช้)
- `deals.owner_employee_id` → `employees` (องค์กรและผู้ใช้)
- `tickets.assignee_employee_id` → `employees` (องค์กรและผู้ใช้)

## ERP: เงินเดือนและเบิกจ่าย

```mermaid
erDiagram
  employee_pay {
    bigint employee_id PK
    varchar pay_type
    decimal base_salary
    decimal daily_rate
    decimal fixed_allowance
    varchar tax_id
    varchar bank_name
    varchar bank_account
    tinyint sso_enabled
  }
  payroll_runs {
    bigint id PK
    varchar run_no UK
    int period_year
    int period_month
    date period_start
    date period_end
    date pay_date
    varchar status
    int employees
    decimal total_earnings
  }
  payslips {
    bigint id PK
    bigint run_id FK
    bigint employee_id FK
    varchar pay_type
    decimal base_salary
    int work_days
    int absent_days
    int unpaid_leave_days
    int late_min
    int ot_min
    int holiday_min
  }
  payslip_items {
    bigint id PK
    bigint payslip_id FK
    varchar kind
    varchar code
    varchar label
    decimal amount
    tinyint taxable
    tinyint manual
    int sort
  }
  expense_categories {
    bigint id PK
    varchar name UK
    bigint account_id FK
    tinyint is_active
  }
  expense_claims {
    bigint id PK
    varchar claim_no UK
    bigint employee_id FK
    varchar title
    date claim_date
    decimal total
    varchar status
    varchar pay_method
    bigint approver_user_id
    varchar approver_note
    datetime decided_at
  }
  expense_items {
    bigint id PK
    bigint claim_id FK
    date expense_date
    bigint category_id FK
    varchar description
    decimal amount
    decimal vat_amount
    bigint document_id FK
  }
  expense_categories ||--o{ expense_items : "category_id"
  expense_claims ||--o{ expense_items : "claim_id"
  payroll_runs ||--o{ payslips : "run_id"
  payslips ||--o{ payslip_items : "payslip_id"
```

เชื่อมไปโมดูลอื่น:

- `employee_pay.employee_id` → `employees` (องค์กรและผู้ใช้)
- `expense_categories.account_id` → `accounts` (ERP: บัญชีและทรัพย์สิน)
- `expense_claims.employee_id` → `employees` (องค์กรและผู้ใช้)
- `expense_items.document_id` → `documents` (งาน KPI และรายงาน)
- `payslips.employee_id` → `employees` (องค์กรและผู้ใช้)

## ERP: ขายและเอกสาร

```mermaid
erDiagram
  products {
    bigint id PK
    varchar sku UK
    varchar name
    varchar kind
    varchar unit
    decimal sale_price
    decimal purchase_price
    decimal avg_cost
    tinyint vat_applicable
    tinyint track_stock
    bigint revenue_account_id FK
    bigint expense_account_id FK
  }
  sales_docs {
    bigint id PK
    varchar doc_type
    varchar doc_no UK
    date doc_date
    date due_date
    bigint customer_id FK
    varchar customer_name
    varchar customer_tax_id
    varchar customer_branch
    varchar customer_address
    bigint reference_id
    bigint warehouse_id FK
  }
  sales_doc_lines {
    bigint id PK
    bigint doc_id FK
    bigint product_id FK
    varchar description
    decimal qty
    varchar unit
    decimal unit_price
    decimal discount_pct
    tinyint vat_exempt
    decimal amount
    int sort
  }
  sales_payments {
    bigint id PK
    bigint invoice_id FK
    bigint receipt_id
    date pay_date
    decimal amount
    decimal wht_amount
    varchar method
    bigint account_id FK
    varchar reference
    varchar status
    bigint journal_entry_id
  }
  doc_sequences {
    varchar doc_type PK
    int year PK
    int last_no
  }
  sales_docs ||--o{ sales_doc_lines : "doc_id"
  products ||--o{ sales_doc_lines : "product_id"
  sales_docs ||--o{ sales_payments : "invoice_id"
```

เชื่อมไปโมดูลอื่น:

- `products.expense_account_id` → `accounts` (ERP: บัญชีและทรัพย์สิน)
- `products.revenue_account_id` → `accounts` (ERP: บัญชีและทรัพย์สิน)
- `sales_docs.customer_id` → `customers` (CRM และบริการ)
- `sales_docs.warehouse_id` → `warehouses` (ERP: จัดซื้อและคลัง)
- `sales_payments.account_id` → `accounts` (ERP: บัญชีและทรัพย์สิน)

## ERP: จัดซื้อและคลัง

```mermaid
erDiagram
  suppliers {
    bigint id PK
    varchar code UK
    varchar name
    varchar kind
    varchar tax_id
    varchar branch_code
    varchar address
    varchar contact_name
    varchar phone
    varchar email
  }
  warehouses {
    bigint id PK
    varchar code UK
    varchar name
    bigint branch_id FK
    tinyint is_default
    tinyint is_active
  }
  purchase_orders {
    bigint id PK
    varchar po_no UK
    date po_date
    date expected_date
    bigint supplier_id FK
    bigint warehouse_id FK
    varchar vat_mode
    decimal vat_rate
    decimal subtotal
    decimal discount
    decimal vat_amount
    decimal total
  }
  purchase_order_lines {
    bigint id PK
    bigint po_id FK
    bigint product_id FK
    varchar description
    decimal qty
    varchar unit
    decimal unit_price
    decimal discount_pct
    tinyint vat_exempt
    decimal amount
    decimal received_qty
  }
  purchase_bills {
    bigint id PK
    varchar bill_no UK
    varchar supplier_invoice_no
    date bill_date
    date due_date
    bigint supplier_id FK
    bigint po_id FK
    bigint warehouse_id FK
    varchar vat_mode
    decimal vat_rate
    decimal subtotal
    decimal discount
    decimal vat_base
  }
  purchase_bill_lines {
    bigint id PK
    bigint bill_id FK
    bigint po_line_id
    bigint product_id FK
    varchar description
    decimal qty
    varchar unit
    decimal unit_price
    decimal discount_pct
    tinyint vat_exempt
    decimal amount
    bigint account_id FK
  }
  purchase_payments {
    bigint id PK
    bigint bill_id FK
    varchar voucher_no UK
    varchar wht_cert_no
    date pay_date
    decimal amount
    decimal wht_amount
    decimal wht_rate
    varchar wht_income_type
    varchar method
    bigint account_id FK
    varchar reference
  }
  stock_moves {
    bigint id PK
    date move_date
    bigint product_id FK
    bigint warehouse_id FK
    decimal qty
    decimal unit_cost
    decimal value
    varchar source
    bigint source_id
    varchar note
    bigint created_by
  }
  stock_balances {
    bigint product_id PK
    bigint warehouse_id PK
    decimal qty
  }
  purchase_orders ||--o{ purchase_bills : "po_id"
  suppliers ||--o{ purchase_bills : "supplier_id"
  warehouses ||--o{ purchase_bills : "warehouse_id"
  purchase_bills ||--o{ purchase_bill_lines : "bill_id"
  suppliers ||--o{ purchase_orders : "supplier_id"
  warehouses ||--o{ purchase_orders : "warehouse_id"
  purchase_orders ||--o{ purchase_order_lines : "po_id"
  purchase_bills ||--o{ purchase_payments : "bill_id"
  warehouses ||--o{ stock_balances : "warehouse_id"
  warehouses ||--o{ stock_moves : "warehouse_id"
```

เชื่อมไปโมดูลอื่น:

- `purchase_bill_lines.account_id` → `accounts` (ERP: บัญชีและทรัพย์สิน)
- `purchase_bill_lines.product_id` → `products` (ERP: ขายและเอกสาร)
- `purchase_order_lines.product_id` → `products` (ERP: ขายและเอกสาร)
- `purchase_payments.account_id` → `accounts` (ERP: บัญชีและทรัพย์สิน)
- `stock_balances.product_id` → `products` (ERP: ขายและเอกสาร)
- `stock_moves.product_id` → `products` (ERP: ขายและเอกสาร)
- `warehouses.branch_id` → `branches` (องค์กรและผู้ใช้)

## ERP: บัญชีและทรัพย์สิน

```mermaid
erDiagram
  accounts {
    bigint id PK
    varchar code UK
    varchar name
    varchar type
    varchar system_role UK
    tinyint is_active
    datetime created_at
  }
  journal_entries {
    bigint id PK
    varchar entry_no UK
    date entry_date
    varchar memo
    varchar source
    bigint source_id
    varchar status
    bigint created_by
    datetime created_at
  }
  journal_lines {
    bigint id PK
    bigint entry_id FK
    bigint account_id FK
    decimal debit
    decimal credit
    varchar memo
    varchar party_type
    bigint party_id
  }
  fixed_assets {
    bigint id PK
    varchar asset_no UK
    varchar name
    varchar category
    date purchase_date
    decimal cost
    decimal salvage_value
    int useful_life_months
    decimal opening_accumulated
    date depr_start
    bigint asset_account_id FK
    bigint accum_account_id FK
    bigint expense_account_id FK
    bigint custodian_employee_id FK
  }
  asset_depreciation {
    bigint asset_id PK
    date period PK
    decimal amount
    bigint journal_entry_id
  }
  fixed_assets ||--o{ asset_depreciation : "asset_id"
  accounts ||--o{ fixed_assets : "accum_account_id"
  accounts ||--o{ fixed_assets : "asset_account_id"
  accounts ||--o{ fixed_assets : "expense_account_id"
  accounts ||--o{ journal_lines : "account_id"
  journal_entries ||--o{ journal_lines : "entry_id"
```

เชื่อมไปโมดูลอื่น:

- `fixed_assets.custodian_employee_id` → `employees` (องค์กรและผู้ใช้)
