//! Video management extras: detection zones (people counting), heatmaps, ONVIF PTZ,
//! ONVIF WS-Discovery, NVR playback through go2rtc, and privacy masks for snapshots.

use std::{collections::HashMap, time::Duration as StdDuration};

use axum::{
    extract::{Path, Query, State},
    routing::{get, post, put},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Timelike, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};

use crate::{
    auth::{audit, AuthUser},
    error::{bad, AppError, AppResult},
    routes::{cameras::sync_stream, Range},
    state::AppState,
    util::{date_range, day_range, is_at_desk, parse_scan_time, random_token, to_local},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/zones", get(list_zones).post(create_zone))
        .route("/zones/{id}", put(update_zone).delete(delete_zone))
        .route("/zones/occupancy", get(zone_occupancy))
        .route("/analytics/heatmap", get(heatmap))
        .route("/cameras/{id}/ptz", post(ptz))
        .route("/cameras/{id}/ptz/presets", get(ptz_presets))
        .route("/cameras/discover", post(discover))
        .route("/cameras/{id}/playback", post(playback))
}

// ---------------------------------------------------------------- zones

#[derive(Serialize, sqlx::FromRow)]
struct ZoneRow {
    id: i64,
    camera_id: i64,
    camera_name: String,
    name: String,
    kind: String,
    #[serde(serialize_with = "crate::util::ser_json_str")]
    roi: String,
    capacity: Option<i32>,
}

const ZONE_KINDS: [&str; 4] = ["area", "entrance", "meeting_room", "restricted"];

