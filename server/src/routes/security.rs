//! Two-factor sign-in (TOTP), OpenID Connect single sign-on, client address
//! resolution and the API rate limit.

use std::{
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};

use axum::{
    extract::{ConnectInfo, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use jsonwebtoken::{decode, decode_header, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{
    auth::{audit, verify_password, AuthUser},
    error::{bad, AppError, AppResult},
    routes::users::finish_login,
    state::AppState,
    totp,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/mfa", get(mfa_status))
        .route("/auth/mfa/setup", post(mfa_setup))
        .route("/auth/mfa/enable", post(mfa_enable))
        .route("/auth/mfa/disable", post(mfa_disable))
        .route("/auth/mfa/verify", post(mfa_verify))
        .route("/auth/oidc", get(oidc_info))
        .route("/auth/oidc/start", get(oidc_start))
        .route("/auth/oidc/callback", get(oidc_callback))
        .route("/auth/oidc/exchange", post(oidc_exchange))
}

fn random_token(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut b);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

// ---------------------------------------------------------------- client address / rate limit

/// The client's address. Behind a trusted reverse proxy (TRUST_PROXY=1) this is the
/// right-most X-Forwarded-For entry, i.e. the address the proxy itself saw; entries
/// further left are supplied by the client and cannot be trusted.
pub fn client_ip(st: &AppState, headers: &HeaderMap, addr: SocketAddr) -> IpAddr {
    if st.cfg.trust_proxy {
        let fwd = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .or_else(|| headers.get("x-real-ip").and_then(|v| v.to_str().ok()));
        if let Some(ip) = fwd.and_then(|v| v.trim().parse().ok()) {
            return ip;
        }
    }
    addr.ip()
}

/// Token bucket per client address for everything under /api.
pub async fn rate_limit(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let rps = st.cfg.rate_limit_rps;
    if rps > 0.0 {
        let addr = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0)
            .unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 0)));
        let ip = client_ip(&st, req.headers(), addr);
        let burst = (rps * 4.0).max(10.0);
        let now = Instant::now();
        let allowed = {
            let mut m = st.rate.lock().await;
            if m.len() > 10_000 {
                m.retain(|_, (_, t)| now.duration_since(*t) < Duration::from_secs(60));
            }
            let e = m.entry(ip).or_insert((burst, now));
            e.0 = (e.0 + now.duration_since(e.1).as_secs_f64() * rps).min(burst);
            e.1 = now;
            if e.0 >= 1.0 {
                e.0 -= 1.0;
                true
            } else {
                false
            }
        };
        if !allowed {
            let mut r = (StatusCode::TOO_MANY_REQUESTS, Json(json!({ "error": "เรียกใช้งานถี่เกินไป กรุณาลองใหม่อีกครั้ง" })))
                .into_response();
            r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
            return r;
        }
    }
    next.run(req).await
}

// ---------------------------------------------------------------- MFA

#[derive(Serialize, Deserialize)]
struct MfaClaims {
    sub: i64,
    tv: i64,
    exp: i64,
    purpose: String,
}

fn mfa_key(st: &AppState) -> Vec<u8> {
    // a different key than session tokens, so a pending-MFA token can never be used as a session
    format!("{}:mfa-pending", st.cfg.jwt_secret).into_bytes()
}

/// Token returned by /auth/login instead of a session when the account has MFA on.
pub fn mfa_token(st: &AppState, id: i64, tv: i64) -> AppResult<String> {
    let c = MfaClaims { sub: id, tv, exp: chrono::Utc::now().timestamp() + 300, purpose: "mfa".into() };
    encode(&Header::default(), &c, &EncodingKey::from_secret(&mfa_key(st))).map_err(|e| AppError::Internal(e.into()))
}

async fn mfa_row(st: &AppState, id: i64) -> AppResult<(bool, Option<String>, Option<i64>, String)> {
    let r: (bool, Option<String>, Option<i64>, String) =
        sqlx::query_as("SELECT mfa_enabled, totp_secret, totp_last_step, username FROM users WHERE id = ?")
            .bind(id)
            .fetch_one(&st.db)
            .await?;
    Ok(r)
}

fn secret_of(st: &AppState, stored: &Option<String>) -> AppResult<String> {
    let s = stored.as_deref().ok_or_else(|| bad("ยังไม่ได้ตั้งค่าแอปยืนยันตัวตน"))?;
    st.crypto.decrypt(s).map_err(|e| AppError::Internal(e))
}

