-- ERP part 3: suppliers, purchase orders, goods receipt / supplier bills (stock in, input VAT,
-- accounts payable) and supplier payments with withholding tax (ภ.ง.ด.3 / ภ.ง.ด.53).

CREATE TABLE suppliers (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  code VARCHAR(30) NULL,
  name VARCHAR(200) NOT NULL,
  -- company -> ภ.ง.ด.53, individual -> ภ.ง.ด.3
  kind VARCHAR(12) NOT NULL DEFAULT 'company',
  tax_id VARCHAR(20) NULL,
  branch_code VARCHAR(20) NULL,
  address VARCHAR(500) NULL,
  contact_name VARCHAR(120) NULL,
  phone VARCHAR(40) NULL,
  email VARCHAR(200) NULL,
  credit_days INT NOT NULL DEFAULT 30,
  -- default withholding rate when we pay this supplier (services 3%, rent 5%, transport 1% ...)
  wht_rate DECIMAL(5,2) NOT NULL DEFAULT 0,
  bank_name VARCHAR(100) NULL,
  bank_account VARCHAR(40) NULL,
  is_active TINYINT(1) NOT NULL DEFAULT 1,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_sup_code (code)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE purchase_orders (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  po_no VARCHAR(30) NOT NULL,
  po_date DATE NOT NULL,
  expected_date DATE NULL,
  supplier_id BIGINT NOT NULL,
  warehouse_id BIGINT NULL,
  vat_mode VARCHAR(10) NOT NULL DEFAULT 'exclusive',
  vat_rate DECIMAL(5,2) NOT NULL DEFAULT 7,
  subtotal DECIMAL(15,2) NOT NULL DEFAULT 0,
  discount DECIMAL(15,2) NOT NULL DEFAULT 0,
  vat_amount DECIMAL(15,2) NOT NULL DEFAULT 0,
  total DECIMAL(15,2) NOT NULL DEFAULT 0,
  -- draft | approved | partial | received | closed | cancelled
  status VARCHAR(12) NOT NULL DEFAULT 'draft',
  note VARCHAR(1000) NULL,
  approved_by BIGINT NULL,
  approved_at DATETIME NULL,
  created_by BIGINT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_po_no (po_no),
  KEY idx_po_sup (supplier_id, status),
  CONSTRAINT fk_po_sup FOREIGN KEY (supplier_id) REFERENCES suppliers(id),
  CONSTRAINT fk_po_wh FOREIGN KEY (warehouse_id) REFERENCES warehouses(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE purchase_order_lines (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  po_id BIGINT NOT NULL,
  product_id BIGINT NULL,
  description VARCHAR(500) NOT NULL,
  qty DECIMAL(15,3) NOT NULL,
  unit VARCHAR(20) NULL,
  unit_price DECIMAL(15,4) NOT NULL,
  discount_pct DECIMAL(5,2) NOT NULL DEFAULT 0,
  vat_exempt TINYINT(1) NOT NULL DEFAULT 0,
  amount DECIMAL(15,2) NOT NULL,
  received_qty DECIMAL(15,3) NOT NULL DEFAULT 0,
  sort INT NOT NULL DEFAULT 0,
  KEY idx_pol_po (po_id),
  CONSTRAINT fk_pol_po FOREIGN KEY (po_id) REFERENCES purchase_orders(id) ON DELETE CASCADE,
  CONSTRAINT fk_pol_prod FOREIGN KEY (product_id) REFERENCES products(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- goods / services received together with the supplier's (tax) invoice
CREATE TABLE purchase_bills (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  bill_no VARCHAR(30) NOT NULL,
  supplier_invoice_no VARCHAR(60) NULL,
  bill_date DATE NOT NULL,
  due_date DATE NULL,
  supplier_id BIGINT NOT NULL,
  po_id BIGINT NULL,
  warehouse_id BIGINT NULL,
  vat_mode VARCHAR(10) NOT NULL DEFAULT 'exclusive',
  vat_rate DECIMAL(5,2) NOT NULL DEFAULT 7,
  subtotal DECIMAL(15,2) NOT NULL DEFAULT 0,
  discount DECIMAL(15,2) NOT NULL DEFAULT 0,
  vat_base DECIMAL(15,2) NOT NULL DEFAULT 0,
  exempt_amount DECIMAL(15,2) NOT NULL DEFAULT 0,
  vat_amount DECIMAL(15,2) NOT NULL DEFAULT 0,
  total DECIMAL(15,2) NOT NULL DEFAULT 0,
  wht_rate DECIMAL(5,2) NOT NULL DEFAULT 0,
  wht_amount DECIMAL(15,2) NOT NULL DEFAULT 0,
  paid_amount DECIMAL(15,2) NOT NULL DEFAULT 0,
  -- posted | partial | paid | void
  status VARCHAR(10) NOT NULL DEFAULT 'posted',
  note VARCHAR(1000) NULL,
  journal_entry_id BIGINT NULL,
  created_by BIGINT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_pb_no (bill_no),
  KEY idx_pb_sup (supplier_id, status),
  KEY idx_pb_po (po_id),
  CONSTRAINT fk_pb_sup FOREIGN KEY (supplier_id) REFERENCES suppliers(id),
  CONSTRAINT fk_pb_po FOREIGN KEY (po_id) REFERENCES purchase_orders(id),
  CONSTRAINT fk_pb_wh FOREIGN KEY (warehouse_id) REFERENCES warehouses(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE purchase_bill_lines (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  bill_id BIGINT NOT NULL,
  po_line_id BIGINT NULL,
  product_id BIGINT NULL,
  description VARCHAR(500) NOT NULL,
  qty DECIMAL(15,3) NOT NULL,
  unit VARCHAR(20) NULL,
  unit_price DECIMAL(15,4) NOT NULL,
  discount_pct DECIMAL(5,2) NOT NULL DEFAULT 0,
  vat_exempt TINYINT(1) NOT NULL DEFAULT 0,
  amount DECIMAL(15,2) NOT NULL,
  -- expense / asset account for lines that are not stock
  account_id BIGINT NULL,
  sort INT NOT NULL DEFAULT 0,
  KEY idx_pbl_bill (bill_id),
  CONSTRAINT fk_pbl_bill FOREIGN KEY (bill_id) REFERENCES purchase_bills(id) ON DELETE CASCADE,
  CONSTRAINT fk_pbl_prod FOREIGN KEY (product_id) REFERENCES products(id),
  CONSTRAINT fk_pbl_acc FOREIGN KEY (account_id) REFERENCES accounts(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE purchase_payments (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  bill_id BIGINT NOT NULL,
  voucher_no VARCHAR(30) NOT NULL,
  -- withholding tax certificate (50 ทวิ) number when tax was withheld
  wht_cert_no VARCHAR(30) NULL,
  pay_date DATE NOT NULL,
  amount DECIMAL(15,2) NOT NULL,
  wht_amount DECIMAL(15,2) NOT NULL DEFAULT 0,
  wht_rate DECIMAL(5,2) NOT NULL DEFAULT 0,
  -- income type printed on the certificate (40(8) services, 40(5) rent, ...)
  wht_income_type VARCHAR(100) NULL,
  method VARCHAR(10) NOT NULL DEFAULT 'transfer',
  account_id BIGINT NOT NULL,
  reference VARCHAR(100) NULL,
  status VARCHAR(10) NOT NULL DEFAULT 'active',
  journal_entry_id BIGINT NULL,
  created_by BIGINT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE KEY uq_pp_no (voucher_no),
  KEY idx_pp_bill (bill_id),
  KEY idx_pp_date (pay_date),
  CONSTRAINT fk_pp_bill FOREIGN KEY (bill_id) REFERENCES purchase_bills(id),
  CONSTRAINT fk_pp_acc FOREIGN KEY (account_id) REFERENCES accounts(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
