//! Attendance logs from fingerprint / face terminals and the scan-to-desk view.

use axum::{
    extract::{Query, State},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::{audit, AuthUser},
    error::{bad, AppResult},
    routes::{
        hr::sessions,
        ingest::{store_scans, Scan},
        Range,
    },
    state::AppState,
    util::{date_range, day_range, local_today, parse_scan_time},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/attendance/logs", get(logs))
        .route("/attendance/daily", get(daily))
        .route("/attendance/unmatched", get(unmatched))
        .route("/attendance/manual", post(manual))
}

#[derive(Serialize, sqlx::FromRow)]
struct LogRow {
    id: i64,
    employee_id: Option<i64>,
    employee_name: Option<String>,
    attendance_ref: String,
    device_name: Option<String>,
    scan_time: DateTime<Utc>,
    direction: String,
    method: String,
}

async fn logs(State(st): State<AppState>, u: AuthUser, Query(r): Query<Range>) -> AppResult<Json<Vec<LogRow>>> {
    u.need("attendance.view")?;
    let off = st.cfg.tz_offset_min;
    let (from, to) = date_range(r.from, r.to, off)?;
    let (start, _) = day_range(from, off);
    let (_, end) = day_range(to, off);
    let mut sql = String::from(
        "SELECT a.id, a.employee_id, CONCAT(e.first_name, ' ', e.last_name) AS employee_name, a.attendance_ref,
           d.name AS device_name, a.scan_time, a.direction, a.method
         FROM attendance_logs a LEFT JOIN employees e ON e.id = a.employee_id LEFT JOIN devices d ON d.id = a.device_id
         WHERE a.scan_time >= ? AND a.scan_time < ?",
    );
    if r.employee_id.is_some() {
        sql.push_str(" AND a.employee_id = ?");
    }
    sql.push_str(" ORDER BY a.scan_time DESC LIMIT 2000");
    let mut q = sqlx::query_as::<_, LogRow>(&sql).bind(start).bind(end);
    if let Some(e) = r.employee_id {
        q = q.bind(e);
    }
    Ok(Json(q.fetch_all(&st.db).await?))
}

#[derive(Deserialize)]
struct DailyQ {
    date: Option<NaiveDate>,
    department_id: Option<i64>,
    branch_id: Option<i64>,
}

/// One row per active employee for the day: schedule, office entry, check-in, desk arrival,
/// lateness, overtime and status (present / leave / offsite / holiday / absent ...).
async fn daily(State(st): State<AppState>, u: AuthUser, Query(q): Query<DailyQ>) -> AppResult<Json<Value>> {
    u.need("attendance.view")?;
    let off = st.cfg.tz_offset_min;
    let date = q.date.unwrap_or_else(|| local_today(off));
    if date <= local_today(off) {
        // make sure today's row reflects the latest scans
        if date == local_today(off) {
            crate::summary::recompute_day(&st, date).await?;
        }
    }
    let rows = sessions(&st, date, date, None, q.department_id, q.branch_id).await?;
    let count = |s: &str| rows.iter().filter(|r| r.status == s).count();
    let late = rows.iter().filter(|r| r.late_min > 0).count();
    Ok(Json(json!({
        "date": date,
        "present": count("present"), "late": late, "leave": count("leave"), "offsite": count("offsite"),
        "absent": count("absent"), "not_yet": count("not_yet"), "rows": rows,
    })))
}

async fn unmatched(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    u.need("attendance.manage")?;
    let rows: Vec<(String, i64, DateTime<Utc>)> = sqlx::query_as(
        "SELECT attendance_ref, COUNT(*), MAX(scan_time) FROM attendance_logs WHERE employee_id IS NULL
         GROUP BY attendance_ref ORDER BY MAX(scan_time) DESC LIMIT 200",
    )
    .fetch_all(&st.db)
    .await?;
    Ok(Json(json!(rows
        .into_iter()
        .map(|(r, n, t)| json!({ "attendance_ref": r, "count": n, "last_scan_at": t }))
        .collect::<Vec<_>>())))
}

#[derive(Deserialize)]
struct ManualReq {
    employee_id: i64,
    scan_time: String,
    direction: Option<String>,
    note: Option<String>,
}

/// HR can add a scan by hand (e.g. the terminal was offline). Audited.
async fn manual(State(st): State<AppState>, u: AuthUser, Json(r): Json<ManualReq>) -> AppResult<Json<Value>> {
    u.need("attendance.manage")?;
    let t = parse_scan_time(&r.scan_time, st.cfg.tz_offset_min).ok_or_else(|| bad("รูปแบบเวลาไม่ถูกต้อง"))?;
    let (aref,): (Option<String>,) = sqlx::query_as("SELECT attendance_ref FROM employees WHERE id = ?")
        .bind(r.employee_id)
        .fetch_one(&st.db)
        .await?;
    let aref = aref.ok_or_else(|| bad("พนักงานยังไม่มีรหัสเครื่องสแกน (attendance_ref)"))?;
    let direction = match r.direction.as_deref() {
        Some("in") => "in",
        Some("out") => "out",
        _ => "unknown",
    };
    let n = store_scans(
        &st,
        None,
        vec![Scan {
            attendance_ref: aref,
            time: t,
            direction: direction.into(),
            method: "manual".into(),
            raw: r.note.clone().map(|n| format!("manual by {}: {n}", u.name())),
        }],
    )
    .await?;
    audit(&st, Some(&u), "manual_scan", "employee", r.employee_id, json!({ "scan_time": t, "note": r.note })).await;
    Ok(Json(json!({ "inserted": n })))
}
