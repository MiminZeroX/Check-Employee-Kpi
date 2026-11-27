//! Activity analytics, timelines, employee self-service, data corrections,
//! notifications and wellness.

use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::{audit, AuthUser},
    error::{bad, AppError, AppResult},
    routes::Range,
    state::AppState,
    summary::recompute_day,
    util::{date_range, day_range, is_valid_state, local_date_of, local_today},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/activity/events", get(list_events))
        .route("/activity/timeline", get(timeline))
        .route("/activity/summary", get(summary))
        .route("/activity/daily", get(daily))
        .route("/activity/recompute", post(recompute))
        .route("/me/summary", get(my_summary))
        .route("/me/daily", get(my_daily))
        .route("/me/events", get(my_events))
        .route("/me/corrections", get(my_corrections).post(create_correction))
        .route("/me/notifications", get(my_notifications))
        .route("/corrections", get(list_corrections))
        .route("/corrections/{id}/review", post(review_correction))
        .route("/notifications", get(list_notifications))
        .route("/notifications/{id}/read", post(read_notification))
        .route("/wellness/overview", get(wellness))
}

// ---------------------------------------------------------------- events

#[derive(Serialize, sqlx::FromRow)]
pub struct EventRow {
    pub id: i64,
    pub workstation_id: i64,
    pub workstation_name: Option<String>,
    pub camera_id: i64,
    pub employee_id: Option<i64>,
    pub employee_name: Option<String>,
    pub state: String,
    pub original_state: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub duration_sec: i32,
    pub confidence: f64,
    pub review_status: String,
}

const EVENT_SELECT: &str = "SELECT a.id, a.workstation_id, w.name AS workstation_name, a.camera_id, a.employee_id,
    CONCAT(e.first_name, ' ', e.last_name) AS employee_name, COALESCE(a.corrected_state, a.state) AS state,
    a.state AS original_state, a.started_at, a.ended_at, a.duration_sec, a.confidence, a.review_status
    FROM activity_events a LEFT JOIN workstations w ON w.id = a.workstation_id
    LEFT JOIN employees e ON e.id = a.employee_id";

#[derive(Deserialize)]
struct EventFilter {
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    employee_id: Option<i64>,
    state: Option<String>,
    limit: Option<i64>,
}

async fn query_events(st: &AppState, f: &EventFilter, employee: Option<i64>) -> AppResult<Vec<EventRow>> {
    let off = st.cfg.tz_offset_min;
    let (from, to) = date_range(f.from, f.to, off)?;
    let (start, _) = day_range(from, off);
    let (_, end) = day_range(to, off);
    let mut sql = format!("{EVENT_SELECT} WHERE a.started_at < ? AND a.ended_at > ?");
    if employee.is_some() {
        sql.push_str(" AND a.employee_id = ?");
    }
    if f.state.is_some() {
        sql.push_str(" AND COALESCE(a.corrected_state, a.state) = ?");
    }
    sql.push_str(" ORDER BY a.started_at DESC LIMIT ?");
    let mut q = sqlx::query_as::<_, EventRow>(&sql).bind(end).bind(start);
    if let Some(e) = employee {
        q = q.bind(e);
    }
    if let Some(s) = &f.state {
        q = q.bind(s);
    }
    Ok(q.bind(f.limit.unwrap_or(500).clamp(1, 5000)).fetch_all(&st.db).await?)
}

async fn list_events(State(st): State<AppState>, u: AuthUser, Query(f): Query<EventFilter>) -> AppResult<Json<Vec<EventRow>>> {
    u.need("analytics.view")?;
    Ok(Json(query_events(&st, &f, f.employee_id).await?))
}

// ---------------------------------------------------------------- timeline

#[derive(Deserialize)]
struct TimelineQ {
    date: Option<NaiveDate>,
    department_id: Option<i64>,
    employee_id: Option<i64>,
}

