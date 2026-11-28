//! Fixed assets: register, monthly straight-line depreciation (first month pro-rated by
//! days, catch-up of missed months, never below the salvage value), disposal with gain /
//! loss, and the asset register in Excel.

use axum::{
    extract::{Path, State},
    http::header,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{Datelike, NaiveDate};
use rust_decimal::{prelude::ToPrimitive, Decimal};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::{audit, AuthUser},
    erp::{self, r2, Acc, Line},
    error::{bad, AppError, AppResult},
    state::AppState,
    util::local_today,
    xlsx::{self, Sheet},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/assets", get(list).post(create))
        .route("/assets/{id}", get(detail).put(update))
        .route("/assets/{id}/dispose", post(dispose))
        .route("/assets/depreciation", post(run_depreciation))
        .route("/assets/depreciation/void", post(void_depreciation))
        .route("/assets/register.xlsx", get(register_xlsx))
}

fn view(u: &AuthUser) -> AppResult<()> {
    u.need_any(&["assets.manage", "accounting.view", "accounting.manage"])
}

// ---------------------------------------------------------------- calculation

fn month_start(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap()
}

fn days_in_month(d: NaiveDate) -> u32 {
    month_start(d).checked_add_months(chrono::Months::new(1)).unwrap().pred_opt().unwrap().day()
}

/// Depreciation for the month starting `period`, given what is already accumulated.
pub fn month_amount(cost: Decimal, salvage: Decimal, life: i32, start: NaiveDate, accumulated: Decimal, period: NaiveDate) -> Decimal {
    let depreciable = (cost - salvage).max(Decimal::ZERO);
    let left = depreciable - accumulated;
    if left <= Decimal::ZERO || life <= 0 || period < month_start(start) {
        return Decimal::ZERO;
    }
    let monthly = depreciable / Decimal::from(life);
    let amount = if period == month_start(start) {
        // first month: only the days the asset was owned
        let days = Decimal::from(days_in_month(start) - start.day() + 1);
        monthly * days / Decimal::from(days_in_month(start))
    } else {
        monthly
    };
    r2(amount).min(left)
}

#[derive(Serialize, sqlx::FromRow, Clone)]
struct AssetRow {
    id: i64,
    asset_no: String,
    name: String,
    category: Option<String>,
    purchase_date: NaiveDate,
    depr_start: NaiveDate,
    cost: Decimal,
    salvage_value: Decimal,
    useful_life_months: i32,
    opening_accumulated: Decimal,
    accumulated: Decimal,
    asset_account_id: i64,
    accum_account_id: i64,
    expense_account_id: i64,
    location: Option<String>,
    custodian_employee_id: Option<i64>,
    custodian_name: Option<String>,
    serial_no: Option<String>,
    status: String,
    disposed_date: Option<NaiveDate>,
    disposal_amount: Option<Decimal>,
    last_period: Option<NaiveDate>,
    note: Option<String>,
}

const ASSET_SELECT: &str = "SELECT a.id, a.asset_no, a.name, a.category, a.purchase_date, a.depr_start, a.cost, a.salvage_value, a.useful_life_months,
    a.opening_accumulated, a.opening_accumulated + COALESCE((SELECT SUM(d.amount) FROM asset_depreciation d WHERE d.asset_id = a.id), 0) AS accumulated,
    a.asset_account_id, a.accum_account_id, a.expense_account_id, a.location, a.custodian_employee_id,
    CONCAT(e.first_name, ' ', e.last_name) AS custodian_name, a.serial_no, a.status, a.disposed_date, a.disposal_amount,
    (SELECT MAX(d.period) FROM asset_depreciation d WHERE d.asset_id = a.id) AS last_period, a.note
    FROM fixed_assets a LEFT JOIN employees e ON e.id = a.custodian_employee_id";

fn with_book_value(a: &AssetRow) -> Value {
    let mut v = serde_json::to_value(a).unwrap_or_default();
    let monthly = if a.useful_life_months > 0 { r2((a.cost - a.salvage_value) / Decimal::from(a.useful_life_months)) } else { Decimal::ZERO };
    v["book_value"] = json!(a.cost - a.accumulated);
    v["monthly_depreciation"] = json!(monthly);
    v
}

