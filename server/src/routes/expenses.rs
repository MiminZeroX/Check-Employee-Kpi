//! Expense claims: employees claim back what they paid for the company (with receipts),
//! the line manager / HR approves (posted to the expense accounts), and the money is paid
//! either with the next salary or separately by finance.

use axum::{
    extract::{Path, Query, State},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::{audit, AuthUser},
    erp::{self, r2, Acc, Line},
    error::{bad, AppError, AppResult},
    routes::hr::managed_employees,
    state::AppState,
    util::local_today,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/expense-categories", get(list_categories).post(create_category))
        .route("/expense-categories/{id}", put(update_category))
        .route("/expenses", get(list).post(create))
        .route("/expenses/{id}", get(detail).put(update).delete(remove))
        .route("/expenses/{id}/submit", post(submit))
        .route("/expenses/{id}/decide", post(decide))
        .route("/expenses/{id}/pay", post(pay))
}

// ---------------------------------------------------------------- categories

#[derive(Serialize, sqlx::FromRow)]
struct Category {
    id: i64,
    name: String,
    account_id: i64,
    account_code: String,
    account_name: String,
    is_active: bool,
}

async fn list_categories(State(st): State<AppState>, _u: AuthUser) -> AppResult<Json<Vec<Category>>> {
    Ok(Json(
        sqlx::query_as(
            "SELECT c.id, c.name, c.account_id, a.code AS account_code, a.name AS account_name, c.is_active
               FROM expense_categories c JOIN accounts a ON a.id = c.account_id ORDER BY c.name",
        )
        .fetch_all(&st.db)
        .await?,
    ))
}

#[derive(Deserialize)]
struct CategoryReq {
    name: String,
    account_id: i64,
    is_active: Option<bool>,
}

async fn check_expense_account(st: &AppState, id: i64) -> AppResult<()> {
    let t: Option<String> = sqlx::query_scalar("SELECT type FROM accounts WHERE id = ? AND is_active = 1").bind(id).fetch_optional(&st.db).await?;
    match t.as_deref() {
        Some("expense") | Some("asset") => Ok(()),
        _ => Err(bad("หมวดค่าใช้จ่ายต้องผูกกับบัญชีค่าใช้จ่าย (หรือสินทรัพย์)")),
    }
}

