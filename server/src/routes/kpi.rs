//! KPI metrics and scoring.
//!
//! Scores are built from work results (tasks, quality, customer satisfaction,
//! punctuality, supervisor reviews). Camera activity is never part of the score;
//! it is shown in reports as context only, as required by the project spec.

use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    routing::{get, put},
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
    util::{date_range, day_range, to_local},
};

pub const SOURCES: [&str; 6] = [
    "manual",
    "task_completion",
    "task_on_time",
    "task_quality",
    "attendance_punctuality",
    "customer_satisfaction",
];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/kpi/metrics", get(list_metrics).post(create_metric))
        .route("/kpi/metrics/{id}", put(update_metric).delete(delete_metric))
        .route("/kpi/scores", get(scores))
        .route("/kpi/records", get(list_records).post(upsert_record))
}

#[derive(Serialize, sqlx::FromRow, Clone)]
pub struct Metric {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub source: String,
    pub weight: f64,
    pub position: Option<String>,
    pub is_active: bool,
}

async fn list_metrics(State(st): State<AppState>, _u: AuthUser) -> AppResult<Json<Vec<Metric>>> {
    Ok(Json(
        sqlx::query_as("SELECT id, code, name, description, source, weight, position, is_active FROM kpi_metrics ORDER BY id")
            .fetch_all(&st.db)
            .await?,
    ))
}

#[derive(Deserialize)]
struct MetricReq {
    code: String,
    name: String,
    description: Option<String>,
    source: String,
    weight: f64,
    position: Option<String>,
    is_active: Option<bool>,
}

impl MetricReq {
    fn validate(&self) -> AppResult<()> {
        if self.code.trim().is_empty() || self.name.trim().is_empty() {
            return Err(bad("กรุณาระบุรหัสและชื่อตัวชี้วัด"));
        }
        if !SOURCES.contains(&self.source.as_str()) {
            return Err(bad("แหล่งข้อมูลของตัวชี้วัดไม่ถูกต้อง (ไม่อนุญาตให้ใช้ข้อมูลจากกล้องเป็นคะแนน)"));
        }
        if !(0.0..=100.0).contains(&self.weight) {
            return Err(bad("น้ำหนักต้องอยู่ระหว่าง 0-100"));
        }
        Ok(())
    }
    fn position(&self) -> Option<String> {
        self.position.as_ref().map(|p| p.trim().to_string()).filter(|p| !p.is_empty())
    }
}