/// Verifies a code and records its time step so it cannot be used twice.
async fn check_code(st: &AppState, id: i64, secret: &str, last: Option<i64>, code: &str) -> AppResult<bool> {
    match totp::verify(secret, code, chrono::Utc::now().timestamp(), last) {
        Some(step) => {
            // conditional update: two concurrent requests with the same code cannot both win
            let r = sqlx::query("UPDATE users SET totp_last_step = ? WHERE id = ? AND (totp_last_step IS NULL OR totp_last_step < ?)")
                .bind(step)
                .bind(id)
                .bind(step)
                .execute(&st.db)
                .await?;
            Ok(r.rows_affected() == 1)
        }
        None => Ok(false),
    }
}

async fn mfa_status(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    let (enabled, ..) = mfa_row(&st, u.id()).await?;
    Ok(Json(json!({ "enabled": enabled })))
}

#[derive(Deserialize)]
struct PasswordReq {
    password: String,
}

async fn require_password(st: &AppState, id: i64, pw: &str) -> AppResult<()> {
    let (hash,): (String,) = sqlx::query_as("SELECT password_hash FROM users WHERE id = ?").bind(id).fetch_one(&st.db).await?;
    if verify_password(pw, &hash) {
        Ok(())
    } else {
        tokio::time::sleep(Duration::from_millis(600)).await;
        Err(bad("รหัสผ่านไม่ถูกต้อง"))
    }
}

