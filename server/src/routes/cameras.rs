//! Camera (VMS) management and workstation / detection-zone management.

use axum::{
    extract::{Path, Query, State},
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
    util::random_token,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/cameras", get(list_cameras).post(create_camera))
        .route("/cameras/{id}", put(update_camera).delete(delete_camera))
        .route("/workstations", get(list_workstations).post(create_workstation))
        .route("/workstations/{id}", put(update_workstation).delete(delete_workstation))
        .route("/workstations/{id}/calibrate", post(calibrate))
}

#[derive(Serialize, sqlx::FromRow, Clone)]
pub struct Camera {
    pub id: i64,
    pub name: String,
    pub branch_id: Option<i64>,
    pub branch_name: Option<String>,
    pub floor: Option<String>,
    pub location: Option<String>,
    pub brand: Option<String>,
    pub rtsp_url: String,
    pub onvif_host: Option<String>,
    pub stream_name: String,
    pub ai_enabled: bool,
    pub ai_fps: f64,
    pub ai_width: i32,
    pub ptz_supported: bool,
    pub status: String,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub workstation_count: i64,
}

const CAMERA_SELECT: &str = "SELECT c.id, c.name, c.branch_id, b.name AS branch_name, c.floor, c.location, c.brand,
    c.rtsp_url, c.onvif_host, c.stream_name, c.ai_enabled, c.ai_fps, c.ai_width, c.ptz_supported, c.status,
    c.last_seen_at, (SELECT COUNT(*) FROM workstations w WHERE w.camera_id = c.id) AS workstation_count
    FROM cameras c LEFT JOIN branches b ON b.id = c.branch_id";

/// Hides the password part of `rtsp://user:pass@host/...`.
pub fn mask_url(url: &str) -> String {
    if let Some((scheme, rest)) = url.split_once("://") {
        if let Some((cred, host)) = rest.split_once('@') {
            if let Some((user, _)) = cred.split_once(':') {
                return format!("{scheme}://{user}:****@{host}");
            }
        }
    }
    url.to_string()
}

pub async fn all_cameras(st: &AppState) -> AppResult<Vec<Camera>> {
    Ok(sqlx::query_as::<_, Camera>(&format!("{CAMERA_SELECT} ORDER BY c.id")).fetch_all(&st.db).await?)
}