async fn create_metric(State(st): State<AppState>, u: AuthUser, Json(r): Json<MetricReq>) -> AppResult<Json<Value>> {
    u.require(&["hr"])?;
    r.validate()?;
    let id = sqlx::query(
        "INSERT INTO kpi_metrics (code, name, description, source, weight, position, is_active) VALUES (?,?,?,?,?,?,?)",
    )
    .bind(r.code.trim())
    .bind(r.name.trim())
    .bind(&r.description)
    .bind(&r.source)
    .bind(r.weight)
    .bind(r.position())
    .bind(r.is_active.unwrap_or(true))
    .execute(&st.db)
    .await?
    .last_insert_id();
    audit(&st, Some(&u), "create", "kpi_metric", id, json!({ "code": r.code, "weight": r.weight })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_metric(
    State(st): State<AppState>,
    u: AuthUser,
    Path(id): Path<i64>,
    Json(r): Json<MetricReq>,
) -> AppResult<Json<Value>> {
    u.require(&["hr"])?;
    r.validate()?;
    sqlx::query("UPDATE kpi_metrics SET code=?, name=?, description=?, source=?, weight=?, position=?, is_active=? WHERE id=?")
        .bind(r.code.trim())
        .bind(r.name.trim())
        .bind(&r.description)
        .bind(&r.source)
        .bind(r.weight)
        .bind(r.position())
        .bind(r.is_active.unwrap_or(true))
        .bind(id)
        .execute(&st.db)
        .await?;
    audit(&st, Some(&u), "update", "kpi_metric", id, json!({ "code": r.code, "weight": r.weight })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_metric(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.require(&["hr"])?;
    sqlx::query("DELETE FROM kpi_metrics WHERE id = ?").bind(id).execute(&st.db).await?;
    audit(&st, Some(&u), "delete", "kpi_metric", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- scoring

#[derive(Serialize, Clone)]
pub struct KpiItem {
    pub metric_id: i64,
    pub code: String,
    pub name: String,
    pub source: String,
    pub weight: f64,
    pub score: Option<f64>,
    pub detail: String,
}

#[derive(Serialize, Clone)]
pub struct KpiResult {
    pub employee_id: i64,
    pub employee_code: String,
    pub name: String,
    pub position: Option<String>,
    pub department: Option<String>,
    /// Weighted score 0-100 over the metrics that have data; None when nothing to score.
    pub total: Option<f64>,
    pub grade: Option<String>,
    pub items: Vec<KpiItem>,
}

pub fn grade(score: f64) -> &'static str {
    match score {
        s if s >= 90.0 => "A (ดีเยี่ยม)",
        s if s >= 80.0 => "B (ดี)",
        s if s >= 70.0 => "C (ตามเป้าหมาย)",
        s if s >= 60.0 => "D (ควรพัฒนา)",
        _ => "E (ต้องปรับปรุง)",
    }
}

struct TaskAgg {
    total: i64,
    done: i64,
    done_with_due: i64,
    on_time: i64,
    quality_x1000: Option<i64>,
}

pub async fn compute(
    st: &AppState,
    from: NaiveDate,
    to: NaiveDate,
    department_id: Option<i64>,
    employee_id: Option<i64>,
) -> AppResult<Vec<KpiResult>> {
    let off = st.cfg.tz_offset_min;
    let (start, _) = day_range(from, off);
    let (_, end) = day_range(to, off);

    let mut esql = String::from(
        "SELECT e.id, e.employee_code, CONCAT(e.first_name, ' ', e.last_name), e.position, d.name, e.work_start
         FROM employees e LEFT JOIN departments d ON d.id = e.department_id WHERE e.status = 'active'",
    );
    if department_id.is_some() {
        esql.push_str(" AND e.department_id = ?");
    }
    if employee_id.is_some() {
        esql.push_str(" AND e.id = ?");
    }
    esql.push_str(" ORDER BY e.employee_code");
    let mut q = sqlx::query_as::<_, (i64, String, String, Option<String>, Option<String>, NaiveTime)>(&esql);
    if let Some(d) = department_id {
        q = q.bind(d);
    }
    if let Some(e) = employee_id {
        q = q.bind(e);
    }
    let emps = q.fetch_all(&st.db).await?;

    let metrics: Vec<Metric> = sqlx::query_as(
        "SELECT id, code, name, description, source, weight, position, is_active FROM kpi_metrics WHERE is_active = 1 ORDER BY id",
    )
    .fetch_all(&st.db)
    .await?;

    let tasks: HashMap<i64, TaskAgg> = sqlx::query_as::<_, (i64, i64, Option<i64>, Option<i64>, Option<i64>, Option<i64>)>(
        "SELECT assignee_employee_id,
           CAST(COUNT(*) AS SIGNED),
           CAST(SUM(status = 'done') AS SIGNED),
           CAST(SUM(status = 'done' AND due_date IS NOT NULL) AS SIGNED),
           CAST(SUM(status = 'done' AND due_date IS NOT NULL
                    AND DATE(DATE_ADD(completed_at, INTERVAL ? MINUTE)) <= due_date) AS SIGNED),
           CAST(AVG(CASE WHEN status = 'done' THEN quality_score END) * 1000 AS SIGNED)
         FROM tasks
         WHERE assignee_employee_id IS NOT NULL AND status <> 'cancelled'
           AND ((due_date BETWEEN ? AND ?) OR (completed_at >= ? AND completed_at < ?))
         GROUP BY assignee_employee_id",
    )
    .bind(off)
    .bind(from)
    .bind(to)
    .bind(start)
    .bind(end)
    .fetch_all(&st.db)
    .await?
    .into_iter()
    .map(|(id, total, done, dwd, ot, q)| {
        (
            id,
            TaskAgg {
                total,
                done: done.unwrap_or(0),
                done_with_due: dwd.unwrap_or(0),
                on_time: ot.unwrap_or(0),
                quality_x1000: q,
            },
        )
    })
    .collect();

    let first_scans: Vec<(i64, DateTime<Utc>)> = sqlx::query_as(
        "SELECT employee_id, first_scan_at FROM daily_summaries
         WHERE work_date BETWEEN ? AND ? AND first_scan_at IS NOT NULL",
    )
    .bind(from)
    .bind(to)
    .fetch_all(&st.db)
    .await?;

    let satisfaction: HashMap<i64, (i64, i64)> = sqlx::query_as::<_, (i64, Option<i64>, i64)>(
        "SELECT owner_employee_id, CAST(AVG(satisfaction) * 1000 AS SIGNED), COUNT(*) FROM customers
         WHERE owner_employee_id IS NOT NULL AND satisfaction IS NOT NULL GROUP BY owner_employee_id",
    )
    .fetch_all(&st.db)
    .await?
    .into_iter()
    .filter_map(|(id, avg, n)| avg.map(|a| (id, (a, n))))
    .collect();

    let manual: HashMap<(i64, i64), (f64, i64)> = sqlx::query_as::<_, (i64, i64, f64, i64)>(
        "SELECT employee_id, metric_id, AVG(score), COUNT(*) FROM kpi_records
         WHERE period_start <= ? AND period_end >= ? GROUP BY employee_id, metric_id",
    )
    .bind(to)
    .bind(from)
    .fetch_all(&st.db)
    .await?
    .into_iter()
    .map(|(e, m, s, n)| ((e, m), (s, n)))
    .collect();

    let mut out = Vec::with_capacity(emps.len());
    for (id, code, name, position, dept, work_start) in emps {
        let t = tasks.get(&id);
        let mut items = Vec::new();
        for m in metrics.iter().filter(|m| m.position.is_none() || m.position == position) {
            let (score, detail): (Option<f64>, String) = match m.source.as_str() {
                "task_completion" => match t {
                    Some(t) if t.total > 0 => (
                        Some(t.done as f64 * 100.0 / t.total as f64),
                        format!("เสร็จ {} จาก {} งาน", t.done, t.total),
                    ),
                    _ => (None, "ไม่มีงานในช่วงเวลานี้".into()),
                },
                "task_on_time" => match t {
                    Some(t) if t.done_with_due > 0 => (
                        Some(t.on_time as f64 * 100.0 / t.done_with_due as f64),
                        format!("ตรงเวลา {} จาก {} งาน", t.on_time, t.done_with_due),
                    ),
                    _ => (None, "ไม่มีงานที่มีกำหนดส่ง".into()),
                },
                "task_quality" => match t.and_then(|t| t.quality_x1000) {
                    Some(q) => {
                        let avg = q as f64 / 1000.0;
                        (Some(avg * 20.0), format!("คะแนนเฉลี่ย {avg:.2} / 5"))
                    }
                    None => (None, "ยังไม่มีการให้คะแนนคุณภาพ".into()),
                },
                "attendance_punctuality" => {
                    let days: Vec<_> = first_scans.iter().filter(|s| s.0 == id).collect();
                    if days.is_empty() {
                        (None, "ไม่มีข้อมูลการสแกนเข้างาน".into())
                    } else {
                        let ok = days.iter().filter(|s| to_local(s.1, off).time() <= work_start).count();
                        (
                            Some(ok as f64 * 100.0 / days.len() as f64),
                            format!("ตรงเวลา {ok} จาก {} วัน", days.len()),
                        )
                    }
                }
                "customer_satisfaction" => match satisfaction.get(&id) {
                    Some((a, n)) => {
                        let avg = *a as f64 / 1000.0;
                        (Some(avg * 20.0), format!("ความพึงพอใจเฉลี่ย {avg:.2} / 5 จากลูกค้า {n} ราย"))
                    }
                    None => (None, "ยังไม่มีคะแนนความพึงพอใจ".into()),
                },
                _ => match manual.get(&(id, m.id)) {
                    Some((s, n)) => (Some(*s), format!("จากการประเมิน {n} ครั้ง")),
                    None => (None, "ยังไม่ได้ประเมิน".into()),
                },
            };
            items.push(KpiItem {
                metric_id: m.id,
                code: m.code.clone(),
                name: m.name.clone(),
                source: m.source.clone(),
                weight: m.weight,
                score: score.map(|s| (s.clamp(0.0, 100.0) * 10.0).round() / 10.0),
                detail,
            });
        }
        let (sum, w) = items
            .iter()
            .filter_map(|i| i.score.map(|s| (s * i.weight, i.weight)))
            .fold((0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
        let total = (w > 0.0).then(|| ((sum / w) * 10.0).round() / 10.0);
        out.push(KpiResult {
            employee_id: id,
            employee_code: code,
            name,
            position,
            department: dept,
            total,
            grade: total.map(|t| grade(t).to_string()),
            items,
        });
    }
    Ok(out)
}

async fn scores(State(st): State<AppState>, u: AuthUser, Query(r): Query<Range>) -> AppResult<Json<Value>> {
    let emp = if u.role() == "employee" {
        Some(u.0.emp.ok_or(AppError::Forbidden)?)
    } else {
        u.require(&["hr", "manager"])?;
        r.employee_id
    };
    let (from, to) = date_range(r.from, r.to, st.cfg.tz_offset_min)?;
    let rows = compute(&st, from, to, r.department_id, emp).await?;
    Ok(Json(json!({ "from": from, "to": to, "rows": rows })))
}

#[derive(Serialize, sqlx::FromRow)]
struct RecordRow {
    id: i64,
    employee_id: i64,
    employee_name: Option<String>,
    metric_id: i64,
    metric_name: Option<String>,
    period_start: NaiveDate,
    period_end: NaiveDate,
    score: f64,
    note: Option<String>,
    created_at: DateTime<Utc>,
}

async fn list_records(State(st): State<AppState>, u: AuthUser, Query(r): Query<Range>) -> AppResult<Json<Vec<RecordRow>>> {
    u.require(&["hr", "manager"])?;
    let (from, to) = date_range(r.from, r.to, st.cfg.tz_offset_min)?;
    let mut sql = String::from(
        "SELECT r.id, r.employee_id, CONCAT(e.first_name, ' ', e.last_name) AS employee_name, r.metric_id,
           m.name AS metric_name, r.period_start, r.period_end, r.score, r.note, r.created_at
         FROM kpi_records r LEFT JOIN employees e ON e.id = r.employee_id LEFT JOIN kpi_metrics m ON m.id = r.metric_id
         WHERE r.period_start <= ? AND r.period_end >= ?",
    );
    if r.employee_id.is_some() {
        sql.push_str(" AND r.employee_id = ?");
    }
    sql.push_str(" ORDER BY r.id DESC LIMIT 1000");
    let mut q = sqlx::query_as::<_, RecordRow>(&sql).bind(to).bind(from);
    if let Some(e) = r.employee_id {
        q = q.bind(e);
    }
    Ok(Json(q.fetch_all(&st.db).await?))
}

#[derive(Deserialize)]
struct RecordReq {
    employee_id: i64,
    metric_id: i64,
    period_start: NaiveDate,
    period_end: NaiveDate,
    score: f64,
    note: Option<String>,
}

async fn upsert_record(State(st): State<AppState>, u: AuthUser, Json(r): Json<RecordReq>) -> AppResult<Json<Value>> {
    u.require(&["hr", "manager"])?;
    if !(0.0..=100.0).contains(&r.score) {
        return Err(bad("คะแนนต้องอยู่ระหว่าง 0-100"));
    }
    if r.period_end < r.period_start {
        return Err(bad("ช่วงเวลาไม่ถูกต้อง"));
    }
    if Some(r.employee_id) == u.0.emp {
        return Err(bad("ไม่สามารถประเมินตนเองได้"));
    }
    sqlx::query(
        "INSERT INTO kpi_records (employee_id, metric_id, period_start, period_end, score, note, reviewer_user_id)
         VALUES (?,?,?,?,?,?,?)
         ON DUPLICATE KEY UPDATE score = VALUES(score), note = VALUES(note), reviewer_user_id = VALUES(reviewer_user_id)",
    )
    .bind(r.employee_id)
    .bind(r.metric_id)
    .bind(r.period_start)
    .bind(r.period_end)
    .bind(r.score)
    .bind(&r.note)
    .bind(u.id())
    .execute(&st.db)
    .await?;
    audit(&st, Some(&u), "kpi_review", "employee", r.employee_id, json!({ "metric_id": r.metric_id, "score": r.score })).await;
    Ok(Json(json!({ "ok": true })))
}
