//! Accounting: chart of accounts, journal (manual and automatic entries), period lock,
//! company tax details and the trial balance. Financial statements, VAT / WHT reports
//! and fixed assets live in `ledger.rs`.

use axum::{
    extract::{Path, Query, State},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::{audit, AuthUser},
    erp::{self, Acc, Line},
    error::{bad, AppError, AppResult},
    state::AppState,
    util::{local_today, setting},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/accounting/accounts", get(list_accounts).post(create_account))
        .route("/accounting/accounts/{id}", put(update_account))
        .route("/accounting/journal", get(list_journal).post(create_journal))
        .route("/accounting/journal/{id}", get(get_journal))
        .route("/accounting/journal/{id}/void", post(void_journal))
        .route("/accounting/trial-balance", get(trial_balance))
        .route("/accounting/settings", get(get_settings).put(put_settings))
}

pub const TYPES: [&str; 5] = ["asset", "liability", "equity", "revenue", "expense"];

#[derive(Serialize, sqlx::FromRow)]
pub struct Account {
    pub id: i64,
    pub code: String,
    pub name: String,
    #[sqlx(rename = "type")]
    #[serde(rename = "type")]
    pub kind: String,
    pub system_role: Option<String>,
    pub is_active: bool,
}

async fn list_accounts(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Vec<Account>>> {
    // categories / products / payments pick accounts, so several roles need the list
    u.need_any(&["accounting.view", "accounting.manage", "expenses.manage", "sales.manage", "purchasing.manage", "assets.manage", "payroll.manage"])?;
    Ok(Json(
        sqlx::query_as("SELECT id, code, name, type, system_role, is_active FROM accounts ORDER BY code").fetch_all(&st.db).await?,
    ))
}

#[derive(Deserialize)]
struct AccountReq {
    code: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    is_active: Option<bool>,
}

impl AccountReq {
    fn validate(&self) -> AppResult<()> {
        let code = self.code.trim();
        if code.is_empty() || code.len() > 20 || !code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') {
            return Err(bad("รหัสบัญชีใช้ได้เฉพาะตัวอักษร ตัวเลข - และ . ไม่เกิน 20 ตัว"));
        }
        if self.name.trim().is_empty() {
            return Err(bad("กรุณาระบุชื่อบัญชี"));
        }
        if !TYPES.contains(&self.kind.as_str()) {
            return Err(bad("ประเภทบัญชีไม่ถูกต้อง"));
        }
        Ok(())
    }
}

