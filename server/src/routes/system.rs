//! Dashboard overview, live state, settings, audit logs, resources, backup and WebSocket.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::header,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::{
    auth::{audit, authenticate, AuthUser},
    error::{bad, AppError, AppResult},
    state::AppState,
    util::{day_range, local_today, setting_str},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/public/config", get(public_config))
        .route("/dashboard/overview", get(overview))
        .route("/live", get(live))
        .route("/settings", get(get_settings).put(put_settings))
        .route("/audit", get(audit_logs))
        .route("/system/resources", get(resources))
        .route("/system/backup", get(backup))
        .route("/ws", get(ws_handler))
}

const SETTING_KEYS: [&str; 13] = [
    "report_doc_prefix",
    "report_show_activity",
    "ticket_sla_hours",
    "org_name",
    "report_header",
    "report_footer",
    "logo_url",
    "retention_days",
    "sitting_alert_min",
    "min_state_sec",
    "away_grace_sec",
    "detect_confidence",
    "monitoring_notice",
];

async fn public_config(State(st): State<AppState>) -> Json<Value> {
    Json(json!({
        "org_name": setting_str(&st.db, "org_name", "").await,
        "monitoring_notice": setting_str(&st.db, "monitoring_notice", "").await,
        "video": st.cfg.go2rtc_api.is_some(),
        "tz_offset_min": st.cfg.tz_offset_min,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn overview(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    u.need("dashboard.view")?;
    let off = st.cfg.tz_offset_min;
    let today = local_today(off);
    let (start, end) = day_range(today, off);

    let (cam_total, cam_online, cam_ai): (i64, i64, i64) = sqlx::query_as(
        "SELECT CAST(COUNT(*) AS SIGNED), CAST(COALESCE(SUM(status = 'online'), 0) AS SIGNED),
                CAST(COALESCE(SUM(ai_enabled), 0) AS SIGNED) FROM cameras",
    )
    .fetch_one(&st.db)
    .await?;
    let (employees,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM employees WHERE status = 'active'").fetch_one(&st.db).await?;
    let (workstations,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM workstations").fetch_one(&st.db).await?;
    let (scanned_today,): (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT employee_id) FROM attendance_logs WHERE employee_id IS NOT NULL AND scan_time >= ? AND scan_time < ?",
    )
    .bind(start)
    .bind(end)
    .fetch_one(&st.db)
    .await?;
    let (pending_corrections,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM data_corrections WHERE status = 'pending'").fetch_one(&st.db).await?;
    let (wellness_today,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM notifications WHERE kind = 'wellness_sitting' AND created_at >= ? AND created_at < ?",
    )
    .bind(start)
    .bind(end)
    .fetch_one(&st.db)
    .await?;
    let (devices_total, devices_seen): (i64, i64) = sqlx::query_as(
        "SELECT CAST(COUNT(*) AS SIGNED),
                CAST(COALESCE(SUM(last_seen_at > UTC_TIMESTAMP() - INTERVAL 10 MINUTE), 0) AS SIGNED)
         FROM devices WHERE is_enabled = 1",
    )
    .fetch_one(&st.db)
    .await?;
    let totals: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT CAST(COALESCE(SUM(focused_sec), 0) AS SIGNED), CAST(COALESCE(SUM(present_sec), 0) AS SIGNED),
                CAST(COALESCE(SUM(phone_sec), 0) AS SIGNED), CAST(COALESCE(SUM(looking_away_sec), 0) AS SIGNED),
                CAST(COALESCE(SUM(away_sec), 0) AS SIGNED), CAST(COALESCE(SUM(unknown_sec), 0) AS SIGNED)
         FROM daily_summaries WHERE work_date = ?",
    )
    .bind(today)
    .fetch_one(&st.db)
    .await?;

    let recent: Vec<(i64, String, String, Option<String>, Option<String>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT n.id, n.kind, n.title, n.body, CONCAT(e.first_name, ' ', e.last_name), n.created_at
         FROM notifications n LEFT JOIN employees e ON e.id = n.employee_id ORDER BY n.id DESC LIMIT 8",
    )
    .fetch_all(&st.db)
    .await?;

    let live = st.live.read().await;
    let mut live_counts = Map::new();
    for d in live.values() {
        let c = live_counts.entry(d.state.clone()).or_insert(json!(0));
        *c = json!(c.as_i64().unwrap_or(0) + 1);
    }

    Ok(Json(json!({
        "date": today,
        "cameras": { "total": cam_total, "online": cam_online, "ai_enabled": cam_ai },
        "employees": employees,
        "workstations": workstations,
        "scanned_today": scanned_today,
        "pending_corrections": pending_corrections,
        "wellness_alerts_today": wellness_today,
        "devices": { "total": devices_total, "seen_10min": devices_seen },
        "today_seconds": { "focused": totals.0, "present": totals.1, "phone": totals.2,
                           "looking_away": totals.3, "away": totals.4, "unknown": totals.5 },
        "live_counts": live_counts,
        "recent_notifications": recent.into_iter().map(|r| json!({
            "id": r.0, "kind": r.1, "title": r.2, "body": r.3, "employee_name": r.4, "created_at": r.5
        })).collect::<Vec<_>>(),
    })))
}

async fn live(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    u.need("live.view")?;
    let live = st.live.read().await;
    Ok(Json(json!(live.values().collect::<Vec<_>>())))
}

// ---------------------------------------------------------------- settings

async fn get_settings(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    u.need_any(&["settings.manage", "ai.manage", "privacy.review", "dashboard.view"])?;
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT setting_key, setting_value FROM system_settings").fetch_all(&st.db).await?;
    let mut m = Map::new();
    for (k, v) in rows {
        m.insert(k, serde_json::from_str(&v).unwrap_or(Value::Null));
    }
    Ok(Json(Value::Object(m)))
}

async fn put_settings(State(st): State<AppState>, u: AuthUser, Json(body): Json<Map<String, Value>>) -> AppResult<Json<Value>> {
    u.need("settings.manage")?;
    for (k, v) in &body {
        if !SETTING_KEYS.contains(&k.as_str()) {
            return Err(bad(format!("ไม่รู้จักการตั้งค่า: {k}")));
        }
        let numeric = matches!(
            k.as_str(),
            "retention_days" | "sitting_alert_min" | "min_state_sec" | "away_grace_sec" | "detect_confidence"
        );
        match k.as_str() {
            "report_show_activity" => {
                if !v.is_boolean() {
                    return Err(bad("report_show_activity ต้องเป็น true/false"));
                }
                continue;
            }
            "ticket_sla_hours" => {
                let ok = v.as_object().is_some_and(|o| {
                    ["low", "normal", "high", "urgent"]
                        .iter()
                        .all(|p| o.get(*p).and_then(|h| h.as_f64()).is_some_and(|h| (0.5..=2000.0).contains(&h)))
                });
                if !ok {
                    return Err(bad("SLA ต้องระบุชั่วโมงของ low, normal, high, urgent (0.5-2000)"));
                }
                continue;
            }
            "report_doc_prefix" => {
                let ok = v.as_str().is_some_and(|p| !p.is_empty() && p.len() <= 12 && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
                if !ok {
                    return Err(bad("คำนำหน้าเลขที่เอกสารใช้ได้เฉพาะ A-Z 0-9 และ - ไม่เกิน 12 ตัว"));
                }
                continue;
            }
            _ => {}
        }
        if numeric && !v.is_number() {
            return Err(bad(format!("{k} ต้องเป็นตัวเลข")));
        }
        if !numeric && !v.is_string() {
            return Err(bad(format!("{k} ต้องเป็นข้อความ")));
        }
    }
    for (k, v) in &body {
        sqlx::query(
            "INSERT INTO system_settings (setting_key, setting_value) VALUES (?, ?)
             ON DUPLICATE KEY UPDATE setting_value = VALUES(setting_value)",
        )
        .bind(k)
        .bind(v.to_string())
        .execute(&st.db)
        .await?;
    }
    audit(&st, Some(&u), "update", "settings", "-", Value::Object(body)).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- audit

#[derive(Deserialize)]
struct AuditQ {
    limit: Option<i64>,
    offset: Option<i64>,
    q: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
struct AuditRow {
    id: i64,
    user_id: Option<i64>,
    username: Option<String>,
    action: String,
    entity: Option<String>,
    entity_id: Option<String>,
    detail: Option<String>,
    created_at: DateTime<Utc>,
}

async fn audit_logs(State(st): State<AppState>, u: AuthUser, Query(q): Query<AuditQ>) -> AppResult<Json<Vec<AuditRow>>> {
    u.need("audit.view")?;
    let limit = q.limit.unwrap_or(200).clamp(1, 1000);
    let offset = q.offset.unwrap_or(0).max(0);
    let rows = match q.q.filter(|s| !s.is_empty()) {
        Some(term) => {
            let like = format!("%{term}%");
            sqlx::query_as(
                "SELECT id, user_id, username, action, entity, entity_id, detail, created_at FROM audit_logs
                 WHERE username LIKE ? OR action LIKE ? OR entity LIKE ? ORDER BY id DESC LIMIT ? OFFSET ?",
            )
            .bind(like.clone())
            .bind(like.clone())
            .bind(like)
            .bind(limit)
            .bind(offset)
            .fetch_all(&st.db)
            .await?
        }
        None => {
            sqlx::query_as(
                "SELECT id, user_id, username, action, entity, entity_id, detail, created_at FROM audit_logs
                 ORDER BY id DESC LIMIT ? OFFSET ?",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&st.db)
            .await?
        }
    };
    Ok(Json(rows))
}

// ---------------------------------------------------------------- resources & backup

async fn resources(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    u.need("system.view")?;
    let (proc_mem, proc_cpu, total_mem, used_mem, cpus) = {
        let mut sys = st.sys.lock().await;
        sys.refresh_memory();
        sys.refresh_cpu_usage();
        let pid = sysinfo::get_current_pid().ok();
        if let Some(pid) = pid {
            sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        }
        let p = pid.and_then(|p| sys.process(p));
        (
            p.map(|p| p.memory()).unwrap_or(0),
            p.map(|p| p.cpu_usage()).unwrap_or(0.0),
            sys.total_memory(),
            sys.used_memory(),
            sys.cpus().len(),
        )
    };
    let db_ok = sqlx::query("SELECT 1").execute(&st.db).await.is_ok();
    let tables: Vec<(String, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT TABLE_NAME, CAST(TABLE_ROWS AS SIGNED), CAST(DATA_LENGTH + INDEX_LENGTH AS SIGNED)
         FROM information_schema.TABLES WHERE TABLE_SCHEMA = DATABASE() ORDER BY DATA_LENGTH + INDEX_LENGTH DESC",
    )
    .fetch_all(&st.db)
    .await
    .unwrap_or_default();
    Ok(Json(json!({
        "uptime_sec": st.started.elapsed().as_secs(),
        "process_memory_bytes": proc_mem,
        "process_cpu_percent": proc_cpu,
        "system_memory_total_bytes": total_mem,
        "system_memory_used_bytes": used_mem,
        "cpu_count": cpus,
        "db_ok": db_ok,
        "db_pool_size": st.db.size(),
        "db_pool_idle": st.db.num_idle(),
        "websocket_clients": st.events.receiver_count(),
        "live_desks": st.live.read().await.len(),
        "tables": tables.into_iter().map(|(n, r, b)| json!({ "name": n, "rows": r, "bytes": b })).collect::<Vec<_>>(),
    })))
}

/// Exports configuration tables (not the large event history; use mysqldump for full backups).
async fn backup(State(st): State<AppState>, u: AuthUser) -> AppResult<Response> {
    u.need("backup.manage")?;
    let mut out = Map::new();
    for (name, sql) in [
        ("branches", "SELECT id, name, address FROM branches"),
        ("departments", "SELECT id, name, branch_id FROM departments"),
        (
            "employees",
            "SELECT id, employee_code, first_name, last_name, nickname, position, department_id, email, phone, attendance_ref,
               CAST(work_start AS CHAR) AS work_start, CAST(work_end AS CHAR) AS work_end, monitoring_consent, status FROM employees",
        ),
        (
            "cameras",
            "SELECT id, name, branch_id, floor, location, brand, stream_name, onvif_host, ai_enabled, ai_fps, ai_width, ptz_supported FROM cameras",
        ),
        ("workstations", "SELECT id, camera_id, name, employee_id, roi, screen_yaw_deg, yaw_tolerance_deg FROM workstations"),
        ("kpi_metrics", "SELECT id, code, name, description, source, weight, position, is_active FROM kpi_metrics"),
        ("system_settings", "SELECT setting_key, setting_value FROM system_settings"),
    ] {
        let rows = sqlx::query(sql).fetch_all(&st.db).await?;
        out.insert(name.into(), json!(rows.iter().map(row_to_json).collect::<Vec<_>>()));
    }
    out.insert("exported_at".into(), json!(Utc::now()));
    out.insert("note".into(), json!("Camera RTSP credentials are intentionally excluded."));
    audit(&st, Some(&u), "export_backup", "system", "-", json!({})).await;
    let body = serde_json::to_string_pretty(&Value::Object(out)).map_err(|e| AppError::Internal(e.into()))?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"smartoffice-config-{}.json\"", Utc::now().format("%Y%m%d-%H%M"))),
        ],
        body,
    )
        .into_response())
}

fn row_to_json(row: &sqlx::mysql::MySqlRow) -> Value {
    use sqlx::{Column, Row, TypeInfo, ValueRef};
    let mut m = Map::new();
    for (i, col) in row.columns().iter().enumerate() {
        let raw = row.try_get_raw(i).ok();
        let v = if raw.as_ref().map(|r| r.is_null()).unwrap_or(true) {
            Value::Null
        } else {
            let ty = col.type_info().name().to_ascii_uppercase();
            if ty.contains("INT") || ty == "BOOLEAN" {
                row.try_get::<i64, _>(i).map(Value::from).or_else(|_| row.try_get::<bool, _>(i).map(Value::from)).unwrap_or(Value::Null)
            } else if ty.contains("DOUBLE") || ty.contains("FLOAT") {
                row.try_get::<f64, _>(i).map(Value::from).unwrap_or(Value::Null)
            } else {
                row.try_get::<String, _>(i).map(Value::from).unwrap_or(Value::Null)
            }
        };
        m.insert(col.name().to_string(), v);
    }
    Value::Object(m)
}

// ---------------------------------------------------------------- websocket

#[derive(Deserialize)]
struct WsQ {
    token: String,
}

async fn ws_handler(State(st): State<AppState>, Query(q): Query<WsQ>, ws: WebSocketUpgrade) -> AppResult<Response> {
    let u = authenticate(&st, &q.token).await?;
    u.need_any(&["dashboard.view", "live.view"])?;
    Ok(ws.on_upgrade(move |socket| ws_loop(st, socket)))
}

async fn ws_loop(st: AppState, socket: WebSocket) {
    let (mut tx, mut rx) = socket.split();
    let mut sub = st.events.subscribe();
    let snapshot = {
        let live = st.live.read().await;
        json!({ "type": "live", "desks": live.values().collect::<Vec<_>>() }).to_string()
    };
    if tx.send(Message::Text(snapshot.into())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            msg = sub.recv() => match msg {
                Ok(m) => if tx.send(Message::Text(m.into())).await.is_err() { break },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            },
            incoming = rx.next() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            }
        }
    }
}
