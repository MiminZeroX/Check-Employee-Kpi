//! Sales: quotation -> invoice / tax invoice -> payments with receipts.
//! VAT (exclusive / inclusive / none, exempt lines), withholding tax deducted by the
//! customer, stock issue at average cost, automatic postings, Thai A4 documents.

use axum::{
    extract::{Path, Query, State},
    response::Html,
    routing::{get, post},
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
        inventory::{default_warehouse, reverse_moves, stock_out, Move},
        payroll::{th_date, PRINT_CSS},
    },
    state::AppState,
    util::{html_escape as esc, local_today, setting},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/sales/docs", get(list).post(create))
        .route("/sales/docs/{id}", get(detail).put(update).delete(remove))
        .route("/sales/docs/{id}/issue", post(issue))
        .route("/sales/docs/{id}/convert", post(convert))
        .route("/sales/docs/{id}/status", post(set_status))
        .route("/sales/docs/{id}/payments", post(add_payment))
        .route("/sales/docs/{id}/void", post(void_invoice))
        .route("/sales/docs/{id}/print", get(print))
        .route("/sales/payments/{id}/void", post(void_payment))
        .route("/sales/summary", get(summary))
}

// ---------------------------------------------------------------- calculation

#[derive(Deserialize, Clone, Debug)]
pub struct LineReq {
    pub product_id: Option<i64>,
    pub description: String,
    pub qty: Decimal,
    pub unit: Option<String>,
    pub unit_price: Decimal,
    #[serde(default)]
    pub discount_pct: Decimal,
    #[serde(default)]
    pub vat_exempt: bool,
}

#[derive(Debug, Default, PartialEq)]
pub struct Totals {
    pub subtotal: Decimal,
    pub discount: Decimal,
    pub vat_base: Decimal,
    pub exempt: Decimal,
    pub vat: Decimal,
    pub total: Decimal,
    pub wht: Decimal,
    /// line amounts after the document discount, before VAT (for revenue postings)
    pub net_lines: Vec<Decimal>,
}

pub fn line_amount(l: &LineReq) -> Decimal {
    r2(l.qty * l.unit_price * (dec!(100) - l.discount_pct) / dec!(100))
}

/// Document totals. The document discount is spread over the lines by amount; with
/// inclusive VAT the line prices already contain the tax.
pub fn totals(lines: &[LineReq], discount: Decimal, vat_mode: &str, vat_rate: Decimal, wht_rate: Decimal) -> Totals {
    let amounts: Vec<Decimal> = lines.iter().map(line_amount).collect();
    let subtotal: Decimal = amounts.iter().sum();
    let discount = r2(discount.clamp(Decimal::ZERO, subtotal));
    // spread the discount, keeping the rounding remainder on the largest line
    let mut after: Vec<Decimal> = amounts
        .iter()
        .map(|a| if subtotal.is_zero() { *a } else { r2(*a - discount * *a / subtotal) })
        .collect();
    let diff = (subtotal - discount) - after.iter().sum::<Decimal>();
    if let Some((i, _)) = after.iter().enumerate().max_by_key(|(_, a)| **a) {
        after[i] += diff;
    }
    let taxable: Decimal = after.iter().zip(lines).filter(|(_, l)| !l.vat_exempt).map(|(a, _)| *a).sum();
    let exempt: Decimal = after.iter().zip(lines).filter(|(_, l)| l.vat_exempt).map(|(a, _)| *a).sum();
    let (vat_base, vat, total, net_lines) = match vat_mode {
        "exclusive" => {
            let vat = r2(taxable * vat_rate / dec!(100));
            (taxable, vat, taxable + exempt + vat, after)
        }
        "inclusive" => {
            let vat = r2(taxable * vat_rate / (dec!(100) + vat_rate));
            // take the VAT out of the taxable lines proportionally
            let mut net: Vec<Decimal> = after
                .iter()
                .zip(lines)
                .map(|(a, l)| if l.vat_exempt || taxable.is_zero() { *a } else { r2(*a - vat * *a / taxable) })
                .collect();
            let want = taxable - vat + exempt;
            let d = want - net.iter().sum::<Decimal>();
            if let Some((i, _)) = net.iter().enumerate().max_by_key(|(_, a)| **a) {
                net[i] += d;
            }
            (taxable - vat, vat, taxable + exempt, net)
        }
        _ => (Decimal::ZERO, Decimal::ZERO, taxable + exempt, after),
    };
    let wht = r2((total - vat) * wht_rate / dec!(100));
    Totals { subtotal, discount, vat_base, exempt: if vat_mode == "none" { Decimal::ZERO } else { exempt }, vat, total, wht, net_lines }
}

// ---------------------------------------------------------------- list / detail

#[derive(Serialize, sqlx::FromRow)]
pub struct DocRow {
    pub id: i64,
    pub doc_type: String,
    pub doc_no: Option<String>,
    pub doc_date: NaiveDate,
    pub due_date: Option<NaiveDate>,
    pub customer_id: i64,
    pub customer_name: String,
    pub customer_tax_id: Option<String>,
    pub customer_branch: Option<String>,
    pub customer_address: Option<String>,
    pub reference_id: Option<i64>,
    pub reference_no: Option<String>,
    pub deal_id: Option<i64>,
    pub salesperson_employee_id: Option<i64>,
    pub warehouse_id: Option<i64>,
    pub vat_mode: String,
    pub vat_rate: Decimal,
    pub subtotal: Decimal,
    pub discount: Decimal,
    pub vat_base: Decimal,
    pub exempt_amount: Decimal,
    pub vat_amount: Decimal,
    pub total: Decimal,
    pub wht_rate: Decimal,
    pub wht_amount: Decimal,
    pub paid_amount: Decimal,
    pub status: String,
    pub note: Option<String>,
    pub journal_entry_id: Option<i64>,
    pub created_at: DateTime<Utc>,
}