async fn mfa_setup(State(st): State<AppState>, u: AuthUser, Json(r): Json<PasswordReq>) -> AppResult<Json<Value>> {
    require_password(&st, u.id(), &r.password).await?;
    let (enabled, _, _, username) = mfa_row(&st, u.id()).await?;
    if enabled {
        return Err(bad("เปิดใช้การยืนยันสองขั้นตอนอยู่แล้ว ปิดก่อนจึงจะตั้งค่าใหม่ได้"));
    }
    let secret = totp::new_secret();
    sqlx::query("UPDATE users SET totp_secret = ?, totp_last_step = NULL WHERE id = ?")
        .bind(st.crypto.encrypt(&secret))
        .bind(u.id())
        .execute(&st.db)
        .await?;
    let uri = totp::otpauth_uri(&st.cfg.mfa_issuer, &username, &secret);
    let qr = qrcode::QrCode::new(uri.as_bytes()).map_err(|e| AppError::Internal(e.into()))?;
    let svg = qr
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(200, 200)
        .dark_color(qrcode::render::svg::Color("#000000"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build();
    let qr_data = format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(svg));
    Ok(Json(json!({ "secret": secret, "uri": uri, "qr": qr_data })))
}

#[derive(Deserialize)]
struct CodeReq {
    code: String,
    #[serde(default)]
    password: Option<String>,
}

async fn mfa_enable(State(st): State<AppState>, u: AuthUser, Json(r): Json<CodeReq>) -> AppResult<Json<Value>> {
    let (enabled, stored, last, _) = mfa_row(&st, u.id()).await?;
    if enabled {
        return Ok(Json(json!({ "enabled": true })));
    }
    let secret = secret_of(&st, &stored)?;
    if !check_code(&st, u.id(), &secret, last, &r.code).await? {
        return Err(bad("รหัส 6 หลักไม่ถูกต้อง ตรวจสอบเวลาบนโทรศัพท์แล้วลองใหม่"));
    }
    sqlx::query("UPDATE users SET mfa_enabled = 1 WHERE id = ?").bind(u.id()).execute(&st.db).await?;
    audit(&st, Some(&u), "mfa_enable", "user", u.id(), json!({})).await;
    Ok(Json(json!({ "enabled": true })))
}

async fn mfa_disable(State(st): State<AppState>, u: AuthUser, Json(r): Json<CodeReq>) -> AppResult<Json<Value>> {
    require_password(&st, u.id(), r.password.as_deref().unwrap_or("")).await?;
    let (enabled, stored, last, _) = mfa_row(&st, u.id()).await?;
    if enabled {
        let secret = secret_of(&st, &stored)?;
        if !check_code(&st, u.id(), &secret, last, &r.code).await? {
            return Err(bad("รหัส 6 หลักไม่ถูกต้อง"));
        }
    }
    sqlx::query("UPDATE users SET mfa_enabled = 0, totp_secret = NULL, totp_last_step = NULL WHERE id = ?")
        .bind(u.id())
        .execute(&st.db)
        .await?;
    audit(&st, Some(&u), "mfa_disable", "user", u.id(), json!({})).await;
    Ok(Json(json!({ "enabled": false })))
}

#[derive(Deserialize)]
struct VerifyReq {
    mfa_token: String,
    code: String,
}

async fn mfa_verify(
    State(st): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(r): Json<VerifyReq>,
) -> AppResult<Json<Value>> {
    let mut v = Validation::new(Algorithm::HS256);
    v.leeway = 5;
    let c = decode::<MfaClaims>(&r.mfa_token, &DecodingKey::from_secret(&mfa_key(&st)), &v)
        .map_err(|_| bad("หมดเวลายืนยันตัวตน กรุณาเข้าสู่ระบบใหม่"))?
        .claims;
    if c.purpose != "mfa" {
        return Err(AppError::Unauthorized);
    }
    let key = format!("mfa:{}", c.sub);
    {
        let mut f = st.login_failures.lock().await;
        f.retain(|_, (_, first)| first.elapsed() < Duration::from_secs(15 * 60));
        if f.get(&key).is_some_and(|(n, _)| *n >= 5) {
            return Err(AppError::TooMany("ใส่รหัสผิดหลายครั้ง กรุณารอ 15 นาทีแล้วลองใหม่".into()));
        }
    }
    let row: Option<(String, String, Option<i64>, bool, i64, bool, Option<String>, Option<i64>)> = sqlx::query_as(
        "SELECT display_name, role, employee_id, is_active, token_version, mfa_enabled, totp_secret, totp_last_step
           FROM users WHERE id = ?",
    )
    .bind(c.sub)
    .fetch_optional(&st.db)
    .await?;
    let Some((name, role, emp, active, tv, enabled, stored, last)) = row else { return Err(AppError::Unauthorized) };
    if !active || tv != c.tv || !enabled {
        return Err(AppError::Unauthorized);
    }
    let secret = secret_of(&st, &stored)?;
    let ip = client_ip(&st, &headers, addr).to_string();
    if !check_code(&st, c.sub, &secret, last, &r.code).await? {
        let mut f = st.login_failures.lock().await;
        f.entry(key).or_insert((0, Instant::now())).0 += 1;
        drop(f);
        tokio::time::sleep(Duration::from_millis(600)).await;
        audit(&st, None, "mfa_failed", "user", c.sub, json!({ "ip": ip })).await;
        return Err(bad("รหัส 6 หลักไม่ถูกต้อง"));
    }
    st.login_failures.lock().await.remove(&key);
    Ok(Json(finish_login(&st, c.sub, name, role, emp, tv, &ip, "password+totp").await?))
}

// ---------------------------------------------------------------- OpenID Connect

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
}

async fn discovery(st: &AppState) -> AppResult<Discovery> {
    let o = st.cfg.oidc.as_ref().ok_or(AppError::NotFound)?;
    let url = format!("{}/.well-known/openid-configuration", o.issuer);
    let d: Discovery = st
        .http
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| AppError::Internal(e.into()))?
        .json()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    if d.issuer.trim_end_matches('/') != o.issuer {
        return Err(AppError::Internal(anyhow::anyhow!("OIDC issuer mismatch: {}", d.issuer)));
    }
    Ok(d)
}

async fn oidc_info(State(st): State<AppState>) -> Json<Value> {
    Json(match &st.cfg.oidc {
        Some(o) => json!({ "enabled": true, "label": o.button_label }),
        None => json!({ "enabled": false }),
    })
}

const OIDC_COOKIE: &str = "so_oidc";

