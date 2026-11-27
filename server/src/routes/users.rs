use std::{net::SocketAddr, sync::OnceLock, time::{Duration, Instant}};

use axum::{
    extract::{ConnectInfo, Path, State},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::{
        audit, hash_password, issue_token, new_claims, validate_password, verify_password, AuthUser, ROLES,
    },
    error::{bad, AppError, AppResult},
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/login", post(login))
        .route("/auth/me", get(me))
        .route("/auth/password", post(change_password))
        .route("/users", get(list).post(create))
        .route("/users/{id}", put(update).delete(remove))
}

#[derive(Deserialize)]
struct LoginReq {
    username: String,
    password: String,
}

const LOCK_WINDOW: Duration = Duration::from_secs(15 * 60);
const MAX_FAILS_PER_USER: u32 = 5;
const MAX_FAILS_PER_IP: u32 = 30;

/// Hash used when the user does not exist, so both cases take the same time
/// (prevents finding valid usernames by timing).
fn dummy_hash() -> &'static str {
    static H: OnceLock<String> = OnceLock::new();
    H.get_or_init(|| hash_password("not-a-real-password").unwrap_or_default())
}

async fn login(
    State(st): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<LoginReq>,
) -> AppResult<Json<Value>> {
    let uname = req.username.trim().to_lowercase();
    let keys = [(format!("u:{uname}"), MAX_FAILS_PER_USER), (format!("ip:{}", addr.ip()), MAX_FAILS_PER_IP)];
    {
        let mut f = st.login_failures.lock().await;
        f.retain(|_, (_, first)| first.elapsed() < LOCK_WINDOW);
        if keys.iter().any(|(k, max)| f.get(k).is_some_and(|(n, _)| n >= max)) {
            return Err(AppError::TooMany("ใส่รหัสผ่านผิดหลายครั้ง กรุณารอ 15 นาทีแล้วลองใหม่".into()));
        }
    }
    let row: Option<(i64, String, String, String, Option<i64>, bool)> = sqlx::query_as(
        "SELECT id, password_hash, role, display_name, employee_id, is_active FROM users WHERE username = ?",
    )
    .bind(req.username.trim())
    .fetch_optional(&st.db)
    .await?;

    let hash = row.as_ref().map(|r| r.1.as_str()).unwrap_or_else(|| dummy_hash());
    let pw_ok = verify_password(&req.password, hash);
    let ok = pw_ok && matches!(&row, Some((_, _, _, _, _, true)));
    if !ok {
        {
            let mut f = st.login_failures.lock().await;
            for (k, _) in &keys {
                let e = f.entry(k.clone()).or_insert((0, Instant::now()));
                e.0 += 1;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        audit(&st, None, "login_failed", "user", req.username.trim(), json!({ "ip": addr.ip().to_string() })).await;
        return Err(AppError::BadRequest("ชื่อผู้ใช้หรือรหัสผ่านไม่ถูกต้อง".into()));
    }
    st.login_failures.lock().await.remove(&keys[0].0);
    let (id, _, role, display_name, emp, _) = row.unwrap();
    let name = if display_name.is_empty() { req.username.clone() } else { display_name };
    let claims = new_claims(&st, id, name.clone(), role.clone(), emp);
    let token = issue_token(&st, claims.clone())?;
    sqlx::query("UPDATE users SET last_login_at = UTC_TIMESTAMP() WHERE id = ?")
        .bind(id)
        .execute(&st.db)
        .await?;
    audit(&st, Some(&AuthUser(claims)), "login", "user", id, json!({ "ip": addr.ip().to_string() })).await;
    Ok(Json(json!({ "token": token, "user": { "id": id, "name": name, "role": role, "employee_id": emp } })))
}

async fn me(user: AuthUser) -> Json<Value> {
    Json(json!({ "id": user.0.sub, "name": user.0.name, "role": user.0.role, "employee_id": user.0.emp }))
}

#[derive(Deserialize)]
struct PwReq {
    current: String,
    new: String,
}

async fn change_password(
    State(st): State<AppState>,
    user: AuthUser,
    Json(req): Json<PwReq>,
) -> AppResult<Json<Value>> {
    validate_password(&req.new)?;
    let (hash,): (String,) = sqlx::query_as("SELECT password_hash FROM users WHERE id = ?")
        .bind(user.id())
        .fetch_one(&st.db)
        .await?;
    if !verify_password(&req.current, &hash) {
        return Err(bad("รหัสผ่านปัจจุบันไม่ถูกต้อง"));
    }
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(hash_password(&req.new)?)
        .bind(user.id())
        .execute(&st.db)
        .await?;
    audit(&st, Some(&user), "change_password", "user", user.id(), json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Serialize, sqlx::FromRow)]
struct UserRow {
    id: i64,
    username: String,
    display_name: String,
    role: String,
    employee_id: Option<i64>,
    is_active: bool,
    last_login_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

async fn list(State(st): State<AppState>, user: AuthUser) -> AppResult<Json<Vec<UserRow>>> {
    user.require(&[])?;
    let rows = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, display_name, role, employee_id, is_active, last_login_at, created_at FROM users ORDER BY id",
    )
    .fetch_all(&st.db)
    .await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
struct UserReq {
    username: Option<String>,
    display_name: Option<String>,
    password: Option<String>,
    role: String,
    employee_id: Option<i64>,
    is_active: Option<bool>,
}

fn check_role(r: &str) -> AppResult<()> {
    if ROLES.contains(&r) {
        Ok(())
    } else {
        Err(bad("บทบาทไม่ถูกต้อง"))
    }
}

async fn create(State(st): State<AppState>, user: AuthUser, Json(req): Json<UserReq>) -> AppResult<Json<Value>> {
    user.require(&[])?;
    check_role(&req.role)?;
    let username = req.username.as_deref().unwrap_or("").trim().to_string();
    if username.is_empty() {
        return Err(bad("กรุณาระบุชื่อผู้ใช้"));
    }
    let pw = req.password.clone().unwrap_or_default();
    validate_password(&pw)?;
    if req.role == "employee" && req.employee_id.is_none() {
        return Err(bad("บัญชีพนักงานต้องผูกกับข้อมูลพนักงาน"));
    }
    let r = sqlx::query(
        "INSERT INTO users (username, display_name, password_hash, role, employee_id, is_active) VALUES (?,?,?,?,?,?)",
    )
    .bind(&username)
    .bind(req.display_name.clone().unwrap_or_default())
    .bind(hash_password(&pw)?)
    .bind(&req.role)
    .bind(req.employee_id)
    .bind(req.is_active.unwrap_or(true))
    .execute(&st.db)
    .await?;
    let id = r.last_insert_id();
    audit(&st, Some(&user), "create", "user", id, json!({ "username": username, "role": req.role })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update(
    State(st): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(req): Json<UserReq>,
) -> AppResult<Json<Value>> {
    user.require(&[])?;
    check_role(&req.role)?;
    if id == user.id() && (req.role != "admin" || req.is_active == Some(false)) {
        return Err(bad("ไม่สามารถลดสิทธิ์หรือปิดบัญชีของตนเองได้"));
    }
    let r = sqlx::query("UPDATE users SET display_name = ?, role = ?, employee_id = ?, is_active = ? WHERE id = ?")
        .bind(req.display_name.clone().unwrap_or_default())
        .bind(&req.role)
        .bind(req.employee_id)
        .bind(req.is_active.unwrap_or(true))
        .bind(id)
        .execute(&st.db)
        .await?;
    if r.rows_affected() == 0 {
        // MySQL reports 0 when nothing changed, so confirm the row exists
        let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(&st.db)
            .await?;
        exists.ok_or(AppError::NotFound)?;
    }
    if let Some(pw) = req.password.as_deref().filter(|p| !p.is_empty()) {
        validate_password(pw)?;
        sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
            .bind(hash_password(pw)?)
            .bind(id)
            .execute(&st.db)
            .await?;
    }
    audit(&st, Some(&user), "update", "user", id, json!({ "role": req.role, "is_active": req.is_active })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn remove(State(st): State<AppState>, user: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    user.require(&[])?;
    if id == user.id() {
        return Err(bad("ไม่สามารถลบบัญชีของตนเองได้"));
    }
    sqlx::query("DELETE FROM users WHERE id = ?").bind(id).execute(&st.db).await?;
    audit(&st, Some(&user), "delete", "user", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}
