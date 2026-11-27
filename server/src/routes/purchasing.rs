//! Purchasing: suppliers, purchase orders (approval), goods receipt with the supplier's
//! tax invoice (stock in at cost, input VAT, accounts payable) and supplier payments
//! with withholding tax: payment voucher + withholding tax certificate (50 ทวิ).

use axum::{
    extract::{Path, Query, State},
    response::Html,
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::{audit, AuthUser},
    erp::{self, fmt_money, r2, Acc, Line, Tx},
    error::{bad, AppError, AppResult},
    routes::{
        accounting::company,
        inventory::{default_warehouse, reverse_moves, stock_in, Move},
        payroll::{th_date, PRINT_CSS},
        sales::{line_amount, totals, LineReq},
    },
    state::AppState,
    util::{html_escape as esc, local_today, setting},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/suppliers", get(list_suppliers).post(create_supplier))
        .route("/suppliers/{id}", put(update_supplier))
        .route("/purchasing/orders", get(list_orders).post(create_order))
        .route("/purchasing/orders/{id}", get(order_detail).put(update_order))
        .route("/purchasing/orders/{id}/approve", post(approve_order))
        .route("/purchasing/orders/{id}/cancel", post(cancel_order))
        .route("/purchasing/orders/{id}/print", get(print_order))
        .route("/purchasing/bills", get(list_bills).post(create_bill))
        .route("/purchasing/bills/{id}", get(bill_detail))
        .route("/purchasing/bills/{id}/void", post(void_bill))
        .route("/purchasing/bills/{id}/payments", post(pay_bill))
        .route("/purchasing/payments/{id}/void", post(void_payment))
        .route("/purchasing/payments/{id}/print", get(print_payment))
        .route("/purchasing/summary", get(summary))
}

fn view(u: &AuthUser) -> AppResult<()> {
    u.need_any(&["purchasing.view", "purchasing.manage", "accounting.view"])
}

// ---------------------------------------------------------------- suppliers

#[derive(Serialize, Deserialize, sqlx::FromRow)]
struct Supplier {
    #[serde(default)]
    id: i64,
    code: Option<String>,
    name: String,
    kind: String,
    tax_id: Option<String>,
    branch_code: Option<String>,
    address: Option<String>,
    contact_name: Option<String>,
    phone: Option<String>,
    email: Option<String>,
    credit_days: i32,
    wht_rate: Decimal,
    bank_name: Option<String>,
    bank_account: Option<String>,
    is_active: bool,
}

const SUP_COLS: &str = "id, code, name, kind, tax_id, branch_code, address, contact_name, phone, email, credit_days, wht_rate, bank_name, bank_account, is_active";

async fn list_suppliers(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Vec<Supplier>>> {
    view(&u)?;
    Ok(Json(sqlx::query_as(&format!("SELECT {SUP_COLS} FROM suppliers ORDER BY name")).fetch_all(&st.db).await?))
}

fn check_supplier(s: &Supplier) -> AppResult<()> {
    if s.name.trim().is_empty() {
        return Err(bad("กรุณาระบุชื่อผู้ขาย"));
    }
    if !["company", "individual"].contains(&s.kind.as_str()) {
        return Err(bad("ประเภทผู้ขายต้องเป็นนิติบุคคลหรือบุคคลธรรมดา"));
    }
    let blank = |v: &Option<String>| v.as_deref().map(str::trim).filter(|x| !x.is_empty()).map(String::from);
    if let Some(t) = blank(&s.tax_id) {
        if !(t.len() == 13 && t.chars().all(|c| c.is_ascii_digit())) {
            return Err(bad("เลขประจำตัวผู้เสียภาษีต้องเป็นตัวเลข 13 หลัก"));
        }
    }
    if !(Decimal::ZERO..=dec!(15)).contains(&s.wht_rate) || !(0..=365).contains(&s.credit_days) {
        return Err(bad("อัตราหัก ณ ที่จ่าย 0-15% และเครดิต 0-365 วัน"));
    }
    Ok(())
}

async fn save_supplier(st: &AppState, id: Option<i64>, s: &Supplier) -> AppResult<i64> {
    check_supplier(s)?;
    let t = |v: &Option<String>| v.as_deref().map(str::trim).filter(|x| !x.is_empty()).map(String::from);
    let sql = match id {
        Some(_) => "UPDATE suppliers SET code=?, name=?, kind=?, tax_id=?, branch_code=?, address=?, contact_name=?, phone=?, email=?, credit_days=?,
                    wht_rate=?, bank_name=?, bank_account=?, is_active=? WHERE id=?",
        None => "INSERT INTO suppliers (code, name, kind, tax_id, branch_code, address, contact_name, phone, email, credit_days, wht_rate, bank_name,
                 bank_account, is_active) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    };
    let mut q = sqlx::query(sql)
        .bind(t(&s.code))
        .bind(s.name.trim())
        .bind(&s.kind)
        .bind(t(&s.tax_id))
        .bind(t(&s.branch_code))
        .bind(t(&s.address))
        .bind(t(&s.contact_name))
        .bind(t(&s.phone))
        .bind(t(&s.email))
        .bind(s.credit_days)
        .bind(s.wht_rate)
        .bind(t(&s.bank_name))
        .bind(t(&s.bank_account))
        .bind(s.is_active);
    if let Some(i) = id {
        q = q.bind(i);
    }
    let r = q.execute(&st.db).await?;
    Ok(id.unwrap_or(r.last_insert_id() as i64))
}

