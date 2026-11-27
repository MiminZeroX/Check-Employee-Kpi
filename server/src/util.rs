use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::MySqlPool;

use crate::error::{bad, AppResult};

pub const STATES: [&str; 6] = ["focused", "present", "phone", "looking_away", "away", "unknown"];

pub fn state_label(s: &str) -> &'static str {
    match s {
        "focused" => "มองหน้าจอ / ทำงานที่โต๊ะ",
        "present" => "อยู่ที่โต๊ะ (ไม่เห็นใบหน้า)",
        "phone" => "ใช้โทรศัพท์มือถือ",
        "looking_away" => "มองไปทางอื่น",
        "away" => "ไม่อยู่ที่โต๊ะ",
        _ => "ไม่สามารถระบุได้",
    }
}

pub fn is_valid_state(s: &str) -> bool {
    STATES.contains(&s)
}

/// States in which the person is physically at the desk.
pub fn is_at_desk(s: &str) -> bool {
    matches!(s, "focused" | "present" | "phone" | "looking_away")
}

pub fn local_today(off_min: i32) -> NaiveDate {
    (Utc::now() + Duration::minutes(off_min as i64)).date_naive()
}

pub fn to_local(dt: DateTime<Utc>, off_min: i32) -> NaiveDateTime {
    dt.naive_utc() + Duration::minutes(off_min as i64)
}

pub fn local_date_of(dt: DateTime<Utc>, off_min: i32) -> NaiveDate {
    to_local(dt, off_min).date()
}

pub fn from_local(naive: NaiveDateTime, off_min: i32) -> DateTime<Utc> {
    Utc.from_utc_datetime(&(naive - Duration::minutes(off_min as i64)))
}

/// UTC instant range [start, end) covering a local calendar day.
pub fn day_range(date: NaiveDate, off_min: i32) -> (DateTime<Utc>, DateTime<Utc>) {
    let start = from_local(date.and_hms_opt(0, 0, 0).unwrap(), off_min);
    (start, start + Duration::days(1))
}

/// Validates an inclusive local date range; defaults to today.
pub fn date_range(
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    off_min: i32,
) -> AppResult<(NaiveDate, NaiveDate)> {
    let today = local_today(off_min);
    let from = from.unwrap_or(today);
    let to = to.unwrap_or(from.max(today).min(from + Duration::days(30)));
    let to = if to < from { from } else { to };
    if (to - from).num_days() > 400 {
        return Err(bad("ช่วงวันที่ยาวเกินไป (สูงสุด 400 วัน)"));
    }
    Ok((from, to))
}

/// Parses a scanner timestamp. RFC3339 values carry their own offset; naive
/// values are taken as office local time.
pub fn parse_scan_time(s: &str, off_min: i32) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y/%m/%d %H:%M:%S"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(from_local(n, off_min));
        }
    }
    None
}

pub fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

pub fn random_token(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

pub fn fmt_duration(sec: i64) -> String {
    let h = sec / 3600;
    let m = (sec % 3600) / 60;
    if h > 0 {
        format!("{h} ชม. {m} นาที")
    } else {
        format!("{m} นาที")
    }
}

pub fn html_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            _ => o.push(c),
        }
    }
    o
}

pub fn csv_field(s: &str) -> String {
    // a leading = + - @ (or tab/CR) would run as a formula when the file is opened in Excel
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{s}") } else { s.to_string() };
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

/// Reads a JSON-encoded system setting.
pub async fn setting(db: &MySqlPool, key: &str) -> Option<serde_json::Value> {
    let v: Option<(String,)> =
        sqlx::query_as("SELECT setting_value FROM system_settings WHERE setting_key = ?")
            .bind(key)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    v.and_then(|(s,)| serde_json::from_str(&s).ok())
}

pub async fn setting_str(db: &MySqlPool, key: &str, default: &str) -> String {
    match setting(db, key).await {
        Some(serde_json::Value::String(s)) => s,
        Some(v) if !v.is_null() => v.to_string(),
        _ => default.to_string(),
    }
}

pub async fn setting_f64(db: &MySqlPool, key: &str, default: f64) -> f64 {
    setting(db, key).await.and_then(|v| v.as_f64()).unwrap_or(default)
}

/// Serialises a column that holds JSON text as embedded JSON instead of a string.
pub fn ser_json_str<S: serde::Serializer>(v: &str, s: S) -> Result<S::Ok, S::Error> {
    use serde::Serialize;
    serde_json::from_str::<serde_json::Value>(v)
        .unwrap_or(serde_json::Value::Null)
        .serialize(s)
}
