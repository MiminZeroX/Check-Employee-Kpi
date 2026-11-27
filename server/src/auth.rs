use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{extract::FromRequestParts, http::request::Parts};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::{
    error::{AppError, AppResult},
    state::AppState,
    util::sha256_hex,
};

/// Every permission the system checks, with a Thai label for the role editor.
/// `admin` always has all of them; other roles get theirs from `role_permissions`.
pub const PERMISSIONS: [(&str, &str); 49] = [
    ("dashboard.view", "ดูแดชบอร์ดภาพรวม"),
    ("live.view", "ดูภาพสด / สถานะโต๊ะ"),
    ("playback.view", "ดูภาพย้อนหลัง"),
    ("cameras.manage", "จัดการกล้อง / PTZ"),
    ("ai.manage", "จัดการโต๊ะ / พื้นที่ตรวจจับ AI"),
    ("devices.view", "ดูอุปกรณ์"),
    ("devices.manage", "จัดการอุปกรณ์"),
    ("employees.view", "ดูข้อมูลพนักงาน"),
    ("employees.manage", "จัดการข้อมูลพนักงาน"),
    ("attendance.view", "ดูเวลาเข้างาน"),
    ("attendance.manage", "บันทึกเวลาแทน / จับคู่รหัส"),
    ("org.manage", "จัดการแผนก / สาขา / ผังองค์กร"),
    ("hr.manage", "จัดการกะ / วันหยุด / ประเภทการลา"),
    ("requests.approve", "อนุมัติคำขอลา / OT / นอกสถานที่"),
    ("analytics.view", "ดูการวิเคราะห์กิจกรรม"),
    ("tasks.view", "ดูงาน"),
    ("tasks.manage", "มอบหมาย / แก้ไขงาน"),
    ("projects.view", "ดูโครงการ"),
    ("projects.manage", "จัดการโครงการ"),
    ("crm.view", "ดูลูกค้า / ดีล"),
    ("crm.manage", "จัดการลูกค้า / ผู้ติดต่อ / ดีล"),
    ("tickets.manage", "จัดการ Ticket บริการ"),
    ("kpi.view", "ดูคะแนน KPI"),
    ("kpi.manage", "ตั้งค่าตัวชี้วัด KPI"),
    ("kpi.review", "ประเมิน KPI"),
    ("wellness.view", "ดูข้อมูลสุขภาพพนักงาน"),
    ("reports.view", "ออกรายงาน / ส่งออกข้อมูล"),
    ("notifications.view", "ดูการแจ้งเตือน"),
    ("documents.view", "ดูเอกสาร"),
    ("documents.manage", "อัปโหลด / ลบเอกสาร"),
    ("privacy.review", "พิจารณาคำขอแก้ไขข้อมูล"),
    ("users.manage", "จัดการผู้ใช้และบทบาท"),
    ("audit.view", "ดู Audit Log"),
    ("settings.manage", "ตั้งค่าระบบ"),
    ("system.view", "ดูทรัพยากรเซิร์ฟเวอร์"),
    ("backup.manage", "สำรอง / คำนวณข้อมูลใหม่"),
    ("webhooks.manage", "จัดการ Webhook / การเชื่อมต่อ"),
    ("payroll.view", "ดูเงินเดือน / สลิปของทุกคน"),
    ("payroll.manage", "คำนวณ / อนุมัติ / จ่ายเงินเดือน"),
    ("expenses.approve", "อนุมัติใบเบิกค่าใช้จ่าย"),
    ("expenses.manage", "จ่ายเงินใบเบิก / ตั้งค่าหมวดค่าใช้จ่าย"),
    ("sales.view", "ดูเอกสารขาย"),
    ("sales.manage", "ออกใบเสนอราคา / ใบแจ้งหนี้ / ใบเสร็จ"),
    ("purchasing.view", "ดูใบสั่งซื้อ / ผู้ขาย"),
    ("purchasing.manage", "สั่งซื้อ / รับสินค้า / จ่ายเจ้าหนี้"),
    ("inventory.manage", "จัดการสินค้าและคลัง / ตรวจนับ"),
    ("accounting.view", "ดูบัญชีและงบการเงิน"),
    ("accounting.manage", "บันทึกบัญชี / ผังบัญชี / ปิดงวด"),
    ("assets.manage", "จัดการทรัพย์สิน / ค่าเสื่อมราคา"),
];

pub fn is_permission(p: &str) -> bool {
    PERMISSIONS.iter().any(|(k, _)| *k == p)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: i64,
    pub name: String,
    pub role: String,
    pub emp: Option<i64>,
    pub exp: i64,
    /// must match users.token_version; bumping it signs the user out everywhere
    #[serde(default)]
    pub tv: i64,
}

/// An authenticated user (JWT bearer token) with the permissions of their current role.
#[derive(Debug, Clone)]
pub struct AuthUser(pub Claims, pub Arc<HashSet<String>>);