async fn create_supplier(State(st): State<AppState>, u: AuthUser, Json(s): Json<Supplier>) -> AppResult<Json<Value>> {
    u.need_any(&["purchasing.manage", "accounting.manage"])?;
    let id = save_supplier(&st, None, &s).await?;
    audit(&st, Some(&u), "create", "supplier", id, json!({ "name": s.name })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_supplier(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(s): Json<Supplier>) -> AppResult<Json<Value>> {
    u.need_any(&["purchasing.manage", "accounting.manage"])?;
    save_supplier(&st, Some(id), &s).await?;
    audit(&st, Some(&u), "update", "supplier", id, json!({ "name": s.name })).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- purchase orders

#[derive(Serialize, sqlx::FromRow)]
struct OrderRow {
    id: i64,
    po_no: String,
    po_date: NaiveDate,
    expected_date: Option<NaiveDate>,
    supplier_id: i64,
    supplier_name: String,
    warehouse_id: Option<i64>,
    vat_mode: String,
    subtotal: Decimal,
    discount: Decimal,
    vat_amount: Decimal,
    total: Decimal,
    status: String,
    note: Option<String>,
    approved_by_name: Option<String>,
    created_at: DateTime<Utc>,
}

const PO_SELECT: &str = "SELECT o.id, o.po_no, o.po_date, o.expected_date, o.supplier_id, s.name AS supplier_name, o.warehouse_id, o.vat_mode,
    o.subtotal, o.discount, o.vat_amount, o.total, o.status, o.note, us.display_name AS approved_by_name, o.created_at
    FROM purchase_orders o JOIN suppliers s ON s.id = o.supplier_id LEFT JOIN users us ON us.id = o.approved_by";

#[derive(Deserialize)]
struct ListQ {
    status: Option<String>,
    supplier_id: Option<i64>,
}

async fn list_orders(State(st): State<AppState>, u: AuthUser, Query(q): Query<ListQ>) -> AppResult<Json<Vec<OrderRow>>> {
    view(&u)?;
    let mut sql = format!("{PO_SELECT} WHERE 1=1");
    if q.status.is_some() {
        sql.push_str(" AND o.status = ?");
    }
    if q.supplier_id.is_some() {
        sql.push_str(" AND o.supplier_id = ?");
    }
    sql.push_str(" ORDER BY o.po_date DESC, o.id DESC LIMIT 2000");
    let mut qq = sqlx::query_as::<_, OrderRow>(&sql);
    if let Some(s) = &q.status {
        qq = qq.bind(s);
    }
    if let Some(s) = q.supplier_id {
        qq = qq.bind(s);
    }
    Ok(Json(qq.fetch_all(&st.db).await?))
}

#[derive(Serialize, sqlx::FromRow)]
struct PoLine {
    id: i64,
    product_id: Option<i64>,
    sku: Option<String>,
    description: String,
    qty: Decimal,
    unit: Option<String>,
    unit_price: Decimal,
    discount_pct: Decimal,
    vat_exempt: bool,
    amount: Decimal,
    received_qty: Decimal,
}

async fn po_lines(db: impl sqlx::MySqlExecutor<'_>, id: i64) -> AppResult<Vec<PoLine>> {
    Ok(sqlx::query_as(
        "SELECT l.id, l.product_id, p.sku, l.description, l.qty, l.unit, l.unit_price, l.discount_pct, l.vat_exempt, l.amount, l.received_qty
           FROM purchase_order_lines l LEFT JOIN products p ON p.id = l.product_id WHERE l.po_id = ? ORDER BY l.sort, l.id",
    )
    .bind(id)
    .fetch_all(db)
    .await?)
}

async fn order_detail(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    view(&u)?;
    let o: OrderRow = sqlx::query_as(&format!("{PO_SELECT} WHERE o.id = ?")).bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let bills: Vec<(i64, String, NaiveDate, Decimal, String)> =
        sqlx::query_as("SELECT id, bill_no, bill_date, total, status FROM purchase_bills WHERE po_id = ? ORDER BY id").bind(id).fetch_all(&st.db).await?;
    Ok(Json(json!({
        "order": o, "lines": po_lines(&st.db, id).await?,
        "bills": bills.into_iter().map(|b| json!({ "id": b.0, "bill_no": b.1, "bill_date": b.2, "total": b.3, "status": b.4 })).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
struct OrderReq {
    supplier_id: i64,
    po_date: Option<NaiveDate>,
    expected_date: Option<NaiveDate>,
    warehouse_id: Option<i64>,
    vat_mode: Option<String>,
    #[serde(default)]
    discount: Decimal,
    note: Option<String>,
    lines: Vec<LineReq>,
}

async fn check_lines(tx: &mut Tx<'_>, lines: &[LineReq], vat_mode: &str, discount: Decimal) -> AppResult<()> {
    if lines.is_empty() || lines.len() > 200 {
        return Err(bad("เอกสารต้องมีรายการ 1-200 บรรทัด"));
    }
    if !["exclusive", "inclusive", "none"].contains(&vat_mode) || discount < Decimal::ZERO {
        return Err(bad("รูปแบบภาษีหรือส่วนลดไม่ถูกต้อง"));
    }
    for l in lines {
        if l.description.trim().is_empty() || l.qty <= Decimal::ZERO || l.unit_price < Decimal::ZERO || !(Decimal::ZERO..=dec!(100)).contains(&l.discount_pct) {
            return Err(bad("ทุกบรรทัดต้องมีรายละเอียด จำนวนมากกว่า 0 และราคาไม่ติดลบ"));
        }
        if let Some(p) = l.product_id {
            let ok: Option<bool> = sqlx::query_scalar("SELECT is_active FROM products WHERE id = ?").bind(p).fetch_optional(&mut **tx).await?;
            if ok != Some(true) {
                return Err(bad("มีสินค้าที่ไม่มีอยู่หรือปิดใช้งาน"));
            }
        }
    }
    Ok(())
}

async fn vat_rate(st: &AppState) -> Decimal {
    erp::d(setting(&st.db, "vat_rate").await.and_then(|v| v.as_f64()).unwrap_or(7.0))
}

async fn write_po_lines(tx: &mut Tx<'_>, id: i64, lines: &[LineReq]) -> AppResult<()> {
    sqlx::query("DELETE FROM purchase_order_lines WHERE po_id = ?").bind(id).execute(&mut **tx).await?;
    for (i, l) in lines.iter().enumerate() {
        sqlx::query(
            "INSERT INTO purchase_order_lines (po_id, product_id, description, qty, unit, unit_price, discount_pct, vat_exempt, amount, sort)
             VALUES (?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(id)
        .bind(l.product_id)
        .bind(l.description.trim().chars().take(500).collect::<String>())
        .bind(l.qty)
        .bind(l.unit.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .bind(l.unit_price)
        .bind(l.discount_pct)
        .bind(l.vat_exempt)
        .bind(line_amount(l))
        .bind(i as i32)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn create_order(State(st): State<AppState>, u: AuthUser, Json(r): Json<OrderReq>) -> AppResult<Json<Value>> {
    u.need("purchasing.manage")?;
    let vat_mode = r.vat_mode.clone().unwrap_or_else(|| "exclusive".into());
    let rate = vat_rate(&st).await;
    let date = r.po_date.unwrap_or_else(|| local_today(st.cfg.tz_offset_min));
    let mut tx = st.db.begin().await?;
    check_lines(&mut tx, &r.lines, &vat_mode, r.discount).await?;
    let ok: Option<bool> = sqlx::query_scalar("SELECT is_active FROM suppliers WHERE id = ?").bind(r.supplier_id).fetch_optional(&mut *tx).await?;
    if ok != Some(true) {
        return Err(bad("ไม่พบผู้ขายหรือปิดใช้งาน"));
    }
    let t = totals(&r.lines, r.discount, &vat_mode, rate, Decimal::ZERO);
    let no = erp::next_doc_no(&mut tx, "PO", date).await?;
    let wh = match r.warehouse_id {
        Some(w) => w,
        None => default_warehouse(&mut *tx).await?,
    };
    let id = sqlx::query(
        "INSERT INTO purchase_orders (po_no, po_date, expected_date, supplier_id, warehouse_id, vat_mode, vat_rate, subtotal, discount, vat_amount,
            total, note, created_by) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&no)
    .bind(date)
    .bind(r.expected_date)
    .bind(r.supplier_id)
    .bind(wh)
    .bind(&vat_mode)
    .bind(rate)
    .bind(t.subtotal)
    .bind(t.discount)
    .bind(t.vat)
    .bind(t.total)
    .bind(r.note.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(u.id())
    .execute(&mut *tx)
    .await?
    .last_insert_id() as i64;
    write_po_lines(&mut tx, id, &r.lines).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "create", "purchase_order", id, json!({ "po_no": no, "total": t.total })).await;
    Ok(Json(json!({ "id": id, "po_no": no, "total": t.total })))
}

async fn update_order(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<OrderReq>) -> AppResult<Json<Value>> {
    u.need("purchasing.manage")?;
    let vat_mode = r.vat_mode.clone().unwrap_or_else(|| "exclusive".into());
    let rate = vat_rate(&st).await;
    let mut tx = st.db.begin().await?;
    let status: Option<String> = sqlx::query_scalar("SELECT status FROM purchase_orders WHERE id = ? FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?;
    if status.ok_or(AppError::NotFound)? != "draft" {
        return Err(bad("แก้ไขได้เฉพาะใบสั่งซื้อฉบับร่าง"));
    }
    check_lines(&mut tx, &r.lines, &vat_mode, r.discount).await?;
    let t = totals(&r.lines, r.discount, &vat_mode, rate, Decimal::ZERO);
    sqlx::query(
        "UPDATE purchase_orders SET supplier_id = ?, po_date = COALESCE(?, po_date), expected_date = ?, warehouse_id = COALESCE(?, warehouse_id),
            vat_mode = ?, vat_rate = ?, subtotal = ?, discount = ?, vat_amount = ?, total = ?, note = ? WHERE id = ?",
    )
    .bind(r.supplier_id)
    .bind(r.po_date)
    .bind(r.expected_date)
    .bind(r.warehouse_id)
    .bind(&vat_mode)
    .bind(rate)
    .bind(t.subtotal)
    .bind(t.discount)
    .bind(t.vat)
    .bind(t.total)
    .bind(r.note.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(id)
    .execute(&mut *tx)
    .await?;
    write_po_lines(&mut tx, id, &r.lines).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "update", "purchase_order", id, json!({ "total": t.total })).await;
    Ok(Json(json!({ "ok": true, "total": t.total })))
}

async fn approve_order(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("purchasing.manage")?;
    let created_by: Option<Option<i64>> = sqlx::query_scalar("SELECT created_by FROM purchase_orders WHERE id = ?").bind(id).fetch_optional(&st.db).await?;
    // segregation of duties: someone else approves, unless the approver is an administrator
    if created_by.ok_or(AppError::NotFound)? == Some(u.id()) && !u.is_admin() {
        return Err(bad("ผู้สร้างใบสั่งซื้ออนุมัติเองไม่ได้"));
    }
    let n = sqlx::query("UPDATE purchase_orders SET status = 'approved', approved_by = ?, approved_at = UTC_TIMESTAMP() WHERE id = ? AND status = 'draft'")
        .bind(u.id())
        .bind(id)
        .execute(&st.db)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(bad("อนุมัติได้เฉพาะใบสั่งซื้อฉบับร่าง"));
    }
    audit(&st, Some(&u), "approve", "purchase_order", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

async fn cancel_order(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("purchasing.manage")?;
    // a partly received order is closed (the rest will not come); an untouched one is cancelled
    let n = sqlx::query(
        "UPDATE purchase_orders SET status = CASE WHEN status = 'partial' THEN 'closed' ELSE 'cancelled' END
          WHERE id = ? AND status IN ('draft', 'approved', 'partial')",
    )
    .bind(id)
    .execute(&st.db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(bad("ยกเลิกได้เฉพาะใบสั่งซื้อที่ยังรับสินค้าไม่ครบ"));
    }
    audit(&st, Some(&u), "cancel", "purchase_order", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

async fn refresh_po(tx: &mut Tx<'_>, po: i64) -> AppResult<()> {
    sqlx::query(
        "UPDATE purchase_orders o SET status = CASE
            WHEN (SELECT COUNT(*) FROM purchase_order_lines l WHERE l.po_id = o.id AND l.received_qty < l.qty) = 0 THEN 'received'
            WHEN (SELECT COALESCE(SUM(l.received_qty), 0) FROM purchase_order_lines l WHERE l.po_id = o.id) > 0 THEN 'partial'
            ELSE 'approved' END
          WHERE o.id = ? AND o.status IN ('approved', 'partial', 'received')",
    )
    .bind(po)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------- bills (goods receipt)

#[derive(Serialize, sqlx::FromRow)]
struct BillRow {
    id: i64,
    bill_no: String,
    supplier_invoice_no: Option<String>,
    bill_date: NaiveDate,
    due_date: Option<NaiveDate>,
    supplier_id: i64,
    supplier_name: String,
    supplier_tax_id: Option<String>,
    po_id: Option<i64>,
    po_no: Option<String>,
    vat_mode: String,
    vat_rate: Decimal,
    subtotal: Decimal,
    discount: Decimal,
    vat_base: Decimal,
    vat_amount: Decimal,
    total: Decimal,
    wht_rate: Decimal,
    wht_amount: Decimal,
    paid_amount: Decimal,
    status: String,
    note: Option<String>,
    journal_entry_id: Option<i64>,
    created_at: DateTime<Utc>,
}

const BILL_SELECT: &str = "SELECT b.id, b.bill_no, b.supplier_invoice_no, b.bill_date, b.due_date, b.supplier_id, s.name AS supplier_name,
    s.tax_id AS supplier_tax_id, b.po_id, o.po_no, b.vat_mode, b.vat_rate, b.subtotal, b.discount, b.vat_base, b.vat_amount, b.total, b.wht_rate,
    b.wht_amount, b.paid_amount, b.status, b.note, b.journal_entry_id, b.created_at
    FROM purchase_bills b JOIN suppliers s ON s.id = b.supplier_id LEFT JOIN purchase_orders o ON o.id = b.po_id";

async fn list_bills(State(st): State<AppState>, u: AuthUser, Query(q): Query<ListQ>) -> AppResult<Json<Vec<BillRow>>> {
    view(&u)?;
    let mut sql = format!("{BILL_SELECT} WHERE 1=1");
    if q.status.is_some() {
        sql.push_str(" AND b.status = ?");
    }
    if q.supplier_id.is_some() {
        sql.push_str(" AND b.supplier_id = ?");
    }
    sql.push_str(" ORDER BY b.bill_date DESC, b.id DESC LIMIT 2000");
    let mut qq = sqlx::query_as::<_, BillRow>(&sql);
    if let Some(s) = &q.status {
        qq = qq.bind(s);
    }
    if let Some(s) = q.supplier_id {
        qq = qq.bind(s);
    }
    Ok(Json(qq.fetch_all(&st.db).await?))
}

#[derive(Serialize, sqlx::FromRow)]
struct PayRow {
    id: i64,
    voucher_no: String,
    wht_cert_no: Option<String>,
    pay_date: NaiveDate,
    amount: Decimal,
    wht_amount: Decimal,
    method: String,
    account_name: String,
    reference: Option<String>,
    status: String,
}

async fn bill_detail(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    view(&u)?;
    let b: BillRow = sqlx::query_as(&format!("{BILL_SELECT} WHERE b.id = ?")).bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let lines: Vec<(Option<i64>, String, Decimal, Option<String>, Decimal, Decimal, Decimal, Option<String>)> = sqlx::query_as(
        "SELECT l.product_id, l.description, l.qty, l.unit, l.unit_price, l.discount_pct, l.amount, a.name
           FROM purchase_bill_lines l LEFT JOIN accounts a ON a.id = l.account_id WHERE l.bill_id = ? ORDER BY l.sort, l.id",
    )
    .bind(id)
    .fetch_all(&st.db)
    .await?;
    let pays: Vec<PayRow> = sqlx::query_as(
        "SELECT p.id, p.voucher_no, p.wht_cert_no, p.pay_date, p.amount, p.wht_amount, p.method, a.name AS account_name, p.reference, p.status
           FROM purchase_payments p JOIN accounts a ON a.id = p.account_id WHERE p.bill_id = ? ORDER BY p.pay_date, p.id",
    )
    .bind(id)
    .fetch_all(&st.db)
    .await?;
    Ok(Json(json!({
        "bill": b, "payments": pays,
        "lines": lines.into_iter().map(|l| json!({ "product_id": l.0, "description": l.1, "qty": l.2, "unit": l.3, "unit_price": l.4,
            "discount_pct": l.5, "amount": l.6, "account_name": l.7 })).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize, Clone)]
struct BillLineReq {
    po_line_id: Option<i64>,
    product_id: Option<i64>,
    description: String,
    qty: Decimal,
    unit: Option<String>,
    unit_price: Decimal,
    #[serde(default)]
    discount_pct: Decimal,
    #[serde(default)]
    vat_exempt: bool,
    /// expense / asset account for a line that is not stock (default: product's account or miscellaneous expense)
    account_id: Option<i64>,
}

impl BillLineReq {
    fn line(&self) -> LineReq {
        LineReq {
            product_id: self.product_id,
            description: self.description.clone(),
            qty: self.qty,
            unit: self.unit.clone(),
            unit_price: self.unit_price,
            discount_pct: self.discount_pct,
            vat_exempt: self.vat_exempt,
        }
    }
}

#[derive(Deserialize)]
struct BillReq {
    supplier_id: i64,
    po_id: Option<i64>,
    supplier_invoice_no: Option<String>,
    bill_date: Option<NaiveDate>,
    due_date: Option<NaiveDate>,
    warehouse_id: Option<i64>,
    vat_mode: Option<String>,
    #[serde(default)]
    discount: Decimal,
    wht_rate: Option<Decimal>,
    note: Option<String>,
    lines: Vec<BillLineReq>,
}

/// Records goods / services received with the supplier's invoice and posts it at once:
/// stock in at net cost (or expense), input VAT, accounts payable.
async fn create_bill(State(st): State<AppState>, u: AuthUser, Json(r): Json<BillReq>) -> AppResult<Json<Value>> {
    u.need("purchasing.manage")?;
    let vat_mode = r.vat_mode.clone().unwrap_or_else(|| "exclusive".into());
    let rate = vat_rate(&st).await;
    let date = r.bill_date.unwrap_or_else(|| local_today(st.cfg.tz_offset_min));
    let mut tx = st.db.begin().await?;
    let reqs: Vec<LineReq> = r.lines.iter().map(BillLineReq::line).collect();
    check_lines(&mut tx, &reqs, &vat_mode, r.discount).await?;
    let sup: Option<(String, i32, Decimal, bool)> = sqlx::query_as("SELECT name, credit_days, wht_rate, is_active FROM suppliers WHERE id = ?")
        .bind(r.supplier_id)
        .fetch_optional(&mut *tx)
        .await?;
    let (sup_name, credit, sup_wht, active) = sup.ok_or_else(|| bad("ไม่พบผู้ขาย"))?;
    if !active {
        return Err(bad("ผู้ขายนี้ปิดใช้งาน"));
    }
    let wht_rate = r.wht_rate.unwrap_or(sup_wht);
    if !(Decimal::ZERO..=dec!(15)).contains(&wht_rate) {
        return Err(bad("อัตราหัก ณ ที่จ่ายต้องอยู่ระหว่าง 0-15%"));
    }
    // receiving against a purchase order: approved, same supplier, not more than ordered
    if let Some(po) = r.po_id {
        let o: Option<(i64, String)> = sqlx::query_as("SELECT supplier_id, status FROM purchase_orders WHERE id = ? FOR UPDATE").bind(po).fetch_optional(&mut *tx).await?;
        let (sid, status) = o.ok_or_else(|| bad("ไม่พบใบสั่งซื้อ"))?;
        if sid != r.supplier_id || !["approved", "partial"].contains(&status.as_str()) {
            return Err(bad("ใบสั่งซื้อต้องอนุมัติแล้ว ยังรับไม่ครบ และเป็นของผู้ขายรายนี้"));
        }
        for l in &r.lines {
            let Some(pl) = l.po_line_id else { continue };
            let row: Option<(Decimal, Decimal)> = sqlx::query_as("SELECT qty, received_qty FROM purchase_order_lines WHERE id = ? AND po_id = ?")
                .bind(pl)
                .bind(po)
                .fetch_optional(&mut *tx)
                .await?;
            let (ordered, got) = row.ok_or_else(|| bad("บรรทัดไม่ตรงกับใบสั่งซื้อ"))?;
            if got + l.qty > ordered {
                return Err(bad(format!("\"{}\" รับเกินจำนวนที่สั่ง (สั่ง {}, รับแล้ว {})", l.description, ordered.normalize(), got.normalize())));
            }
        }
    } else if r.lines.iter().any(|l| l.po_line_id.is_some()) {
        return Err(bad("ระบุบรรทัดใบสั่งซื้อแต่ไม่ได้ระบุใบสั่งซื้อ"));
    }
    let t = totals(&reqs, r.discount, &vat_mode, rate, wht_rate);
    let no = erp::next_doc_no(&mut tx, "GR", date).await?;
    let wh = match r.warehouse_id {
        Some(w) => w,
        None => default_warehouse(&mut *tx).await?,
    };
    let id = sqlx::query(
        "INSERT INTO purchase_bills (bill_no, supplier_invoice_no, bill_date, due_date, supplier_id, po_id, warehouse_id, vat_mode, vat_rate,
            subtotal, discount, vat_base, exempt_amount, vat_amount, total, wht_rate, wht_amount, note, created_by)
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&no)
    .bind(r.supplier_invoice_no.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(date)
    .bind(r.due_date.unwrap_or(date + Duration::days(credit.max(0) as i64)))
    .bind(r.supplier_id)
    .bind(r.po_id)
    .bind(wh)
    .bind(&vat_mode)
    .bind(if vat_mode == "none" { Decimal::ZERO } else { rate })
    .bind(t.subtotal)
    .bind(t.discount)
    .bind(t.vat_base)
    .bind(t.exempt)
    .bind(t.vat)
    .bind(t.total)
    .bind(wht_rate)
    .bind(t.wht)
    .bind(r.note.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(u.id())
    .execute(&mut *tx)
    .await?
    .last_insert_id() as i64;
    let mut lines_out = Vec::new();
    for (i, (l, net)) in r.lines.iter().zip(&t.net_lines).enumerate() {
        let (track, prod_acc): (bool, Option<i64>) = match l.product_id {
            Some(p) => {
                let (track, acc): (bool, Option<i64>) =
                    sqlx::query_as("SELECT track_stock, expense_account_id FROM products WHERE id = ?").bind(p).fetch_one(&mut *tx).await?;
                if !l.qty.is_zero() {
                    sqlx::query("UPDATE products SET purchase_price = ? WHERE id = ?").bind(l.unit_price).bind(p).execute(&mut *tx).await?;
                }
                (track, acc)
            }
            None => (false, None),
        };
        let account = if track {
            stock_in(
                &mut tx,
                Move { date, product: l.product_id.unwrap(), warehouse: wh, qty: l.qty, source: "purchase_receipt", source_id: Some(id), note: Some(no.clone()), user: Some(u.id()) },
                *net / l.qty,
            )
            .await?;
            None
        } else {
            let a = l.account_id.or(prod_acc);
            if let Some(a) = a {
                let t: Option<String> = sqlx::query_scalar("SELECT type FROM accounts WHERE id = ? AND is_active = 1").bind(a).fetch_optional(&mut *tx).await?;
                if !matches!(t.as_deref(), Some("expense") | Some("asset")) {
                    return Err(bad("บัญชีของรายการต้องเป็นบัญชีค่าใช้จ่ายหรือสินทรัพย์"));
                }
            }
            a
        };
        lines_out.push(
            match (track, account) {
                (true, _) => Line::dr(Acc::Role("inventory"), *net),
                (false, Some(a)) => Line::dr(Acc::Id(a), *net),
                (false, None) => Line::dr(Acc::Role("expense_default"), *net),
            }
            .memo(l.description.chars().take(120).collect::<String>()),
        );
        sqlx::query(
            "INSERT INTO purchase_bill_lines (bill_id, po_line_id, product_id, description, qty, unit, unit_price, discount_pct, vat_exempt, amount, account_id, sort)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(id)
        .bind(l.po_line_id)
        .bind(l.product_id)
        .bind(l.description.trim().chars().take(500).collect::<String>())
        .bind(l.qty)
        .bind(l.unit.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .bind(l.unit_price)
        .bind(l.discount_pct)
        .bind(l.vat_exempt)
        .bind(line_amount(&l.line()))
        .bind(account)
        .bind(i as i32)
        .execute(&mut *tx)
        .await?;
        if let Some(pl) = l.po_line_id {
            sqlx::query("UPDATE purchase_order_lines SET received_qty = received_qty + ? WHERE id = ?").bind(l.qty).bind(pl).execute(&mut *tx).await?;
        }
    }
    lines_out.push(Line::dr(Acc::Role("vat_input"), t.vat).memo(format!("ภาษีซื้อ {}", r.supplier_invoice_no.as_deref().unwrap_or(&no))));
    lines_out.push(Line::cr(Acc::Role("ap"), t.total).party("supplier", r.supplier_id).memo(format!("{no} {sup_name}")));
    let je = erp::post(&mut tx, date, &format!("ซื้อ {no} {sup_name}"), "purchase_receipt", Some(id), lines_out, Some(u.id())).await?;
    sqlx::query("UPDATE purchase_bills SET journal_entry_id = ? WHERE id = ?").bind(je).bind(id).execute(&mut *tx).await?;
    if let Some(po) = r.po_id {
        refresh_po(&mut tx, po).await?;
    }
    tx.commit().await?;
    audit(&st, Some(&u), "create", "purchase_bill", id, json!({ "bill_no": no, "total": t.total })).await;
    Ok(Json(json!({ "id": id, "bill_no": no, "total": t.total, "journal_entry_id": je })))
}

async fn void_bill(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("purchasing.manage")?;
    let today = local_today(st.cfg.tz_offset_min);
    let mut tx = st.db.begin().await?;
    let b: Option<(String, Option<i64>)> = sqlx::query_as("SELECT status, po_id FROM purchase_bills WHERE id = ? FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?;
    let (status, po) = b.ok_or(AppError::NotFound)?;
    if status != "posted" {
        return Err(bad("ยกเลิกได้เฉพาะรายการที่ยังไม่มีการจ่ายเงิน (ยกเลิกการจ่ายก่อน)"));
    }
    erp::void_entries(&mut tx, "purchase_receipt", id).await?;
    reverse_moves(&mut tx, "purchase_receipt", id, "purchase_void", today, Some(u.id())).await?;
    let pls: Vec<(i64, Decimal)> = sqlx::query_as("SELECT po_line_id, qty FROM purchase_bill_lines WHERE bill_id = ? AND po_line_id IS NOT NULL")
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
    for (pl, q) in pls {
        sqlx::query("UPDATE purchase_order_lines SET received_qty = GREATEST(received_qty - ?, 0) WHERE id = ?").bind(q).bind(pl).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE purchase_bills SET status = 'void' WHERE id = ?").bind(id).execute(&mut *tx).await?;
    if let Some(po) = po {
        refresh_po(&mut tx, po).await?;
    }
    tx.commit().await?;
    audit(&st, Some(&u), "void", "purchase_bill", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- payments

#[derive(Deserialize)]
struct PayReq {
    pay_date: Option<NaiveDate>,
    amount: Decimal,
    #[serde(default)]
    wht_amount: Decimal,
    wht_income_type: Option<String>,
    method: Option<String>,
    account_id: Option<i64>,
    reference: Option<String>,
}

async fn refresh_bill(tx: &mut Tx<'_>, id: i64) -> AppResult<()> {
    sqlx::query(
        "UPDATE purchase_bills b SET paid_amount = (SELECT COALESCE(SUM(amount + wht_amount), 0) FROM purchase_payments p WHERE p.bill_id = b.id AND p.status = 'active'),
            status = CASE WHEN (SELECT COALESCE(SUM(amount + wht_amount), 0) FROM purchase_payments p WHERE p.bill_id = b.id AND p.status = 'active') >= b.total THEN 'paid'
                          WHEN (SELECT COUNT(*) FROM purchase_payments p WHERE p.bill_id = b.id AND p.status = 'active') > 0 THEN 'partial'
                          ELSE 'posted' END
          WHERE b.id = ? AND b.status <> 'void'",
    )
    .bind(id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Pays a supplier: Dr AP / Cr bank (net) / Cr WHT payable; voucher + 50 ทวิ numbers.
async fn pay_bill(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<PayReq>) -> AppResult<Json<Value>> {
    u.need("purchasing.manage")?;
    let method = r.method.as_deref().unwrap_or("transfer");
    if !["transfer", "cash", "cheque", "card"].contains(&method) {
        return Err(bad("วิธีจ่ายเงินไม่ถูกต้อง"));
    }
    if r.amount < Decimal::ZERO || r.wht_amount < Decimal::ZERO || (r.amount + r.wht_amount).is_zero() {
        return Err(bad("จำนวนเงินต้องมากกว่า 0"));
    }
    let date = r.pay_date.unwrap_or_else(|| local_today(st.cfg.tz_offset_min));
    let mut tx = st.db.begin().await?;
    let b: BillRow = sqlx::query_as(&format!("{BILL_SELECT} WHERE b.id = ? FOR UPDATE")).bind(id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    if !["posted", "partial"].contains(&b.status.as_str()) {
        return Err(bad("จ่ายได้เฉพาะรายการที่ยังค้างชำระ"));
    }
    let settle = r2(r.amount) + r2(r.wht_amount);
    if settle > b.total - b.paid_amount {
        return Err(bad(format!("ยอดจ่าย ({}) มากกว่ายอดค้าง ({})", fmt_money(settle), fmt_money(b.total - b.paid_amount))));
    }
    let account = match r.account_id {
        Some(a) => {
            let t: Option<String> = sqlx::query_scalar("SELECT type FROM accounts WHERE id = ? AND is_active = 1").bind(a).fetch_optional(&mut *tx).await?;
            if t.as_deref() != Some("asset") {
                return Err(bad("บัญชีที่จ่ายต้องเป็นบัญชีเงินสด / ธนาคาร"));
            }
            a
        }
        None => erp::account_id(&mut tx, &Acc::Role(if method == "cash" { "cash" } else { "bank" })).await?,
    };
    let voucher = erp::next_doc_no(&mut tx, "PV", date).await?;
    let cert = if r.wht_amount > Decimal::ZERO { Some(erp::next_doc_no(&mut tx, "WT", date).await?) } else { None };
    let base = b.total - b.vat_amount;
    let wht_rate = if base > Decimal::ZERO && r.wht_amount > Decimal::ZERO { (r.wht_amount / base * dec!(100)).round_dp(2) } else { Decimal::ZERO };
    let je = erp::post(
        &mut tx,
        date,
        &format!("จ่ายชำระ {} {} ({voucher})", b.bill_no, b.supplier_name),
        "purchase_payment",
        None,
        vec![
            Line::dr(Acc::Role("ap"), settle).party("supplier", b.supplier_id),
            Line::cr(Acc::Id(account), r2(r.amount)).memo(r.reference.clone().unwrap_or_default()),
            Line::cr(Acc::Role("wht_payable"), r2(r.wht_amount)).memo(format!("หัก ณ ที่จ่าย {}", cert.as_deref().unwrap_or(""))),
        ],
        Some(u.id()),
    )
    .await?;
    let pid = sqlx::query(
        "INSERT INTO purchase_payments (bill_id, voucher_no, wht_cert_no, pay_date, amount, wht_amount, wht_rate, wht_income_type, method, account_id,
            reference, journal_entry_id, created_by) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(id)
    .bind(&voucher)
    .bind(&cert)
    .bind(date)
    .bind(r2(r.amount))
    .bind(r2(r.wht_amount))
    .bind(wht_rate)
    .bind(r.wht_income_type.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("ค่าบริการ / ค่าจ้างทำของ (มาตรา 40(8))"))
    .bind(method)
    .bind(account)
    .bind(r.reference.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(je)
    .bind(u.id())
    .execute(&mut *tx)
    .await?
    .last_insert_id() as i64;
    sqlx::query("UPDATE journal_entries SET source_id = ? WHERE id = ?").bind(pid).bind(je).execute(&mut *tx).await?;
    refresh_bill(&mut tx, id).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "pay", "purchase_bill", id, json!({ "voucher_no": voucher, "amount": r.amount, "wht": r.wht_amount })).await;
    Ok(Json(json!({ "ok": true, "payment_id": pid, "voucher_no": voucher, "wht_cert_no": cert, "journal_entry_id": je })))
}

async fn void_payment(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("purchasing.manage")?;
    let mut tx = st.db.begin().await?;
    let p: Option<(i64, String)> = sqlx::query_as("SELECT bill_id, status FROM purchase_payments WHERE id = ? FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?;
    let (bill, status) = p.ok_or(AppError::NotFound)?;
    if status != "active" {
        return Err(bad("รายการจ่ายนี้ถูกยกเลิกแล้ว"));
    }
    erp::void_entries(&mut tx, "purchase_payment", id).await?;
    sqlx::query("UPDATE purchase_payments SET status = 'void' WHERE id = ?").bind(id).execute(&mut *tx).await?;
    refresh_bill(&mut tx, bill).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "void_payment", "purchase_bill", bill, json!({ "payment_id": id })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn summary(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    view(&u)?;
    let today = local_today(st.cfg.tz_offset_min);
    let (ap, overdue, overdue_n): (Decimal, Decimal, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(total - paid_amount), 0), COALESCE(SUM(CASE WHEN due_date < ? THEN total - paid_amount END), 0),
                CAST(COALESCE(SUM(due_date < ?), 0) AS SIGNED)
           FROM purchase_bills WHERE status IN ('posted', 'partial')",
    )
    .bind(today)
    .bind(today)
    .fetch_one(&st.db)
    .await?;
    let (pending_po, pending_value): (i64, Decimal) =
        sqlx::query_as("SELECT COUNT(*), COALESCE(SUM(total), 0) FROM purchase_orders WHERE status IN ('draft', 'approved', 'partial')").fetch_one(&st.db).await?;
    let (low,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM products p WHERE p.track_stock = 1 AND p.is_active = 1 AND p.reorder_point > 0
           AND COALESCE((SELECT SUM(qty) FROM stock_balances b WHERE b.product_id = p.id), 0) <= p.reorder_point",
    )
    .fetch_one(&st.db)
    .await?;
    Ok(Json(json!({ "payable": ap, "overdue": overdue, "overdue_count": overdue_n, "open_orders": pending_po, "open_order_value": pending_value, "low_stock": low })))
}

// ---------------------------------------------------------------- printing

async fn print_order(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Html<String>> {
    view(&u)?;
    let o: OrderRow = sqlx::query_as(&format!("{PO_SELECT} WHERE o.id = ?")).bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let s: (Option<String>, Option<String>, Option<String>, Option<String>) =
        sqlx::query_as("SELECT tax_id, address, contact_name, phone FROM suppliers WHERE id = ?").bind(o.supplier_id).fetch_one(&st.db).await?;
    let co = company(&st).await;
    let rows: String = po_lines(&st.db, id)
        .await?
        .iter()
        .enumerate()
        .map(|(i, l)| {
            format!(
                "<tr><td class=\"c\">{}</td><td>{}</td><td class=\"r\">{} {}</td><td class=\"r\">{}</td><td class=\"r\">{}</td></tr>",
                i + 1,
                esc(&l.description),
                l.qty.normalize(),
                esc(l.unit.as_deref().unwrap_or("")),
                fmt_money(l.unit_price),
                fmt_money(l.amount)
            )
        })
        .collect();
    audit(&st, Some(&u), "print", "purchase_order", id, json!({})).await;
    Ok(Html(format!(
        r#"<!doctype html><html lang="th"><head><meta charset="utf-8"><title>ใบสั่งซื้อ {no}</title><style>{css}</style></head><body>
<div class="toolbar"><button onclick="window.print()">พิมพ์ / บันทึกเป็น PDF (A4)</button></div><div class="page">
<div class="head"><div><h1>{org}</h1><div class="muted">{addr}</div><div class="muted">เลขประจำตัวผู้เสียภาษี {tax}</div></div>
<div class="r"><div class="big">ใบสั่งซื้อ</div><div class="muted">Purchase Order</div>{status}</div></div>
<div class="grid2"><div class="box"><b>ผู้ขาย</b><br>{sup}<br>{saddr}<br>เลขประจำตัวผู้เสียภาษี {stax}<br>ติดต่อ {contact} {phone}</div>
<div class="box">เลขที่ <b>{no}</b><br>วันที่ {date}<br>ต้องการรับสินค้า {exp}</div></div>
<table><thead><tr><th>ลำดับ</th><th>รายการ</th><th class="r">จำนวน</th><th class="r">ราคา/หน่วย</th><th class="r">จำนวนเงิน</th></tr></thead><tbody>{rows}</tbody></table>
<div class="grid2" style="margin-top:10px"><div class="box">({words})<br><span class="muted">{note}</span></div>
<table style="margin-top:0"><tr><td>รวมเงิน</td><td class="r">{sub}</td></tr><tr><td>ส่วนลด</td><td class="r">{disc}</td></tr><tr><td>ภาษีมูลค่าเพิ่ม</td><td class="r">{vat}</td></tr>
<tr class="tot"><td>รวมทั้งสิ้น</td><td class="r big">{total}</td></tr></table></div>
<div class="sign"><div>ผู้สั่งซื้อ</div><div>ผู้อนุมัติ {approver}</div></div></div></body></html>"#,
        css = PRINT_CSS,
        no = esc(&o.po_no),
        org = esc(co["name"].as_str().unwrap_or("")),
        addr = esc(co["address"].as_str().unwrap_or("")),
        tax = esc(co["tax_id"].as_str().unwrap_or("")),
        status = if o.status == "draft" { "<div class=\"stamp\">ฉบับร่าง (ยังไม่อนุมัติ)</div>" } else if o.status == "cancelled" { "<div class=\"stamp\">ยกเลิก</div>" } else { "" },
        sup = esc(&o.supplier_name),
        saddr = esc(s.1.as_deref().unwrap_or("")),
        stax = esc(s.0.as_deref().unwrap_or("-")),
        contact = esc(s.2.as_deref().unwrap_or("")),
        phone = esc(s.3.as_deref().unwrap_or("")),
        date = th_date(o.po_date),
        exp = o.expected_date.map(th_date).unwrap_or_else(|| "-".into()),
        rows = rows,
        words = erp::baht_text(o.total),
        note = esc(o.note.as_deref().unwrap_or("")),
        sub = fmt_money(o.subtotal),
        disc = fmt_money(o.discount),
        vat = fmt_money(o.vat_amount),
        total = fmt_money(o.total),
        approver = esc(o.approved_by_name.as_deref().unwrap_or("")),
    )))
}

/// Payment voucher, plus the withholding tax certificate (50 ทวิ) when tax was withheld.
async fn print_payment(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Html<String>> {
    view(&u)?;
    let p: Option<(i64, String, Option<String>, NaiveDate, Decimal, Decimal, Decimal, Option<String>, String, String)> = sqlx::query_as(
        "SELECT p.bill_id, p.voucher_no, p.wht_cert_no, p.pay_date, p.amount, p.wht_amount, p.wht_rate, p.wht_income_type, p.method, p.status
           FROM purchase_payments p WHERE p.id = ?",
    )
    .bind(id)
    .fetch_optional(&st.db)
    .await?;
    let (bill_id, voucher, cert, date, amount, wht, rate, income, method, status) = p.ok_or(AppError::NotFound)?;
    let b: BillRow = sqlx::query_as(&format!("{BILL_SELECT} WHERE b.id = ?")).bind(bill_id).fetch_one(&st.db).await?;
    let s: (String, Option<String>, Option<String>) = sqlx::query_as("SELECT kind, address, branch_code FROM suppliers WHERE id = ?").bind(b.supplier_id).fetch_one(&st.db).await?;
    let co = company(&st).await;
    let gs = |k: &str| esc(co[k].as_str().unwrap_or(""));
    let void = if status == "void" { "<div class=\"stamp\" style=\"color:#c00;border-color:#c00\">ยกเลิก</div>" } else { "" };
    let mut pages = format!(
        r#"<div class="page"><div class="head"><div><h1>{org}</h1><div class="muted">{addr}</div></div><div class="r"><div class="big">ใบสำคัญจ่าย</div><div class="muted">Payment Voucher</div>{void}</div></div>
<div class="grid2"><div class="box"><b>จ่ายให้</b><br>{sup}<br>เลขประจำตัวผู้เสียภาษี {stax}</div><div class="box">เลขที่ <b>{voucher}</b><br>วันที่ {date}<br>วิธีจ่าย {method}</div></div>
<table><thead><tr><th>รายการ</th><th class="r">จำนวนเงิน</th></tr></thead><tbody>
<tr><td>ชำระตาม {bill} (ใบกำกับภาษีผู้ขายเลขที่ {inv})</td><td class="r">{settle}</td></tr>
<tr><td>หัก ภาษีเงินได้หัก ณ ที่จ่าย {rate}% {certline}</td><td class="r">-{wht}</td></tr>
<tr class="tot"><td>จ่ายสุทธิ</td><td class="r big">{amount}</td></tr></tbody></table>
<p>({words})</p><div class="sign"><div>ผู้จ่ายเงิน</div><div>ผู้รับเงิน</div></div></div>"#,
        org = gs("name"),
        addr = gs("address"),
        void = void,
        sup = esc(&b.supplier_name),
        stax = esc(b.supplier_tax_id.as_deref().unwrap_or("-")),
        voucher = esc(&voucher),
        date = th_date(date),
        method = match method.as_str() { "cash" => "เงินสด", "cheque" => "เช็ค", "card" => "บัตร", _ => "โอนเงิน" },
        bill = esc(&b.bill_no),
        inv = esc(b.supplier_invoice_no.as_deref().unwrap_or("-")),
        settle = fmt_money(amount + wht),
        rate = rate.normalize(),
        certline = cert.as_deref().map(|c| format!("(หนังสือรับรองเลขที่ {})", esc(c))).unwrap_or_default(),
        wht = fmt_money(wht),
        amount = fmt_money(amount),
        words = erp::baht_text(amount),
    );
    if let Some(cert) = &cert {
        let form = if s.0 == "individual" { "ภ.ง.ด.3" } else { "ภ.ง.ด.53" };
        for copy in ["ฉบับที่ 1 (สำหรับผู้ถูกหักภาษี ใช้แนบพร้อมแบบแสดงรายการภาษี)", "ฉบับที่ 2 (สำหรับผู้ถูกหักภาษี เก็บไว้เป็นหลักฐาน)"] {
            pages.push_str(&format!(
                r#"<div class="page"><div class="c"><div class="big">หนังสือรับรองการหักภาษี ณ ที่จ่าย</div><div>ตามมาตรา 50 ทวิ แห่งประมวลรัษฎากร</div><div class="muted">{copy}</div><div>เล่มที่/เลขที่ <b>{cert}</b> {void}</div></div>
<div class="box" style="margin-top:12px"><b>ผู้มีหน้าที่หักภาษี ณ ที่จ่าย</b><br>{org}<br>เลขประจำตัวผู้เสียภาษีอากร {otax} {obranch}<br>{oaddr}</div>
<div class="box" style="margin-top:8px"><b>ผู้ถูกหักภาษี ณ ที่จ่าย</b><br>{sup}<br>เลขประจำตัวผู้เสียภาษีอากร {stax} {sbranch}<br>{saddr}</div>
<p>ลำดับที่ในแบบ: <b>{form}</b></p>
<table><thead><tr><th>ประเภทเงินได้พึงประเมินที่จ่าย</th><th>วัน เดือน ปี ที่จ่าย</th><th class="r">จำนวนเงินที่จ่าย</th><th class="r">ภาษีที่หักและนำส่งไว้</th></tr></thead>
<tbody><tr><td>{income}</td><td>{date}</td><td class="r">{base}</td><td class="r">{wht}</td></tr>
<tr class="tot"><td colspan="2">รวมเงินที่จ่ายและภาษีที่หักนำส่ง</td><td class="r">{base}</td><td class="r">{wht}</td></tr></tbody></table>
<p>รวมเงินภาษีที่หักนำส่ง (ตัวอักษร) ({words})</p>
<p>ผู้จ่ายเงิน: [✓] หัก ณ ที่จ่าย &nbsp; [ ] ออกให้ตลอดไป &nbsp; [ ] ออกให้ครั้งเดียว</p>
<p class="muted">ขอรับรองว่าข้อความและตัวเลขดังกล่าวข้างต้นถูกต้องตรงกับความจริงทุกประการ</p>
<div class="sign"><div>ผู้จ่ายเงิน วันที่ {date}</div><div>ตราประทับนิติบุคคล (ถ้ามี)</div></div></div>"#,
                copy = copy,
                cert = esc(cert),
                void = void,
                org = gs("name"),
                otax = gs("tax_id"),
                obranch = gs("branch"),
                oaddr = gs("address"),
                sup = esc(&b.supplier_name),
                stax = esc(b.supplier_tax_id.as_deref().unwrap_or("-")),
                sbranch = match s.2.as_deref() { Some("00000") | None => "สำนักงานใหญ่".to_string(), Some(x) => format!("สาขา {}", esc(x)) },
                saddr = esc(s.1.as_deref().unwrap_or("")),
                form = form,
                income = esc(income.as_deref().unwrap_or("")),
                date = th_date(date),
                base = fmt_money(if rate > Decimal::ZERO { r2(wht * dec!(100) / rate) } else { b.total - b.vat_amount }),
                wht = fmt_money(wht),
                words = erp::baht_text(wht),
            ));
        }
    }
    audit(&st, Some(&u), "print", "purchase_payment", id, json!({})).await;
    Ok(Html(format!(
        r#"<!doctype html><html lang="th"><head><meta charset="utf-8"><title>ใบสำคัญจ่าย {v}</title><style>{css} .page{{page-break-after:always}}</style></head><body>
<div class="toolbar"><button onclick="window.print()">พิมพ์ / บันทึกเป็น PDF (A4)</button></div>{pages}</body></html>"#,
        v = esc(&voucher),
        css = PRINT_CSS,
        pages = pages
    )))
}
