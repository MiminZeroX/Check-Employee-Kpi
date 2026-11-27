-- ERP part 1: accounting core (chart of accounts, double-entry journal, document numbers),
-- payroll (Thai social security / withholding tax / provident fund) and expense claims.

-- ---------------------------------------------------------------- accounting core
CREATE TABLE accounts (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  code VARCHAR(20) NOT NULL,
  name VARCHAR(200) NOT NULL,
  -- asset | liability | equity | revenue | expense
  type VARCHAR(20) NOT NULL,
  -- fixed meaning used for automatic postings (cash, ar, vat_output, salary_expense, ...)
  system_role VARCHAR(40) NULL,
  is_active TINYINT(1) NOT NULL DEFAULT 1,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_acc_code (code),
  UNIQUE KEY uq_acc_role (system_role)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE journal_entries (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  entry_no VARCHAR(30) NOT NULL,
  entry_date DATE NOT NULL,
  memo VARCHAR(300) NULL,
  -- manual | payroll | expense_claim | expense_payment | sales_invoice | sales_payment | purchase_receipt
  -- | purchase_payment | stock_adjust | depreciation | asset_disposal
  source VARCHAR(30) NOT NULL DEFAULT 'manual',
  source_id BIGINT NULL,
  -- posted | void
  status VARCHAR(10) NOT NULL DEFAULT 'posted',
  created_by BIGINT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_je_no (entry_no),
  KEY idx_je_date (entry_date),
  KEY idx_je_source (source, source_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE journal_lines (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  entry_id BIGINT NOT NULL,
  account_id BIGINT NOT NULL,
  debit DECIMAL(15,2) NOT NULL DEFAULT 0,
  credit DECIMAL(15,2) NOT NULL DEFAULT 0,
  memo VARCHAR(300) NULL,
  -- employee | customer | supplier (sub-ledger)
  party_type VARCHAR(20) NULL,
  party_id BIGINT NULL,
  KEY idx_jl_entry (entry_id),
  KEY idx_jl_account (account_id),
  KEY idx_jl_party (party_type, party_id),
  CONSTRAINT fk_jl_entry FOREIGN KEY (entry_id) REFERENCES journal_entries(id) ON DELETE CASCADE,
  CONSTRAINT fk_jl_account FOREIGN KEY (account_id) REFERENCES accounts(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- running numbers per document type and (Buddhist) year: JV2569-00001, PR2569-00001, ...
CREATE TABLE doc_sequences (
  doc_type VARCHAR(20) NOT NULL,
  year INT NOT NULL,
  last_no INT NOT NULL DEFAULT 0,
  PRIMARY KEY (doc_type, year)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- Chart of accounts for a Thai SME (edit names / add accounts in the app)
INSERT INTO accounts (code, name, type, system_role) VALUES
 ('1100', 'เงินสด', 'asset', 'cash'),
 ('1110', 'เงินฝากธนาคาร', 'asset', 'bank'),
 ('1200', 'ลูกหนี้การค้า', 'asset', 'ar'),
 ('1210', 'ภาษีเงินได้ถูกหัก ณ ที่จ่าย', 'asset', 'wht_receivable'),
 ('1220', 'ลูกหนี้อื่น - พนักงาน', 'asset', 'employee_receivable'),
 ('1300', 'สินค้าคงเหลือ', 'asset', 'inventory'),
 ('1400', 'ภาษีซื้อ', 'asset', 'vat_input'),
 ('1500', 'อุปกรณ์และเครื่องใช้สำนักงาน', 'asset', 'fixed_asset'),
 ('1510', 'ค่าเสื่อมราคาสะสม', 'asset', 'accumulated_depreciation'),
 ('2100', 'เจ้าหนี้การค้า', 'liability', 'ap'),
 ('2200', 'ภาษีขาย', 'liability', 'vat_output'),
 ('2300', 'ภาษีหัก ณ ที่จ่ายค้างนำส่ง', 'liability', 'wht_payable'),
 ('2310', 'เงินประกันสังคมค้างนำส่ง', 'liability', 'sso_payable'),
 ('2320', 'กองทุนสำรองเลี้ยงชีพค้างนำส่ง', 'liability', 'pvd_payable'),
 ('2330', 'เงินเดือนค้างจ่าย', 'liability', 'salary_payable'),
 ('2340', 'ค่าใช้จ่ายค้างจ่าย - พนักงาน', 'liability', 'employee_payable'),
 ('3100', 'ทุนจดทะเบียน', 'equity', 'capital'),
 ('3200', 'กำไร (ขาดทุน) สะสม', 'equity', 'retained_earnings'),
 ('4100', 'รายได้จากการขาย', 'revenue', 'sales'),
 ('4200', 'รายได้จากการให้บริการ', 'revenue', 'service_revenue'),
 ('4900', 'รายได้อื่น', 'revenue', 'other_income'),
 ('5100', 'ต้นทุนขาย', 'expense', 'cogs'),
 ('5200', 'เงินเดือนและค่าจ้าง', 'expense', 'salary_expense'),
 ('5210', 'ค่าล่วงเวลา', 'expense', 'ot_expense'),
 ('5220', 'เงินสมทบประกันสังคม', 'expense', 'sso_expense'),
 ('5230', 'เงินสมทบกองทุนสำรองเลี้ยงชีพ', 'expense', 'pvd_expense'),
 ('5300', 'ค่าเดินทาง', 'expense', NULL),
 ('5310', 'ค่ารับรอง', 'expense', NULL),
 ('5320', 'วัสดุสำนักงาน', 'expense', NULL),
 ('5330', 'ค่าสาธารณูปโภค', 'expense', NULL),
 ('5340', 'ค่าโทรศัพท์และอินเทอร์เน็ต', 'expense', NULL),
 ('5350', 'ค่าอบรมและสัมมนา', 'expense', NULL),
 ('5390', 'ค่าใช้จ่ายเบ็ดเตล็ด', 'expense', 'expense_default'),
 ('5400', 'ค่าเสื่อมราคา', 'expense', 'depreciation_expense'),
 ('5500', 'ผลต่างจากการตรวจนับสินค้า', 'expense', 'stock_adjustment'),
 ('5600', 'ขาดทุนจากการจำหน่ายทรัพย์สิน', 'expense', 'asset_loss');

-- ---------------------------------------------------------------- payroll
CREATE TABLE employee_pay (
  employee_id BIGINT PRIMARY KEY,
  -- monthly | daily
  pay_type VARCHAR(10) NOT NULL DEFAULT 'monthly',
  base_salary DECIMAL(15,2) NOT NULL DEFAULT 0,
  daily_rate DECIMAL(15,2) NOT NULL DEFAULT 0,
  -- taxable fixed monthly allowance (position, transport, ...)
  fixed_allowance DECIMAL(15,2) NOT NULL DEFAULT 0,
  -- 13-digit national ID / tax ID (for ภ.ง.ด.1 and สปส.1-10)
  tax_id VARCHAR(20) NULL,
  bank_name VARCHAR(100) NULL,
  bank_account VARCHAR(40) NULL,
  sso_enabled TINYINT(1) NOT NULL DEFAULT 1,
  -- provident fund contribution (% of salary)
  pvd_rate DECIMAL(5,2) NOT NULL DEFAULT 0,
  pvd_employer_rate DECIMAL(5,2) NOT NULL DEFAULT 0,
  -- yearly allowances other than the personal allowance and social security (spouse, children, insurance, ...)
  tax_allowance_annual DECIMAL(15,2) NOT NULL DEFAULT 0,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
  CONSTRAINT fk_epay_emp FOREIGN KEY (employee_id) REFERENCES employees(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE payroll_runs (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  run_no VARCHAR(30) NOT NULL,
  period_year INT NOT NULL,
  period_month INT NOT NULL,
  period_start DATE NOT NULL,
  period_end DATE NOT NULL,
  pay_date DATE NOT NULL,
  -- draft | approved | paid | void
  status VARCHAR(10) NOT NULL DEFAULT 'draft',
  employees INT NOT NULL DEFAULT 0,
  total_earnings DECIMAL(15,2) NOT NULL DEFAULT 0,
  total_deductions DECIMAL(15,2) NOT NULL DEFAULT 0,
  total_net DECIMAL(15,2) NOT NULL DEFAULT 0,
  total_employer DECIMAL(15,2) NOT NULL DEFAULT 0,
  note VARCHAR(300) NULL,
  created_by BIGINT NULL,
  approved_by BIGINT NULL,
  approved_at DATETIME NULL,
  paid_at DATETIME NULL,
  journal_entry_id BIGINT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_pr_no (run_no),
  KEY idx_pr_period (period_year, period_month)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE payslips (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  run_id BIGINT NOT NULL,
  employee_id BIGINT NOT NULL,
  pay_type VARCHAR(10) NOT NULL,
  base_salary DECIMAL(15,2) NOT NULL DEFAULT 0,
  work_days INT NOT NULL DEFAULT 0,
  absent_days INT NOT NULL DEFAULT 0,
  unpaid_leave_days INT NOT NULL DEFAULT 0,
  late_min INT NOT NULL DEFAULT 0,
  ot_min INT NOT NULL DEFAULT 0,
  holiday_min INT NOT NULL DEFAULT 0,
  total_earnings DECIMAL(15,2) NOT NULL DEFAULT 0,
  total_deductions DECIMAL(15,2) NOT NULL DEFAULT 0,
  net_pay DECIMAL(15,2) NOT NULL DEFAULT 0,
  taxable_income DECIMAL(15,2) NOT NULL DEFAULT 0,
  sso_base DECIMAL(15,2) NOT NULL DEFAULT 0,
  sso_employee DECIMAL(15,2) NOT NULL DEFAULT 0,
  sso_employer DECIMAL(15,2) NOT NULL DEFAULT 0,
  wht DECIMAL(15,2) NOT NULL DEFAULT 0,
  pvd_employee DECIMAL(15,2) NOT NULL DEFAULT 0,
  pvd_employer DECIMAL(15,2) NOT NULL DEFAULT 0,
  UNIQUE KEY uq_ps_run_emp (run_id, employee_id),
  KEY idx_ps_emp (employee_id),
  CONSTRAINT fk_ps_run FOREIGN KEY (run_id) REFERENCES payroll_runs(id) ON DELETE CASCADE,
  CONSTRAINT fk_ps_emp FOREIGN KEY (employee_id) REFERENCES employees(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE payslip_items (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  payslip_id BIGINT NOT NULL,
  -- earning | deduction
  kind VARCHAR(10) NOT NULL,
  -- salary, allowance, ot, holiday, bonus, reimbursement, other | sso, wht, pvd, unpaid_leave, absence, late, other
  code VARCHAR(20) NOT NULL,
  label VARCHAR(200) NOT NULL,
  amount DECIMAL(15,2) NOT NULL,
  taxable TINYINT(1) NOT NULL DEFAULT 1,
  -- added by a person (kept when the run is recalculated)
  manual TINYINT(1) NOT NULL DEFAULT 0,
  sort INT NOT NULL DEFAULT 0,
  KEY idx_psi_slip (payslip_id),
  CONSTRAINT fk_psi_slip FOREIGN KEY (payslip_id) REFERENCES payslips(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- ---------------------------------------------------------------- expense claims
CREATE TABLE expense_categories (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  name VARCHAR(120) NOT NULL,
  account_id BIGINT NOT NULL,
  is_active TINYINT(1) NOT NULL DEFAULT 1,
  UNIQUE KEY uq_expcat_name (name),
  CONSTRAINT fk_expcat_acc FOREIGN KEY (account_id) REFERENCES accounts(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

INSERT INTO expense_categories (name, account_id)
SELECT 'ค่าเดินทาง / ค่าน้ำมัน', id FROM accounts WHERE code = '5300' UNION ALL
SELECT 'ค่ารับรองลูกค้า', id FROM accounts WHERE code = '5310' UNION ALL
SELECT 'วัสดุสำนักงาน', id FROM accounts WHERE code = '5320' UNION ALL
SELECT 'ค่าโทรศัพท์ / อินเทอร์เน็ต', id FROM accounts WHERE code = '5340' UNION ALL
SELECT 'ค่าอบรม / สัมมนา', id FROM accounts WHERE code = '5350' UNION ALL
SELECT 'อื่น ๆ', id FROM accounts WHERE code = '5390';

CREATE TABLE expense_claims (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  claim_no VARCHAR(30) NOT NULL,
  employee_id BIGINT NOT NULL,
  title VARCHAR(200) NOT NULL,
  claim_date DATE NOT NULL,
  total DECIMAL(15,2) NOT NULL DEFAULT 0,
  -- draft | submitted | approved | rejected | paid
  status VARCHAR(12) NOT NULL DEFAULT 'draft',
  -- cash (paid by finance) | payroll (added to the next payslip)
  pay_method VARCHAR(10) NOT NULL DEFAULT 'payroll',
  approver_user_id BIGINT NULL,
  approver_note VARCHAR(300) NULL,
  decided_at DATETIME NULL,
  paid_at DATETIME NULL,
  payroll_run_id BIGINT NULL,
  journal_entry_id BIGINT NULL,
  payment_entry_id BIGINT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_exp_no (claim_no),
  KEY idx_exp_emp (employee_id, status),
  CONSTRAINT fk_exp_emp FOREIGN KEY (employee_id) REFERENCES employees(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE expense_items (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  claim_id BIGINT NOT NULL,
  expense_date DATE NOT NULL,
  category_id BIGINT NOT NULL,
  description VARCHAR(300) NOT NULL,
  amount DECIMAL(15,2) NOT NULL,
  -- input VAT included in amount (from a full tax invoice)
  vat_amount DECIMAL(15,2) NOT NULL DEFAULT 0,
  document_id BIGINT NULL,
  KEY idx_expi_claim (claim_id),
  CONSTRAINT fk_expi_claim FOREIGN KEY (claim_id) REFERENCES expense_claims(id) ON DELETE CASCADE,
  CONSTRAINT fk_expi_cat FOREIGN KEY (category_id) REFERENCES expense_categories(id),
  CONSTRAINT fk_expi_doc FOREIGN KEY (document_id) REFERENCES documents(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- ---------------------------------------------------------------- settings & permissions
-- Social security: 5% of wages between 1,650 and 17,500 baht (ceiling from 2026; check the
-- current ministerial regulation). Overtime per the Labour Protection Act s.61-63.
INSERT INTO system_settings (setting_key, setting_value) VALUES
 ('payroll_sso_rate', '5'),
 ('payroll_sso_min_base', '1650'),
 ('payroll_sso_max_base', '17500'),
 ('payroll_ot_rate', '1.5'),
 ('payroll_holiday_rate', '1'),
 ('payroll_holiday_ot_rate', '3'),
 ('payroll_deduct_absence', 'true'),
 ('payroll_deduct_late', 'false'),
 ('accounting_lock_date', '""'),
 ('company_tax_id', '""'),
 ('company_address', '""'),
 ('company_branch', '"สำนักงานใหญ่"');

INSERT INTO role_permissions (role, permission) VALUES
 ('hr','payroll.view'),('hr','payroll.manage'),('hr','expenses.approve'),
 ('manager','expenses.approve'),('manager','sales.view'),('manager','purchasing.view');