async fn oidc_start(State(st): State<AppState>) -> AppResult<Response> {
    let o = st.cfg.oidc.clone().ok_or(AppError::NotFound)?;
    let d = discovery(&st).await?;
    let (state, nonce, verifier) = (random_token(24), random_token(24), random_token(48));
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    {
        let mut p = st.pending.lock().await;
        p.retain(|_, (_, t)| t.elapsed() < Duration::from_secs(600));
        p.insert(format!("oidc:{state}"), (json!({ "nonce": nonce, "verifier": verifier }), Instant::now()));
    }
    let mut url = reqwest::Url::parse(&d.authorization_endpoint).map_err(|e| AppError::Internal(e.into()))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &o.client_id)
        .append_pair("redirect_uri", &o.redirect_url)
        .append_pair("scope", "openid email profile")
        .append_pair("state", &state)
        .append_pair("nonce", &nonce)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    let mut r = Redirect::to(url.as_str()).into_response();
    // ties the login to this browser (prevents login CSRF); Lax so it survives the provider's redirect back
    let secure = if o.redirect_url.starts_with("https://") { "; Secure" } else { "" };
    let cookie = format!("{OIDC_COOKIE}={state}; Path=/api/auth/oidc; HttpOnly; SameSite=Lax; Max-Age=600{secure}");
    r.headers_mut().insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).map_err(|e| AppError::Internal(e.into()))?);
    Ok(r)
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct IdClaims {
    sub: String,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    email_verified: Option<bool>,
    #[serde(default)]
    preferred_username: Option<String>,
}

fn sso_fail(msg: &str) -> Response {
    let enc: String = msg
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    let mut r = Redirect::to(&format!("/#sso_error={enc}")).into_response();
    r.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("so_oidc=; Path=/api/auth/oidc; HttpOnly; SameSite=Lax; Max-Age=0"),
    );
    r
}

async fn oidc_callback(State(st): State<AppState>, headers: HeaderMap, Query(q): Query<CallbackQuery>) -> Response {
    match oidc_complete(&st, &headers, q).await {
        Ok(code) => {
            let mut r = Redirect::to(&format!("/#sso={code}")).into_response();
            r.headers_mut().insert(
                header::SET_COOKIE,
                HeaderValue::from_static("so_oidc=; Path=/api/auth/oidc; HttpOnly; SameSite=Lax; Max-Age=0"),
            );
            r
        }
        Err(msg) => {
            tracing::warn!("OIDC sign-in failed: {msg}");
            sso_fail(&msg)
        }
    }
}

