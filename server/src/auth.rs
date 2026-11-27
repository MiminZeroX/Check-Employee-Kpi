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

pub const ROLES: [&str; 5] = ["admin", "hr", "manager", "viewer", "employee"];

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: i64,
    pub name: String,
    pub role: String,
    pub emp: Option<i64>,
    pub exp: i64,
}

/// An authenticated dashboard user (JWT bearer token).
#[derive(Debug, Clone)]
pub struct AuthUser(pub Claims);

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
    /// Succeeds when the user has one of the given roles (admin always passes).
    pub fn require(&self, roles: &[&str]) -> AppResult<()> {
        if self.0.role == "admin" || roles.contains(&self.0.role.as_str()) {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }
    /// Staff roles that may see organisation-wide data.
    pub fn require_staff(&self) -> AppResult<()> {
        self.require(&["hr", "manager", "viewer"])
    }
}

pub fn issue_token(state: &AppState, c: Claims) -> AppResult<String> {
    encode(&Header::default(), &c, &EncodingKey::from_secret(state.cfg.jwt_secret.as_bytes()))
        .map_err(|e| AppError::Internal(e.into()))
}

pub fn new_claims(state: &AppState, id: i64, name: String, role: String, emp: Option<i64>) -> Claims {
    Claims {
        sub: id,
        name,
        role,
        emp,
        exp: (Utc::now() + Duration::hours(state.cfg.jwt_ttl_hours)).timestamp(),
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

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .ok_or(AppError::Unauthorized)?;
        let token = header.strip_prefix("Bearer ").ok_or(AppError::Unauthorized)?;
        let claims = verify_token(state, token)?;
        // make sure the account is still active
        let active: Option<(bool,)> = sqlx::query_as("SELECT is_active FROM users WHERE id = ?")
            .bind(claims.sub)
            .fetch_optional(&state.db)
            .await?;
        match active {
            Some((true,)) => Ok(AuthUser(claims)),
            _ => Err(AppError::Unauthorized),
        }
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
