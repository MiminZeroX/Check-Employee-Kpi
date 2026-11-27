//! Branches, departments and employees.

use axum::{
    extract::{Path, Query, State},
    routing::{get, put},
    Json, Router,
};
use chrono::{DateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::{audit, AuthUser},
    error::{bad, AppResult},
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/branches", get(list_branches).post(create_branch))
        .route("/branches/{id}", put(update_branch).delete(delete_branch))
        .route("/departments", get(list_departments).post(create_department))
        .route("/departments/{id}", put(update_department).delete(delete_department))
        .route("/employees", get(list_employees).post(create_employee))
        .route("/employees/{id}", get(get_employee).put(update_employee).delete(delete_employee))
}

// ---------- branches ----------

#[derive(Serialize, sqlx::FromRow)]
struct Branch {
    id: i64,
    name: String,
    address: Option<String>,
}

#[derive(Deserialize)]
struct BranchReq {
    name: String,
    address: Option<String>,
}

async fn list_branches(State(st): State<AppState>, _u: AuthUser) -> AppResult<Json<Vec<Branch>>> {
    Ok(Json(
        sqlx::query_as("SELECT id, name, address FROM branches ORDER BY name").fetch_all(&st.db).await?,
    ))
}

async fn create_branch(State(st): State<AppState>, u: AuthUser, Json(r): Json<BranchReq>) -> AppResult<Json<Value>> {
    u.need("org.manage")?;
    if r.name.trim().is_empty() {
        return Err(bad("กรุณาระบุชื่อสาขา"));
    }
    let id = sqlx::query("INSERT INTO branches (organization_id, name, address) VALUES (1, ?, ?)")
        .bind(r.name.trim())
        .bind(&r.address)
        .execute(&st.db)
        .await?
        .last_insert_id();
    audit(&st, Some(&u), "create", "branch", id, json!({ "name": r.name })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_branch(
    State(st): State<AppState>,
    u: AuthUser,
    Path(id): Path<i64>,
    Json(r): Json<BranchReq>,
) -> AppResult<Json<Value>> {
    u.need("org.manage")?;
    sqlx::query("UPDATE branches SET name = ?, address = ? WHERE id = ?")
        .bind(r.name.trim())
        .bind(&r.address)
        .bind(id)
        .execute(&st.db)
        .await?;
    audit(&st, Some(&u), "update", "branch", id, json!({ "name": r.name })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_branch(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("org.manage")?;
    sqlx::query("DELETE FROM branches WHERE id = ?").bind(id).execute(&st.db).await?;
    audit(&st, Some(&u), "delete", "branch", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------- departments ----------

#[derive(Serialize, sqlx::FromRow)]
struct Department {
    id: i64,
    name: String,
    branch_id: Option<i64>,
    branch_name: Option<String>,
    employee_count: i64,
}

#[derive(Deserialize)]
struct DepartmentReq {
    name: String,
    branch_id: Option<i64>,
}

async fn list_departments(State(st): State<AppState>, _u: AuthUser) -> AppResult<Json<Vec<Department>>> {
    Ok(Json(
        sqlx::query_as(
            "SELECT d.id, d.name, d.branch_id, b.name AS branch_name,
                    (SELECT COUNT(*) FROM employees e WHERE e.department_id = d.id AND e.status = 'active') AS employee_count
             FROM departments d LEFT JOIN branches b ON b.id = d.branch_id ORDER BY d.name",
        )
        .fetch_all(&st.db)
        .await?,
    ))
}

async fn create_department(
    State(st): State<AppState>,
    u: AuthUser,
    Json(r): Json<DepartmentReq>,
) -> AppResult<Json<Value>> {
    u.need("org.manage")?;
    if r.name.trim().is_empty() {
        return Err(bad("กรุณาระบุชื่อแผนก"));
    }
    let id = sqlx::query("INSERT INTO departments (name, branch_id) VALUES (?, ?)")
        .bind(r.name.trim())
        .bind(r.branch_id)
        .execute(&st.db)
        .await?
        .last_insert_id();
    audit(&st, Some(&u), "create", "department", id, json!({ "name": r.name })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_department(
    State(st): State<AppState>,
    u: AuthUser,
    Path(id): Path<i64>,
    Json(r): Json<DepartmentReq>,
) -> AppResult<Json<Value>> {
    u.need("org.manage")?;
    sqlx::query("UPDATE departments SET name = ?, branch_id = ? WHERE id = ?")
        .bind(r.name.trim())
        .bind(r.branch_id)
        .bind(id)
        .execute(&st.db)
        .await?;
    audit(&st, Some(&u), "update", "department", id, json!({ "name": r.name })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_department(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("org.manage")?;
    sqlx::query("DELETE FROM departments WHERE id = ?").bind(id).execute(&st.db).await?;
    audit(&st, Some(&u), "delete", "department", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------- employees ----------

#[derive(Serialize, sqlx::FromRow)]
pub struct Employee {
    pub id: i64,
    pub employee_code: String,
    pub first_name: String,
    pub last_name: String,
    pub nickname: Option<String>,
    pub position: Option<String>,
    pub department_id: Option<i64>,
    pub department_name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub attendance_ref: Option<String>,
    pub work_start: NaiveTime,
    pub work_end: NaiveTime,
    pub monitoring_consent: bool,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

pub const EMPLOYEE_SELECT: &str = "SELECT e.id, e.employee_code, e.first_name, e.last_name, e.nickname, e.position,
    e.department_id, d.name AS department_name, e.email, e.phone, e.attendance_ref, e.work_start, e.work_end,
    e.monitoring_consent, e.status, e.created_at
    FROM employees e LEFT JOIN departments d ON d.id = e.department_id";

#[derive(Deserialize)]
struct EmpFilter {
    q: Option<String>,
    department_id: Option<i64>,
    status: Option<String>,
}

async fn list_employees(
    State(st): State<AppState>,
    u: AuthUser,
    Query(f): Query<EmpFilter>,
) -> AppResult<Json<Vec<Employee>>> {
    u.need("employees.view")?;
    let mut sql = format!("{EMPLOYEE_SELECT} WHERE 1=1");
    if f.department_id.is_some() {
        sql.push_str(" AND e.department_id = ?");
    }
    if f.status.is_some() {
        sql.push_str(" AND e.status = ?");
    }
    if f.q.is_some() {
        sql.push_str(" AND (e.first_name LIKE ? OR e.last_name LIKE ? OR e.employee_code LIKE ? OR e.nickname LIKE ?)");
    }
    sql.push_str(" ORDER BY e.employee_code");
    let mut q = sqlx::query_as::<_, Employee>(&sql);
    if let Some(d) = f.department_id {
        q = q.bind(d);
    }
    if let Some(s) = &f.status {
        q = q.bind(s);
    }
    if let Some(term) = &f.q {
        let like = format!("%{}%", term.trim());
        q = q.bind(like.clone()).bind(like.clone()).bind(like.clone()).bind(like);
    }
    Ok(Json(q.fetch_all(&st.db).await?))
}

async fn get_employee(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Employee>> {
    if !u.can("employees.view") && u.0.emp != Some(id) {
        return Err(crate::error::AppError::Forbidden);
    }
    let e = sqlx::query_as::<_, Employee>(&format!("{EMPLOYEE_SELECT} WHERE e.id = ?"))
        .bind(id)
        .fetch_one(&st.db)
        .await?;
    Ok(Json(e))
}

#[derive(Deserialize)]
struct EmployeeReq {
    employee_code: String,
    first_name: String,
    last_name: Option<String>,
    nickname: Option<String>,
    position: Option<String>,
    department_id: Option<i64>,
    email: Option<String>,
    phone: Option<String>,
    attendance_ref: Option<String>,
    work_start: Option<String>,
    work_end: Option<String>,
    monitoring_consent: Option<bool>,
    status: Option<String>,
}

fn parse_time(s: Option<&str>, default: &str) -> AppResult<NaiveTime> {
    let s = s.filter(|v| !v.trim().is_empty()).unwrap_or(default).trim();
    NaiveTime::parse_from_str(s, "%H:%M:%S")
        .or_else(|_| NaiveTime::parse_from_str(s, "%H:%M"))
        .map_err(|_| bad(format!("รูปแบบเวลาไม่ถูกต้อง: {s}")))
}

fn blank_to_none(s: &Option<String>) -> Option<String> {
    s.as_ref().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

impl EmployeeReq {
    fn validate(&self) -> AppResult<(NaiveTime, NaiveTime, String)> {
        if self.employee_code.trim().is_empty() || self.first_name.trim().is_empty() {
            return Err(bad("กรุณาระบุรหัสพนักงานและชื่อ"));
        }
        let status = self.status.clone().unwrap_or_else(|| "active".into());
        if !["active", "inactive"].contains(&status.as_str()) {
            return Err(bad("สถานะไม่ถูกต้อง"));
        }
        Ok((
            parse_time(self.work_start.as_deref(), "08:30")?,
            parse_time(self.work_end.as_deref(), "17:30")?,
            status,
        ))
    }
}

async fn create_employee(
    State(st): State<AppState>,
    u: AuthUser,
    Json(r): Json<EmployeeReq>,
) -> AppResult<Json<Value>> {
    u.need("employees.manage")?;
    let (ws, we, status) = r.validate()?;
    let id = sqlx::query(
        "INSERT INTO employees (employee_code, first_name, last_name, nickname, position, department_id, email, phone,
           attendance_ref, work_start, work_end, monitoring_consent, status)
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(r.employee_code.trim())
    .bind(r.first_name.trim())
    .bind(r.last_name.clone().unwrap_or_default().trim())
    .bind(blank_to_none(&r.nickname))
    .bind(blank_to_none(&r.position))
    .bind(r.department_id)
    .bind(blank_to_none(&r.email))
    .bind(blank_to_none(&r.phone))
    .bind(blank_to_none(&r.attendance_ref))
    .bind(ws)
    .bind(we)
    .bind(r.monitoring_consent.unwrap_or(false))
    .bind(status)
    .execute(&st.db)
    .await?
    .last_insert_id();
    // link historical scans that arrived before the employee was registered
    if let Some(aref) = blank_to_none(&r.attendance_ref) {
        sqlx::query("UPDATE attendance_logs SET employee_id = ? WHERE attendance_ref = ? AND employee_id IS NULL")
            .bind(id)
            .bind(aref)
            .execute(&st.db)
            .await?;
    }
    audit(&st, Some(&u), "create", "employee", id, json!({ "code": r.employee_code })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_employee(
    State(st): State<AppState>,
    u: AuthUser,
    Path(id): Path<i64>,
    Json(r): Json<EmployeeReq>,
) -> AppResult<Json<Value>> {
    u.need("employees.manage")?;
    let (ws, we, status) = r.validate()?;
    sqlx::query(
        "UPDATE employees SET employee_code=?, first_name=?, last_name=?, nickname=?, position=?, department_id=?,
           email=?, phone=?, attendance_ref=?, work_start=?, work_end=?, monitoring_consent=?, status=? WHERE id=?",
    )
    .bind(r.employee_code.trim())
    .bind(r.first_name.trim())
    .bind(r.last_name.clone().unwrap_or_default().trim())
    .bind(blank_to_none(&r.nickname))
    .bind(blank_to_none(&r.position))
    .bind(r.department_id)
    .bind(blank_to_none(&r.email))
    .bind(blank_to_none(&r.phone))
    .bind(blank_to_none(&r.attendance_ref))
    .bind(ws)
    .bind(we)
    .bind(r.monitoring_consent.unwrap_or(false))
    .bind(status)
    .bind(id)
    .execute(&st.db)
    .await?;
    if let Some(aref) = blank_to_none(&r.attendance_ref) {
        sqlx::query("UPDATE attendance_logs SET employee_id = ? WHERE attendance_ref = ? AND employee_id IS NULL")
            .bind(id)
            .bind(aref)
            .execute(&st.db)
            .await?;
    }
    audit(&st, Some(&u), "update", "employee", id, json!({ "code": r.employee_code })).await;
    Ok(Json(json!({ "ok": true })))
}

/// Employees are deactivated rather than deleted so that history stays consistent.
async fn delete_employee(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("employees.manage")?;
    sqlx::query("UPDATE employees SET status = 'inactive' WHERE id = ?").bind(id).execute(&st.db).await?;
    sqlx::query("UPDATE workstations SET employee_id = NULL WHERE employee_id = ?").bind(id).execute(&st.db).await?;
    audit(&st, Some(&u), "deactivate", "employee", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}
