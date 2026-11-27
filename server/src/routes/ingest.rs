//! Machine-to-machine endpoints:
//! * AI worker: configuration, activity segments, live heartbeat
//! * Attendance terminals: generic JSON, Hikvision ISAPI event push, ZKTeco ADMS (iclock)

use std::collections::HashMap;

use axum::{
    body::Bytes,
    extract::{Query, State},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{MySql, QueryBuilder};

use crate::{
    error::{bad, AppError, AppResult},
    routes::cameras::all_cameras,
    state::{AppState, LiveDesk},
    util::{is_at_desk, is_valid_state, local_date_of, parse_scan_time, setting_f64},
};

use crate::auth::DeviceAuth;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/ingest/config", get(worker_config))
        .route("/ingest/events", post(ingest_events))
        .route("/ingest/heartbeat", post(heartbeat))
        .route("/ingest/attendance", post(ingest_attendance))
        .route("/ingest/hikvision", post(ingest_hikvision))
}

pub fn iclock_routes() -> Router<AppState> {
    Router::new()
        .route("/iclock/cdata", get(iclock_handshake).post(iclock_upload))
        .route("/iclock/getrequest", get(iclock_ok))
        .route("/iclock/devicecmd", post(iclock_ok))
}

fn require_worker(d: &DeviceAuth) -> AppResult<()> {
    if d.kind == "ai_worker" {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

// ---------------------------------------------------------------- AI worker

async fn worker_config(State(st): State<AppState>, dev: DeviceAuth) -> AppResult<Json<Value>> {
    require_worker(&dev)?;
    let cams = all_cameras(&st).await?;
    let ws: Vec<(i64, i64, String, String, f64, f64)> = sqlx::query_as(
        "SELECT id, camera_id, name, roi, screen_yaw_deg, yaw_tolerance_deg FROM workstations ORDER BY id",
    )
    .fetch_all(&st.db)
    .await?;

    let cameras: Vec<Value> = cams
        .into_iter()
        .filter(|c| c.ai_enabled)
        .map(|c| {
            let source = match &st.cfg.go2rtc_rtsp {
                Some(base) => format!("{}/{}", base.trim_end_matches('/'), c.stream_name),
                None => c.rtsp_url.clone(),
            };
            let desks: Vec<Value> = ws
                .iter()
                .filter(|w| w.1 == c.id)
                .map(|w| {
                    json!({
                        "id": w.0, "name": w.2,
                        "roi": serde_json::from_str::<Value>(&w.3).unwrap_or(Value::Null),
                        "screen_yaw_deg": w.4, "yaw_tolerance_deg": w.5,
                    })
                })
                .collect();
            json!({ "id": c.id, "name": c.name, "source": source, "fps": c.ai_fps, "width": c.ai_width, "workstations": desks })
        })
        .collect();

    Ok(Json(json!({
        "cameras": cameras,
        "detection": {
            "min_state_sec": setting_f64(&st.db, "min_state_sec", 5.0).await,
            "away_grace_sec": setting_f64(&st.db, "away_grace_sec", 20.0).await,
            "confidence": setting_f64(&st.db, "detect_confidence", 0.45).await,
        }
    })))
}

#[derive(Deserialize)]
struct SegmentIn {
    workstation_id: i64,
    state: String,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    confidence: Option<f64>,
}

#[derive(Deserialize)]
struct EventsReq {
    events: Vec<SegmentIn>,
}

async fn workstation_map(st: &AppState) -> AppResult<HashMap<i64, (i64, Option<i64>)>> {
    let rows: Vec<(i64, i64, Option<i64>)> =
        sqlx::query_as("SELECT id, camera_id, employee_id FROM workstations").fetch_all(&st.db).await?;
    Ok(rows.into_iter().map(|(id, c, e)| (id, (c, e))).collect())
}

async fn ingest_events(State(st): State<AppState>, dev: DeviceAuth, Json(req): Json<EventsReq>) -> AppResult<Json<Value>> {
    require_worker(&dev)?;
    if req.events.len() > 2000 {
        return Err(bad("too many events in one batch (max 2000)"));
    }
    let wsmap = workstation_map(&st).await?;
    let now = Utc::now();
    let off = st.cfg.tz_offset_min;
    let mut rows = Vec::new();
    let mut skipped = 0;
    for e in req.events {
        let dur = (e.ended_at - e.started_at).num_milliseconds();
        let valid = is_valid_state(&e.state)
            && dur > 0
            && dur <= 86_400_000
            && e.ended_at <= now + Duration::minutes(5);
        match (valid, wsmap.get(&e.workstation_id)) {
            (true, Some(&(cam, emp))) => rows.push((e, cam, emp, ((dur + 500) / 1000) as i32)),
            _ => skipped += 1,
        }
    }
    let mut dates = Vec::new();
    if !rows.is_empty() {
        let mut qb: QueryBuilder<MySql> = QueryBuilder::new(
            "INSERT INTO activity_events (workstation_id, camera_id, employee_id, state, started_at, ended_at, duration_sec, confidence) ",
        );
        qb.push_values(rows.iter(), |mut b, (e, cam, emp, dur)| {
            b.push_bind(e.workstation_id)
                .push_bind(*cam)
                .push_bind(*emp)
                .push_bind(&e.state)
                .push_bind(e.started_at)
                .push_bind(e.ended_at)
                .push_bind(*dur)
                .push_bind(e.confidence.unwrap_or(0.0).clamp(0.0, 1.0));
        });
        qb.build().execute(&st.db).await?;
        for (e, ..) in &rows {
            dates.push(local_date_of(e.started_at, off));
            dates.push(local_date_of(e.ended_at, off));
        }
        st.mark_dirty(dates).await;
        st.broadcast(json!({ "type": "events", "count": rows.len() }));
    }
    Ok(Json(json!({ "inserted": rows.len(), "skipped": skipped })))
}

#[derive(Deserialize)]
struct CamStatus {
    camera_id: i64,
    online: bool,
    fps: Option<f64>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct DeskStatus {
    workstation_id: i64,
    state: String,
    confidence: Option<f64>,
    since: Option<DateTime<Utc>>,
    yaw: Option<f64>,
}

#[derive(Deserialize)]
struct HeartbeatReq {
    #[serde(default)]
    cameras: Vec<CamStatus>,
    #[serde(default)]
    desks: Vec<DeskStatus>,
}

async fn heartbeat(State(st): State<AppState>, dev: DeviceAuth, Json(req): Json<HeartbeatReq>) -> AppResult<Json<Value>> {
    require_worker(&dev)?;
    let now = Utc::now();

    // ---- camera status
    let old: HashMap<i64, String> = sqlx::query_as::<_, (i64, String)>("SELECT id, status FROM cameras")
        .fetch_all(&st.db)
        .await?
        .into_iter()
        .collect();
    for c in &req.cameras {
        let status = if c.online { "online" } else { "offline" };
        if !old.contains_key(&c.camera_id) {
            continue;
        }
        sqlx::query("UPDATE cameras SET status = ?, last_seen_at = IF(?, UTC_TIMESTAMP(), last_seen_at) WHERE id = ?")
            .bind(status)
            .bind(c.online)
            .bind(c.camera_id)
            .execute(&st.db)
            .await?;
        if old.get(&c.camera_id).map(|s| s.as_str()) != Some(status) {
            st.broadcast(json!({ "type": "camera", "camera_id": c.camera_id, "status": status, "fps": c.fps, "error": c.error }));
        }
    }

    // ---- live desks + wellness (continuous sitting)
    let wsmap = workstation_map(&st).await?;
    let alert_min = setting_f64(&st.db, "sitting_alert_min", 50.0).await.max(10.0);
    let mut alerts: Vec<(i64, i64)> = Vec::new();
    let mut snapshot = Vec::new();
    {
        let mut live = st.live.write().await;
        for d in req.desks {
            let Some(&(cam, emp)) = wsmap.get(&d.workstation_id) else { continue };
            if !is_valid_state(&d.state) {
                continue;
            }
            let since = d.since.unwrap_or(now);
            let entry = live.entry(d.workstation_id).or_insert_with(|| LiveDesk {
                workstation_id: d.workstation_id,
                camera_id: cam,
                employee_id: emp,
                state: d.state.clone(),
                confidence: 0.0,
                since,
                sitting_since: None,
                yaw: None,
                updated_at: now,
                sitting_alerted: false,
            });
            entry.camera_id = cam;
            entry.employee_id = emp;
            entry.state = d.state.clone();
            entry.confidence = d.confidence.unwrap_or(0.0);
            entry.since = since;
            entry.updated_at = now;
            if d.yaw.is_some() {
                entry.yaw = d.yaw;
            }
            if is_at_desk(&d.state) {
                if entry.sitting_since.is_none() {
                    entry.sitting_since = Some(since);
                    entry.sitting_alerted = false;
                }
            } else if d.state == "away" {
                entry.sitting_since = None;
                entry.sitting_alerted = false;
            }
            if let (Some(ss), Some(e)) = (entry.sitting_since, entry.employee_id) {
                let mins = (now - ss).num_minutes();
                if mins >= alert_min as i64 && !entry.sitting_alerted {
                    entry.sitting_alerted = true;
                    alerts.push((e, mins));
                }
            }
            snapshot.push(entry.clone());
        }
    }
    for (emp, mins) in alerts {
        sqlx::query("INSERT INTO notifications (employee_id, kind, title, body) VALUES (?, 'wellness_sitting', ?, ?)")
            .bind(emp)
            .bind("ถึงเวลาพักสายตาและยืดเส้นยืดสาย")
            .bind(format!(
                "นั่งทำงานต่อเนื่องประมาณ {mins} นาที แนะนำให้ลุกเดิน 3-5 นาที และพักสายตาตามหลัก 20-20-20 (ทุก 20 นาที มองไกล 20 ฟุต นาน 20 วินาที)"
            ))
            .execute(&st.db)
            .await?;
        st.broadcast(json!({ "type": "wellness", "employee_id": emp, "minutes": mins }));
    }
    if !snapshot.is_empty() {
        st.broadcast(json!({ "type": "live", "desks": snapshot }));
    }
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- attendance

pub struct Scan {
    pub attendance_ref: String,
    pub time: DateTime<Utc>,
    pub direction: String,
    pub method: String,
    pub raw: Option<String>,
}

fn norm_direction(s: Option<&str>) -> String {
    match s.map(|v| v.to_ascii_lowercase()) {
        Some(v) if ["in", "checkin", "check_in", "0"].contains(&v.as_str()) => "in".into(),
        Some(v) if ["out", "checkout", "check_out", "1"].contains(&v.as_str()) => "out".into(),
        _ => "unknown".into(),
    }
}

fn norm_method(s: Option<&str>) -> String {
    let v = s.unwrap_or("").to_ascii_lowercase();
    if v.contains("face") {
        "face".into()
    } else if v.contains("finger") {
        "finger".into()
    } else if v.contains("card") {
        "card".into()
    } else if v.contains("pass") || v.contains("pin") {
        "password".into()
    } else if v == "manual" {
        "manual".into()
    } else {
        "other".into()
    }
}

/// Stores scans (duplicates are ignored) and links them to employees by `attendance_ref`.
pub async fn store_scans(st: &AppState, device_id: Option<i64>, scans: Vec<Scan>) -> AppResult<u64> {
    let off = st.cfg.tz_offset_min;
    let mut inserted = 0;
    let mut dates = Vec::new();
    for s in scans {
        if s.attendance_ref.is_empty() || s.attendance_ref.len() > 60 {
            continue;
        }
        let r = sqlx::query(
            "INSERT IGNORE INTO attendance_logs (employee_id, attendance_ref, device_id, scan_time, direction, method, raw)
             VALUES ((SELECT id FROM employees WHERE attendance_ref = ? LIMIT 1), ?, ?, ?, ?, ?, ?)",
        )
        .bind(&s.attendance_ref)
        .bind(&s.attendance_ref)
        .bind(device_id)
        .bind(s.time)
        .bind(&s.direction)
        .bind(&s.method)
        .bind(s.raw.as_deref().map(|r| r.chars().take(500).collect::<String>()))
        .execute(&st.db)
        .await?;
        inserted += r.rows_affected();
        dates.push(local_date_of(s.time, off));
    }
    if inserted > 0 {
        st.mark_dirty(dates).await;
        st.broadcast(json!({ "type": "attendance", "count": inserted }));
    }
    Ok(inserted)
}

#[derive(Deserialize)]
struct AttIn {
    attendance_ref: String,
    scan_time: String,
    direction: Option<String>,
    method: Option<String>,
}

#[derive(Deserialize)]
struct AttReq {
    records: Vec<AttIn>,
}

async fn ingest_attendance(State(st): State<AppState>, dev: DeviceAuth, Json(req): Json<AttReq>) -> AppResult<Json<Value>> {
    if dev.kind == "ai_worker" {
        return Err(AppError::Forbidden);
    }
    let off = st.cfg.tz_offset_min;
    let mut scans = Vec::new();
    let mut invalid = 0;
    for r in req.records {
        match parse_scan_time(&r.scan_time, off) {
            Some(t) => scans.push(Scan {
                attendance_ref: r.attendance_ref.trim().to_string(),
                time: t,
                direction: norm_direction(r.direction.as_deref()),
                method: norm_method(r.method.as_deref()),
                raw: None,
            }),
            None => invalid += 1,
        }
    }
    let n = store_scans(&st, Some(dev.id), scans).await?;
    Ok(Json(json!({ "inserted": n, "invalid": invalid })))
}

/// Hikvision access-control / face terminals push `AccessControllerEvent` JSON
/// (either raw JSON or inside multipart form data) to an HTTP listener.
/// Configure the terminal's HTTP host to `http://SERVER:8080/api/ingest/hikvision?key=DEVICE_KEY`.
async fn ingest_hikvision(State(st): State<AppState>, dev: DeviceAuth, body: Bytes) -> AppResult<&'static str> {
    if dev.kind == "ai_worker" {
        return Err(AppError::Forbidden);
    }
    let text = String::from_utf8_lossy(&body);
    let (Some(a), Some(b)) = (text.find('{'), text.rfind('}')) else { return Ok("OK") };
    let Ok(v) = serde_json::from_str::<Value>(&text[a..=b]) else { return Ok("OK") };
    let ev = &v["AccessControllerEvent"];
    let emp = ev["employeeNoString"]
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| ev["employeeNo"].as_i64().map(|n| n.to_string()));
    let (Some(emp), Some(t)) = (emp, v["dateTime"].as_str().and_then(|t| parse_scan_time(t, st.cfg.tz_offset_min))) else {
        return Ok("OK");
    };
    let scan = Scan {
        attendance_ref: emp,
        time: t,
        direction: norm_direction(ev["attendanceStatus"].as_str()),
        method: norm_method(ev["currentVerifyMode"].as_str()),
        raw: Some(ev.to_string()),
    };
    store_scans(&st, Some(dev.id), vec![scan]).await?;
    Ok("OK")
}

// ---------------------------------------------------------------- ZKTeco ADMS

async fn zk_device(st: &AppState, sn: &str) -> AppResult<Option<(i64, bool)>> {
    if sn.is_empty() || sn.len() > 80 {
        return Ok(None);
    }
    let row: Option<(i64, bool)> = sqlx::query_as("SELECT id, is_enabled FROM devices WHERE serial_no = ?")
        .bind(sn)
        .fetch_optional(&st.db)
        .await?;
    if row.is_some() {
        sqlx::query("UPDATE devices SET last_seen_at = UTC_TIMESTAMP() WHERE serial_no = ?")
            .bind(sn)
            .execute(&st.db)
            .await?;
        return Ok(row);
    }
    // unknown terminal: register it disabled so an admin can approve it
    sqlx::query("INSERT IGNORE INTO devices (name, kind, serial_no, is_enabled, last_seen_at) VALUES (?, 'zkteco', ?, 0, UTC_TIMESTAMP())")
        .bind(format!("ZKTeco {sn} (รออนุมัติ)"))
        .bind(sn)
        .execute(&st.db)
        .await?;
    tracing::info!("new ZKTeco terminal {sn} registered (disabled until approved)");
    Ok(None)
}

async fn iclock_handshake(State(st): State<AppState>, Query(q): Query<HashMap<String, String>>) -> AppResult<String> {
    let sn = q.get("SN").cloned().unwrap_or_default();
    zk_device(&st, &sn).await?;
    let tz = st.cfg.tz_offset_min / 60;
    Ok(format!(
        "GET OPTION FROM: {sn}\nATTLOGStamp=None\nOPERLOGStamp=9999\nATTPHOTOStamp=None\nErrorDelay=30\nDelay=10\n\
         TransTimes=00:00;14:05\nTransInterval=1\nTransFlag=TransData AttLog\nTimeZone={tz}\nRealtime=1\nEncrypt=None\n"
    ))
}

fn zk_method(verify: &str) -> &'static str {
    match verify {
        "1" => "finger",
        "15" => "face",
        "0" => "password",
        "2" | "3" | "4" => "card",
        _ => "other",
    }
}

async fn iclock_upload(
    State(st): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
    body: String,
) -> AppResult<String> {
    let sn = q.get("SN").cloned().unwrap_or_default();
    let table = q.get("table").cloned().unwrap_or_default();
    let dev = zk_device(&st, &sn).await?;
    let Some((dev_id, true)) = dev else { return Ok("OK".into()) };
    if table != "ATTLOG" {
        return Ok("OK".into());
    }
    let off = st.cfg.tz_offset_min;
    let mut scans = Vec::new();
    for line in body.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 2 {
            continue;
        }
        let Some(t) = parse_scan_time(f[1], off) else { continue };
        let status = f.get(2).copied().unwrap_or("");
        let direction = match status {
            "0" | "4" => "in",
            "1" | "5" => "out",
            _ => "unknown",
        };
        scans.push(Scan {
            attendance_ref: f[0].trim().to_string(),
            time: t,
            direction: direction.into(),
            method: zk_method(f.get(3).copied().unwrap_or("")).into(),
            raw: Some(line.to_string()),
        });
    }
    let n = scans.len();
    store_scans(&st, Some(dev_id), scans).await?;
    Ok(format!("OK: {n}"))
}

async fn iclock_ok() -> &'static str {
    "OK"
}