async fn list_cameras(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Vec<Camera>>> {
    u.require_staff()?;
    let mut cams = all_cameras(&st).await?;
    for c in cams.iter_mut() {
        c.rtsp_url = mask_url(&c.rtsp_url);
    }
    Ok(Json(cams))
}

#[derive(Deserialize)]
struct CameraReq {
    name: String,
    branch_id: Option<i64>,
    floor: Option<String>,
    location: Option<String>,
    brand: Option<String>,
    rtsp_url: String,
    onvif_host: Option<String>,
    ai_enabled: Option<bool>,
    ai_fps: Option<f64>,
    ai_width: Option<i32>,
    ptz_supported: Option<bool>,
}

impl CameraReq {
    fn validate(&self) -> AppResult<()> {
        if self.name.trim().is_empty() {
            return Err(bad("กรุณาระบุชื่อกล้อง"));
        }
        let u = self.rtsp_url.trim();
        if !(u.starts_with("rtsp://") || u.starts_with("rtsps://") || u.starts_with("http://") || u.starts_with("https://")) {
            return Err(bad("URL ของกล้องต้องขึ้นต้นด้วย rtsp:// หรือ http(s)://"));
        }
        let fps = self.ai_fps.unwrap_or(2.0);
        if !(0.2..=15.0).contains(&fps) {
            return Err(bad("FPS สำหรับ AI ต้องอยู่ระหว่าง 0.2 - 15"));
        }
        let w = self.ai_width.unwrap_or(960);
        if !(320..=1920).contains(&w) {
            return Err(bad("ความกว้างภาพสำหรับ AI ต้องอยู่ระหว่าง 320 - 1920"));
        }
        Ok(())
    }
}

/// Registers (or re-registers) a camera stream in go2rtc so browsers can view it
/// via WebRTC/MSE and the AI worker can read one shared RTSP restream.
pub async fn sync_stream(st: &AppState, name: &str, src: &str) {
    let Some(api) = &st.cfg.go2rtc_api else { return };
    let url = format!("{}/api/streams", api.trim_end_matches('/'));
    let r = st.http.put(&url).query(&[("name", name), ("src", src)]).send().await;
    match r {
        Ok(resp) if resp.status().is_success() => {}
        Ok(resp) => tracing::warn!("go2rtc rejected stream {name}: {}", resp.status()),
        Err(e) => tracing::warn!("go2rtc not reachable for stream {name}: {e}"),
    }
}

async fn remove_stream(st: &AppState, name: &str) {
    let Some(api) = &st.cfg.go2rtc_api else { return };
    let url = format!("{}/api/streams", api.trim_end_matches('/'));
    let _ = st.http.delete(&url).query(&[("src", name)]).send().await;
}

/// Registers every camera that go2rtc does not know yet (e.g. after go2rtc restarted).
/// Streams that already exist are left untouched so live viewers are not interrupted.
pub async fn sync_all_streams(st: &AppState) {
    let Some(api) = &st.cfg.go2rtc_api else { return };
    let url = format!("{}/api/streams", api.trim_end_matches('/'));
    let existing: std::collections::HashSet<String> = match st.http.get(&url).send().await {
        Ok(r) if r.status().is_success() => r
            .json::<serde_json::Map<String, Value>>()
            .await
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default(),
        Ok(r) => {
            tracing::warn!("go2rtc stream list failed: {}", r.status());
            return;
        }
        Err(e) => {
            tracing::debug!("go2rtc not reachable: {e}");
            return;
        }
    };
    match all_cameras(st).await {
        Ok(cams) => {
            for c in cams.into_iter().filter(|c| !existing.contains(&c.stream_name)) {
                sync_stream(st, &c.stream_name, &c.rtsp_url).await;
            }
        }
        Err(e) => tracing::warn!("could not load cameras for go2rtc sync: {e:?}"),
    }
}

async fn create_camera(State(st): State<AppState>, u: AuthUser, Json(r): Json<CameraReq>) -> AppResult<Json<Value>> {
    u.require(&[])?;
    r.validate()?;
    let stream = format!("cam_{}", random_token(4));
    let id = sqlx::query(
        "INSERT INTO cameras (name, branch_id, floor, location, brand, rtsp_url, onvif_host, stream_name,
           ai_enabled, ai_fps, ai_width, ptz_supported) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(r.name.trim())
    .bind(r.branch_id)
    .bind(&r.floor)
    .bind(&r.location)
    .bind(&r.brand)
    .bind(r.rtsp_url.trim())
    .bind(&r.onvif_host)
    .bind(&stream)
    .bind(r.ai_enabled.unwrap_or(true))
    .bind(r.ai_fps.unwrap_or(2.0))
    .bind(r.ai_width.unwrap_or(960))
    .bind(r.ptz_supported.unwrap_or(false))
    .execute(&st.db)
    .await?
    .last_insert_id();
    sync_stream(&st, &stream, r.rtsp_url.trim()).await;
    audit(&st, Some(&u), "create", "camera", id, json!({ "name": r.name })).await;
    Ok(Json(json!({ "id": id, "stream_name": stream })))
}

async fn update_camera(
    State(st): State<AppState>,
    u: AuthUser,
    Path(id): Path<i64>,
    Json(r): Json<CameraReq>,
) -> AppResult<Json<Value>> {
    u.require(&[])?;
    r.validate()?;
    let (old_url, stream): (String, String) =
        sqlx::query_as("SELECT rtsp_url, stream_name FROM cameras WHERE id = ?").bind(id).fetch_one(&st.db).await?;
    // the UI only ever sees the masked URL; keep the stored secret unless a new one was typed
    let url = if r.rtsp_url.contains("****") { old_url.clone() } else { r.rtsp_url.trim().to_string() };
    sqlx::query(
        "UPDATE cameras SET name=?, branch_id=?, floor=?, location=?, brand=?, rtsp_url=?, onvif_host=?,
           ai_enabled=?, ai_fps=?, ai_width=?, ptz_supported=? WHERE id=?",
    )
    .bind(r.name.trim())
    .bind(r.branch_id)
    .bind(&r.floor)
    .bind(&r.location)
    .bind(&r.brand)
    .bind(&url)
    .bind(&r.onvif_host)
    .bind(r.ai_enabled.unwrap_or(true))
    .bind(r.ai_fps.unwrap_or(2.0))
    .bind(r.ai_width.unwrap_or(960))
    .bind(r.ptz_supported.unwrap_or(false))
    .bind(id)
    .execute(&st.db)
    .await?;
    if url != old_url {
        sync_stream(&st, &stream, &url).await;
    }
    audit(&st, Some(&u), "update", "camera", id, json!({ "name": r.name, "ai_enabled": r.ai_enabled })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_camera(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.require(&[])?;
    let row: Option<(String,)> =
        sqlx::query_as("SELECT stream_name FROM cameras WHERE id = ?").bind(id).fetch_optional(&st.db).await?;
    sqlx::query("DELETE FROM cameras WHERE id = ?").bind(id).execute(&st.db).await?;
    if let Some((stream,)) = row {
        remove_stream(&st, &stream).await;
    }
    st.live.write().await.retain(|_, d| d.camera_id != id);
    audit(&st, Some(&u), "delete", "camera", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------- workstations ----------

#[derive(Serialize, sqlx::FromRow)]
pub struct Workstation {
    pub id: i64,
    pub camera_id: i64,
    pub camera_name: String,
    pub name: String,
    pub employee_id: Option<i64>,
    pub employee_name: Option<String>,
    #[serde(serialize_with = "crate::util::ser_json_str")]
    pub roi: String,
    pub screen_yaw_deg: f64,
    pub yaw_tolerance_deg: f64,
}

pub const WS_SELECT: &str = "SELECT w.id, w.camera_id, c.name AS camera_name, w.name, w.employee_id,
    CONCAT(e.first_name, ' ', e.last_name) AS employee_name, w.roi, w.screen_yaw_deg, w.yaw_tolerance_deg
    FROM workstations w JOIN cameras c ON c.id = w.camera_id LEFT JOIN employees e ON e.id = w.employee_id";

#[derive(Deserialize)]
struct WsFilter {
    camera_id: Option<i64>,
}

async fn list_workstations(
    State(st): State<AppState>,
    u: AuthUser,
    Query(f): Query<WsFilter>,
) -> AppResult<Json<Vec<Workstation>>> {
    u.require_staff()?;
    let rows = match f.camera_id {
        Some(c) => {
            sqlx::query_as::<_, Workstation>(&format!("{WS_SELECT} WHERE w.camera_id = ? ORDER BY w.name"))
                .bind(c)
                .fetch_all(&st.db)
                .await?
        }
        None => {
            sqlx::query_as::<_, Workstation>(&format!("{WS_SELECT} ORDER BY c.name, w.name"))
                .fetch_all(&st.db)
                .await?
        }
    };
    Ok(Json(rows))
}

#[derive(Deserialize)]
struct WsReq {
    camera_id: i64,
    name: String,
    employee_id: Option<i64>,
    roi: Option<Vec<[f64; 2]>>,
    screen_yaw_deg: Option<f64>,
    yaw_tolerance_deg: Option<f64>,
}

fn validate_roi(roi: &Option<Vec<[f64; 2]>>) -> AppResult<String> {
    let roi = match roi {
        Some(r) if !r.is_empty() => r.clone(),
        // default: whole frame until the admin draws a zone
        _ => vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
    };
    if roi.len() < 3 || roi.len() > 32 {
        return Err(bad("พื้นที่ตรวจจับต้องมี 3-32 จุด"));
    }
    if roi.iter().any(|p| !(0.0..=1.0).contains(&p[0]) || !(0.0..=1.0).contains(&p[1])) {
        return Err(bad("พิกัดพื้นที่ตรวจจับต้องอยู่ระหว่าง 0 ถึง 1"));
    }
    Ok(serde_json::to_string(&roi).unwrap())
}

async fn create_workstation(State(st): State<AppState>, u: AuthUser, Json(r): Json<WsReq>) -> AppResult<Json<Value>> {
    u.require(&[])?;
    if r.name.trim().is_empty() {
        return Err(bad("กรุณาระบุชื่อโต๊ะทำงาน"));
    }
    let roi = validate_roi(&r.roi)?;
    let id = sqlx::query(
        "INSERT INTO workstations (camera_id, name, employee_id, roi, screen_yaw_deg, yaw_tolerance_deg) VALUES (?,?,?,?,?,?)",
    )
    .bind(r.camera_id)
    .bind(r.name.trim())
    .bind(r.employee_id)
    .bind(roi)
    .bind(r.screen_yaw_deg.unwrap_or(0.0))
    .bind(r.yaw_tolerance_deg.unwrap_or(30.0).clamp(5.0, 90.0))
    .execute(&st.db)
    .await?
    .last_insert_id();
    audit(&st, Some(&u), "create", "workstation", id, json!({ "name": r.name, "employee_id": r.employee_id })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_workstation(
    State(st): State<AppState>,
    u: AuthUser,
    Path(id): Path<i64>,
    Json(r): Json<WsReq>,
) -> AppResult<Json<Value>> {
    u.require(&[])?;
    let roi = validate_roi(&r.roi)?;
    sqlx::query(
        "UPDATE workstations SET camera_id=?, name=?, employee_id=?, roi=?, screen_yaw_deg=?, yaw_tolerance_deg=? WHERE id=?",
    )
    .bind(r.camera_id)
    .bind(r.name.trim())
    .bind(r.employee_id)
    .bind(roi)
    .bind(r.screen_yaw_deg.unwrap_or(0.0))
    .bind(r.yaw_tolerance_deg.unwrap_or(30.0).clamp(5.0, 90.0))
    .bind(id)
    .execute(&st.db)
    .await?;
    if let Some(d) = st.live.write().await.get_mut(&id) {
        d.employee_id = r.employee_id;
    }
    audit(&st, Some(&u), "update", "workstation", id, json!({ "name": r.name, "employee_id": r.employee_id })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_workstation(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.require(&[])?;
    sqlx::query("DELETE FROM workstations WHERE id = ?").bind(id).execute(&st.db).await?;
    st.live.write().await.remove(&id);
    audit(&st, Some(&u), "delete", "workstation", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}

/// Uses the most recently measured head yaw as the "looking at the screen" direction.
async fn calibrate(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.require(&[])?;
    let yaw = st
        .live
        .read()
        .await
        .get(&id)
        .and_then(|d| d.yaw)
        .ok_or_else(|| bad("ยังไม่มีข้อมูลทิศทางใบหน้าจาก AI สำหรับโต๊ะนี้ ให้ผู้ใช้นั่งมองจอแล้วลองใหม่"))?;
    sqlx::query("UPDATE workstations SET screen_yaw_deg = ? WHERE id = ?")
        .bind(yaw)
        .bind(id)
        .execute(&st.db)
        .await?;
    audit(&st, Some(&u), "calibrate", "workstation", id, json!({ "screen_yaw_deg": yaw })).await;
    Ok(Json(json!({ "screen_yaw_deg": yaw })))
}