async fn list(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    view(&u)?;
    let rows: Vec<AssetRow> = sqlx::query_as(&format!("{ASSET_SELECT} ORDER BY a.asset_no")).fetch_all(&st.db).await?;
    let active: Vec<&AssetRow> = rows.iter().filter(|a| a.status == "active").collect();
    let cost: Decimal = active.iter().map(|a| a.cost).sum();
    let acc: Decimal = active.iter().map(|a| a.accumulated).sum();
    Ok(Json(json!({ "assets": rows.iter().map(with_book_value).collect::<Vec<_>>(), "total_cost": cost, "total_accumulated": acc, "total_book_value": cost - acc })))
}

async fn detail(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    view(&u)?;
    let a: AssetRow = sqlx::query_as(&format!("{ASSET_SELECT} WHERE a.id = ?")).bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let hist: Vec<(NaiveDate, Decimal, i64)> =
        sqlx::query_as("SELECT period, amount, journal_entry_id FROM asset_depreciation WHERE asset_id = ? ORDER BY period").bind(id).fetch_all(&st.db).await?;
    // projected schedule for the remaining life
    let mut schedule = Vec::new();
    let mut acc = a.accumulated;
    let mut p = a.last_period.map(|l| l.checked_add_months(chrono::Months::new(1)).unwrap()).unwrap_or(month_start(a.depr_start));
    while a.status == "active" && schedule.len() < 600 {
        let amt = month_amount(a.cost, a.salvage_value, a.useful_life_months, a.depr_start, acc, p);
        if amt.is_zero() {
            break;
        }
        acc += amt;
        schedule.push(json!({ "period": p, "amount": amt, "accumulated": acc, "book_value": a.cost - acc }));
        p = p.checked_add_months(chrono::Months::new(1)).unwrap();
    }
    Ok(Json(json!({
        "asset": with_book_value(&a),
        "history": hist.into_iter().map(|h| json!({ "period": h.0, "amount": h.1, "journal_entry_id": h.2 })).collect::<Vec<_>>(),
        "schedule": schedule,
    })))
}

#[derive(Deserialize)]
struct AssetReq {
    name: String,
    category: Option<String>,
    purchase_date: NaiveDate,
    cost: Decimal,
    #[serde(default = "one")]
    salvage_value: Decimal,
    useful_life_months: i32,
    #[serde(default)]
    opening_accumulated: Decimal,
    /// first month this system depreciates (for assets taken over with accumulated depreciation)
    depreciate_from: Option<NaiveDate>,
    asset_account_id: Option<i64>,
    accum_account_id: Option<i64>,
    expense_account_id: Option<i64>,
    location: Option<String>,
    custodian_employee_id: Option<i64>,
    serial_no: Option<String>,
    note: Option<String>,
    /// also post the purchase (Dr asset / Cr this cash-bank account); leave empty when the
    /// purchase was already recorded through purchasing
    pay_account_id: Option<i64>,
}

fn one() -> Decimal {
    Decimal::ONE
}

impl AssetReq {
    fn validate(&self) -> AppResult<()> {
        if self.name.trim().is_empty() {
            return Err(bad("กรุณาระบุชื่อทรัพย์สิน"));
        }
        if self.cost <= Decimal::ZERO || self.salvage_value < Decimal::ZERO || self.salvage_value > self.cost {
            return Err(bad("ราคาทุนต้องมากกว่า 0 และมูลค่าซากต้องไม่เกินราคาทุน"));
        }
        if !(1..=1200).contains(&self.useful_life_months) {
            return Err(bad("อายุการใช้งานต้องอยู่ระหว่าง 1-1200 เดือน"));
        }
        if self.opening_accumulated < Decimal::ZERO || self.opening_accumulated > self.cost - self.salvage_value {
            return Err(bad("ค่าเสื่อมราคาสะสมยกมาต้องไม่เกินราคาทุนหักมูลค่าซาก"));
        }
        Ok(())
    }
}

