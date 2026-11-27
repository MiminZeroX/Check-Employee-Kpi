//! HR: shifts, shift assignments, holidays, leave types, leave / off-site / overtime
//! requests with approval, leave balances, work sessions and the org chart.

use std::collections::HashSet;

use axum::{
    extract::{Path, Query, State},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::{audit, AuthUser},
    error::{bad, AppError, AppResult},
    hooks,
    routes::Range,
    schedule::Calendar,
    state::AppState,
    util::{date_range, day_range, from_local, local_date_of, local_today},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/shifts", get(list_shifts).post(create_shift))
        .route("/shifts/{id}", put(update_shift).delete(delete_shift))
        .route("/shift-assignments", get(list_assignments).post(assign_shifts))
        .route("/shift-assignments/{id}", axum::routing::delete(delete_assignment))
        .route("/holidays", get(list_holidays).post(create_holiday))
        .route("/holidays/{id}", axum::routing::delete(delete_holiday))
        .route("/holidays/import-fixed", post(import_fixed_holidays))
        .route("/leave-types", get(list_leave_types).post(create_leave_type))
        .route("/leave-types/{id}", put(update_leave_type).delete(delete_leave_type))
        .route("/requests", get(list_requests).post(create_request))
        .route("/requests/{id}/decide", post(decide_request))
        .route("/requests/{id}/cancel", post(cancel_request))
        .route("/leave-balances", get(leave_balances))
        .route("/approvals", get(approvals))
        .route("/work-sessions", get(work_sessions))
        .route("/me/work-sessions", get(my_work_sessions))
        .route("/org-chart", get(org_chart))
}

fn parse_time(s: &str) -> AppResult<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M:%S")
        .or_else(|_| NaiveTime::parse_from_str(s.trim(), "%H:%M"))
        .map_err(|_| bad(format!("รูปแบบเวลาไม่ถูกต้อง: {s}")))
}

/// Employees whose requests this user may approve: None = everybody (HR / admin),
/// otherwise the people in the departments they manage, including sub-departments.
pub async fn managed_employees(st: &AppState, u: &AuthUser) -> AppResult<Option<HashSet<i64>>> {
    if u.can("hr.manage") {
        return Ok(None);
    }
    let Some(me) = u.0.emp else { return Ok(Some(HashSet::new())) };
    let depts: Vec<(i64, Option<i64>, Option<i64>)> =
        sqlx::query_as("SELECT id, parent_id, manager_employee_id FROM departments").fetch_all(&st.db).await?;
    let mut managed: HashSet<i64> = depts.iter().filter(|d| d.2 == Some(me)).map(|d| d.0).collect();
    loop {
        let more: Vec<i64> = depts
            .iter()
            .filter(|d| d.1.is_some_and(|p| managed.contains(&p)) && !managed.contains(&d.0))
            .map(|d| d.0)
            .collect();
        if more.is_empty() {
            break;
        }
        managed.extend(more);
    }
    let emps: Vec<(i64, Option<i64>)> = sqlx::query_as("SELECT id, department_id FROM employees").fetch_all(&st.db).await?;
    Ok(Some(emps.into_iter().filter(|e| e.1.is_some_and(|d| managed.contains(&d))).map(|e| e.0).collect()))
}

// ---------------------------------------------------------------- shifts

#[derive(Serialize, sqlx::FromRow)]
struct ShiftRow {
    id: i64,
    name: String,
    start_time: NaiveTime,
    end_time: NaiveTime,
    break_start: Option<NaiveTime>,
    break_end: Option<NaiveTime>,
    work_days: String,
    grace_min: i32,
    is_default: bool,
    employee_count: i64,
}