const DOC_SELECT: &str = "SELECT d.id, d.doc_type, d.doc_no, d.doc_date, d.due_date, d.customer_id, d.customer_name, d.customer_tax_id,
    d.customer_branch, d.customer_address, d.reference_id, r.doc_no AS reference_no, d.deal_id, d.salesperson_employee_id, d.warehouse_id,
    d.vat_mode, d.vat_rate, d.subtotal, d.discount, d.vat_base, d.exempt_amount, d.vat_amount, d.total, d.wht_rate, d.wht_amount,
    d.paid_amount, d.status, d.note, d.journal_entry_id, d.created_at
    FROM sales_docs d LEFT JOIN sales_docs r ON r.id = d.reference_id";

#[derive(Deserialize)]
struct ListQ {
    doc_type: Option<String>,
    status: Option<String>,
    customer_id: Option<i64>,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
}

async fn list(State(st): State<AppState>, u: AuthUser, Query(q): Query<ListQ>) -> AppResult<Json<Vec<DocRow>>> {
    u.need_any(&["sales.view", "sales.manage", "accounting.view"])?;
    let mut sql = format!("{DOC_SELECT} WHERE 1=1");
    for (cond, on) in [
        (" AND d.doc_type = ?", q.doc_type.is_some()),
        (" AND d.status = ?", q.status.is_some()),
        (" AND d.customer_id = ?", q.customer_id.is_some()),
        (" AND d.doc_date >= ?", q.from.is_some()),
        (" AND d.doc_date <= ?", q.to.is_some()),
    ] {
        if on {
            sql.push_str(cond);
        }
    }
    sql.push_str(" ORDER BY d.doc_date DESC, d.id DESC LIMIT 2000");
    let mut qq = sqlx::query_as::<_, DocRow>(&sql);
    if let Some(v) = &q.doc_type {
        qq = qq.bind(v);
    }
    if let Some(v) = &q.status {
        qq = qq.bind(v);
    }
    if let Some(v) = q.customer_id {
        qq = qq.bind(v);
    }
    if let Some(v) = q.from {
        qq = qq.bind(v);
    }
    if let Some(v) = q.to {
        qq = qq.bind(v);
    }
    Ok(Json(qq.fetch_all(&st.db).await?))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct LineRow {
    pub id: i64,
    pub product_id: Option<i64>,
    pub sku: Option<String>,
    pub description: String,
    pub qty: Decimal,
    pub unit: Option<String>,
    pub unit_price: Decimal,
    pub discount_pct: Decimal,
    pub vat_exempt: bool,
    pub amount: Decimal,
}

async fn lines(db: impl sqlx::MySqlExecutor<'_>, id: i64) -> AppResult<Vec<LineRow>> {
    Ok(sqlx::query_as(
        "SELECT l.id, l.product_id, p.sku, l.description, l.qty, l.unit, l.unit_price, l.discount_pct, l.vat_exempt, l.amount
           FROM sales_doc_lines l LEFT JOIN products p ON p.id = l.product_id WHERE l.doc_id = ? ORDER BY l.sort, l.id",
    )
    .bind(id)
    .fetch_all(db)
    .await?)
}

async fn load(db: impl sqlx::MySqlExecutor<'_>, id: i64) -> AppResult<DocRow> {
    sqlx::query_as(&format!("{DOC_SELECT} WHERE d.id = ?")).bind(id).fetch_optional(db).await?.ok_or(AppError::NotFound)
}

#[derive(Serialize, sqlx::FromRow)]
struct PaymentRow {
    id: i64,
    receipt_id: Option<i64>,
    receipt_no: Option<String>,
    pay_date: NaiveDate,
    amount: Decimal,
    wht_amount: Decimal,
    method: String,
    account_name: String,
    reference: Option<String>,
    status: String,
}

async fn payments(db: impl sqlx::MySqlExecutor<'_>, invoice: i64) -> AppResult<Vec<PaymentRow>> {
    Ok(sqlx::query_as(
        "SELECT p.id, p.receipt_id, r.doc_no AS receipt_no, p.pay_date, p.amount, p.wht_amount, p.method, a.name AS account_name, p.reference, p.status
           FROM sales_payments p JOIN accounts a ON a.id = p.account_id LEFT JOIN sales_docs r ON r.id = p.receipt_id
          WHERE p.invoice_id = ? ORDER BY p.pay_date, p.id",
    )
    .bind(invoice)
    .fetch_all(db)
    .await?)
}

async fn detail(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need_any(&["sales.view", "sales.manage", "accounting.view"])?;
    let d = load(&st.db, id).await?;
    let pays = if d.doc_type == "invoice" { payments(&st.db, id).await? } else { vec![] };
    // documents made from this one (quotation -> invoice, invoice -> receipts)
    let children: Vec<(i64, String, Option<String>, String)> =
        sqlx::query_as("SELECT id, doc_type, doc_no, status FROM sales_docs WHERE reference_id = ? ORDER BY id").bind(id).fetch_all(&st.db).await?;
    Ok(Json(json!({
        "doc": d, "lines": lines(&st.db, id).await?, "payments": pays,
        "children": children.into_iter().map(|c| json!({ "id": c.0, "doc_type": c.1, "doc_no": c.2, "status": c.3 })).collect::<Vec<_>>(),
    })))
}

// ---------------------------------------------------------------- create / update

#[derive(Deserialize)]
struct DocReq {
    doc_type: Option<String>,
    doc_date: Option<NaiveDate>,
    due_date: Option<NaiveDate>,
    customer_id: i64,
    deal_id: Option<i64>,
    salesperson_employee_id: Option<i64>,
    warehouse_id: Option<i64>,
    vat_mode: Option<String>,
    #[serde(default)]
    discount: Decimal,
    #[serde(default)]
    wht_rate: Decimal,
    note: Option<String>,
    lines: Vec<LineReq>,
}

async fn vat_rate(st: &AppState) -> Decimal {
    erp::d(setting(&st.db, "vat_rate").await.and_then(|v| v.as_f64()).unwrap_or(7.0))
}

async fn validate(tx: &mut Tx<'_>, r: &DocReq) -> AppResult<()> {
    if r.lines.is_empty() || r.lines.len() > 200 {
        return Err(bad("เอกสารต้องมีรายการ 1-200 บรรทัด"));
    }
    if !["exclusive", "inclusive", "none"].contains(&r.vat_mode.as_deref().unwrap_or("exclusive")) {
        return Err(bad("รูปแบบภาษีมูลค่าเพิ่มไม่ถูกต้อง"));
    }
    if r.discount < Decimal::ZERO || !(Decimal::ZERO..=dec!(15)).contains(&r.wht_rate) {
        return Err(bad("ส่วนลดต้องไม่ติดลบ และอัตราหัก ณ ที่จ่ายต้องอยู่ระหว่าง 0-15%"));
    }
    for l in &r.lines {
        if l.description.trim().is_empty() {
            return Err(bad("กรุณาระบุรายละเอียดทุกบรรทัด"));
        }
        if l.qty <= Decimal::ZERO || l.unit_price < Decimal::ZERO || !(Decimal::ZERO..=dec!(100)).contains(&l.discount_pct) {
            return Err(bad("จำนวนต้องมากกว่า 0 ราคาไม่ติดลบ และส่วนลด 0-100%"));
        }
        if l.qty * l.unit_price >= dec!(1000000000000) {
            return Err(bad("จำนวนเงินสูงเกินไป"));
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

async fn customer_snapshot(tx: &mut Tx<'_>, id: i64) -> AppResult<(String, Option<String>, Option<String>, Option<String>, i32)> {
    let c: Option<(String, Option<String>, Option<String>, Option<String>, Option<String>, i32)> =
        sqlx::query_as("SELECT name, company, tax_id, branch_code, address, credit_days FROM customers WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;
    let (name, company, tax, branch, addr, days) = c.ok_or_else(|| bad("ไม่พบลูกค้า"))?;
    let display = company.filter(|c| !c.trim().is_empty()).unwrap_or(name);
    let branch = match branch.as_deref().map(str::trim) {
        Some("") | None => tax.as_ref().map(|_| "สำนักงานใหญ่".to_string()),
        Some("00000") => Some("สำนักงานใหญ่".into()),
        Some(b) => Some(format!("สาขา {b}")),
    };
    Ok((display, tax, branch, addr, days))
}

async fn write_lines(tx: &mut Tx<'_>, id: i64, ls: &[LineReq]) -> AppResult<()> {
    sqlx::query("DELETE FROM sales_doc_lines WHERE doc_id = ?").bind(id).execute(&mut **tx).await?;
    for (i, l) in ls.iter().enumerate() {
        sqlx::query(
            "INSERT INTO sales_doc_lines (doc_id, product_id, description, qty, unit, unit_price, discount_pct, vat_exempt, amount, sort)
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

async fn save_totals(tx: &mut Tx<'_>, id: i64, t: &Totals) -> AppResult<()> {
    sqlx::query("UPDATE sales_docs SET subtotal = ?, discount = ?, vat_base = ?, exempt_amount = ?, vat_amount = ?, total = ?, wht_amount = ? WHERE id = ?")
        .bind(t.subtotal)
        .bind(t.discount)
        .bind(t.vat_base)
        .bind(t.exempt)
        .bind(t.vat)
        .bind(t.total)
        .bind(t.wht)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn create(State(st): State<AppState>, u: AuthUser, Json(r): Json<DocReq>) -> AppResult<Json<Value>> {
    u.need("sales.manage")?;
    let doc_type = r.doc_type.as_deref().unwrap_or("quotation");
    if !["quotation", "invoice"].contains(&doc_type) {
        return Err(bad("สร้างได้เฉพาะใบเสนอราคาหรือใบแจ้งหนี้ (ใบเสร็จออกจากการรับชำระ)"));
    }
    let date = r.doc_date.unwrap_or_else(|| local_today(st.cfg.tz_offset_min));
    let rate = vat_rate(&st).await;
    let mut tx = st.db.begin().await?;
    validate(&mut tx, &r).await?;
    let (name, tax, branch, addr, credit) = customer_snapshot(&mut tx, r.customer_id).await?;
    let vat_mode = r.vat_mode.clone().unwrap_or_else(|| "exclusive".into());
    let due = r.due_date.unwrap_or(date + Duration::days(if doc_type == "quotation" { 30 } else { credit.max(0) as i64 }));
    // quotations are numbered right away; tax invoices only when issued (no gaps)
    let no = if doc_type == "quotation" { Some(erp::next_doc_no(&mut tx, "QT", date).await?) } else { None };
    let wh = match r.warehouse_id {
        Some(w) => w,
        None => default_warehouse(&mut *tx).await?,
    };
    let id = sqlx::query(
        "INSERT INTO sales_docs (doc_type, doc_no, doc_date, due_date, customer_id, customer_name, customer_tax_id, customer_branch,
            customer_address, deal_id, salesperson_employee_id, warehouse_id, vat_mode, vat_rate, wht_rate, status, note, created_by)
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(doc_type)
    .bind(&no)
    .bind(date)
    .bind(due)
    .bind(r.customer_id)
    .bind(&name)
    .bind(&tax)
    .bind(&branch)
    .bind(&addr)
    .bind(r.deal_id)
    .bind(r.salesperson_employee_id)
    .bind(wh)
    .bind(&vat_mode)
    .bind(if vat_mode == "none" { Decimal::ZERO } else { rate })
    .bind(r.wht_rate)
    .bind(if doc_type == "quotation" { "open" } else { "draft" })
    .bind(r.note.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(u.id())
    .execute(&mut *tx)
    .await?
    .last_insert_id() as i64;
    write_lines(&mut tx, id, &r.lines).await?;
    let t = totals(&r.lines, r.discount, &vat_mode, rate, r.wht_rate);
    save_totals(&mut tx, id, &t).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "create", "sales_doc", id, json!({ "type": doc_type, "doc_no": no, "total": t.total })).await;
    Ok(Json(json!({ "id": id, "doc_no": no, "total": t.total })))
}

async fn update(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<DocReq>) -> AppResult<Json<Value>> {
    u.need("sales.manage")?;
    let rate = vat_rate(&st).await;
    let mut tx = st.db.begin().await?;
    let d = load(&mut *tx, id).await?;
    let editable = (d.doc_type == "quotation" && d.status == "open") || (d.doc_type == "invoice" && d.status == "draft");
    if !editable {
        return Err(bad("แก้ไขได้เฉพาะใบเสนอราคาที่ยังเปิดอยู่ หรือใบแจ้งหนี้ฉบับร่าง"));
    }
    validate(&mut tx, &r).await?;
    let (name, tax, branch, addr, _) = customer_snapshot(&mut tx, r.customer_id).await?;
    let vat_mode = r.vat_mode.clone().unwrap_or_else(|| "exclusive".into());
    sqlx::query(
        "UPDATE sales_docs SET doc_date = ?, due_date = ?, customer_id = ?, customer_name = ?, customer_tax_id = ?, customer_branch = ?,
            customer_address = ?, deal_id = ?, salesperson_employee_id = ?, warehouse_id = COALESCE(?, warehouse_id), vat_mode = ?, vat_rate = ?,
            wht_rate = ?, note = ? WHERE id = ?",
    )
    .bind(r.doc_date.unwrap_or(d.doc_date))
    .bind(r.due_date.or(d.due_date))
    .bind(r.customer_id)
    .bind(&name)
    .bind(&tax)
    .bind(&branch)
    .bind(&addr)
    .bind(r.deal_id)
    .bind(r.salesperson_employee_id)
    .bind(r.warehouse_id)
    .bind(&vat_mode)
    .bind(if vat_mode == "none" { Decimal::ZERO } else { rate })
    .bind(r.wht_rate)
    .bind(r.note.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(id)
    .execute(&mut *tx)
    .await?;
    write_lines(&mut tx, id, &r.lines).await?;
    let t = totals(&r.lines, r.discount, &vat_mode, rate, r.wht_rate);
    save_totals(&mut tx, id, &t).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "update", "sales_doc", id, json!({ "total": t.total })).await;
    Ok(Json(json!({ "ok": true, "total": t.total })))
}

async fn remove(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("sales.manage")?;
    let d = load(&st.db, id).await?;
    let deletable = (d.doc_type == "quotation" && d.status != "converted") || (d.doc_type == "invoice" && d.status == "draft");
    if !deletable {
        return Err(bad("ลบได้เฉพาะใบเสนอราคาที่ยังไม่แปลง หรือใบแจ้งหนี้ฉบับร่าง (เอกสารที่ออกแล้วให้ยกเลิกแทน)"));
    }
    sqlx::query("DELETE FROM sales_docs WHERE id = ?").bind(id).execute(&st.db).await?;
    audit(&st, Some(&u), "delete", "sales_doc", id, json!({ "doc_no": d.doc_no })).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct StatusReq {
    status: String,
}

async fn set_status(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<StatusReq>) -> AppResult<Json<Value>> {
    u.need("sales.manage")?;
    if !["open", "accepted", "rejected"].contains(&r.status.as_str()) {
        return Err(bad("สถานะไม่ถูกต้อง"));
    }
    let n = sqlx::query("UPDATE sales_docs SET status = ? WHERE id = ? AND doc_type = 'quotation' AND status IN ('open', 'accepted', 'rejected')")
        .bind(&r.status)
        .bind(id)
        .execute(&st.db)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(bad("เปลี่ยนสถานะได้เฉพาะใบเสนอราคาที่ยังไม่แปลงเป็นใบแจ้งหนี้"));
    }
    audit(&st, Some(&u), "status", "sales_doc", id, json!({ "status": r.status })).await;
    Ok(Json(json!({ "ok": true })))
}

/// Quotation -> draft invoice with the same customer and lines.
async fn convert(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("sales.manage")?;
    let today = local_today(st.cfg.tz_offset_min);
    let mut tx = st.db.begin().await?;
    let q = load(&mut *tx, id).await?;
    if q.doc_type != "quotation" || !["open", "accepted"].contains(&q.status.as_str()) {
        return Err(bad("แปลงได้เฉพาะใบเสนอราคาที่เปิดอยู่หรือลูกค้าตอบรับแล้ว"));
    }
    let (_, _, _, _, credit) = customer_snapshot(&mut tx, q.customer_id).await?;
    let inv = sqlx::query(
        "INSERT INTO sales_docs (doc_type, doc_date, due_date, customer_id, customer_name, customer_tax_id, customer_branch, customer_address,
            reference_id, deal_id, salesperson_employee_id, warehouse_id, vat_mode, vat_rate, subtotal, discount, vat_base, exempt_amount,
            vat_amount, total, wht_rate, wht_amount, status, note, created_by)
         SELECT 'invoice', ?, ?, customer_id, customer_name, customer_tax_id, customer_branch, customer_address, id, deal_id,
            salesperson_employee_id, warehouse_id, vat_mode, vat_rate, subtotal, discount, vat_base, exempt_amount, vat_amount, total,
            wht_rate, wht_amount, 'draft', note, ? FROM sales_docs WHERE id = ?",
    )
    .bind(today)
    .bind(today + Duration::days(credit.max(0) as i64))
    .bind(u.id())
    .bind(id)
    .execute(&mut *tx)
    .await?
    .last_insert_id() as i64;
    sqlx::query(
        "INSERT INTO sales_doc_lines (doc_id, product_id, description, qty, unit, unit_price, discount_pct, vat_exempt, amount, sort)
         SELECT ?, product_id, description, qty, unit, unit_price, discount_pct, vat_exempt, amount, sort FROM sales_doc_lines WHERE doc_id = ?",
    )
    .bind(inv)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE sales_docs SET status = 'converted' WHERE id = ?").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "convert", "sales_doc", id, json!({ "invoice_id": inv })).await;
    Ok(Json(json!({ "id": inv })))
}

// ---------------------------------------------------------------- issue / pay / void

/// Draft invoice -> issued tax invoice: number, stock issue, revenue / VAT / AR posting.
async fn issue(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("sales.manage")?;
    let mut tx = st.db.begin().await?;
    let d: DocRow = sqlx::query_as(&format!("{DOC_SELECT} WHERE d.id = ? FOR UPDATE")).bind(id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    if d.doc_type != "invoice" || d.status != "draft" {
        return Err(bad("ออกได้เฉพาะใบแจ้งหนี้ฉบับร่าง"));
    }
    erp::check_open(&mut tx, d.doc_date).await?;
    let ls = lines(&mut *tx, id).await?;
    let reqs: Vec<LineReq> = ls
        .iter()
        .map(|l| LineReq {
            product_id: l.product_id,
            description: l.description.clone(),
            qty: l.qty,
            unit: l.unit.clone(),
            unit_price: l.unit_price,
            discount_pct: l.discount_pct,
            vat_exempt: l.vat_exempt,
        })
        .collect();
    let t = totals(&reqs, d.discount, &d.vat_mode, d.vat_rate, d.wht_rate);
    let no = erp::next_doc_no(&mut tx, "IV", d.doc_date).await?;
    // revenue per account: product's own revenue account, else sales (goods) / service revenue
    let mut lines_out: Vec<Line> = vec![Line::dr(Acc::Role("ar"), t.total).party("customer", d.customer_id).memo(format!("{no} {}", d.customer_name))];
    let mut cogs = Decimal::ZERO;
    let wh = match d.warehouse_id {
        Some(w) => w,
        None => default_warehouse(&mut *tx).await?,
    };
    for (l, net) in ls.iter().zip(&t.net_lines) {
        let (acc, track) = match l.product_id {
            Some(p) => {
                let (rev, kind, track): (Option<i64>, String, bool) =
                    sqlx::query_as("SELECT revenue_account_id, kind, track_stock FROM products WHERE id = ?").bind(p).fetch_one(&mut *tx).await?;
                (rev.map(Acc::Id).unwrap_or(Acc::Role(if kind == "service" { "service_revenue" } else { "sales" })), track)
            }
            None => (Acc::Role("sales"), false),
        };
        lines_out.push(Line::cr(acc, *net).memo(l.description.chars().take(120).collect::<String>()));
        if track {
            cogs += stock_out(
                &mut tx,
                Move { date: d.doc_date, product: l.product_id.unwrap(), warehouse: wh, qty: l.qty, source: "sales_invoice", source_id: Some(id), note: Some(no.clone()), user: Some(u.id()) },
            )
            .await?;
        }
    }
    lines_out.push(Line::cr(Acc::Role("vat_output"), t.vat).memo(format!("ภาษีขาย {no}")));
    if cogs > Decimal::ZERO {
        lines_out.push(Line::dr(Acc::Role("cogs"), cogs).memo(format!("ต้นทุนขาย {no}")));
        lines_out.push(Line::cr(Acc::Role("inventory"), cogs));
    }
    let je = erp::post(&mut tx, d.doc_date, &format!("ขาย {no} {}", d.customer_name), "sales_invoice", Some(id), lines_out, Some(u.id())).await?;
    sqlx::query("UPDATE sales_docs SET doc_no = ?, status = 'issued', issued_at = UTC_TIMESTAMP(), journal_entry_id = ? WHERE id = ?")
        .bind(&no)
        .bind(je)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    save_totals(&mut tx, id, &t).await?;
    if let Some(q) = d.reference_id {
        sqlx::query("UPDATE sales_docs SET status = 'converted' WHERE id = ? AND doc_type = 'quotation'").bind(q).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    audit(&st, Some(&u), "issue", "sales_doc", id, json!({ "doc_no": no, "total": t.total })).await;
    Ok(Json(json!({ "ok": true, "doc_no": no, "journal_entry_id": je })))
}

#[derive(Deserialize)]
struct PaymentReq {
    pay_date: Option<NaiveDate>,
    amount: Decimal,
    #[serde(default)]
    wht_amount: Decimal,
    method: Option<String>,
    account_id: Option<i64>,
    reference: Option<String>,
}

fn outstanding(d: &DocRow) -> Decimal {
    d.total - d.paid_amount
}

/// Records money received (plus tax withheld by the customer) and issues a receipt.
async fn add_payment(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<PaymentReq>) -> AppResult<Json<Value>> {
    u.need("sales.manage")?;
    let method = r.method.as_deref().unwrap_or("transfer");
    if !["transfer", "cash", "cheque", "card"].contains(&method) {
        return Err(bad("วิธีชำระเงินไม่ถูกต้อง"));
    }
    if r.amount < Decimal::ZERO || r.wht_amount < Decimal::ZERO || (r.amount + r.wht_amount).is_zero() {
        return Err(bad("จำนวนเงินต้องมากกว่า 0"));
    }
    let date = r.pay_date.unwrap_or_else(|| local_today(st.cfg.tz_offset_min));
    let mut tx = st.db.begin().await?;
    let d: DocRow = sqlx::query_as(&format!("{DOC_SELECT} WHERE d.id = ? FOR UPDATE")).bind(id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    if d.doc_type != "invoice" || !["issued", "partial"].contains(&d.status.as_str()) {
        return Err(bad("รับชำระได้เฉพาะใบแจ้งหนี้ที่ออกแล้วและยังชำระไม่ครบ"));
    }
    let settle = r2(r.amount) + r2(r.wht_amount);
    if settle > outstanding(&d) {
        return Err(bad(format!("ยอดรับชำระ ({}) มากกว่ายอดค้าง ({})", fmt_money(settle), fmt_money(outstanding(&d)))));
    }
    let account = match r.account_id {
        Some(a) => {
            let t: Option<String> = sqlx::query_scalar("SELECT type FROM accounts WHERE id = ? AND is_active = 1").bind(a).fetch_optional(&mut *tx).await?;
            if t.as_deref() != Some("asset") {
                return Err(bad("บัญชีรับเงินต้องเป็นบัญชีเงินสด / ธนาคาร"));
            }
            a
        }
        None => erp::account_id(&mut tx, &Acc::Role(if method == "cash" { "cash" } else { "bank" })).await?,
    };
    // the receipt document
    let rc_no = erp::next_doc_no(&mut tx, "RC", date).await?;
    let receipt = sqlx::query(
        "INSERT INTO sales_docs (doc_type, doc_no, doc_date, customer_id, customer_name, customer_tax_id, customer_branch, customer_address,
            reference_id, vat_mode, vat_rate, subtotal, total, wht_amount, paid_amount, status, created_by, issued_at)
         VALUES ('receipt', ?, ?, ?, ?, ?, ?, ?, ?, 'none', 0, ?, ?, ?, ?, 'issued', ?, UTC_TIMESTAMP())",
    )
    .bind(&rc_no)
    .bind(date)
    .bind(d.customer_id)
    .bind(&d.customer_name)
    .bind(&d.customer_tax_id)
    .bind(&d.customer_branch)
    .bind(&d.customer_address)
    .bind(id)
    .bind(settle)
    .bind(settle)
    .bind(r2(r.wht_amount))
    .bind(r2(r.amount))
    .bind(u.id())
    .execute(&mut *tx)
    .await?
    .last_insert_id() as i64;
    let invoice_no = d.doc_no.clone().unwrap_or_default();
    let je = erp::post(
        &mut tx,
        date,
        &format!("รับชำระ {invoice_no} ({rc_no}) {}", d.customer_name),
        "sales_payment",
        Some(receipt),
        vec![
            Line::dr(Acc::Id(account), r2(r.amount)).memo(r.reference.clone().unwrap_or_default()),
            Line::dr(Acc::Role("wht_receivable"), r2(r.wht_amount)).memo(format!("ลูกค้าหัก ณ ที่จ่าย {invoice_no}")),
            Line::cr(Acc::Role("ar"), settle).party("customer", d.customer_id),
        ],
        Some(u.id()),
    )
    .await?;
    sqlx::query(
        "INSERT INTO sales_payments (invoice_id, receipt_id, pay_date, amount, wht_amount, method, account_id, reference, journal_entry_id, created_by)
         VALUES (?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(id)
    .bind(receipt)
    .bind(date)
    .bind(r2(r.amount))
    .bind(r2(r.wht_amount))
    .bind(method)
    .bind(account)
    .bind(r.reference.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(je)
    .bind(u.id())
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE sales_docs SET journal_entry_id = ? WHERE id = ?").bind(je).bind(receipt).execute(&mut *tx).await?;
    refresh_paid(&mut tx, id).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "payment", "sales_doc", id, json!({ "receipt_no": rc_no, "amount": r.amount, "wht": r.wht_amount })).await;
    Ok(Json(json!({ "ok": true, "receipt_id": receipt, "receipt_no": rc_no })))
}

async fn refresh_paid(tx: &mut Tx<'_>, invoice: i64) -> AppResult<()> {
    sqlx::query(
        "UPDATE sales_docs d SET paid_amount = (SELECT COALESCE(SUM(amount + wht_amount), 0) FROM sales_payments p WHERE p.invoice_id = d.id AND p.status = 'active'),
            status = CASE WHEN (SELECT COALESCE(SUM(amount + wht_amount), 0) FROM sales_payments p WHERE p.invoice_id = d.id AND p.status = 'active') >= d.total THEN 'paid'
                          WHEN (SELECT COUNT(*) FROM sales_payments p WHERE p.invoice_id = d.id AND p.status = 'active') > 0 THEN 'partial'
                          ELSE 'issued' END
          WHERE d.id = ?",
    )
    .bind(invoice)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn void_payment(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("sales.manage")?;
    let mut tx = st.db.begin().await?;
    let p: Option<(i64, Option<i64>, String)> = sqlx::query_as("SELECT invoice_id, receipt_id, status FROM sales_payments WHERE id = ? FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    let (invoice, receipt, status) = p.ok_or(AppError::NotFound)?;
    if status != "active" {
        return Err(bad("รายการรับชำระนี้ถูกยกเลิกแล้ว"));
    }
    if let Some(rc) = receipt {
        erp::void_entries(&mut tx, "sales_payment", rc).await?;
        sqlx::query("UPDATE sales_docs SET status = 'void' WHERE id = ?").bind(rc).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE sales_payments SET status = 'void' WHERE id = ?").bind(id).execute(&mut *tx).await?;
    refresh_paid(&mut tx, invoice).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "void_payment", "sales_doc", invoice, json!({ "payment_id": id })).await;
    Ok(Json(json!({ "ok": true })))
}

/// Cancels an issued invoice (number is kept, marked void): reverses stock and postings.
async fn void_invoice(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("sales.manage")?;
    let today = local_today(st.cfg.tz_offset_min);
    let mut tx = st.db.begin().await?;
    let d: DocRow = sqlx::query_as(&format!("{DOC_SELECT} WHERE d.id = ? FOR UPDATE")).bind(id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    if d.doc_type != "invoice" || d.status != "issued" {
        return Err(bad("ยกเลิกได้เฉพาะใบแจ้งหนี้ที่ออกแล้วและยังไม่มีการรับชำระ (ยกเลิกการรับชำระก่อน)"));
    }
    erp::void_entries(&mut tx, "sales_invoice", id).await?;
    reverse_moves(&mut tx, "sales_invoice", id, "sales_void", today, Some(u.id())).await?;
    sqlx::query("UPDATE sales_docs SET status = 'void' WHERE id = ?").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "void", "sales_doc", id, json!({ "doc_no": d.doc_no })).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- summary

async fn summary(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    u.need_any(&["sales.view", "sales.manage", "accounting.view"])?;
    let today = local_today(st.cfg.tz_offset_min);
    let month_start = chrono::Datelike::with_day(&today, 1).unwrap_or(today);
    let (month_sales,): (Decimal,) = sqlx::query_as(
        "SELECT COALESCE(SUM(total - vat_amount), 0) FROM sales_docs WHERE doc_type = 'invoice' AND status IN ('issued','partial','paid') AND doc_date >= ?",
    )
    .bind(month_start)
    .fetch_one(&st.db)
    .await?;
    let (ar, overdue, overdue_n): (Decimal, Decimal, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(total - paid_amount), 0),
                COALESCE(SUM(CASE WHEN due_date < ? THEN total - paid_amount END), 0),
                CAST(COALESCE(SUM(due_date < ?), 0) AS SIGNED)
           FROM sales_docs WHERE doc_type = 'invoice' AND status IN ('issued','partial')",
    )
    .bind(today)
    .bind(today)
    .fetch_one(&st.db)
    .await?;
    let (open_quotes, open_value): (i64, Decimal) =
        sqlx::query_as("SELECT COUNT(*), COALESCE(SUM(total), 0) FROM sales_docs WHERE doc_type = 'quotation' AND status IN ('open','accepted')")
            .fetch_one(&st.db)
            .await?;
    Ok(Json(json!({
        "month_sales": month_sales, "receivable": ar, "overdue": overdue, "overdue_count": overdue_n,
        "open_quotations": open_quotes, "open_quotation_value": open_value,
    })))
}

// ---------------------------------------------------------------- printing

async fn print(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Html<String>> {
    u.need_any(&["sales.view", "sales.manage", "accounting.view"])?;
    let d = load(&st.db, id).await?;
    if d.doc_type == "invoice" && d.status == "draft" {
        return Err(bad("ใบแจ้งหนี้ฉบับร่างยังพิมพ์เป็นใบกำกับภาษีไม่ได้ กรุณากดออกเอกสารก่อน"));
    }
    let co = company(&st).await;
    let vat_registered = setting(&st.db, "vat_registered").await.and_then(|v| v.as_bool()).unwrap_or(true);
    let (title, title_en) = match d.doc_type.as_str() {
        "quotation" => ("ใบเสนอราคา", "Quotation"),
        "receipt" => ("ใบเสร็จรับเงิน", "Receipt"),
        _ if vat_registered && d.vat_mode != "none" => ("ใบแจ้งหนี้ / ใบกำกับภาษี", "Invoice / Tax Invoice"),
        _ => ("ใบแจ้งหนี้", "Invoice"),
    };
    let s = |v: &Value| esc(v.as_str().unwrap_or(""));
    let mut body_rows = String::new();
    let mut totals_rows = String::new();
    if d.doc_type == "receipt" {
        let inv = match d.reference_id {
            Some(r) => Some(load(&st.db, r).await?),
            None => None,
        };
        let (cash, wht): (Decimal, Decimal) = (d.paid_amount, d.wht_amount);
        body_rows.push_str(&format!(
            "<tr><td class=\"c\">1</td><td>ชำระตาม{} เลขที่ {} ลงวันที่ {}</td><td class=\"r\">{}</td></tr>",
            if inv.as_ref().is_some_and(|i| i.vat_mode != "none") { "ใบแจ้งหนี้/ใบกำกับภาษี" } else { "ใบแจ้งหนี้" },
            esc(inv.as_ref().and_then(|i| i.doc_no.as_deref()).unwrap_or("-")),
            inv.as_ref().map(|i| th_date(i.doc_date)).unwrap_or_default(),
            fmt_money(d.total),
        ));
        totals_rows.push_str(&format!(
            "<tr><td>ยอดชำระ</td><td class=\"r\">{}</td></tr><tr><td>ภาษีหัก ณ ที่จ่าย (ลูกค้าหักไว้)</td><td class=\"r\">{}</td></tr><tr class=\"tot\"><td>รับเงินสุทธิ</td><td class=\"r big\">{}</td></tr>",
            fmt_money(d.total), fmt_money(wht), fmt_money(cash)
        ));
    } else {
        for (i, l) in lines(&st.db, id).await?.iter().enumerate() {
            body_rows.push_str(&format!(
                "<tr><td class=\"c\">{}</td><td>{}{}</td><td class=\"r\">{} {}</td><td class=\"r\">{}</td><td class=\"r\">{}</td><td class=\"r\">{}</td></tr>",
                i + 1,
                esc(&l.description),
                if l.vat_exempt && d.vat_mode != "none" { " <span class=\"muted\">(ยกเว้นภาษี)</span>" } else { "" },
                l.qty.normalize(),
                esc(l.unit.as_deref().unwrap_or("")),
                fmt_money(l.unit_price),
                if l.discount_pct.is_zero() { String::new() } else { format!("{}%", l.discount_pct.normalize()) },
                fmt_money(l.amount),
            ));
        }
        totals_rows.push_str(&format!("<tr><td>รวมเงิน</td><td class=\"r\">{}</td></tr>", fmt_money(d.subtotal)));
        if !d.discount.is_zero() {
            totals_rows.push_str(&format!("<tr><td>ส่วนลด</td><td class=\"r\">-{}</td></tr>", fmt_money(d.discount)));
        }
        if d.vat_mode != "none" {
            if !d.exempt_amount.is_zero() {
                totals_rows.push_str(&format!("<tr><td>มูลค่าที่ได้รับยกเว้นภาษี</td><td class=\"r\">{}</td></tr>", fmt_money(d.exempt_amount)));
            }
            totals_rows.push_str(&format!(
                "<tr><td>มูลค่าก่อนภาษี</td><td class=\"r\">{}</td></tr><tr><td>ภาษีมูลค่าเพิ่ม {}%</td><td class=\"r\">{}</td></tr>",
                fmt_money(d.vat_base),
                d.vat_rate.normalize(),
                fmt_money(d.vat_amount)
            ));
        }
        totals_rows.push_str(&format!("<tr class=\"tot\"><td>จำนวนเงินรวมทั้งสิ้น</td><td class=\"r big\">{}</td></tr>", fmt_money(d.total)));
        if !d.wht_amount.is_zero() {
            totals_rows.push_str(&format!(
                "<tr><td>หัก ณ ที่จ่าย {}% (ถ้าลูกค้าเป็นนิติบุคคล)</td><td class=\"r\">{}</td></tr><tr><td>ยอดชำระสุทธิ</td><td class=\"r\">{}</td></tr>",
                d.wht_rate.normalize(),
                fmt_money(d.wht_amount),
                fmt_money(d.total - d.wht_amount)
            ));
        }
    }
    let head_cols = if d.doc_type == "receipt" {
        "<th style=\"width:40px\">ลำดับ</th><th>รายการ</th><th class=\"r\" style=\"width:140px\">จำนวนเงิน</th>"
    } else {
        "<th style=\"width:40px\">ลำดับ</th><th>รายการ</th><th class=\"r\">จำนวน</th><th class=\"r\">ราคา/หน่วย</th><th class=\"r\">ส่วนลด</th><th class=\"r\">จำนวนเงิน</th>"
    };
    let terms = setting(&st.db, "sales_terms").await.and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
    let status_note = if d.status == "void" { "<div class=\"stamp\" style=\"color:#c00;border-color:#c00;font-size:16px\">ยกเลิก (VOID)</div>" } else { "" };
    audit(&st, Some(&u), "print", "sales_doc", id, json!({ "doc_no": d.doc_no })).await;
    let mut pages = String::new();
    // tax invoices / receipts: original for the customer + copy for the seller
    let copies: &[&str] = if d.doc_type == "quotation" { &[""] } else { &["ต้นฉบับ (ลูกค้า)", "สำเนา (ผู้ขาย)"] };
    for copy in copies {
        pages.push_str(&format!(
            r#"<div class="page">
<div class="head"><div>{logo}<h1>{org}</h1><div class="muted">{addr}</div><div class="muted">เลขประจำตัวผู้เสียภาษี {taxid} · {branch}</div></div>
<div class="r"><div class="big">{title}</div><div class="muted">{title_en}</div>{copy}{status}</div></div>
<div class="grid2"><div class="box"><b>ลูกค้า</b><br>{cust}<br>{caddr}<br>เลขประจำตัวผู้เสียภาษี {ctax} {cbranch}</div>
<div class="box">เลขที่ <b>{no}</b><br>วันที่ {date}{due}{refline}</div></div>
<table><thead><tr>{head_cols}</tr></thead><tbody>{rows}</tbody></table>
<div class="grid2" style="margin-top:10px"><div class="box" style="align-self:start">({words})<br><span class="muted">{note}</span><br><span class="muted small">{terms}</span></div>
<table style="margin-top:0">{tot}</table></div>
<div class="sign"><div>{sign_left}</div><div>ผู้มีอำนาจลงนาม</div></div>
</div>"#,
            logo = co["logo_url"].as_str().filter(|l| l.starts_with("https://") || l.starts_with("data:image/")).map(|l| format!("<img src=\"{}\" alt=\"\" style=\"height:40px\">", esc(l))).unwrap_or_default(),
            org = s(&co["name"]),
            addr = s(&co["address"]),
            taxid = s(&co["tax_id"]),
            branch = s(&co["branch"]),
            title = title,
            title_en = title_en,
            copy = if copy.is_empty() { String::new() } else { format!("<div class=\"stamp\">{copy}</div>") },
            status = status_note,
            cust = esc(&d.customer_name),
            caddr = esc(d.customer_address.as_deref().unwrap_or("")),
            ctax = esc(d.customer_tax_id.as_deref().unwrap_or("-")),
            cbranch = esc(d.customer_branch.as_deref().unwrap_or("")),
            no = esc(d.doc_no.as_deref().unwrap_or("-")),
            date = th_date(d.doc_date),
            due = d.due_date.filter(|_| d.doc_type != "receipt").map(|x| format!("<br>{} {}", if d.doc_type == "quotation" { "ยืนราคาถึง" } else { "ครบกำหนดชำระ" }, th_date(x))).unwrap_or_default(),
            refline = d.reference_no.as_deref().map(|r| format!("<br>อ้างอิง {}", esc(r))).unwrap_or_default(),
            head_cols = head_cols,
            rows = body_rows,
            words = erp::baht_text(d.total),
            note = esc(d.note.as_deref().unwrap_or("")),
            terms = if d.doc_type == "quotation" { esc(&terms) } else { String::new() },
            tot = totals_rows,
            sign_left = if d.doc_type == "receipt" { "ผู้รับเงิน" } else if d.doc_type == "quotation" { "ผู้เสนอราคา" } else { "ผู้รับสินค้า / บริการ" },
        ));
    }
    Ok(Html(format!(
        r#"<!doctype html><html lang="th"><head><meta charset="utf-8"><title>{title} {no}</title><style>{css} .page{{page-break-after:always}} .small{{font-size:11px}}</style></head><body>
<div class="toolbar"><button onclick="window.print()">พิมพ์ / บันทึกเป็น PDF (A4)</button></div>{pages}</body></html>"#,
        title = title,
        no = esc(d.doc_no.as_deref().unwrap_or("")),
        css = PRINT_CSS,
        pages = pages
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(qty: Decimal, price: Decimal, exempt: bool) -> LineReq {
        LineReq { product_id: None, description: "x".into(), qty, unit: None, unit_price: price, discount_pct: Decimal::ZERO, vat_exempt: exempt }
    }

    #[test]
    fn exclusive_vat() {
        let t = totals(&[l(dec!(2), dec!(1000), false), l(dec!(1), dec!(500), false)], dec!(0), "exclusive", dec!(7), dec!(3));
        assert_eq!(t.subtotal, dec!(2500));
        assert_eq!(t.vat, dec!(175));
        assert_eq!(t.total, dec!(2675));
        assert_eq!(t.wht, dec!(75));
    }

    #[test]
    fn inclusive_vat_with_discount_and_exempt_line() {
        // 1,070 incl. VAT + 200 exempt, discount 127 spread by amount
        let t = totals(&[l(dec!(1), dec!(1070), false), l(dec!(1), dec!(200), true)], dec!(127), "inclusive", dec!(7), dec!(0));
        assert_eq!(t.subtotal, dec!(1270));
        // taxable after discount: 1070 - 127*1070/1270 = 963.00 ; exempt 200 - 20 = 180
        assert_eq!(t.total, dec!(1143));
        assert_eq!(t.vat, dec!(63.00));
        assert_eq!(t.vat_base, dec!(900.00));
        assert_eq!(t.exempt, dec!(180.00));
        assert_eq!(t.net_lines.iter().sum::<Decimal>(), t.total - t.vat);
    }

    #[test]
    fn rounding_remainder_kept() {
        let t = totals(&[l(dec!(1), dec!(10), false), l(dec!(1), dec!(10), false), l(dec!(1), dec!(10), false)], dec!(10), "exclusive", dec!(7), dec!(0));
        assert_eq!(t.net_lines.iter().sum::<Decimal>(), dec!(20));
        assert_eq!(t.vat, dec!(1.40));
    }
}
