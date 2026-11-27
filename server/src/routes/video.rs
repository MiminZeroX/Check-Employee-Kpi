//! Authenticated proxy in front of go2rtc.
//!
//! go2rtc's own API has no access control and can be told to run commands
//! (exec/echo sources), so it must never be reachable from the office network.
//! Browsers watch cameras only through `/video/*` here, which:
//!   * requires a logged-in staff user (HttpOnly, SameSite=Strict cookie, path /video)
//!   * forwards only the read-only player paths; stream management is never reachable

use std::sync::OnceLock;

use axum::{
    body::Body,
    extract::{
        ws::{Message as AxMsg, WebSocket, WebSocketUpgrade},
        Path, RawQuery, State,
    },
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message as TMsg;

use crate::{
    auth::{issue_token, new_claims, verify_token, AuthUser, Claims},
    error::{AppError, AppResult},
    state::AppState,
};

const COOKIE: &str = "so_video";
/// Player files and read-only endpoints that the browser may reach.
const ALLOWED: [&str; 6] = ["stream.html", "video-rtc.js", "video-stream.js", "api/frame.jpeg", "api/stream.mjpeg", "api/ws"];

pub fn api_routes() -> Router<AppState> {
    Router::new()
        .route("/video/session", post(open_session).delete(close_session))
        .route("/video/status", get(status))
}

pub fn proxy_routes() -> Router<AppState> {
    Router::new().route("/video/api/ws", get(ws_proxy)).route("/video/{*path}", get(http_proxy))
}

fn upstream(st: &AppState) -> AppResult<String> {
    st.cfg
        .go2rtc_api
        .as_ref()
        .map(|u| u.trim_end_matches('/').to_string())
        .ok_or_else(|| AppError::BadRequest("GO2RTC_API is not configured".into()))
}

/// Client without a total timeout: video streams stay open.
fn stream_client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("http client")
    })
}

fn cookie_header(value: &str, max_age: i64) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{COOKIE}={value}; Path=/video; HttpOnly; SameSite=Strict; Max-Age={max_age}"
    ))
    .expect("cookie")
}

/// Issues the video cookie for the logged-in user (staff only).
async fn open_session(State(st): State<AppState>, u: AuthUser) -> AppResult<Response> {
    u.require_staff()?;
    let c = new_claims(&st, u.id(), u.name().to_string(), u.role().to_string(), u.0.emp);
    let token = issue_token(&st, c)?;
    let mut r = Json(json!({ "ok": true })).into_response();
    r.headers_mut().insert(header::SET_COOKIE, cookie_header(&token, st.cfg.jwt_ttl_hours * 3600));
    Ok(r)
}

async fn close_session() -> Response {
    let mut r = Json(json!({ "ok": true })).into_response();
    r.headers_mut().insert(header::SET_COOKIE, cookie_header("", 0));
    r
}

async fn status(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Value>> {
    u.require_staff()?;
    let ok = match &st.cfg.go2rtc_api {
        Some(api) => st.http.get(format!("{}/api", api.trim_end_matches('/'))).send().await.is_ok_and(|r| r.status().is_success()),
        None => false,
    };
    Ok(Json(json!({ "available": ok })))
}

async fn viewer(st: &AppState, headers: &HeaderMap) -> AppResult<Claims> {
    let token = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|h| h.split(';'))
        .find_map(|kv| kv.trim().strip_prefix(&format!("{COOKIE}=")).map(|v| v.to_string()))
        .ok_or(AppError::Unauthorized)?;
    let claims = verify_token(st, &token)?;
    if claims.role == "employee" {
        return Err(AppError::Forbidden);
    }
    let active: Option<(bool,)> =
        sqlx::query_as("SELECT is_active FROM users WHERE id = ?").bind(claims.sub).fetch_optional(&st.db).await?;
    if !matches!(active, Some((true,))) {
        return Err(AppError::Unauthorized);
    }
    Ok(claims)
}

async fn http_proxy(
    State(st): State<AppState>,
    Path(path): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> AppResult<Response> {
    if !ALLOWED.contains(&path.as_str()) || path == "api/ws" {
        return Err(AppError::NotFound);
    }
    viewer(&st, &headers).await?;
    let mut url = format!("{}/{}", upstream(&st)?, path);
    if let Some(q) = query {
        url.push('?');
        url.push_str(&q);
    }
    let resp = stream_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("go2rtc unreachable: {e}")))?;
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let ctype = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
    let mut out = Response::new(Body::from_stream(resp.bytes_stream()));
    *out.status_mut() = status;
    let h = out.headers_mut();
    if let Some(ct) = ctype.and_then(|c| HeaderValue::from_str(&c).ok()) {
        h.insert(header::CONTENT_TYPE, ct);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // the go2rtc player page is framed by our own pages and uses its own inline module script
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("SAMEORIGIN"));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; \
             img-src 'self' data: blob:; media-src 'self' blob:; connect-src 'self' ws: wss:; \
             object-src 'none'; base-uri 'none'; frame-ancestors 'self'",
        ),
    );
    Ok(out)
}

async fn ws_proxy(
    State(st): State<AppState>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> AppResult<Response> {
    viewer(&st, &headers).await?;
    let base = upstream(&st)?;
    let ws_base = base.replacen("http://", "ws://", 1).replacen("https://", "wss://", 1);
    let url = match query {
        Some(q) => format!("{ws_base}/api/ws?{q}"),
        None => format!("{ws_base}/api/ws"),
    };
    Ok(ws.on_upgrade(move |socket| pump(socket, url)))
}

async fn pump(client: WebSocket, url: String) {
    let upstream = match tokio_tungstenite::connect_async(&url).await {
        Ok((s, _)) => s,
        Err(e) => {
            tracing::warn!("go2rtc websocket failed: {e}");
            return;
        }
    };
    let (mut up_tx, mut up_rx) = upstream.split();
    let (mut cl_tx, mut cl_rx) = client.split();
    let to_up = async {
        while let Some(Ok(m)) = cl_rx.next().await {
            let m = match m {
                AxMsg::Text(t) => TMsg::text(t.as_str().to_string()),
                AxMsg::Binary(b) => TMsg::binary(b),
                AxMsg::Ping(p) => TMsg::Ping(p),
                AxMsg::Pong(p) => TMsg::Pong(p),
                AxMsg::Close(_) => break,
            };
            if up_tx.send(m).await.is_err() {
                break;
            }
        }
        let _ = up_tx.close().await;
    };
    let to_client = async {
        while let Some(Ok(m)) = up_rx.next().await {
            let m = match m {
                TMsg::Text(t) => AxMsg::Text(t.as_str().into()),
                TMsg::Binary(b) => AxMsg::Binary(b),
                TMsg::Ping(p) => AxMsg::Ping(p),
                TMsg::Pong(p) => AxMsg::Pong(p),
                TMsg::Close(_) => break,
                TMsg::Frame(_) => continue,
            };
            if cl_tx.send(m).await.is_err() {
                break;
            }
        }
        let _ = cl_tx.close().await;
    };
    tokio::select! { _ = to_up => {}, _ = to_client => {} }
}