pub async fn build_timeline(
    st: &AppState,
    date: NaiveDate,
    department_id: Option<i64>,
    employee_id: Option<i64>,
) -> AppResult<Vec<Value>> {
    let (start, end) = day_range(date, st.cfg.tz_offset_min);
    let mut emp_sql = String::from(
        "SELECT e.id, CONCAT(e.first_name, ' ', e.last_name), e.employee_code, d.name FROM employees e
         LEFT JOIN departments d ON d.id = e.department_id WHERE e.status = 'active'",
    );
    if department_id.is_some() {
        emp_sql.push_str(" AND e.department_id = ?");
    }
    if employee_id.is_some() {
        emp_sql.push_str(" AND e.id = ?");
    }
    emp_sql.push_str(" ORDER BY e.employee_code");
    let mut q = sqlx::query_as::<_, (i64, String, String, Option<String>)>(&emp_sql);
    if let Some(d) = department_id {
        q = q.bind(d);
    }
    if let Some(e) = employee_id {
        q = q.bind(e);
    }
    let emps = q.fetch_all(&st.db).await?;

    let segs: Vec<(i64, String, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT employee_id, COALESCE(corrected_state, state), started_at, ended_at FROM activity_events
         WHERE employee_id IS NOT NULL AND review_status <> 'rejected' AND started_at < ? AND ended_at > ?
         ORDER BY employee_id, started_at",
    )
    .bind(end)
    .bind(start)
    .fetch_all(&st.db)
    .await?;
    let scans: Vec<(i64, DateTime<Utc>, String, String)> = sqlx::query_as(
        "SELECT employee_id, scan_time, direction, method FROM attendance_logs
         WHERE employee_id IS NOT NULL AND scan_time >= ? AND scan_time < ? ORDER BY scan_time",
    )
    .bind(start)
    .bind(end)
    .fetch_all(&st.db)
    .await?;

    Ok(emps
        .into_iter()
        .map(|(id, name, code, dept)| {
            let s: Vec<Value> = segs
                .iter()
                .filter(|x| x.0 == id)
                .map(|x| json!({ "state": x.1, "start": x.2.max(start), "end": x.3.min(end) }))
                .collect();
            let sc: Vec<Value> = scans
                .iter()
                .filter(|x| x.0 == id)
                .map(|x| json!({ "time": x.1, "direction": x.2, "method": x.3 }))
                .collect();
            json!({ "employee_id": id, "name": name, "employee_code": code, "department": dept, "segments": s, "scans": sc })
        })
        .collect())
}

async fn timeline(State(st): State<AppState>, u: AuthUser, Query(q): Query<TimelineQ>) -> AppResult<Json<Value>> {
    u.need("analytics.view")?;
    let date = q.date.unwrap_or_else(|| local_today(st.cfg.tz_offset_min));
    let (start, end) = day_range(date, st.cfg.tz_offset_min);
    let rows = build_timeline(&st, date, q.department_id, q.employee_id).await?;
    Ok(Json(json!({ "date": date, "start": start, "end": end, "employees": rows })))
}

// ---------------------------------------------------------------- summaries

#[derive(Serialize, sqlx::FromRow, Clone)]
pub struct SummaryRow {
    pub employee_id: i64,
    pub employee_code: String,
    pub name: String,
    pub position: Option<String>,
    pub department_name: Option<String>,
    pub days: i64,
    pub focused_sec: i64,
    pub present_sec: i64,
    pub phone_sec: i64,
    pub looking_away_sec: i64,
    pub away_sec: i64,
    pub unknown_sec: i64,
    pub phone_count: i64,
    pub away_count: i64,
    pub longest_sitting_sec: i64,
    pub wellness_alerts: i64,
    pub avg_scan_to_desk_sec: Option<i64>,
}

