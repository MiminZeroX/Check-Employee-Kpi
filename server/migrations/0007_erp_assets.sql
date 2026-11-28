-- ERP part 4: fixed asset register with monthly straight-line depreciation and disposal.

CREATE TABLE fixed_assets (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  asset_no VARCHAR(30) NOT NULL,
  name VARCHAR(200) NOT NULL,
  category VARCHAR(100) NULL,
  purchase_date DATE NOT NULL,
  cost DECIMAL(15,2) NOT NULL,
  salvage_value DECIMAL(15,2) NOT NULL DEFAULT 1,
  useful_life_months INT NOT NULL,
  -- depreciation already accumulated before the asset was entered here
  opening_accumulated DECIMAL(15,2) NOT NULL DEFAULT 0,
  -- first day depreciated by this system (purchase date, or the month the register starts for older assets)
  depr_start DATE NOT NULL,
  asset_account_id BIGINT NOT NULL,
  accum_account_id BIGINT NOT NULL,
  expense_account_id BIGINT NOT NULL,
  location VARCHAR(200) NULL,
  custodian_employee_id BIGINT NULL,
  serial_no VARCHAR(100) NULL,
  -- active | disposed
  status VARCHAR(10) NOT NULL DEFAULT 'active',
  disposed_date DATE NULL,
  disposal_amount DECIMAL(15,2) NULL,
  purchase_entry_id BIGINT NULL,
  disposal_entry_id BIGINT NULL,
  note VARCHAR(500) NULL,
  created_by BIGINT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_fa_no (asset_no),
  CONSTRAINT fk_fa_asset_acc FOREIGN KEY (asset_account_id) REFERENCES accounts(id),
  CONSTRAINT fk_fa_accum_acc FOREIGN KEY (accum_account_id) REFERENCES accounts(id),
  CONSTRAINT fk_fa_exp_acc FOREIGN KEY (expense_account_id) REFERENCES accounts(id),
  CONSTRAINT fk_fa_custodian FOREIGN KEY (custodian_employee_id) REFERENCES employees(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- one row per asset per month that has been depreciated
CREATE TABLE asset_depreciation (
  asset_id BIGINT NOT NULL,
  period DATE NOT NULL,
  amount DECIMAL(15,2) NOT NULL,
  journal_entry_id BIGINT NOT NULL,
  PRIMARY KEY (asset_id, period),
  KEY idx_ad_je (journal_entry_id),
  CONSTRAINT fk_ad_asset FOREIGN KEY (asset_id) REFERENCES fixed_assets(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

INSERT INTO accounts (code, name, type, system_role) VALUES ('4910', 'กำไรจากการจำหน่ายทรัพย์สิน', 'revenue', 'asset_gain');
