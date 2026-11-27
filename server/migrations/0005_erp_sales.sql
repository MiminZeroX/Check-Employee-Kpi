-- ERP part 2: products / services, warehouses and stock, sales documents
-- (quotation -> invoice / tax invoice -> receipt) with VAT and withholding tax.

-- details a tax invoice must show about the buyer (Revenue Code s.86/4)
ALTER TABLE customers
  ADD COLUMN tax_id VARCHAR(20) NULL AFTER phone,
  ADD COLUMN branch_code VARCHAR(20) NULL AFTER tax_id,
  ADD COLUMN address VARCHAR(500) NULL AFTER branch_code,
  ADD COLUMN credit_days INT NOT NULL DEFAULT 30 AFTER address;

CREATE TABLE products (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  sku VARCHAR(40) NOT NULL,
  name VARCHAR(200) NOT NULL,
  -- product | service
  kind VARCHAR(10) NOT NULL DEFAULT 'product',
  unit VARCHAR(20) NOT NULL DEFAULT 'ชิ้น',
  sale_price DECIMAL(15,4) NOT NULL DEFAULT 0,
  purchase_price DECIMAL(15,4) NOT NULL DEFAULT 0,
  -- moving-average cost of the stock on hand
  avg_cost DECIMAL(15,4) NOT NULL DEFAULT 0,
  -- 1 = subject to VAT, 0 = VAT exempt (s.81)
  vat_applicable TINYINT(1) NOT NULL DEFAULT 1,
  track_stock TINYINT(1) NOT NULL DEFAULT 1,
  reorder_point DECIMAL(15,3) NOT NULL DEFAULT 0,
  revenue_account_id BIGINT NULL,
  expense_account_id BIGINT NULL,
  is_active TINYINT(1) NOT NULL DEFAULT 1,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_prod_sku (sku),
  CONSTRAINT fk_prod_rev FOREIGN KEY (revenue_account_id) REFERENCES accounts(id),
  CONSTRAINT fk_prod_exp FOREIGN KEY (expense_account_id) REFERENCES accounts(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE warehouses (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  code VARCHAR(20) NOT NULL,
  name VARCHAR(120) NOT NULL,
  branch_id BIGINT NULL,
  is_default TINYINT(1) NOT NULL DEFAULT 0,
  is_active TINYINT(1) NOT NULL DEFAULT 1,
  UNIQUE KEY uq_wh_code (code),
  CONSTRAINT fk_wh_branch FOREIGN KEY (branch_id) REFERENCES branches(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

INSERT INTO warehouses (code, name, is_default) VALUES ('MAIN', 'คลังหลัก', 1);

CREATE TABLE stock_balances (
  product_id BIGINT NOT NULL,
  warehouse_id BIGINT NOT NULL,
  qty DECIMAL(15,3) NOT NULL DEFAULT 0,
  PRIMARY KEY (product_id, warehouse_id),
  CONSTRAINT fk_sb_prod FOREIGN KEY (product_id) REFERENCES products(id) ON DELETE CASCADE,
  CONSTRAINT fk_sb_wh FOREIGN KEY (warehouse_id) REFERENCES warehouses(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- stock card: + received, - issued
CREATE TABLE stock_moves (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  move_date DATE NOT NULL,
  product_id BIGINT NOT NULL,
  warehouse_id BIGINT NOT NULL,
  qty DECIMAL(15,3) NOT NULL,
  unit_cost DECIMAL(15,4) NOT NULL DEFAULT 0,
  value DECIMAL(15,2) NOT NULL DEFAULT 0,
  -- opening | sales_invoice | sales_void | purchase_receipt | purchase_void | adjustment | transfer
  source VARCHAR(30) NOT NULL,
  source_id BIGINT NULL,
  note VARCHAR(300) NULL,
  created_by BIGINT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_sm_prod (product_id, move_date),
  KEY idx_sm_source (source, source_id),
  CONSTRAINT fk_sm_prod FOREIGN KEY (product_id) REFERENCES products(id),
  CONSTRAINT fk_sm_wh FOREIGN KEY (warehouse_id) REFERENCES warehouses(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE sales_docs (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  -- quotation | invoice | receipt
  doc_type VARCHAR(12) NOT NULL,
  -- assigned when the document is issued (tax invoices must be numbered without gaps)
  doc_no VARCHAR(30) NULL,
  doc_date DATE NOT NULL,
  due_date DATE NULL,
  customer_id BIGINT NOT NULL,
  -- buyer details as printed (kept even if the customer record changes later)
  customer_name VARCHAR(200) NOT NULL,
  customer_tax_id VARCHAR(20) NULL,
  customer_branch VARCHAR(40) NULL,
  customer_address VARCHAR(500) NULL,
  -- invoice <- quotation, receipt <- invoice
  reference_id BIGINT NULL,
  deal_id BIGINT NULL,
  salesperson_employee_id BIGINT NULL,
  warehouse_id BIGINT NULL,
  -- exclusive (price + VAT) | inclusive (price includes VAT) | none
  vat_mode VARCHAR(10) NOT NULL DEFAULT 'exclusive',
  vat_rate DECIMAL(5,2) NOT NULL DEFAULT 7,
  subtotal DECIMAL(15,2) NOT NULL DEFAULT 0,
  discount DECIMAL(15,2) NOT NULL DEFAULT 0,
  vat_base DECIMAL(15,2) NOT NULL DEFAULT 0,
  exempt_amount DECIMAL(15,2) NOT NULL DEFAULT 0,
  vat_amount DECIMAL(15,2) NOT NULL DEFAULT 0,
  total DECIMAL(15,2) NOT NULL DEFAULT 0,
  -- tax the customer withholds when paying (1% transport, 2% advertising, 3% services, 5% rent ...)
  wht_rate DECIMAL(5,2) NOT NULL DEFAULT 0,
  wht_amount DECIMAL(15,2) NOT NULL DEFAULT 0,
  paid_amount DECIMAL(15,2) NOT NULL DEFAULT 0,
  -- quotation: open | accepted | rejected | converted ; invoice: draft | issued | partial | paid | void ; receipt: issued | void
  status VARCHAR(12) NOT NULL,
  note VARCHAR(1000) NULL,
  journal_entry_id BIGINT NULL,
  created_by BIGINT NULL,
  issued_at DATETIME NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_sd_no (doc_no),
  KEY idx_sd_type (doc_type, status, doc_date),
  KEY idx_sd_cust (customer_id),
  KEY idx_sd_ref (reference_id),
  CONSTRAINT fk_sd_cust FOREIGN KEY (customer_id) REFERENCES customers(id),
  CONSTRAINT fk_sd_wh FOREIGN KEY (warehouse_id) REFERENCES warehouses(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE sales_doc_lines (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  doc_id BIGINT NOT NULL,
  product_id BIGINT NULL,
  description VARCHAR(500) NOT NULL,
  qty DECIMAL(15,3) NOT NULL,
  unit VARCHAR(20) NULL,
  unit_price DECIMAL(15,4) NOT NULL,
  discount_pct DECIMAL(5,2) NOT NULL DEFAULT 0,
  vat_exempt TINYINT(1) NOT NULL DEFAULT 0,
  amount DECIMAL(15,2) NOT NULL,
  sort INT NOT NULL DEFAULT 0,
  KEY idx_sdl_doc (doc_id),
  CONSTRAINT fk_sdl_doc FOREIGN KEY (doc_id) REFERENCES sales_docs(id) ON DELETE CASCADE,
  CONSTRAINT fk_sdl_prod FOREIGN KEY (product_id) REFERENCES products(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- money received against an invoice; each payment has its own receipt document
CREATE TABLE sales_payments (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  invoice_id BIGINT NOT NULL,
  receipt_id BIGINT NULL,
  pay_date DATE NOT NULL,
  amount DECIMAL(15,2) NOT NULL,
  wht_amount DECIMAL(15,2) NOT NULL DEFAULT 0,
  -- transfer | cash | cheque | card
  method VARCHAR(10) NOT NULL DEFAULT 'transfer',
  account_id BIGINT NOT NULL,
  reference VARCHAR(100) NULL,
  -- active | void
  status VARCHAR(10) NOT NULL DEFAULT 'active',
  journal_entry_id BIGINT NULL,
  created_by BIGINT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_sp_inv (invoice_id),
  CONSTRAINT fk_sp_inv FOREIGN KEY (invoice_id) REFERENCES sales_docs(id),
  CONSTRAINT fk_sp_acc FOREIGN KEY (account_id) REFERENCES accounts(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

INSERT INTO system_settings (setting_key, setting_value) VALUES
 ('vat_rate', '7'),
 ('vat_registered', 'true'),
 ('sales_terms', '"ยืนราคา 30 วัน · ชำระเงินภายในวันที่ครบกำหนด"');
