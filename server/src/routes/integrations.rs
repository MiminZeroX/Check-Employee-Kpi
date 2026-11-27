//! Integrations: outgoing webhooks (with test delivery) for HR / ERP / CRM / chat systems.

use axum::{
    extract::{Path, State},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::{audit, AuthUser},
    error::{bad, AppResult},
    hooks::{self, EVENTS},
    state::AppState,
    util::random_token,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/webhooks", get(list).post(create))
        .route("/webhooks/events", get(events))
        .route("/webhooks/{id}", put(update).delete(remove))
        .route("/webhooks/{id}/test", post(test))
        .route("/webhooks/{id}/rotate-secret", post(rotate))
}

#[derive(Serialize, sqlx::FromRow)]
struct HookRow {
    id: i64,
    name: String,
    url: String,
    events: String,
    is_active: bool,
    last_status: Option<i32>,
    last_error: Option<String>,
    last_sent_at: Option<DateTime<Utc>>,
}

async fn list(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Vec<HookRow>>> {
    u.need("webhooks.manage")?;
    Ok(Json(
        sqlx::query_as("SELECT id, name, url, events, is_active, last_status, last_error, last_sent_at FROM webhooks ORDER BY id")
            .fetch_all(&st.db)
            .await?,
    ))
}

async fn events(u: AuthUser) -> AppResult<Json<Value>> {
    u.need("webhooks.manage")?;
    Ok(Json(json!(EVENTS)))
}

#[derive(Deserialize)]
struct HookReq {
    name: String,
    url: String,
    events: String,
    is_active: Option<bool>,
}

impl HookReq {
    fn validate(&self) -> AppResult<String> {
        if self.name.trim().is_empty() {
            return Err(bad("กรุณาระบุชื่อ"));
        }
        let url = self.url.trim();
        if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 500 {
            return Err(bad("URL ต้องขึ้นต้นด้วย http:// หรือ https://"));
        }
        let ev: Vec<&str> = self.events.split(',').map(str::trim).filter(|e| !e.is_empty()).collect();
        if ev.is_empty() || ev.iter().any(|e| *e != "*" && !EVENTS.contains(e)) {
            return Err(bad(format!("เหตุการณ์ไม่ถูกต้อง ใช้ * หรือ {}", EVENTS.join(", "))));
        }
        Ok(ev.join(","))
    }
}

async fn create(State(st): State<AppState>, u: AuthUser, Json(r): Json<HookReq>) -> AppResult<Json<Value>> {
    u.need("webhooks.manage")?;
    let events = r.validate()?;
    let secret = random_token(24);
    let id = sqlx::query("INSERT INTO webhooks (name, url, events, secret, is_active) VALUES (?,?,?,?,?)")
        .bind(r.name.trim())
        .bind(r.url.trim())
        .bind(events)
        .bind(st.crypto.encrypt(&secret))
        .bind(r.is_active.unwrap_or(true))
        .execute(&st.db)
        .await?
        .last_insert_id();
    audit(&st, Some(&u), "create", "webhook", id, json!({ "url": r.url })).await;
    Ok(Json(json!({ "id": id, "secret": secret })))
}

async fn update(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<HookReq>) -> AppResult<Json<Value>> {
    u.need("webhooks.manage")?;
    let events = r.validate()?;
    sqlx::query("UPDATE webhooks SET name=?, url=?, events=?, is_active=? WHERE id=?")
        .bind(r.name.trim())
        .bind(r.url.trim())
        .bind(events)
        .bind(r.is_active.unwrap_or(true))
        .bind(id)
        .execute(&st.db)
        .await?;
    audit(&st, Some(&u), "update", "webhook", id, json!({ "url": r.url })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn remove(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("webhooks.manage")?;
    sqlx::query("DELETE FROM webhooks WHERE id = ?").bind(id).execute(&st.db).await?;
    audit(&st, Some(&u), "delete", "webhook", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

async fn rotate(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("webhooks.manage")?;
    let secret = random_token(24);
    sqlx::query("UPDATE webhooks SET secret = ? WHERE id = ?").bind(st.crypto.encrypt(&secret)).bind(id).execute(&st.db).await?;
    audit(&st, Some(&u), "rotate_secret", "webhook", id, json!({})).await;
    Ok(Json(json!({ "secret": secret })))
}

/// Sends a "ping" event right away and returns the receiver's HTTP status.
async fn test(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("webhooks.manage")?;
    let (url, secret): (String, String) = sqlx::query_as("SELECT url, secret FROM webhooks WHERE id = ?").bind(id).fetch_one(&st.db).await?;
    let secret = st.crypto.decrypt(&secret).unwrap_or_default();
    let body = json!({ "event": "ping", "sent_at": Utc::now(), "data": { "message": "ทดสอบการเชื่อมต่อจาก Smart Office" } }).to_string();
    let status = hooks::deliver(&st, id, &url, "ping", &secret, body).await;
    Ok(Json(json!({ "status": status })))
}
