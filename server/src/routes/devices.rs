//! Machine clients: AI workers and attendance terminals (ZKTeco, Hikvision, generic).

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
    state::AppState,
    util::{random_token, sha256_hex},
};

pub const DEVICE_KINDS: [&str; 4] = ["ai_worker", "zkteco", "hikvision", "generic"];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/devices", get(list).post(create))
        .route("/devices/{id}", put(update).delete(remove))
        .route("/devices/{id}/rotate-key", post(rotate))
}

#[derive(Serialize, sqlx::FromRow)]
struct Device {
    id: i64,
    name: String,
    kind: String,
    serial_no: Option<String>,
    has_key: i64,
    branch_id: Option<i64>,
    is_enabled: bool,
    last_seen_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

async fn list(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Vec<Device>>> {
    u.need("devices.view")?;
    Ok(Json(
        sqlx::query_as(
            "SELECT id, name, kind, serial_no, CAST(api_key_hash IS NOT NULL AS SIGNED) AS has_key, branch_id, is_enabled,
                    last_seen_at, created_at FROM devices ORDER BY id",
        )
        .fetch_all(&st.db)
        .await?,
    ))
}

#[derive(Deserialize)]
struct DeviceReq {
    name: String,
    kind: String,
    serial_no: Option<String>,
    branch_id: Option<i64>,
    is_enabled: Option<bool>,
}

impl DeviceReq {
    fn validate(&self) -> AppResult<Option<String>> {
        if self.name.trim().is_empty() {
            return Err(bad("กรุณาระบุชื่ออุปกรณ์"));
        }
        if !DEVICE_KINDS.contains(&self.kind.as_str()) {
            return Err(bad("ประเภทอุปกรณ์ไม่ถูกต้อง"));
        }
        let sn = self.serial_no.as_ref().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        if self.kind == "zkteco" && sn.is_none() {
            return Err(bad("เครื่อง ZKTeco ต้องระบุ Serial Number (SN)"));
        }
        Ok(sn)
    }
}

async fn create(State(st): State<AppState>, u: AuthUser, Json(r): Json<DeviceReq>) -> AppResult<Json<Value>> {
    u.need("devices.manage")?;
    let sn = r.validate()?;
    // ZKTeco ADMS identifies itself by serial number only, every other kind uses an API key
    let key = (r.kind != "zkteco").then(|| random_token(24));
    let id = sqlx::query(
        "INSERT INTO devices (name, kind, serial_no, api_key_hash, branch_id, is_enabled) VALUES (?,?,?,?,?,?)",
    )
    .bind(r.name.trim())
    .bind(&r.kind)
    .bind(sn)
    .bind(key.as_deref().map(sha256_hex))
    .bind(r.branch_id)
    .bind(r.is_enabled.unwrap_or(true))
    .execute(&st.db)
    .await?
    .last_insert_id();
    audit(&st, Some(&u), "create", "device", id, json!({ "name": r.name, "kind": r.kind })).await;
    Ok(Json(json!({ "id": id, "api_key": key })))
}

async fn update(
    State(st): State<AppState>,
    u: AuthUser,
    Path(id): Path<i64>,
    Json(r): Json<DeviceReq>,
) -> AppResult<Json<Value>> {
    u.need("devices.manage")?;
    let sn = r.validate()?;
    sqlx::query("UPDATE devices SET name=?, kind=?, serial_no=?, branch_id=?, is_enabled=? WHERE id=?")
        .bind(r.name.trim())
        .bind(&r.kind)
        .bind(sn)
        .bind(r.branch_id)
        .bind(r.is_enabled.unwrap_or(true))
        .bind(id)
        .execute(&st.db)
        .await?;
    audit(&st, Some(&u), "update", "device", id, json!({ "name": r.name, "is_enabled": r.is_enabled })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn remove(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("devices.manage")?;
    sqlx::query("DELETE FROM devices WHERE id = ?").bind(id).execute(&st.db).await?;
    audit(&st, Some(&u), "delete", "device", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

async fn rotate(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("devices.manage")?;
    let key = random_token(24);
    sqlx::query("UPDATE devices SET api_key_hash = ? WHERE id = ?")
        .bind(sha256_hex(&key))
        .bind(id)
        .execute(&st.db)
        .await?;
    audit(&st, Some(&u), "rotate_key", "device", id, json!({})).await;
    Ok(Json(json!({ "api_key": key })))
}