async fn create_category(State(st): State<AppState>, u: AuthUser, Json(r): Json<CategoryReq>) -> AppResult<Json<Value>> {
    u.need_any(&["expenses.manage", "accounting.manage"])?;
    if r.name.trim().is_empty() {
        return Err(bad("กรุณาระบุชื่อหมวด"));
    }
    check_expense_account(&st, r.account_id).await?;
    let id = sqlx::query("INSERT INTO expense_categories (name, account_id, is_active) VALUES (?,?,?)")
        .bind(r.name.trim())
        .bind(r.account_id)
        .bind(r.is_active.unwrap_or(true))
        .execute(&st.db)
        .await?
        .last_insert_id();
    audit(&st, Some(&u), "create", "expense_category", id, json!({ "name": r.name })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_category(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<CategoryReq>) -> AppResult<Json<Value>> {
    u.need_any(&["expenses.manage", "accounting.manage"])?;
    if r.name.trim().is_empty() {
        return Err(bad("กรุณาระบุชื่อหมวด"));
    }
    check_expense_account(&st, r.account_id).await?;
    sqlx::query("UPDATE expense_categories SET name = ?, account_id = ?, is_active = ? WHERE id = ?")
        .bind(r.name.trim())
        .bind(r.account_id)
        .bind(r.is_active.unwrap_or(true))
        .bind(id)
        .execute(&st.db)
        .await?;
    audit(&st, Some(&u), "update", "expense_category", id, json!({ "name": r.name })).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- claims

#[derive(Serialize, sqlx::FromRow)]
struct ClaimRow {
    id: i64,
    claim_no: String,
    employee_id: i64,
    employee_code: String,
    employee_name: String,
    department: Option<String>,
    title: String,
    claim_date: NaiveDate,
    total: Decimal,
    status: String,
    pay_method: String,
    approver_name: Option<String>,
    approver_note: Option<String>,
    decided_at: Option<DateTime<Utc>>,
    paid_at: Option<DateTime<Utc>>,
    payroll_run_no: Option<String>,
    items: i64,
    receipts: i64,
    created_at: DateTime<Utc>,
}

const CLAIM_SELECT: &str = "SELECT c.id, c.claim_no, c.employee_id, e.employee_code, CONCAT(e.first_name, ' ', e.last_name) AS employee_name,
    d.name AS department, c.title, c.claim_date, c.total, c.status, c.pay_method, us.display_name AS approver_name, c.approver_note,
    c.decided_at, c.paid_at, pr.run_no AS payroll_run_no,
    (SELECT COUNT(*) FROM expense_items i WHERE i.claim_id = c.id) AS items,
    (SELECT COUNT(*) FROM documents doc WHERE doc.entity = 'expense' AND doc.entity_id = c.id) AS receipts, c.created_at
    FROM expense_claims c JOIN employees e ON e.id = c.employee_id LEFT JOIN departments d ON d.id = e.department_id
    LEFT JOIN users us ON us.id = c.approver_user_id LEFT JOIN payroll_runs pr ON pr.id = c.payroll_run_id";

#[derive(Deserialize)]
struct ListQ {
    /// mine | review (claims I may approve) | all
    scope: Option<String>,
    status: Option<String>,
}

/// Which employees' claims this user may see / approve (None = everyone).
async fn scope(st: &AppState, u: &AuthUser) -> AppResult<Option<std::collections::HashSet<i64>>> {
    if u.can("expenses.manage") {
        return Ok(None);
    }
    if u.can("expenses.approve") {
        return managed_employees(st, u).await;
    }
    Ok(Some(u.0.emp.into_iter().collect()))
}

async fn list(State(st): State<AppState>, u: AuthUser, Query(q): Query<ListQ>) -> AppResult<Json<Vec<ClaimRow>>> {
    let mine = q.scope.as_deref().unwrap_or("mine") == "mine";
    let mut sql = format!("{CLAIM_SELECT} WHERE 1=1");
    if mine {
        u.0.emp.ok_or_else(|| bad("บัญชีนี้ไม่ได้ผูกกับพนักงาน"))?;
        sql.push_str(" AND c.employee_id = ?");
    } else if !(u.can("expenses.approve") || u.can("expenses.manage")) {
        return Err(AppError::Forbidden);
    } else {
        // approvers never see colleagues' drafts
        sql.push_str(" AND c.status <> 'draft'");
    }
    if q.status.is_some() {
        sql.push_str(" AND c.status = ?");
    }
    sql.push_str(" ORDER BY c.id DESC LIMIT 1000");
    let mut qq = sqlx::query_as::<_, ClaimRow>(&sql);
    if mine {
        qq = qq.bind(u.0.emp);
    }
    if let Some(s) = &q.status {
        qq = qq.bind(s);
    }
    let mut rows = qq.fetch_all(&st.db).await?;
    if !mine {
        if let Some(allowed) = scope(&st, &u).await? {
            rows.retain(|r| allowed.contains(&r.employee_id));
        }
    }
    Ok(Json(rows))
}

#[derive(Serialize, sqlx::FromRow)]
struct ItemRow {
    id: i64,
    expense_date: NaiveDate,
    category_id: i64,
    category_name: String,
    description: String,
    amount: Decimal,
    vat_amount: Decimal,
    document_id: Option<i64>,
}

/// Owner, or an approver whose scope covers the employee. Others get 404.
async fn load_claim(st: &AppState, u: &AuthUser, id: i64) -> AppResult<ClaimRow> {
    let c: ClaimRow = sqlx::query_as(&format!("{CLAIM_SELECT} WHERE c.id = ?")).bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    if u.0.emp == Some(c.employee_id) {
        return Ok(c);
    }
    if c.status != "draft" && (u.can("expenses.approve") || u.can("expenses.manage")) {
        if let Some(allowed) = scope(st, u).await? {
            if !allowed.contains(&c.employee_id) {
                return Err(AppError::NotFound);
            }
        }
        return Ok(c);
    }
    Err(AppError::NotFound)
}

/// Used by the documents module: may this user see / attach receipts of claim `id`?
pub async fn may_access_receipts(st: &AppState, u: &AuthUser, id: i64, upload: bool) -> AppResult<bool> {
    match load_claim(st, u, id).await {
        Ok(c) if upload => Ok(u.0.emp == Some(c.employee_id) && ["draft", "submitted"].contains(&c.status.as_str())),
        Ok(_) => Ok(true),
        Err(AppError::NotFound) => Ok(false),
        Err(e) => Err(e),
    }
}

async fn detail(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let c = load_claim(&st, &u, id).await?;
    let items: Vec<ItemRow> = sqlx::query_as(
        "SELECT i.id, i.expense_date, i.category_id, cat.name AS category_name, i.description, i.amount, i.vat_amount, i.document_id
           FROM expense_items i JOIN expense_categories cat ON cat.id = i.category_id WHERE i.claim_id = ? ORDER BY i.expense_date, i.id",
    )
    .bind(id)
    .fetch_all(&st.db)
    .await?;
    let docs: Vec<(i64, String, String, i64)> =
        sqlx::query_as("SELECT id, title, file_name, size_bytes FROM documents WHERE entity = 'expense' AND entity_id = ? ORDER BY id")
            .bind(id)
            .fetch_all(&st.db)
            .await?;
    let can_decide = c.status == "submitted" && u.0.emp != Some(c.employee_id) && (u.can("expenses.approve") || u.can("expenses.manage"));
    Ok(Json(json!({
        "claim": c, "items": items, "can_decide": can_decide,
        "receipts": docs.into_iter().map(|d| json!({ "id": d.0, "title": d.1, "file_name": d.2, "size_bytes": d.3 })).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
struct ItemReq {
    expense_date: NaiveDate,
    category_id: i64,
    description: String,
    amount: Decimal,
    #[serde(default)]
    vat_amount: Decimal,
}

#[derive(Deserialize)]
struct ClaimReq {
    title: String,
    pay_method: Option<String>,
    items: Vec<ItemReq>,
}

async fn validate(st: &AppState, r: &ClaimReq) -> AppResult<Decimal> {
    if r.title.trim().is_empty() {
        return Err(bad("กรุณาระบุหัวข้อการเบิก"));
    }
    if r.items.is_empty() || r.items.len() > 100 {
        return Err(bad("ใบเบิกต้องมีรายการ 1-100 รายการ"));
    }
    if !["payroll", "cash"].contains(&r.pay_method.as_deref().unwrap_or("payroll")) {
        return Err(bad("วิธีรับเงินไม่ถูกต้อง"));
    }
    let today = local_today(st.cfg.tz_offset_min);
    for i in &r.items {
        if i.description.trim().is_empty() {
            return Err(bad("กรุณาระบุรายละเอียดทุกรายการ"));
        }
        if i.amount <= Decimal::ZERO || i.amount >= dec!(10000000) {
            return Err(bad("จำนวนเงินต้องมากกว่า 0"));
        }
        if i.vat_amount < Decimal::ZERO || i.vat_amount >= i.amount {
            return Err(bad("ภาษีมูลค่าเพิ่มต้องน้อยกว่ายอดรวมของรายการ"));
        }
        if i.expense_date > today || (today - i.expense_date).num_days() > 365 {
            return Err(bad("วันที่ใช้จ่ายต้องไม่เกินวันนี้และไม่เก่ากว่า 1 ปี"));
        }
        let ok: Option<bool> = sqlx::query_scalar("SELECT is_active FROM expense_categories WHERE id = ?").bind(i.category_id).fetch_optional(&st.db).await?;
        if ok != Some(true) {
            return Err(bad("หมวดค่าใช้จ่ายไม่ถูกต้อง"));
        }
    }
    Ok(r.items.iter().map(|i| r2(i.amount)).sum())
}

async fn write_items(tx: &mut erp::Tx<'_>, id: i64, items: &[ItemReq]) -> AppResult<()> {
    sqlx::query("DELETE FROM expense_items WHERE claim_id = ?").bind(id).execute(&mut **tx).await?;
    for i in items {
        sqlx::query("INSERT INTO expense_items (claim_id, expense_date, category_id, description, amount, vat_amount) VALUES (?,?,?,?,?,?)")
            .bind(id)
            .bind(i.expense_date)
            .bind(i.category_id)
            .bind(i.description.trim().chars().take(300).collect::<String>())
            .bind(r2(i.amount))
            .bind(r2(i.vat_amount))
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

async fn create(State(st): State<AppState>, u: AuthUser, Json(r): Json<ClaimReq>) -> AppResult<Json<Value>> {
    let emp = u.0.emp.ok_or_else(|| bad("บัญชีนี้ไม่ได้ผูกกับพนักงาน จึงเบิกค่าใช้จ่ายไม่ได้"))?;
    let total = validate(&st, &r).await?;
    let today = local_today(st.cfg.tz_offset_min);
    let mut tx = st.db.begin().await?;
    let no = erp::next_doc_no(&mut tx, "EX", today).await?;
    let id = sqlx::query("INSERT INTO expense_claims (claim_no, employee_id, title, claim_date, total, pay_method) VALUES (?,?,?,?,?,?)")
        .bind(&no)
        .bind(emp)
        .bind(r.title.trim())
        .bind(today)
        .bind(total)
        .bind(r.pay_method.as_deref().unwrap_or("payroll"))
        .execute(&mut *tx)
        .await?
        .last_insert_id() as i64;
    write_items(&mut tx, id, &r.items).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "create", "expense_claim", id, json!({ "claim_no": no, "total": total })).await;
    Ok(Json(json!({ "id": id, "claim_no": no, "total": total })))
}

async fn update(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<ClaimReq>) -> AppResult<Json<Value>> {
    let c = load_claim(&st, &u, id).await?;
    if u.0.emp != Some(c.employee_id) || c.status != "draft" {
        return Err(bad("แก้ไขได้เฉพาะใบเบิกของตนเองที่ยังเป็นฉบับร่าง"));
    }
    let total = validate(&st, &r).await?;
    let mut tx = st.db.begin().await?;
    sqlx::query("UPDATE expense_claims SET title = ?, total = ?, pay_method = ? WHERE id = ? AND status = 'draft'")
        .bind(r.title.trim())
        .bind(total)
        .bind(r.pay_method.as_deref().unwrap_or("payroll"))
        .bind(id)
        .execute(&mut *tx)
        .await?;
    write_items(&mut tx, id, &r.items).await?;
    tx.commit().await?;
    Ok(Json(json!({ "ok": true, "total": total })))
}

async fn remove(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let c = load_claim(&st, &u, id).await?;
    if u.0.emp != Some(c.employee_id) || c.status != "draft" {
        return Err(bad("ลบได้เฉพาะใบเบิกของตนเองที่ยังเป็นฉบับร่าง"));
    }
    sqlx::query("DELETE FROM expense_claims WHERE id = ? AND status = 'draft'").bind(id).execute(&st.db).await?;
    audit(&st, Some(&u), "delete", "expense_claim", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

async fn submit(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let c = load_claim(&st, &u, id).await?;
    if u.0.emp != Some(c.employee_id) {
        return Err(AppError::NotFound);
    }
    let r = sqlx::query("UPDATE expense_claims SET status = 'submitted' WHERE id = ? AND status = 'draft'").bind(id).execute(&st.db).await?;
    if r.rows_affected() == 0 {
        return Err(bad("ส่งได้เฉพาะใบเบิกฉบับร่าง"));
    }
    audit(&st, Some(&u), "submit", "expense_claim", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct DecideReq {
    approve: bool,
    note: Option<String>,
}

async fn decide(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<DecideReq>) -> AppResult<Json<Value>> {
    u.need_any(&["expenses.approve", "expenses.manage"])?;
    let c = load_claim(&st, &u, id).await?;
    if u.0.emp == Some(c.employee_id) {
        return Err(bad("อนุมัติใบเบิกของตนเองไม่ได้"));
    }
    if c.status != "submitted" {
        return Err(bad("ใบเบิกนี้ไม่ได้รอการอนุมัติ"));
    }
    let note = r.note.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if !r.approve && note.is_none() {
        return Err(bad("กรุณาระบุเหตุผลที่ไม่อนุมัติ"));
    }
    let today = local_today(st.cfg.tz_offset_min);
    let mut tx = st.db.begin().await?;
    let mut je = None;
    if r.approve {
        // expense (net of VAT) + input VAT, owed to the employee
        let items: Vec<(i64, Decimal, Decimal, String)> = sqlx::query_as(
            "SELECT cat.account_id, i.amount, i.vat_amount, i.description FROM expense_items i
               JOIN expense_categories cat ON cat.id = i.category_id WHERE i.claim_id = ?",
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
        let mut lines: Vec<Line> = items.iter().map(|(acc, amt, vat, desc)| Line::dr(Acc::Id(*acc), *amt - *vat).memo(desc.clone())).collect();
        lines.push(Line::dr(Acc::Role("vat_input"), items.iter().map(|i| i.2).sum()));
        lines.push(Line::cr(Acc::Role("employee_payable"), c.total).party("employee", c.employee_id).memo(format!("{} {}", c.claim_no, c.employee_name)));
        je = Some(erp::post(&mut tx, today, &format!("ใบเบิก {} {}", c.claim_no, c.title), "expense_claim", Some(id), lines, Some(u.id())).await?);
    }
    let upd = sqlx::query(
        "UPDATE expense_claims SET status = ?, approver_user_id = ?, approver_note = ?, decided_at = UTC_TIMESTAMP(), journal_entry_id = ?
          WHERE id = ? AND status = 'submitted'",
    )
    .bind(if r.approve { "approved" } else { "rejected" })
    .bind(u.id())
    .bind(note)
    .bind(je)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if upd.rows_affected() == 0 {
        return Err(AppError::Conflict("ใบเบิกถูกพิจารณาไปแล้ว".into()));
    }
    sqlx::query("INSERT INTO notifications (employee_id, kind, title, body) VALUES (?, 'expense_result', ?, ?)")
        .bind(c.employee_id)
        .bind(format!("ใบเบิก {} {}", c.claim_no, if r.approve { "ได้รับการอนุมัติ" } else { "ไม่ได้รับการอนุมัติ" }))
        .bind(note.unwrap_or(if c.pay_method == "payroll" { "จะจ่ายพร้อมเงินเดือนรอบถัดไป" } else { "" }))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    audit(&st, Some(&u), if r.approve { "approve" } else { "reject" }, "expense_claim", id, json!({ "total": c.total })).await;
    Ok(Json(json!({ "ok": true, "journal_entry_id": je })))
}

#[derive(Deserialize, Default)]
struct PayReq {
    pay_account_id: Option<i64>,
}

/// Finance pays an approved claim directly (cash / transfer).
async fn pay(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, body: Option<Json<PayReq>>) -> AppResult<Json<Value>> {
    u.need("expenses.manage")?;
    let req = body.map(|b| b.0).unwrap_or_default();
    let c = load_claim(&st, &u, id).await?;
    if c.status != "approved" || c.payroll_run_no.is_some() {
        return Err(bad("จ่ายได้เฉพาะใบเบิกที่อนุมัติแล้วและยังไม่อยู่ในรอบเงินเดือน"));
    }
    let acc = match req.pay_account_id {
        Some(a) => {
            let t: Option<String> = sqlx::query_scalar("SELECT type FROM accounts WHERE id = ? AND is_active = 1").bind(a).fetch_optional(&st.db).await?;
            if t.as_deref() != Some("asset") {
                return Err(bad("บัญชีที่ใช้จ่ายต้องเป็นบัญชีเงินสด / ธนาคาร"));
            }
            Acc::Id(a)
        }
        None => Acc::Role("bank"),
    };
    let today = local_today(st.cfg.tz_offset_min);
    let mut tx = st.db.begin().await?;
    let je = erp::post(
        &mut tx,
        today,
        &format!("จ่ายใบเบิก {} {}", c.claim_no, c.employee_name),
        "expense_payment",
        Some(id),
        vec![Line::dr(Acc::Role("employee_payable"), c.total).party("employee", c.employee_id), Line::cr(acc, c.total)],
        Some(u.id()),
    )
    .await?;
    let upd = sqlx::query("UPDATE expense_claims SET status = 'paid', paid_at = UTC_TIMESTAMP(), payment_entry_id = ? WHERE id = ? AND status = 'approved' AND payroll_run_id IS NULL")
        .bind(je)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if upd.rows_affected() == 0 {
        return Err(AppError::Conflict("ใบเบิกนี้ถูกจ่ายหรือนำเข้ารอบเงินเดือนแล้ว".into()));
    }
    tx.commit().await?;
    audit(&st, Some(&u), "pay", "expense_claim", id, json!({ "journal_entry_id": je })).await;
    Ok(Json(json!({ "ok": true, "journal_entry_id": je })))
}