impl AuthUser {
    pub fn id(&self) -> i64 {
        self.0.sub
    }
    pub fn role(&self) -> &str {
        &self.0.role
    }
    pub fn name(&self) -> &str {
        &self.0.name
    }
    pub fn is_admin(&self) -> bool {
        self.0.role == "admin"
    }
    pub fn can(&self, perm: &str) -> bool {
        self.is_admin() || self.1.contains(perm)
    }
    pub fn need(&self, perm: &str) -> AppResult<()> {
        if self.can(perm) {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }
    pub fn need_any(&self, perms: &[&str]) -> AppResult<()> {
        if perms.iter().any(|p| self.can(p)) {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }
    /// Permission list for the UI.
    pub fn permissions(&self) -> Vec<String> {
        if self.is_admin() {
            PERMISSIONS.iter().map(|(k, _)| k.to_string()).collect()
        } else {
            let mut v: Vec<String> = self.1.iter().cloned().collect();
            v.sort();
            v
        }
    }
}

pub fn issue_token(state: &AppState, c: Claims) -> AppResult<String> {
    encode(&Header::default(), &c, &EncodingKey::from_secret(state.cfg.jwt_secret.as_bytes()))
        .map_err(|e| AppError::Internal(e.into()))
}

pub fn new_claims(state: &AppState, id: i64, name: String, role: String, emp: Option<i64>, tv: i64) -> Claims {
    Claims {
        sub: id,
        name,
        role,
        emp,
        exp: (Utc::now() + Duration::hours(state.cfg.jwt_ttl_hours)).timestamp(),
        tv,
    }
}

pub fn verify_token(state: &AppState, token: &str) -> AppResult<Claims> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(state.cfg.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map(|d| d.claims)
    .map_err(|_| AppError::Unauthorized)
}

/// Loads `role -> permissions` into memory (called at start-up and after role edits).
pub async fn load_permissions(state: &AppState) -> anyhow::Result<()> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT role, permission FROM role_permissions").fetch_all(&state.db).await?;
    let roles: Vec<(String,)> = sqlx::query_as("SELECT name FROM roles").fetch_all(&state.db).await?;
    let mut map: HashMap<String, HashSet<String>> = roles.into_iter().map(|(r,)| (r, HashSet::new())).collect();
    for (r, p) in rows {
        map.entry(r).or_default().insert(p);
    }
    *state.role_perms.write().await = map.into_iter().map(|(k, v)| (k, Arc::new(v))).collect();
    Ok(())
}

/// Validates a token against the database: account active, token not revoked.
/// The role comes from the database, so role changes apply immediately.
pub async fn authenticate(state: &AppState, token: &str) -> AppResult<AuthUser> {
    let mut claims = verify_token(state, token)?;
    let row: Option<(bool, String, i64, Option<i64>)> =
        sqlx::query_as("SELECT is_active, role, token_version, employee_id FROM users WHERE id = ?")
            .bind(claims.sub)
            .fetch_optional(&state.db)
            .await?;
    match row {
        Some((true, role, tv, emp)) if tv == claims.tv => {
            claims.role = role;
            claims.emp = emp;
            let perms = state.role_perms.read().await.get(&claims.role).cloned().unwrap_or_default();
            Ok(AuthUser(claims, perms))
        }
        _ => Err(AppError::Unauthorized),
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .ok_or(AppError::Unauthorized)?;
        let token = header.strip_prefix("Bearer ").ok_or(AppError::Unauthorized)?;
        authenticate(state, token).await
    }
}

/// A machine client (AI worker or attendance terminal) authenticated by `X-Device-Key`
/// header or `key` query parameter.
#[derive(Debug, Clone)]
pub struct DeviceAuth {
    pub id: i64,
    pub kind: String,
}

impl FromRequestParts<AppState> for DeviceAuth {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let key = parts
            .headers
            .get("x-device-key")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string())
            .or_else(|| {
                parts.uri.query().and_then(|q| {
                    q.split('&')
                        .find_map(|kv| kv.strip_prefix("key=").map(|s| s.to_string()))
                })
            })
            .ok_or(AppError::Unauthorized)?;
        let row: Option<(i64, String, bool)> =
            sqlx::query_as("SELECT id, kind, is_enabled FROM devices WHERE api_key_hash = ?")
                .bind(sha256_hex(&key))
                .fetch_optional(&state.db)
                .await?;
        match row {
            Some((id, kind, true)) => {
                let _ = sqlx::query("UPDATE devices SET last_seen_at = UTC_TIMESTAMP() WHERE id = ?")
                    .bind(id)
                    .execute(&state.db)
                    .await;
                Ok(DeviceAuth { id, kind })
            }
            _ => Err(AppError::Unauthorized),
        }
    }
}

pub fn hash_password(pw: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::Internal(anyhow::anyhow!("hash: {e}")))
}

pub fn verify_password(pw: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|h| Argon2::default().verify_password(pw.as_bytes(), &h).is_ok())
        .unwrap_or(false)
}

pub fn validate_password(pw: &str) -> AppResult<()> {
    if pw.chars().count() < 8 {
        return Err(AppError::BadRequest("รหัสผ่านต้องมีอย่างน้อย 8 ตัวอักษร".into()));
    }
    Ok(())
}

/// Writes an audit log entry; failures are logged and ignored.
pub async fn audit(
    state: &AppState,
    user: Option<&AuthUser>,
    action: &str,
    entity: &str,
    entity_id: impl ToString,
    detail: serde_json::Value,
) {
    let r = sqlx::query(
        "INSERT INTO audit_logs (user_id, username, action, entity, entity_id, detail) VALUES (?,?,?,?,?,?)",
    )
    .bind(user.map(|u| u.id()))
    .bind(user.map(|u| u.name().to_string()))
    .bind(action)
    .bind(entity)
    .bind(entity_id.to_string())
    .bind(detail.to_string())
    .execute(&state.db)
    .await;
    if let Err(e) = r {
        tracing::warn!("audit log failed: {e}");
    }
}
