//! CRM (customers) and task / project management.

use axum::{
    extract::{Path, Query, State},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::{audit, AuthUser},
    error::{bad, AppError, AppResult},
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/customers", get(list_customers).post(create_customer))
        .route("/customers/{id}", put(update_customer).delete(delete_customer))
        .route("/tasks", get(list_tasks).post(create_task))
        .route("/tasks/{id}", put(update_task).delete(delete_task))
        .route("/tasks/{id}/status", post(set_task_status))
}

// ---------------------------------------------------------------- customers

#[derive(Serialize, sqlx::FromRow)]
struct Customer {
    id: i64,
    name: String,
    company: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    status: String,
    owner_employee_id: Option<i64>,
    owner_name: Option<String>,
    satisfaction: Option<i8>,
    notes: Option<String>,
    open_tasks: i64,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct CustomerFilter {
    q: Option<String>,
    status: Option<String>,
}

async fn list_customers(
    State(st): State<AppState>,
    u: AuthUser,
    Query(f): Query<CustomerFilter>,
) -> AppResult<Json<Vec<Customer>>> {
    u.need("crm.view")?;
    let mut sql = String::from(
        "SELECT c.id, c.name, c.company, c.email, c.phone, c.status, c.owner_employee_id,
           CONCAT(e.first_name, ' ', e.last_name) AS owner_name, c.satisfaction, c.notes,
           (SELECT COUNT(*) FROM tasks t WHERE t.customer_id = c.id AND t.status IN ('todo','in_progress')) AS open_tasks,
           c.created_at
         FROM customers c LEFT JOIN employees e ON e.id = c.owner_employee_id WHERE 1=1",
    );
    if f.status.is_some() {
        sql.push_str(" AND c.status = ?");
    }
    if f.q.is_some() {
        sql.push_str(" AND (c.name LIKE ? OR c.company LIKE ? OR c.email LIKE ?)");
    }
    sql.push_str(" ORDER BY c.id DESC LIMIT 1000");
    let mut q = sqlx::query_as::<_, Customer>(&sql);
    if let Some(s) = &f.status {
        q = q.bind(s);
    }
    if let Some(t) = &f.q {
        let like = format!("%{}%", t.trim());
        q = q.bind(like.clone()).bind(like.clone()).bind(like);
    }
    Ok(Json(q.fetch_all(&st.db).await?))
}

#[derive(Deserialize)]
struct CustomerReq {
    name: String,
    company: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    status: Option<String>,
    owner_employee_id: Option<i64>,
    satisfaction: Option<i8>,
    notes: Option<String>,
}

impl CustomerReq {
    fn validate(&self) -> AppResult<String> {
        if self.name.trim().is_empty() {
            return Err(bad("กรุณาระบุชื่อลูกค้า"));
        }
        if let Some(s) = self.satisfaction {
            if !(1..=5).contains(&s) {
                return Err(bad("คะแนนความพึงพอใจต้องอยู่ระหว่าง 1-5"));
            }
        }
        let status = self.status.clone().unwrap_or_else(|| "lead".into());
        if !["lead", "active", "inactive"].contains(&status.as_str()) {
            return Err(bad("สถานะลูกค้าไม่ถูกต้อง"));
        }
        Ok(status)
    }
}

async fn create_customer(State(st): State<AppState>, u: AuthUser, Json(r): Json<CustomerReq>) -> AppResult<Json<Value>> {
    u.need("crm.manage")?;
    let status = r.validate()?;
    let id = sqlx::query(
        "INSERT INTO customers (name, company, email, phone, status, owner_employee_id, satisfaction, notes) VALUES (?,?,?,?,?,?,?,?)",
    )
    .bind(r.name.trim())
    .bind(&r.company)
    .bind(&r.email)
    .bind(&r.phone)
    .bind(status)
    .bind(r.owner_employee_id)
    .bind(r.satisfaction)
    .bind(&r.notes)
    .execute(&st.db)
    .await?
    .last_insert_id();
    audit(&st, Some(&u), "create", "customer", id, json!({ "name": r.name })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_customer(
    State(st): State<AppState>,
    u: AuthUser,
    Path(id): Path<i64>,
    Json(r): Json<CustomerReq>,
) -> AppResult<Json<Value>> {
    u.need("crm.manage")?;
    let status = r.validate()?;
    sqlx::query(
        "UPDATE customers SET name=?, company=?, email=?, phone=?, status=?, owner_employee_id=?, satisfaction=?, notes=? WHERE id=?",
    )
    .bind(r.name.trim())
    .bind(&r.company)
    .bind(&r.email)
    .bind(&r.phone)
    .bind(status)
    .bind(r.owner_employee_id)
    .bind(r.satisfaction)
    .bind(&r.notes)
    .bind(id)
    .execute(&st.db)
    .await?;
    audit(&st, Some(&u), "update", "customer", id, json!({ "name": r.name })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_customer(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("crm.manage")?;
    sqlx::query("DELETE FROM customers WHERE id = ?").bind(id).execute(&st.db).await?;
    audit(&st, Some(&u), "delete", "customer", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- tasks

#[derive(Serialize, sqlx::FromRow)]
struct Task {
    id: i64,
    title: String,
    description: Option<String>,
    project: Option<String>,
    project_id: Option<i64>,
    assignee_employee_id: Option<i64>,
    assignee_name: Option<String>,
    customer_id: Option<i64>,
    customer_name: Option<String>,
    priority: String,
    status: String,
    due_date: Option<NaiveDate>,
    completed_at: Option<DateTime<Utc>>,
    quality_score: Option<i8>,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct TaskFilter {
    assignee: Option<i64>,
    project_id: Option<i64>,
    status: Option<String>,
    customer_id: Option<i64>,
}

async fn list_tasks(State(st): State<AppState>, u: AuthUser, Query(f): Query<TaskFilter>) -> AppResult<Json<Vec<Task>>> {
    let assignee = if u.can("tasks.view") { f.assignee } else { Some(u.0.emp.ok_or(AppError::Forbidden)?) };
    let mut sql = String::from(
        "SELECT t.id, t.title, t.description, COALESCE(p.name, t.project) AS project, t.project_id, t.assignee_employee_id,
           CONCAT(e.first_name, ' ', e.last_name) AS assignee_name, t.customer_id, c.name AS customer_name,
           t.priority, t.status, t.due_date, t.completed_at, t.quality_score, t.created_at
         FROM tasks t LEFT JOIN employees e ON e.id = t.assignee_employee_id LEFT JOIN customers c ON c.id = t.customer_id
         LEFT JOIN projects p ON p.id = t.project_id
         WHERE 1=1",
    );
    if assignee.is_some() {
        sql.push_str(" AND t.assignee_employee_id = ?");
    }
    if f.status.is_some() {
        sql.push_str(" AND t.status = ?");
    }
    if f.customer_id.is_some() {
        sql.push_str(" AND t.customer_id = ?");
    }
    if f.project_id.is_some() {
        sql.push_str(" AND t.project_id = ?");
    }
    sql.push_str(" ORDER BY FIELD(t.status, 'in_progress', 'todo', 'done', 'cancelled'), t.due_date IS NULL, t.due_date, t.id DESC LIMIT 2000");
    let mut q = sqlx::query_as::<_, Task>(&sql);
    if let Some(a) = assignee {
        q = q.bind(a);
    }
    if let Some(s) = &f.status {
        q = q.bind(s);
    }
    if let Some(c) = f.customer_id {
        q = q.bind(c);
    }
    if let Some(p) = f.project_id {
        q = q.bind(p);
    }
    Ok(Json(q.fetch_all(&st.db).await?))
}

const TASK_STATUSES: [&str; 4] = ["todo", "in_progress", "done", "cancelled"];

#[derive(Deserialize)]
struct TaskReq {
    title: String,
    description: Option<String>,
    project: Option<String>,
    project_id: Option<i64>,
    assignee_employee_id: Option<i64>,
    customer_id: Option<i64>,
    priority: Option<String>,
    status: Option<String>,
    due_date: Option<NaiveDate>,
    quality_score: Option<i8>,
    /// Optional, for importing historical tasks; defaults to "now" when a task becomes done.
    completed_at: Option<DateTime<Utc>>,
}

impl TaskReq {
    fn validate(&self) -> AppResult<(String, String)> {
        if self.title.trim().is_empty() {
            return Err(bad("กรุณาระบุชื่องาน"));
        }
        let status = self.status.clone().unwrap_or_else(|| "todo".into());
        if !TASK_STATUSES.contains(&status.as_str()) {
            return Err(bad("สถานะงานไม่ถูกต้อง"));
        }
        let priority = self.priority.clone().unwrap_or_else(|| "normal".into());
        if !["low", "normal", "high"].contains(&priority.as_str()) {
            return Err(bad("ความสำคัญไม่ถูกต้อง"));
        }
        if let Some(q) = self.quality_score {
            if !(1..=5).contains(&q) {
                return Err(bad("คะแนนคุณภาพต้องอยู่ระหว่าง 1-5"));
            }
        }
        Ok((status, priority))
    }
}

async fn create_task(State(st): State<AppState>, u: AuthUser, Json(r): Json<TaskReq>) -> AppResult<Json<Value>> {
    u.need("tasks.manage")?;
    let (status, priority) = r.validate()?;
    let id = sqlx::query(
        "INSERT INTO tasks (title, description, project, project_id, assignee_employee_id, customer_id, priority, status, due_date,
           quality_score, completed_at) VALUES (?,?,?,?,?,?,?,?,?,?, IF(? = 'done', COALESCE(?, UTC_TIMESTAMP()), NULL))",
    )
    .bind(r.title.trim())
    .bind(&r.description)
    .bind(&r.project)
    .bind(r.project_id)
    .bind(r.assignee_employee_id)
    .bind(r.customer_id)
    .bind(priority)
    .bind(&status)
    .bind(r.due_date)
    .bind(r.quality_score)
    .bind(&status)
    .bind(r.completed_at)
    .execute(&st.db)
    .await?
    .last_insert_id();
    audit(&st, Some(&u), "create", "task", id, json!({ "title": r.title })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_task(
    State(st): State<AppState>,
    u: AuthUser,
    Path(id): Path<i64>,
    Json(r): Json<TaskReq>,
) -> AppResult<Json<Value>> {
    u.need("tasks.manage")?;
    let (status, priority) = r.validate()?;
    sqlx::query(
        "UPDATE tasks SET title=?, description=?, project=?, project_id=?, assignee_employee_id=?, customer_id=?, priority=?,
           completed_at = CASE WHEN ? = 'done' THEN COALESCE(?, completed_at, UTC_TIMESTAMP()) ELSE NULL END,
           status=?, due_date=?, quality_score=? WHERE id=?",
    )
    .bind(r.title.trim())
    .bind(&r.description)
    .bind(&r.project)
    .bind(r.project_id)
    .bind(r.assignee_employee_id)
    .bind(r.customer_id)
    .bind(priority)
    .bind(&status)
    .bind(r.completed_at)
    .bind(&status)
    .bind(r.due_date)
    .bind(r.quality_score)
    .bind(id)
    .execute(&st.db)
    .await?;
    audit(&st, Some(&u), "update", "task", id, json!({ "status": status, "quality": r.quality_score })).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct StatusReq {
    status: String,
}

/// Employees may move their own tasks between todo / in_progress / done.
async fn set_task_status(
    State(st): State<AppState>,
    u: AuthUser,
    Path(id): Path<i64>,
    Json(r): Json<StatusReq>,
) -> AppResult<Json<Value>> {
    if !["todo", "in_progress", "done"].contains(&r.status.as_str()) {
        return Err(bad("สถานะงานไม่ถูกต้อง"));
    }
    let (assignee,): (Option<i64>,) =
        sqlx::query_as("SELECT assignee_employee_id FROM tasks WHERE id = ?").bind(id).fetch_one(&st.db).await?;
    // the assignee may move their own task; otherwise tasks.manage is needed
    if assignee.is_none() || assignee != u.0.emp {
        u.need("tasks.manage")?;
    }
    sqlx::query(
        "UPDATE tasks SET completed_at = CASE WHEN ? = 'done' THEN COALESCE(completed_at, UTC_TIMESTAMP()) ELSE NULL END,
           status = ? WHERE id = ?",
    )
    .bind(&r.status)
    .bind(&r.status)
    .bind(id)
    .execute(&st.db)
    .await?;
    audit(&st, Some(&u), "task_status", "task", id, json!({ "status": r.status })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_task(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("tasks.manage")?;
    sqlx::query("DELETE FROM tasks WHERE id = ?").bind(id).execute(&st.db).await?;
    audit(&st, Some(&u), "delete", "task", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}
