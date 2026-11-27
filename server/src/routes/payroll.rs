//! Payroll: pay setup per employee, monthly runs calculated from the real work calendar
//! (absence, unpaid leave, approved overtime, holiday work), Thai social security,
//! withholding tax (annualised, ภ.ง.ด.1) and provident fund; payslips, bank file,
//! ภ.ง.ด.1 / สปส.1-10 exports and the accounting entry when the run is paid.

use axum::{
    extract::{Path, State},
    http::header,
    response::{Html, IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::{Datelike, NaiveDate};
use rust_decimal::{prelude::*, Decimal};
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::{audit, AuthUser},
    erp::{self, fmt_money, r0, r2, Acc, Line, Tx},
    error::{bad, AppError, AppResult},
    routes::accounting::company,
    state::AppState,
    util::{html_escape as esc, setting},
    xlsx::{self, Sheet},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/payroll/settings", get(get_settings).put(put_settings))
        .route("/payroll/employees", get(list_pay))
        .route("/payroll/employees/{id}", put(update_pay))
        .route("/payroll/runs", get(list_runs).post(create_run))
        .route("/payroll/runs/{id}", get(get_run))
        .route("/payroll/runs/{id}/compute", post(recompute))
        .route("/payroll/runs/{id}/approve", post(approve))
        .route("/payroll/runs/{id}/pay", post(pay))
        .route("/payroll/runs/{id}/void", post(void_run))
        .route("/payroll/runs/{id}/bank.csv", get(bank_csv))
        .route("/payroll/runs/{id}/pnd1.xlsx", get(pnd1_xlsx))
        .route("/payroll/runs/{id}/sso.xlsx", get(sso_xlsx))
        .route("/payroll/payslips/{id}", get(get_payslip))
        .route("/payroll/payslips/{id}/items", post(add_item))
        .route("/payroll/payslips/{id}/print", get(print_payslip))
        .route("/payroll/items/{id}", delete(delete_item))
        .route("/me/payslips", get(my_payslips))
}

// ---------------------------------------------------------------- settings

#[derive(Clone, Debug)]
pub struct PaySettings {
    sso_rate: Decimal,
    sso_min: Decimal,
    sso_max: Decimal,
    ot_rate: Decimal,
    holiday_rate: Decimal,
    holiday_ot_rate: Decimal,
    deduct_absence: bool,
    deduct_late: bool,
}

const SETTING_KEYS: [(&str, f64); 6] = [
    ("payroll_sso_rate", 5.0),
    ("payroll_sso_min_base", 1650.0),
    ("payroll_sso_max_base", 17500.0),
    ("payroll_ot_rate", 1.5),
    ("payroll_holiday_rate", 1.0),
    ("payroll_holiday_ot_rate", 3.0),
];

async fn load_settings(st: &AppState) -> PaySettings {
    let num = |k: &'static str, def: f64| async move { erp::d(setting(&st.db, k).await.and_then(|v| v.as_f64()).unwrap_or(def)) };
    let flag = |k: &'static str, def: bool| async move { setting(&st.db, k).await.and_then(|v| v.as_bool()).unwrap_or(def) };
    PaySettings {
        sso_rate: num("payroll_sso_rate", 5.0).await,
        sso_min: num("payroll_sso_min_base", 1650.0).await,
        sso_max: num("payroll_sso_max_base", 17500.0).await,
        ot_rate: num("payroll_ot_rate", 1.5).await,
        holiday_rate: num("payroll_holiday_rate", 1.0).await,
        holiday_ot_rate: num("payroll_holiday_ot_rate", 3.0).await,
        deduct_absence: flag("payroll_deduct_absence", true).await,
        deduct_late: flag("payroll_deduct_late", false).await,
    }
}

async fn get_settings(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    u.need_any(&["payroll.view", "payroll.manage"])?;
    let mut m = serde_json::Map::new();
    for (k, def) in SETTING_KEYS {
        m.insert(k.into(), setting(&st.db, k).await.unwrap_or(json!(def)));
    }
    for k in ["payroll_deduct_absence", "payroll_deduct_late"] {
        m.insert(k.into(), setting(&st.db, k).await.unwrap_or(json!(k == "payroll_deduct_absence")));
    }
    Ok(Json(Value::Object(m)))
}

async fn put_settings(State(st): State<AppState>, u: AuthUser, Json(body): Json<serde_json::Map<String, Value>>) -> AppResult<Json<Value>> {
    u.need("payroll.manage")?;
    for (k, v) in &body {
        match k.as_str() {
            "payroll_deduct_absence" | "payroll_deduct_late" if v.is_boolean() => {}
            _ if SETTING_KEYS.iter().any(|(s, _)| s == k) => {
                let n = v.as_f64().ok_or_else(|| bad(format!("{k} ต้องเป็นตัวเลข")))?;
                if !(0.0..=1_000_000.0).contains(&n) {
                    return Err(bad(format!("{k} อยู่นอกช่วงที่รับได้")));
                }
            }
            _ => return Err(bad(format!("ไม่รู้จักการตั้งค่า: {k}"))),
        }
    }
    for (k, v) in &body {
        sqlx::query("INSERT INTO system_settings (setting_key, setting_value) VALUES (?, ?) ON DUPLICATE KEY UPDATE setting_value = VALUES(setting_value)")
            .bind(k)
            .bind(v.to_string())
            .execute(&st.db)
            .await?;
    }
    audit(&st, Some(&u), "update", "payroll_settings", "-", Value::Object(body)).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- pay setup

#[derive(Serialize, Deserialize, sqlx::FromRow, Clone, Debug)]
pub struct PaySetup {
    #[serde(default)]
    pub employee_id: i64,
    pub pay_type: String,
    pub base_salary: Decimal,
    pub daily_rate: Decimal,
    pub fixed_allowance: Decimal,
    pub tax_id: Option<String>,
    pub bank_name: Option<String>,
    pub bank_account: Option<String>,
    pub sso_enabled: bool,
    pub pvd_rate: Decimal,
    pub pvd_employer_rate: Decimal,
    pub tax_allowance_annual: Decimal,
}

#[derive(Serialize, sqlx::FromRow)]
struct PayRow {
    /// = employee_id (the UI edits rows by id)
    id: i64,
    employee_id: i64,
    employee_code: String,
    name: String,
    department: Option<String>,
    position: Option<String>,
    configured: bool,
    pay_type: Option<String>,
    base_salary: Option<Decimal>,
    daily_rate: Option<Decimal>,
    fixed_allowance: Option<Decimal>,
    tax_id: Option<String>,
    bank_name: Option<String>,
    bank_account: Option<String>,
    sso_enabled: Option<bool>,
    pvd_rate: Option<Decimal>,
    pvd_employer_rate: Option<Decimal>,
    tax_allowance_annual: Option<Decimal>,
}

async fn list_pay(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Vec<PayRow>>> {
    u.need_any(&["payroll.view", "payroll.manage"])?;
    Ok(Json(
        sqlx::query_as(
            "SELECT e.id, e.id AS employee_id, e.employee_code, CONCAT(e.first_name, ' ', e.last_name) AS name, d.name AS department, e.position,
                    p.employee_id IS NOT NULL AS configured, p.pay_type, p.base_salary, p.daily_rate, p.fixed_allowance, p.tax_id,
                    p.bank_name, p.bank_account, p.sso_enabled, p.pvd_rate, p.pvd_employer_rate, p.tax_allowance_annual
               FROM employees e LEFT JOIN departments d ON d.id = e.department_id LEFT JOIN employee_pay p ON p.employee_id = e.id
              WHERE e.status = 'active' ORDER BY e.employee_code",
        )
        .fetch_all(&st.db)
        .await?,
    ))
}

async fn update_pay(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<PaySetup>) -> AppResult<Json<Value>> {
    u.need("payroll.manage")?;
    if !["monthly", "daily"].contains(&r.pay_type.as_str()) {
        return Err(bad("รูปแบบการจ่ายต้องเป็นรายเดือนหรือรายวัน"));
    }
    let money_ok = |v: Decimal| v >= Decimal::ZERO && v < dec!(100000000);
    if ![r.base_salary, r.daily_rate, r.fixed_allowance, r.tax_allowance_annual].into_iter().all(money_ok) {
        return Err(bad("จำนวนเงินต้องไม่ติดลบ"));
    }
    if r.pay_type == "monthly" && r.base_salary <= Decimal::ZERO || r.pay_type == "daily" && r.daily_rate <= Decimal::ZERO {
        return Err(bad("กรุณาระบุเงินเดือนหรือค่าจ้างรายวัน"));
    }
    if ![r.pvd_rate, r.pvd_employer_rate].iter().all(|p| *p >= Decimal::ZERO && *p <= dec!(15)) {
        return Err(bad("อัตรากองทุนสำรองเลี้ยงชีพต้องอยู่ระหว่าง 0-15%"));
    }
    let tax_id = r.tax_id.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if tax_id.is_some_and(|t| !(t.len() == 13 && t.chars().all(|c| c.is_ascii_digit()))) {
        return Err(bad("เลขประจำตัวผู้เสียภาษีต้องเป็นตัวเลข 13 หลัก"));
    }
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM employees WHERE id = ?").bind(id).fetch_optional(&st.db).await?;
    exists.ok_or(AppError::NotFound)?;
    sqlx::query(
        "INSERT INTO employee_pay (employee_id, pay_type, base_salary, daily_rate, fixed_allowance, tax_id, bank_name, bank_account,
            sso_enabled, pvd_rate, pvd_employer_rate, tax_allowance_annual) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)
         ON DUPLICATE KEY UPDATE pay_type=VALUES(pay_type), base_salary=VALUES(base_salary), daily_rate=VALUES(daily_rate),
            fixed_allowance=VALUES(fixed_allowance), tax_id=VALUES(tax_id), bank_name=VALUES(bank_name), bank_account=VALUES(bank_account),
            sso_enabled=VALUES(sso_enabled), pvd_rate=VALUES(pvd_rate), pvd_employer_rate=VALUES(pvd_employer_rate),
            tax_allowance_annual=VALUES(tax_allowance_annual)",
    )
    .bind(id)
    .bind(&r.pay_type)
    .bind(r2(r.base_salary))
    .bind(r2(r.daily_rate))
    .bind(r2(r.fixed_allowance))
    .bind(tax_id)
    .bind(r.bank_name.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(r.bank_account.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(r.sso_enabled)
    .bind(r.pvd_rate)
    .bind(r.pvd_employer_rate)
    .bind(r2(r.tax_allowance_annual))
    .execute(&st.db)
    .await?;
    // salaries are sensitive: the audit log records that it changed, not the amount
    audit(&st, Some(&u), "update", "employee_pay", id, json!({ "pay_type": r.pay_type })).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- calculation

#[derive(Debug, Clone)]
struct Item {
    kind: &'static str,
    code: String,
    label: String,
    amount: Decimal,
    taxable: bool,
    manual: bool,
}

impl Item {
    fn earn(code: &str, label: impl Into<String>, amount: Decimal, taxable: bool) -> Self {
        Self { kind: "earning", code: code.into(), label: label.into(), amount: r2(amount), taxable, manual: false }
    }
    fn ded(code: &str, label: impl Into<String>, amount: Decimal) -> Self {
        Self { kind: "deduction", code: code.into(), label: label.into(), amount: r2(amount), taxable: false, manual: false }
    }
}

/// Work facts for one employee in the pay period (from work_sessions).
#[derive(Default, Debug)]
struct Facts {
    work_days: i32,
    paid_days: i32,
    absent_days: i32,
    unpaid_leave_days: i32,
    late_min: i32,
    ot_min: i32,
    /// approved overtime minutes worked on each holiday / day off
    holiday_mins: Vec<i32>,
}

async fn facts(tx: &mut Tx<'_>, emp: i64, from: NaiveDate, to: NaiveDate, today: NaiveDate) -> AppResult<Facts> {
    let rows: Vec<(NaiveDate, String, String, i32, i32)> = sqlx::query_as(
        "SELECT work_date, day_type, status, late_min, approved_overtime_min FROM work_sessions
          WHERE employee_id = ? AND work_date BETWEEN ? AND ?",
    )
    .bind(emp)
    .bind(from)
    .bind(to)
    .fetch_all(&mut **tx)
    .await?;
    // leave days whose leave type is unpaid
    let unpaid: Vec<(NaiveDate,)> = sqlx::query_as(
        "SELECT DISTINCT w.work_date FROM work_sessions w
           JOIN requests r ON r.employee_id = w.employee_id AND r.kind = 'leave' AND r.status = 'approved'
                          AND r.start_at < w.scheduled_end AND r.end_at > w.scheduled_start
           JOIN leave_types lt ON lt.id = r.leave_type_id AND lt.paid = 0
          WHERE w.employee_id = ? AND w.work_date BETWEEN ? AND ? AND w.status = 'leave'",
    )
    .bind(emp)
    .bind(from)
    .bind(to)
    .fetch_all(&mut **tx)
    .await?;
    let unpaid: std::collections::HashSet<NaiveDate> = unpaid.into_iter().map(|r| r.0).collect();
    let mut f = Facts::default();
    for (date, day_type, status, late, ot) in rows {
        if day_type == "workday" {
            f.work_days += 1;
            f.ot_min += ot.max(0);
            f.late_min += late.max(0);
            match status.as_str() {
                // days that have not happened yet are not absences
                "absent" if date < today => f.absent_days += 1,
                "leave" if unpaid.contains(&date) => f.unpaid_leave_days += 1,
                "present" | "offsite" | "leave" => f.paid_days += 1,
                _ => {}
            }
        } else {
            if ot > 0 {
                f.holiday_mins.push(ot);
            }
            // public holidays are paid days for daily-paid staff too (Labour Protection Act s.29, 56)
            if day_type == "holiday" && ot == 0 {
                f.paid_days += 1;
            }
        }
    }
    Ok(f)
}

struct Calc {
    items: Vec<Item>,
    facts: Facts,
    taxable: Decimal,
    sso_base: Decimal,
    sso_ee: Decimal,
    sso_er: Decimal,
    wht: Decimal,
    pvd_ee: Decimal,
    pvd_er: Decimal,
}

fn calculate(p: &PaySetup, s: &PaySettings, f: Facts, manual: Vec<Item>, reimb: Vec<Item>) -> Calc {
    let monthly = p.pay_type == "monthly";
    // hourly rate: monthly salary / 30 days / 8 hours, or daily wage / 8 (s.68)
    let daily = if monthly { p.base_salary / dec!(30) } else { p.daily_rate };
    let hourly = daily / dec!(8);
    let mut items = Vec::new();
    if monthly {
        items.push(Item::earn("salary", "เงินเดือน", p.base_salary, true));
    } else {
        items.push(Item::earn("salary", format!("ค่าจ้างรายวัน {} วัน", f.paid_days), p.daily_rate * Decimal::from(f.paid_days), true));
    }
    if p.fixed_allowance > Decimal::ZERO {
        items.push(Item::earn("allowance", "ค่าตำแหน่ง / เบี้ยเลี้ยงประจำ", p.fixed_allowance, true));
    }
    if f.ot_min > 0 {
        let h = Decimal::from(f.ot_min) / dec!(60);
        items.push(Item::earn("ot", format!("ค่าล่วงเวลา {} ชม. × {}", h.round_dp(2).normalize(), s.ot_rate.normalize()), hourly * h * s.ot_rate, true));
    }
    if !f.holiday_mins.is_empty() {
        // within normal hours (first 8 h): holiday pay; beyond: holiday overtime (s.62, 63)
        // daily-paid staff get double pay for holiday work, monthly staff one extra day's pay
        let normal_rate = if monthly { s.holiday_rate } else { s.holiday_rate + Decimal::ONE };
        let (mut normal, mut extra) = (0i32, 0i32);
        for m in &f.holiday_mins {
            normal += (*m).min(480);
            extra += (*m - 480).max(0);
        }
        let amount = hourly * (Decimal::from(normal) / dec!(60) * normal_rate + Decimal::from(extra) / dec!(60) * s.holiday_ot_rate);
        items.push(Item::earn("holiday", format!("ทำงาน / ล่วงเวลาในวันหยุด {} ชม.", (Decimal::from(normal + extra) / dec!(60)).round_dp(2).normalize()), amount, true));
    }
    items.extend(manual.iter().filter(|i| i.kind == "earning").cloned());
    items.extend(reimb);
    if monthly && s.deduct_absence && f.absent_days > 0 {
        items.push(Item::ded("absence", format!("ขาดงาน {} วัน", f.absent_days), daily * Decimal::from(f.absent_days)));
    }
    if monthly && f.unpaid_leave_days > 0 {
        items.push(Item::ded("unpaid_leave", format!("ลาไม่รับค่าจ้าง {} วัน", f.unpaid_leave_days), daily * Decimal::from(f.unpaid_leave_days)));
    }
    if s.deduct_late && f.late_min > 0 {
        items.push(Item::ded("late", format!("มาสาย {} นาที", f.late_min), hourly / dec!(60) * Decimal::from(f.late_min)));
    }
    let pay_cut: Decimal = items.iter().filter(|i| ["absence", "unpaid_leave", "late"].contains(&i.code.as_str())).map(|i| i.amount).sum();
    let taxable_earn: Decimal = items.iter().filter(|i| i.kind == "earning" && i.taxable).map(|i| i.amount).sum();
    let bonus: Decimal = items.iter().filter(|i| i.kind == "earning" && i.code == "bonus" && i.taxable).map(|i| i.amount).sum();
    let taxable = (taxable_earn - pay_cut).max(Decimal::ZERO);
    let regular = (taxable - bonus).max(Decimal::ZERO);

    // social security on wages (bonus excluded), whole baht
    let (sso_base, sso_ee) = if p.sso_enabled && regular > Decimal::ZERO {
        let base = regular.clamp(s.sso_min, s.sso_max);
        (base, r0(base * s.sso_rate / dec!(100)))
    } else {
        (Decimal::ZERO, Decimal::ZERO)
    };
    // provident fund on salary / wages
    let pvd_base = regular;
    let pvd_ee = r2(pvd_base * p.pvd_rate / dec!(100));
    let pvd_er = r2(pvd_base * p.pvd_employer_rate / dec!(100));
    // withholding tax: annualise regular income; a bonus is taxed on the difference it makes (Revenue Dept. method)
    let annual_deductions = sso_ee * dec!(12) + pvd_ee * dec!(12) + p.tax_allowance_annual;
    let annual = regular * dec!(12);
    let base_tax = erp::annual_employment_tax(annual, annual_deductions);
    let mut wht = base_tax / dec!(12);
    if bonus > Decimal::ZERO {
        wht += erp::annual_employment_tax(annual + bonus, annual_deductions) - base_tax;
    }
    let wht = r2(wht.max(Decimal::ZERO));
    if sso_ee > Decimal::ZERO {
        items.push(Item::ded("sso", format!("ประกันสังคม {}%", s.sso_rate.normalize()), sso_ee));
    }
    if pvd_ee > Decimal::ZERO {
        items.push(Item::ded("pvd", format!("กองทุนสำรองเลี้ยงชีพ {}%", p.pvd_rate.normalize()), pvd_ee));
    }
    if wht > Decimal::ZERO {
        items.push(Item::ded("wht", "ภาษีเงินได้หัก ณ ที่จ่าย", wht));
    }
    items.extend(manual.into_iter().filter(|i| i.kind == "deduction"));
    Calc { items, facts: f, taxable, sso_base, sso_ee, sso_er: sso_ee, wht, pvd_ee, pvd_er }
}

#[derive(sqlx::FromRow, Debug)]
struct RunRow {
    id: i64,
    run_no: String,
    period_year: i32,
    period_month: i32,
    period_start: NaiveDate,
    period_end: NaiveDate,
    pay_date: NaiveDate,
    status: String,
}

async fn load_run(db: impl sqlx::MySqlExecutor<'_>, id: i64) -> AppResult<RunRow> {
    sqlx::query_as(
        "SELECT id, run_no, period_year, period_month, period_start, period_end, pay_date, status FROM payroll_runs WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(db)
    .await?
    .ok_or(AppError::NotFound)
}

/// (Re)calculates every payslip of a draft run. Manual items are kept.
async fn compute_run(st: &AppState, tx: &mut Tx<'_>, run: &RunRow) -> AppResult<usize> {
    let s = load_settings(st).await;
    let today = crate::util::local_today(st.cfg.tz_offset_min);
    let setups: Vec<PaySetup> = sqlx::query_as(
        "SELECT p.employee_id, p.pay_type, p.base_salary, p.daily_rate, p.fixed_allowance, p.tax_id, p.bank_name, p.bank_account,
                p.sso_enabled, p.pvd_rate, p.pvd_employer_rate, p.tax_allowance_annual
           FROM employee_pay p JOIN employees e ON e.id = p.employee_id WHERE e.status = 'active'",
    )
    .fetch_all(&mut **tx)
    .await?;
    // employees no longer on the payroll
    let ids: Vec<i64> = setups.iter().map(|p| p.employee_id).collect();
    let existing: Vec<(i64, i64)> = sqlx::query_as("SELECT id, employee_id FROM payslips WHERE run_id = ?").bind(run.id).fetch_all(&mut **tx).await?;
    for (pid, emp) in &existing {
        if !ids.contains(emp) {
            sqlx::query("DELETE FROM payslips WHERE id = ?").bind(pid).execute(&mut **tx).await?;
        }
    }
    // approved claims paid with the salary: release what this run held, then take them again
    sqlx::query("UPDATE expense_claims SET payroll_run_id = NULL WHERE payroll_run_id = ? AND status = 'approved'")
        .bind(run.id)
        .execute(&mut **tx)
        .await?;
    for p in &setups {
        let f = facts(tx, p.employee_id, run.period_start, run.period_end, today).await?;
        let slip_id: i64 = match existing.iter().find(|e| e.1 == p.employee_id) {
            Some((id, _)) => *id,
            None => sqlx::query("INSERT INTO payslips (run_id, employee_id, pay_type) VALUES (?,?,?)")
                .bind(run.id)
                .bind(p.employee_id)
                .bind(&p.pay_type)
                .execute(&mut **tx)
                .await?
                .last_insert_id() as i64,
        };
        let manual: Vec<Item> = sqlx::query_as::<_, (String, String, String, Decimal, bool)>(
            "SELECT kind, code, label, amount, taxable FROM payslip_items WHERE payslip_id = ? AND manual = 1 ORDER BY id",
        )
        .bind(slip_id)
        .fetch_all(&mut **tx)
        .await?
        .into_iter()
        .map(|(k, code, label, amount, taxable)| Item {
            kind: if k == "earning" { "earning" } else { "deduction" },
            code,
            label,
            amount,
            taxable,
            manual: true,
        })
        .collect();
        let claims: Vec<(i64, String, Decimal)> = sqlx::query_as(
            "SELECT id, claim_no, total FROM expense_claims
              WHERE employee_id = ? AND status = 'approved' AND pay_method = 'payroll' AND payroll_run_id IS NULL",
        )
        .bind(p.employee_id)
        .fetch_all(&mut **tx)
        .await?;
        for (cid, _, _) in &claims {
            sqlx::query("UPDATE expense_claims SET payroll_run_id = ? WHERE id = ?").bind(run.id).bind(cid).execute(&mut **tx).await?;
        }
        let reimb = claims.iter().map(|(_, no, total)| Item::earn("reimbursement", format!("เบิกค่าใช้จ่าย {no}"), *total, false)).collect();
        let c = calculate(p, &s, f, manual, reimb);
        write_slip(tx, slip_id, p, &c).await?;
    }
    update_totals(tx, run.id).await?;
    Ok(setups.len())
}

async fn write_slip(tx: &mut Tx<'_>, slip_id: i64, p: &PaySetup, c: &Calc) -> AppResult<()> {
    sqlx::query("DELETE FROM payslip_items WHERE payslip_id = ?").bind(slip_id).execute(&mut **tx).await?;
    for (i, it) in c.items.iter().enumerate() {
        sqlx::query("INSERT INTO payslip_items (payslip_id, kind, code, label, amount, taxable, manual, sort) VALUES (?,?,?,?,?,?,?,?)")
            .bind(slip_id)
            .bind(it.kind)
            .bind(&it.code)
            .bind(&it.label)
            .bind(it.amount)
            .bind(it.taxable)
            .bind(it.manual)
            .bind(i as i32)
            .execute(&mut **tx)
            .await?;
    }
    let earn: Decimal = c.items.iter().filter(|i| i.kind == "earning").map(|i| i.amount).sum();
    let ded: Decimal = c.items.iter().filter(|i| i.kind == "deduction").map(|i| i.amount).sum();
    sqlx::query(
        "UPDATE payslips SET pay_type = ?, base_salary = ?, work_days = ?, absent_days = ?, unpaid_leave_days = ?, late_min = ?, ot_min = ?,
            holiday_min = ?, total_earnings = ?, total_deductions = ?, net_pay = ?, taxable_income = ?, sso_base = ?, sso_employee = ?,
            sso_employer = ?, wht = ?, pvd_employee = ?, pvd_employer = ? WHERE id = ?",
    )
    .bind(&p.pay_type)
    .bind(if p.pay_type == "monthly" { p.base_salary } else { p.daily_rate })
    .bind(c.facts.work_days)
    .bind(c.facts.absent_days)
    .bind(c.facts.unpaid_leave_days)
    .bind(c.facts.late_min)
    .bind(c.facts.ot_min)
    .bind(c.facts.holiday_mins.iter().sum::<i32>())
    .bind(earn)
    .bind(ded)
    .bind(earn - ded)
    .bind(c.taxable)
    .bind(c.sso_base)
    .bind(c.sso_ee)
    .bind(c.sso_er)
    .bind(c.wht)
    .bind(c.pvd_ee)
    .bind(c.pvd_er)
    .bind(slip_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn update_totals(tx: &mut Tx<'_>, run_id: i64) -> AppResult<()> {
    sqlx::query(
        "UPDATE payroll_runs r SET
            employees = (SELECT COUNT(*) FROM payslips WHERE run_id = r.id),
            total_earnings = (SELECT COALESCE(SUM(total_earnings), 0) FROM payslips WHERE run_id = r.id),
            total_deductions = (SELECT COALESCE(SUM(total_deductions), 0) FROM payslips WHERE run_id = r.id),
            total_net = (SELECT COALESCE(SUM(net_pay), 0) FROM payslips WHERE run_id = r.id),
            total_employer = (SELECT COALESCE(SUM(sso_employer + pvd_employer), 0) FROM payslips WHERE run_id = r.id)
          WHERE r.id = ?",
    )
    .bind(run_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------- runs

#[derive(Serialize, sqlx::FromRow)]
struct RunSummary {
    id: i64,
    run_no: String,
    period_year: i32,
    period_month: i32,
    period_start: NaiveDate,
    period_end: NaiveDate,
    pay_date: NaiveDate,
    status: String,
    employees: i32,
    total_earnings: Decimal,
    total_deductions: Decimal,
    total_net: Decimal,
    total_employer: Decimal,
    journal_entry_id: Option<i64>,
    note: Option<String>,
}

const RUN_SELECT: &str = "SELECT id, run_no, period_year, period_month, period_start, period_end, pay_date, status, employees,
    total_earnings, total_deductions, total_net, total_employer, journal_entry_id, note FROM payroll_runs";

async fn list_runs(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Vec<RunSummary>>> {
    u.need_any(&["payroll.view", "payroll.manage"])?;
    Ok(Json(sqlx::query_as(&format!("{RUN_SELECT} ORDER BY period_year DESC, period_month DESC, id DESC LIMIT 240")).fetch_all(&st.db).await?))
}

#[derive(Deserialize)]
struct RunReq {
    year: i32,
    month: u32,
    pay_date: Option<NaiveDate>,
    note: Option<String>,
}

async fn create_run(State(st): State<AppState>, u: AuthUser, Json(r): Json<RunReq>) -> AppResult<Json<Value>> {
    u.need("payroll.manage")?;
    let start = NaiveDate::from_ymd_opt(r.year, r.month, 1).filter(|_| (2000..=2200).contains(&r.year)).ok_or_else(|| bad("เดือน/ปีไม่ถูกต้อง"))?;
    let end = start.checked_add_months(chrono::Months::new(1)).unwrap().pred_opt().unwrap();
    let pay_date = r.pay_date.unwrap_or(end);
    let dup: Option<String> = sqlx::query_scalar("SELECT run_no FROM payroll_runs WHERE period_year = ? AND period_month = ? AND status <> 'void'")
        .bind(r.year)
        .bind(r.month)
        .fetch_optional(&st.db)
        .await?;
    if let Some(no) = dup {
        return Err(AppError::Conflict(format!("มีรอบเงินเดือนของเดือนนี้แล้ว ({no}) ยกเลิกรอบเดิมก่อนถ้าต้องการสร้างใหม่")));
    }
    let mut tx = st.db.begin().await?;
    let no = erp::next_doc_no(&mut tx, "PR", start).await?;
    let id = sqlx::query(
        "INSERT INTO payroll_runs (run_no, period_year, period_month, period_start, period_end, pay_date, note, created_by) VALUES (?,?,?,?,?,?,?,?)",
    )
    .bind(&no)
    .bind(r.year)
    .bind(r.month)
    .bind(start)
    .bind(end)
    .bind(pay_date)
    .bind(r.note.as_deref().map(str::trim).filter(|s| !s.is_empty()))
    .bind(u.id())
    .execute(&mut *tx)
    .await?
    .last_insert_id() as i64;
    let run = load_run(&mut *tx, id).await?;
    let n = compute_run(&st, &mut tx, &run).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "create", "payroll_run", id, json!({ "run_no": no, "employees": n })).await;
    Ok(Json(json!({ "id": id, "run_no": no, "employees": n })))
}

#[derive(Serialize, sqlx::FromRow)]
struct SlipRow {
    id: i64,
    employee_id: i64,
    employee_code: String,
    name: String,
    department: Option<String>,
    pay_type: String,
    base_salary: Decimal,
    work_days: i32,
    absent_days: i32,
    unpaid_leave_days: i32,
    late_min: i32,
    ot_min: i32,
    holiday_min: i32,
    total_earnings: Decimal,
    total_deductions: Decimal,
    net_pay: Decimal,
    taxable_income: Decimal,
    sso_base: Decimal,
    sso_employee: Decimal,
    sso_employer: Decimal,
    wht: Decimal,
    pvd_employee: Decimal,
    pvd_employer: Decimal,
    tax_id: Option<String>,
    bank_name: Option<String>,
    bank_account: Option<String>,
}

const SLIP_SELECT: &str = "SELECT s.id, s.employee_id, e.employee_code, CONCAT(e.first_name, ' ', e.last_name) AS name, d.name AS department,
    s.pay_type, s.base_salary, s.work_days, s.absent_days, s.unpaid_leave_days, s.late_min, s.ot_min, s.holiday_min, s.total_earnings,
    s.total_deductions, s.net_pay, s.taxable_income, s.sso_base, s.sso_employee, s.sso_employer, s.wht, s.pvd_employee, s.pvd_employer,
    p.tax_id, p.bank_name, p.bank_account
    FROM payslips s JOIN employees e ON e.id = s.employee_id LEFT JOIN departments d ON d.id = e.department_id
    LEFT JOIN employee_pay p ON p.employee_id = s.employee_id";

async fn slips(st: &AppState, run_id: i64) -> AppResult<Vec<SlipRow>> {
    Ok(sqlx::query_as(&format!("{SLIP_SELECT} WHERE s.run_id = ? ORDER BY e.employee_code")).bind(run_id).fetch_all(&st.db).await?)
}

async fn get_run(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need_any(&["payroll.view", "payroll.manage"])?;
    let run: RunSummary = sqlx::query_as(&format!("{RUN_SELECT} WHERE id = ?")).bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let slips = slips(&st, id).await?;
    let unconfigured: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT e.id, e.employee_code, CONCAT(e.first_name, ' ', e.last_name) FROM employees e
          LEFT JOIN employee_pay p ON p.employee_id = e.id WHERE e.status = 'active' AND p.employee_id IS NULL ORDER BY e.employee_code",
    )
    .fetch_all(&st.db)
    .await?;
    let sum = |f: fn(&SlipRow) -> Decimal| slips.iter().map(f).sum::<Decimal>();
    let totals = json!({
        "sso_employee": sum(|s| s.sso_employee), "sso_employer": sum(|s| s.sso_employer), "wht": sum(|s| s.wht),
        "pvd_employee": sum(|s| s.pvd_employee), "pvd_employer": sum(|s| s.pvd_employer),
    });
    Ok(Json(json!({
        "run": run, "payslips": slips, "totals": totals,
        "unconfigured": unconfigured.into_iter().map(|(i, c, n)| json!({ "id": i, "employee_code": c, "name": n })).collect::<Vec<_>>(),
    })))
}

async fn recompute(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("payroll.manage")?;
    let mut tx = st.db.begin().await?;
    let run = load_run(&mut *tx, id).await?;
    if run.status != "draft" {
        return Err(bad("คำนวณใหม่ได้เฉพาะรอบที่ยังเป็นฉบับร่าง"));
    }
    let n = compute_run(&st, &mut tx, &run).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "compute", "payroll_run", id, json!({ "employees": n })).await;
    Ok(Json(json!({ "ok": true, "employees": n })))
}

async fn approve(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("payroll.manage")?;
    let r = sqlx::query("UPDATE payroll_runs SET status = 'approved', approved_by = ?, approved_at = UTC_TIMESTAMP() WHERE id = ? AND status = 'draft' AND employees > 0")
        .bind(u.id())
        .bind(id)
        .execute(&st.db)
        .await?;
    if r.rows_affected() == 0 {
        return Err(bad("อนุมัติได้เฉพาะรอบฉบับร่างที่มีพนักงานอย่างน้อย 1 คน"));
    }
    // employees see their payslip from now on
    let emps: Vec<(i64,)> = sqlx::query_as("SELECT employee_id FROM payslips WHERE run_id = ?").bind(id).fetch_all(&st.db).await?;
    let run = load_run(&st.db, id).await?;
    for (e,) in emps {
        sqlx::query("INSERT INTO notifications (employee_id, kind, title, body) VALUES (?, 'payslip', ?, ?)")
            .bind(e)
            .bind(format!("สลิปเงินเดือน {:02}/{} พร้อมแล้ว", run.period_month, run.period_year + 543))
            .bind(format!("กำหนดจ่าย {}", run.pay_date.format("%d/%m/%Y")))
            .execute(&st.db)
            .await?;
    }
    audit(&st, Some(&u), "approve", "payroll_run", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize, Default)]
struct PayReq {
    /// cash / bank account that pays the salaries (default: bank)
    pay_account_id: Option<i64>,
}

async fn pay(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, body: Option<Json<PayReq>>) -> AppResult<Json<Value>> {
    u.need("payroll.manage")?;
    let req = body.map(|b| b.0).unwrap_or_default();
    let mut tx = st.db.begin().await?;
    let run = load_run(&mut *tx, id).await?;
    if run.status != "approved" {
        return Err(bad("บันทึกจ่ายได้เฉพาะรอบที่อนุมัติแล้ว"));
    }
    let pay_acc = match req.pay_account_id {
        Some(a) => {
            let t: Option<String> = sqlx::query_scalar("SELECT type FROM accounts WHERE id = ? AND is_active = 1").bind(a).fetch_optional(&mut *tx).await?;
            if t.as_deref() != Some("asset") {
                return Err(bad("บัญชีที่ใช้จ่ายต้องเป็นบัญชีสินทรัพย์ (เงินสด / ธนาคาร)"));
            }
            Acc::Id(a)
        }
        None => Acc::Role("bank"),
    };
    // totals by item code across all payslips
    let by_code: Vec<(String, String, Decimal)> = sqlx::query_as(
        "SELECT i.kind, i.code, SUM(i.amount) FROM payslip_items i JOIN payslips s ON s.id = i.payslip_id WHERE s.run_id = ? GROUP BY i.kind, i.code",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    let get = |kind: &str, codes: &[&str]| -> Decimal {
        by_code.iter().filter(|(k, c, _)| k == kind && codes.contains(&c.as_str())).map(|x| x.2).sum()
    };
    let (sso_er, pvd_er, net): (Decimal, Decimal, Decimal) = sqlx::query_as(
        "SELECT COALESCE(SUM(sso_employer), 0), COALESCE(SUM(pvd_employer), 0), COALESCE(SUM(net_pay), 0) FROM payslips WHERE run_id = ?",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    let salary = get("earning", &["salary", "allowance", "bonus", "other"]) - get("deduction", &["absence", "unpaid_leave", "late"]);
    let label = format!("เงินเดือน {:02}/{} ({})", run.period_month, run.period_year + 543, run.run_no);
    let lines = vec![
        Line::dr(Acc::Role("salary_expense"), salary).memo(&label),
        Line::dr(Acc::Role("ot_expense"), get("earning", &["ot", "holiday"])),
        Line::dr(Acc::Role("employee_payable"), get("earning", &["reimbursement"])).memo("จ่ายคืนค่าใช้จ่ายพนักงาน"),
        Line::dr(Acc::Role("sso_expense"), sso_er),
        Line::dr(Acc::Role("pvd_expense"), pvd_er),
        Line::cr(pay_acc, net).memo("จ่ายเงินเดือนสุทธิ"),
        Line::cr(Acc::Role("wht_payable"), get("deduction", &["wht"])).memo("ภ.ง.ด.1"),
        Line::cr(Acc::Role("sso_payable"), get("deduction", &["sso"]) + sso_er).memo("สปส.1-10"),
        Line::cr(Acc::Role("pvd_payable"), get("deduction", &["pvd"]) + pvd_er),
        Line::cr(Acc::Role("employee_receivable"), get("deduction", &["other"])),
    ];
    let je = erp::post(&mut tx, run.pay_date, &label, "payroll", Some(id), lines, Some(u.id())).await?;
    sqlx::query("UPDATE payroll_runs SET status = 'paid', paid_at = UTC_TIMESTAMP(), journal_entry_id = ? WHERE id = ?")
        .bind(je)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE expense_claims SET status = 'paid', paid_at = UTC_TIMESTAMP() WHERE payroll_run_id = ? AND status = 'approved'")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    audit(&st, Some(&u), "pay", "payroll_run", id, json!({ "journal_entry_id": je })).await;
    Ok(Json(json!({ "ok": true, "journal_entry_id": je })))
}

async fn void_run(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("payroll.manage")?;
    let mut tx = st.db.begin().await?;
    let run = load_run(&mut *tx, id).await?;
    if run.status == "void" {
        return Err(bad("รอบนี้ถูกยกเลิกแล้ว"));
    }
    if run.status == "paid" {
        erp::void_entries(&mut tx, "payroll", id).await?;
    }
    // claims go back to "approved" and will be picked up by the next run
    sqlx::query("UPDATE expense_claims SET status = 'approved', paid_at = NULL, payroll_run_id = NULL WHERE payroll_run_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE payroll_runs SET status = 'void' WHERE id = ?").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "void", "payroll_run", id, json!({ "was": run.status })).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- payslips

#[derive(Serialize, sqlx::FromRow)]
struct ItemRow {
    id: i64,
    kind: String,
    code: String,
    label: String,
    amount: Decimal,
    taxable: bool,
    manual: bool,
}

/// Payroll staff see every payslip; an employee only their own once the run is approved.
async fn slip_access(st: &AppState, u: &AuthUser, slip_id: i64) -> AppResult<(i64, i64, String)> {
    let row: Option<(i64, i64, String)> = sqlx::query_as(
        "SELECT s.run_id, s.employee_id, r.status FROM payslips s JOIN payroll_runs r ON r.id = s.run_id WHERE s.id = ?",
    )
    .bind(slip_id)
    .fetch_optional(&st.db)
    .await?;
    let (run_id, emp, status) = row.ok_or(AppError::NotFound)?;
    let own = u.0.emp == Some(emp) && ["approved", "paid"].contains(&status.as_str());
    if !(u.can("payroll.view") || u.can("payroll.manage") || own) {
        return Err(AppError::NotFound);
    }
    Ok((run_id, emp, status))
}

async fn slip_items(st: &AppState, slip_id: i64) -> AppResult<Vec<ItemRow>> {
    Ok(sqlx::query_as("SELECT id, kind, code, label, amount, taxable, manual FROM payslip_items WHERE payslip_id = ? ORDER BY sort, id")
        .bind(slip_id)
        .fetch_all(&st.db)
        .await?)
}

async fn get_payslip(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let (run_id, _, _) = slip_access(&st, &u, id).await?;
    let slip: SlipRow = sqlx::query_as(&format!("{SLIP_SELECT} WHERE s.id = ?")).bind(id).fetch_one(&st.db).await?;
    let run: RunSummary = sqlx::query_as(&format!("{RUN_SELECT} WHERE id = ?")).bind(run_id).fetch_one(&st.db).await?;
    Ok(Json(json!({ "payslip": slip, "run": run, "items": slip_items(&st, id).await? })))
}

#[derive(Deserialize)]
struct ItemReq {
    kind: String,
    code: String,
    label: String,
    amount: Decimal,
    taxable: Option<bool>,
}

async fn add_item(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<ItemReq>) -> AppResult<Json<Value>> {
    u.need("payroll.manage")?;
    let ok = match r.kind.as_str() {
        "earning" => ["bonus", "other"].contains(&r.code.as_str()),
        "deduction" => r.code == "other",
        _ => false,
    };
    if !ok {
        return Err(bad("เพิ่มได้เฉพาะ โบนัส / รายได้อื่น / รายการหักอื่น"));
    }
    if r.amount <= Decimal::ZERO || r.amount >= dec!(100000000) || r.label.trim().is_empty() {
        return Err(bad("กรุณาระบุรายการและจำนวนเงินที่มากกว่า 0"));
    }
    let mut tx = st.db.begin().await?;
    let (run_id, emp): (i64, i64) = sqlx::query_as("SELECT run_id, employee_id FROM payslips WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
    let run = load_run(&mut *tx, run_id).await?;
    if run.status != "draft" {
        return Err(bad("แก้ไขได้เฉพาะรอบที่ยังเป็นฉบับร่าง"));
    }
    sqlx::query("INSERT INTO payslip_items (payslip_id, kind, code, label, amount, taxable, manual, sort) VALUES (?,?,?,?,?,?,1,500)")
        .bind(id)
        .bind(&r.kind)
        .bind(&r.code)
        .bind(r.label.trim().chars().take(200).collect::<String>())
        .bind(r2(r.amount))
        .bind(r.kind == "earning" && r.taxable.unwrap_or(true))
        .execute(&mut *tx)
        .await?;
    // taxes / social security depend on the new item: recalculate the run
    compute_run(&st, &mut tx, &run).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "add_item", "payslip", id, json!({ "employee_id": emp, "code": r.code })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_item(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("payroll.manage")?;
    let mut tx = st.db.begin().await?;
    let (run_id, manual): (i64, bool) = sqlx::query_as(
        "SELECT s.run_id, i.manual FROM payslip_items i JOIN payslips s ON s.id = i.payslip_id WHERE i.id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let run = load_run(&mut *tx, run_id).await?;
    if run.status != "draft" || !manual {
        return Err(bad("ลบได้เฉพาะรายการที่เพิ่มเองในรอบฉบับร่าง"));
    }
    sqlx::query("DELETE FROM payslip_items WHERE id = ?").bind(id).execute(&mut *tx).await?;
    compute_run(&st, &mut tx, &run).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "delete_item", "payslip_item", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

async fn my_payslips(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    let emp = u.0.emp.ok_or(AppError::Forbidden)?;
    let rows: Vec<(i64, String, i32, i32, NaiveDate, String, Decimal, Decimal, Decimal)> = sqlx::query_as(
        "SELECT s.id, r.run_no, r.period_year, r.period_month, r.pay_date, r.status, s.total_earnings, s.total_deductions, s.net_pay
           FROM payslips s JOIN payroll_runs r ON r.id = s.run_id
          WHERE s.employee_id = ? AND r.status IN ('approved', 'paid') ORDER BY r.period_year DESC, r.period_month DESC LIMIT 60",
    )
    .bind(emp)
    .fetch_all(&st.db)
    .await?;
    Ok(Json(json!(rows
        .into_iter()
        .map(|r| json!({ "id": r.0, "run_no": r.1, "period_year": r.2, "period_month": r.3, "pay_date": r.4, "status": r.5,
                         "total_earnings": r.6, "total_deductions": r.7, "net_pay": r.8 }))
        .collect::<Vec<_>>())))
}

const TH_MONTHS: [&str; 12] = ["มกราคม", "กุมภาพันธ์", "มีนาคม", "เมษายน", "พฤษภาคม", "มิถุนายน", "กรกฎาคม", "สิงหาคม", "กันยายน", "ตุลาคม", "พฤศจิกายน", "ธันวาคม"];

pub fn th_date(d: NaiveDate) -> String {
    format!("{} {} {}", d.day(), TH_MONTHS[d.month0() as usize], d.year() + 543)
}

/// Shared A4 page styling for printed ERP documents.
pub const PRINT_CSS: &str = r#"@page{size:A4;margin:14mm}*{box-sizing:border-box}body{font-family:Tahoma,'Sarabun',sans-serif;font-size:12.5px;color:#111;margin:0;background:#e9edf2}
.page{background:#fff;width:210mm;min-height:297mm;margin:12px auto;padding:14mm;box-shadow:0 2px 10px rgba(0,0,0,.15)}
.toolbar{text-align:center;padding:10px}.toolbar button{font:inherit;padding:8px 16px;border-radius:8px;border:1px solid #888;background:#fff;cursor:pointer}
h1{font-size:19px;margin:0}h2{font-size:14px;margin:16px 0 6px}.muted{color:#555}.r{text-align:right}.c{text-align:center}
.head{display:flex;justify-content:space-between;gap:16px;border-bottom:2px solid #111;padding-bottom:10px;margin-bottom:12px}
.box{border:1px solid #bbb;border-radius:6px;padding:8px 10px}.grid2{display:grid;grid-template-columns:1fr 1fr;gap:10px}
table{width:100%;border-collapse:collapse;margin-top:6px}th,td{border:1px solid #bbb;padding:5px 7px;vertical-align:top}th{background:#f0f3f7}
.tot td{font-weight:bold;background:#f7f9fb}.big{font-size:16px;font-weight:bold}.sign{display:flex;justify-content:space-around;margin-top:46px;text-align:center}
.sign div{width:40%;border-top:1px dotted #333;padding-top:6px}.stamp{font-size:11px;border:1px solid #111;display:inline-block;padding:2px 8px;border-radius:4px}
@media print{body{background:#fff}.page{margin:0;box-shadow:none;width:auto;min-height:auto;padding:0}.toolbar{display:none}}"#;

async fn print_payslip(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Html<String>> {
    let (run_id, _, _) = slip_access(&st, &u, id).await?;
    let s: SlipRow = sqlx::query_as(&format!("{SLIP_SELECT} WHERE s.id = ?")).bind(id).fetch_one(&st.db).await?;
    let run = load_run(&st.db, run_id).await?;
    let items = slip_items(&st, id).await?;
    let co = company(&st).await;
    let (pos,): (Option<String>,) = sqlx::query_as("SELECT position FROM employees WHERE id = ?").bind(s.employee_id).fetch_one(&st.db).await?;
    let rows = |kind: &str| {
        items
            .iter()
            .filter(|i| i.kind == kind)
            .map(|i| format!("<tr><td>{}</td><td class=\"r\">{}</td></tr>", esc(&i.label), fmt_money(i.amount)))
            .collect::<String>()
    };
    // year-to-date figures for the employee (approved / paid runs of the same year up to this one)
    let (ytd_income, ytd_wht, ytd_sso): (Decimal, Decimal, Decimal) = sqlx::query_as(
        "SELECT COALESCE(SUM(s.taxable_income), 0), COALESCE(SUM(s.wht), 0), COALESCE(SUM(s.sso_employee), 0)
           FROM payslips s JOIN payroll_runs r ON r.id = s.run_id
          WHERE s.employee_id = ? AND r.period_year = ? AND r.period_month <= ? AND r.status IN ('approved', 'paid')",
    )
    .bind(s.employee_id)
    .bind(run.period_year)
    .bind(run.period_month)
    .fetch_one(&st.db)
    .await?;
    audit(&st, Some(&u), "view_payslip", "payslip", id, json!({})).await;
    let html = format!(
        r#"<!doctype html><html lang="th"><head><meta charset="utf-8"><title>สลิปเงินเดือน {code} {month}/{year}</title><style>{css}</style></head><body>
<div class="toolbar"><button onclick="window.print()">พิมพ์ / บันทึกเป็น PDF (A4)</button></div>
<div class="page">
<div class="head"><div><h1>{org}</h1><div class="muted">{addr}</div><div class="muted">เลขประจำตัวผู้เสียภาษี {taxid} {branch}</div></div>
<div class="r"><div class="big">ใบแจ้งเงินเดือน (Payslip)</div><div>งวด {month_th} {year}</div><div class="muted">{run_no} · จ่ายวันที่ {paydate}</div><div class="stamp">เอกสารลับเฉพาะบุคคล</div></div></div>
<div class="grid2"><div class="box"><b>{name}</b><br>รหัส {code} · {dept}<br>ตำแหน่ง {pos}</div>
<div class="box">เลขประจำตัวผู้เสียภาษี {emp_tax}<br>บัญชีรับเงิน {bank} {acct}<br>วันทำงาน {wd} วัน · ขาด {ab} · ลาไม่รับค่าจ้าง {ul} · OT {ot} ชม.</div></div>
<div class="grid2" style="margin-top:12px"><div><h2>รายได้</h2><table><tbody>{earn}</tbody><tfoot><tr class="tot"><td>รวมรายได้</td><td class="r">{te}</td></tr></tfoot></table></div>
<div><h2>รายการหัก</h2><table><tbody>{ded}</tbody><tfoot><tr class="tot"><td>รวมรายการหัก</td><td class="r">{td}</td></tr></tfoot></table></div></div>
<table style="margin-top:14px"><tr class="tot"><td>เงินได้สุทธิ (Net pay)</td><td class="r big">{net}</td></tr><tr><td colspan="2" class="c">({words})</td></tr></table>
<h2>สะสมทั้งปี {year}</h2><table><tr><th>เงินได้</th><th>ภาษีหัก ณ ที่จ่าย</th><th>ประกันสังคม</th><th>นายจ้างสมทบประกันสังคมเดือนนี้</th></tr>
<tr><td class="r">{ytd_i}</td><td class="r">{ytd_w}</td><td class="r">{ytd_s}</td><td class="r">{er}</td></tr></table>
<div class="sign"><div>ผู้จ่ายเงิน</div><div>ผู้รับเงิน</div></div>
</div></body></html>"#,
        css = PRINT_CSS,
        org = esc(co["name"].as_str().unwrap_or("")),
        addr = esc(co["address"].as_str().unwrap_or("")),
        taxid = esc(co["tax_id"].as_str().unwrap_or("-")),
        branch = esc(co["branch"].as_str().unwrap_or("")),
        code = esc(&s.employee_code),
        month = run.period_month,
        month_th = TH_MONTHS[(run.period_month - 1).clamp(0, 11) as usize],
        year = run.period_year + 543,
        run_no = esc(&run.run_no),
        paydate = th_date(run.pay_date),
        name = esc(&s.name),
        dept = esc(s.department.as_deref().unwrap_or("-")),
        pos = esc(pos.as_deref().unwrap_or("-")),
        emp_tax = esc(s.tax_id.as_deref().unwrap_or("-")),
        bank = esc(s.bank_name.as_deref().unwrap_or("-")),
        acct = esc(&mask_account(s.bank_account.as_deref())),
        wd = s.work_days,
        ab = s.absent_days,
        ul = s.unpaid_leave_days,
        ot = Decimal::from(s.ot_min) / dec!(60),
        earn = rows("earning"),
        ded = rows("deduction"),
        te = fmt_money(s.total_earnings),
        td = fmt_money(s.total_deductions),
        net = fmt_money(s.net_pay),
        words = erp::baht_text(s.net_pay),
        ytd_i = fmt_money(ytd_income),
        ytd_w = fmt_money(ytd_wht),
        ytd_s = fmt_money(ytd_sso),
        er = fmt_money(s.sso_employer),
    );
    Ok(Html(html))
}

fn mask_account(a: Option<&str>) -> String {
    match a {
        Some(a) if a.chars().count() > 4 => {
            let n = a.chars().count();
            format!("xxx-{}", a.chars().skip(n - 4).collect::<String>())
        }
        Some(a) => a.to_string(),
        None => "-".into(),
    }
}

// ---------------------------------------------------------------- exports

async fn export_run(st: &AppState, u: &AuthUser, id: i64) -> AppResult<(RunRow, Vec<SlipRow>)> {
    u.need_any(&["payroll.view", "payroll.manage"])?;
    let run = load_run(&st.db, id).await?;
    if run.status == "void" {
        return Err(bad("รอบนี้ถูกยกเลิกแล้ว"));
    }
    Ok((run, slips(st, id).await?))
}

fn file(name: &str, ctype: &str, body: Vec<u8>) -> Response {
    ([(header::CONTENT_TYPE, ctype.to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))], body).into_response()
}

async fn bank_csv(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Response> {
    let (run, slips) = export_run(&st, &u, id).await?;
    let mut sh = Sheet::new("bank", &["employee_code", "name", "bank", "account", "amount"]);
    for s in slips.iter().filter(|s| s.net_pay > Decimal::ZERO) {
        sh.push(vec![
            s.employee_code.clone().into(),
            s.name.clone().into(),
            s.bank_name.clone().into(),
            s.bank_account.clone().into(),
            s.net_pay.to_f64().unwrap_or(0.0).into(),
        ]);
    }
    audit(&st, Some(&u), "export_bank", "payroll_run", id, json!({})).await;
    let mut body = "\u{feff}".as_bytes().to_vec();
    body.extend(sh.to_csv().into_bytes());
    Ok(file(&format!("{}-bank.csv", run.run_no), "text/csv; charset=utf-8", body))
}

fn split_name(n: &str) -> (String, String) {
    match n.split_once(' ') {
        Some((a, b)) => (a.to_string(), b.trim().to_string()),
        None => (n.to_string(), String::new()),
    }
}

/// Attachment for ภ.ง.ด.1 (withholding on salaries, filed by the 7th of the next month).
async fn pnd1_xlsx(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Response> {
    let (run, slips) = export_run(&st, &u, id).await?;
    let mut sh = Sheet::new("ภ.ง.ด.1", &["ลำดับ", "เลขประจำตัวผู้เสียภาษี", "ชื่อ", "ชื่อสกุล", "วันที่จ่าย", "เงินได้ที่จ่าย", "ภาษีที่หักและนำส่ง", "เงื่อนไข"]);
    for (i, s) in slips.iter().enumerate() {
        let (first, last) = split_name(&s.name);
        sh.push(vec![
            ((i + 1) as i64).into(),
            s.tax_id.clone().into(),
            first.into(),
            last.into(),
            format!("{}/{}", run.pay_date.format("%d/%m"), run.pay_date.year() + 543).into(),
            s.taxable_income.to_f64().unwrap_or(0.0).into(),
            s.wht.to_f64().unwrap_or(0.0).into(),
            "1".into(),
        ]);
    }
    audit(&st, Some(&u), "export_pnd1", "payroll_run", id, json!({})).await;
    Ok(file(&format!("{}-pnd1.xlsx", run.run_no), "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", xlsx::workbook(&[sh])))
}

/// Contribution list for สปส.1-10 (social security, filed by the 15th of the next month).
async fn sso_xlsx(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Response> {
    let (run, slips) = export_run(&st, &u, id).await?;
    let mut sh = Sheet::new("สปส.1-10", &["ลำดับ", "เลขประจำตัวประชาชน", "ชื่อ - สกุล", "ค่าจ้างที่ใช้คำนวณ", "เงินสมทบผู้ประกันตน", "เงินสมทบนายจ้าง"]);
    let mut n = 0;
    for s in slips.iter().filter(|s| s.sso_employee > Decimal::ZERO) {
        n += 1;
        sh.push(vec![
            (n as i64).into(),
            s.tax_id.clone().into(),
            s.name.clone().into(),
            s.sso_base.to_f64().unwrap_or(0.0).into(),
            s.sso_employee.to_f64().unwrap_or(0.0).into(),
            s.sso_employer.to_f64().unwrap_or(0.0).into(),
        ]);
    }
    audit(&st, Some(&u), "export_sso", "payroll_run", id, json!({})).await;
    Ok(file(&format!("{}-sso.xlsx", run.run_no), "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", xlsx::workbook(&[sh])))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(salary: Decimal) -> PaySetup {
        PaySetup {
            employee_id: 1,
            pay_type: "monthly".into(),
            base_salary: salary,
            daily_rate: Decimal::ZERO,
            fixed_allowance: Decimal::ZERO,
            tax_id: None,
            bank_name: None,
            bank_account: None,
            sso_enabled: true,
            pvd_rate: Decimal::ZERO,
            pvd_employer_rate: Decimal::ZERO,
            tax_allowance_annual: Decimal::ZERO,
        }
    }

    fn settings() -> PaySettings {
        PaySettings {
            sso_rate: dec!(5),
            sso_min: dec!(1650),
            sso_max: dec!(17500),
            ot_rate: dec!(1.5),
            holiday_rate: dec!(1),
            holiday_ot_rate: dec!(3),
            deduct_absence: true,
            deduct_late: false,
        }
    }

    #[test]
    fn salary_50000() {
        let c = calculate(&setup(dec!(50000)), &settings(), Facts { work_days: 22, paid_days: 22, ..Default::default() }, vec![], vec![]);
        assert_eq!(c.sso_ee, dec!(875));
        // (600,000 - 100,000 - 60,000 - 10,500) -> 20,450 / 12
        assert_eq!(c.wht, dec!(1704.17));
        let net: Decimal = c.items.iter().map(|i| if i.kind == "earning" { i.amount } else { -i.amount }).sum();
        assert_eq!(net, dec!(50000) - dec!(875) - dec!(1704.17));
    }

    #[test]
    fn low_salary_no_tax_sso_min() {
        let c = calculate(&setup(dec!(15000)), &settings(), Facts::default(), vec![], vec![]);
        assert_eq!(c.sso_ee, dec!(750));
        assert_eq!(c.wht, dec!(0));
    }

    #[test]
    fn overtime_and_absence() {
        // 30,000/month: daily 1,000, hourly 125; 10 h OT -> 1,875; 1 absent day -> -1,000;
        // holiday 10 h: 8 h x 1 + 2 h x 3 = 14 h x 125 = 1,750
        let f = Facts { work_days: 22, paid_days: 21, absent_days: 1, ot_min: 600, holiday_mins: vec![600], ..Default::default() };
        let c = calculate(&setup(dec!(30000)), &settings(), f, vec![], vec![]);
        let amt = |code: &str| c.items.iter().find(|i| i.code == code).map(|i| i.amount).unwrap();
        assert_eq!(amt("ot"), dec!(1875));
        assert_eq!(amt("holiday"), dec!(1750));
        assert_eq!(amt("absence"), dec!(1000));
        assert_eq!(c.taxable, dec!(32625));
        assert_eq!(c.sso_ee, dec!(875));
    }

    #[test]
    fn bonus_taxed_on_difference_and_reimbursement_untaxed() {
        let bonus = Item { kind: "earning", code: "bonus".into(), label: "โบนัส".into(), amount: dec!(100000), taxable: true, manual: true };
        let reimb = Item::earn("reimbursement", "EX", dec!(2500), false);
        let c = calculate(&setup(dec!(50000)), &settings(), Facts::default(), vec![bonus], vec![reimb]);
        // regular 20,450/12 = 1,704.17 + (tax on 700,000 with same deductions - 20,450)
        // 700,000 - 100,000 - 60,000 - 10,500 = 529,500 -> 27,500 + 4,425 = 31,925 ; diff 11,475
        assert_eq!(c.wht, dec!(13179.17));
        assert_eq!(c.sso_ee, dec!(875));
        assert_eq!(c.taxable, dec!(150000));
    }

    #[test]
    fn daily_paid() {
        let mut p = setup(Decimal::ZERO);
        p.pay_type = "daily".into();
        p.daily_rate = dec!(400);
        let c = calculate(&p, &settings(), Facts { paid_days: 20, holiday_mins: vec![480], ..Default::default() }, vec![], vec![]);
        let amt = |code: &str| c.items.iter().find(|i| i.code == code).map(|i| i.amount).unwrap();
        assert_eq!(amt("salary"), dec!(8000));
        // holiday work for daily staff: 2x (8 h x 50 x 2)
        assert_eq!(amt("holiday"), dec!(800));
        assert_eq!(c.sso_ee, dec!(440));
    }
}