pub async fn summaries(
    st: &AppState,
    from: NaiveDate,
    to: NaiveDate,
    department_id: Option<i64>,
    employee_id: Option<i64>,
) -> AppResult<Vec<SummaryRow>> {
    let mut sql = String::from(
        "SELECT e.id AS employee_id, e.employee_code, CONCAT(e.first_name, ' ', e.last_name) AS name, e.position,
          d.name AS department_name,
          CAST(COUNT(s.work_date) AS SIGNED) AS days,
          CAST(COALESCE(SUM(s.focused_sec), 0) AS SIGNED) AS focused_sec,
          CAST(COALESCE(SUM(s.present_sec), 0) AS SIGNED) AS present_sec,
          CAST(COALESCE(SUM(s.phone_sec), 0) AS SIGNED) AS phone_sec,
          CAST(COALESCE(SUM(s.looking_away_sec), 0) AS SIGNED) AS looking_away_sec,
          CAST(COALESCE(SUM(s.away_sec), 0) AS SIGNED) AS away_sec,
          CAST(COALESCE(SUM(s.unknown_sec), 0) AS SIGNED) AS unknown_sec,
          CAST(COALESCE(SUM(s.phone_count), 0) AS SIGNED) AS phone_count,
          CAST(COALESCE(SUM(s.away_count), 0) AS SIGNED) AS away_count,
          CAST(COALESCE(MAX(s.longest_sitting_sec), 0) AS SIGNED) AS longest_sitting_sec,
          CAST(COALESCE(SUM(s.wellness_alerts), 0) AS SIGNED) AS wellness_alerts,
          CAST(AVG(s.scan_to_desk_sec) AS SIGNED) AS avg_scan_to_desk_sec
         FROM employees e LEFT JOIN departments d ON d.id = e.department_id
         LEFT JOIN daily_summaries s ON s.employee_id = e.id AND s.work_date BETWEEN ? AND ?
         WHERE e.status = 'active'",
    );
    if department_id.is_some() {
        sql.push_str(" AND e.department_id = ?");
    }
    if employee_id.is_some() {
        sql.push_str(" AND e.id = ?");
    }
    sql.push_str(" GROUP BY e.id, e.employee_code, e.first_name, e.last_name, e.position, d.name ORDER BY e.employee_code");
    let mut q = sqlx::query_as::<_, SummaryRow>(&sql).bind(from).bind(to);
    if let Some(d) = department_id {
        q = q.bind(d);
    }
    if let Some(e) = employee_id {
        q = q.bind(e);
    }
    Ok(q.fetch_all(&st.db).await?)
}

