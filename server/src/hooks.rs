//! Outgoing webhooks: other systems (HR, ERP, CRM, chat) can subscribe to events.
//! Each POST carries `X-SmartOffice-Event` and `X-SmartOffice-Signature: sha256=<hmac>`
//! (HMAC-SHA256 of the raw body with the webhook's secret).

use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

use crate::state::AppState;

/// Event names offered to subscribers.
pub const EVENTS: [&str; 10] = [
    "attendance.scan",
    "request.created",
    "request.decided",
    "correction.created",
    "correction.decided",
    "wellness.alert",
    "camera.status",
    "ticket.created",
    "ticket.updated",
    "deal.stage_changed",
];

pub fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

/// Delivers an event to every active webhook subscribed to it (in the background).
pub fn emit(st: &AppState, event: &'static str, data: Value) {
    let st = st.clone();
    tokio::spawn(async move {
        let hooks: Vec<(i64, String, String, String)> =
            match sqlx::query_as("SELECT id, url, events, secret FROM webhooks WHERE is_active = 1").fetch_all(&st.db).await {
                Ok(h) => h,
                Err(e) => {
                    tracing::warn!("webhooks: {e}");
                    return;
                }
            };
        for (id, url, events, secret) in hooks {
            let wanted = events.split(',').map(str::trim).any(|e| e == "*" || e == event);
            if !wanted {
                continue;
            }
            let body = json!({ "event": event, "sent_at": chrono::Utc::now(), "data": data }).to_string();
            let secret = st.crypto.decrypt(&secret).unwrap_or_default();
            deliver(&st, id, &url, event, &secret, body).await;
        }
    });
}

pub async fn deliver(st: &AppState, id: i64, url: &str, event: &str, secret: &str, body: String) -> Option<u16> {
    let sig = sign(secret, body.as_bytes());
    let mut status = None;
    let mut error = None;
    // two attempts, then record the failure for the admin page
    for attempt in 0..2 {
        match st
            .http
            .post(url)
            .header("content-type", "application/json")
            .header("x-smartoffice-event", event)
            .header("x-smartoffice-signature", &sig)
            .body(body.clone())
            .send()
            .await
        {
            Ok(r) => {
                status = Some(r.status().as_u16());
                error = (!r.status().is_success()).then(|| format!("HTTP {}", r.status()));
                if r.status().is_success() {
                    break;
                }
            }
            Err(e) => error = Some(e.to_string().chars().take(280).collect()),
        }
        if attempt == 0 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
    let _ = sqlx::query("UPDATE webhooks SET last_status = ?, last_error = ?, last_sent_at = UTC_TIMESTAMP() WHERE id = ?")
        .bind(status.map(|s| s as i32))
        .bind(error)
        .bind(id)
        .execute(&st.db)
        .await;
    status
}