async fn oidc_complete(st: &AppState, headers: &HeaderMap, q: CallbackQuery) -> Result<String, String> {
    let generic = "เข้าสู่ระบบด้วยบัญชีองค์กรไม่สำเร็จ".to_string();
    let o = st.cfg.oidc.clone().ok_or_else(|| generic.clone())?;
    if let Some(e) = q.error {
        return Err(format!("ผู้ให้บริการปฏิเสธการเข้าสู่ระบบ ({})", e.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(40).collect::<String>()));
    }
    let (code, state) = q.code.zip(q.state).ok_or_else(|| generic.clone())?;
    let cookie_state = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().strip_prefix("so_oidc="))
        .next()
        .map(str::to_string);
    if cookie_state.as_deref() != Some(state.as_str()) {
        return Err("คำขอเข้าสู่ระบบหมดอายุหรือไม่ได้เริ่มจากเบราว์เซอร์นี้ กรุณาลองใหม่".into());
    }
    let pend = st.pending.lock().await.remove(&format!("oidc:{state}")).filter(|(_, t)| t.elapsed() < Duration::from_secs(600));
    let (pend, _) = pend.ok_or_else(|| "คำขอเข้าสู่ระบบหมดอายุ กรุณาลองใหม่".to_string())?;
    let d = discovery(st).await.map_err(|_| "ติดต่อผู้ให้บริการยืนยันตัวตนไม่ได้".to_string())?;

    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code),
        ("redirect_uri", o.redirect_url.clone()),
        ("client_id", o.client_id.clone()),
        ("code_verifier", pend["verifier"].as_str().unwrap_or_default().to_string()),
    ];
    if !o.client_secret.is_empty() {
        form.push(("client_secret", o.client_secret.clone()));
    }
    let tok: Value = st
        .http
        .post(&d.token_endpoint)
        .form(&form)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("แลกรหัสไม่สำเร็จ: {e}"))?
        .json()
        .await
        .map_err(|_| generic.clone())?;
    let id_token = tok["id_token"].as_str().ok_or_else(|| generic.clone())?;

    // verify the ID token signature with the provider's published keys
    let head = decode_header(id_token).map_err(|_| generic.clone())?;
    if !matches!(
        head.alg,
        Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512 | Algorithm::PS256 | Algorithm::PS384 | Algorithm::PS512 | Algorithm::ES256 | Algorithm::ES384
    ) {
        return Err("รูปแบบลายเซ็นของโทเคนไม่รองรับ".into());
    }
    let jwks: jsonwebtoken::jwk::JwkSet = st
        .http
        .get(&d.jwks_uri)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|_| generic.clone())?
        .json()
        .await
        .map_err(|_| generic.clone())?;
    let jwk = match &head.kid {
        Some(kid) => jwks.find(kid),
        None if jwks.keys.len() == 1 => jwks.keys.first(),
        None => None,
    }
    .ok_or_else(|| "ไม่พบกุญแจของผู้ให้บริการ".to_string())?;
    let key = DecodingKey::from_jwk(jwk).map_err(|_| generic.clone())?;
    let mut v = Validation::new(head.alg);
    v.set_audience(&[&o.client_id]);
    v.set_issuer(&[o.issuer.as_str(), d.issuer.as_str()]);
    v.leeway = 60;
    let c = decode::<IdClaims>(id_token, &key, &v).map_err(|e| format!("โทเคนไม่ถูกต้อง: {e}"))?.claims;
    if c.nonce.as_deref() != pend["nonce"].as_str() {
        return Err("โทเคนไม่ถูกต้อง (nonce)".into());
    }

    // find the account: linked subject first, then a verified e-mail of a user or their employee record
    let subject = format!("{}|{}", o.issuer, c.sub);
    let mut uid: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE oidc_subject = ?")
        .bind(&subject)
        .fetch_optional(&st.db)
        .await
        .map_err(|_| generic.clone())?;
    if uid.is_none() {
        let email = c.email.clone().or(c.preferred_username.clone()).filter(|e| e.contains('@'));
        if let (Some(email), true) = (email, c.email_verified != Some(false)) {
            let found: Vec<(i64,)> = sqlx::query_as(
                "SELECT DISTINCT u.id FROM users u LEFT JOIN employees e ON e.id = u.employee_id
                  WHERE u.oidc_subject IS NULL AND (LOWER(u.email) = LOWER(?) OR LOWER(e.email) = LOWER(?) OR LOWER(u.username) = LOWER(?))",
            )
            .bind(&email)
            .bind(&email)
            .bind(&email)
            .fetch_all(&st.db)
            .await
            .map_err(|_| generic.clone())?;
            if found.len() == 1 {
                sqlx::query("UPDATE users SET oidc_subject = ? WHERE id = ? AND oidc_subject IS NULL")
                    .bind(&subject)
                    .bind(found[0].0)
                    .execute(&st.db)
                    .await
                    .map_err(|_| generic.clone())?;
                uid = Some(found[0].0);
            }
        }
    }
    let uid = uid.ok_or_else(|| "บัญชีนี้ยังไม่ได้ผูกกับผู้ใช้ในระบบ กรุณาติดต่อผู้ดูแลระบบ".to_string())?;
    let active: bool = sqlx::query_scalar("SELECT is_active FROM users WHERE id = ?")
        .bind(uid)
        .fetch_one(&st.db)
        .await
        .map_err(|_| generic.clone())?;
    if !active {
        return Err("บัญชีนี้ถูกระงับ".into());
    }
    // hand the session to the page through a one-time code, never a token in the URL
    let one_time = random_token(24);
    st.pending.lock().await.insert(format!("sso:{one_time}"), (json!({ "uid": uid }), Instant::now()));
    Ok(one_time)
}

#[derive(Deserialize)]
struct ExchangeReq {
    code: String,
}

async fn oidc_exchange(
    State(st): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(r): Json<ExchangeReq>,
) -> AppResult<Json<Value>> {
    let p = st.pending.lock().await.remove(&format!("sso:{}", r.code));
    let (v, _) = p.filter(|(_, t)| t.elapsed() < Duration::from_secs(60)).ok_or(AppError::Unauthorized)?;
    let uid = v["uid"].as_i64().ok_or(AppError::Unauthorized)?;
    let (name, username, role, emp, active, tv): (String, String, String, Option<i64>, bool, i64) =
        sqlx::query_as("SELECT display_name, username, role, employee_id, is_active, token_version FROM users WHERE id = ?")
            .bind(uid)
            .fetch_one(&st.db)
            .await?;
    if !active {
        return Err(AppError::Unauthorized);
    }
    let name = if name.is_empty() { username } else { name };
    let ip = client_ip(&st, &headers, addr).to_string();
    Ok(Json(finish_login(&st, uid, name, role, emp, tv, &ip, "oidc").await?))
}