async fn list_shifts(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Vec<ShiftRow>>> {
    u.need_any(&["hr.manage", "attendance.view", "employees.view"])?;
    Ok(Json(
        sqlx::query_as(
            "SELECT s.id, s.name, s.start_time, s.end_time, s.break_start, s.break_end, s.work_days, s.grace_min, s.is_default,
               (SELECT COUNT(*) FROM employees e WHERE e.shift_id = s.id AND e.status = 'active') AS employee_count
             FROM shifts s ORDER BY s.id",
        )
        .fetch_all(&st.db)
        .await?,
    ))
}

#[derive(Deserialize)]
struct ShiftReq {
    name: String,
    start_time: String,
    end_time: String,
    break_start: Option<String>,
    break_end: Option<String>,
    work_days: Option<String>,
    grace_min: Option<i32>,
    is_default: Option<bool>,
}

impl ShiftReq {
    fn parsed(&self) -> AppResult<(NaiveTime, NaiveTime, Option<NaiveTime>, Option<NaiveTime>, String)> {
        if self.name.trim().is_empty() {
            return Err(bad("กรุณาระบุชื่อกะ"));
        }
        let opt = |v: &Option<String>| v.as_deref().filter(|s| !s.trim().is_empty()).map(parse_time).transpose();
        let days = self.work_days.clone().unwrap_or_else(|| "1,2,3,4,5".into());
        if days.split(',').any(|d| !matches!(d.trim().parse::<u32>(), Ok(1..=7))) {
            return Err(bad("วันทำงานต้องเป็นเลข 1-7 คั่นด้วยจุลภาค (1 = จันทร์)"));
        }
        Ok((parse_time(&self.start_time)?, parse_time(&self.end_time)?, opt(&self.break_start)?, opt(&self.break_end)?, days))
    }
}

async fn save_default(st: &AppState, id: i64, is_default: bool) -> AppResult<()> {
    if is_default {
        sqlx::query("UPDATE shifts SET is_default = (id = ?)").bind(id).execute(&st.db).await?;
    }
    Ok(())
}

async fn create_shift(State(st): State<AppState>, u: AuthUser, Json(r): Json<ShiftReq>) -> AppResult<Json<Value>> {
    u.need("hr.manage")?;
    let (s, e, bs, be, days) = r.parsed()?;
    let id = sqlx::query("INSERT INTO shifts (name, start_time, end_time, break_start, break_end, work_days, grace_min) VALUES (?,?,?,?,?,?,?)")
        .bind(r.name.trim())
        .bind(s)
        .bind(e)
        .bind(bs)
        .bind(be)
        .bind(days)
        .bind(r.grace_min.unwrap_or(0).clamp(0, 120))
        .execute(&st.db)
        .await?
        .last_insert_id() as i64;
    save_default(&st, id, r.is_default.unwrap_or(false)).await?;
    audit(&st, Some(&u), "create", "shift", id, json!({ "name": r.name })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_shift(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<ShiftReq>) -> AppResult<Json<Value>> {
    u.need("hr.manage")?;
    let (s, e, bs, be, days) = r.parsed()?;
    sqlx::query("UPDATE shifts SET name=?, start_time=?, end_time=?, break_start=?, break_end=?, work_days=?, grace_min=? WHERE id=?")
        .bind(r.name.trim())
        .bind(s)
        .bind(e)
        .bind(bs)
        .bind(be)
        .bind(days)
        .bind(r.grace_min.unwrap_or(0).clamp(0, 120))
        .bind(id)
        .execute(&st.db)
        .await?;
    save_default(&st, id, r.is_default.unwrap_or(false)).await?;
    st.mark_dirty([local_today(st.cfg.tz_offset_min)]).await;
    audit(&st, Some(&u), "update", "shift", id, json!({ "name": r.name })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_shift(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("hr.manage")?;
    sqlx::query("DELETE FROM shifts WHERE id = ?").bind(id).execute(&st.db).await?;
    audit(&st, Some(&u), "delete", "shift", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- shift assignments

#[derive(Serialize, sqlx::FromRow)]
struct AssignmentRow {
    id: i64,
    employee_id: i64,
    employee_name: Option<String>,
    work_date: NaiveDate,
    shift_id: Option<i64>,
    shift_name: Option<String>,
    note: Option<String>,
}

async fn list_assignments(State(st): State<AppState>, u: AuthUser, Query(r): Query<Range>) -> AppResult<Json<Vec<AssignmentRow>>> {
    u.need_any(&["hr.manage", "attendance.view"])?;
    let (from, to) = date_range(r.from, r.to, st.cfg.tz_offset_min)?;
    Ok(Json(
        sqlx::query_as(
            "SELECT a.id, a.employee_id, CONCAT(e.first_name, ' ', e.last_name) AS employee_name, a.work_date, a.shift_id,
               s.name AS shift_name, a.note
             FROM shift_assignments a JOIN employees e ON e.id = a.employee_id LEFT JOIN shifts s ON s.id = a.shift_id
             WHERE a.work_date BETWEEN ? AND ? ORDER BY a.work_date, e.employee_code",
        )
        .bind(from)
        .bind(to)
        .fetch_all(&st.db)
        .await?,
    ))
}

#[derive(Deserialize)]
struct AssignReq {
    employee_ids: Vec<i64>,
    from: NaiveDate,
    to: NaiveDate,
    /// None = day off
    shift_id: Option<i64>,
    /// ISO weekdays to apply (default: every day in the range)
    weekdays: Option<Vec<u32>>,
    note: Option<String>,
}

async fn assign_shifts(State(st): State<AppState>, u: AuthUser, Json(r): Json<AssignReq>) -> AppResult<Json<Value>> {
    u.need("hr.manage")?;
    if r.to < r.from || (r.to - r.from).num_days() > 366 {
        return Err(bad("ช่วงวันที่ไม่ถูกต้อง (สูงสุด 1 ปี)"));
    }
    if r.employee_ids.is_empty() || r.employee_ids.len() > 1000 {
        return Err(bad("กรุณาเลือกพนักงาน"));
    }
    let mut n = 0;
    let mut d = r.from;
    let mut dates = Vec::new();
    while d <= r.to {
        if r.weekdays.as_ref().is_none_or(|w| w.contains(&d.weekday().number_from_monday())) {
            for e in &r.employee_ids {
                sqlx::query(
                    "INSERT INTO shift_assignments (employee_id, work_date, shift_id, note) VALUES (?,?,?,?)
                     ON DUPLICATE KEY UPDATE shift_id = VALUES(shift_id), note = VALUES(note)",
                )
                .bind(e)
                .bind(d)
                .bind(r.shift_id)
                .bind(&r.note)
                .execute(&st.db)
                .await?;
                n += 1;
            }
            dates.push(d);
        }
        d += Duration::days(1);
    }
    st.mark_dirty(dates.into_iter().filter(|d| *d <= local_today(st.cfg.tz_offset_min))).await;
    audit(&st, Some(&u), "assign_shift", "employee", format!("{} people", r.employee_ids.len()), json!({ "from": r.from, "to": r.to, "shift_id": r.shift_id })).await;
    Ok(Json(json!({ "assigned": n })))
}

async fn delete_assignment(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("hr.manage")?;
    sqlx::query("DELETE FROM shift_assignments WHERE id = ?").bind(id).execute(&st.db).await?;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- holidays

#[derive(Serialize, sqlx::FromRow)]
struct HolidayRow {
    id: i64,
    holiday_date: NaiveDate,
    name: String,
    branch_id: Option<i64>,
    branch_name: Option<String>,
}

#[derive(Deserialize)]
struct YearQ {
    year: Option<i32>,
    employee_id: Option<i64>,
}

async fn list_holidays(State(st): State<AppState>, _u: AuthUser, Query(q): Query<YearQ>) -> AppResult<Json<Vec<HolidayRow>>> {
    let year = q.year.unwrap_or_else(|| local_today(st.cfg.tz_offset_min).year());
    Ok(Json(
        sqlx::query_as(
            "SELECT h.id, h.holiday_date, h.name, h.branch_id, b.name AS branch_name FROM holidays h
             LEFT JOIN branches b ON b.id = h.branch_id WHERE YEAR(h.holiday_date) = ? ORDER BY h.holiday_date",
        )
        .bind(year)
        .fetch_all(&st.db)
        .await?,
    ))
}

#[derive(Deserialize)]
struct HolidayReq {
    holiday_date: NaiveDate,
    name: String,
    branch_id: Option<i64>,
}

async fn create_holiday(State(st): State<AppState>, u: AuthUser, Json(r): Json<HolidayReq>) -> AppResult<Json<Value>> {
    u.need("hr.manage")?;
    if r.name.trim().is_empty() {
        return Err(bad("กรุณาระบุชื่อวันหยุด"));
    }
    let id = sqlx::query("INSERT INTO holidays (holiday_date, name, branch_id) VALUES (?,?,?)")
        .bind(r.holiday_date)
        .bind(r.name.trim())
        .bind(r.branch_id)
        .execute(&st.db)
        .await?
        .last_insert_id();
    st.mark_dirty([r.holiday_date]).await;
    audit(&st, Some(&u), "create", "holiday", id, json!({ "date": r.holiday_date, "name": r.name })).await;
    Ok(Json(json!({ "id": id })))
}

async fn delete_holiday(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("hr.manage")?;
    sqlx::query("DELETE FROM holidays WHERE id = ?").bind(id).execute(&st.db).await?;
    Ok(Json(json!({ "ok": true })))
}

/// Thai public holidays that fall on the same date every year. Holidays that follow the
/// lunar calendar (Makha / Visakha / Asanha Bucha, Khao Phansa) and substitution days
/// change every year and must be added from the official announcement.
async fn import_fixed_holidays(State(st): State<AppState>, u: AuthUser, Json(q): Json<Value>) -> AppResult<Json<Value>> {
    u.need("hr.manage")?;
    let year = q["year"].as_i64().ok_or_else(|| bad("กรุณาระบุปี (ค.ศ.)"))? as i32;
    let fixed = [
        (1, 1, "วันขึ้นปีใหม่"),
        (4, 6, "วันจักรี"),
        (4, 13, "วันสงกรานต์"),
        (4, 14, "วันสงกรานต์"),
        (4, 15, "วันสงกรานต์"),
        (5, 1, "วันแรงงานแห่งชาติ"),
        (5, 4, "วันฉัตรมงคล"),
        (6, 3, "วันเฉลิมพระชนมพรรษาสมเด็จพระราชินี"),
        (7, 28, "วันเฉลิมพระชนมพรรษาพระบาทสมเด็จพระเจ้าอยู่หัว"),
        (8, 12, "วันแม่แห่งชาติ"),
        (10, 13, "วันนวมินทรมหาราช"),
        (10, 23, "วันปิยมหาราช"),
        (12, 5, "วันพ่อแห่งชาติ"),
        (12, 10, "วันรัฐธรรมนูญ"),
        (12, 31, "วันสิ้นปี"),
    ];
    let mut n = 0;
    for (m, d, name) in fixed {
        let Some(date) = NaiveDate::from_ymd_opt(year, m, d) else { continue };
        let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM holidays WHERE holiday_date = ? AND branch_id IS NULL")
            .bind(date)
            .fetch_optional(&st.db)
            .await?;
        if exists.is_none() {
            sqlx::query("INSERT INTO holidays (holiday_date, name) VALUES (?, ?)").bind(date).bind(name).execute(&st.db).await?;
            n += 1;
        }
    }
    audit(&st, Some(&u), "import_holidays", "holiday", year, json!({ "added": n })).await;
    Ok(Json(json!({ "added": n })))
}

// ---------------------------------------------------------------- leave types

#[derive(Serialize, sqlx::FromRow)]
struct LeaveTypeRow {
    id: i64,
    code: String,
    name: String,
    annual_quota_days: f64,
    paid: bool,
    is_active: bool,
}

async fn list_leave_types(State(st): State<AppState>, _u: AuthUser) -> AppResult<Json<Vec<LeaveTypeRow>>> {
    Ok(Json(
        sqlx::query_as("SELECT id, code, name, annual_quota_days, paid, is_active FROM leave_types ORDER BY id")
            .fetch_all(&st.db)
            .await?,
    ))
}

#[derive(Deserialize)]
struct LeaveTypeReq {
    code: String,
    name: String,
    annual_quota_days: f64,
    paid: Option<bool>,
    is_active: Option<bool>,
}

async fn create_leave_type(State(st): State<AppState>, u: AuthUser, Json(r): Json<LeaveTypeReq>) -> AppResult<Json<Value>> {
    u.need("hr.manage")?;
    if r.code.trim().is_empty() || r.name.trim().is_empty() || !(0.0..=366.0).contains(&r.annual_quota_days) {
        return Err(bad("กรุณาระบุรหัส ชื่อ และโควตา (0-366 วัน)"));
    }
    let id = sqlx::query("INSERT INTO leave_types (code, name, annual_quota_days, paid, is_active) VALUES (?,?,?,?,?)")
        .bind(r.code.trim())
        .bind(r.name.trim())
        .bind(r.annual_quota_days)
        .bind(r.paid.unwrap_or(true))
        .bind(r.is_active.unwrap_or(true))
        .execute(&st.db)
        .await?
        .last_insert_id();
    audit(&st, Some(&u), "create", "leave_type", id, json!({ "code": r.code })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_leave_type(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<LeaveTypeReq>) -> AppResult<Json<Value>> {
    u.need("hr.manage")?;
    sqlx::query("UPDATE leave_types SET code=?, name=?, annual_quota_days=?, paid=?, is_active=? WHERE id=?")
        .bind(r.code.trim())
        .bind(r.name.trim())
        .bind(r.annual_quota_days.clamp(0.0, 366.0))
        .bind(r.paid.unwrap_or(true))
        .bind(r.is_active.unwrap_or(true))
        .bind(id)
        .execute(&st.db)
        .await?;
    audit(&st, Some(&u), "update", "leave_type", id, json!({ "code": r.code })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_leave_type(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("hr.manage")?;
    sqlx::query("DELETE FROM leave_types WHERE id = ?").bind(id).execute(&st.db).await?;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- requests

#[derive(Serialize, sqlx::FromRow)]
pub struct RequestRow {
    id: i64,
    employee_id: i64,
    employee_name: Option<String>,
    department_name: Option<String>,
    kind: String,
    leave_type_id: Option<i64>,
    leave_type_name: Option<String>,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    days: Option<f64>,
    hours: Option<f64>,
    location: Option<String>,
    reason: String,
    status: String,
    approver_name: Option<String>,
    approver_note: Option<String>,
    decided_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

const REQ_SELECT: &str = "SELECT r.id, r.employee_id, CONCAT(e.first_name, ' ', e.last_name) AS employee_name,
    d.name AS department_name, r.kind, r.leave_type_id, lt.name AS leave_type_name, r.start_at, r.end_at, r.days, r.hours,
    r.location, r.reason, r.status, us.display_name AS approver_name, r.approver_note, r.decided_at, r.created_at
    FROM requests r JOIN employees e ON e.id = r.employee_id LEFT JOIN departments d ON d.id = e.department_id
    LEFT JOIN leave_types lt ON lt.id = r.leave_type_id LEFT JOIN users us ON us.id = r.approver_user_id";

#[derive(Deserialize)]
struct ReqFilter {
    status: Option<String>,
    kind: Option<String>,
    employee_id: Option<i64>,
    mine: Option<bool>,
}

async fn list_requests(State(st): State<AppState>, u: AuthUser, Query(f): Query<ReqFilter>) -> AppResult<Json<Vec<RequestRow>>> {
    // own requests, or (approvers) the people they may approve
    let scope = if f.mine == Some(true) || !u.can("requests.approve") && !u.can("attendance.view") {
        Some(u.0.emp.into_iter().collect::<HashSet<_>>())
    } else if u.can("attendance.view") {
        None
    } else {
        managed_employees(&st, &u).await?
    };
    let mut sql = format!("{REQ_SELECT} WHERE 1=1");
    if f.status.is_some() {
        sql.push_str(" AND r.status = ?");
    }
    if f.kind.is_some() {
        sql.push_str(" AND r.kind = ?");
    }
    if f.employee_id.is_some() {
        sql.push_str(" AND r.employee_id = ?");
    }
    sql.push_str(" ORDER BY r.status = 'pending' DESC, r.start_at DESC LIMIT 1000");
    let mut q = sqlx::query_as::<_, RequestRow>(&sql);
    if let Some(s) = &f.status {
        q = q.bind(s);
    }
    if let Some(k) = &f.kind {
        q = q.bind(k);
    }
    if let Some(e) = f.employee_id {
        q = q.bind(e);
    }
    let rows = q.fetch_all(&st.db).await?;
    Ok(Json(match scope {
        Some(set) => rows.into_iter().filter(|r| set.contains(&r.employee_id)).collect(),
        None => rows,
    }))
}

#[derive(Deserialize)]
struct NewRequest {
    employee_id: Option<i64>,
    kind: String,
    leave_type_id: Option<i64>,
    start_date: NaiveDate,
    end_date: Option<NaiveDate>,
    /// leave: full | am | pm
    half_day: Option<String>,
    start_time: Option<String>,
    end_time: Option<String>,
    hours: Option<f64>,
    location: Option<String>,
    reason: String,
}

async fn create_request(State(st): State<AppState>, u: AuthUser, Json(r): Json<NewRequest>) -> AppResult<Json<Value>> {
    let off = st.cfg.tz_offset_min;
    // for someone else: HR or that person's approver
    let emp = match r.employee_id {
        Some(e) if Some(e) != u.0.emp => {
            u.need("requests.approve")?;
            if let Some(set) = managed_employees(&st, &u).await? {
                if !set.contains(&e) {
                    return Err(AppError::Forbidden);
                }
            }
            e
        }
        _ => u.0.emp.ok_or_else(|| bad("บัญชีนี้ไม่ได้ผูกกับข้อมูลพนักงาน"))?,
    };
    if r.reason.trim().chars().count() < 3 {
        return Err(bad("กรุณาระบุเหตุผล"));
    }
    let end_date = r.end_date.unwrap_or(r.start_date);
    if end_date < r.start_date || (end_date - r.start_date).num_days() > 180 {
        return Err(bad("ช่วงวันที่ไม่ถูกต้อง"));
    }
    let cal = Calendar::load(&st.db, off, r.start_date, end_date).await?;
    let sched = cal.day(emp, r.start_date);
    let at = |d: NaiveDate, t: NaiveTime| from_local(d.and_time(t), off);
    let (start_at, end_at, days, hours) = match r.kind.as_str() {
        "leave" => {
            let lt = r.leave_type_id.ok_or_else(|| bad("กรุณาเลือกประเภทการลา"))?;
            let half = r.half_day.as_deref().unwrap_or("full");
            let (s, e) = match (half, sched.start, sched.end) {
                ("am", Some(s), Some(e)) => (s, s + (e - s) / 2),
                ("pm", Some(s), Some(e)) => (s + (e - s) / 2, e),
                ("am" | "pm", _, _) => return Err(bad("วันนี้ไม่มีกะทำงาน จึงลาครึ่งวันไม่ได้")),
                _ => (day_range(r.start_date, off).0, day_range(end_date, off).1 - Duration::seconds(1)),
            };
            let mut days = cal.working_days(emp, r.start_date, end_date);
            if half != "full" {
                days = days.min(1.0) * 0.5;
            }
            if days <= 0.0 {
                return Err(bad("ช่วงที่เลือกไม่มีวันทำงาน (เป็นวันหยุดทั้งหมด)"));
            }
            // quota check
            let (quota, name): (f64, String) = sqlx::query_as("SELECT annual_quota_days, name FROM leave_types WHERE id = ? AND is_active = 1")
                .bind(lt)
                .fetch_optional(&st.db)
                .await?
                .ok_or_else(|| bad("ประเภทการลาไม่ถูกต้อง"))?;
            if quota > 0.0 {
                let used = used_days(&st, emp, lt, r.start_date.year()).await?;
                if used + days > quota + 1e-9 {
                    return Err(bad(format!("{name} คงเหลือ {:.1} วัน แต่ขอ {days:.1} วัน", (quota - used).max(0.0))));
                }
            }
            (s, e, Some(days), None)
        }
        "offsite" => {
            let s = match (&r.start_time, sched.start) {
                (Some(t), _) => at(r.start_date, parse_time(t)?),
                (None, Some(s)) => s,
                _ => day_range(r.start_date, off).0,
            };
            let e = match (&r.end_time, cal.day(emp, end_date).end) {
                (Some(t), _) => at(end_date, parse_time(t)?),
                (None, Some(e)) => e,
                _ => day_range(end_date, off).1 - Duration::seconds(1),
            };
            if r.location.as_deref().unwrap_or("").trim().is_empty() {
                return Err(bad("กรุณาระบุสถานที่ปฏิบัติงาน"));
            }
            (s, e, None, Some(((e - s).num_minutes() as f64 / 60.0 * 10.0).round() / 10.0))
        }
        "overtime" => {
            let s = match (&r.start_time, sched.end) {
                (Some(t), _) => at(r.start_date, parse_time(t)?),
                (None, Some(e)) => e,
                _ => return Err(bad("กรุณาระบุเวลาเริ่ม OT")),
            };
            let h = r.hours.filter(|h| *h > 0.0 && *h <= 16.0).ok_or_else(|| bad("ชั่วโมง OT ต้องอยู่ระหว่าง 0-16"))?;
            (s, s + Duration::minutes((h * 60.0) as i64), None, Some(h))
        }
        _ => return Err(bad("ประเภทคำขอไม่ถูกต้อง")),
    };
    if end_at <= start_at {
        return Err(bad("เวลาสิ้นสุดต้องหลังเวลาเริ่ม"));
    }
    let clash: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM requests WHERE employee_id = ? AND kind = ? AND status IN ('pending','approved') AND start_at < ? AND end_at > ? LIMIT 1",
    )
    .bind(emp)
    .bind(&r.kind)
    .bind(end_at)
    .bind(start_at)
    .fetch_optional(&st.db)
    .await?;
    if clash.is_some() {
        return Err(bad("มีคำขอประเภทเดียวกันในช่วงเวลานี้อยู่แล้ว"));
    }
    let id = sqlx::query(
        "INSERT INTO requests (employee_id, kind, leave_type_id, start_at, end_at, days, hours, location, reason, created_by_user_id)
         VALUES (?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(emp)
    .bind(&r.kind)
    .bind(if r.kind == "leave" { r.leave_type_id } else { None })
    .bind(start_at)
    .bind(end_at)
    .bind(days)
    .bind(hours)
    .bind(r.location.as_deref().map(str::trim))
    .bind(r.reason.trim())
    .bind(u.id())
    .execute(&st.db)
    .await?
    .last_insert_id();
    audit(&st, Some(&u), "create_request", "request", id, json!({ "kind": r.kind, "employee_id": emp })).await;
    st.broadcast(json!({ "type": "request", "id": id }));
    hooks::emit(&st, "request.created", json!({ "id": id, "employee_id": emp, "kind": r.kind, "start_at": start_at, "end_at": end_at, "days": days, "hours": hours }));
    Ok(Json(json!({ "id": id, "days": days, "hours": hours })))
}

async fn used_days(st: &AppState, emp: i64, leave_type: i64, year: i32) -> AppResult<f64> {
    let (used,): (Option<f64>,) = sqlx::query_as(
        "SELECT SUM(days) FROM requests WHERE employee_id = ? AND leave_type_id = ? AND kind = 'leave'
           AND status IN ('approved','pending') AND YEAR(start_at) = ?",
    )
    .bind(emp)
    .bind(leave_type)
    .bind(year)
    .fetch_one(&st.db)
    .await?;
    Ok(used.unwrap_or(0.0))
}

#[derive(Deserialize)]
struct DecideReq {
    approve: bool,
    note: Option<String>,
}

async fn decide_request(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<DecideReq>) -> AppResult<Json<Value>> {
    u.need("requests.approve")?;
    let (emp, kind, status, start_at, end_at): (i64, String, String, DateTime<Utc>, DateTime<Utc>) =
        sqlx::query_as("SELECT employee_id, kind, status, start_at, end_at FROM requests WHERE id = ?")
            .bind(id)
            .fetch_one(&st.db)
            .await?;
    if status != "pending" {
        return Err(bad("คำขอนี้ได้รับการพิจารณาแล้ว"));
    }
    if Some(emp) == u.0.emp {
        return Err(bad("ไม่สามารถอนุมัติคำขอของตนเองได้"));
    }
    if let Some(set) = managed_employees(&st, &u).await? {
        if !set.contains(&emp) {
            return Err(AppError::Forbidden);
        }
    }
    if !r.approve && r.note.as_deref().unwrap_or("").trim().is_empty() {
        return Err(bad("กรุณาระบุเหตุผลที่ไม่อนุมัติ"));
    }
    let new_status = if r.approve { "approved" } else { "rejected" };
    sqlx::query("UPDATE requests SET status = ?, approver_user_id = ?, approver_note = ?, decided_at = UTC_TIMESTAMP() WHERE id = ?")
        .bind(new_status)
        .bind(u.id())
        .bind(&r.note)
        .bind(id)
        .execute(&st.db)
        .await?;
    let kind_th = match kind.as_str() {
        "leave" => "ลา",
        "offsite" => "ปฏิบัติงานนอกสถานที่",
        _ => "ทำงานล่วงเวลา (OT)",
    };
    sqlx::query("INSERT INTO notifications (employee_id, kind, title, body) VALUES (?, 'request_result', ?, ?)")
        .bind(emp)
        .bind(format!("คำขอ{kind_th}{}", if r.approve { "ได้รับการอนุมัติ" } else { "ไม่ได้รับการอนุมัติ" }))
        .bind(r.note.clone().unwrap_or_default())
        .execute(&st.db)
        .await?;
    // past days are re-evaluated (late / absent / overtime)
    let off = st.cfg.tz_offset_min;
    let today = local_today(off);
    let mut d = local_date_of(start_at, off);
    let last = local_date_of(end_at, off).min(today);
    let mut dates = Vec::new();
    while d <= last {
        dates.push(d);
        d += Duration::days(1);
    }
    st.mark_dirty(dates).await;
    audit(&st, Some(&u), new_status, "request", id, json!({ "note": r.note })).await;
    hooks::emit(&st, "request.decided", json!({ "id": id, "employee_id": emp, "kind": kind, "status": new_status }));
    Ok(Json(json!({ "ok": true })))
}

async fn cancel_request(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let (emp, status): (i64, String) = sqlx::query_as("SELECT employee_id, status FROM requests WHERE id = ?").bind(id).fetch_one(&st.db).await?;
    if Some(emp) != u.0.emp && !u.can("hr.manage") {
        return Err(AppError::Forbidden);
    }
    if status != "pending" {
        return Err(bad("ยกเลิกได้เฉพาะคำขอที่รอพิจารณา"));
    }
    sqlx::query("UPDATE requests SET status = 'cancelled' WHERE id = ?").bind(id).execute(&st.db).await?;
    audit(&st, Some(&u), "cancel", "request", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

async fn leave_balances(State(st): State<AppState>, u: AuthUser, Query(q): Query<YearQ>) -> AppResult<Json<Value>> {
    let emp = match q.employee_id {
        Some(e) if Some(e) != u.0.emp => {
            u.need_any(&["requests.approve", "attendance.view"])?;
            e
        }
        _ => u.0.emp.ok_or_else(|| bad("บัญชีนี้ไม่ได้ผูกกับข้อมูลพนักงาน"))?,
    };
    let year = q.year.unwrap_or_else(|| local_today(st.cfg.tz_offset_min).year());
    let types: Vec<(i64, String, String, f64, bool)> =
        sqlx::query_as("SELECT id, code, name, annual_quota_days, paid FROM leave_types WHERE is_active = 1 ORDER BY id")
            .fetch_all(&st.db)
            .await?;
    let used: Vec<(i64, String, f64)> = sqlx::query_as(
        "SELECT leave_type_id, status, CAST(SUM(days) AS DOUBLE) FROM requests WHERE employee_id = ? AND kind = 'leave'
           AND status IN ('approved','pending') AND YEAR(start_at) = ? AND leave_type_id IS NOT NULL GROUP BY leave_type_id, status",
    )
    .bind(emp)
    .bind(year)
    .fetch_all(&st.db)
    .await
    .unwrap_or_default();
    let rows: Vec<Value> = types
        .into_iter()
        .map(|(id, code, name, quota, paid)| {
            let g = |s: &str| used.iter().filter(|u| u.0 == id && u.1 == s).map(|u| u.2).sum::<f64>();
            let (approved, pending) = (g("approved"), g("pending"));
            json!({ "leave_type_id": id, "code": code, "name": name, "quota": quota, "paid": paid, "used": approved,
                    "pending": pending, "remaining": if quota > 0.0 { Some((quota - approved - pending).max(0.0)) } else { None } })
        })
        .collect();
    Ok(Json(json!({ "employee_id": emp, "year": year, "rows": rows })))
}

/// Everything waiting for this user's decision.
async fn approvals(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    let mut out = json!({ "requests": [], "corrections": 0 });
    if u.can("requests.approve") {
        let scope = managed_employees(&st, &u).await?;
        let rows: Vec<RequestRow> = sqlx::query_as(&format!("{REQ_SELECT} WHERE r.status = 'pending' ORDER BY r.start_at"))
            .fetch_all(&st.db)
            .await?;
        let rows: Vec<RequestRow> = rows
            .into_iter()
            .filter(|r| scope.as_ref().is_none_or(|s| s.contains(&r.employee_id)) && Some(r.employee_id) != u.0.emp)
            .collect();
        out["requests"] = json!(rows);
    }
    if u.can("privacy.review") {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM data_corrections WHERE status = 'pending'").fetch_one(&st.db).await?;
        out["corrections"] = json!(n);
    }
    Ok(Json(out))
}

// ---------------------------------------------------------------- work sessions

#[derive(Serialize, sqlx::FromRow)]
pub struct WorkSessionRow {
    pub employee_id: i64,
    pub employee_code: String,
    pub name: String,
    pub department: Option<String>,
    pub work_date: NaiveDate,
    pub day_type: String,
    pub status: String,
    pub shift_name: Option<String>,
    pub scheduled_start: Option<DateTime<Utc>>,
    pub scheduled_end: Option<DateTime<Utc>>,
    pub office_entry_at: Option<DateTime<Utc>>,
    pub check_in: Option<DateTime<Utc>>,
    pub check_out: Option<DateTime<Utc>>,
    pub first_desk_at: Option<DateTime<Utc>>,
    pub worked_sec: i32,
    pub desk_sec: i32,
    pub break_sec: i32,
    pub late_min: i32,
    pub early_leave_min: i32,
    pub overtime_min: i32,
    pub approved_overtime_min: i32,
}

pub const WS_SELECT: &str = "SELECT w.employee_id, e.employee_code, CONCAT(e.first_name, ' ', e.last_name) AS name,
    d.name AS department, w.work_date, w.day_type, w.status, s.name AS shift_name, w.scheduled_start, w.scheduled_end,
    w.office_entry_at, w.check_in, w.check_out, w.first_desk_at, w.worked_sec, w.desk_sec, w.break_sec, w.late_min,
    w.early_leave_min, w.overtime_min, w.approved_overtime_min
    FROM work_sessions w JOIN employees e ON e.id = w.employee_id LEFT JOIN departments d ON d.id = e.department_id
    LEFT JOIN shifts s ON s.id = w.shift_id";

pub async fn sessions(st: &AppState, from: NaiveDate, to: NaiveDate, emp: Option<i64>, dept: Option<i64>, branch: Option<i64>) -> AppResult<Vec<WorkSessionRow>> {
    let mut sql = format!("{WS_SELECT} WHERE w.work_date BETWEEN ? AND ? AND e.status = 'active'");
    if emp.is_some() {
        sql.push_str(" AND w.employee_id = ?");
    }
    if dept.is_some() {
        sql.push_str(" AND e.department_id = ?");
    }
    if branch.is_some() {
        sql.push_str(" AND d.branch_id = ?");
    }
    sql.push_str(" ORDER BY w.work_date DESC, e.employee_code LIMIT 5000");
    let mut q = sqlx::query_as::<_, WorkSessionRow>(&sql).bind(from).bind(to);
    for v in [emp, dept, branch].into_iter().flatten() {
        q = q.bind(v);
    }
    Ok(q.fetch_all(&st.db).await?)
}

async fn work_sessions(State(st): State<AppState>, u: AuthUser, Query(r): Query<Range>) -> AppResult<Json<Vec<WorkSessionRow>>> {
    u.need("attendance.view")?;
    let (from, to) = date_range(r.from, r.to, st.cfg.tz_offset_min)?;
    Ok(Json(sessions(&st, from, to, r.employee_id, r.department_id, r.branch_id).await?))
}

async fn my_work_sessions(State(st): State<AppState>, u: AuthUser, Query(r): Query<Range>) -> AppResult<Json<Vec<WorkSessionRow>>> {
    let emp = u.0.emp.ok_or_else(|| bad("บัญชีนี้ไม่ได้ผูกกับข้อมูลพนักงาน"))?;
    let (from, to) = date_range(r.from, r.to, st.cfg.tz_offset_min)?;
    Ok(Json(sessions(&st, from, to, Some(emp), None, None).await?))
}

// ---------------------------------------------------------------- org chart

async fn org_chart(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    u.need("employees.view")?;
    let depts: Vec<(i64, String, Option<i64>, Option<i64>, Option<String>, Option<String>, Option<String>, i64)> = sqlx::query_as(
        "SELECT d.id, d.name, d.parent_id, d.manager_employee_id, CONCAT(m.first_name, ' ', m.last_name), m.position, b.name,
           (SELECT COUNT(*) FROM employees e WHERE e.department_id = d.id AND e.status = 'active')
         FROM departments d LEFT JOIN employees m ON m.id = d.manager_employee_id LEFT JOIN branches b ON b.id = d.branch_id
         ORDER BY d.name",
    )
    .fetch_all(&st.db)
    .await?;
    let people: Vec<(i64, Option<i64>, String, Option<String>)> = sqlx::query_as(
        "SELECT id, department_id, CONCAT(first_name, ' ', last_name), position FROM employees WHERE status = 'active' ORDER BY employee_code",
    )
    .fetch_all(&st.db)
    .await?;
    let nodes: Vec<Value> = depts
        .into_iter()
        .map(|(id, name, parent, mgr, mgr_name, mgr_pos, branch, n)| {
            let members: Vec<Value> = people
                .iter()
                .filter(|p| p.1 == Some(id))
                .map(|p| json!({ "id": p.0, "name": p.2, "position": p.3, "is_manager": Some(p.0) == mgr }))
                .collect();
            json!({ "id": id, "name": name, "parent_id": parent, "manager_employee_id": mgr, "manager_name": mgr_name,
                    "manager_position": mgr_pos, "branch": branch, "employee_count": n, "members": members })
        })
        .collect();
    Ok(Json(json!({ "departments": nodes })))
}
