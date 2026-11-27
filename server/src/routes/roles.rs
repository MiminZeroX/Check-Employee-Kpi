//! Roles and their permissions (editable RBAC).

use axum::{
    extract::{Path, State},
    routing::{get, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::{audit, is_permission, load_permissions, AuthUser, PERMISSIONS},
    error::{bad, AppResult},
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/roles", get(list).post(create))
        .route("/roles/{name}", put(update).delete(remove))
}

async fn list(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    u.need("users.manage")?;
    let roles: Vec<(String, String, bool, i64)> = sqlx::query_as(
        "SELECT r.name, r.label, r.is_system, (SELECT COUNT(*) FROM users us WHERE us.role = r.name) FROM roles r ORDER BY r.is_system DESC, r.name",
    )
    .fetch_all(&st.db)
    .await?;
    let perms = st.role_perms.read().await;
    let rows: Vec<Value> = roles
        .into_iter()
        .map(|(name, label, system, users)| {
            let p: Vec<&str> = if name == "admin" {
                PERMISSIONS.iter().map(|(k, _)| *k).collect()
            } else {
                let mut v: Vec<&str> = perms.get(&name).map(|s| s.iter().map(|x| x.as_str()).collect()).unwrap_or_default();
                v.sort();
                v
            };
            json!({ "name": name, "label": label, "is_system": system, "users": users, "permissions": p, "editable": name != "admin" })
        })
        .collect();
    let catalog: Vec<Value> = PERMISSIONS.iter().map(|(k, l)| json!({ "key": k, "label": l })).collect();
    Ok(Json(json!({ "roles": rows, "permissions": catalog })))
}

#[derive(Deserialize)]
struct RoleReq {
    name: Option<String>,
    label: String,
    permissions: Vec<String>,
}

fn validate(r: &RoleReq) -> AppResult<()> {
    if r.label.trim().is_empty() {
        return Err(bad("กรุณาระบุชื่อบทบาท"));
    }
    if let Some(p) = r.permissions.iter().find(|p| !is_permission(p)) {
        return Err(bad(format!("ไม่รู้จักสิทธิ์: {p}")));
    }
    Ok(())
}

async fn save_permissions(st: &AppState, role: &str, perms: &[String]) -> AppResult<()> {
    let mut tx = st.db.begin().await?;
    sqlx::query("DELETE FROM role_permissions WHERE role = ?").bind(role).execute(&mut *tx).await?;
    for p in perms {
        sqlx::query("INSERT IGNORE INTO role_permissions (role, permission) VALUES (?, ?)")
            .bind(role)
            .bind(p)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    load_permissions(st).await?;
    Ok(())
}

async fn create(State(st): State<AppState>, u: AuthUser, Json(r): Json<RoleReq>) -> AppResult<Json<Value>> {
    u.need("users.manage")?;
    validate(&r)?;
    let name = r.name.clone().unwrap_or_default().trim().to_lowercase();
    if name.is_empty() || name.len() > 40 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(bad("รหัสบทบาทใช้ได้เฉพาะ a-z 0-9 _ - (ไม่เกิน 40 ตัว)"));
    }
    // only an admin may hand out user-management rights
    if !u.is_admin() && r.permissions.iter().any(|p| p == "users.manage") {
        return Err(crate::error::AppError::Forbidden);
    }
    sqlx::query("INSERT INTO roles (name, label) VALUES (?, ?)").bind(&name).bind(r.label.trim()).execute(&st.db).await?;
    save_permissions(&st, &name, &r.permissions).await?;
    audit(&st, Some(&u), "create", "role", &name, json!({ "permissions": r.permissions })).await;
    Ok(Json(json!({ "name": name })))
}

async fn update(State(st): State<AppState>, u: AuthUser, Path(name): Path<String>, Json(r): Json<RoleReq>) -> AppResult<Json<Value>> {
    u.need("users.manage")?;
    validate(&r)?;
    if name == "admin" {
        return Err(bad("บทบาทผู้ดูแลระบบมีทุกสิทธิ์เสมอ แก้ไขไม่ได้"));
    }
    if !u.is_admin() && r.permissions.iter().any(|p| p == "users.manage") {
        return Err(crate::error::AppError::Forbidden);
    }
    sqlx::query("UPDATE roles SET label = ? WHERE name = ?").bind(r.label.trim()).bind(&name).execute(&st.db).await?;
    save_permissions(&st, &name, &r.permissions).await?;
    audit(&st, Some(&u), "update", "role", &name, json!({ "permissions": r.permissions })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn remove(State(st): State<AppState>, u: AuthUser, Path(name): Path<String>) -> AppResult<Json<Value>> {
    u.need("users.manage")?;
    let row: Option<(bool, i64)> = sqlx::query_as(
        "SELECT r.is_system, (SELECT COUNT(*) FROM users us WHERE us.role = r.name) FROM roles r WHERE r.name = ?",
    )
    .bind(&name)
    .fetch_optional(&st.db)
    .await?;
    match row {
        None => return Err(crate::error::AppError::NotFound),
        Some((true, _)) => return Err(bad("ลบบทบาทพื้นฐานของระบบไม่ได้")),
        Some((_, n)) if n > 0 => return Err(bad(format!("ยังมีผู้ใช้ {n} คนในบทบาทนี้"))),
        _ => {}
    }
    sqlx::query("DELETE FROM roles WHERE name = ?").bind(&name).execute(&st.db).await?;
    load_permissions(&st).await?;
    audit(&st, Some(&u), "delete", "role", &name, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}