async fn create_account(State(st): State<AppState>, u: AuthUser, Json(r): Json<AccountReq>) -> AppResult<Json<Value>> {
    u.need("accounting.manage")?;
    r.validate()?;
    let id = sqlx::query("INSERT INTO accounts (code, name, type, is_active) VALUES (?,?,?,?)")
        .bind(r.code.trim())
        .bind(r.name.trim())
        .bind(&r.kind)
        .bind(r.is_active.unwrap_or(true))
        .execute(&st.db)
        .await?
        .last_insert_id();
    audit(&st, Some(&u), "create", "account", id, json!({ "code": r.code, "name": r.name })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_account(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<AccountReq>) -> AppResult<Json<Value>> {
    u.need("accounting.manage")?;
    r.validate()?;
    let cur: Option<(String, Option<String>)> = sqlx::query_as("SELECT type, system_role FROM accounts WHERE id = ?")
        .bind(id)
        .fetch_optional(&st.db)
        .await?;
    let (kind, role) = cur.ok_or(AppError::NotFound)?;
    // accounts used by automatic postings keep their type and stay active
    if role.is_some() && (kind != r.kind || r.is_active == Some(false)) {
        return Err(bad("บัญชีนี้ใช้ในการลงบัญชีอัตโนมัติ เปลี่ยนประเภทหรือปิดใช้งานไม่ได้ (เปลี่ยนชื่อ/รหัสได้)"));
    }
    if kind != r.kind {
        let used: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM journal_lines WHERE account_id = ?").bind(id).fetch_one(&st.db).await?;
        if used > 0 {
            return Err(bad("บัญชีนี้มีรายการแล้ว เปลี่ยนประเภทไม่ได้"));
        }
    }
    sqlx::query("UPDATE accounts SET code = ?, name = ?, type = ?, is_active = ? WHERE id = ?")
        .bind(r.code.trim())
        .bind(r.name.trim())
        .bind(&r.kind)
        .bind(r.is_active.unwrap_or(true))
        .bind(id)
        .execute(&st.db)
        .await?;
    audit(&st, Some(&u), "update", "account", id, json!({ "code": r.code, "name": r.name })).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- journal

#[derive(Deserialize, Default)]
pub struct PeriodQ {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub source: Option<String>,
    pub account_id: Option<i64>,
    pub as_of: Option<NaiveDate>,
}

pub fn period(st: &AppState, q: &PeriodQ) -> AppResult<(NaiveDate, NaiveDate)> {
    let today = local_today(st.cfg.tz_offset_min);
    let to = q.to.unwrap_or(today);
    let from = q.from.unwrap_or_else(|| NaiveDate::from_ymd_opt(chrono::Datelike::year(&to), 1, 1).unwrap());
    if from > to {
        return Err(bad("ช่วงวันที่ไม่ถูกต้อง"));
    }
    if (to - from).num_days() > 3700 {
        return Err(bad("ช่วงวันที่ยาวเกินไป (สูงสุด 10 ปี)"));
    }
    Ok((from, to))
}

#[derive(Serialize, sqlx::FromRow)]
struct EntryRow {
    id: i64,
    entry_no: String,
    entry_date: NaiveDate,
    memo: Option<String>,
    source: String,
    source_id: Option<i64>,
    status: String,
    total: Decimal,
    created_by_name: Option<String>,
    created_at: DateTime<Utc>,
}

async fn list_journal(State(st): State<AppState>, u: AuthUser, Query(q): Query<PeriodQ>) -> AppResult<Json<Vec<EntryRow>>> {
    u.need_any(&["accounting.view", "accounting.manage"])?;
    let (from, to) = period(&st, &q)?;
    let mut sql = String::from(
        "SELECT e.id, e.entry_no, e.entry_date, e.memo, e.source, e.source_id, e.status,
                COALESCE((SELECT SUM(debit) FROM journal_lines l WHERE l.entry_id = e.id), 0) AS total,
                us.display_name AS created_by_name, e.created_at
           FROM journal_entries e LEFT JOIN users us ON us.id = e.created_by
          WHERE e.entry_date BETWEEN ? AND ?",
    );
    if q.source.is_some() {
        sql.push_str(" AND e.source = ?");
    }
    sql.push_str(" ORDER BY e.entry_date DESC, e.id DESC LIMIT 2000");
    let mut qq = sqlx::query_as::<_, EntryRow>(&sql).bind(from).bind(to);
    if let Some(s) = &q.source {
        qq = qq.bind(s);
    }
    Ok(Json(qq.fetch_all(&st.db).await?))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct LineRow {
    pub id: i64,
    pub account_id: i64,
    pub account_code: String,
    pub account_name: String,
    pub debit: Decimal,
    pub credit: Decimal,
    pub memo: Option<String>,
    pub party_type: Option<String>,
    pub party_id: Option<i64>,
}

pub async fn entry_lines(st: &AppState, id: i64) -> AppResult<Vec<LineRow>> {
    Ok(sqlx::query_as(
        "SELECT l.id, l.account_id, a.code AS account_code, a.name AS account_name, l.debit, l.credit, l.memo, l.party_type, l.party_id
           FROM journal_lines l JOIN accounts a ON a.id = l.account_id WHERE l.entry_id = ? ORDER BY l.debit = 0, l.id",
    )
    .bind(id)
    .fetch_all(&st.db)
    .await?)
}

async fn get_journal(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need_any(&["accounting.view", "accounting.manage"])?;
    let e: Option<(String, NaiveDate, Option<String>, String, Option<i64>, String)> =
        sqlx::query_as("SELECT entry_no, entry_date, memo, source, source_id, status FROM journal_entries WHERE id = ?")
            .bind(id)
            .fetch_optional(&st.db)
            .await?;
    let (no, date, memo, source, source_id, status) = e.ok_or(AppError::NotFound)?;
    Ok(Json(json!({
        "id": id, "entry_no": no, "entry_date": date, "memo": memo, "source": source, "source_id": source_id,
        "status": status, "lines": entry_lines(&st, id).await?,
    })))
}

#[derive(Deserialize)]
struct ManualLine {
    account_id: i64,
    #[serde(default)]
    debit: Decimal,
    #[serde(default)]
    credit: Decimal,
    memo: Option<String>,
}

#[derive(Deserialize)]
struct ManualEntry {
    entry_date: NaiveDate,
    memo: String,
    lines: Vec<ManualLine>,
}

async fn create_journal(State(st): State<AppState>, u: AuthUser, Json(r): Json<ManualEntry>) -> AppResult<Json<Value>> {
    u.need("accounting.manage")?;
    if r.memo.trim().is_empty() {
        return Err(bad("กรุณาระบุคำอธิบายรายการ"));
    }
    if r.lines.len() < 2 || r.lines.len() > 200 {
        return Err(bad("รายการบัญชีต้องมีอย่างน้อย 2 บรรทัด"));
    }
    for l in &r.lines {
        if l.debit < Decimal::ZERO || l.credit < Decimal::ZERO || (!l.debit.is_zero() && !l.credit.is_zero()) {
            return Err(bad("แต่ละบรรทัดต้องมีเดบิตหรือเครดิตอย่างใดอย่างหนึ่ง และไม่ติดลบ"));
        }
    }
    let dr: Decimal = r.lines.iter().map(|l| erp::r2(l.debit)).sum();
    let cr: Decimal = r.lines.iter().map(|l| erp::r2(l.credit)).sum();
    if dr != cr || dr.is_zero() {
        return Err(bad(format!("ยอดเดบิต ({dr}) และเครดิต ({cr}) ต้องเท่ากันและไม่เป็นศูนย์")));
    }
    let mut tx = st.db.begin().await?;
    for l in &r.lines {
        let ok: Option<bool> = sqlx::query_scalar("SELECT is_active FROM accounts WHERE id = ?").bind(l.account_id).fetch_optional(&mut *tx).await?;
        if ok != Some(true) {
            return Err(bad("มีบัญชีที่ไม่มีอยู่หรือปิดใช้งาน"));
        }
    }
    let lines = r
        .lines
        .iter()
        .map(|l| {
            let mut x = if l.debit.is_zero() { Line::cr(Acc::Id(l.account_id), l.credit) } else { Line::dr(Acc::Id(l.account_id), l.debit) };
            x.memo = l.memo.clone().filter(|m| !m.trim().is_empty());
            x
        })
        .collect();
    let id = erp::post(&mut tx, r.entry_date, r.memo.trim(), "manual", None, lines, Some(u.id())).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "create", "journal", id, json!({ "date": r.entry_date, "total": dr })).await;
    Ok(Json(json!({ "id": id })))
}

async fn void_journal(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("accounting.manage")?;
    let e: Option<(String, String, NaiveDate)> = sqlx::query_as("SELECT source, status, entry_date FROM journal_entries WHERE id = ?")
        .bind(id)
        .fetch_optional(&st.db)
        .await?;
    let (source, status, date) = e.ok_or(AppError::NotFound)?;
    if source != "manual" {
        return Err(bad("รายการนี้สร้างจากเอกสาร ให้ยกเลิกที่เอกสารต้นทาง"));
    }
    if status != "posted" {
        return Err(bad("รายการถูกยกเลิกแล้ว"));
    }
    let mut tx = st.db.begin().await?;
    erp::check_open(&mut tx, date).await?;
    sqlx::query("UPDATE journal_entries SET status = 'void' WHERE id = ?").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "void", "journal", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- trial balance

#[derive(Serialize, sqlx::FromRow)]
pub struct BalanceRow {
    pub id: i64,
    pub code: String,
    pub name: String,
    #[sqlx(rename = "type")]
    #[serde(rename = "type")]
    pub kind: String,
    pub opening: Decimal,
    pub debit: Decimal,
    pub credit: Decimal,
}

/// Opening balance (debit - credit before `from`) and movements within [from, to] per account.
pub async fn balances(st: &AppState, from: NaiveDate, to: NaiveDate) -> AppResult<Vec<BalanceRow>> {
    Ok(sqlx::query_as(
        "SELECT a.id, a.code, a.name, a.type,
                COALESCE(SUM(CASE WHEN e.entry_date < ? THEN l.debit - l.credit END), 0) AS opening,
                COALESCE(SUM(CASE WHEN e.entry_date BETWEEN ? AND ? THEN l.debit END), 0) AS debit,
                COALESCE(SUM(CASE WHEN e.entry_date BETWEEN ? AND ? THEN l.credit END), 0) AS credit
           FROM accounts a
           LEFT JOIN journal_lines l ON l.account_id = a.id
           LEFT JOIN journal_entries e ON e.id = l.entry_id AND e.status = 'posted' AND e.entry_date <= ?
          GROUP BY a.id, a.code, a.name, a.type
          ORDER BY a.code",
    )
    .bind(from)
    .bind(from)
    .bind(to)
    .bind(from)
    .bind(to)
    .bind(to)
    .fetch_all(&st.db)
    .await?)
}

async fn trial_balance(State(st): State<AppState>, u: AuthUser, Query(q): Query<PeriodQ>) -> AppResult<Json<Value>> {
    u.need_any(&["accounting.view", "accounting.manage"])?;
    let (from, to) = period(&st, &q)?;
    let rows: Vec<Value> = balances(&st, from, to)
        .await?
        .into_iter()
        .filter(|r| !(r.opening.is_zero() && r.debit.is_zero() && r.credit.is_zero()))
        .map(|r| {
            let closing = r.opening + r.debit - r.credit;
            json!({
                "id": r.id, "code": r.code, "name": r.name, "type": r.kind, "opening": r.opening,
                "debit": r.debit, "credit": r.credit, "closing": closing,
                "closing_debit": closing.max(Decimal::ZERO), "closing_credit": (-closing).max(Decimal::ZERO),
            })
        })
        .collect();
    let sum = |k: &str| rows.iter().map(|r| r[k].as_f64().unwrap_or(0.0)).sum::<f64>();
    let totals = json!({ "debit": sum("debit"), "credit": sum("credit"), "closing_debit": sum("closing_debit"), "closing_credit": sum("closing_credit") });
    Ok(Json(json!({ "from": from, "to": to, "rows": rows, "totals": totals })))
}

// ---------------------------------------------------------------- settings (company / lock date)

const COMPANY_KEYS: [&str; 4] = ["company_tax_id", "company_address", "company_branch", "accounting_lock_date"];

async fn get_settings(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    u.need_any(&["accounting.view", "accounting.manage", "sales.manage", "purchasing.manage", "payroll.manage"])?;
    let mut m = serde_json::Map::new();
    for k in COMPANY_KEYS {
        m.insert(k.into(), setting(&st.db, k).await.unwrap_or(Value::String(String::new())));
    }
    m.insert("org_name".into(), setting(&st.db, "org_name").await.unwrap_or(Value::Null));
    Ok(Json(Value::Object(m)))
}

async fn put_settings(State(st): State<AppState>, u: AuthUser, Json(body): Json<serde_json::Map<String, Value>>) -> AppResult<Json<Value>> {
    u.need("accounting.manage")?;
    for (k, v) in &body {
        if !COMPANY_KEYS.contains(&k.as_str()) {
            return Err(bad(format!("ไม่รู้จักการตั้งค่า: {k}")));
        }
        let s = v.as_str().ok_or_else(|| bad(format!("{k} ต้องเป็นข้อความ")))?;
        match k.as_str() {
            "company_tax_id" if !s.is_empty() && !(s.len() == 13 && s.chars().all(|c| c.is_ascii_digit())) => {
                return Err(bad("เลขประจำตัวผู้เสียภาษีต้องเป็นตัวเลข 13 หลัก"));
            }
            "accounting_lock_date" if !s.is_empty() => {
                erp::parse_date(s)?;
            }
            _ if s.len() > 500 => return Err(bad(format!("{k} ยาวเกินไป"))),
            _ => {}
        }
    }
    for (k, v) in &body {
        sqlx::query("INSERT INTO system_settings (setting_key, setting_value) VALUES (?, ?) ON DUPLICATE KEY UPDATE setting_value = VALUES(setting_value)")
            .bind(k)
            .bind(v.to_string())
            .execute(&st.db)
            .await?;
    }
    audit(&st, Some(&u), "update", "accounting_settings", "-", Value::Object(body)).await;
    Ok(Json(json!({ "ok": true })))
}

/// Text form of a setting (company details on printed documents).
pub async fn company(st: &AppState) -> Value {
    let get = |k: &'static str| async move { crate::util::setting_str(&st.db, k, "").await };
    json!({
        "name": get("org_name").await, "tax_id": get("company_tax_id").await,
        "address": get("company_address").await, "branch": get("company_branch").await,
        "logo_url": get("logo_url").await,
    })
}