async fn create(State(st): State<AppState>, u: AuthUser, Json(r): Json<AssetReq>) -> AppResult<Json<Value>> {
    u.need("assets.manage")?;
    r.validate()?;
    let mut tx = st.db.begin().await?;
    let acc = |role: &'static str| Acc::Role(role);
    let asset_acc = match r.asset_account_id { Some(a) => a, None => erp::account_id(&mut tx, &acc("fixed_asset")).await? };
    let accum_acc = match r.accum_account_id { Some(a) => a, None => erp::account_id(&mut tx, &acc("accumulated_depreciation")).await? };
    let exp_acc = match r.expense_account_id { Some(a) => a, None => erp::account_id(&mut tx, &acc("depreciation_expense")).await? };
    for (a, want) in [(asset_acc, "asset"), (accum_acc, "asset"), (exp_acc, "expense")] {
        let t: Option<String> = sqlx::query_scalar("SELECT type FROM accounts WHERE id = ? AND is_active = 1").bind(a).fetch_optional(&mut *tx).await?;
        if t.as_deref() != Some(want) {
            return Err(bad("บัญชีทรัพย์สิน / ค่าเสื่อมราคาที่เลือกไม่ถูกประเภท"));
        }
    }
    let depr_start = match (r.depreciate_from, r.opening_accumulated > Decimal::ZERO) {
        (Some(d), _) => d.max(r.purchase_date),
        // taken over with accumulated depreciation: continue from this month
        (None, true) => month_start(local_today(st.cfg.tz_offset_min)).max(r.purchase_date),
        (None, false) => r.purchase_date,
    };
    let no = erp::next_doc_no(&mut tx, "FA", r.purchase_date).await?;
    let id = sqlx::query(
        "INSERT INTO fixed_assets (asset_no, name, category, purchase_date, cost, salvage_value, useful_life_months, opening_accumulated, depr_start,
            asset_account_id, accum_account_id, expense_account_id, location, custodian_employee_id, serial_no, note, created_by)
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&no)
    .bind(r.name.trim())
    .bind(r.category.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(r.purchase_date)
    .bind(r2(r.cost))
    .bind(r2(r.salvage_value))
    .bind(r.useful_life_months)
    .bind(r2(r.opening_accumulated))
    .bind(depr_start)
    .bind(asset_acc)
    .bind(accum_acc)
    .bind(exp_acc)
    .bind(r.location.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(r.custodian_employee_id)
    .bind(r.serial_no.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(r.note.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(u.id())
    .execute(&mut *tx)
    .await?
    .last_insert_id() as i64;
    if let Some(pay) = r.pay_account_id {
        let t: Option<String> = sqlx::query_scalar("SELECT type FROM accounts WHERE id = ? AND is_active = 1").bind(pay).fetch_optional(&mut *tx).await?;
        if t.as_deref() != Some("asset") {
            return Err(bad("บัญชีที่จ่ายซื้อต้องเป็นบัญชีเงินสด / ธนาคาร"));
        }
        let je = erp::post(&mut tx, r.purchase_date, &format!("ซื้อทรัพย์สิน {no} {}", r.name.trim()), "asset_purchase", Some(id),
            vec![Line::dr(Acc::Id(asset_acc), r2(r.cost)), Line::cr(Acc::Id(pay), r2(r.cost))], Some(u.id())).await?;
        sqlx::query("UPDATE fixed_assets SET purchase_entry_id = ? WHERE id = ?").bind(je).bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    audit(&st, Some(&u), "create", "fixed_asset", id, json!({ "asset_no": no, "cost": r.cost })).await;
    Ok(Json(json!({ "id": id, "asset_no": no })))
}

#[derive(Deserialize)]
struct AssetUpdate {
    name: String,
    category: Option<String>,
    location: Option<String>,
    custodian_employee_id: Option<i64>,
    serial_no: Option<String>,
    note: Option<String>,
    salvage_value: Option<Decimal>,
    useful_life_months: Option<i32>,
}

/// Descriptive fields can always change; salvage value and useful life only before the first depreciation.
async fn update(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<AssetUpdate>) -> AppResult<Json<Value>> {
    u.need("assets.manage")?;
    if r.name.trim().is_empty() {
        return Err(bad("กรุณาระบุชื่อทรัพย์สิน"));
    }
    let a: AssetRow = sqlx::query_as(&format!("{ASSET_SELECT} WHERE a.id = ?")).bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let changes_terms = r.salvage_value.is_some_and(|s| s != a.salvage_value) || r.useful_life_months.is_some_and(|l| l != a.useful_life_months);
    if changes_terms && a.last_period.is_some() {
        return Err(bad("คิดค่าเสื่อมราคาไปแล้ว เปลี่ยนมูลค่าซาก / อายุการใช้งานไม่ได้"));
    }
    if r.salvage_value.is_some_and(|s| s < Decimal::ZERO || s > a.cost) || r.useful_life_months.is_some_and(|l| !(1..=1200).contains(&l)) {
        return Err(bad("มูลค่าซากหรืออายุการใช้งานไม่ถูกต้อง"));
    }
    sqlx::query(
        "UPDATE fixed_assets SET name = ?, category = ?, location = ?, custodian_employee_id = ?, serial_no = ?, note = ?,
            salvage_value = ?, useful_life_months = ? WHERE id = ?",
    )
    .bind(r.name.trim())
    .bind(r.category.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(r.location.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(r.custodian_employee_id)
    .bind(r.serial_no.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(r.note.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(r.salvage_value.unwrap_or(a.salvage_value))
    .bind(r.useful_life_months.unwrap_or(a.useful_life_months))
    .bind(id)
    .execute(&st.db)
    .await?;
    audit(&st, Some(&u), "update", "fixed_asset", id, json!({ "name": r.name })).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- depreciation run

#[derive(Deserialize)]
struct PeriodReq {
    year: i32,
    month: u32,
}

/// Depreciates every active asset up to the given month (missed months are caught up)
/// and posts one entry dated the last day of that month.
async fn run_depreciation(State(st): State<AppState>, u: AuthUser, Json(r): Json<PeriodReq>) -> AppResult<Json<Value>> {
    u.need("assets.manage")?;
    let until = NaiveDate::from_ymd_opt(r.year, r.month, 1).ok_or_else(|| bad("เดือน/ปีไม่ถูกต้อง"))?;
    let end = until.checked_add_months(chrono::Months::new(1)).unwrap().pred_opt().unwrap();
    if until > month_start(local_today(st.cfg.tz_offset_min)) {
        return Err(bad("คิดค่าเสื่อมราคาล่วงหน้าไม่ได้"));
    }
    let mut tx = st.db.begin().await?;
    let assets: Vec<AssetRow> = sqlx::query_as(&format!("{ASSET_SELECT} WHERE a.status = 'active' AND a.depr_start <= ? FOR UPDATE")).bind(end).fetch_all(&mut *tx).await?;
    let mut rows: Vec<(i64, NaiveDate, Decimal)> = Vec::new();
    let mut lines = Vec::new();
    for a in &assets {
        let mut acc = a.accumulated;
        let mut p = a.last_period.map(|l| l.checked_add_months(chrono::Months::new(1)).unwrap()).unwrap_or(month_start(a.depr_start));
        let mut total = Decimal::ZERO;
        while p <= until {
            let amt = month_amount(a.cost, a.salvage_value, a.useful_life_months, a.depr_start, acc, p);
            if amt.is_zero() {
                break;
            }
            acc += amt;
            total += amt;
            rows.push((a.id, p, amt));
            p = p.checked_add_months(chrono::Months::new(1)).unwrap();
        }
        if total > Decimal::ZERO {
            lines.push(Line::dr(Acc::Id(a.expense_account_id), total).memo(format!("{} {}", a.asset_no, a.name)));
            lines.push(Line::cr(Acc::Id(a.accum_account_id), total).memo(a.asset_no.clone()));
        }
    }
    if rows.is_empty() {
        return Ok(Json(json!({ "ok": true, "assets": 0, "amount": 0 })));
    }
    let amount: Decimal = rows.iter().map(|r| r.2).sum();
    let key = (r.year * 100 + r.month as i32) as i64;
    let je = erp::post(&mut tx, end, &format!("ค่าเสื่อมราคา {:02}/{}", r.month, r.year + 543), "depreciation", Some(key), lines, Some(u.id())).await?;
    for (asset, p, amt) in &rows {
        sqlx::query("INSERT INTO asset_depreciation (asset_id, period, amount, journal_entry_id) VALUES (?,?,?,?)")
            .bind(asset)
            .bind(p)
            .bind(amt)
            .bind(je)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    let n = rows.iter().map(|r| r.0).collect::<std::collections::HashSet<_>>().len();
    audit(&st, Some(&u), "depreciation", "fixed_asset", key, json!({ "assets": n, "amount": amount })).await;
    Ok(Json(json!({ "ok": true, "assets": n, "months": rows.len(), "amount": amount, "journal_entry_id": je })))
}

/// Undoes the depreciation runs posted for a month (latest runs first).
async fn void_depreciation(State(st): State<AppState>, u: AuthUser, Json(r): Json<PeriodReq>) -> AppResult<Json<Value>> {
    u.need("assets.manage")?;
    let key = (r.year * 100 + r.month as i32) as i64;
    let mut tx = st.db.begin().await?;
    let entries: Vec<(i64,)> = sqlx::query_as("SELECT id FROM journal_entries WHERE source = 'depreciation' AND source_id = ? AND status = 'posted'")
        .bind(key)
        .fetch_all(&mut *tx)
        .await?;
    if entries.is_empty() {
        return Err(bad("ไม่มีรายการค่าเสื่อมราคาของเดือนนี้"));
    }
    // later months must be undone first, otherwise the schedule would have a gap
    let later: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM asset_depreciation d WHERE d.period > ? AND d.asset_id IN
           (SELECT asset_id FROM asset_depreciation WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE source = 'depreciation' AND source_id = ?))",
    )
    .bind(NaiveDate::from_ymd_opt(r.year, r.month, 1).ok_or_else(|| bad("เดือน/ปีไม่ถูกต้อง"))?)
    .bind(key)
    .fetch_one(&mut *tx)
    .await?;
    if later > 0 {
        return Err(bad("มีค่าเสื่อมราคาเดือนถัดไปแล้ว กรุณายกเลิกเดือนล่าสุดก่อน"));
    }
    let disposed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fixed_assets a WHERE a.status = 'disposed' AND a.id IN
           (SELECT asset_id FROM asset_depreciation WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE source = 'depreciation' AND source_id = ?))",
    )
    .bind(key)
    .fetch_one(&mut *tx)
    .await?;
    if disposed > 0 {
        return Err(bad("มีทรัพย์สินที่จำหน่ายไปแล้วในรายการนี้ ยกเลิกไม่ได้"));
    }
    erp::void_entries(&mut tx, "depreciation", key).await?;
    for (je,) in entries {
        sqlx::query("DELETE FROM asset_depreciation WHERE journal_entry_id = ?").bind(je).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    audit(&st, Some(&u), "void_depreciation", "fixed_asset", key, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- disposal

#[derive(Deserialize)]
struct DisposeReq {
    date: NaiveDate,
    /// proceeds (0 = written off / scrapped)
    #[serde(default)]
    amount: Decimal,
    account_id: Option<i64>,
    note: Option<String>,
}

async fn dispose(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<DisposeReq>) -> AppResult<Json<Value>> {
    u.need("assets.manage")?;
    if r.amount < Decimal::ZERO {
        return Err(bad("จำนวนเงินที่ได้รับต้องไม่ติดลบ"));
    }
    let mut tx = st.db.begin().await?;
    let a: AssetRow = sqlx::query_as(&format!("{ASSET_SELECT} WHERE a.id = ? FOR UPDATE")).bind(id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    if a.status != "active" {
        return Err(bad("ทรัพย์สินนี้จำหน่ายไปแล้ว"));
    }
    if r.date < a.purchase_date {
        return Err(bad("วันที่จำหน่ายต้องไม่ก่อนวันที่ซื้อ"));
    }
    // depreciation must be up to date until the month before the disposal
    let prev = month_start(r.date).pred_opt().map(month_start);
    let need = prev.filter(|p| *p >= month_start(a.depr_start));
    if let Some(p) = need {
        let missing = month_amount(a.cost, a.salvage_value, a.useful_life_months, a.depr_start, a.accumulated, p);
        if a.last_period.is_none_or(|l| l < p) && !missing.is_zero() {
            return Err(bad(format!("กรุณาคิดค่าเสื่อมราคาถึงเดือน {:02}/{} ก่อนจำหน่าย", p.month(), p.year() + 543)));
        }
    }
    let book = a.cost - a.accumulated;
    let gain = r2(r.amount) - book;
    let mut lines = vec![Line::dr(Acc::Id(a.accum_account_id), a.accumulated), Line::cr(Acc::Id(a.asset_account_id), a.cost)];
    if r.amount > Decimal::ZERO {
        let acc = match r.account_id {
            Some(x) => x,
            None => erp::account_id(&mut tx, &Acc::Role("bank")).await?,
        };
        lines.push(Line::dr(Acc::Id(acc), r2(r.amount)));
    }
    if gain > Decimal::ZERO {
        lines.push(Line::cr(Acc::Role("asset_gain"), gain));
    } else if gain < Decimal::ZERO {
        lines.push(Line::dr(Acc::Role("asset_loss"), -gain));
    }
    let je = erp::post(&mut tx, r.date, &format!("จำหน่ายทรัพย์สิน {} {}", a.asset_no, a.name), "asset_disposal", Some(id), lines, Some(u.id())).await?;
    sqlx::query("UPDATE fixed_assets SET status = 'disposed', disposed_date = ?, disposal_amount = ?, disposal_entry_id = ?, note = COALESCE(?, note) WHERE id = ?")
        .bind(r.date)
        .bind(r2(r.amount))
        .bind(je)
        .bind(r.note.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    audit(&st, Some(&u), "dispose", "fixed_asset", id, json!({ "amount": r.amount, "gain": gain })).await;
    Ok(Json(json!({ "ok": true, "book_value": book, "gain": gain, "journal_entry_id": je })))
}

async fn register_xlsx(State(st): State<AppState>, u: AuthUser) -> AppResult<Response> {
    view(&u)?;
    let rows: Vec<AssetRow> = sqlx::query_as(&format!("{ASSET_SELECT} ORDER BY a.asset_no")).fetch_all(&st.db).await?;
    let f = |v: Decimal| v.to_f64().unwrap_or(0.0);
    let mut sh = Sheet::new("ทะเบียนทรัพย์สิน", &["เลขที่", "ชื่อ", "หมวด", "วันที่ซื้อ", "ราคาทุน", "มูลค่าซาก", "อายุ (เดือน)", "ค่าเสื่อมต่อเดือน",
        "ค่าเสื่อมสะสม", "มูลค่าตามบัญชี", "สถานที่", "ผู้ดูแล", "สถานะ"]);
    for a in &rows {
        let monthly = if a.useful_life_months > 0 { r2((a.cost - a.salvage_value) / Decimal::from(a.useful_life_months)) } else { Decimal::ZERO };
        sh.push(vec![
            a.asset_no.clone().into(), a.name.clone().into(), a.category.clone().into(), a.purchase_date.to_string().into(), f(a.cost).into(),
            f(a.salvage_value).into(), (a.useful_life_months as i64).into(), f(monthly).into(), f(a.accumulated).into(), f(a.cost - a.accumulated).into(),
            a.location.clone().into(), a.custodian_name.clone().into(), (if a.status == "active" { "ใช้งาน" } else { "จำหน่ายแล้ว" }).into(),
        ]);
    }
    audit(&st, Some(&u), "export_xlsx", "fixed_assets", "-", json!({})).await;
    Ok((
        [
            (header::CONTENT_TYPE, "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string()),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"fixed-assets.xlsx\"".to_string()),
        ],
        xlsx::workbook(&[sh]),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn straight_line_with_first_month_prorated() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 16).unwrap();
        // 36,001 cost, 1 salvage, 36 months: 1,000 / month; January owned 16 of 31 days
        let jan = month_amount(dec!(36001), dec!(1), 36, start, Decimal::ZERO, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        assert_eq!(jan, dec!(516.13));
        let feb = month_amount(dec!(36001), dec!(1), 36, start, jan, NaiveDate::from_ymd_opt(2026, 2, 1).unwrap());
        assert_eq!(feb, dec!(1000));
        // never below salvage: only what is left
        let last = month_amount(dec!(36001), dec!(1), 36, start, dec!(35700), NaiveDate::from_ymd_opt(2029, 1, 1).unwrap());
        assert_eq!(last, dec!(300));
        assert_eq!(month_amount(dec!(36001), dec!(1), 36, start, dec!(36000), NaiveDate::from_ymd_opt(2029, 2, 1).unwrap()), Decimal::ZERO);
        // before the start month nothing
        assert_eq!(month_amount(dec!(36001), dec!(1), 36, start, Decimal::ZERO, NaiveDate::from_ymd_opt(2025, 12, 1).unwrap()), Decimal::ZERO);
    }
}