async fn summary(State(st): State<AppState>, u: AuthUser, Query(r): Query<Range>) -> AppResult<Json<Value>> {
    u.need("analytics.view")?;
    let (from, to) = date_range(r.from, r.to, st.cfg.tz_offset_min)?;
    let rows = summaries(&st, from, to, r.department_id, r.employee_id).await?;
    Ok(Json(json!({ "from": from, "to": to, "rows": rows })))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct DailyRow {
    pub work_date: NaiveDate,
    pub first_scan_at: Option<DateTime<Utc>>,
    pub last_scan_at: Option<DateTime<Utc>>,
    pub first_desk_at: Option<DateTime<Utc>>,
    pub scan_to_desk_sec: Option<i32>,
    pub focused_sec: i32,
    pub present_sec: i32,
    pub phone_sec: i32,
    pub looking_away_sec: i32,
    pub away_sec: i32,
    pub unknown_sec: i32,
    pub phone_count: i32,
    pub away_count: i32,
    pub longest_sitting_sec: i32,
    pub wellness_alerts: i32,
}

pub async fn daily_rows(st: &AppState, emp: i64, from: NaiveDate, to: NaiveDate) -> AppResult<Vec<DailyRow>> {
    Ok(sqlx::query_as(
        "SELECT work_date, first_scan_at, last_scan_at, first_desk_at, scan_to_desk_sec, focused_sec, present_sec,
           phone_sec, looking_away_sec, away_sec, unknown_sec, phone_count, away_count, longest_sitting_sec, wellness_alerts
         FROM daily_summaries WHERE employee_id = ? AND work_date BETWEEN ? AND ? ORDER BY work_date",
    )
    .bind(emp)
    .bind(from)
    .bind(to)
    .fetch_all(&st.db)
    .await?)
}

async fn daily(State(st): State<AppState>, u: AuthUser, Query(r): Query<Range>) -> AppResult<Json<Vec<DailyRow>>> {
    u.need("analytics.view")?;
    let emp = r.employee_id.ok_or_else(|| bad("employee_id is required"))?;
    let (from, to) = date_range(r.from, r.to, st.cfg.tz_offset_min)?;
    Ok(Json(daily_rows(&st, emp, from, to).await?))
}

#[derive(Deserialize)]
struct RecomputeReq {
    from: NaiveDate,
    to: NaiveDate,
}

async fn recompute(State(st): State<AppState>, u: AuthUser, Json(r): Json<RecomputeReq>) -> AppResult<Json<Value>> {
    u.need("backup.manage")?;
    let (from, to) = date_range(Some(r.from), Some(r.to), st.cfg.tz_offset_min)?;
    let mut d = from;
    let mut n = 0;
    while d <= to {
        n += recompute_day(&st, d).await?;
        d = d.succ_opt().unwrap();
    }
    audit(&st, Some(&u), "recompute", "daily_summaries", format!("{from}..{to}"), json!({})).await;
    Ok(Json(json!({ "rows": n })))
}

// ---------------------------------------------------------------- self-service

fn my_id(u: &AuthUser) -> AppResult<i64> {
    u.0.emp.ok_or_else(|| bad("บัญชีนี้ไม่ได้ผูกกับข้อมูลพนักงาน"))
}

async fn my_summary(State(st): State<AppState>, u: AuthUser, Query(r): Query<Range>) -> AppResult<Json<Value>> {
    let emp = my_id(&u)?;
    let (from, to) = date_range(r.from, r.to, st.cfg.tz_offset_min)?;
    let rows = summaries(&st, from, to, None, Some(emp)).await?;
    Ok(Json(json!({ "from": from, "to": to, "summary": rows.into_iter().next() })))
}

async fn my_daily(State(st): State<AppState>, u: AuthUser, Query(r): Query<Range>) -> AppResult<Json<Vec<DailyRow>>> {
    let emp = my_id(&u)?;
    let (from, to) = date_range(r.from, r.to, st.cfg.tz_offset_min)?;
    Ok(Json(daily_rows(&st, emp, from, to).await?))
}

async fn my_events(State(st): State<AppState>, u: AuthUser, Query(f): Query<EventFilter>) -> AppResult<Json<Vec<EventRow>>> {
    let emp = my_id(&u)?;
    Ok(Json(query_events(&st, &f, Some(emp)).await?))
}

#[derive(Serialize, sqlx::FromRow)]
struct CorrectionRow {
    id: i64,
    activity_event_id: i64,
    employee_id: i64,
    employee_name: Option<String>,
    reason: String,
    requested_state: Option<String>,
    status: String,
    review_note: Option<String>,
    created_at: DateTime<Utc>,
    reviewed_at: Option<DateTime<Utc>>,
    event_state: Option<String>,
    event_started_at: Option<DateTime<Utc>>,
    event_ended_at: Option<DateTime<Utc>>,
}

const CORR_SELECT: &str = "SELECT c.id, c.activity_event_id, c.employee_id,
    CONCAT(e.first_name, ' ', e.last_name) AS employee_name, c.reason, c.requested_state, c.status, c.review_note,
    c.created_at, c.reviewed_at, a.state AS event_state, a.started_at AS event_started_at, a.ended_at AS event_ended_at
    FROM data_corrections c LEFT JOIN employees e ON e.id = c.employee_id
    LEFT JOIN activity_events a ON a.id = c.activity_event_id";

async fn my_corrections(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Vec<CorrectionRow>>> {
    let emp = my_id(&u)?;
    Ok(Json(
        sqlx::query_as(&format!("{CORR_SELECT} WHERE c.employee_id = ? ORDER BY c.id DESC LIMIT 200"))
            .bind(emp)
            .fetch_all(&st.db)
            .await?,
    ))
}

#[derive(Deserialize)]
struct CorrectionReq {
    activity_event_id: i64,
    reason: String,
    requested_state: Option<String>,
}

async fn create_correction(State(st): State<AppState>, u: AuthUser, Json(r): Json<CorrectionReq>) -> AppResult<Json<Value>> {
    let emp = my_id(&u)?;
    if r.reason.trim().chars().count() < 5 {
        return Err(bad("กรุณาระบุเหตุผลอย่างน้อย 5 ตัวอักษร"));
    }
    let requested = r.requested_state.filter(|s| !s.is_empty());
    if let Some(s) = &requested {
        if !is_valid_state(s) {
            return Err(bad("สถานะที่ขอแก้ไขไม่ถูกต้อง"));
        }
    }
    let owner: Option<(Option<i64>,)> = sqlx::query_as("SELECT employee_id FROM activity_events WHERE id = ?")
        .bind(r.activity_event_id)
        .fetch_optional(&st.db)
        .await?;
    if owner.map(|o| o.0) != Some(Some(emp)) {
        return Err(AppError::Forbidden);
    }
    let id = sqlx::query(
        "INSERT INTO data_corrections (activity_event_id, employee_id, reason, requested_state) VALUES (?,?,?,?)",
    )
    .bind(r.activity_event_id)
    .bind(emp)
    .bind(r.reason.trim())
    .bind(&requested)
    .execute(&st.db)
    .await?
    .last_insert_id();
    sqlx::query("UPDATE activity_events SET review_status = 'disputed' WHERE id = ? AND review_status = 'auto'")
        .bind(r.activity_event_id)
        .execute(&st.db)
        .await?;
    audit(&st, Some(&u), "request_correction", "activity_event", r.activity_event_id, json!({ "correction_id": id })).await;
    st.broadcast(json!({ "type": "correction", "id": id }));
    Ok(Json(json!({ "id": id })))
}

#[derive(Deserialize)]
struct CorrFilter {
    status: Option<String>,
}

async fn list_corrections(
    State(st): State<AppState>,
    u: AuthUser,
    Query(f): Query<CorrFilter>,
) -> AppResult<Json<Vec<CorrectionRow>>> {
    u.need("privacy.review")?;
    let rows = match f.status {
        Some(s) => {
            sqlx::query_as(&format!("{CORR_SELECT} WHERE c.status = ? ORDER BY c.id DESC LIMIT 500"))
                .bind(s)
                .fetch_all(&st.db)
                .await?
        }
        None => sqlx::query_as(&format!("{CORR_SELECT} ORDER BY c.id DESC LIMIT 500")).fetch_all(&st.db).await?,
    };
    Ok(Json(rows))
}

#[derive(Deserialize)]
struct ReviewReq {
    approve: bool,
    /// New state when approving; omit to discard the segment entirely.
    corrected_state: Option<String>,
    note: Option<String>,
}

async fn review_correction(
    State(st): State<AppState>,
    u: AuthUser,
    Path(id): Path<i64>,
    Json(r): Json<ReviewReq>,
) -> AppResult<Json<Value>> {
    u.need("privacy.review")?;
    let (event_id, emp, status, requested): (i64, i64, String, Option<String>) = sqlx::query_as(
        "SELECT activity_event_id, employee_id, status, requested_state FROM data_corrections WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&st.db)
    .await?;
    if status != "pending" {
        return Err(bad("คำขอนี้ได้รับการพิจารณาแล้ว"));
    }
    let new_state = r.corrected_state.filter(|s| !s.is_empty()).or(requested);
    if let Some(s) = &new_state {
        if !is_valid_state(s) {
            return Err(bad("สถานะไม่ถูกต้อง"));
        }
    }
    if r.approve {
        match &new_state {
            Some(s) => {
                sqlx::query("UPDATE activity_events SET review_status = 'corrected', corrected_state = ? WHERE id = ?")
                    .bind(s)
                    .bind(event_id)
                    .execute(&st.db)
                    .await?
            }
            None => {
                sqlx::query("UPDATE activity_events SET review_status = 'rejected' WHERE id = ?")
                    .bind(event_id)
                    .execute(&st.db)
                    .await?
            }
        };
    } else {
        sqlx::query("UPDATE activity_events SET review_status = 'auto' WHERE id = ? AND review_status = 'disputed'")
            .bind(event_id)
            .execute(&st.db)
            .await?;
    }
    sqlx::query(
        "UPDATE data_corrections SET status = ?, reviewer_user_id = ?, review_note = ?, reviewed_at = UTC_TIMESTAMP() WHERE id = ?",
    )
    .bind(if r.approve { "approved" } else { "rejected" })
    .bind(u.id())
    .bind(&r.note)
    .bind(id)
    .execute(&st.db)
    .await?;
    sqlx::query("INSERT INTO notifications (employee_id, kind, title, body) VALUES (?, 'correction_result', ?, ?)")
        .bind(emp)
        .bind(if r.approve { "คำขอแก้ไขข้อมูลได้รับการอนุมัติ" } else { "คำขอแก้ไขข้อมูลไม่ได้รับการอนุมัติ" })
        .bind(r.note.clone().unwrap_or_default())
        .execute(&st.db)
        .await?;
    let started: (DateTime<Utc>,) = sqlx::query_as("SELECT started_at FROM activity_events WHERE id = ?")
        .bind(event_id)
        .fetch_one(&st.db)
        .await?;
    st.mark_dirty([local_date_of(started.0, st.cfg.tz_offset_min)]).await;
    audit(&st, Some(&u), if r.approve { "approve_correction" } else { "reject_correction" }, "data_correction", id, json!({ "state": new_state })).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- notifications

#[derive(Serialize, sqlx::FromRow)]
struct NotificationRow {
    id: i64,
    employee_id: Option<i64>,
    employee_name: Option<String>,
    kind: String,
    title: String,
    body: Option<String>,
    is_read: bool,
    created_at: DateTime<Utc>,
}

const NOTIF_SELECT: &str = "SELECT n.id, n.employee_id, CONCAT(e.first_name, ' ', e.last_name) AS employee_name,
    n.kind, n.title, n.body, n.is_read, n.created_at FROM notifications n LEFT JOIN employees e ON e.id = n.employee_id";

async fn list_notifications(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Vec<NotificationRow>>> {
    u.need("notifications.view")?;
    Ok(Json(sqlx::query_as(&format!("{NOTIF_SELECT} ORDER BY n.id DESC LIMIT 300")).fetch_all(&st.db).await?))
}

async fn my_notifications(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Vec<NotificationRow>>> {
    let emp = my_id(&u)?;
    Ok(Json(
        sqlx::query_as(&format!("{NOTIF_SELECT} WHERE n.employee_id = ? ORDER BY n.id DESC LIMIT 100"))
            .bind(emp)
            .fetch_all(&st.db)
            .await?,
    ))
}

async fn read_notification(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    if !u.can("notifications.view") {
        sqlx::query("UPDATE notifications SET is_read = 1 WHERE id = ? AND employee_id = ?")
            .bind(id)
            .bind(my_id(&u)?)
            .execute(&st.db)
            .await?;
    } else {
        sqlx::query("UPDATE notifications SET is_read = 1 WHERE id = ?").bind(id).execute(&st.db).await?;
    }
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- wellness

async fn wellness(State(st): State<AppState>, u: AuthUser, Query(r): Query<Range>) -> AppResult<Json<Value>> {
    u.need("wellness.view")?;
    let off = st.cfg.tz_offset_min;
    let (from, to) = date_range(r.from, r.to, off)?;
    let rows: Vec<(i64, String, String, Option<String>, NaiveTime, i64, i64, i64, Option<i64>)> = sqlx::query_as(
        "SELECT e.id, e.employee_code, CONCAT(e.first_name, ' ', e.last_name), d.name, e.work_end,
           CAST(COUNT(s.work_date) AS SIGNED),
           CAST(COALESCE(SUM(s.wellness_alerts), 0) AS SIGNED),
           CAST(COALESCE(MAX(s.longest_sitting_sec), 0) AS SIGNED),
           CAST(AVG(NULLIF(s.longest_sitting_sec, 0)) AS SIGNED)
         FROM employees e LEFT JOIN departments d ON d.id = e.department_id
         LEFT JOIN daily_summaries s ON s.employee_id = e.id AND s.work_date BETWEEN ? AND ?
         WHERE e.status = 'active'
         GROUP BY e.id, e.employee_code, e.first_name, e.last_name, d.name, e.work_end
         ORDER BY e.employee_code",
    )
    .bind(from)
    .bind(to)
    .fetch_all(&st.db)
    .await?;

    // overtime per day from the work sessions (shift end, holidays and days off are applied there)
    let overtime: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT employee_id, CAST(SUM(overtime_min) AS SIGNED) FROM work_sessions WHERE work_date BETWEEN ? AND ? GROUP BY employee_id",
    )
    .bind(from)
    .bind(to)
    .fetch_all(&st.db)
    .await?;

    let out: Vec<Value> = rows
        .into_iter()
        .map(|(id, code, name, dept, _work_end, days, alerts, longest, avg_longest)| {
            let overtime_min: i64 = overtime.iter().filter(|o| o.0 == id).map(|o| o.1).sum();
            let level = if longest > 120 * 60 || overtime_min > 20 * 60 {
                "attention"
            } else if longest > 90 * 60 || alerts > days * 2 {
                "watch"
            } else {
                "good"
            };
            json!({ "employee_id": id, "employee_code": code, "name": name, "department": dept, "days": days,
                    "wellness_alerts": alerts, "longest_sitting_sec": longest, "avg_longest_sitting_sec": avg_longest,
                    "overtime_min": overtime_min, "level": level })
        })
        .collect();
    Ok(Json(json!({ "from": from, "to": to, "rows": out })))
}
