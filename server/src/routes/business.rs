//! Projects, customer contacts, the sales pipeline (deals) and service tickets.

use axum::{
    extract::{Path, Query, State},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::{audit, AuthUser},
    error::{bad, AppError, AppResult},
    hooks,
    state::AppState,
    util::setting,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/projects", get(list_projects).post(create_project))
        .route("/projects/{id}", put(update_project).delete(delete_project))
        .route("/contacts", get(list_contacts).post(create_contact))
        .route("/contacts/{id}", put(update_contact).delete(delete_contact))
        .route("/deals", get(list_deals).post(create_deal))
        .route("/deals/summary", get(deal_summary))
        .route("/deals/{id}", put(update_deal).delete(delete_deal))
        .route("/deals/{id}/stage", post(move_deal))
        .route("/tickets", get(list_tickets).post(create_ticket))
        .route("/tickets/{id}", put(update_ticket).delete(delete_ticket))
        .route("/tickets/{id}/status", post(ticket_status))
}

fn blank(s: &Option<String>) -> Option<String> {
    s.as_ref().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

// ---------------------------------------------------------------- projects

#[derive(Serialize, sqlx::FromRow)]
struct ProjectRow {
    id: i64,
    code: Option<String>,
    name: String,
    customer_id: Option<i64>,
    customer_name: Option<String>,
    owner_employee_id: Option<i64>,
    owner_name: Option<String>,
    status: String,
    start_date: Option<NaiveDate>,
    due_date: Option<NaiveDate>,
    budget: Option<f64>,
    description: Option<String>,
    tasks_total: i64,
    tasks_done: i64,
    tasks_overdue: i64,
    tasks_on_time: i64,
}

const PROJECT_STATUSES: [&str; 5] = ["planning", "active", "on_hold", "done", "cancelled"];

async fn list_projects(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Vec<ProjectRow>>> {
    u.need_any(&["projects.view", "tasks.view"])?;
    Ok(Json(
        sqlx::query_as(
            "SELECT p.id, p.code, p.name, p.customer_id, c.name AS customer_name, p.owner_employee_id,
               CONCAT(e.first_name, ' ', e.last_name) AS owner_name, p.status, p.start_date, p.due_date, p.budget, p.description,
               CAST((SELECT COUNT(*) FROM tasks t WHERE t.project_id = p.id AND t.status <> 'cancelled') AS SIGNED) AS tasks_total,
               CAST((SELECT COUNT(*) FROM tasks t WHERE t.project_id = p.id AND t.status = 'done') AS SIGNED) AS tasks_done,
               CAST((SELECT COUNT(*) FROM tasks t WHERE t.project_id = p.id AND t.status IN ('todo','in_progress')
                       AND t.due_date < CURDATE()) AS SIGNED) AS tasks_overdue,
               CAST((SELECT COUNT(*) FROM tasks t WHERE t.project_id = p.id AND t.status = 'done' AND t.due_date IS NOT NULL
                       AND DATE(t.completed_at) <= t.due_date) AS SIGNED) AS tasks_on_time
             FROM projects p LEFT JOIN customers c ON c.id = p.customer_id LEFT JOIN employees e ON e.id = p.owner_employee_id
             ORDER BY FIELD(p.status, 'active', 'planning', 'on_hold', 'done', 'cancelled'), p.due_date IS NULL, p.due_date",
        )
        .fetch_all(&st.db)
        .await?,
    ))
}

#[derive(Deserialize)]
struct ProjectReq {
    code: Option<String>,
    name: String,
    customer_id: Option<i64>,
    owner_employee_id: Option<i64>,
    status: Option<String>,
    start_date: Option<NaiveDate>,
    due_date: Option<NaiveDate>,
    budget: Option<f64>,
    description: Option<String>,
}

impl ProjectReq {
    fn validate(&self) -> AppResult<String> {
        if self.name.trim().is_empty() {
            return Err(bad("กรุณาระบุชื่อโครงการ"));
        }
        let s = self.status.clone().unwrap_or_else(|| "active".into());
        if !PROJECT_STATUSES.contains(&s.as_str()) {
            return Err(bad("สถานะโครงการไม่ถูกต้อง"));
        }
        if let (Some(a), Some(b)) = (self.start_date, self.due_date) {
            if b < a {
                return Err(bad("วันสิ้นสุดต้องไม่ก่อนวันเริ่ม"));
            }
        }
        Ok(s)
    }
}

async fn create_project(State(st): State<AppState>, u: AuthUser, Json(r): Json<ProjectReq>) -> AppResult<Json<Value>> {
    u.need("projects.manage")?;
    let status = r.validate()?;
    let id = sqlx::query(
        "INSERT INTO projects (code, name, customer_id, owner_employee_id, status, start_date, due_date, budget, description) VALUES (?,?,?,?,?,?,?,?,?)",
    )
    .bind(blank(&r.code))
    .bind(r.name.trim())
    .bind(r.customer_id)
    .bind(r.owner_employee_id)
    .bind(status)
    .bind(r.start_date)
    .bind(r.due_date)
    .bind(r.budget)
    .bind(&r.description)
    .execute(&st.db)
    .await?
    .last_insert_id();
    audit(&st, Some(&u), "create", "project", id, json!({ "name": r.name })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_project(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<ProjectReq>) -> AppResult<Json<Value>> {
    u.need("projects.manage")?;
    let status = r.validate()?;
    sqlx::query(
        "UPDATE projects SET code=?, name=?, customer_id=?, owner_employee_id=?, status=?, start_date=?, due_date=?, budget=?, description=? WHERE id=?",
    )
    .bind(blank(&r.code))
    .bind(r.name.trim())
    .bind(r.customer_id)
    .bind(r.owner_employee_id)
    .bind(status)
    .bind(r.start_date)
    .bind(r.due_date)
    .bind(r.budget)
    .bind(&r.description)
    .bind(id)
    .execute(&st.db)
    .await?;
    audit(&st, Some(&u), "update", "project", id, json!({ "name": r.name })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_project(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("projects.manage")?;
    sqlx::query("DELETE FROM projects WHERE id = ?").bind(id).execute(&st.db).await?;
    audit(&st, Some(&u), "delete", "project", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- contacts

#[derive(Serialize, sqlx::FromRow)]
struct ContactRow {
    id: i64,
    customer_id: i64,
    customer_name: Option<String>,
    name: String,
    position: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    is_primary: bool,
    notes: Option<String>,
}

#[derive(Deserialize)]
struct CustomerQ {
    customer_id: Option<i64>,
}

async fn list_contacts(State(st): State<AppState>, u: AuthUser, Query(q): Query<CustomerQ>) -> AppResult<Json<Vec<ContactRow>>> {
    u.need("crm.view")?;
    let sql = "SELECT k.id, k.customer_id, c.name AS customer_name, k.name, k.position, k.email, k.phone, k.is_primary, k.notes
               FROM contacts k JOIN customers c ON c.id = k.customer_id";
    let rows = match q.customer_id {
        Some(c) => sqlx::query_as(&format!("{sql} WHERE k.customer_id = ? ORDER BY k.is_primary DESC, k.name")).bind(c).fetch_all(&st.db).await?,
        None => sqlx::query_as(&format!("{sql} ORDER BY c.name, k.is_primary DESC, k.name")).fetch_all(&st.db).await?,
    };
    Ok(Json(rows))
}

#[derive(Deserialize)]
struct ContactReq {
    customer_id: i64,
    name: String,
    position: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    is_primary: Option<bool>,
    notes: Option<String>,
}

async fn save_contact(st: &AppState, id: Option<i64>, r: &ContactReq) -> AppResult<i64> {
    if r.name.trim().is_empty() {
        return Err(bad("กรุณาระบุชื่อผู้ติดต่อ"));
    }
    if r.is_primary == Some(true) {
        sqlx::query("UPDATE contacts SET is_primary = 0 WHERE customer_id = ?").bind(r.customer_id).execute(&st.db).await?;
    }
    let q = match id {
        Some(_) => "UPDATE contacts SET customer_id=?, name=?, position=?, email=?, phone=?, is_primary=?, notes=? WHERE id=?",
        None => "INSERT INTO contacts (customer_id, name, position, email, phone, is_primary, notes) VALUES (?,?,?,?,?,?,?)",
    };
    let mut q = sqlx::query(q)
        .bind(r.customer_id)
        .bind(r.name.trim())
        .bind(blank(&r.position))
        .bind(blank(&r.email))
        .bind(blank(&r.phone))
        .bind(r.is_primary.unwrap_or(false))
        .bind(&r.notes);
    if let Some(i) = id {
        q = q.bind(i);
    }
    let res = q.execute(&st.db).await?;
    Ok(id.unwrap_or(res.last_insert_id() as i64))
}

async fn create_contact(State(st): State<AppState>, u: AuthUser, Json(r): Json<ContactReq>) -> AppResult<Json<Value>> {
    u.need("crm.manage")?;
    let id = save_contact(&st, None, &r).await?;
    audit(&st, Some(&u), "create", "contact", id, json!({ "customer_id": r.customer_id })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_contact(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<ContactReq>) -> AppResult<Json<Value>> {
    u.need("crm.manage")?;
    save_contact(&st, Some(id), &r).await?;
    audit(&st, Some(&u), "update", "contact", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_contact(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("crm.manage")?;
    sqlx::query("DELETE FROM contacts WHERE id = ?").bind(id).execute(&st.db).await?;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- deals (sales pipeline)

pub const STAGES: [(&str, i32); 6] = [("lead", 10), ("qualified", 25), ("proposal", 50), ("negotiation", 75), ("won", 100), ("lost", 0)];

#[derive(Serialize, sqlx::FromRow)]
struct DealRow {
    id: i64,
    title: String,
    customer_id: Option<i64>,
    customer_name: Option<String>,
    owner_employee_id: Option<i64>,
    owner_name: Option<String>,
    stage: String,
    value: f64,
    probability: i32,
    expected_close: Option<NaiveDate>,
    closed_at: Option<DateTime<Utc>>,
    notes: Option<String>,
    created_at: DateTime<Utc>,
}

const DEAL_SELECT: &str = "SELECT d.id, d.title, d.customer_id, c.name AS customer_name, d.owner_employee_id,
    CONCAT(e.first_name, ' ', e.last_name) AS owner_name, d.stage, d.value, d.probability, d.expected_close, d.closed_at,
    d.notes, d.created_at FROM deals d LEFT JOIN customers c ON c.id = d.customer_id LEFT JOIN employees e ON e.id = d.owner_employee_id";

async fn list_deals(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Vec<DealRow>>> {
    u.need("crm.view")?;
    Ok(Json(sqlx::query_as(&format!("{DEAL_SELECT} ORDER BY d.expected_close IS NULL, d.expected_close, d.id DESC")).fetch_all(&st.db).await?))
}

async fn deal_summary(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    u.need("crm.view")?;
    let rows: Vec<(String, i64, f64, f64)> = sqlx::query_as(
        "SELECT stage, CAST(COUNT(*) AS SIGNED), CAST(COALESCE(SUM(value), 0) AS DOUBLE), CAST(COALESCE(SUM(value * probability / 100), 0) AS DOUBLE)
         FROM deals GROUP BY stage",
    )
    .fetch_all(&st.db)
    .await?;
    let stages: Vec<Value> = STAGES
        .iter()
        .map(|(s, _)| {
            let r = rows.iter().find(|r| r.0 == *s);
            json!({ "stage": s, "count": r.map_or(0, |r| r.1), "value": r.map_or(0.0, |r| r.2), "weighted": r.map_or(0.0, |r| r.3) })
        })
        .collect();
    let open: f64 = rows.iter().filter(|r| r.0 != "won" && r.0 != "lost").map(|r| r.3).sum();
    let won = rows.iter().find(|r| r.0 == "won").map_or(0, |r| r.1);
    let lost = rows.iter().find(|r| r.0 == "lost").map_or(0, |r| r.1);
    let win_rate = if won + lost > 0 { Some(won as f64 * 100.0 / (won + lost) as f64) } else { None };
    Ok(Json(json!({ "stages": stages, "weighted_pipeline": open, "win_rate": win_rate })))
}

#[derive(Deserialize)]
struct DealReq {
    title: String,
    customer_id: Option<i64>,
    owner_employee_id: Option<i64>,
    stage: Option<String>,
    value: Option<f64>,
    probability: Option<i32>,
    expected_close: Option<NaiveDate>,
    notes: Option<String>,
}

fn stage_ok(s: &str) -> AppResult<i32> {
    STAGES.iter().find(|(k, _)| *k == s).map(|(_, p)| *p).ok_or_else(|| bad("ขั้นตอนการขายไม่ถูกต้อง"))
}

async fn create_deal(State(st): State<AppState>, u: AuthUser, Json(r): Json<DealReq>) -> AppResult<Json<Value>> {
    u.need("crm.manage")?;
    if r.title.trim().is_empty() {
        return Err(bad("กรุณาระบุชื่อดีล"));
    }
    let stage = r.stage.clone().unwrap_or_else(|| "lead".into());
    let default_p = stage_ok(&stage)?;
    let id = sqlx::query(
        "INSERT INTO deals (title, customer_id, owner_employee_id, stage, value, probability, expected_close, notes, closed_at)
         VALUES (?,?,?,?,?,?,?,?, IF(? IN ('won','lost'), UTC_TIMESTAMP(), NULL))",
    )
    .bind(r.title.trim())
    .bind(r.customer_id)
    .bind(r.owner_employee_id)
    .bind(&stage)
    .bind(r.value.unwrap_or(0.0).max(0.0))
    .bind(r.probability.unwrap_or(default_p).clamp(0, 100))
    .bind(r.expected_close)
    .bind(&r.notes)
    .bind(&stage)
    .execute(&st.db)
    .await?
    .last_insert_id();
    audit(&st, Some(&u), "create", "deal", id, json!({ "title": r.title, "stage": stage })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_deal(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<DealReq>) -> AppResult<Json<Value>> {
    u.need("crm.manage")?;
    let stage = r.stage.clone().unwrap_or_else(|| "lead".into());
    let default_p = stage_ok(&stage)?;
    sqlx::query(
        "UPDATE deals SET title=?, customer_id=?, owner_employee_id=?, stage=?, value=?, probability=?, expected_close=?, notes=?,
           closed_at = IF(? IN ('won','lost'), COALESCE(closed_at, UTC_TIMESTAMP()), NULL) WHERE id=?",
    )
    .bind(r.title.trim())
    .bind(r.customer_id)
    .bind(r.owner_employee_id)
    .bind(&stage)
    .bind(r.value.unwrap_or(0.0).max(0.0))
    .bind(r.probability.unwrap_or(default_p).clamp(0, 100))
    .bind(r.expected_close)
    .bind(&r.notes)
    .bind(&stage)
    .bind(id)
    .execute(&st.db)
    .await?;
    audit(&st, Some(&u), "update", "deal", id, json!({ "stage": stage })).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct StageReq {
    stage: String,
}

/// Moves a deal on the pipeline board; probability follows the stage default.
async fn move_deal(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<StageReq>) -> AppResult<Json<Value>> {
    u.need("crm.manage")?;
    let p = stage_ok(&r.stage)?;
    sqlx::query(
        "UPDATE deals SET stage = ?, probability = ?, closed_at = IF(? IN ('won','lost'), COALESCE(closed_at, UTC_TIMESTAMP()), NULL) WHERE id = ?",
    )
    .bind(&r.stage)
    .bind(p)
    .bind(&r.stage)
    .bind(id)
    .execute(&st.db)
    .await?;
    audit(&st, Some(&u), "move_stage", "deal", id, json!({ "stage": r.stage })).await;
    hooks::emit(&st, "deal.stage_changed", json!({ "id": id, "stage": r.stage }));
    Ok(Json(json!({ "ok": true, "probability": p })))
}

async fn delete_deal(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("crm.manage")?;
    sqlx::query("DELETE FROM deals WHERE id = ?").bind(id).execute(&st.db).await?;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- tickets

#[derive(Serialize, sqlx::FromRow)]
struct TicketRow {
    id: i64,
    subject: String,
    description: Option<String>,
    customer_id: Option<i64>,
    customer_name: Option<String>,
    contact_id: Option<i64>,
    contact_name: Option<String>,
    assignee_employee_id: Option<i64>,
    assignee_name: Option<String>,
    priority: String,
    status: String,
    due_at: Option<DateTime<Utc>>,
    resolved_at: Option<DateTime<Utc>>,
    satisfaction: Option<i8>,
    created_at: DateTime<Utc>,
}

const TICKET_SELECT: &str = "SELECT t.id, t.subject, t.description, t.customer_id, c.name AS customer_name, t.contact_id,
    k.name AS contact_name, t.assignee_employee_id, CONCAT(e.first_name, ' ', e.last_name) AS assignee_name, t.priority,
    t.status, t.due_at, t.resolved_at, t.satisfaction, t.created_at FROM tickets t LEFT JOIN customers c ON c.id = t.customer_id
    LEFT JOIN contacts k ON k.id = t.contact_id LEFT JOIN employees e ON e.id = t.assignee_employee_id";

const TICKET_STATUSES: [&str; 5] = ["open", "in_progress", "waiting", "resolved", "closed"];

#[derive(Deserialize)]
struct TicketFilter {
    status: Option<String>,
    mine: Option<bool>,
}

async fn list_tickets(State(st): State<AppState>, u: AuthUser, Query(f): Query<TicketFilter>) -> AppResult<Json<Vec<TicketRow>>> {
    let mine = f.mine == Some(true) || !(u.can("crm.view") || u.can("tickets.manage"));
    let mut sql = format!("{TICKET_SELECT} WHERE 1=1");
    if mine {
        sql.push_str(" AND t.assignee_employee_id = ?");
    }
    if f.status.is_some() {
        sql.push_str(" AND t.status = ?");
    }
    sql.push_str(" ORDER BY FIELD(t.status, 'open', 'in_progress', 'waiting', 'resolved', 'closed'), t.due_at IS NULL, t.due_at LIMIT 2000");
    let mut q = sqlx::query_as::<_, TicketRow>(&sql);
    if mine {
        q = q.bind(u.0.emp.ok_or(AppError::Forbidden)?);
    }
    if let Some(s) = &f.status {
        q = q.bind(s);
    }
    Ok(Json(q.fetch_all(&st.db).await?))
}

#[derive(Deserialize)]
struct TicketReq {
    subject: String,
    description: Option<String>,
    customer_id: Option<i64>,
    contact_id: Option<i64>,
    assignee_employee_id: Option<i64>,
    priority: Option<String>,
    status: Option<String>,
    satisfaction: Option<i8>,
}

/// Response deadline from the SLA hours per priority (System Settings).
async fn sla_due(st: &AppState, priority: &str) -> DateTime<Utc> {
    let hours = setting(&st.db, "ticket_sla_hours")
        .await
        .and_then(|v| v[priority].as_f64())
        .unwrap_or(match priority {
            "urgent" => 4.0,
            "high" => 24.0,
            "low" => 72.0,
            _ => 48.0,
        });
    Utc::now() + Duration::minutes((hours * 60.0) as i64)
}

impl TicketReq {
    fn validate(&self) -> AppResult<(String, String)> {
        if self.subject.trim().is_empty() {
            return Err(bad("กรุณาระบุหัวข้อ"));
        }
        let p = self.priority.clone().unwrap_or_else(|| "normal".into());
        if !["low", "normal", "high", "urgent"].contains(&p.as_str()) {
            return Err(bad("ความเร่งด่วนไม่ถูกต้อง"));
        }
        let s = self.status.clone().unwrap_or_else(|| "open".into());
        if !TICKET_STATUSES.contains(&s.as_str()) {
            return Err(bad("สถานะไม่ถูกต้อง"));
        }
        if let Some(x) = self.satisfaction {
            if !(1..=5).contains(&x) {
                return Err(bad("คะแนนความพึงพอใจต้องอยู่ระหว่าง 1-5"));
            }
        }
        Ok((p, s))
    }
}

async fn create_ticket(State(st): State<AppState>, u: AuthUser, Json(r): Json<TicketReq>) -> AppResult<Json<Value>> {
    u.need_any(&["tickets.manage", "crm.manage"])?;
    let (priority, status) = r.validate()?;
    let due = sla_due(&st, &priority).await;
    let id = sqlx::query(
        "INSERT INTO tickets (subject, description, customer_id, contact_id, assignee_employee_id, priority, status, due_at, satisfaction,
           resolved_at) VALUES (?,?,?,?,?,?,?,?,?, IF(? IN ('resolved','closed'), UTC_TIMESTAMP(), NULL))",
    )
    .bind(r.subject.trim())
    .bind(&r.description)
    .bind(r.customer_id)
    .bind(r.contact_id)
    .bind(r.assignee_employee_id)
    .bind(&priority)
    .bind(&status)
    .bind(due)
    .bind(r.satisfaction)
    .bind(&status)
    .execute(&st.db)
    .await?
    .last_insert_id();
    if let Some(e) = r.assignee_employee_id {
        sqlx::query("INSERT INTO notifications (employee_id, kind, title, body) VALUES (?, 'ticket', ?, ?)")
            .bind(e)
            .bind(format!("ได้รับมอบหมาย Ticket #{id}"))
            .bind(r.subject.trim())
            .execute(&st.db)
            .await?;
    }
    audit(&st, Some(&u), "create", "ticket", id, json!({ "subject": r.subject })).await;
    hooks::emit(&st, "ticket.created", json!({ "id": id, "subject": r.subject, "priority": priority, "due_at": due }));
    Ok(Json(json!({ "id": id, "due_at": due })))
}

async fn update_ticket(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<TicketReq>) -> AppResult<Json<Value>> {
    u.need_any(&["tickets.manage", "crm.manage"])?;
    let (priority, status) = r.validate()?;
    sqlx::query(
        "UPDATE tickets SET subject=?, description=?, customer_id=?, contact_id=?, assignee_employee_id=?, priority=?, satisfaction=?,
           resolved_at = CASE WHEN ? IN ('resolved','closed') THEN COALESCE(resolved_at, UTC_TIMESTAMP()) ELSE NULL END,
           status=? WHERE id=?",
    )
    .bind(r.subject.trim())
    .bind(&r.description)
    .bind(r.customer_id)
    .bind(r.contact_id)
    .bind(r.assignee_employee_id)
    .bind(&priority)
    .bind(r.satisfaction)
    .bind(&status)
    .bind(&status)
    .bind(id)
    .execute(&st.db)
    .await?;
    audit(&st, Some(&u), "update", "ticket", id, json!({ "status": status })).await;
    hooks::emit(&st, "ticket.updated", json!({ "id": id, "status": status }));
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct TicketStatusReq {
    status: String,
}

/// The assignee may move their own ticket along; others need tickets.manage.
async fn ticket_status(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<TicketStatusReq>) -> AppResult<Json<Value>> {
    if !TICKET_STATUSES.contains(&r.status.as_str()) {
        return Err(bad("สถานะไม่ถูกต้อง"));
    }
    let (assignee,): (Option<i64>,) = sqlx::query_as("SELECT assignee_employee_id FROM tickets WHERE id = ?").bind(id).fetch_one(&st.db).await?;
    if assignee.is_none() || assignee != u.0.emp {
        u.need("tickets.manage")?;
    }
    sqlx::query(
        "UPDATE tickets SET resolved_at = CASE WHEN ? IN ('resolved','closed') THEN COALESCE(resolved_at, UTC_TIMESTAMP()) ELSE NULL END,
           status = ? WHERE id = ?",
    )
    .bind(&r.status)
    .bind(&r.status)
    .bind(id)
    .execute(&st.db)
    .await?;
    audit(&st, Some(&u), "ticket_status", "ticket", id, json!({ "status": r.status })).await;
    hooks::emit(&st, "ticket.updated", json!({ "id": id, "status": r.status }));
    Ok(Json(json!({ "ok": true })))
}

async fn delete_ticket(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("tickets.manage")?;
    sqlx::query("DELETE FROM tickets WHERE id = ?").bind(id).execute(&st.db).await?;
    Ok(Json(json!({ "ok": true })))
}