async fn list_zones(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Vec<ZoneRow>>> {
    u.need_any(&["live.view", "ai.manage"])?;
    Ok(Json(
        sqlx::query_as(
            "SELECT z.id, z.camera_id, c.name AS camera_name, z.name, z.kind, z.roi, z.capacity
             FROM detection_zones z JOIN cameras c ON c.id = z.camera_id ORDER BY c.name, z.name",
        )
        .fetch_all(&st.db)
        .await?,
    ))
}

#[derive(Deserialize)]
struct ZoneReq {
    camera_id: i64,
    name: String,
    kind: String,
    roi: Option<Vec<[f64; 2]>>,
    capacity: Option<i32>,
}

fn zone_roi(r: &ZoneReq) -> AppResult<String> {
    if r.name.trim().is_empty() || !ZONE_KINDS.contains(&r.kind.as_str()) {
        return Err(bad("กรุณาระบุชื่อและประเภทพื้นที่"));
    }
    let roi = r.roi.clone().filter(|p| !p.is_empty()).unwrap_or_else(|| vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
    if roi.len() < 3 || roi.len() > 32 || roi.iter().any(|p| !(0.0..=1.0).contains(&p[0]) || !(0.0..=1.0).contains(&p[1])) {
        return Err(bad("พื้นที่ต้องมี 3-32 จุด และพิกัดอยู่ระหว่าง 0-1"));
    }
    Ok(serde_json::to_string(&roi).unwrap())
}

async fn create_zone(State(st): State<AppState>, u: AuthUser, Json(r): Json<ZoneReq>) -> AppResult<Json<Value>> {
    u.need("ai.manage")?;
    let roi = zone_roi(&r)?;
    let id = sqlx::query("INSERT INTO detection_zones (camera_id, name, kind, roi, capacity) VALUES (?,?,?,?,?)")
        .bind(r.camera_id)
        .bind(r.name.trim())
        .bind(&r.kind)
        .bind(roi)
        .bind(r.capacity)
        .execute(&st.db)
        .await?
        .last_insert_id();
    audit(&st, Some(&u), "create", "zone", id, json!({ "name": r.name, "kind": r.kind })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_zone(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<ZoneReq>) -> AppResult<Json<Value>> {
    u.need("ai.manage")?;
    let roi = zone_roi(&r)?;
    sqlx::query("UPDATE detection_zones SET camera_id=?, name=?, kind=?, roi=?, capacity=? WHERE id=?")
        .bind(r.camera_id)
        .bind(r.name.trim())
        .bind(&r.kind)
        .bind(roi)
        .bind(r.capacity)
        .bind(id)
        .execute(&st.db)
        .await?;
    audit(&st, Some(&u), "update", "zone", id, json!({ "name": r.name })).await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_zone(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("ai.manage")?;
    sqlx::query("DELETE FROM detection_zones WHERE id = ?").bind(id).execute(&st.db).await?;
    st.zone_live.write().await.remove(&id);
    Ok(Json(json!({ "ok": true })))
}

/// Per zone: average and peak people by hour of day, entries, utilisation of capacity, live count.
async fn zone_occupancy(State(st): State<AppState>, u: AuthUser, Query(r): Query<Range>) -> AppResult<Json<Value>> {
    u.need_any(&["live.view", "analytics.view"])?;
    let off = st.cfg.tz_offset_min;
    let (from, to) = date_range(r.from, r.to, off)?;
    let (start, _) = day_range(from, off);
    let (_, end) = day_range(to, off);
    let zones: Vec<(i64, String, String, Option<i32>, String)> = sqlx::query_as(
        "SELECT z.id, z.name, z.kind, z.capacity, c.name FROM detection_zones z JOIN cameras c ON c.id = z.camera_id ORDER BY z.name",
    )
    .fetch_all(&st.db)
    .await?;
    let rows: Vec<(i64, DateTime<Utc>, i32, f64, i32, i32)> = sqlx::query_as(
        "SELECT zone_id, bucket_start, max_people, people_sum, samples, entries FROM zone_occupancy WHERE bucket_start >= ? AND bucket_start < ?",
    )
    .bind(start)
    .bind(end)
    .fetch_all(&st.db)
    .await?;
    let live = st.zone_live.read().await;
    let out: Vec<Value> = zones
        .into_iter()
        .map(|(id, name, kind, cap, cam)| {
            let mut hours = vec![(0.0f64, 0i32, 0i32); 24]; // people_sum, samples, peak
            let mut entries = 0;
            for r in rows.iter().filter(|r| r.0 == id) {
                let h = to_local(r.1, off).hour() as usize;
                hours[h].0 += r.3;
                hours[h].1 += r.4;
                hours[h].2 = hours[h].2.max(r.2);
                entries += r.5;
            }
            let avg: Vec<Option<f64>> = hours.iter().map(|h| (h.1 > 0).then(|| (h.0 / h.1 as f64 * 10.0).round() / 10.0)).collect();
            let peak = hours.iter().map(|h| h.2).max().unwrap_or(0);
            json!({ "zone_id": id, "name": name, "kind": kind, "capacity": cap, "camera": cam, "avg_by_hour": avg,
                    "peak": peak, "entries": entries, "people_now": live.get(&id).filter(|l| (Utc::now() - l.1).num_seconds() < 120).map(|l| l.0) })
        })
        .collect();
    Ok(Json(json!({ "from": from, "to": to, "zones": out })))
}

// ---------------------------------------------------------------- activity heatmap

/// Weekday x hour: average number of people at their desks and the share looking at the screen.
async fn heatmap(State(st): State<AppState>, u: AuthUser, Query(r): Query<Range>) -> AppResult<Json<Value>> {
    u.need("analytics.view")?;
    let off = st.cfg.tz_offset_min;
    let (from, to) = date_range(r.from, r.to, off)?;
    if (to - from).num_days() > 92 {
        return Err(bad("Heatmap รองรับช่วงไม่เกิน 92 วัน"));
    }
    let (start, _) = day_range(from, off);
    let (_, end) = day_range(to, off);
    let mut sql = String::from(
        "SELECT COALESCE(a.corrected_state, a.state), a.started_at, a.ended_at FROM activity_events a
         JOIN employees e ON e.id = a.employee_id LEFT JOIN departments d ON d.id = e.department_id
         WHERE a.review_status <> 'rejected' AND a.started_at < ? AND a.ended_at > ?",
    );
    if r.department_id.is_some() {
        sql.push_str(" AND e.department_id = ?");
    }
    if r.branch_id.is_some() {
        sql.push_str(" AND d.branch_id = ?");
    }
    let mut q = sqlx::query_as::<_, (String, DateTime<Utc>, DateTime<Utc>)>(&sql).bind(end).bind(start);
    for v in [r.department_id, r.branch_id].into_iter().flatten() {
        q = q.bind(v);
    }
    let events = q.fetch_all(&st.db).await?;
    // seconds per (weekday 0..6, hour 0..23)
    let mut desk = vec![[0f64; 24]; 7];
    let mut focused = vec![[0f64; 24]; 7];
    for (state, s, e) in events {
        if !is_at_desk(&state) {
            continue;
        }
        let mut t = s.max(start);
        let e = e.min(end);
        while t < e {
            let local = to_local(t, off);
            let next_hour = t + Duration::seconds(3600 - (local.minute() * 60 + local.second()) as i64);
            let chunk_end = next_hour.min(e);
            let secs = (chunk_end - t).num_seconds() as f64;
            let (wd, h) = (local.weekday().num_days_from_monday() as usize, local.hour() as usize);
            desk[wd][h] += secs;
            if state == "focused" {
                focused[wd][h] += secs;
            }
            t = chunk_end;
        }
    }
    // number of each weekday in the range, to turn totals into an average day
    let mut weekdays = [0f64; 7];
    let mut d = from;
    while d <= to {
        weekdays[d.weekday().num_days_from_monday() as usize] += 1.0;
        d += Duration::days(1);
    }
    let cells: Vec<Vec<Value>> = (0..7)
        .map(|w| {
            (0..24)
                .map(|h| {
                    let n = weekdays[w].max(1.0);
                    let people = desk[w][h] / 3600.0 / n;
                    json!({ "people": (people * 10.0).round() / 10.0,
                            "focus_pct": if desk[w][h] > 0.0 { Some((focused[w][h] * 100.0 / desk[w][h]).round()) } else { None } })
                })
                .collect()
        })
        .collect();
    Ok(Json(json!({ "from": from, "to": to, "cells": cells })))
}

// ---------------------------------------------------------------- ONVIF

fn soap(user: &str, pass: &str, body: &str) -> String {
    let mut nonce = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut nonce);
    let created = Utc::now().format("%Y-%m-%dT%H:%M:%S.000Z").to_string();
    let mut h = Sha1::new();
    h.update(nonce);
    h.update(created.as_bytes());
    h.update(pass.as_bytes());
    let digest = B64.encode(h.finalize());
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Header><Security s:mustUnderstand="1" xmlns="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd"><UsernameToken><Username>{}</Username><Password Type="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-username-token-profile-1.0#PasswordDigest">{digest}</Password><Nonce EncodingType="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-soap-message-security-1.0#Base64Binary">{}</Nonce><Created xmlns="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd">{created}</Created></UsernameToken></Security></s:Header><s:Body>{body}</s:Body></s:Envelope>"#,
        esc(user),
        B64.encode(nonce)
    )
}

async fn soap_call(st: &AppState, url: &str, user: &str, pass: &str, body: &str) -> AppResult<String> {
    let resp = st
        .http
        .post(url)
        .header("content-type", "application/soap+xml; charset=utf-8")
        .body(soap(user, pass, body))
        .send()
        .await
        .map_err(|e| bad(format!("ติดต่อกล้องผ่าน ONVIF ไม่ได้: {e}")))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let reason = regex_lite::Regex::new(r"<[^>]*Text[^>]*>([^<]+)<").ok().and_then(|re| re.captures(&text)).map(|c| c[1].to_string());
        return Err(bad(format!("กล้องตอบกลับ HTTP {status}{}", reason.map(|r| format!(": {r}")).unwrap_or_default())));
    }
    Ok(text)
}

fn capture(text: &str, pattern: &str) -> Option<String> {
    regex_lite::Regex::new(pattern).ok()?.captures(text).map(|c| c[1].trim().to_string())
}

struct Onvif {
    ptz_url: String,
    profile: String,
    user: String,
    pass: String,
}

/// Finds the PTZ service and first media profile (cached per camera).
async fn onvif(st: &AppState, cam: i64) -> AppResult<Onvif> {
    let (host, user, pass, ptz): (Option<String>, Option<String>, Option<String>, bool) =
        sqlx::query_as("SELECT onvif_host, onvif_user, onvif_password, ptz_supported FROM cameras WHERE id = ?")
            .bind(cam)
            .fetch_one(&st.db)
            .await?;
    if !ptz {
        return Err(bad("กล้องนี้ไม่ได้ตั้งค่าว่ารองรับ PTZ"));
    }
    let host = host.filter(|h| !h.trim().is_empty()).ok_or_else(|| bad("กรุณาระบุ ONVIF host ของกล้อง"))?;
    let user = user.unwrap_or_default();
    let pass = st.crypto.decrypt(&pass.unwrap_or_default()).unwrap_or_default();
    let base = if host.starts_with("http") { host.trim_end_matches('/').to_string() } else { format!("http://{}", host.trim_end_matches('/')) };
    let device = if base.contains("/onvif/") { base.clone() } else { format!("{base}/onvif/device_service") };
    let caps = soap_call(st, &device, &user, &pass, r#"<GetCapabilities xmlns="http://www.onvif.org/ver10/device/wsdl"><Category>All</Category></GetCapabilities>"#).await?;
    let media = capture(&caps, r"(?s)Media>\s*<[^>]*XAddr>([^<]+)<").ok_or_else(|| bad("กล้องไม่มีบริการ ONVIF Media"))?;
    let ptz_url = capture(&caps, r"(?s)PTZ>\s*<[^>]*XAddr>([^<]+)<").ok_or_else(|| bad("กล้องไม่มีบริการ ONVIF PTZ"))?;
    let profiles = soap_call(st, &media, &user, &pass, r#"<GetProfiles xmlns="http://www.onvif.org/ver10/media/wsdl"/>"#).await?;
    let profile = capture(&profiles, r#"Profiles[^>]*token="([^"]+)""#).ok_or_else(|| bad("ไม่พบ media profile ของกล้อง"))?;
    Ok(Onvif { ptz_url, profile, user, pass })
}

#[derive(Deserialize)]
struct PtzReq {
    /// move | stop | preset
    action: String,
    pan: Option<f64>,
    tilt: Option<f64>,
    zoom: Option<f64>,
    preset: Option<String>,
}

async fn ptz(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<PtzReq>) -> AppResult<Json<Value>> {
    u.need("cameras.manage")?;
    let o = onvif(&st, id).await?;
    let ns = r#"xmlns="http://www.onvif.org/ver20/ptz/wsdl""#;
    let body = match r.action.as_str() {
        "move" => {
            let c = |v: Option<f64>| v.unwrap_or(0.0).clamp(-1.0, 1.0);
            format!(
                r#"<ContinuousMove {ns}><ProfileToken>{}</ProfileToken><Velocity><PanTilt x="{}" y="{}" xmlns="http://www.onvif.org/ver10/schema"/><Zoom x="{}" xmlns="http://www.onvif.org/ver10/schema"/></Velocity></ContinuousMove>"#,
                o.profile,
                c(r.pan),
                c(r.tilt),
                c(r.zoom)
            )
        }
        "stop" => format!(r#"<Stop {ns}><ProfileToken>{}</ProfileToken><PanTilt>true</PanTilt><Zoom>true</Zoom></Stop>"#, o.profile),
        "preset" => {
            let p = r.preset.as_deref().filter(|p| p.len() <= 64 && p.chars().all(|c| c.is_ascii_alphanumeric() || "_-".contains(c)));
            let p = p.ok_or_else(|| bad("ตำแหน่ง preset ไม่ถูกต้อง"))?;
            format!(r#"<GotoPreset {ns}><ProfileToken>{}</ProfileToken><PresetToken>{p}</PresetToken></GotoPreset>"#, o.profile)
        }
        _ => return Err(bad("คำสั่ง PTZ ไม่ถูกต้อง")),
    };
    soap_call(&st, &o.ptz_url, &o.user, &o.pass, &body).await?;
    if r.action != "stop" {
        audit(&st, Some(&u), "ptz", "camera", id, json!({ "action": r.action, "preset": r.preset })).await;
    }
    Ok(Json(json!({ "ok": true })))
}

async fn ptz_presets(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need("cameras.manage")?;
    let o = onvif(&st, id).await?;
    let text = soap_call(
        &st,
        &o.ptz_url,
        &o.user,
        &o.pass,
        &format!(r#"<GetPresets xmlns="http://www.onvif.org/ver20/ptz/wsdl"><ProfileToken>{}</ProfileToken></GetPresets>"#, o.profile),
    )
    .await?;
    let re = regex_lite::Regex::new(r#"(?s)Preset[^>]*token="([^"]+)"[^>]*>.*?<[^>]*Name>([^<]*)<"#).map_err(|e| AppError::Internal(e.into()))?;
    let presets: Vec<Value> = re.captures_iter(&text).take(256).map(|c| json!({ "token": &c[1], "name": &c[2] })).collect();
    Ok(Json(json!(presets)))
}

/// ONVIF WS-Discovery: multicast probe on the server's network for 3 seconds.
/// (Under Docker this needs host networking to reach the camera VLAN.)
async fn discover(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    u.need("cameras.manage")?;
    let sock = tokio::net::UdpSocket::bind("0.0.0.0:0").await.map_err(|e| AppError::Internal(e.into()))?;
    let probe = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><e:Envelope xmlns:e="http://www.w3.org/2003/05/soap-envelope" xmlns:w="http://schemas.xmlsoap.org/ws/2004/08/addressing" xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery" xmlns:dn="http://www.onvif.org/ver10/network/wsdl"><e:Header><w:MessageID>uuid:{}</w:MessageID><w:To e:mustUnderstand="true">urn:schemas-xmlsoap-org:ws:2005:04:discovery</w:To><w:Action e:mustUnderstand="true">http://schemas.xmlsoap.org/ws/2005/04/discovery/Probe</w:Action></e:Header><e:Body><d:Probe><d:Types>dn:NetworkVideoTransmitter</d:Types></d:Probe></e:Body></e:Envelope>"#,
        random_token(16)
    );
    let _ = sock.set_multicast_ttl_v4(2);
    sock.send_to(probe.as_bytes(), "239.255.255.250:3702").await.map_err(|e| bad(format!("ส่ง probe ไม่ได้: {e}")))?;
    let mut found: HashMap<String, Value> = HashMap::new();
    let deadline = tokio::time::Instant::now() + StdDuration::from_secs(3);
    let mut buf = vec![0u8; 65535];
    while let Ok(Ok((n, from))) = tokio::time::timeout_at(deadline, sock.recv_from(&mut buf)).await {
        let text = String::from_utf8_lossy(&buf[..n]).to_string();
        let Some(xaddr) = capture(&text, r"<[^>]*XAddrs>([^<]+)<") else { continue };
        let xaddr = xaddr.split_whitespace().next().unwrap_or_default().to_string();
        let scopes = capture(&text, r"<[^>]*Scopes[^>]*>([^<]+)<").unwrap_or_default();
        let scope = |k: &str| {
            scopes
                .split_whitespace()
                .find_map(|s| s.strip_prefix(&format!("onvif://www.onvif.org/{k}/")))
                .map(|v| v.replace("%20", " ").replace('_', " "))
        };
        let host = xaddr.split("//").nth(1).and_then(|r| r.split('/').next()).unwrap_or_default().to_string();
        found.insert(
            xaddr.clone(),
            json!({ "xaddr": xaddr, "onvif_host": host, "ip": from.ip().to_string(), "name": scope("name"), "hardware": scope("hardware") }),
        );
    }
    audit(&st, Some(&u), "discover", "camera", "-", json!({ "found": found.len() })).await;
    Ok(Json(json!(found.into_values().collect::<Vec<_>>())))
}

// ---------------------------------------------------------------- playback

#[derive(Deserialize)]
struct PlaybackReq {
    start: String,
    minutes: Option<i64>,
}

/// Registers a temporary go2rtc stream for the camera's NVR recording.
/// The camera's playback URL is a template, e.g. Hikvision:
/// rtsp://user:pass@NVR:554/Streaming/tracks/101?starttime={start}&endtime={end}
async fn playback(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<PlaybackReq>) -> AppResult<Json<Value>> {
    u.need("playback.view")?;
    let off = st.cfg.tz_offset_min;
    let (tpl,): (Option<String>,) = sqlx::query_as("SELECT playback_url FROM cameras WHERE id = ?").bind(id).fetch_one(&st.db).await?;
    let tpl = tpl.filter(|t| !t.trim().is_empty()).ok_or_else(|| bad("กล้องนี้ยังไม่ได้ตั้งค่า URL ภาพย้อนหลังจาก NVR"))?;
    let tpl = st.crypto.decrypt(&tpl).map_err(AppError::Internal)?;
    let start = parse_scan_time(&r.start, off).ok_or_else(|| bad("รูปแบบเวลาไม่ถูกต้อง"))?;
    if start > Utc::now() {
        return Err(bad("เลือกเวลาในอดีต"));
    }
    let end = start + Duration::minutes(r.minutes.unwrap_or(30).clamp(1, 240));
    let fmt = |t: DateTime<Utc>| t.format("%Y%m%dT%H%M%SZ").to_string();
    let local = |t: DateTime<Utc>| to_local(t, off).format("%Y%m%dT%H%M%S").to_string();
    let url = tpl
        .replace("{start}", &fmt(start))
        .replace("{end}", &fmt(end))
        .replace("{start_local}", &local(start))
        .replace("{end_local}", &local(end));
    let name = format!("pb_{}", random_token(6));
    sync_stream(&st, &name, &url).await;
    st.playback_streams.lock().await.insert(name.clone(), Utc::now());
    audit(&st, Some(&u), "playback", "camera", id, json!({ "start": start, "end": end })).await;
    Ok(Json(json!({ "stream": name, "start": start, "end": end })))
}

/// Removes playback streams older than two hours (called by the background job).
pub async fn cleanup_playback(st: &AppState) {
    let old: Vec<String> = {
        let mut m = st.playback_streams.lock().await;
        let old: Vec<String> = m.iter().filter(|(_, t)| (Utc::now() - **t).num_hours() >= 2).map(|(k, _)| k.clone()).collect();
        for k in &old {
            m.remove(k);
        }
        old
    };
    let Some(api) = &st.cfg.go2rtc_api else { return };
    for name in old {
        let _ = st.http.delete(format!("{}/api/streams", api.trim_end_matches('/'))).query(&[("src", &name)]).send().await;
    }
}

// ---------------------------------------------------------------- privacy masks

/// Paints the camera's privacy-mask polygons black on a JPEG snapshot (server side,
/// so masked areas never leave the server in snapshots).
pub fn mask_jpeg(data: &[u8], masks: &[Vec<[f64; 2]>]) -> Option<Vec<u8>> {
    let img = image::load_from_memory_with_format(data, image::ImageFormat::Jpeg).ok()?;
    let mut rgb = img.to_rgb8();
    let (w, h) = (rgb.width() as f64, rgb.height() as f64);
    for poly in masks.iter().filter(|p| p.len() >= 3) {
        let pts: Vec<(f64, f64)> = poly.iter().map(|p| (p[0] * w, p[1] * h)).collect();
        let (ymin, ymax) = pts.iter().fold((h, 0.0f64), |(a, b), p| (a.min(p.1), b.max(p.1)));
        for y in (ymin.max(0.0) as u32)..(ymax.min(h - 1.0) as u32 + 1) {
            let yc = y as f64 + 0.5;
            let mut xs: Vec<f64> = Vec::new();
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                if (a.1 <= yc) != (b.1 <= yc) {
                    xs.push(a.0 + (yc - a.1) / (b.1 - a.1) * (b.0 - a.0));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            for pair in xs.chunks(2) {
                if let [x0, x1] = pair {
                    for x in (x0.max(0.0) as u32)..(x1.min(w) as u32) {
                        rgb.put_pixel(x, y, image::Rgb([0, 0, 0]));
                    }
                }
            }
        }
    }
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(rgb).write_to(&mut out, image::ImageFormat::Jpeg).ok()?;
    Some(out.into_inner())
}

#[allow(dead_code)]
fn _unused(_: NaiveDate) {}
